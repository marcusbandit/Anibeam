//! How wide a cached image is, read off the file's header (#1475).
//!
//! A backdrop is only worth drawing across the window when it has the
//! pixels for it, so the series page asks how wide the background and the
//! banner are before it hands one out. Nothing is decoded: PNG, GIF and WebP
//! say their size in their first 30 bytes, and a JPEG in its frame header,
//! which a walk reaches by seeking over each metadata segment's length, so
//! a read takes a few dozen bytes whatever the file holds before the frame.
//! Each answer is kept for the life of the process, keyed by the file's
//! path, length and modified time, so a page read stats the file and reads
//! nothing more.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use anibeam_foundation::sync::recover;

/// The most files the memo keeps. A page asks about two at a time and the
/// image cache's row cap is 5000, so a session rarely gets near it.
const MEMO_CAP: usize = 4096;

/// The bytes read before the format is known: enough for every format's
/// size but a JPEG's, which a JPEG then seeks on for.
const HEAD: usize = 32;

/// The width in pixels a PNG, JPEG, GIF or WebP says it is, or None for
/// anything else or a header cut short: a width nobody could read.
pub fn width(bytes: &[u8]) -> Option<u32> {
    width_in(&mut std::io::Cursor::new(bytes))
}

/// `width`, off anything that reads and seeks: the open file, or bytes.
pub fn width_in<R: Read + Seek>(r: &mut R) -> Option<u32> {
    let mut head = Vec::with_capacity(HEAD);
    r.by_ref().take(HEAD as u64).read_to_end(&mut head).ok()?;
    let bytes = head.as_slice();
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        // The first chunk is IHDR, width first.
        if bytes.get(12..16)? != b"IHDR" {
            return None;
        }
        return be32(bytes, 16);
    }
    if bytes.starts_with(&[0xff, 0xd8]) {
        r.seek(SeekFrom::Start(2)).ok()?;
        return jpeg_width(r);
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some(u32::from(u16::from_le_bytes([
            *bytes.get(6)?,
            *bytes.get(7)?,
        ])));
    }
    if bytes.starts_with(b"RIFF") && bytes.get(8..12)? == b"WEBP" {
        return webp_width(bytes);
    }
    None
}

fn be32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn read<const N: usize, R: Read>(r: &mut R) -> Option<[u8; N]> {
    let mut out = [0u8; N];
    r.read_exact(&mut out).ok()?;
    Some(out)
}

/// Walks the segments from just after the start-of-image marker to the
/// first start-of-frame marker, seeking over each segment's payload, and
/// reads the width out of the frame header: precision, height, width.
fn jpeg_width<R: Read + Seek>(r: &mut R) -> Option<u32> {
    let mut byte = read::<1, R>(r)?[0];
    loop {
        if byte != 0xff {
            return None;
        }
        // Fill bytes may pad a marker.
        let mut marker = read::<1, R>(r)?[0];
        while marker == 0xff {
            marker = read::<1, R>(r)?[0];
        }
        match marker {
            // Standalone markers carry no length.
            0x01 | 0xd0..=0xd7 => {
                byte = read::<1, R>(r)?[0];
                continue;
            }
            // The scan began, or the image ended, before any frame header.
            0xd9 | 0xda => return None,
            // Every SOFn but DHT (C4), JPG (C8) and DAC (CC).
            0xc0..=0xcf if !matches!(marker, 0xc4 | 0xc8 | 0xcc) => {
                let frame = read::<7, R>(r)?;
                return Some(u32::from(u16::from_be_bytes([frame[5], frame[6]])));
            }
            _ => {}
        }
        let length = u16::from_be_bytes(read::<2, R>(r)?);
        if length < 2 {
            return None;
        }
        r.seek(SeekFrom::Current(i64::from(length) - 2)).ok()?;
        byte = read::<1, R>(r)?[0];
    }
}

fn webp_width(bytes: &[u8]) -> Option<u32> {
    match bytes.get(12..16)? {
        // Lossy: a frame tag, the start code, then 14 bits of width.
        b"VP8 " => {
            let raw = u16::from_le_bytes([*bytes.get(26)?, *bytes.get(27)?]);
            Some(u32::from(raw & 0x3fff))
        }
        // Lossless: a signature byte, then 14 bits of width minus one.
        b"VP8L" => {
            let raw = u16::from_le_bytes([*bytes.get(21)?, *bytes.get(22)?]);
            Some(u32::from(raw & 0x3fff) + 1)
        }
        // Extended: 24 bits of canvas width minus one.
        b"VP8X" => {
            let raw = bytes.get(24..27)?;
            Some(u32::from_le_bytes([raw[0], raw[1], raw[2], 0]) + 1)
        }
        _ => None,
    }
}

/// What the memo knows about one file, and the file it knew it of.
#[derive(Clone, Copy, PartialEq)]
struct Seen {
    len: u64,
    modified: Option<SystemTime>,
    width: Option<u32>,
}

fn memo() -> &'static Mutex<HashMap<PathBuf, Seen>> {
    static MEMO: OnceLock<Mutex<HashMap<PathBuf, Seen>>> = OnceLock::new();
    MEMO.get_or_init(Mutex::default)
}

