//! What a release title says that anirss does not split out: the episode
//! (and season), whether it is a batch, the release group, the resolution,
//! and the size in bytes for sorting. Release names follow a few house
//! styles, and this reads the common ones and nothing cleverer:
//!
//! - fansub: `[Lumen] Sousou no Frieren - 01 (1080p) [ABCD1234].mkv`
//! - tagged: `[Kumo-Subs] Sousou no Frieren - 28 [1080p CR WEB-DL AVC AAC][MultiSub]`
//! - scene: `Avatar Seven Havens S01E03 1080p WEB h264-ETHEL`
//! - batches: `[Lumen] Frieren (01-28) (1080p) [Batch]`, `Show S01 1080p`
//!
//! The library's file classifier is not used here: a file with no number
//! in it is episode 1 to the library, which is right for a film on disk
//! and wrong for a release list, where it would mark a batch or a film as
//! episode 1. A title this cannot read gets no episode at all.

use std::sync::LazyLock;

use regex::Regex;

use anibeam_contract::*;
use anibeam_foundation::size::size_bytes;

static GROUP_LEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*\[([^\]]+)\]").unwrap());
/// A scene group at the end: `h264-ETHEL`, `x265-NeoNoir[Uploader.example]`
static GROUP_TAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"-([A-Za-z0-9]{2,20})(?:\[[^\]]*\])?(?:\.[a-z0-9]{2,4})?\s*$").unwrap()
});
static RESOLUTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(4320|2160|1440|1080|720|576|540|480|360)[pi]\b").unwrap());
static FOUR_K: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(4k|uhd)\b").unwrap());
static SEASON_EPISODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bS(\d{1,2})\s?E(\d{1,4}(?:\.\d)?)(?:v\d)?\b").unwrap());
/// The fansub convention: ` - 01`, ` - 12.5`, ` - 03v2`
static DASH_EPISODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s-\s(\d{1,4}(?:\.\d)?)(?:v\d)?(?:\s|$|[\[(.])").unwrap());
static WORD_EPISODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(?:ep|episode|e)\s?\.?\s?(\d{1,4})\b").unwrap());
/// `(01-28)`, `01 ~ 12`, `E01-E12`
static RANGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:^|[\s(\[_])(?:e|ep)?(\d{1,4})\s?[-~]\s?(?:e|ep)?(\d{1,4})(?:$|[\s)\]_v])")
        .unwrap()
});
static BATCH_WORD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(batch|complete series|complete season|season pack)\b").unwrap()
});
/// A season with no episode after it: `S01 1080p`, `S02.1080p`
pub(crate) static SEASON_PACK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bS(\d{1,2})(?:[\s._\[(]|$)").unwrap());
/// A season a title, or a show's name, states in words or as a season code outside an
/// SxxEyy marker: `Season 04`, `4th Season`, `S2`. Its word boundaries are ASCII ones, as
/// JavaScript's are: `GetShow.qml` reads show names with the same pattern.
static SEASON_WORD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?-u:\b)(?:season\s*([0-9]{1,2})|([0-9]{1,2})(?:st|nd|rd|th)\s+season|s([0-9]{1,2}))(?-u:\b)",
    )
    .unwrap()
});
/// What follows the near end of a span of seasons: `-4`, ` - S04`, ` ~ 3`, `+S2`, ` & 2`,
/// ` and 2`. Its groups: the space before the joiner, the joiner (none for `and`), the
/// space after it, a season word or code on the far end, the far end's number.
static SEASON_SPAN_NEXT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(\s*)(?:([-~+&])|and\s)(\s*)((?:season|s)\s*)?([0-9]{1,2})(?:[^0-9]|$)")
        .unwrap()
});
/// What comes before the far end of a span of seasons: `S01 - `, `Season 1 & `.
static SEASON_SPAN_BEFORE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:season\s*|(?-u:\b)s)[0-9]{1,2}\s*(?:[-~+&]|and\s)\s*$").unwrap()
});
/// A season named in words, which takes no number after it: `The Final Season 05` is the
/// fifth episode of the final season, never season 5.
static NAMED_SEASON: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?-u:\b)(?:final|last)\s+$").unwrap());
/// `Complete` before a season word: `Complete Season 2` is season 2, but a range after it
/// is episodes, `Complete Season 01-12`.
static COMPLETE_BEFORE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?-u:\b)complete\s+$").unwrap());
/// A season word with its season said before it, so a range after it is episodes:
/// `The Final Season 01-16`, `2nd Season 01-12`, `Complete Season 01-12`.
static NAMED_BEFORE_RANGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?-u:\b)(?:final|complete|last|[0-9]{1,2}(?:st|nd|rd|th))\s+season$").unwrap()
});
/// Where the video came from: `WEB-DL`, `WEBRip`, `WEB`, `BluRay`, `BDRip`, `HDTV`, `DVD`
static RIP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:^|[^a-z0-9])(web[-. ]?dl|web[-. ]?rip|web|blu[-. ]?ray|bd[-. ]?rip|bd|hdtv|dvd[-. ]?rip|dvd)(?:$|[^a-z0-9])").unwrap()
});
/// A version after the episode number: ` - 03v2`, `S01E03v2`
static VERSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:\bS\d{1,2}\s?E|\s-\s)\d{1,4}(?:\.\d)?v(\d)\b").unwrap());
/// The CRC32 a fansub group tags its file with: `[80977B2B]`. An all-digit tag
/// is read as a date (`[20241005]`), not a CRC.
static CRC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[([0-9A-Fa-f]{8})\]").unwrap());

