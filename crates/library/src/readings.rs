//! What a file is read as beyond its name. The walk reads every name on its
//! own; two things it cannot see change that reading before the scan writes
//! it (#167, #170):
//!
//! - The series' own record. A series matched to a run of specials or OVAs
//!   (AniList format SPECIAL or OVA) that holds no regular episode reads its
//!   numbered specials as its episodes: `OVA 3` of the Girls und Panzer OVA
//!   Series is that record's episode 3, not a special of some other show.
//!   A film record (format MOVIE, one episode) over a folder holding one
//!   file reads that file as its episode 1, whatever number its name
//!   carries (#1479).
//! - The user's quick fixes, stored by path in `file_fixes`, which win over
//!   both the name and the record.

use std::collections::HashMap;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use crate::classifier::{self, Classified};
use crate::labels;
use crate::walk::ScannedFile;
use anibeam_contract::*;

/// What the readings need of the series' AniList record.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct Record {
    /// `SPECIAL`, `TV`, `MOVIE`.
    pub format: Option<String>,
    /// The episode count, None while airing with no count.
    pub episodes: Option<u32>,
}

/// The record formats whose numbered specials are the series' own episodes.
fn specials_are_episodes(format: Option<&str>) -> bool {
    matches!(format, Some("SPECIAL" | "OVA"))
}

fn numbered_special(c: &Classified) -> bool {
    c.extra == Some(ExtraKind::Sp) && c.extra_index.is_some()
}

/// Whether the record turns this series' numbered specials into its
/// episodes: the record is a run of specials or OVAs, a file reads as a
/// numbered special, and none reads as a regular episode.
fn record_rule_applies(format: Option<&str>, files: &[&Classified]) -> bool {
    specials_are_episodes(format)
        && files.iter().all(|c| c.extra.is_some())
        && files.iter().any(|c| numbered_special(c))
}

/// A name the film rule reads past: an episode or a special, other than the
/// first episode. An opening or a preview stays what its name says.
fn reads_past_film(c: &Classified) -> bool {
    let first = c.extra.is_none() && c.number == 1.0 && c.season.is_none_or(|s| s <= 1);
    !first && matches!(c.extra, None | Some(ExtraKind::Sp))
}

/// Whether the record makes this series' one file its film: a folder (a
/// loose film file keeps its name as its key), a record of format MOVIE with
/// one episode, one file, and that file read as some other episode or a
/// special. `Dai 63-kai ... - Movie` is the film, not episode 63 (#1479).
fn film_rule_applies(kind: SeriesKind, record: &Record, files: &[&Classified]) -> bool {
    kind == SeriesKind::Show
        && record.format.as_deref() == Some("MOVIE")
        && record.episodes == Some(1)
        && matches!(files, [only] if reads_past_film(only))
}

/// One series' walked files, read past their names: the record rules first,
/// then the fixes, keyed by path.
pub(crate) fn apply(
    kind: SeriesKind,
    record: &Record,
    files: &mut [ScannedFile],
    fixes: &HashMap<String, FileReading>,
) {
    let named: Vec<&Classified> = files.iter().map(|f| &f.classified).collect();
    let film = film_rule_applies(kind, record, &named);
    if film {
        set(
            &mut files[0],
            &FileReading::Episode {
                number: 1.0,
                season: None,
            },
        );
    } else if record_rule_applies(record.format.as_deref(), &named) {
        for f in files.iter_mut().filter(|f| numbered_special(&f.classified)) {
            let number = f64::from(f.classified.extra_index.unwrap_or(0));
            set(
                f,
                &FileReading::Episode {
                    number,
                    season: None,
                },
            );
        }
    }
    for f in files.iter_mut() {
        if let Some(reading) = fixes.get(&f.path) {
            set(f, reading);
        }
    }
}

fn set(f: &mut ScannedFile, reading: &FileReading) {
    let (classified, label, key) = parts(reading, &file_name(&f.path));
    f.classified = classified;
    f.label = label;
    f.episode_key = key;
}

/// What a reading writes for a file of this name: the classification, the
/// row's label and the history key, the way the walk writes its own.
pub(crate) fn parts(reading: &FileReading, name: &str) -> (Classified, String, String) {
    match *reading {
        FileReading::Episode { number, season } => (
            Classified {
                extra: None,
                number,
                season,
                extra_index: None,
                extra_variant: None,
                raw_label: None,
            },
            classifier::clean_episode_title(name),
            classifier::number_key(season, number),
        ),
        FileReading::Extra { kind, index } => (
            Classified {
                extra: Some(kind),
                number: f64::from(index.unwrap_or(0)),
                season: (kind == ExtraKind::Sp).then_some(0),
                extra_index: index,
                extra_variant: None,
                raw_label: None,
            },
            labels::extra_label(kind, index, None, None),
            name.to_string(),
        ),
    }
}

