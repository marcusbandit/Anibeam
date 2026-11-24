//! ani.zip, the episode side-fetch by AniList id (#1036): every episode's
//! title, synopsis, still, runtime, air date and TheTVDB numbering, the
//! show's artwork, and its MAL id.
//!
//! The rows live in their own tables (`anizip_*`), keyed by the AniList id,
//! and the reads merge them in: an ani.zip title wins over AniList's
//! streaming title, which wins over Jikan's (`record::merge_episodes`), and a
//! placeholder such as `Episode 5` never wins over anything. Keeping them
//! apart is what lets an ani.zip outage cost nothing: a fetch that failed
//! writes nothing, so every row the last answer wrote stays, and AniList's
//! and Jikan's own rows never depended on it.
//!
//! A match fetches it beside AniList (`fetch::fetch_and_write`). The series
//! matched before ani.zip was asked are walked once at launch (`start`), one
//! request each, and a walk that finds ani.zip down stops and keeps the rest
//! for the next launch.

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
/// The mapping's MAL id fills the media row's and the series' own where
/// AniList had none, and never replaces one AniList gave.
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
    if let Some(mal_id) = show.mal_id {
        tx.execute(
            "UPDATE anilist_media SET mal_id = ?2 WHERE id = ?1 AND mal_id IS NULL",
            params![id, as_i64(mal_id)],
        )?;
        tx.execute(
            "UPDATE series SET mal_id = ?2 WHERE anilist_id = ?1 AND provider = 'anilist' AND mal_id IS NULL",
            params![id, as_i64(mal_id)],
        )?;
    }
    Ok(())
}

/// The images a series wants fetched now: its clear logo, and the still of
/// every episode it has a file of. The rest of the artwork, and the stills
/// of episodes not on disk, wait until something asks for them.
pub fn image_urls(conn: &Connection, series: u64) -> Result<Vec<String>, CoreError> {
    let mut stmt = conn.prepare(
        "SELECT url FROM (
            SELECT i.url, 0 AS part, CASE WHEN lower(i.lang) LIKE 'en%' THEN 0 WHEN i.lang IS NULL THEN 1 ELSE 2 END AS pick, i.rank
            FROM anizip_images i JOIN series s ON s.anilist_id = i.anilist_id
            WHERE s.id = ?1 AND s.provider = 'anilist' AND i.kind = 'Clearlogo'
            ORDER BY pick, i.rank LIMIT 1
         )
         UNION
         SELECT DISTINCT z.still_url FROM anizip_episodes z
            JOIN series s ON s.anilist_id = z.anilist_id
            JOIN files f ON f.series_id = s.id AND f.kind != 'extra' AND f.number = z.number
         WHERE s.id = ?1 AND s.provider = 'anilist' AND z.still_url IS NOT NULL",
    )?;
    let rows = stmt.query_map(params![as_i64(series)], |r| r.get::<_, String>(0))?;
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
                let filled = walk(&owner, &ctx).await?;
                Ok(Finished {
                    plain: None,
                    level: if filled == 0 {
                        Level::Debug
                    } else {
                        Level::Info
                    },
                    message: format!("ani.zip episode details filled for {filled} series"),
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
/// many ani.zip answered for. A show it cannot answer for right now, an
/// outage, ends the walk with the rest still unasked.
async fn walk(metadata: &Metadata, ctx: &Arc<JobCtx>) -> Result<u64, CoreError> {
    let work = metadata.services.store.write_async(|c| unasked(c)).await?;
    let total = work.len() as u64;
    let mut filled = 0u64;
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
                let before = mal_of(tx, anilist_id)?;
                write(tx, anilist_id, &read, now)?;
                let after = mal_of(tx, anilist_id)?;
                // A MAL id just found is one Jikan was never asked about,
                // so the series is owed Jikan's titles now (#1065).
                let found = before.is_none() && after.is_some();
                if found {
                    crate::owed::mark(tx, anilist_id, true)?;
                }
                Ok(found)
            })
            .await?;
        if !answered {
            continue;
        }
        filled += 1;
        for id in series {
            let urls = metadata
                .services
                .store
                .write_async(move |c| image_urls(c, id))
                .await?;
            metadata.net.images.ensure(&urls).await;
            if let Some(card) = card_for(&metadata.services, id).await? {
                ctx.changed(card);
            }
        }
        if filled_mal && let Some(this) = metadata.this() {
            crate::owed::start(&this);
        }
    }
    Ok(filled)
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

fn mal_of(conn: &Connection, anilist_id: u64) -> Result<Option<i64>, CoreError> {
    use rusqlite::OptionalExtension;
    Ok(conn
        .query_row(
            "SELECT mal_id FROM anilist_media WHERE id = ?1",
            params![as_i64(anilist_id)],
            |r| r.get::<_, Option<i64>>(0),
        )
        .optional()?
        .flatten())
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