/// The facts of a bare name, a file's or a torrent's, read as a release
/// title is; a path counts by its last part.
pub fn read_name(name: &str) -> ReleaseFacts {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    read(&crate::titled(base))
}

/// What a title, or a show's name, says of its season outside an SxxEyy marker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SeasonSaid {
    Nothing,
    /// One season; `words` when it is written out (`Season 4`, `4th Season`) rather than
    /// as a code (`S4`).
    One {
        season: u32,
        words: bool,
    },
    /// A span or a pack of seasons: `Season 1-4`, `S01 - S04`, `Season 1 ~ 3`, `S1+S2`,
    /// `Season 1 & 2`. It names no one season.
    Span,
}

/// What `title` says of its season (`SEASON_WORD`): the first season it states, unless
/// that is the near end of a span. A spaced dash and a padded number after it is an
/// episode (`Season 2 - 03`); one unpadded digit past a season written in words is a
/// span's far end (`Season 1 - 4`), but past a code it is the episode (`S2 - 7`).
/// `GetShow.qml` `seasonSaid` reads it the same way.
pub(crate) fn season_said(title: &str) -> SeasonSaid {
    for c in SEASON_WORD.captures_iter(title) {
        let Some(whole) = c.get(0) else { continue };
        let (number, words) = match (c.get(1), c.get(2), c.get(3)) {
            (Some(n), _, _) | (None, Some(n), _) => (n, true),
            (None, None, Some(n)) => (n, false),
            _ => continue,
        };
        let before = &title[..whole.start()];
        if c.get(1).is_some() && NAMED_SEASON.is_match(before) {
            continue;
        }
        if SEASON_SPAN_BEFORE.is_match(before) {
            continue;
        }
        let after = &title[whole.end()..];
        if c.get(1).is_some()
            && COMPLETE_BEFORE.is_match(before)
            && SEASON_SPAN_NEXT.is_match(after)
        {
            continue;
        }
        let Ok(season) = number.as_str().parse::<u32>() else {
            continue;
        };
        if let Some(n) = SEASON_SPAN_NEXT.captures(after) {
            let far = &n[5];
            let spaced = !n[1].is_empty() || !n[3].is_empty();
            let span = n.get(4).is_some()
                || n.get(2).is_none_or(|j| j.as_str() != "-")
                || !spaced
                || (words && far.len() == 1 && far.parse::<u32>().is_ok_and(|f| f > season));
            if span {
                return SeasonSaid::Span;
            }
        }
        return SeasonSaid::One { season, words };
    }
    SeasonSaid::Nothing
}

/// The one season a title, or a show's name, states (`season_said`), when it states one,
/// in words or as a code. `grid` reads a show's names with it, and a release's season when
/// its facts carry none: `ReleaseFacts::season` keeps to codes.
pub(crate) fn season_word(title: &str) -> Option<u32> {
    match season_said(title) {
        SeasonSaid::One { season, .. } => Some(season),
        _ => None,
    }
}

