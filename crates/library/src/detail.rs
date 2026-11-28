//! One series page: the episodes, the extras, the characters and the
//! recommendations. Built off the same `Snapshot` a card is, so a page and
//! the card behind it can never disagree.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use serde_json::Value;

use crate::cards::{next_up, upcoming};
use crate::classifier;
use crate::labels;
use crate::snapshot::{FileRow, MediaRow, SeriesRow, Snapshot, season_of};
use anibeam_contract::i18n::tr;
use anibeam_contract::*;
use anibeam_foundation::store::sql::{as_i64, placeholders};
use anibeam_foundation::time;
use anibeam_foundation::titles;
use anibeam_foundation::watch::MARK_FRACTION;

/// One regular episode as ani.zip keeps it, filled in where it is silent
/// by the franchise fallback (#1461), for the series page.
#[derive(Default)]
struct AniZipEpisodeView {
    title: Option<String>,
    still: Option<String>,
    /// A still url is known for the episode, cached yet or not.
    has_still_url: bool,
    synopsis: Option<String>,
    aired_at: Option<i64>,
}

/// What the series page reads off ani.zip's tables and the fallback's.
#[derive(Default)]
struct AniZipView {
    episodes: HashMap<i64, AniZipEpisodeView>,
    logo: Option<String>,
    artwork: Vec<Artwork>,
    /// The first background in ani.zip's order, the one the image cache
    /// fetches and pins, once it is on disk (#1475).
    fanart: Option<String>,
}

impl AniZipView {
    /// The episode with this number, a whole number only.
    fn episode(&self, number: f64) -> Option<&AniZipEpisodeView> {
        (number.fract() == 0.0 && number >= 1.0)
            .then(|| self.episodes.get(&(number as i64)))
            .flatten()
    }
}

/// The wide picture behind the hero and the series head (#1475): of
/// ani.zip's first background and the banner, whichever are on disk, the
/// one with more pixels across. A width the header does not give away loses
/// to one it does, and a tie goes to the background. None only when neither
/// is there, and the shells wash the poster in. No minimum: a narrow banner
/// still beats the blurred poster.
fn backdrop(
    fanart: Option<String>,
    banner: Option<&str>,
    width_of: impl Fn(&Path) -> Option<u32>,
) -> Option<String> {
    let on_disk = |p: &str| Path::new(p).is_file();
    let fanart = fanart.filter(|f| on_disk(f));
    let banner = banner.filter(|b| on_disk(b));
    match (fanart, banner) {
        (Some(f), Some(b)) => {
            // None sorts below any width, so an unread width loses.
            if width_of(Path::new(b)) > width_of(Path::new(&f)) {
                Some(b.to_string())
            } else {
                Some(f)
            }
        }
        (Some(f), None) => Some(f),
        (None, b) => b.map(str::to_string),
    }
}

/// Midnight UTC of a `YYYY-MM-DD`.
fn day_start(day: &str) -> Option<i64> {
    let mut parts = day.splitn(3, '-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u8 = parts.next()?.parse().ok()?;
    let day: u8 = parts.next()?.parse().ok()?;
    let date =
        ::time::Date::from_calendar_date(year, ::time::Month::try_from(month).ok()?, day).ok()?;
    Some(date.midnight().assume_utc().unix_timestamp())
}

/// The file of the TheTVDB logo `anilist_id` draws, by the logo language
/// setting, among those the image cache holds and did not turn away. A logo
/// TheTVDB's page names no language for takes the one ani.zip gives the
/// same file. `None` when neither names the language of any of them, so
/// ani.zip's own order decides, or when none is on disk yet.
fn tvdb_logo(conn: &Connection, anilist_id: u64) -> Result<Option<String>, CoreError> {
    let mut stmt = conn.prepare(
        "SELECT coalesce(l.lang, (SELECT lower(a.lang) FROM anizip_images a
                                  WHERE a.anilist_id = k.anilist_id AND a.kind = 'Clearlogo' AND a.url = l.url)),
                i.path
         FROM tvdb_logo_asks k
         JOIN anizip_shows z ON z.anilist_id = k.anilist_id AND z.tvdb_id = k.tvdb_id
         JOIN tvdb_logos l ON l.tvdb_id = k.tvdb_id
         JOIN images i ON i.url = l.url
         WHERE NOT EXISTS (SELECT 1 FROM logo_checks v WHERE v.url = l.url AND v.verdict = 0)
           AND k.anilist_id = ?1 ORDER BY l.rank",
    )?;
    let rows = stmt.query_map([anilist_id as i64], |r| {
        Ok((r.get::<_, Option<String>>(0)?, r.get::<_, String>(1)?))
    })?;
    let logos = rows.collect::<Result<Vec<_>, _>>()?;
    if logos.iter().all(|(lang, _)| lang.is_none()) {
        return Ok(None);
    }
    let prefs = anibeam_foundation::prefs::load_preferences(conn).unwrap_or_default();
    let want = crate::logo::wanted(prefs.logo_language, prefs.title_language);
    // ani.zip's `en-US` and TheTVDB's `en` are one language here.
    let langs: Vec<Option<&str>> = logos
        .iter()
        .map(|(l, _)| l.as_deref().and_then(|l| l.split(['-', '_']).next()))
        .collect();
    Ok(crate::logo::pick(&langs, want).map(|at| logos[at].1.clone()))
}

/// The torrent each of these files came from, by name, keyed by path: the
/// current origins the downloads team records once the torrent client lists
/// a file (`file_origins`, #241). A file it never listed, or one replaced
/// since, is absent.
fn torrents_of(conn: &Connection, files: &[FileRow]) -> Result<HashMap<String, String>, CoreError> {
    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    Ok(anibeam_foundation::store::origins::of(conn, &paths)?
        .into_iter()
        .map(|(path, origin)| (path, origin.name))
        .collect())
}

impl Snapshot {
    /// The series page. Takes the connection because the character images
    /// and the recommendations are one query each, and neither belongs in a
    /// whole-library snapshot.
    pub fn detail(&self, conn: &Connection, id: u64) -> Result<Option<SeriesDetail>, CoreError> {
        Ok(self.detail_and_stills(conn, id)?.map(|(d, _)| d))
    }

