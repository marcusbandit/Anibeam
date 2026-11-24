//! A clear logo's file as the image cache keeps it (#1470). TheTVDB's clear
//! logos are meant to be the show's name on nothing, but some carry a wide
//! transparent margin, which leaves the mark floating inside the box the
//! series head draws it in, and some are no clear logo at all: the name on
//! a solid card, whose edge may even be clear around a stamp-like shape.
//! The cache trims the first to the mark and marks the second as not a
//! logo, so the pick passes over it.

use std::io::Cursor;

/// What the cache found out about a logo's file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogoFile {
    /// A clear logo. `trimmed` is the file cut to its mark when it had
    /// fully transparent rows or columns around it, `None` when it had none.
    Clear { trimmed: Option<Vec<u8>> },
    /// A picture, not a logo: no transparent pixel anywhere on its edge, or
    /// a mark that fills nearly all of its own box, the name on a card.
    Opaque,
    /// Not a file this can read, which is left as it is and drawn.
    Unread,
}

impl LogoFile {
    /// The value `logo_checks.verdict` keeps: 1 a clear logo, 0 not a logo,
    /// 2 a file this could not read, which is drawn as it is.
    pub fn check(&self) -> i64 {
        match self {
            LogoFile::Clear { .. } => 1,
            LogoFile::Opaque => 0,
            LogoFile::Unread => 2,
        }
    }
}

/// The share of its trimmed box a mark may cover solidly and still be a
/// name drawn on nothing. Lettering covers well under two thirds of its box;
/// a name on a card, scalloped or rounded, covers nearly all of it.
const MAX_SOLID: f64 = 0.85;

/// The alpha from which a pixel counts as solid.
const SOLID_ALPHA: u8 = 128;

/// The alpha under which an edge pixel counts as clear: the faint haze or
/// glow some logos carry out to the canvas edge is no background.
const CLEAR_ALPHA: u8 = 8;

/// A logo the cache would decode at most: anything larger is left unread
/// rather than held in memory whole.
const MAX_PIXELS: u64 = 16_000_000;

/// Reads `bytes` as a logo. A JPEG has no transparency at all, so it is
/// never a clear logo; a PNG is judged by its edge and trimmed to its mark;
/// anything else is left unread.
pub fn inspect(bytes: &[u8]) -> LogoFile {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return LogoFile::Opaque;
    }
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return LogoFile::Unread;
    }
    match decode(bytes) {
        Some(image) => judge(&image),
        None => LogoFile::Unread,
    }
}

/// The colour information a PNG carries beside its pixels, kept so a
/// trimmed logo draws in the same colours as the file it came from.
#[derive(Default)]
struct Colour {
    icc_profile: Option<Vec<u8>>,
    gamma: Option<png::ScaledFloat>,
    chromaticities: Option<png::SourceChromaticities>,
    srgb: Option<png::SrgbRenderingIntent>,
}

/// A decoded PNG, eight bits a channel.
struct Decoded {
    width: usize,
    height: usize,
    channels: usize,
    color: png::ColorType,
    colour: Colour,
    data: Vec<u8>,
}

impl Decoded {
    /// The alpha of the pixel at `x`, `y`; 255 for a format with none.
    fn alpha(&self, x: usize, y: usize) -> u8 {
        match self.color {
            png::ColorType::Rgba | png::ColorType::GrayscaleAlpha => {
                self.data[(y * self.width + x) * self.channels + self.channels - 1]
            }
            _ => 255,
        }
    }
}

fn decode(bytes: &[u8]) -> Option<Decoded> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let (width, height) = reader.info().size();
    let colour = {
        let info = reader.info();
        Colour {
            icc_profile: info.icc_profile.as_ref().map(|p| p.to_vec()),
            gamma: info.gamma(),
            chromaticities: info.chromaticities(),
            srgb: info.srgb,
        }
    };
    if u64::from(width) * u64::from(height) > MAX_PIXELS || width == 0 || height == 0 {
        return None;
    }
    let mut data = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut data).ok()?;
    if info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    let channels = info.color_type.samples();
    data.truncate(info.buffer_size());
    Some(Decoded {
        width: info.width as usize,
        height: info.height as usize,
        channels,
        color: info.color_type,
        colour,
        data,
    })
}