/// Whether the season a release title states is written out in words (`Season 04`,
/// `4th Season`), as AniList's names number seasons, rather than as a code (`S04E01`,
/// `S2`), which follows the TV listings and may number them otherwise.
pub(crate) fn season_in_words(title: &str) -> bool {
    !SEASON_EPISODE.is_match(title)
        && matches!(season_said(title), SeasonSaid::One { words: true, .. })
}

pub fn read(item: &Release) -> ReleaseFacts {
    let title = item.title.as_str();
    let group = GROUP_LEAD
        .captures(title)
        .map(|c| c[1].trim().to_string())
        .or_else(|| {
            // A scene name has no brackets up front; its group trails it
            (!title.trim_start().starts_with('[') && SEASON_EPISODE.is_match(title)
                || SEASON_PACK.is_match(title))
            .then(|| GROUP_TAIL.captures(title).map(|c| c[1].to_string()))
            .flatten()
        })
        .unwrap_or_default();
    let resolution = if !item.resolution.trim().is_empty() {
        item.resolution.trim().to_string()
    } else if let Some(c) = RESOLUTION.captures(title) {
        format!("{}p", &c[1])
    } else if FOUR_K.is_match(title) {
        "2160p".to_string()
    } else {
        String::new()
    };

    let mut season = None;
    let mut episode = None;
    if let Some(c) = SEASON_EPISODE.captures(title) {
        season = c[1].parse().ok();
        episode = c[2].parse().ok();
    }
    let range = if episode.is_none() { span(title) } else { None };
    let said = season_said(title);
    // Only a code: the downloads match files on disk by this season, numbered as the TV
    // listings number them, and a season written out follows AniList's names, which may
    // number it otherwise (`Kimetsu no Yaiba Season 2 - 05` is the listings' S03E05).
    // `grid::of_show` reads the written one itself.
    if let SeasonSaid::One {
        season: s,
        words: false,
    } = said
    {
        season = season.or(Some(s));
    }
    let batch = episode.is_none()
        && (BATCH_WORD.is_match(title)
            // A span of seasons is a pack of them
            || said == SeasonSaid::Span
            || range.is_some()
            // "Show S2 - 03" names a season and then its episode: not a pack
            || (!DASH_EPISODE.is_match(title)
                && SEASON_PACK
                    .captures(title)
                    // A code season_said leaves out ("S02_1080p")
                    .inspect(|c| {
                        if season.is_none() {
                            season = c[1].parse().ok();
                        }
                    })
                    .is_some()));
    if !batch && episode.is_none() {
        episode = DASH_EPISODE
            .captures(title)
            .and_then(|c| c[1].parse().ok())
            .or_else(|| WORD_EPISODE.captures(title).and_then(|c| c[1].parse().ok()))
            .filter(|n: &f64| *n < 2000.0 || *n > 2099.0);
    }
    let span = range.filter(|_| batch);
    ReleaseFacts {
        episode,
        season,
        batch,
        first_episode: span.map(|s| s.0),
        last_episode: span.map(|s| s.1),
        group,
        resolution,
        size_bytes: size_bytes(&item.size),
        rip: rip(title),
        on_disk: false,
        other_on_disk: false,
    }
}

/// The rip a title names, in its usual spelling; empty when none.
/// The last one counts: a show's own name comes first and can hold the word
/// (`The Web Detective`), the release tags come after it.
pub fn rip(title: &str) -> String {
    let Some(c) = RIP.captures_iter(title).last() else {
        return String::new();
    };
    let word: String = c[1]
        .to_ascii_lowercase()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    match word.as_str() {
        "webdl" => "WEB-DL",
        "webrip" => "WEBRip",
        "web" => "WEB",
        "bluray" | "bd" => "BluRay",
        "bdrip" => "BDRip",
        "hdtv" => "HDTV",
        "dvdrip" => "DVDRip",
        _ => "DVD",
    }
    .to_string()
}

/// The release's version of its episode: 2 for ` - 03v2`, 1 when it names none.
pub fn version(title: &str) -> u32 {
    VERSION
        .captures(title)
        .and_then(|c| c[1].parse().ok())
        .unwrap_or(1)
}