    /// [`Self::detail`], and the files whose episode has a still url the
    /// image cache has not fetched yet: a still is on its way for those, so
    /// nothing stands in for it (#1469).
    pub fn detail_and_stills(
        &self,
        conn: &Connection,
        id: u64,
    ) -> Result<Option<(SeriesDetail, HashSet<u64>)>, CoreError> {
        let Some(row) = self.row(id) else {
            return Ok(None);
        };
        let card = self.card_of(row);
        let media = row.media.as_ref();
        let now = time::to_secs(self.now);
        let (latest_aired, next_scheduled) = self.aired_and_next(row, now);
        let files = self.files_of(id);
        let episodes = self.episodes_on_disk(id);
        let entry = self.tracker(row);

        let total = media.and_then(|m| m.episodes).filter(|t| *t > 0);
        let on_disk = episodes.len() as u64;
        let (progress_total, estimate) = match total {
            Some(t) => (Some(t), false),
            None if latest_aired > 0 => (Some(latest_aired.max(card.watched.unwrap_or(0))), true),
            None => (Some(on_disk as u32), false),
        };

        let disk: Vec<(u64, u32, f64)> = episodes
            .iter()
            .map(|f| (f.id, season_of(f), f.number))
            .collect();
        let next = next_up(&disk, self.last_completed(id), card.watched);

        // Files numbered past a known episode count are almost always
        // misnamed, duplicates or stray specials; `past_the_end` decides
        // which list each file is judged against.
        let unmatched: HashSet<u64> = match (row.kind, total) {
            (SeriesKind::Show, Some(t)) => episodes
                .iter()
                .filter(|f| self.past_the_end(row, f, t))
                .map(|f| f.id)
                .collect(),
            _ => HashSet::new(),
        };

        let completed_keys: HashSet<&str> = self
            .completed
            .get(&id)
            .map(|rows| rows.iter().map(|(k, _)| k.as_str()).collect())
            .unwrap_or_default();
        let versions = episode_versions(&episodes);
        let zip = self.anizip_of(conn, row)?;
        // The torrent's name is a nicety: a read that fails leaves it out
        // rather than failing the page.
        let torrents = torrents_of(conn, files).unwrap_or_else(|e| {
            tracing::debug!("series {id}: file origins not read: {e}");
            HashMap::new()
        });
        let torrent = |f: &FileRow| torrents.get(&f.path).cloned();
        let episode = |f: &FileRow| {
            let air = self.episode_airing(row, f);
            let detail = zip.episode(f.number);
            let tracked = card.watched.is_some_and(|w| f.number <= f64::from(w));
            let here = completed_keys.contains(f.episode_key.as_str());
            Episode {
                file: f.id,
                number: f.number,
                season: f.season,
                code: labels::episode_code(f.season, f.number),
                // ani.zip's title, else the provider's, else whatever the
                // release name carries after its episode token.
                title: detail
                    .and_then(|d| d.title.clone())
                    .or_else(|| air.and_then(|a| a.title.clone()))
                    .or_else(|| {
                        Path::new(&f.path)
                            .file_name()
                            .and_then(|n| classifier::file_episode_title(&n.to_string_lossy()))
                    }),
                air_date: air
                    .and_then(|a| a.aired_at)
                    .or_else(|| detail.and_then(|d| d.aired_at))
                    .map(time::from_secs),
                path: f.path.clone(),
                sidecars: f.sidecars.clone(),
                resume: self.resume_of(id, &f.episode_key),
                stopped_short: self.stopped_short(id, &f.episode_key),
                watched: here || tracked,
                next_up: next == Some(f.id),
                downloading: None,
                cube: if tracked {
                    "w"
                } else if here {
                    "l"
                } else {
                    "d"
                }
                .to_string(),
                fixed: f.fixed,
                versions: versions.get(&version_key(f)).cloned().unwrap_or_default(),
                still: detail.and_then(|d| d.still.clone()),
                frame: None,
                synopsis: detail.and_then(|d| d.synopsis.clone()),
                size: f.size,
                torrent: torrent(f),
            }
        };

        // The aired episodes with no file, read off the card's own cube row
        // so the page and every cube row agree on what is missing. Those the
        // tracker covers ('w') are left out: a series watched elsewhere would
        // otherwise list every episode it never had. A record numbering
        // without seasons takes the newest file's, so the rows sort and code
        // like the files around them (S04E09 after S04E08).
        let season = self
            .card_season(row)
            .or_else(|| episodes.last().and_then(|f| f.season));
        let airing = self.airing_of(row);
        // Every aired episode with no file here, a watched one included: watched elsewhere is
        // still not on this device (#156). Its cube stays watched, so it reads as tracked.
        let absent: Vec<AbsentEpisode> = card
            .cubes
            .chars()
            .zip(1u32..)
            .filter(|(c, n)| {
                matches!(c, 'm' | 'g')
                    || (matches!(c, 'w' | 'l') && !card.episodes_here.contains(n))
            })
            .map(|(c, number)| {
                let air = airing.iter().find(|a| a.number == number);
                let detail = zip.episode(f64::from(number));
                AbsentEpisode {
                    number,
                    season,
                    code: labels::episode_code(season, f64::from(number)),
                    title: detail
                        .and_then(|d| d.title.clone())
                        .or_else(|| air.and_then(|a| a.title.clone())),
                    air_date: air
                        .and_then(|a| a.aired_at)
                        .or_else(|| detail.and_then(|d| d.aired_at))
                        .map(time::from_secs),
                    cube: c.to_string(),
                    still: detail.and_then(|d| d.still.clone()),
                    synopsis: detail.and_then(|d| d.synopsis.clone()),
                }
            })
            .collect();
        // A TVmaze count set by hand runs within the card's season (#283)
        let plan_total = self.season_total(row, total);
        let released = self.released_of(
            row,
            &episodes,
            plan_total,
            latest_aired,
            next_scheduled,
            card.watched,
        );
        let notes = self.notes_of(row);
        let upcoming = upcoming(airing, &notes, released, plan_total, card.status, now)
            .into_iter()
            .map(|(number, at, predicted)| {
                let detail = zip.episode(f64::from(number));
                UpcomingEpisode {
                    number,
                    season,
                    code: labels::episode_code(season, f64::from(number)),
                    title: detail.and_then(|d| d.title.clone()).or_else(|| {
                        airing
                            .iter()
                            .find(|a| a.number == number)
                            .and_then(|a| a.title.clone())
                    }),
                    at: time::from_secs(at),
                    predicted,
                    still: detail.and_then(|d| d.still.clone()),
                    synopsis: detail.and_then(|d| d.synopsis.clone()),
                }
            })
            .collect();

        let absent_numbers: Vec<u32> = absent.iter().map(|a| a.number).collect();
        let suggested_fix = suggest(
            row.kind,
            media.and_then(|m| m.format.as_deref()),
            total,
            row.provider == Some(Provider::Tvmaze),
            files,
            &absent_numbers,
        );

        let mut extras: Vec<&FileRow> = files.iter().filter(|f| !f.is_episode).collect();
        // Two releases of one extra share a key and a folder; the same name in another
        // folder (Season 2/NCOP.mkv) is another extra.
        let extra_versions = versions_by(&extras, extra_version_key);
        extras.sort_by(|a, b| {
            extra_order(a.extra_kind)
                .cmp(&extra_order(b.extra_kind))
                .then_with(|| a.extra_index.unwrap_or(0).cmp(&b.extra_index.unwrap_or(0)))
                .then_with(|| a.label.cmp(&b.label))
                .then_with(|| a.path.cmp(&b.path))
        });

        let stills_coming: HashSet<u64> = episodes
            .iter()
            .filter(|f| {
                zip.episode(f.number)
                    .is_some_and(|d| d.has_still_url && d.still.is_none())
            })
            .map(|f| f.id)
            .collect();

        let detail = SeriesDetail {
            banner: row.banner_path.clone(),
            // Plain text in paragraphs for every shell. AniList's description is HTML;
            // TVmaze's summary was made plain when it was stored, and a second pass would
            // read a decoded `<Title>` in it as a tag (#450).
            synopsis: media
                .and_then(|m| m.description.as_deref())
                .map(|d| {
                    if row.provider == Some(Provider::Tvmaze) {
                        d.to_string()
                    } else {
                        anibeam_foundation::html::plain_text(d)
                    }
                })
                .unwrap_or_default(),
            year: media.and_then(|m| m.year),
            season: media.and_then(|m| m.season.clone()),
            studio: media.and_then(|m| m.studio.clone()),
            episode_minutes: media.and_then(|m| m.duration).filter(|m| *m > 0),
            genres: media.map(|m| m.genres.clone()).unwrap_or_default(),
            tags: media.map(|m| tags_of(&m.tags)).unwrap_or_default(),
            rewatch_count: entry.map(|e| e.repeat).filter(|r| *r > 0),
            notes: entry.and_then(|e| e.notes.clone()),
            site_url: media.and_then(|m| m.site_url.clone()),
            progress: ProgressLine {
                watched: card.watched,
                total: progress_total,
                estimate,
                on_disk,
            },
            next_up: next,
            episodes: episodes
                .iter()
                .filter(|f| !unmatched.contains(&f.id))
                .map(|f| episode(f))
                .collect(),
            absent,
            upcoming,
            schedule_notes: notes.iter().map(|n| n.to_wire()).collect(),
            extras: extras
                .iter()
                .map(|f| {
                    let kind = f.extra_kind.unwrap_or(ExtraKind::Other);
                    Extra {
                        file: f.id,
                        kind,
                        code: labels::extra_code_with_index(kind, f.extra_index),
                        label: f.label.clone(),
                        path: f.path.clone(),
                        sidecars: f.sidecars.clone(),
                        resume: self.resume_of(id, &f.episode_key),
                        index: f.extra_index,
                        fixed: f.fixed,
                        versions: extra_versions
                            .get(&extra_version_key(f))
                            .cloned()
                            .unwrap_or_default(),
                        size: f.size,
                        torrent: torrent(f),
                    }
                })
                .collect(),
            unmatched_files: episodes
                .iter()
                .filter(|f| unmatched.contains(&f.id))
                .map(|f| episode(f))
                .collect(),
            characters: self.characters(conn, media)?,
            recommendations: self.recommendations(conn, row.anilist_id)?,
            has_graph: row
                .anilist_id
                .is_some_and(|a| self.graph_seeds.contains(&a)),
            relations_known: match row.anilist_id {
                Some(a) => relations_known(conn, a)?,
                None => false,
            },
            suggested_fix,
            logo: zip.logo,
            artwork: zip.artwork,
            backdrop: backdrop(
                zip.fanart,
                row.banner_path.as_deref(),
                anibeam_net::dimensions::width_of,
            ),
            logo_choice: conn
                .query_row(
                    "SELECT logo_choice FROM series WHERE id = ?1",
                    [id as i64],
                    |r| r.get::<_, Option<i64>>(0),
                )
                .optional()?
                .flatten()
                .map(|v| v != 0),
            card,
        };
        Ok(Some((detail, stills_coming)))
    }

