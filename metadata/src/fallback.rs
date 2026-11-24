//! The episode fallback (#1461): episode details and a clear logo for a
//! season ani.zip has not mapped yet.
//!
//! ani.zip answers nothing for a season AniDB and TheTVDB have no entry for,
//! which is every new season for its first weeks. TVmaze usually has it
//! already, under the same show as the seasons before it. So the fallback
//! walks the series' sequel chain in the franchise graph, nearest season
//! first, asks ani.zip about each neighbour until one names a TheTVDB id,
//! finds the TVmaze show behind that id (`lookup/shows?thetvdb=`), and takes
//! the season the neighbour's own TheTVDB season points at: one more for each
//! season the neighbour sits before this one, one fewer for each it sits
//! after. A series with no clear logo of its own keeps the nearest season's.
//!
//! The rows live in `fallback_*`, keyed by the AniList id, and the reads merge
//! them after ani.zip's and before AniList's and Jikan's: the series page
//! (`anibeam_library::detail`) and the player's title
//! (`anibeam_playback::session`). A placeholder never wins over anything.
//! Every request goes through the shared limiter, and an outage on any of
//! them writes nothing and deletes nothing.

use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, params};

use crate::Metadata;
use crate::anizip::{self, AniZipRead};
use anibeam_contract::CoreError;
use anibeam_foundation::store::sql::as_i64;
use anibeam_net::tvmaze::TvmazeEpisode;

/// How far along the sequel chain the walk goes each way.
const MAX_HOPS: usize = 4;

/// How many franchise seasons the walk asks ani.zip about at most.
const MAX_NEIGHBOURS: usize = 6;

/// A format that counts as a season of the chain. A film or an OVA between
/// two seasons is walked through but not counted.
const SEASON_FORMATS: [&str; 3] = ["TV", "TV_SHORT", "ONA"];

/// What the series is missing from ani.zip: its regular episodes, its logo.
fn missing(conn: &Connection, anilist_id: u64) -> Result<(bool, bool), CoreError> {
    let id = as_i64(anilist_id);
    let episodes: bool = conn.query_row(
        "SELECT NOT EXISTS (SELECT 1 FROM anizip_episodes WHERE anilist_id = ?1 AND number IS NOT NULL)",
        params![id],
        |r| r.get(0),
    )?;
    let logo: bool = conn.query_row(
        "SELECT NOT EXISTS (SELECT 1 FROM anizip_images WHERE anilist_id = ?1 AND kind = 'Clearlogo')",
        params![id],
        |r| r.get(0),
    )?;
    Ok((episodes, logo))
}

/// The next season along the chain from `at`: its sequel for `step` 1, its
/// prequel for -1, a season format first. `None` when the graph names none.
fn next_in(conn: &Connection, at: u64, step: i32) -> Result<Option<(u64, bool)>, CoreError> {
    let (out, back) = if step > 0 {
        ("SEQUEL", "PREQUEL")
    } else {
        ("PREQUEL", "SEQUEL")
    };
    let mut stmt = conn.prepare(
        "SELECT n.id, m.format FROM (
            SELECT to_id AS id FROM relations WHERE from_id = ?1 AND relation = ?2
            UNION SELECT from_id FROM relations WHERE to_id = ?1 AND relation = ?3
         ) n LEFT JOIN anilist_media m ON m.id = n.id
         ORDER BY n.id",
    )?;
    let rows = stmt.query_map(params![as_i64(at), out, back], |r| {
        Ok((r.get::<_, i64>(0)? as u64, r.get::<_, Option<String>>(1)?))
    })?;
    let mut best: Option<(u64, bool)> = None;
    for row in rows {
        let (id, format) = row?;
        let season = format
            .as_deref()
            .is_none_or(|f| SEASON_FORMATS.contains(&f));
        match best {
            Some((_, true)) => {}
            Some((_, false)) if !season => {}
            _ => best = Some((id, season)),
        }
    }
    Ok(best)
}

