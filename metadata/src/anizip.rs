//! ani.zip, the episode side-fetch by AniList id (#1036): every episode's
//! title, synopsis, still, runtime, air date and TheTVDB numbering, the
//! show's artwork, and the MAL id its community mapping names.
//!
//! The rows live in their own tables (`anizip_*`), keyed by the AniList id,
//! and the reads merge them in at read time: the series page
//! (`anibeam_library::detail`) and the player's title
//! (`anibeam_playback::session`) take an ani.zip title over the
//! `anilist_episodes` one, where AniList's streaming title already beat
//! Jikan's, and a placeholder such as `Episode 5` never wins over anything.
//! Keeping them apart is what lets an ani.zip outage cost nothing: a fetch
//! that failed writes nothing, so every row the last answer wrote stays, and
//! AniList's and Jikan's own rows never depended on it.
//!
//! The mapped MAL id is only ever used to ask Jikan for titles. It never
//! reaches `series.mal_id` or `anilist_media.mal_id`, which the trackers
//! write MAL lists through: a community mapping is not proof enough for
//! that, and the MAL search (#782) checks its own answers.
//!
//! A match fetches it beside AniList (`fetch::fetch_and_write`), and the
//! airing refresh asks again for a releasing show whose answer is older than
//! the airing window, so episodes that air later get their rows too. The
//! series matched before ani.zip was asked are walked once at launch
//! (`start`), one request each, and a walk that finds ani.zip down stops and
//! keeps the rest for the next launch.

use std::sync::atomic::Ordering;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use regex::Regex;
use rusqlite::{Connection, Transaction, params};

use crate::Metadata;
use crate::apply::card_for;
use crate::fetch::message_of;
use anibeam_contract::*;
use anibeam_foundation::jobs::{Finished, JobCtx};
use anibeam_foundation::store::sql::as_i64;
use anibeam_foundation::sync::recover;
use anibeam_net::anizip::{AniZipEpisode, AniZipShow};

/// What ani.zip writes when it has no title, in English or romaji.
static PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:(?:episode|ep\.?|special|sp\.?)\s*\d+(?:\.\d+)?|tba|tbd|n/a)$").unwrap()
});

/// What one ask of ani.zip came back with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AniZipRead {
    /// ani.zip answered with the show.
    Answered(AniZipShow),
    /// ani.zip answered that it has no entry for the id.
    Missing,
    /// No answer: it failed, or it went down inside the outage window and
    /// was not asked. Nothing is written on its word.
    Failed,
}

impl AniZipRead {
    /// The mapping's MAL id, when ani.zip answered with one.
    pub fn mal_id(&self) -> Option<u64> {
        match self {
            AniZipRead::Answered(show) => show.mal_id,
            _ => None,
        }
    }
}

/// A title that is a title: trimmed, and never a placeholder.
pub fn real_title(title: Option<&str>) -> Option<String> {
    title
        .map(str::trim)
        .filter(|t| !t.is_empty() && !PLACEHOLDER.is_match(t))
        .map(str::to_string)
}

/// The title an episode row keeps: the English one, else the romaji one.
pub fn title_of(e: &AniZipEpisode) -> Option<String> {
    real_title(e.title_en.as_deref()).or_else(|| real_title(e.title_romaji.as_deref()))
}

impl Metadata {
    /// ani.zip's answer for `anilist_id`, or `Failed` without asking while
    /// it is inside the outage window. Only a failure that looks like
    /// ani.zip being down opens the window; one show's bad answer costs that
    /// show and nothing else.
    pub async fn anizip_show(&self, anilist_id: u64) -> AniZipRead {
        if self.anizip_down() {
            return AniZipRead::Failed;
        }
        match self.net.anizip.show(anilist_id).await {
            Ok(Some(show)) => {
                self.anizip_back();
                AniZipRead::Answered(show)
            }
            Ok(None) => {
                self.anizip_back();
                AniZipRead::Missing
            }
            Err(e) => {
                if crate::team::is_outage(&e) {
                    self.report_anizip_outage(&message_of(&e));
                } else {
                    tracing::debug!("ani.zip for AniList {anilist_id} did not read: {e}");
                }
                AniZipRead::Failed
            }
        }
    }