/// The width of the image file at `path`, read once per file and then
/// answered from memory. A file that is gone, or not a format `width`
/// reads, has none.
pub fn width_of(path: &Path) -> Option<u32> {
    let meta = std::fs::metadata(path).ok()?;
    let (len, modified) = (meta.len(), meta.modified().ok());
    let seen = recover(memo()).get(path).copied();
    if let Some(seen) = seen
        && seen.len == len
        && seen.modified == modified
    {
        return seen.width;
    }
    let width = std::fs::File::open(path)
        .ok()
        .and_then(|f| width_in(&mut std::io::BufReader::new(f)));
    let mut kept = recover(memo());
    // A bound on a process-long map: the image cache evicts and replaces
    // files the memo never hears of, so past the cap it starts over, and
    // each path costs one header read again.
    if kept.len() >= MEMO_CAP && !kept.contains_key(path) {
        kept.clear();
    }
    kept.insert(
        path.to_path_buf(),
        Seen {
            len,
            modified,
            width,
        },
    );
    width
}

/// Tiny headers in each format, for the tests here and the series page's.
#[doc(hidden)]
pub mod testing {
    /// A PNG signature and IHDR chunk, which is all `width` reads.
    pub fn png(width: u32, height: u32) -> Vec<u8> {
        let mut out = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
        out.extend(width.to_be_bytes());
        out.extend(height.to_be_bytes());
        out.extend([8, 2, 0, 0, 0, 0, 0, 0, 0]);
        out
    }

    /// A JPEG with an APP0 segment ahead of its baseline frame header.
    pub fn jpeg(width: u16, height: u16) -> Vec<u8> {
        let mut out = vec![0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10];
        out.extend(b"JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00");
        out.extend([0xff, 0xc0, 0x00, 0x11, 0x08]);
        out.extend(height.to_be_bytes());
        out.extend(width.to_be_bytes());
        out.extend([3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        out.extend([0xff, 0xd9]);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{jpeg, png};
    use super::*;

    #[test]
    fn png_and_jpeg_say_their_width() {
        assert_eq!(width(&png(1920, 1080)), Some(1920));
        assert_eq!(width(&jpeg(1900, 400)), Some(1900));
    }

    #[test]
    fn a_jpeg_frame_after_large_metadata_is_found() {
        let mut bytes = vec![0xff, 0xd8];
        // An EXIF segment and a colour profile ahead of the frame.
        for marker in [0xe1, 0xe2] {
            bytes.extend([0xff, marker, 0xff, 0xf0]);
            bytes.extend(std::iter::repeat_n(0u8, 0xfff0 - 2));
        }
        // A progressive frame header this time.
        bytes.extend([0xff, 0xc2, 0x00, 0x11, 0x08, 0x02, 0x1c, 0x0f, 0x00]);
        assert_eq!(width(&bytes), Some(3840));
    }

    #[test]
    fn gif_and_webp_say_their_width() {
        assert_eq!(width(b"GIF89a\x40\x06\x84\x03"), Some(1600));
        let mut lossy = b"RIFF\x00\x00\x00\x00WEBPVP8 \x00\x00\x00\x00".to_vec();
        lossy.extend([0x30, 0x01, 0x00, 0x9d, 0x01, 0x2a, 0x80, 0x07, 0x38, 0x04]);
        assert_eq!(width(&lossy), Some(1920));
        let mut extended = b"RIFF\x00\x00\x00\x00WEBPVP8X\x0a\x00\x00\x00".to_vec();
        extended.extend([0, 0, 0, 0, 0x7f, 0x07, 0x00, 0x37, 0x04, 0x00]);
        assert_eq!(width(&extended), Some(1920));
    }

    #[test]
    fn anything_else_has_no_width() {
        assert_eq!(width(b"image"), None);
        assert_eq!(width(&[]), None);
        // A header cut off before the frame.
        assert_eq!(width(&jpeg(1920, 1080)[..20]), None);
        assert_eq!(width(&png(1920, 1080)[..18]), None);
    }

    #[test]
    fn a_file_is_read_once_and_reread_when_it_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("banner.jpg");
        std::fs::write(&path, jpeg(1900, 400)).unwrap();
        let stamp = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(width_of(&path), Some(1900));
        // Other bytes of the same length under the same stamp: the memo
        // answers, so the file was not read again.
        std::fs::write(&path, jpeg(1200, 400)).unwrap();
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(stamp).unwrap();
        drop(file);
        assert_eq!(width_of(&path), Some(1900));
        // A new stamp is a new file.
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(stamp + std::time::Duration::from_secs(60))
            .unwrap();
        drop(file);
        assert_eq!(width_of(&path), Some(1200));
        // So is a new length.
        std::fs::write(&path, png(640, 360)).unwrap();
        assert_eq!(width_of(&path), Some(640));
        assert_eq!(width_of(&dir.path().join("gone.jpg")), None);
    }

    #[test]
    fn a_jpeg_frame_far_past_any_buffer_is_reached_by_seeking() {
        // Ten 64 KB segments, over 600 KB of metadata ahead of the frame.
        let mut bytes = vec![0xff, 0xd8];
        for _ in 0..10 {
            bytes.extend([0xff, 0xe1, 0xff, 0xff]);
            bytes.extend(std::iter::repeat_n(0u8, 0xffff - 2));
        }
        bytes.extend([0xff, 0xc0, 0x00, 0x11, 0x08, 0x04, 0x38, 0x07, 0x80]);
        assert!(bytes.len() > 600 * 1024);
        assert_eq!(width(&bytes), Some(1920));
    }
}