/// Whether the graph still owes `id` its own edges.
fn edges_owed(conn: &Connection, id: u64) -> Result<bool, CoreError> {
    Ok(conn
        .query_row(
            "SELECT relations_fetched_at IS NULL FROM anilist_media WHERE id = ?1",
            params![as_i64(id)],
            |r| r.get::<_, bool>(0),
        )
        .optional()?
        .unwrap_or(true))
}

/// Why a walk stopped short of an answer.
enum Stop {
    /// A provider is down: nothing is written.
    Outage,
}

/// The franchise seasons around `start`, nearest first and the prequel
/// before the sequel at the same distance, each with its place relative to
/// `start`: -1 for the season before, 2 for the one after the next. A node
/// whose edges the graph does not hold yet is asked of AniList once, the
/// way the crawl asks.
async fn chain(metadata: &Metadata, start: u64, now: i64) -> Result<Vec<(u64, i32)>, Stop> {
    let mut seen: HashSet<u64> = HashSet::from([start]);
    let mut out: Vec<(u64, i32, usize)> = Vec::new();
    for step in [-1i32, 1] {
        let mut at = start;
        let mut seasons = 0i32;
        for hop in 0..MAX_HOPS {
            let mut next = read(metadata, move |c| next_in(c, at, step));
            if next.is_none() && at != start && read(metadata, move |c| edges_owed(c, at)) {
                match metadata.net.anilist.enrichment_raw(at).await {
                    Ok((Some(enrichment), _)) => {
                        if crate::franchise::crawl::write_node(
                            &metadata.services,
                            at,
                            &enrichment,
                            now,
                        )
                        .await
                        .is_err()
                        {
                            break;
                        }
                        next = read(metadata, move |c| next_in(c, at, step));
                    }
                    Ok((None, _)) => {}
                    Err(e) if crate::team::is_outage(&e) => return Err(Stop::Outage),
                    Err(_) => {}
                }
            }
            let Some((id, season)) = next else {
                break;
            };
            if !seen.insert(id) {
                break;
            }
            if season {
                seasons += 1;
                out.push((id, step * seasons, hop));
            }
            at = id;
        }
    }
    out.sort_by_key(|(_, place, hop)| (place.unsigned_abs(), *hop, *place > 0));
    Ok(out
        .into_iter()
        .take(MAX_NEIGHBOURS)
        .map(|(id, place, _)| (id, place))
        .collect())
}

/// A read off the store that cannot fail the walk: an error reads as nothing.
fn read<T: Default + Send + 'static>(
    metadata: &Metadata,
    f: impl FnOnce(&Connection) -> Result<T, CoreError> + Send + 'static,
) -> T {
    metadata.services.store.read(f).unwrap_or_default()
}

/// What ani.zip holds for one franchise season, once asked.
#[derive(Default)]
struct Neighbour {
    tvdb_id: Option<u64>,
    /// The first and last TheTVDB season its regular episodes sit in.
    seasons: Option<(u32, u32)>,
    logo: Option<String>,
}