    /// Whether ani.zip failed inside the outage window.
    pub(crate) fn anizip_down(&self) -> bool {
        let window = self.anizip_window();
        recover(&self.anizip_outage).is_some_and(|at| at.elapsed() < window)
    }

    fn anizip_window(&self) -> Duration {
        Duration::from_millis(self.anizip_window_ms.load(Ordering::SeqCst))
    }

    /// An answer closes a window that has passed.
    fn anizip_back(&self) {
        let window = self.anizip_window();
        let mut last = recover(&self.anizip_outage);
        if last.is_some_and(|at| at.elapsed() >= window) {
            *last = None;
        }
    }

    /// Says ani.zip is not answering, at most once a window, and starts the
    /// window in which it is not asked.
    fn report_anizip_outage(&self, message: &str) {
        let window = self.anizip_window();
        let mut last = recover(&self.anizip_outage);
        if last.is_some_and(|at| at.elapsed() < window) {
            return;
        }
        *last = Some(Instant::now());
        drop(last);
        self.services.bus.warn(
            Stage::Metadata,
            format!("ani.zip is not answering: {message}"),
            EventBody::Notice,
        );
    }
}

/// Writes what ani.zip said about `anilist_id`. An answer replaces the
/// show's episodes and its artwork, except that an empty list replaces
/// nothing: that is a reply with nothing in it, not a show that lost its
/// episodes. A show ani.zip has no entry for is written down as asked and
/// keeps whatever an earlier answer left. A failure writes nothing at all.
///
/// The mapping's MAL id is kept on `anizip_shows` only, for Jikan.
pub fn write(
    tx: &Transaction,
    anilist_id: u64,
    read: &AniZipRead,
    now: i64,
) -> Result<(), CoreError> {
    let id = as_i64(anilist_id);
    let show = match read {
        AniZipRead::Failed => return Ok(()),
        AniZipRead::Missing => {
            tx.execute(
                "INSERT INTO anizip_shows (anilist_id, found, fetched_at) VALUES (?1, 0, ?2)
                 ON CONFLICT(anilist_id) DO UPDATE SET found = 0, fetched_at = excluded.fetched_at",
                params![id, now],
            )?;
            return Ok(());
        }
        AniZipRead::Answered(show) => show,
    };
    tx.execute(
        "INSERT INTO anizip_shows (anilist_id, found, mal_id, anidb_id, tvdb_id, fetched_at)
         VALUES (?1, 1, ?2, ?3, ?4, ?5)
         ON CONFLICT(anilist_id) DO UPDATE SET
            found = 1, mal_id = excluded.mal_id, anidb_id = excluded.anidb_id,
            tvdb_id = excluded.tvdb_id, fetched_at = excluded.fetched_at",
        params![
            id,
            show.mal_id.map(as_i64),
            show.anidb_id.map(as_i64),
            show.tvdb_id.map(as_i64),
            now
        ],
    )?;
    if show.episodes.iter().any(|e| e.number.is_some()) {
        // ani.zip has the season now: the fallback's episodes give way
        // rather than fill its gaps with another source's guesses (#1461).
        tx.execute(
            "DELETE FROM fallback_episodes WHERE anilist_id = ?1",
            params![id],
        )?;
        tx.execute(
            "UPDATE fallback_shows SET via_anilist = NULL, tvmaze_id = NULL, season = NULL WHERE anilist_id = ?1",
            params![id],
        )?;
    }
    if !show.episodes.is_empty() {
        tx.execute(
            "DELETE FROM anizip_episodes WHERE anilist_id = ?1",
            params![id],
        )?;
        let mut insert = tx.prepare(
            "INSERT OR IGNORE INTO anizip_episodes
                (anilist_id, key, number, season, episode, absolute, title, synopsis, still_url, runtime, air_date, aired_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        )?;
        for e in &show.episodes {
            insert.execute(params![
                id,
                e.key,
                e.number,
                e.season,
                e.episode,
                e.absolute,
                title_of(e),
                e.synopsis,
                e.still_url,
                e.runtime,
                e.air_date,
                e.aired_at,
            ])?;
        }
    }
    if !show.images.is_empty() {
        tx.execute(
            "DELETE FROM anizip_images WHERE anilist_id = ?1",
            params![id],
        )?;
        let mut insert = tx.prepare(
            "INSERT OR IGNORE INTO anizip_images (anilist_id, kind, url, lang, rank) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for (rank, image) in show.images.iter().enumerate() {
            insert.execute(params![id, image.kind, image.url, image.lang, rank as i64])?;
        }
    }
    Ok(())
}

/// Every episode row and artwork row behind `anilist_id`, as one string, so
/// a refresh can tell whether an answer changed anything.
pub fn fingerprint(conn: &Connection, anilist_id: u64) -> Result<String, CoreError> {
    Ok(conn.query_row(
        "SELECT coalesce((SELECT group_concat(key || '|' || coalesce(title, '') || '|' || coalesce(still_url, '') || '|' || coalesce(aired_at, air_date, ''), ';')
                          FROM (SELECT * FROM anizip_episodes WHERE anilist_id = ?1 ORDER BY key)), '')
                || '#' ||
                coalesce((SELECT group_concat(url, ';') FROM (SELECT url FROM anizip_images WHERE anilist_id = ?1 ORDER BY rank)), '')",
        params![as_i64(anilist_id)],
        |r| r.get(0),
    )?)
}