fn judge(image: &Decoded) -> LogoFile {
    let (w, h) = (image.width, image.height);
    let clear = |x, y| image.alpha(x, y) < CLEAR_ALPHA;
    let clear_edge = (0..w).any(|x| clear(x, 0) || clear(x, h - 1))
        || (0..h).any(|y| clear(0, y) || clear(w - 1, y));
    if !clear_edge {
        return LogoFile::Opaque;
    }
    // The box around every pixel that is not fully transparent.
    let mut bounds: Option<(usize, usize, usize, usize)> = None;
    for y in 0..h {
        for x in 0..w {
            if image.alpha(x, y) != 0 {
                bounds = Some(match bounds {
                    None => (x, y, x, y),
                    Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                });
            }
        }
    }
    // Nothing drawn at all: there is no mark to show.
    let Some((x0, y0, x1, y1)) = bounds else {
        return LogoFile::Opaque;
    };
    let solid = (y0..=y1)
        .flat_map(|y| (x0..=x1).map(move |x| (x, y)))
        .filter(|&(x, y)| image.alpha(x, y) >= SOLID_ALPHA)
        .count();
    let area = (x1 - x0 + 1) * (y1 - y0 + 1);
    if solid as f64 >= MAX_SOLID * area as f64 {
        return LogoFile::Opaque;
    }
    if (x0, y0, x1, y1) == (0, 0, w - 1, h - 1) {
        return LogoFile::Clear { trimmed: None };
    }
    let row = image.width * image.channels;
    let mut cut = Vec::with_capacity((x1 - x0 + 1) * (y1 - y0 + 1) * image.channels);
    for y in y0..=y1 {
        let start = y * row + x0 * image.channels;
        cut.extend_from_slice(&image.data[start..start + (x1 - x0 + 1) * image.channels]);
    }
    LogoFile::Clear {
        trimmed: encode(
            (x1 - x0 + 1) as u32,
            (y1 - y0 + 1) as u32,
            image.color,
            &image.colour,
            &cut,
        ),
    }
}