pub(crate) fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The reading a stored file row carries.
pub(crate) fn of_row(
    kind: &str,
    number: Option<f64>,
    season: Option<u32>,
    extra_kind: Option<&str>,
    extra_index: Option<u32>,
) -> FileReading {
    if kind == "extra" {
        FileReading::Extra {
            kind: extra_kind
                .and_then(ExtraKind::from_column)
                .unwrap_or(ExtraKind::Other),
            index: extra_index,
        }
    } else {
        FileReading::Episode {
            number: number.unwrap_or(0.0),
            season,
        }
    }
}

/// A reading in words: `Episode 3`, `Season 2 episode 5`, `Opening 1`,
/// `Special`.
pub(crate) fn describe(reading: &FileReading) -> String {
    match *reading {
        FileReading::Episode {
            number,
            season: None | Some(1),
        } => format!("Episode {}", classifier::format_number(number)),
        FileReading::Episode {
            number,
            season: Some(s),
        } => format!("Season {s} episode {}", classifier::format_number(number)),
        // The sheet's word for the last kind, not the label's "Bonus".
        FileReading::Extra {
            kind: ExtraKind::Other,
            index,
        } => index.map_or_else(|| "Other".to_string(), |i| format!("Other {i}")),
        FileReading::Extra { kind, index } => labels::extra_label(kind, index, None, None),
    }
}

/// Every quick fix, by path. One row per fixed file, few by nature, so a scan
/// reads them all once.
pub(crate) fn load_fixes(conn: &Connection) -> Result<HashMap<String, FileReading>, CoreError> {
    let mut stmt = conn.prepare("SELECT path, reading FROM file_fixes")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut out = HashMap::new();
    for row in rows {
        let (path, reading) = row?;
        // A reading this build cannot parse is left to the name, not fatal.
        if let Ok(reading) = serde_json::from_str(&reading) {
            out.insert(path, reading);
        }
    }
    Ok(out)
}

/// The series' AniList record, its format and episode count, when it has
/// one.
pub(crate) fn record(conn: &Connection, series: u64) -> Result<Record, CoreError> {
    Ok(conn
        .query_row(
            "SELECT m.format, m.episodes FROM series s JOIN anilist_media m ON m.id = s.anilist_id WHERE s.id = ?1",
            params![series as i64],
            |r| {
                Ok(Record {
                    format: r.get(0)?,
                    episodes: r.get::<_, Option<i64>>(1)?.and_then(|n| u32::try_from(n).ok()),
                })
            },
        )
        .optional()?
        .unwrap_or_default())
}