/// Whether the airing refresh should ask ani.zip again for `anilist_id`:
/// it was asked once, whatever it said, and that answer is older than the
/// airing window. A season it had no entry for is asked again too, so once
/// ani.zip maps it its answer takes over from the fallback (#1461). A show
/// it was never asked about is the launch walk's.
pub fn due(conn: &Connection, anilist_id: u64, now: i64) -> Result<bool, CoreError> {
    use rusqlite::OptionalExtension;
    let fetched_at: Option<i64> = conn
        .query_row(
            "SELECT fetched_at FROM anizip_shows WHERE anilist_id = ?1",
            params![as_i64(anilist_id)],
            |r| r.get(0),
        )
        .optional()?;
    let window = i64::try_from(crate::airing::TTL.as_secs()).unwrap_or(i64::MAX);
    Ok(fetched_at.is_some_and(|at| now.saturating_sub(at) >= window))
}

/// The images a show wants fetched now: its one clear logo, English first,
/// else the logo the fallback borrowed from its franchise (#1461), its first
/// background, which the hero and the series head draw (#1475), and the
/// still of every episode a series matched to it has a file of, ani.zip's or
/// the fallback's. The rest of the artwork, and the stills of episodes not
/// on disk, wait until something asks for them.
pub fn image_urls(conn: &Connection, anilist_id: u64) -> Result<Vec<String>, CoreError> {
    let mut stmt = conn.prepare(
        "SELECT url FROM (
            SELECT i.url FROM anizip_images i
            WHERE i.anilist_id = ?1 AND i.kind = 'Clearlogo'
              AND EXISTS (SELECT 1 FROM series s WHERE s.anilist_id = i.anilist_id AND s.provider = 'anilist')
            ORDER BY CASE WHEN lower(i.lang) LIKE 'en%' THEN 0 WHEN i.lang IS NULL THEN 1 ELSE 2 END, i.rank
            LIMIT 1
         )
         UNION
         SELECT url FROM (
            SELECT i.url FROM anizip_images i
            WHERE i.anilist_id = ?1 AND i.kind = 'Fanart'
              AND EXISTS (SELECT 1 FROM series s WHERE s.anilist_id = i.anilist_id AND s.provider = 'anilist')
            ORDER BY i.rank
            LIMIT 1
         )
         UNION
         SELECT z.still_url FROM anizip_episodes z
            JOIN series s ON s.anilist_id = z.anilist_id AND s.provider = 'anilist'
            JOIN files f ON f.series_id = s.id AND f.kind != 'extra' AND f.number = z.number
         WHERE z.anilist_id = ?1 AND z.still_url IS NOT NULL
         UNION
         SELECT b.logo_url FROM fallback_shows b
         WHERE b.anilist_id = ?1 AND b.logo_url IS NOT NULL
           AND EXISTS (SELECT 1 FROM series s WHERE s.anilist_id = b.anilist_id AND s.provider = 'anilist')
           AND NOT EXISTS (SELECT 1 FROM anizip_images i WHERE i.anilist_id = b.anilist_id AND i.kind = 'Clearlogo')
         UNION
         SELECT e.still_url FROM fallback_episodes e
            JOIN series s ON s.anilist_id = e.anilist_id AND s.provider = 'anilist'
            JOIN files f ON f.series_id = s.id AND f.kind != 'extra' AND f.number = e.number
         WHERE e.anilist_id = ?1 AND e.still_url IS NOT NULL
           AND NOT EXISTS (SELECT 1 FROM anizip_episodes ze2 WHERE ze2.anilist_id = e.anilist_id AND ze2.number IS NOT NULL)",
    )?;
    let rows = stmt.query_map(params![as_i64(anilist_id)], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Starts the walk over the matched series ani.zip has never been asked
/// about, unless one is running or there are none. Returns the job's id,
/// or `None` when nothing started.
pub fn start(metadata: &Arc<Metadata>) -> Option<u64> {
    let any = metadata
        .services
        .store
        .read(|c| Ok(!unasked(c)?.is_empty()))
        .unwrap_or(false);
    if !any || metadata.anizip_walking.swap(true, Ordering::SeqCst) {
        return None;
    }
    let walking = Walking(metadata.clone());
    let owner = metadata.clone();
    Some(
        metadata
            .services
            .jobs
            .clone()
            .start(JobKind::RefreshAiring, move |ctx| async move {
                let _walking = walking;
                let (filled, fell_back) = walk(&owner, &ctx).await?;
                Ok(Finished {
                    plain: None,
                    level: if filled + fell_back == 0 {
                        Level::Debug
                    } else {
                        Level::Info
                    },
                    message: format!(
                        "ani.zip filled episode details for {filled} series, the franchise fallback for {fell_back} more"
                    ),
                    body: EventBody::Notice,
                })
            }),
    )
}

/// Holds `anizip_walking` up for as long as the walk's job body lives.
struct Walking(Arc<Metadata>);

impl Drop for Walking {
    fn drop(&mut self) {
        self.0.anizip_walking.store(false, Ordering::SeqCst);
    }
}

/// The AniList ids a series is matched to that ani.zip was never asked
/// about, each with the series on it and the name the progress line shows.
fn unasked(conn: &Connection) -> Result<Vec<(u64, Vec<u64>, String)>, CoreError> {
    let mut stmt = conn.prepare(
        "SELECT s.anilist_id, s.id, s.folder_name FROM series s
         WHERE s.provider = 'anilist' AND s.anilist_id IS NOT NULL
           AND NOT EXISTS (SELECT 1 FROM anizip_shows z WHERE z.anilist_id = s.anilist_id)
         ORDER BY s.anilist_id, s.id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)? as u64,
            r.get::<_, i64>(1)? as u64,
            r.get::<_, String>(2)?,
        ))
    })?;
    let mut out: Vec<(u64, Vec<u64>, String)> = Vec::new();
    for row in rows {
        let (anilist_id, series, name) = row?;
        match out.last_mut() {
            Some((id, list, _)) if *id == anilist_id => list.push(series),
            _ => out.push((anilist_id, vec![series], name)),
        }
    }
    Ok(out)
}