fn encode(
    width: u32,
    height: u32,
    color: png::ColorType,
    colour: &Colour,
    data: &[u8],
) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut info = png::Info::with_size(width, height);
        info.color_type = color;
        info.bit_depth = png::BitDepth::Eight;
        info.icc_profile = colour.icc_profile.clone().map(Into::into);
        info.source_gamma = colour.gamma;
        info.source_chromaticities = colour.chromaticities;
        info.srgb = colour.srgb;
        let encoder = png::Encoder::with_info(&mut out, info).ok()?;
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(data).ok()?;
        writer.finish().ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `w` by `h` RGBA PNG, every pixel from `pixel(x, y)`.
    fn png_of(w: u32, h: u32, pixel: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let mut data = Vec::new();
        for y in 0..h {
            for x in 0..w {
                data.extend_from_slice(&pixel(x, y));
            }
        }
        encode(w, h, png::ColorType::Rgba, &Colour::default(), &data).unwrap()
    }

    fn size_of(bytes: &[u8]) -> (usize, usize) {
        let image = decode(bytes).unwrap();
        (image.width, image.height)
    }

    #[test]
    fn a_transparent_margin_is_trimmed_to_the_mark() {
        // Two strokes, at (3, 2) and (6, 4), in a 10 by 6 file, the rest clear.
        let logo = png_of(10, 6, |x, y| {
            if (x, y) == (3, 2) || (x, y) == (6, 4) {
                [200, 10, 10, 255]
            } else {
                [0, 0, 0, 0]
            }
        });
        let LogoFile::Clear {
            trimmed: Some(trimmed),
        } = inspect(&logo)
        else {
            panic!("{:?}", inspect(&logo));
        };
        assert_eq!(size_of(&trimmed), (4, 3));
        let image = decode(&trimmed).unwrap();
        assert_eq!(&image.data[..4], &[200, 10, 10, 255]);
        // Trimmed once, there is nothing left to trim.
        assert_eq!(inspect(&trimmed), LogoFile::Clear { trimmed: None });
    }

    #[test]
    fn a_logo_that_reaches_its_edges_with_clear_corners_stays_as_it_is() {
        // A diamond touching every edge, its corners clear.
        let logo = png_of(5, 5, |x, y| {
            let d = (x as i32 - 2).abs() + (y as i32 - 2).abs();
            if d <= 2 {
                [255, 255, 255, 255]
            } else {
                [0, 0, 0, 0]
            }
        });
        assert_eq!(inspect(&logo), LogoFile::Clear { trimmed: None });
        // Half transparent is not transparent: a faint margin is drawn.
        let faint = png_of(4, 4, |x, y| {
            if (x, y) == (0, 0) {
                [0, 0, 0, 0]
            } else {
                [9, 9, 9, 1]
            }
        });
        assert_eq!(inspect(&faint), LogoFile::Clear { trimmed: None });
    }

    #[test]
    fn a_faint_glow_to_the_edge_is_no_background_and_the_colour_stays() {
        // Lettering in the middle, a glow of alpha 3 out to every edge.
        let glow = png_of(10, 6, |x, y| {
            if (x == 3 || x == 6) && (2..4).contains(&y) {
                [255, 255, 255, 255]
            } else {
                [255, 255, 255, 3]
            }
        });
        assert_eq!(inspect(&glow), LogoFile::Clear { trimmed: None });
        // A logo that names its colour space keeps it when it is trimmed.
        let colour = Colour {
            gamma: Some(png::ScaledFloat::new(0.45455)),
            srgb: Some(png::SrgbRenderingIntent::Perceptual),
            ..Colour::default()
        };
        let mut data = vec![0u8; 6 * 4 * 4];
        for (x, y) in [(1, 1), (3, 2)] {
            data[(y * 6 + x) * 4..(y * 6 + x) * 4 + 4].copy_from_slice(&[9, 9, 9, 255]);
        }
        let logo = encode(6, 4, png::ColorType::Rgba, &colour, &data).unwrap();
        let LogoFile::Clear {
            trimmed: Some(trimmed),
        } = inspect(&logo)
        else {
            panic!("{:?}", inspect(&logo));
        };
        let image = decode(&trimmed).unwrap();
        assert_eq!((image.width, image.height), (3, 2));
        assert_eq!(
            image.colour.srgb,
            Some(png::SrgbRenderingIntent::Perceptual)
        );
        // A gamma of its own, without sRGB, stays too.
        let gamma = Colour {
            gamma: Some(png::ScaledFloat::new(0.45455)),
            ..Colour::default()
        };
        let logo = encode(6, 4, png::ColorType::Rgba, &gamma, &data).unwrap();
        let LogoFile::Clear {
            trimmed: Some(trimmed),
        } = inspect(&logo)
        else {
            panic!("{:?}", inspect(&logo));
        };
        assert!(decode(&trimmed).unwrap().colour.gamma.is_some());
    }

    #[test]
    fn a_name_on_a_solid_rectangle_is_not_a_logo() {
        let card = png_of(8, 4, |x, y| {
            if (2..6).contains(&x) && (1..3).contains(&y) {
                [255, 255, 255, 255]
            } else {
                [20, 40, 200, 255]
            }
        });
        assert_eq!(inspect(&card), LogoFile::Opaque);
        assert_eq!(inspect(&card).check(), 0);
        // A stamp: a clear margin all round and clear notches along its
        // edge, but the card inside is solid.
        let stamp = png_of(12, 8, |x, y| {
            let inside = (2..10).contains(&x) && (1..7).contains(&y);
            let notch = (y == 1 || y == 6) && x % 3 == 0;
            if inside && !notch {
                [20, 40, 200, 255]
            } else {
                [0, 0, 0, 0]
            }
        });
        assert_eq!(inspect(&stamp), LogoFile::Opaque);
        // An RGB file has no alpha at all, and a JPEG none either.
        let mut rgb = Vec::new();
        for _ in 0..4 {
            rgb.extend_from_slice(&[1, 2, 3]);
        }
        let rgb = encode(2, 2, png::ColorType::Rgb, &Colour::default(), &rgb).unwrap();
        assert_eq!(inspect(&rgb), LogoFile::Opaque);
        assert_eq!(inspect(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0]), LogoFile::Opaque);
        // Wholly transparent: nothing to draw.
        assert_eq!(
            inspect(&png_of(3, 3, |_, _| [0, 0, 0, 0])),
            LogoFile::Opaque
        );
    }

    #[test]
    fn a_file_that_does_not_read_is_left_alone() {
        assert_eq!(inspect(b""), LogoFile::Unread);
        assert_eq!(inspect(b"GIF89a...."), LogoFile::Unread);
        assert_eq!(inspect(b"\x89PNG\r\n\x1a\nbroken"), LogoFile::Unread);
        assert_eq!(LogoFile::Unread.check(), 2);
        assert_eq!(LogoFile::Clear { trimmed: None }.check(), 1);
    }
}