fn neighbour(conn: &Connection, id: u64) -> Result<Neighbour, CoreError> {
    let id = as_i64(id);
    let tvdb_id: Option<i64> = conn
        .query_row(
            "SELECT tvdb_id FROM anizip_shows WHERE anilist_id = ?1",
            params![id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let seasons: (Option<i64>, Option<i64>) = conn.query_row(
        "SELECT min(season), max(season) FROM anizip_episodes
         WHERE anilist_id = ?1 AND number IS NOT NULL AND season > 0",
        params![id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let logo: Option<String> = conn
        .query_row(
            "SELECT url FROM anizip_images WHERE anilist_id = ?1 AND kind = 'Clearlogo'
             ORDER BY CASE WHEN lower(lang) LIKE 'en%' THEN 0 WHEN lang IS NULL THEN 1 ELSE 2 END, rank
             LIMIT 1",
            params![id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(Neighbour {
        tvdb_id: tvdb_id.filter(|v| *v > 0).map(|v| v as u64),
        seasons: match seasons {
            (Some(first), Some(last)) => Some((first as u32, last as u32)),
            _ => None,
        },
        logo,
    })
}

/// Whether ani.zip was asked about `id` already.
fn asked(conn: &Connection, id: u64) -> Result<bool, CoreError> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM anizip_shows WHERE anilist_id = ?1)",
        params![as_i64(id)],
        |r| r.get(0),
    )?)
}

/// The TheTVDB season `place` seasons away from a neighbour holding
/// `seasons`: past its last for a season after it, before its first for a
/// season before it.
fn season_for(seasons: (u32, u32), place: i32) -> Option<u32> {
    let (first, last) = seasons;
    let target = if place < 0 {
        i64::from(last) + i64::from(place.unsigned_abs())
    } else {
        i64::from(first) - i64::from(place.unsigned_abs())
    };
    u32::try_from(target).ok().filter(|s| *s > 0)
}

/// The TVmaze show and season the walk found, the season it came through.
struct Found {
    via: u64,
    tvmaze_id: u64,
    season: u32,
    episodes: Vec<TvmazeEpisode>,
}

/// Runs the fallback for `anilist_id` when ani.zip leaves it without regular
/// episodes or without a logo. Returns whether any row changed. An outage on
/// any request writes nothing; a walk that found nothing records that it
/// looked and keeps whatever an earlier walk wrote.
pub async fn run(metadata: &Metadata, anilist_id: u64, now: i64) -> Result<bool, CoreError> {
    let (want_episodes, want_logo) = metadata
        .services
        .store
        .read(move |c| missing(c, anilist_id))?;
    if !want_episodes && !want_logo {
        return Ok(false);
    }
    let Ok(neighbours) = chain(metadata, anilist_id, now).await else {
        return Ok(false);
    };
    let mut logo: Option<String> = None;
    let mut target: Option<(u64, u64, u32)> = None;
    for (id, place) in neighbours {
        if (!want_logo || logo.is_some()) && (!want_episodes || target.is_some()) {
            break;
        }
        if !read(metadata, move |c| asked(c, id)) {
            let answer = metadata.anizip_show(id).await;
            if answer == AniZipRead::Failed {
                if metadata.anizip_down() {
                    return Ok(false);
                }
                continue;
            }
            metadata
                .services
                .store
                .tx_async(move |tx| anizip::write(tx, id, &answer, now))
                .await?;
        }
        let n = read(metadata, move |c| neighbour(c, id));
        if want_logo && logo.is_none() {
            logo = n.logo.clone();
        }
        if want_episodes
            && target.is_none()
            && let (Some(tvdb), Some(seasons)) = (n.tvdb_id, n.seasons)
            && let Some(season) = season_for(seasons, place)
        {
            target = Some((id, tvdb, season));
        }
    }
    let mut found: Option<Found> = None;
    if let Some((via, tvdb, season)) = target {
        let show = match metadata.net.tvmaze.lookup_thetvdb(tvdb).await {
            Ok(Some(show_id)) => metadata.net.tvmaze.show(show_id).await,
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        };
        match show {
            Ok(Some(show)) => {
                let episodes: Vec<TvmazeEpisode> = show
                    .episodes()
                    .into_iter()
                    .filter(|e| e.season == season)
                    .collect();
                found = Some(Found {
                    via,
                    tvmaze_id: show.id,
                    season,
                    episodes,
                });
            }
            Ok(None) => {}
            Err(e) if crate::team::is_outage(&e) => return Ok(false),
            Err(e) => tracing::debug!("the TVmaze fallback for AniList {anilist_id} failed: {e}"),
        }
    }
    metadata
        .services
        .store
        .tx_async(move |tx| {
            let before = fingerprint(tx, anilist_id)?;
            write(tx, anilist_id, found.as_ref(), logo.as_deref(), now)?;
            Ok(before != fingerprint(tx, anilist_id)?)
        })
        .await
}

/// Writes what the walk found. Episodes replace the show's earlier ones only
/// when there are some; a logo replaces the earlier one only when one was
/// found; the stamp always moves.
fn write(
    tx: &rusqlite::Transaction,
    anilist_id: u64,
    found: Option<&Found>,
    logo: Option<&str>,
    now: i64,
) -> Result<(), CoreError> {
    let id = as_i64(anilist_id);
    tx.execute(
        "INSERT INTO fallback_shows (anilist_id, fetched_at) VALUES (?1, ?2)
         ON CONFLICT(anilist_id) DO UPDATE SET fetched_at = excluded.fetched_at",
        params![id, now],
    )?;
    if let Some(url) = logo {
        tx.execute(
            "UPDATE fallback_shows SET logo_url = ?2 WHERE anilist_id = ?1",
            params![id, url],
        )?;
    }
    let Some(found) = found.filter(|f| !f.episodes.is_empty()) else {
        return Ok(());
    };
    tx.execute(
        "UPDATE fallback_shows SET via_anilist = ?2, tvmaze_id = ?3, season = ?4 WHERE anilist_id = ?1",
        params![id, as_i64(found.via), as_i64(found.tvmaze_id), found.season],
    )?;
    tx.execute(
        "DELETE FROM fallback_episodes WHERE anilist_id = ?1",
        params![id],
    )?;
    let mut insert = tx.prepare(
        "INSERT OR IGNORE INTO fallback_episodes (anilist_id, number, title, synopsis, still_url, runtime, aired_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    for e in &found.episodes {
        let synopsis = e
            .summary
            .as_deref()
            .map(anibeam_foundation::html::plain_text)
            .filter(|s| !s.trim().is_empty());
        insert.execute(params![
            id,
            e.number,
            anizip::real_title(e.name.as_deref()),
            synopsis,
            e.image.as_deref().filter(|u| u.starts_with("https://")),
            e.runtime,
            e.airstamp,
        ])?;
    }
    Ok(())
}

/// Every fallback row behind `anilist_id`, as one string, so a caller can
/// tell whether a run changed anything.
pub fn fingerprint(conn: &Connection, anilist_id: u64) -> Result<String, CoreError> {
    Ok(conn.query_row(
        "SELECT coalesce((SELECT logo_url FROM fallback_shows WHERE anilist_id = ?1), '') || '#' ||
                coalesce((SELECT group_concat(number || '|' || coalesce(title, '') || '|' || coalesce(still_url, '') || '|' || coalesce(synopsis, '') || '|' || coalesce(aired_at, ''), ';')
                          FROM (SELECT * FROM fallback_episodes WHERE anilist_id = ?1 ORDER BY number)), '')",
        params![as_i64(anilist_id)],
        |r| r.get(0),
    )?)
}

/// Whether the airing refresh should run the fallback again: ani.zip still
/// has no episodes for the show, and the last walk is older than the airing
/// window or there was none.
pub fn due(conn: &Connection, anilist_id: u64, now: i64) -> Result<bool, CoreError> {
    let (want_episodes, _) = missing(conn, anilist_id)?;
    if !want_episodes {
        return Ok(false);
    }
    let fetched_at: Option<i64> = conn
        .query_row(
            "SELECT fetched_at FROM fallback_shows WHERE anilist_id = ?1",
            params![as_i64(anilist_id)],
            |r| r.get(0),
        )
        .optional()?;
    let window = i64::try_from(crate::airing::TTL.as_secs()).unwrap_or(i64::MAX);
    Ok(fetched_at.is_none_or(|at| now.saturating_sub(at) >= window))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_season_counts_from_the_neighbours_edge() {
        // The season before this one sits in TheTVDB season 2: this is 3.
        assert_eq!(season_for((2, 2), -1), Some(3));
        // Two seasons back, spread over seasons 1 and 2: this is 4.
        assert_eq!(season_for((1, 2), -2), Some(4));
        // The season after this one is TheTVDB season 3: this is 2.
        assert_eq!(season_for((3, 3), 1), Some(2));
        // Nothing before season 1.
        assert_eq!(season_for((1, 1), 1), None);
    }
}