/// Every unasked show, one request each at the limiter's pace. Returns how
/// many ani.zip answered for, and how many more the franchise fallback
/// filled where ani.zip did not. A show it cannot answer for right now, an
/// outage, ends the walk with the rest still unasked.
async fn walk(metadata: &Metadata, ctx: &Arc<JobCtx>) -> Result<(u64, u64), CoreError> {
    let work = metadata.services.store.write_async(|c| unasked(c)).await?;
    let total = work.len() as u64;
    let mut filled = 0u64;
    let mut fell_back_for = 0u64;
    let mut owed_any = false;
    for (done, (anilist_id, series, name)) in work.into_iter().enumerate() {
        ctx.checkpoint()?;
        ctx.progress(done as u64, Some(total), &name);
        let read = metadata.anizip_show(anilist_id).await;
        if read == AniZipRead::Failed {
            if metadata.anizip_down() {
                break;
            }
            continue;
        }
        let answered = matches!(read, AniZipRead::Answered(_));
        let now = anibeam_foundation::time::now_secs();
        let filled_mal = metadata
            .services
            .store
            .tx_async(move |tx| {
                let before = jikan_mal(tx, anilist_id)?;
                write(tx, anilist_id, &read, now)?;
                let after = jikan_mal(tx, anilist_id)?;
                // A MAL id just found is one Jikan was never asked about,
                // so the series is owed Jikan's titles now (#1065).
                let found = before.is_none() && after.is_some();
                if found {
                    crate::owed::mark(tx, anilist_id, true)?;
                }
                Ok(found)
            })
            .await?;
        owed_any |= filled_mal;
        // What ani.zip left out, the franchise around it may have (#1461).
        let fell_back = crate::fallback::run(metadata, anilist_id, now, Some(ctx.as_ref())).await?;
        if !answered && !fell_back {
            continue;
        }
        if answered {
            filled += 1;
        } else {
            fell_back_for += 1;
        }
        let urls = metadata
            .services
            .store
            .write_async(move |c| image_urls(c, anilist_id))
            .await?;
        metadata.net.images.ensure(&urls).await;
        for id in series {
            if let Some(card) = card_for(&metadata.services, id).await? {
                ctx.changed(card);
            }
        }
    }
    // Once, after every id is written down. A walk already running from the
    // launch reads the list again before it ends, so it takes these too.
    if owed_any && let Some(this) = metadata.this() {
        crate::owed::start(&this);
    }
    Ok((filled, fell_back_for))
}