/// A name as compared: no folders, no video extension, lower case, and dots,
/// underscores and runs of spaces as one space.
fn plain(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let lower = base.to_lowercase();
    let stem = anibeam_foundation::names::without_video_extension(&lower);
    stem.replace(['.', '_'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether the file `file` is the release `title` (#237), `facts` being what
/// was read off that release: the same name, or, when a torrent named itself
/// apart from its file, the same group, resolution, rip and version, each
/// named by both or by neither. Two CRC tags settle it either way. A release
/// naming no group is only ever its own name.
pub fn same_release(title: &str, facts: &ReleaseFacts, file: &str) -> bool {
    if plain(title) == plain(file) {
        return true;
    }
    let crc = |s: &str| {
        CRC.captures_iter(s)
            .map(|c| c[1].to_ascii_uppercase())
            .filter(|c| c.chars().any(|ch| ch.is_ascii_alphabetic()))
            .last()
    };
    if let (Some(a), Some(b)) = (crc(title), crc(file)) {
        return a == b;
    }
    let f = read_name(file);
    !facts.group.is_empty()
        && facts.group.eq_ignore_ascii_case(&f.group)
        && facts.resolution.eq_ignore_ascii_case(&f.resolution)
        && facts.rip == f.rip
        && version(title) == version(file)
}

/// The first range of episodes a title names, as first and last: not a year span or a
/// resolution pair. A pair of numbers right after a season word is seasons, or a season
/// and then an episode: `Season 1-4`, `Season 2 - 03`. After a season already named they
/// are episodes: `The Final Season 01-16`, `2nd Season 01-12`.
fn span(title: &str) -> Option<(u32, u32)> {
    RANGE.captures_iter(title).find_map(|c| {
        let before = title[..c.get(0)?.start()].trim_end();
        if before.to_ascii_lowercase().ends_with("season") && !NAMED_BEFORE_RANGE.is_match(before) {
            return None;
        }
        let (a, b) = (
            c[1].parse::<u32>().unwrap_or(0),
            c[2].parse::<u32>().unwrap_or(0),
        );
        (a < b && b < 1900 && b - a < 500).then_some((a, b))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(title: &str, size: &str) -> Release {
        serde_json::from_value(serde_json::json!({"title": title, "link": "x", "size": size}))
            .unwrap()
    }

    #[test]
    fn fansub_tag_and_scene_names_read() {
        let f = read(&release(
            "[Lumen] Sousou no Frieren - 01 (1080p) [ABCD1234].mkv",
            "1.4 GiB",
        ));
        assert_eq!((f.episode, f.season, f.batch), (Some(1.0), None, false));
        assert_eq!(
            (f.group.as_str(), f.resolution.as_str()),
            ("Lumen", "1080p")
        );
        assert_eq!(f.size_bytes, Some(1_503_238_554));

        let f = read(&release(
            "[Kumo-Subs] Sousou no Frieren - 28 [1080p CR WEB-DL AVC AAC][MultiSub][D2E8BC01]",
            "",
        ));
        assert_eq!(
            (f.episode, f.group.as_str(), f.resolution.as_str()),
            (Some(28.0), "Kumo-Subs", "1080p")
        );

        let f = read(&release(
            "[Lumen] Kusuriya no Hitorigoto - 12.5 (720p) [F00].mkv",
            "",
        ));
        assert_eq!(f.episode, Some(12.5));

        let f = read(&release("[ASW] Show - 03v2 [1080p HEVC][AAAA].mkv", ""));
        assert_eq!(f.episode, Some(3.0));

        let f = read(&release(
            "Avatar Seven Havens S01E03 1080p WEB h264-ETHEL",
            "1.5 GB",
        ));
        assert_eq!((f.season, f.episode, f.batch), (Some(1), Some(3.0), false));
        assert_eq!(
            (f.group.as_str(), f.resolution.as_str()),
            ("ETHEL", "1080p")
        );
    }

    #[test]
    fn a_season_then_a_dashed_episode_is_one_episode() {
        let f = read(&release(
            "[Naruto-Kun.Hu] Black Clover S2 - 03 [1080p].mkv",
            "",
        ));
        assert_eq!((f.episode, f.batch), (Some(3.0), false));
    }

    #[test]
    fn a_season_word_names_the_season() {
        // The Judas batch of the show's fourth season, as its own title words it
        let t = "[Judas] Re:Zero kara Hajimeru Isekai Seikatsu (Season 04) [1080p][Batch]";
        let f = read(&release(t, ""));
        assert_eq!((f.season, f.batch, f.episode), (None, true, None));
        assert_eq!(season_word(t), Some(4));
        let t = "[Grp] Show 4th Season - 03 [1080p]";
        let f = read(&release(t, ""));
        assert_eq!((f.season, f.episode, f.batch), (None, Some(3.0), false));
        assert_eq!(season_word(t), Some(4));
        let t = "[Grp] Show Season 2 - 03 [1080p]";
        let f = read(&release(t, ""));
        assert_eq!((f.season, f.episode, f.batch), (None, Some(3.0), false));
        assert_eq!(season_word(t), Some(2));
        assert_eq!(read(&release("Show S02E05 [1080p]", "")).season, Some(2));
        assert_eq!(read(&release("Show S2 - 03 [1080p]", "")).season, Some(2));
        // A span of seasons, a year and a plain episode name no season
        let f = read(&release(
            "[Judas] Black Clover (Season 1-4) [1080p][Batch]",
            "",
        ));
        assert_eq!((f.season, f.batch), (None, true));
        let f = read(&release("Show S01-S04 [1080p]", ""));
        assert_eq!((f.season, f.batch), (None, true));
        assert_eq!(read(&release("Show Season 2020 [1080p]", "")).season, None);
        assert_eq!(read(&release("Plain Show - 05", "")).season, None);
    }

    #[test]
    fn a_span_of_seasons_names_none_and_is_a_pack() {
        for t in [
            "Show S01 - S04 [1080p]",
            "Show S01 ~ S03 [1080p]",
            "Show (Season 1 - 4) [1080p]",
            "Show Season 1 - Season 4 [1080p]",
            "Show Season 1 ~ 3 [1080p]",
            "Show S1+S2 [1080p]",
            "Show S1 + S2 + S3 [1080p]",
            "Show Season 1 & 2 [1080p]",
            "Show Season 1 and 2 [1080p]",
        ] {
            assert_eq!(season_said(t), SeasonSaid::Span, "{t}");
            let f = read(&release(t, ""));
            assert_eq!((f.season, f.batch, f.episode), (None, true, None), "{t}");
        }
        // A season, then its episode
        for t in [
            "[Grp] Show Season 2 - 03 [1080p]",
            "[Grp] Show S2 - 03 [1080p]",
            "[Grp] Show Season 2 - 13 [1080p]",
        ] {
            let f = read(&release(t, ""));
            assert_eq!(season_word(t), Some(2), "{t}");
            assert!(!f.batch && f.episode.is_some(), "{t}");
        }
        // A dash before a season leads it in, it does not end a span
        let t = "[Grp] Re Zero - Season 2 - 03 [1080p]";
        let f = read(&release(t, ""));
        assert_eq!((season_word(t), f.episode), (Some(2), Some(3.0)));
    }

    #[test]
    fn a_season_named_in_words_keeps_its_episodes() {
        let f = read(&release(
            "[Grp] Shingeki no Kyojin The Final Season 01-16 [Batch]",
            "",
        ));
        assert_eq!(
            (f.season, f.batch, f.first_episode, f.last_episode),
            (None, true, Some(1), Some(16))
        );
        let t = "[Grp] Show 2nd Season 01-12 [Batch]";
        let f = read(&release(t, ""));
        assert_eq!(
            (season_word(t), f.batch, f.first_episode, f.last_episode),
            (Some(2), true, Some(1), Some(12))
        );
        let t = "[Grp] Show Complete Season 01-12 [1080p]";
        let f = read(&release(t, ""));
        assert_eq!(
            (season_said(t), f.batch, f.first_episode, f.last_episode),
            (SeasonSaid::Nothing, true, Some(1), Some(12))
        );
        // Complete is no season's name: its number is the season
        let t = "[Grp] Show Complete Season 2 [1080p]";
        let f = read(&release(t, ""));
        assert_eq!((season_word(t), f.batch, f.episode), (Some(2), true, None));
        assert!(season_in_words(t));
        // Its episode is no season
        let f = read(&release(
            "Shingeki no Kyojin The Final Season 05 [1080p]",
            "",
        ));
        assert_eq!(f.season, None);
        let f = read(&release(
            "[Grp] Shingeki no Kyojin The Final Season - 05 [1080p]",
            "",
        ));
        assert_eq!((f.season, f.episode), (None, Some(5.0)));
    }

    #[test]
    fn a_season_code_and_a_dashed_episode_are_no_span() {
        // A bare code, a spaced dash and an unpadded episode: season 2, episode 7
        for (t, season, episode) in [
            ("[Grp] Show S2 - 7 [720p]", 2, 7.0),
            ("Show S1 - 5", 1, 5.0),
        ] {
            assert_eq!(
                season_said(t),
                SeasonSaid::One {
                    season,
                    words: false
                },
                "{t}"
            );
            let f = read(&release(t, ""));
            assert_eq!(
                (f.season, f.episode, f.batch),
                (Some(season), Some(episode), false),
                "{t}"
            );
        }
        // Written out, or with a season on the far end, it is a span
        for t in [
            "Show Season 1 - 4",
            "Show S1 - S4",
            "Show S1-S4",
            "Show S01 - S04",
        ] {
            assert_eq!(season_said(t), SeasonSaid::Span, "{t}");
        }
    }

    #[test]
    fn a_season_written_out_stays_out_of_the_facts() {
        // The downloads match files on disk by the facts' season, which the TV listings
        // number: AniList's "Season 2" of Kimetsu no Yaiba is the listings' third
        let t = "Kimetsu no Yaiba Season 2 - 05 [1080p]";
        let f = read(&release(t, ""));
        assert_eq!((f.season, f.episode), (None, Some(5.0)));
        assert_eq!(season_word(t), Some(2));
        // A code still sets it
        assert_eq!(
            read(&release("Kimetsu no Yaiba S03E05 [1080p]", "")).season,
            Some(3)
        );
    }

    #[test]
    fn a_season_written_out_is_told_from_a_code() {
        assert!(season_in_words("[Judas] Show (Season 04) [Batch]"));
        assert!(season_in_words("[Grp] Show 4th Season - 03"));
        assert!(!season_in_words("[Grp] Show S2 - 03"));
        assert!(!season_in_words("Show S02E05 1080p WEB h264-GRP"));
        assert!(!season_in_words("Show Season 2 S02E05 1080p"));
        assert!(!season_in_words("Show S01 - S04 [Batch]"));
        assert_eq!(season_word("Fate/Zero 2nd Season"), Some(2));
        assert_eq!(
            season_word("Re:ZERO -Starting Life in Another World- Season 4"),
            Some(4)
        );
        assert_eq!(season_word("Shingeki no Kyojin: The Final Season"), None);
        // A code season_said cannot bound still reads off the pack
        let f = read(&release("Show S02_1080p", ""));
        assert_eq!((f.season, f.batch), (Some(2), true));
        // Words inside other words are none
        assert_eq!(season_said("Preseason 2 Show - 01"), SeasonSaid::Nothing);
        assert_eq!(season_said("Show Seasons 2 - 01"), SeasonSaid::Nothing);
    }

    #[test]
    fn batches_carry_no_episode() {
        let f = read(&release(
            "[Lumen] Sousou no Frieren (01-28) (1080p) [Batch]",
            "",
        ));
        assert!(f.batch);
        assert_eq!(f.episode, None);
        assert_eq!((f.first_episode, f.last_episode), (Some(1), Some(28)));
        let f = read(&release(
            "[Kumo-Subs] Kino no Tabi: The Beautiful World - The Animated Series - 01 ~ 12 [1080p][Batch]",
            "",
        ));
        assert!(f.batch);
        assert_eq!((f.first_episode, f.last_episode), (Some(1), Some(12)));
        let f = read(&release(
            "Avatar Seven Havens S01 1080p WEB-DL x264-GROUP",
            "",
        ));
        assert!(f.batch);
        assert_eq!((f.season, f.episode), (Some(1), None));
        assert_eq!(f.first_episode, None);
        let f = read(&release("[Judas] Frieren - Complete Season [1080p]", ""));
        assert!(f.batch);
    }

    #[test]
    fn a_title_with_no_episode_gets_none_not_one() {
        let f = read(&release("Perfect Blue (1997) [BD 1080p HEVC]", ""));
        assert_eq!((f.episode, f.batch), (None, false));
        assert_eq!(f.resolution, "1080p");
        let f = read(&release("Some Film 2160p UHD BluRay", ""));
        assert_eq!(f.resolution, "2160p");
    }

    #[test]
    fn rips_read_in_their_usual_spelling() {
        assert_eq!(
            rip("[Kumo-Subs] Show - 28 [1080p CR WEB-DL AVC AAC]"),
            "WEB-DL"
        );
        assert_eq!(rip("Show S01E03 1080p WEBRip x265-Group"), "WEBRip");
        assert_eq!(rip("Show.S01E03.1080p.WEB.h264-ETHEL"), "WEB");
        assert_eq!(rip("[Group] Show - 01 (BD 1080p)"), "BluRay");
        assert_eq!(rip("[Lumen] Show - 01 (1080p) [ABCD1234].mkv"), "");
    }

    #[test]
    fn only_the_same_release_is_the_file_on_disk() {
        let file = "/lib/Show/[Lumen] Show - 05 (1080p) [ABCD1234].mkv";
        assert!(same_release(
            "[Lumen] Show - 05 (1080p) [ABCD1234].mkv",
            &read_name("[Lumen] Show - 05 (1080p) [ABCD1234].mkv"),
            file
        ));
        // The same group and resolution, named apart: still this release
        assert!(same_release(
            "[Lumen] Show - 05 (1080p)",
            &read_name("[Lumen] Show - 05 (1080p)"),
            file
        ));
        // Another group, another resolution, another version: not it
        assert!(!same_release(
            "[Kumo-Subs] Show - 05 [1080p CR WEB-DL AVC AAC]",
            &read_name("[Kumo-Subs] Show - 05 [1080p CR WEB-DL AVC AAC]"),
            file
        ));
        assert!(!same_release(
            "[Lumen] Show - 05 (720p) [EEEE0000].mkv",
            &read_name("[Lumen] Show - 05 (720p) [EEEE0000].mkv"),
            file
        ));
        assert!(!same_release(
            "[Lumen] Show - 05v2 (1080p) [FFFF0000].mkv",
            &read_name("[Lumen] Show - 05v2 (1080p) [FFFF0000].mkv"),
            file
        ));
        assert!(same_release(
            "Show.S01E05.1080p.WEB.h264-ETHEL",
            &read_name("Show.S01E05.1080p.WEB.h264-ETHEL"),
            "Show S01E05 1080p WEB h264-ETHEL.mkv"
        ));
        assert!(!same_release(
            "Show S01E05 1080p WEBRip x265-Other",
            &read_name("Show S01E05 1080p WEBRip x265-Other"),
            "Show S01E05 1080p WEB h264-ETHEL.mkv"
        ));
    }

    #[test]
    fn kumo_subs_releases_of_one_episode_are_told_apart() {
        // The owner's file, and what the source listed for that episode (#237)
        let file = "/a/[Kumo-Subs] Katainaka no Ossan Kensei ni Naru II - 06 [1080p AMZN WEB-DL AVC EAC3][MultiSub][80977B2B].mkv";
        let listed = [
            (
                "[Kumo-Subs] Katainaka no Ossan, Kensei ni Naru II - 06 [1080p AMZN WEBRip HEVC EAC3][MultiSub][C781A1C2]",
                false,
            ),
            (
                "[Kumo-Subs] Katainaka no Ossan, Kensei ni Naru II - 06 [540p AMZN WEB-DL AVC EAC3][MultiSub][8B85B88D]",
                false,
            ),
            (
                "[Kumo-Subs] Katainaka no Ossan, Kensei ni Naru II - 06 [720p AMZN WEB-DL AVC EAC3][MultiSub][19163F35]",
                false,
            ),
            (
                "[Kumo-Subs] Katainaka no Ossan, Kensei ni Naru II - 06 [1080p AMZN WEB-DL AVC EAC3][MultiSub][80977B2B]",
                true,
            ),
        ];
        for (title, here) in listed {
            assert_eq!(
                same_release(title, &read_name(title), file),
                here,
                "{title}"
            );
        }
    }

    #[test]
    fn a_release_naming_less_than_the_file_is_not_it() {
        let file = "[Lumen] Show - 05 (720p) [ABCD1234].mkv";
        let bare = "[Lumen] Show - 05";
        assert!(!same_release(bare, &read_name(bare), file));
        // A date tag is no CRC: the group and the rest decide
        let dated = "[Lumen] Show - 05 (720p) [20241005]";
        assert!(same_release(dated, &read_name(dated), file));
        assert_eq!(
            rip("[Group] The Web Detective - 03 [1080p BluRay]"),
            "BluRay"
        );
        assert_eq!(
            read_name("C:\\Anime\\Show\\[Lumen] Show - 05 (1080p).mkv").group,
            "Lumen"
        );
    }
}