    /// What ani.zip keeps for an AniList series (#1036): each regular
    /// episode's title and still, the clear logo, and the artwork. Nothing
    /// for any other match or an unmatched folder, the same rule the image
    /// cache fetches and pins by. A path is handed out only when the image cache holds
    /// the file's row.
    fn anizip_of(&self, conn: &Connection, row: &SeriesRow) -> Result<AniZipView, CoreError> {
        let mut view = AniZipView::default();
        let Some(anilist_id) = row
            .anilist_id
            .filter(|_| row.provider == Some(Provider::Anilist))
        else {
            return Ok(view);
        };
        let local = |p: Option<String>| {
            p.map(|p| {
                Path::new(&self.images_dir)
                    .join(p)
                    .to_string_lossy()
                    .into_owned()
            })
        };
        let mut stmt = conn.prepare(
            "SELECT z.number, z.title, i.path, z.aired_at, z.air_date, z.synopsis,
                    z.still_url IS NOT NULL
             FROM anizip_episodes z LEFT JOIN images i ON i.url = z.still_url
             WHERE z.anilist_id = ?1 AND z.number IS NOT NULL",
        )?;
        let rows = stmt.query_map([anilist_id as i64], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<i64>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, bool>(6)?,
            ))
        })?;
        for row in rows {
            let (number, title, still, aired_at, air_date, synopsis, has_still_url) = row?;
            let aired_at = aired_at.or_else(|| air_date.as_deref().and_then(day_start));
            view.episodes.insert(
                number,
                AniZipEpisodeView {
                    title,
                    still: local(still),
                    has_still_url,
                    synopsis,
                    aired_at,
                },
            );
        }
        // The franchise fallback stands in while ani.zip has no regular
        // episodes for the series, and only then (#1461): once ani.zip maps
        // the season, another source's guesses never fill its gaps.
        let mut stmt = conn.prepare(
            "SELECT e.number, e.title, i.path, e.aired_at, e.synopsis,
                    e.still_url IS NOT NULL
             FROM fallback_episodes e LEFT JOIN images i ON i.url = e.still_url
             WHERE e.anilist_id = ?1
               AND NOT EXISTS (SELECT 1 FROM anizip_episodes z WHERE z.anilist_id = ?1 AND z.number IS NOT NULL)",
        )?;
        let rows = stmt.query_map([anilist_id as i64], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<i64>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, bool>(5)?,
            ))
        })?;
        for row in rows {
            let (number, title, still, aired_at, synopsis, has_still_url) = row?;
            let e = view.episodes.entry(number).or_default();
            e.title = e.title.take().or(title);
            e.still = e.still.take().or_else(|| local(still));
            e.has_still_url |= has_still_url;
            e.synopsis = e.synopsis.take().or(synopsis);
            e.aired_at = e.aired_at.or(aired_at);
        }
        let mut stmt = conn.prepare(
            "SELECT a.kind, a.url, a.lang, i.path, EXISTS (SELECT 1 FROM logo_checks v WHERE v.url = a.url AND v.verdict = 0) FROM anizip_images a
             LEFT JOIN images i ON i.url = a.url
             WHERE a.anilist_id = ?1 ORDER BY a.rank",
        )?;
        let rows = stmt.query_map([anilist_id as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<bool>>(4)?.unwrap_or(false),
            ))
        })?;
        // Each usable logo's language and file, in the reply's order.
        let mut logos: Vec<(Option<String>, Option<String>)> = Vec::new();
        let mut seen_fanart = false;
        for row in rows {
            let (kind, url, lang, path, not_a_logo) = row?;
            let path = local(path);
            // A logo on a solid rectangle is no logo (#1470): the next one
            // is picked, then a franchise season's, then the title.
            if kind == "Clearlogo" && !not_a_logo {
                logos.push((lang.clone(), path.clone()));
            } else if kind == "Fanart" && !seen_fanart {
                // The first one only, on disk or not: the one the cache
                // fetches and pins, so the pick and the pin agree.
                seen_fanart = true;
                view.fanart = path.clone();
            }
            view.artwork.push(Artwork { kind, url, path });
        }
        // A logo on disk wins over a better one that is not, so a page
        // never goes without while a usable file is there.
        // No logo of its own at all: the nearest franchise season's
        // (#1461). One of its own that is not on disk yet is waited for,
        // the same rule the image cache fetches and pins by.
        // Among those on disk, the logo language setting picks by ani.zip's
        // own languages (#1475 review), the way it picks TheTVDB's below:
        // the wanted one, then the other of English and Japanese, then any.
        // The image cache fetches the first of each of those three.
        let own_logo = !logos.is_empty();
        let on_disk: Vec<&(Option<String>, Option<String>)> =
            logos.iter().filter(|(_, path)| path.is_some()).collect();
        let langs: Vec<Option<&str>> = on_disk
            .iter()
            .map(|(l, _)| l.as_deref().and_then(|l| l.split(['-', '_']).next()))
            .collect();
        let prefs = anibeam_foundation::prefs::load_preferences(conn).unwrap_or_default();
        let want = crate::logo::wanted(prefs.logo_language, prefs.title_language);
        view.logo = crate::logo::pick(&langs, want).and_then(|at| on_disk[at].1.clone());
        // TheTVDB's logos, where it named their languages (#1470), win over
        // ani.zip's order: the one in the language wanted, then the other of
        // English and Japanese, then any, of those on disk.
        if let Some(path) = tvdb_logo(conn, anilist_id)? {
            view.logo = local(Some(path));
            return Ok(view);
        }
        if !own_logo {
            let borrowed: Option<String> = conn
                .query_row(
                    "SELECT i.path FROM fallback_shows b JOIN images i ON i.url = b.logo_url
                     WHERE b.anilist_id = ?1 AND NOT EXISTS (SELECT 1 FROM logo_checks v WHERE v.url = b.logo_url AND v.verdict = 0)",
                    [anilist_id as i64],
                    |r| r.get(0),
                )
                .optional()?;
            view.logo = local(borrowed);
        }
        Ok(view)
    }

    fn resume_of(&self, series: u64, key: &str) -> Option<ResumePoint> {
        self.resume
            .get(&(series, key.to_string()))
            .map(|(position, duration)| ResumePoint {
                position: *position,
                duration: *duration,
            })
    }

    /// Whether the episode's resume point sits before its end line: the
    /// outro's start when the skip windows name one, else the tracker
    /// mark's 85 percent. The same line `playback::session` marks on.
    fn stopped_short(&self, series: u64, key: &str) -> bool {
        let k = (series, key.to_string());
        let Some((position, duration)) = self.resume.get(&k) else {
            return false;
        };
        let line = self
            .outro
            .get(&k)
            .copied()
            .unwrap_or(duration * MARK_FRACTION);
        *position < line
    }

    /// The season and number of the most recently completed episode,
    /// resolved through the file that carries its key, or the key itself
    /// when no file does.
    fn last_completed(&self, id: u64) -> Option<(u32, f64)> {
        let (key, _) = self.completed.get(&id)?.iter().max_by_key(|(_, at)| *at)?;
        self.files_of(id)
            .iter()
            .find(|f| f.is_episode && f.episode_key == *key)
            .map(|f| (season_of(f), f.number))
            .or_else(|| classifier::parse_episode_key(key))
    }

    /// The top characters, their portraits resolved through the image cache
    /// in one query. A portrait that has not been fetched yet is None.
    fn characters(
        &self,
        conn: &Connection,
        media: Option<&MediaRow>,
    ) -> Result<Vec<Person>, CoreError> {
        let Some(media) = media else {
            return Ok(Vec::new());
        };
        let people = json_array(&media.characters);
        if people.is_empty() {
            return Ok(Vec::new());
        }
        let urls: Vec<String> = people
            .iter()
            .filter_map(|c| {
                c.get("image_url")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect();
        let mut cached: HashMap<String, String> = HashMap::new();
        if !urls.is_empty() {
            let sql = format!(
                "SELECT url, path FROM images WHERE url IN ({})",
                placeholders(urls.len())
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params_from_iter(urls.iter()), |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (url, path) = row?;
                cached.insert(url, path);
            }
        }
        Ok(people
            .iter()
            .map(|c| Person {
                anilist_id: c.get("id").and_then(Value::as_u64).filter(|id| *id > 0),
                name: c
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                image: c
                    .get("image_url")
                    .and_then(Value::as_str)
                    .and_then(|u| cached.get(u))
                    .map(|p| {
                        Path::new(&self.images_dir)
                            .join(p)
                            .to_string_lossy()
                            .into_owned()
                    }),
                role: c
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            })
            .collect())
    }

    /// AniList's recommendations for this series, in AniList's own order,
    /// each one told whether the library owns it and where it sits on the
    /// list.
    fn recommendations(
        &self,
        conn: &Connection,
        anilist_id: Option<u64>,
    ) -> Result<Vec<Recommendation>, CoreError> {
        let Some(anilist_id) = anilist_id else {
            return Ok(Vec::new());
        };
        let mut stmt = conn.prepare(
            "SELECT r.recommended_id, m.title_romaji, m.title_english, i.path,
                    (SELECT s.id FROM series s WHERE s.anilist_id = m.id AND s.missing_since IS NULL ORDER BY EXISTS (SELECT 1 FROM files f WHERE f.series_id = s.id) DESC, s.id LIMIT 1),
                    (SELECT ta.status FROM tracker_entries ta WHERE ta.tracker = 'anilist' AND ta.media_id = m.id AND ta.profile_id = ?2),
                    (SELECT tm.status FROM tracker_entries tm WHERE tm.tracker = 'mal' AND tm.media_id = m.mal_id AND tm.profile_id = ?2),
                    m.title_native,
                    (SELECT l.status FROM local_entries l JOIN series ls ON ls.id = l.series_id WHERE ls.anilist_id = m.id ORDER BY ls.missing_since IS NOT NULL, ls.id LIMIT 1)
             FROM recommendations r
             JOIN anilist_media m ON m.id = r.recommended_id
             LEFT JOIN images i ON i.url = m.cover_url
             WHERE r.anilist_id = ?1
             ORDER BY r.rank",
        )?;
        // Each tracker list is the reader's own (#852).
        let rows = stmt.query_map([anilist_id as i64, self.profile as i64], |r| {
            let romaji: Option<String> = r.get(1)?;
            let english: Option<String> = r.get(2)?;
            let anilist_status: Option<String> = r.get(5)?;
            let mal_status: Option<String> = r.get(6)?;
            let native: Option<String> = r.get(7)?;
            // The list kept on this device is the owner's alone, as `load_for` reads it.
            let kept_here: Option<String> = r
                .get::<_, Option<String>>(8)?
                .filter(|_| self.profile == anibeam_foundation::trust::OWNER);
            let (main, other) = match self.main_tracker {
                Tracker::Anilist => (anilist_status, mal_status),
                Tracker::Mal => (mal_status, anilist_status),
            };
            Ok(Recommendation {
                anilist_id: r.get::<_, i64>(0)? as u64,
                title: titles::resolve(
                    self.lang,
                    romaji.as_deref(),
                    english.as_deref(),
                    native.as_deref(),
                    "",
                ),
                poster: r.get::<_, Option<String>>(3)?.map(|p| {
                    Path::new(&self.images_dir)
                        .join(p)
                        .to_string_lossy()
                        .into_owned()
                }),
                owned: r.get::<_, Option<i64>>(4)?.map(|v| v as u64),
                list_status: main
                    .or(other)
                    .or(kept_here)
                    .and_then(|s| ListStatus::from_column(&s)),
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}

/// The versions of every episode that is on disk more than once, keyed by
/// `season_of` and the number's bits. Files read as an episode of the same
/// season and number are two releases of it; the web shell's old rule, moved
/// here so every shell lists one row per episode. Takes the episodes on disk.
pub(crate) fn episode_versions(files: &[&FileRow]) -> HashMap<(u32, u64), Vec<EpisodeVersion>> {
    versions_by(files, version_key)
}

/// Which extra a file is, for its versions: its history key in its folder.
fn extra_version_key(f: &FileRow) -> (String, String) {
    let folder = f.path.rsplit_once(['/', '\\']).map_or("", |(dir, _)| dir);
    (f.episode_key.clone(), folder.to_string())
}

/// The versions of every file that shares its `key` with another, each group
/// named by release as `episode_versions` names an episode's. Extras group by
/// their history key, so two revisions of one extra are its two releases.
fn versions_by<K: std::hash::Hash + Eq>(
    files: &[&FileRow],
    key: impl Fn(&FileRow) -> K,
) -> HashMap<K, Vec<EpisodeVersion>> {
    let mut by: HashMap<K, Vec<&FileRow>> = HashMap::new();
    for f in files.iter().copied() {
        by.entry(key(f)).or_default().push(f);
    }
    by.into_iter()
        .filter(|(_, group)| group.len() > 1)
        .map(|(key, group)| {
            let labels: Vec<String> = group
                .iter()
                .map(|f| classifier::release_label(&f.path))
                .collect();
            let names: Vec<&str> = group
                .iter()
                .map(|f| classifier::base_name(&f.path))
                .collect();
            let alike = |list: &[&str], x: &str| list.iter().filter(|l| **l == x).count() > 1;
            let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
            let versions = group
                .iter()
                .enumerate()
                .map(|(i, f)| EpisodeVersion {
                    file: f.id,
                    // Two that still read alike get their file names, and two
                    // of one name in different folders their whole paths
                    label: if !alike(&label_refs, &labels[i]) {
                        labels[i].clone()
                    } else if !alike(&names, names[i]) {
                        names[i].to_string()
                    } else {
                        f.path.clone()
                    },
                })
                .collect();
            (key, versions)
        })
        .collect()
}

/// Which episode a file is, for `episode_versions`.
pub(crate) fn version_key(f: &FileRow) -> (u32, u64) {
    (season_of(f), f.number.to_bits())
}

/// A misreading of a series' files the page can see on its own (#170), and
/// the fixes that would undo it. The first of these that fits, never touching
/// a file the user already fixed:
///
/// - A one-episode record (a lone OVA, a special) holding one file, an
///   episode or a special, that is not read as that episode. A film record
///   reads its one file as the film at the scan (`readings`), so this only
///   speaks for a film when the count came from somewhere else.
/// - Numbered specials where the episodes they number are missing and no
///   regular episode is on disk.
/// - Every episode numbered past the record's count, two or more in one
///   unbroken run no longer than the count: a later season numbered on from
///   the first, 13 to 24 for this record's 1 to 12. A TVmaze match counts
///   within its season and is left out.
fn suggest(
    kind: SeriesKind,
    format: Option<&str>,
    total: Option<u32>,
    tvmaze: bool,
    files: &[FileRow],
    absent: &[u32],
) -> Option<SuggestedFix> {
    if kind != SeriesKind::Show || files.is_empty() || files.iter().any(|f| f.fixed) {
        return None;
    }
    let to_episode = |f: &FileRow, number: u32| FileFix {
        file: f.id,
        reading: Some(FileReading::Episode {
            number: f64::from(number),
            season: None,
        }),
    };

    if let ([only], Some(1)) = (files, total)
        && !(only.is_episode && only.number == 1.0)
        && (only.is_episode || only.extra_kind == Some(ExtraKind::Sp))
    {
        // The file's own label is what the scan stored for it
        let (read, label) = if only.is_episode {
            ("episode", String::new())
        } else {
            ("label", only.label.to_lowercase())
        };
        let what = if format == Some("MOVIE") {
            "film"
        } else {
            "episode"
        };
        return Some(SuggestedFix {
            text: tr(
                "core-library-fix-one-file",
                &[
                    ("read", read.into()),
                    ("number", classifier::format_number(only.number).into()),
                    ("label", label.into()),
                    ("what", what.into()),
                ],
            ),
            fixes: vec![to_episode(only, 1)],
        });
    }

    let specials: Vec<&FileRow> = files
        .iter()
        .filter(|f| !f.is_episode && f.extra_kind == Some(ExtraKind::Sp))
        .collect();
    let indexes: Vec<u32> = specials.iter().filter_map(|f| f.extra_index).collect();
    let distinct: HashSet<u32> = indexes.iter().copied().collect();
    // Only where no regular episode is on disk at all: beside episodes, a
    // numbered special is a special, whatever is missing around it.
    if !specials.is_empty()
        && files.iter().all(|f| !f.is_episode)
        && indexes.len() == specials.len()
        && distinct.len() == indexes.len()
        && indexes.iter().all(|n| absent.contains(n))
    {
        let mut numbers = indexes.clone();
        numbers.sort_unstable();
        return Some(SuggestedFix {
            text: tr(
                "core-library-fix-specials",
                &[
                    ("count", specials.len().into()),
                    ("episodes", numbers.len().into()),
                    ("numbers", runs(&numbers).into()),
                ],
            ),
            fixes: specials
                .iter()
                .map(|f| to_episode(f, f.extra_index.unwrap_or(0)))
                .collect(),
        });
    }

    let episodes: Vec<&FileRow> = files.iter().filter(|f| f.is_episode).collect();
    if let (Some(total), false, false) = (total, tvmaze, episodes.is_empty()) {
        let whole: Option<Vec<u32>> = episodes
            .iter()
            .map(|f| {
                (f.number.fract() == 0.0 && f.season.is_none_or(|s| s <= 1))
                    .then_some(f.number as u32)
            })
            .collect();
        if let Some(mut numbers) = whole {
            numbers.sort_unstable();
            let first = numbers[0];
            let last = numbers[numbers.len() - 1];
            let unbroken = numbers.windows(2).all(|w| w[1] == w[0] + 1);
            if numbers.len() > 1 && first > total && unbroken && last - first < total {
                let shift = first - 1;
                return Some(SuggestedFix {
                    text: tr(
                        "core-library-fix-numbered-on",
                        &[
                            ("from", runs(&numbers).into()),
                            (
                                "to",
                                runs(&numbers.iter().map(|n| n - shift).collect::<Vec<_>>()).into(),
                            ),
                        ],
                    ),
                    fixes: episodes
                        .iter()
                        .map(|f| to_episode(f, f.number as u32 - shift))
                        .collect(),
                });
            }
        }
    }
    None
}

/// Sorted whole numbers in words: `1 to 6`, `2, 4 and 7`, `1 and 2`, `3`.
fn runs(numbers: &[u32]) -> String {
    match numbers {
        [] => String::new(),
        [only] => only.to_string(),
        [first, _, .., last] if numbers.windows(2).all(|w| w[1] == w[0] + 1) => tr(
            "core-library-runs-span",
            &[
                ("first", first.to_string().into()),
                ("last", last.to_string().into()),
            ],
        ),
        [init @ .., last] => tr(
            "core-library-runs-list",
            &[
                (
                    "init",
                    init.iter()
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                        .into(),
                ),
                ("last", last.to_string().into()),
            ],
        ),
    }
}

/// Openings, then endings, previews, specials and the rest.
fn extra_order(kind: Option<ExtraKind>) -> u8 {
    match kind.unwrap_or(ExtraKind::Other) {
        ExtraKind::Op => 0,
        ExtraKind::Ed => 1,
        ExtraKind::Pv => 2,
        ExtraKind::Sp => 3,
        ExtraKind::Other => 4,
    }
}

fn tags_of(raw: &Value) -> Vec<Tag> {
    json_array(raw)
        .iter()
        .map(|t| Tag {
            name: t
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            rank: t.get("rank").and_then(Value::as_u64).unwrap_or(0) as u32,
            spoiler: t.get("spoiler").and_then(Value::as_bool).unwrap_or(false),
            adult: t.get("adult").and_then(Value::as_bool).unwrap_or(false),
        })
        .collect()
}

fn json_array(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}

/// Whether AniList's relations for this id have been fetched: by a full
/// metadata fetch or by the crawl. Until then a node with no edges may
/// only be a node whose edges nobody asked for yet.
fn relations_known(conn: &Connection, anilist_id: u64) -> Result<bool, CoreError> {
    let known: Option<bool> = conn
        .query_row(
            "SELECT relations_fetched_at IS NOT NULL FROM anilist_media WHERE id = ?1",
            params![as_i64(anilist_id)],
            |r| r.get(0),
        )
        .optional()?;
    Ok(known.unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(id: u64, name: &str, episode: Option<f64>, special: Option<Option<u32>>) -> FileRow {
        FileRow {
            id,
            series_id: 1,
            path: format!("/lib/Show/{name}"),
            mtime: 0,
            is_episode: episode.is_some(),
            number: episode.unwrap_or(0.0),
            season: None,
            extra_kind: special.map(|_| ExtraKind::Sp),
            extra_index: special.flatten(),
            label: if episode.is_some() {
                name.to_string()
            } else {
                "Special".to_string()
            },
            episode_key: name.to_string(),
            sidecars: Vec::new(),
            fixed: false,
            size: 0,
        }
    }

    #[test]
    fn two_revisions_of_one_extra_are_its_versions() {
        // SP01 and SP01v2 share the key the scan gives them (#468); a lone
        // SP02 has no versions.
        let mut a = file(1, "[A] Show - SP01.mkv", None, Some(Some(1)));
        let mut b = file(2, "[A] Show - SP01v2.mkv", None, Some(Some(1)));
        let c = file(3, "[A] Show - SP02.mkv", None, Some(Some(2)));
        a.episode_key = "[A] Show - SP01.mkv".into();
        b.episode_key = "[A] Show - SP01.mkv".into();
        let mut d = file(4, "Season 2/[A] Show - SP01.mkv", None, Some(Some(1)));
        d.episode_key = "[A] Show - SP01.mkv".into();
        let v = versions_by(&[&a, &b, &c, &d], extra_version_key);
        assert_eq!(
            v.len(),
            1,
            "the same name in another folder is another extra"
        );
        let files: Vec<u64> = v[&extra_version_key(&a)].iter().map(|x| x.file).collect();
        assert_eq!(files, [1, 2]);
    }

    fn numbers(fix: &SuggestedFix) -> Vec<(u64, f64)> {
        fix.fixes
            .iter()
            .map(|f| match f.reading {
                Some(FileReading::Episode { number, .. }) => (f.file, number),
                _ => panic!("not an episode"),
            })
            .collect()
    }

    #[test]
    fn a_film_folder_whose_one_file_reads_as_another_number() {
        // Girls und Panzer der Film: `Dai 63-kai ... - Movie` read as episode 63.
        let files = [file(7, "GuP - Dai 63-kai - Movie.mkv", Some(63.0), None)];
        let fix = suggest(
            SeriesKind::Show,
            Some("MOVIE"),
            Some(1),
            false,
            &files,
            &[1],
        )
        .unwrap();
        assert_eq!(
            fix.text,
            "The one file here reads as episode 63. It looks like the movie."
        );
        assert_eq!(numbers(&fix), vec![(7, 1.0)]);
        // Already episode 1, or a loose film file: nothing to say.
        let files = [file(7, "Film.mkv", Some(1.0), None)];
        assert!(suggest(SeriesKind::Show, Some("MOVIE"), Some(1), false, &files, &[]).is_none());
        let files = [file(7, "Film 2019.mkv", Some(63.0), None)];
        assert!(
            suggest(
                SeriesKind::Movie,
                Some("MOVIE"),
                Some(1),
                false,
                &files,
                &[]
            )
            .is_none()
        );
    }

    fn ovas() -> Vec<FileRow> {
        (1..=6)
            .map(|n| file(n, &format!("OVA {n}.mkv"), None, Some(Some(n as u32))))
            .collect()
    }

    #[test]
    fn numbered_specials_where_their_episodes_are_missing() {
        let all = [1, 2, 3, 4, 5, 6];
        let fix = suggest(SeriesKind::Show, Some("TV"), Some(6), false, &ovas(), &all).unwrap();
        assert_eq!(
            fix.text,
            "6 specials look like the missing episodes 1 to 6."
        );
        assert_eq!(numbers(&fix)[5], (6, 6.0));
        // A special whose episode is on disk, or one with no number, is no such case.
        assert!(
            suggest(
                SeriesKind::Show,
                Some("TV"),
                Some(6),
                false,
                &ovas(),
                &[1, 2, 3]
            )
            .is_none()
        );
        let mut loose = ovas();
        loose[0].extra_index = None;
        assert!(suggest(SeriesKind::Show, Some("TV"), Some(6), false, &loose, &all).is_none());
        // Nor a series the user already fixed.
        let mut fixed = ovas();
        fixed[2].fixed = true;
        assert!(suggest(SeriesKind::Show, Some("TV"), Some(6), false, &fixed, &all).is_none());
    }

    fn numbered_on() -> Vec<FileRow> {
        (13..=24)
            .map(|n| file(n, &format!("Show - {n}.mkv"), Some(n as f64), None))
            .collect()
    }

    #[test]
    fn a_season_numbered_on_from_the_last() {
        let fix = suggest(
            SeriesKind::Show,
            Some("TV"),
            Some(12),
            false,
            &numbered_on(),
            &[],
        )
        .unwrap();
        assert_eq!(
            fix.text,
            "Episodes 13 to 24 look like episodes 1 to 12, continuing numbering from an earlier season."
        );
        assert_eq!(numbers(&fix)[0], (13, 1.0));
        // A gap, a run longer than the count, a TVmaze match or one file in range: no.
        assert!(
            suggest(
                SeriesKind::Show,
                Some("TV"),
                Some(12),
                true,
                &numbered_on(),
                &[]
            )
            .is_none()
        );
        assert!(
            suggest(
                SeriesKind::Show,
                Some("TV"),
                Some(11),
                false,
                &numbered_on(),
                &[]
            )
            .is_none()
        );
        let mut gap = numbered_on();
        gap.remove(3);
        assert!(suggest(SeriesKind::Show, Some("TV"), Some(12), false, &gap, &[]).is_none());
        let mut mixed = numbered_on();
        mixed.push(file(99, "Show - 01.mkv", Some(1.0), None));
        assert!(suggest(SeriesKind::Show, Some("TV"), Some(12), false, &mixed, &[]).is_none());
    }

    #[test]
    fn no_suggestion_for_what_is_likely_right() {
        // Recap specials beside the series' own episodes, 1 and 2 deleted after watching.
        let mut beside: Vec<FileRow> = (3..=12)
            .map(|n| file(n, &format!("Show - {n}.mkv"), Some(n as f64), None))
            .collect();
        beside.push(file(21, "Show SP1.mkv", None, Some(Some(1))));
        beside.push(file(22, "Show SP2.mkv", None, Some(Some(2))));
        assert!(
            suggest(
                SeriesKind::Show,
                Some("TV"),
                Some(12),
                false,
                &beside,
                &[1, 2]
            )
            .is_none()
        );
        // One file past the end is a bonus episode as likely as a renumbering.
        let lone = [file(13, "Show - 13.mkv", Some(13.0), None)];
        assert!(suggest(SeriesKind::Show, Some("TV"), Some(12), false, &lone, &[]).is_none());
        // A lone opening is not the film.
        let mut opening = file(7, "Show NCOP1.mkv", None, None);
        opening.extra_kind = Some(ExtraKind::Op);
        assert!(
            suggest(
                SeriesKind::Show,
                Some("MOVIE"),
                Some(1),
                false,
                &[opening],
                &[1]
            )
            .is_none()
        );
    }

    #[test]
    fn the_wider_of_the_background_and_the_banner_is_the_backdrop() {
        use anibeam_net::dimensions::{testing, width_of};
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, bytes: Vec<u8>| {
            let path = dir.path().join(name);
            std::fs::write(&path, bytes).unwrap();
            path.to_string_lossy().into_owned()
        };
        let fanart = write("fanart.jpg", testing::jpeg(1920, 1080));
        let old_fanart = write("old.jpg", testing::jpeg(1280, 720));
        let wide = write("wide.jpg", testing::jpeg(1900, 400));
        let wider = write("wider.jpg", testing::jpeg(2560, 600));
        let small = write("small.png", testing::png(1200, 300));
        let same = write("same.jpg", testing::jpeg(1920, 400));
        let avif = write("odd.avif", b"....ftypavif".to_vec());
        let gone = dir.path().join("gone.jpg").to_string_lossy().into_owned();

        // The wider one wins, whichever it is.
        assert_eq!(
            backdrop(Some(fanart.clone()), Some(&wide), width_of),
            Some(fanart.clone())
        );
        assert_eq!(
            backdrop(Some(fanart.clone()), Some(&wider), width_of),
            Some(wider.clone())
        );
        assert_eq!(
            backdrop(Some(old_fanart.clone()), Some(&wide), width_of),
            Some(wide.clone())
        );
        // A tie goes to the background.
        assert_eq!(
            backdrop(Some(fanart.clone()), Some(&same), width_of),
            Some(fanart.clone())
        );
        // A width nobody could read loses to one that was read, both ways.
        assert_eq!(
            backdrop(Some(avif.clone()), Some(&small), width_of),
            Some(small.clone())
        );
        assert_eq!(
            backdrop(Some(old_fanart.clone()), Some(&avif), width_of),
            Some(old_fanart.clone())
        );
        // No minimum: a narrow banner alone is still the backdrop.
        assert_eq!(backdrop(None, Some(&small), width_of), Some(small.clone()));
        assert_eq!(
            backdrop(Some(old_fanart.clone()), None, width_of),
            Some(old_fanart)
        );
        // Only what is on disk counts; nothing at all is no backdrop.
        assert_eq!(
            backdrop(Some(gone.clone()), Some(&small), width_of),
            Some(small)
        );
        assert_eq!(backdrop(Some(gone.clone()), Some(&gone), width_of), None);
        assert_eq!(backdrop(None, None, width_of), None);
    }

    #[test]
    fn numbers_in_words() {
        assert_eq!(runs(&[3]), "3");
        assert_eq!(runs(&[1, 2, 3]), "1 to 3");
        assert_eq!(runs(&[2, 4, 7]), "2, 4 and 7");
        assert_eq!(runs(&[1, 2]), "1 and 2");
    }

    #[test]
    fn two_files_of_one_episode_are_versions() {
        let a = file(
            1,
            "[TBK] Show - 01 [1080p WEB-DL H264].mkv",
            Some(1.0),
            None,
        );
        let b = file(2, "Show.S01E01.1080p.x265-ELiTE.mkv", Some(1.0), None);
        let c = file(3, "Show - 02.mkv", Some(2.0), None);
        let files = [&a, &b, &c];
        let v = episode_versions(&files);
        assert_eq!(v.len(), 1);
        let list = &v[&(1, 1.0f64.to_bits())];
        assert_eq!(list.iter().map(|x| x.file).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(list[0].label, "TBK, 1080p WEB-DL H264");
        assert_eq!(list[1].label, "ELiTE, 1080p x265");
    }

    #[test]
    fn versions_that_read_alike_fall_back_to_file_names() {
        let a = file(1, "[G] Show - 01 [1080p].mkv", Some(1.0), None);
        let b = file(2, "[G] Show - 01 [1080p] copy.mkv", Some(1.0), None);
        let v = episode_versions(&[&a, &b]);
        let list = &v[&(1, 1.0f64.to_bits())];
        assert_eq!(list[0].label, "[G] Show - 01 [1080p].mkv");
        assert_eq!(list[1].label, "[G] Show - 01 [1080p] copy.mkv");
    }

    #[test]
    fn versions_of_one_name_fall_back_to_paths() {
        // `file` puts each under /lib/Show/; one sits behind a backslash
        let a = file(1, "S1/Show - 01.mkv", Some(1.0), None);
        let b = file(2, "b\\Show - 01.mkv", Some(1.0), None);
        let c = file(3, "extra/Show - 01.mkv", Some(1.0), None);
        let v = episode_versions(&[&a, &b, &c]);
        let list = &v[&(1, 1.0f64.to_bits())];
        let labels: Vec<&str> = list.iter().map(|x| x.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "/lib/Show/S1/Show - 01.mkv",
                "/lib/Show/b\\Show - 01.mkv",
                "/lib/Show/extra/Show - 01.mkv"
            ]
        );
    }

    #[test]
    fn each_profile_sees_its_own_list_status_on_a_recommendation() {
        // Two people on one host (#852): the owner watches the recommended
        // show, the friend has finished it. Each page says its reader's own.
        let dir = tempfile::tempdir().unwrap();
        let store = anibeam_foundation::store::Store::open(&dir.path().join("anibeam.db")).unwrap();
        let sql = |batch: &'static str| {
            store
                .write(move |c| {
                    c.execute_batch(batch)?;
                    Ok(())
                })
                .unwrap()
        };
        sql("INSERT INTO profiles (id, created_at) VALUES (2, 0);
             INSERT INTO anilist_media (id, mal_id, title_romaji) VALUES (10, 110, 'Show'), (11, 111, 'Other');
             INSERT INTO recommendations (anilist_id, recommended_id, rank) VALUES (10, 11, 1);
             INSERT INTO sources (id, path, added_at) VALUES (1, '/tv', 0);
             INSERT INTO series (id, source_id, kind, path, folder_name, added_at, anilist_id) VALUES (1, 1, 'show', '/tv/Show', 'Show', 0, 10);
             INSERT INTO tracker_entries (profile_id, tracker, media_id, status, fetched_at) VALUES
                 (1, 'anilist', 11, 'watching', 0), (2, 'anilist', 11, 'completed', 0),
                 (1, 'mal', 111, 'paused', 0), (2, 'mal', 111, 'dropped', 0);");
        let status = |profile: u64, main: Tracker| {
            store
                .read(move |c| {
                    let mut snap = Snapshot::load_for(
                        c,
                        profile,
                        Path::new("/images"),
                        std::time::SystemTime::now(),
                        Some(&[1]),
                    )?;
                    snap.main_tracker = main;
                    Ok(snap.detail(c, 1)?.unwrap().recommendations[0].list_status)
                })
                .unwrap()
        };
        let owner = anibeam_foundation::trust::OWNER;
        assert_eq!(status(owner, Tracker::Anilist), Some(ListStatus::Watching));
        assert_eq!(status(2, Tracker::Anilist), Some(ListStatus::Completed));
        assert_eq!(status(owner, Tracker::Mal), Some(ListStatus::Paused));
        assert_eq!(status(2, Tracker::Mal), Some(ListStatus::Dropped));
        // Someone else's row never stands in for a reader who has none.
        sql("DELETE FROM tracker_entries WHERE profile_id = 1;");
        assert_eq!(status(owner, Tracker::Anilist), None);
        assert_eq!(status(2, Tracker::Anilist), Some(ListStatus::Completed));
        // The list kept on this device is the owner's alone.
        sql("DELETE FROM tracker_entries;
             INSERT INTO series (id, source_id, kind, path, folder_name, added_at, anilist_id) VALUES (2, 1, 'show', '/tv/Other', 'Other', 0, 11);
             INSERT INTO local_entries (series_id, status, updated_at) VALUES (2, 'planning', 0);");
        assert_eq!(status(owner, Tracker::Anilist), Some(ListStatus::Planning));
        assert_eq!(status(2, Tracker::Anilist), None);
    }

    #[test]
    fn an_episode_carries_its_size_and_the_torrent_it_came_from() {
        // About episode (#1562) reads both off the page's own record. Episode 1
        // was written after its origin was recorded, so that origin is not its.
        let dir = tempfile::tempdir().unwrap();
        let store = anibeam_foundation::store::Store::open(&dir.path().join("anibeam.db")).unwrap();
        store
            .write(|c| {
                c.execute_batch(
                    "INSERT INTO sources (id, path, added_at) VALUES (1, '/tv', 0);
                     INSERT INTO series (id, source_id, kind, path, folder_name, added_at) VALUES (1, 1, 'show', '/tv/Show', 'Show', 0);
                     INSERT INTO files (id, series_id, path, size, mtime, kind, number, label, episode_key, seen_at) VALUES
                         (1, 1, '/tv/Show/Show - 01.mkv', 1234, 0, 'episode', 1, 'Show - 01', '1', 0),
                         (2, 1, '/tv/Show/Show - 02.mkv', 5678, 0, 'episode', 2, 'Show - 02', '2', 0),
                         (3, 1, '/tv/Show/NCOP.mkv', 99, 0, 'extra', NULL, 'NCOP', 'NCOP.mkv', 0);
                     UPDATE files SET extra_kind = 'op' WHERE id = 3;
                     INSERT INTO file_origins (path, info_hash, torrent_name, recorded_at) VALUES
                         ('/tv/Show/Show - 01.mkv', 'old', '[Other] Show - 01', -1),
                         ('/tv/Show/Show - 02.mkv', 'abc', '[Group] Show - 02 [1080p]', 0),
                         ('/tv/Show/NCOP.mkv', 'def', '[Group] Show Batch', 0);",
                )?;
                Ok(())
            })
            .unwrap();
        let detail = store
            .read(|c| {
                let snap = Snapshot::load_for(
                    c,
                    anibeam_foundation::trust::OWNER,
                    Path::new("/images"),
                    std::time::SystemTime::now(),
                    Some(&[1]),
                )?;
                Ok(snap.detail(c, 1)?.unwrap())
            })
            .unwrap();
        let eps: Vec<(u64, Option<&str>)> = detail
            .episodes
            .iter()
            .map(|e| (e.size, e.torrent.as_deref()))
            .collect();
        assert_eq!(
            eps,
            [(1234, None), (5678, Some("[Group] Show - 02 [1080p]"))]
        );
        assert_eq!(detail.extras[0].size, 99);
        assert_eq!(
            detail.extras[0].torrent.as_deref(),
            Some("[Group] Show Batch")
        );
    }

    #[test]
    fn recommendations_skip_missing_series_tombstones() {
        // A removed or renamed folder leaves a missing row behind; a
        // recommendation must link to the live folder instead (#1566).
        let dir = tempfile::tempdir().unwrap();
        let store = anibeam_foundation::store::Store::open(&dir.path().join("anibeam.db")).unwrap();
        store
            .write(|c| {
                c.execute_batch(
                    "INSERT INTO sources (id, path, added_at) VALUES (1, '/tv', 0);
                     INSERT INTO anilist_media (id, title_romaji) VALUES
                         (10, 'Current'),
                         (20, 'Gone'),
                         (30, 'Renamed'),
                         (40, 'TwoLive');
                     INSERT INTO recommendations (anilist_id, recommended_id, rank) VALUES
                         (10, 20, 1),
                         (10, 30, 2),
                         (10, 40, 3);
                     INSERT INTO series (id, source_id, kind, path, folder_name, missing_since, added_at, anilist_id) VALUES
                         (1, 1, 'show', '/tv/Current', 'Current', NULL, 0, 10),
                         (2, 1, 'show', '/tv/Gone', 'Gone', 1000, 0, 20),
                         (3, 1, 'show', '/tv/Renamed-old', 'Renamed-old', 1000, 0, 30),
                         (4, 1, 'show', '/tv/Renamed-new', 'Renamed-new', NULL, 0, 30),
                         (5, 1, 'show', '/tv/TwoLive-empty', 'TwoLive-empty', NULL, 0, 40),
                         (6, 1, 'show', '/tv/TwoLive-full', 'TwoLive-full', NULL, 0, 40);
                     INSERT INTO files (id, series_id, path, size, mtime, kind, number, label, episode_key, seen_at) VALUES
                         (1, 6, '/tv/TwoLive-full/ep01.mkv', 100, 0, 'episode', 1, 'ep01', '1', 0);
                     INSERT INTO local_entries (series_id, status, updated_at) VALUES
                         (2, 'completed', 0),
                         (3, 'dropped', 0),
                         (4, 'watching', 0);",
                )?;
                Ok(())
            })
            .unwrap();
        let recs = store
            .read(|c| {
                let snap = Snapshot::load_for(
                    c,
                    anibeam_foundation::trust::OWNER,
                    Path::new("/images"),
                    std::time::SystemTime::now(),
                    Some(&[1]),
                )?;
                Ok(snap.detail(c, 1)?.unwrap().recommendations)
            })
            .unwrap();
        let summary: Vec<(u64, Option<u64>, Option<ListStatus>)> = recs
            .iter()
            .map(|r| (r.anilist_id, r.owned, r.list_status))
            .collect();
        assert_eq!(
            summary,
            [
                (20, None, Some(ListStatus::Completed)),
                (30, Some(4), Some(ListStatus::Watching)),
                (40, Some(6), None),
            ]
        );
    }
}