/// The MAL id an earlier ani.zip answer mapped `anilist_id` to.
pub fn kept_mal(conn: &Connection, anilist_id: u64) -> Result<Option<u64>, CoreError> {
    use rusqlite::OptionalExtension;
    Ok(conn
        .query_row(
            "SELECT mal_id FROM anizip_shows WHERE anilist_id = ?1",
            params![as_i64(anilist_id)],
            |r| r.get::<_, Option<i64>>(0),
        )
        .optional()?
        .flatten()
        .map(|v| v as u64))
}

/// The MAL id Jikan is asked with for `anilist_id`: AniList's own, else
/// the one ani.zip mapped it to.
pub fn jikan_mal(conn: &Connection, anilist_id: u64) -> Result<Option<i64>, CoreError> {
    Ok(conn.query_row(
        "SELECT coalesce(
            (SELECT mal_id FROM anilist_media WHERE id = ?1),
            (SELECT mal_id FROM anizip_shows WHERE anilist_id = ?1))",
        params![as_i64(anilist_id)],
        |r| r.get::<_, Option<i64>>(0),
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_are_no_title() {
        assert_eq!(real_title(Some("Episode 5")), None);
        assert_eq!(real_title(Some("ep. 12")), None);
        assert_eq!(real_title(Some("Special 1")), None);
        assert_eq!(real_title(Some("TBA")), None);
        assert_eq!(real_title(Some("  ")), None);
        assert_eq!(
            real_title(Some(" Episode 5: The Return ")).as_deref(),
            Some("Episode 5: The Return")
        );
        assert_eq!(real_title(Some("The Storm")).as_deref(), Some("The Storm"));
    }

    #[test]
    fn the_english_title_wins_and_a_placeholder_falls_to_romaji() {
        let e = |en: Option<&str>, romaji: Option<&str>| AniZipEpisode {
            title_en: en.map(str::to_string),
            title_romaji: romaji.map(str::to_string),
            ..AniZipEpisode::default()
        };
        assert_eq!(
            title_of(&e(Some("The Storm"), Some("Arashi"))).as_deref(),
            Some("The Storm")
        );
        assert_eq!(
            title_of(&e(Some("Episode 2"), Some("Arashi"))).as_deref(),
            Some("Arashi")
        );
        assert_eq!(title_of(&e(Some("Episode 2"), Some("Episode 2"))), None);
    }
}