/// The series' kind, Show when the row is gone.
fn series_kind(conn: &Connection, series: u64) -> Result<SeriesKind, CoreError> {
    Ok(conn
        .query_row(
            "SELECT kind FROM series WHERE id = ?1",
            params![series as i64],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .and_then(|k| SeriesKind::from_column(&k))
        .unwrap_or(SeriesKind::Show))
}

/// Whether the series' rows disagree with what its record now says: a
/// record rule would read a file past its name and the rows do not, or the
/// other way round. A match that changed is worth a scan only then, so a TV
/// series with an `SP1` beside its episodes never queues one.
pub fn record_matters(conn: &Connection, series: u64) -> Result<bool, CoreError> {
    let record = record(conn, series)?;
    let kind = series_kind(conn, series)?;
    let mut stmt = conn.prepare(
        "SELECT f.path, f.kind, f.number, f.season,
                EXISTS (SELECT 1 FROM file_fixes x WHERE x.path = f.path)
         FROM files f WHERE f.series_id = ?1",
    )?;
    let rows = stmt
        .query_map(params![series as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<f64>>(2)?,
                r.get::<_, Option<i64>>(3)?,
                r.get::<_, bool>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let names: Vec<Classified> = rows
        .iter()
        .map(|(path, ..)| classifier::classify(&file_name(path)))
        .collect();
    let named: Vec<&Classified> = names.iter().collect();

    // The film rule: one file its name reads past, stored as episode 1.
    let film_would = film_rule_applies(kind, &record, &named);
    let film_does = matches!(
        rows.as_slice(),
        [(_, row_kind, Some(n), season, false)]
            if row_kind == "episode" && *n == 1.0 && season.is_none_or(|s| s <= 1)
    ) && reads_past_film(&names[0]);
    let would = record_rule_applies(record.format.as_deref(), &named);
    // A specials record reads a lone `SP1` as episode 1 too; that is its own
    // rule's row, not the film rule's.
    if film_would || (film_does && !would) {
        return Ok(film_would != film_does);
    }

    let does = rows
        .iter()
        .zip(&names)
        .any(|((_, kind, _, _, fixed), c)| !fixed && numbered_special(c) && kind == "episode");
    Ok(would != does)
}

/// Rescans `series` when its rows disagree with its record (`record_matters`).
/// For the places a match changes outside a job: clearing one, say.
pub fn reread_if_record_matters(library: &std::sync::Arc<crate::Library>, series: u64) {
    if library
        .services
        .store
        .read(|c| record_matters(c, series))
        .unwrap_or(false)
    {
        // A source away right now gets its scan when it is back.
        let _ = super::scan::rescan_series(library, series);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classifier::classify;

    fn walked(path: &str) -> ScannedFile {
        let name = file_name(path);
        let classified = classify(&name);
        ScannedFile {
            path: path.to_string(),
            size: 1,
            mtime: 1,
            label: String::new(),
            episode_key: classifier::episode_key(&classified, &name),
            classified,
            sidecars: Vec::new(),
        }
    }

    fn rec(format: Option<&str>, episodes: Option<u32>) -> Record {
        Record {
            format: format.map(str::to_string),
            episodes,
        }
    }

    /// A film in its own folder whose name carries a number that is not
    /// its episode: an ordinal in the title, then the type.
    const FILM: &str =
        "/lib/Some Film/[Fansub] Some Show - Dai 63-kai Taikai - Movie [1080p][HEVC].mkv";

    #[test]
    fn a_film_record_reads_its_one_file_as_the_film() {
        let mut files = vec![walked(FILM)];
        assert_eq!(files[0].classified.number, 63.0, "the name alone reads 63");
        apply(
            SeriesKind::Show,
            &rec(Some("MOVIE"), Some(1)),
            &mut files,
            &HashMap::new(),
        );
        let f = &files[0];
        assert_eq!(
            (f.classified.extra, f.classified.number, f.classified.season),
            (None, 1.0, None)
        );
        assert_eq!(f.episode_key, classifier::number_key(None, 1.0));
        // A lone special in a film's folder is the film too.
        let mut files = vec![walked("/lib/Some Film/Some Show - SP.mkv")];
        assert_eq!(files[0].classified.extra, Some(ExtraKind::Sp));
        apply(
            SeriesKind::Show,
            &rec(Some("MOVIE"), Some(1)),
            &mut files,
            &HashMap::new(),
        );
        assert_eq!(
            (files[0].classified.extra, files[0].classified.number),
            (None, 1.0)
        );
    }

    #[test]
    fn the_film_rule_needs_a_one_episode_film_record_over_one_file_in_a_folder() {
        let unchanged = |kind: SeriesKind, record: Record, mut files: Vec<ScannedFile>| {
            let before: Vec<f64> = files.iter().map(|f| f.classified.number).collect();
            apply(kind, &record, &mut files, &HashMap::new());
            let after: Vec<f64> = files.iter().map(|f| f.classified.number).collect();
            assert_eq!(before, after, "{kind:?} {record:?}");
        };
        unchanged(
            SeriesKind::Show,
            rec(Some("TV"), Some(1)),
            vec![walked(FILM)],
        );
        unchanged(
            SeriesKind::Show,
            rec(Some("MOVIE"), Some(3)),
            vec![walked(FILM)],
        );
        unchanged(
            SeriesKind::Show,
            rec(Some("MOVIE"), None),
            vec![walked(FILM)],
        );
        unchanged(SeriesKind::Show, rec(None, None), vec![walked(FILM)]);
        // A loose film file keeps its name as its key.
        unchanged(
            SeriesKind::Movie,
            rec(Some("MOVIE"), Some(1)),
            vec![walked(FILM)],
        );
        unchanged(
            SeriesKind::Show,
            rec(Some("MOVIE"), Some(1)),
            vec![walked(FILM), walked("/lib/Some Film/Some Show - 02.mkv")],
        );
        // An opening stays an opening.
        let mut files = vec![walked("/lib/Some Film/Some Show - NCOP1.mkv")];
        apply(
            SeriesKind::Show,
            &rec(Some("MOVIE"), Some(1)),
            &mut files,
            &HashMap::new(),
        );
        assert_eq!(files[0].classified.extra, Some(ExtraKind::Op));
    }

    #[test]
    fn a_fix_wins_over_the_film_rule() {
        let mut files = vec![walked(FILM)];
        let mut fixes = HashMap::new();
        fixes.insert(
            FILM.to_string(),
            FileReading::Extra {
                kind: ExtraKind::Other,
                index: None,
            },
        );
        apply(
            SeriesKind::Show,
            &rec(Some("MOVIE"), Some(1)),
            &mut files,
            &fixes,
        );
        assert_eq!(files[0].classified.extra, Some(ExtraKind::Other));
    }

    fn gup() -> Vec<ScannedFile> {
        (1..=6)
            .map(|n| {
                walked(&format!(
                    "/lib/GuP Specials/[ak-Submarines] Girls und Panzer - OVA {n} [BD 1080p].mkv"
                ))
            })
            .collect()
    }

    #[test]
    fn a_specials_record_reads_its_numbered_specials_as_episodes() {
        let mut files = gup();
        apply(
            SeriesKind::Show,
            &rec(Some("SPECIAL"), None),
            &mut files,
            &HashMap::new(),
        );
        for (i, f) in files.iter().enumerate() {
            let n = (i + 1) as f64;
            assert_eq!(
                (f.classified.extra, f.classified.number, f.classified.season),
                (None, n, None)
            );
            assert_eq!(f.episode_key, classifier::format_number(n));
        }
        let mut files = gup();
        apply(
            SeriesKind::Show,
            &rec(Some("OVA"), None),
            &mut files,
            &HashMap::new(),
        );
        assert!(files.iter().all(|f| f.classified.extra.is_none()));
    }

    #[test]
    fn ovas_numbered_as_a_season_read_as_the_records_episodes() {
        // `Season 1 OVAs/Seitokai Yakuindomo OVA - S01E01.mkv` and on, the
        // folder matched to the OVA record (AniList 10119, 8 episodes).
        let mut files: Vec<ScannedFile> = (1..=8)
            .map(|n| {
                walked(&format!(
                    "/lib/Season 1 OVAs/Seitokai Yakuindomo OVA - S01E{n:02}.mkv"
                ))
            })
            .collect();
        apply(
            SeriesKind::Show,
            &rec(Some("OVA"), None),
            &mut files,
            &HashMap::new(),
        );
        let numbers: Vec<f64> = files.iter().map(|f| f.classified.number).collect();
        assert!(files.iter().all(|f| f.classified.extra.is_none()));
        assert_eq!(numbers, (1..=8).map(f64::from).collect::<Vec<_>>());
    }

    #[test]
    fn any_other_record_or_a_regular_episode_leaves_them_specials() {
        for format in [None, Some("TV"), Some("MOVIE")] {
            let mut files = gup();
            apply(
                SeriesKind::Show,
                &rec(format, None),
                &mut files,
                &HashMap::new(),
            );
            assert!(
                files
                    .iter()
                    .all(|f| f.classified.extra == Some(ExtraKind::Sp)),
                "{format:?}"
            );
        }
        let mut files = gup();
        files.push(walked("/lib/GuP Specials/Girls und Panzer - 01.mkv"));
        apply(
            SeriesKind::Show,
            &rec(Some("SPECIAL"), None),
            &mut files,
            &HashMap::new(),
        );
        assert_eq!(
            files
                .iter()
                .filter(|f| f.classified.extra.is_some())
                .count(),
            6
        );
    }

    #[test]
    fn a_fix_wins_over_the_name_and_the_record() {
        let mut files = gup();
        let mut fixes = HashMap::new();
        fixes.insert(
            files[2].path.clone(),
            FileReading::Extra {
                kind: ExtraKind::Op,
                index: Some(1),
            },
        );
        fixes.insert(
            files[3].path.clone(),
            FileReading::Episode {
                number: 9.0,
                season: Some(2),
            },
        );
        apply(
            SeriesKind::Show,
            &rec(Some("SPECIAL"), None),
            &mut files,
            &fixes,
        );
        assert_eq!(
            (files[2].classified.extra, files[2].label.as_str()),
            (Some(ExtraKind::Op), "Opening 1")
        );
        assert_eq!(files[2].episode_key, file_name(&files[2].path));
        assert_eq!(
            (files[3].classified.number, files[3].episode_key.as_str()),
            (9.0, "S02E09")
        );
        assert_eq!(files[0].classified.number, 1.0);
    }

    #[test]
    fn readings_in_words() {
        assert_eq!(
            describe(&FileReading::Episode {
                number: 3.0,
                season: None
            }),
            "Episode 3"
        );
        assert_eq!(
            describe(&FileReading::Episode {
                number: 5.0,
                season: Some(2)
            }),
            "Season 2 episode 5"
        );
        assert_eq!(
            describe(&FileReading::Extra {
                kind: ExtraKind::Sp,
                index: None
            }),
            "Special"
        );
        assert_eq!(
            describe(&FileReading::Extra {
                kind: ExtraKind::Ed,
                index: Some(2)
            }),
            "Ending 2"
        );
        assert_eq!(
            of_row("extra", None, Some(0), Some("sp"), None),
            FileReading::Extra {
                kind: ExtraKind::Sp,
                index: None
            }
        );
        assert_eq!(
            of_row("episode", Some(4.0), None, None, None),
            FileReading::Episode {
                number: 4.0,
                season: None
            }
        );
    }
}
