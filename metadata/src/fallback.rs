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
//! after, or the rest of the neighbour's own season when the neighbour is the
//! first cour of a split season. A series with no clear logo of its own keeps
//! a franchise season's, walking back the chain first and only then forward
//! (#1463): an earlier season's logo is the show's name, where a later one may
//! carry its own season's.
//!
//! Another season's episodes on a page are worse than none, so the fallback
//! writes episodes only when it is sure: the series has to be a TV season
//! itself, and the TVmaze season it picked has to start within a few weeks of
//! the entry's own start date and hold about as many episodes as AniList says
//! the entry has. Anything less and it writes no episodes.
//!
//! The rows live in `fallback_*`, keyed by the AniList id, and the reads merge
//! them after ani.zip's and before AniList's and Jikan's, and only while
//! ani.zip has no regular episodes for the series: the series page
//! (`anibeam_library::detail`) and the player's title
//! (`anibeam_playback::session`). Once ani.zip answers with episodes its
//! write deletes the fallback's (`anizip::write`). A placeholder never wins
//! over anything. Every request goes through the shared limiter, and an
//! outage on any of them writes nothing and deletes nothing.

use std::collections::HashSet;
use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension, params};

use crate::Metadata;
use crate::anizip::{self, AniZipRead};
use crate::franchise::crawl;
use anibeam_contract::*;
use anibeam_foundation::jobs::{Finished, JobCtx};
use anibeam_foundation::store::sql::as_i64;
use anibeam_net::tvmaze::TvmazeEpisode;

/// How far along the sequel chain the walk goes each way.
const MAX_HOPS: usize = 4;

/// How many franchise seasons the walk asks ani.zip about at most.
const MAX_NEIGHBOURS: usize = 6;

/// A format that counts as a season of the chain. A film or an OVA between
/// two seasons is walked through but not counted, and one of its own only
/// ever borrows a logo.
const SEASON_FORMATS: [&str; 3] = ["TV", "TV_SHORT", "ONA"];

/// How far the TVmaze season's first episode may air from the entry's own
/// start date and still be taken for it.
const FIT_SECS: i64 = 45 * 86_400;

/// The airing window, which is also how long an answer about a neighbour
/// that named no TheTVDB season is trusted before it is asked again.
fn window() -> i64 {
    i64::try_from(crate::airing::TTL.as_secs()).unwrap_or(i64::MAX)
}

/// What the series is missing from ani.zip: its regular episodes, its logo.
fn missing(conn: &Connection, anilist_id: u64) -> Result<(bool, bool), CoreError> {
    let id = as_i64(anilist_id);
    let episodes: bool = conn.query_row(
        "SELECT NOT EXISTS (SELECT 1 FROM anizip_episodes WHERE anilist_id = ?1 AND number IS NOT NULL)",
        params![id],
        |r| r.get(0),
    )?;
    let logo: bool = conn.query_row(
        // A logo the image cache turned away as a card is none (#1470).
        "SELECT NOT EXISTS (SELECT 1 FROM anizip_images i WHERE i.anilist_id = ?1 AND i.kind = 'Clearlogo'
                AND NOT EXISTS (SELECT 1 FROM logo_checks v WHERE v.url = i.url AND v.verdict = 0))",
        params![id],
        |r| r.get(0),
    )?;
    Ok((episodes, logo))
}

/// What the entry itself says: whether it is a season, when it started, how
/// many episodes it has, and whether it is finished.
#[derive(Default)]
struct Entry {
    season: bool,
    start: Option<i64>,
    episodes: Option<u32>,
    finished: bool,
}

/// `anilist_media`'s format, start date, episode count and status.
type EntryRow = (Option<String>, Option<String>, Option<i64>, Option<String>);

fn entry(conn: &Connection, anilist_id: u64) -> Result<Entry, CoreError> {
    let row: Option<EntryRow> = conn
        .query_row(
            "SELECT format, start_date, episodes, status FROM anilist_media WHERE id = ?1",
            params![as_i64(anilist_id)],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let Some((format, start, episodes, status)) = row else {
        return Ok(Entry::default());
    };
    Ok(Entry {
        season: format
            .as_deref()
            .is_none_or(|f| SEASON_FORMATS.contains(&f)),
        start: start.as_deref().and_then(day_start),
        episodes: episodes
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n > 0),
        finished: status.as_deref() == Some("FINISHED"),
    })
}

/// Midnight UTC of a `YYYY-MM-DD`.
fn day_start(day: &str) -> Option<i64> {
    let ymd = ::time::macros::format_description!("[year]-[month]-[day]");
    let date = ::time::Date::parse(day.trim(), ymd).ok()?;
    Some(date.midnight().assume_utc().unix_timestamp())
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

/// Whether the graph still owes `id` its own edges and AniList may be asked
/// for them now: never stamped, and not parked by a rate limit.
fn edges_owed(conn: &Connection, id: u64, now: i64) -> Result<bool, CoreError> {
    Ok(conn
        .query_row(
            "SELECT relations_fetched_at IS NULL AND coalesce(crawl_deferred_until, 0) <= ?2
             FROM anilist_media WHERE id = ?1",
            params![as_i64(id), now],
            |r| r.get::<_, bool>(0),
        )
        .optional()?
        .unwrap_or(true))
}

/// Why a walk stopped short of an answer.
enum Stop {
    /// A provider is down or said stop: nothing is written.
    Outage,
    /// The job was cancelled.
    Cancelled(CoreError),
}

/// The franchise seasons around `start`, nearest first and the prequel
/// before the sequel at the same distance, each with its place relative to
/// `start`: -1 for the season before, 2 for the one after the next. A node
/// whose edges the graph does not hold yet is asked of AniList once, with
/// the crawl's own bookkeeping: an answer with nothing in it stamps the node
/// edgeless, and a rate limit parks it.
async fn chain(
    metadata: &Metadata,
    start: u64,
    now: i64,
    ctx: Option<&JobCtx>,
) -> Result<Vec<(u64, i32)>, Stop> {
    let mut seen: HashSet<u64> = HashSet::from([start]);
    let mut out: Vec<(u64, i32, usize)> = Vec::new();
    for step in [-1i32, 1] {
        let mut at = start;
        let mut seasons = 0i32;
        for hop in 0..MAX_HOPS {
            if let Some(ctx) = ctx {
                ctx.checkpoint().map_err(Stop::Cancelled)?;
            }
            let mut next = read(metadata, move |c| next_in(c, at, step));
            if next.is_none() && at != start && read(metadata, move |c| edges_owed(c, at, now)) {
                let stamped = match metadata.net.anilist.enrichment_raw(at).await {
                    Ok((Some(enrichment), _)) => {
                        crawl::write_node(&metadata.services, at, &enrichment, now)
                            .await
                            .is_ok()
                    }
                    Ok((None, _)) => {
                        let _ = crawl::mark(&metadata.services, at, crawl::STAMP, now).await;
                        false
                    }
                    Err(e) if crate::apply::is_rate_limited(&e) => {
                        let until = now + crawl::retry_after(&e);
                        let _ = crawl::mark(&metadata.services, at, crawl::DEFER, until).await;
                        return Err(Stop::Outage);
                    }
                    Err(e) if crate::team::is_outage(&e) => return Err(Stop::Outage),
                    Err(_) => {
                        let _ = crawl::mark(&metadata.services, at, crawl::STAMP, now).await;
                        false
                    }
                };
                if stamped {
                    next = read(metadata, move |c| next_in(c, at, step));
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
fn read<T: Default>(metadata: &Metadata, f: impl FnOnce(&Connection) -> Result<T, CoreError>) -> T {
    metadata.services.store.read(f).unwrap_or_default()
}

/// What ani.zip holds for one franchise season, once asked.
#[derive(Default)]
struct Neighbour {
    tvdb_id: Option<u64>,
    /// The first and last TheTVDB season its regular episodes sit in.
    seasons: Option<(u32, u32)>,
    /// The first TheTVDB episode number it holds in its first season, and
    /// the last it holds in its last: where a split season's cours meet.
    first_episode: Option<u32>,
    last_episode: Option<u32>,
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
    let seasons = match seasons {
        (Some(first), Some(last)) => Some((first as u32, last as u32)),
        _ => None,
    };
    let edge = |sql: &str, season: u32| -> Result<Option<u32>, CoreError> {
        Ok(conn
            .query_row(sql, params![id, season], |r| r.get::<_, Option<i64>>(0))?
            .and_then(|v| u32::try_from(v).ok()))
    };
    let (first_episode, last_episode) = match seasons {
        Some((first, last)) => (
            edge(
                "SELECT min(episode) FROM anizip_episodes WHERE anilist_id = ?1 AND number IS NOT NULL AND season = ?2",
                first,
            )?,
            edge(
                "SELECT max(episode) FROM anizip_episodes WHERE anilist_id = ?1 AND number IS NOT NULL AND season = ?2",
                last,
            )?,
        ),
        None => (None, None),
    };
    let logo: Option<String> = conn
        .query_row(
            "SELECT url FROM anizip_images i WHERE anilist_id = ?1 AND kind = 'Clearlogo'
               AND NOT EXISTS (SELECT 1 FROM logo_checks v WHERE v.url = i.url AND v.verdict = 0)
             ORDER BY CASE WHEN lower(lang) LIKE 'en%' THEN 0 WHEN lang IS NULL THEN 1 ELSE 2 END, rank
             LIMIT 1",
            params![id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(Neighbour {
        tvdb_id: tvdb_id.filter(|v| *v > 0).map(|v| v as u64),
        seasons,
        first_episode,
        last_episode,
        logo,
    })
}

/// How the walk stands with one neighbour's ani.zip answer.
enum Known {
    /// Good to read: a whole answer, or one inside the airing window.
    Fresh,
    /// Never asked, or an answer that named no TheTVDB season and has aged
    /// past the airing window, so it is asked again.
    Stale,
    /// A library series ani.zip has not been asked about. The ani.zip walk
    /// owns its first ask, which also marks its MAL id owed and fetches its
    /// images, so the fallback leaves it alone.
    Library,
}

fn known(conn: &Connection, id: u64, now: i64) -> Result<Known, CoreError> {
    let id = as_i64(id);
    let row: Option<(bool, Option<i64>, i64)> = conn
        .query_row(
            "SELECT found, tvdb_id, fetched_at FROM anizip_shows WHERE anilist_id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let in_library: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM series WHERE anilist_id = ?1 AND provider = 'anilist')",
        params![id],
        |r| r.get(0),
    )?;
    Ok(match row {
        None if in_library => Known::Library,
        None => Known::Stale,
        Some((found, tvdb, fetched_at)) => {
            let whole = found
                && tvdb.is_some()
                && conn.query_row(
                    "SELECT EXISTS (SELECT 1 FROM anizip_episodes WHERE anilist_id = ?1 AND number IS NOT NULL)",
                    params![id],
                    |r| r.get::<_, bool>(0),
                )?;
            if whole || in_library || now.saturating_sub(fetched_at) < window() {
                Known::Fresh
            } else {
                Known::Stale
            }
        }
    })
}

/// What asking about one franchise season came to.
enum Asked {
    /// Its rows are in the tables.
    Known,
    /// Nothing to read for it now: the next season is tried.
    Skip,
    /// ani.zip is down: the walk writes nothing.
    Down,
}

/// Makes sure `id`'s ani.zip rows are fresh enough to read, asking ani.zip
/// when they are not.
async fn ask(metadata: &Metadata, id: u64, now: i64) -> Result<Asked, CoreError> {
    match read(metadata, move |c| known(c, id, now).map(Some)) {
        Some(Known::Fresh) => return Ok(Asked::Known),
        Some(Known::Library) | None => return Ok(Asked::Skip),
        Some(Known::Stale) => {}
    }
    let answer = metadata.anizip_show(id).await;
    if answer == AniZipRead::Failed {
        return Ok(if metadata.anizip_down() {
            Asked::Down
        } else {
            Asked::Skip
        });
    }
    metadata
        .services
        .store
        .tx_async(move |tx| anizip::write(tx, id, &answer, now))
        .await?;
    Ok(Asked::Known)
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

/// This entry's episodes out of the TVmaze show, numbered from one, or
/// nothing when the walk cannot be sure which they are.
///
/// The season the neighbour points at is the guess. A neighbour right before
/// this entry that stops partway through its TheTVDB season is the first cour
/// of a split season, so this entry is the rest of that season; a neighbour
/// right after that starts partway through its season is the second cour, so
/// this entry is the start of it. Whatever the guess, it is only taken when
/// it fits the entry: its first episode airs within `FIT_SECS` of the
/// entry's start date, and it holds about as many episodes as AniList says
/// the entry has. TheTVDB's and TVmaze's seasons do not always line up, and
/// a guess that does not fit writes nothing.
fn pick(all: &[TvmazeEpisode], n: &Neighbour, place: i32, entry: &Entry) -> Vec<TvmazeEpisode> {
    let Some(seasons) = n.seasons else {
        return Vec::new();
    };
    let of = |season: u32| -> Vec<TvmazeEpisode> {
        all.iter().filter(|e| e.season == season).cloned().collect()
    };
    let renumber = |eps: Vec<TvmazeEpisode>, from: u32| -> Vec<TvmazeEpisode> {
        eps.into_iter()
            .filter(|e| e.number >= from)
            .map(|mut e| {
                e.number = e.number + 1 - from;
                e
            })
            .collect()
    };
    let guess = match (place, n.last_episode, n.first_episode) {
        (-1, Some(last), _) if of(seasons.1).iter().any(|e| e.number > last) => {
            renumber(of(seasons.1), last + 1)
        }
        (1, _, Some(first)) if first > 1 => of(seasons.0)
            .into_iter()
            .filter(|e| e.number < first)
            .collect(),
        _ => match season_for(seasons, place) {
            Some(season) => of(season),
            None => Vec::new(),
        },
    };
    if fits(&guess, entry) {
        guess
    } else {
        Vec::new()
    }
}

/// Whether a guessed season is this entry's: it starts near the entry's
/// start date and is about as long as AniList says the entry is. An entry
/// with no start date cannot be checked, so nothing fits it.
fn fits(guess: &[TvmazeEpisode], entry: &Entry) -> bool {
    let (Some(start), Some(first)) = (entry.start, guess.first().and_then(|e| e.airstamp)) else {
        return false;
    };
    if (first - start).abs() > FIT_SECS {
        return false;
    }
    match entry.episodes {
        // A running season may list fewer episodes than it will have, never
        // many more.
        Some(n) => {
            let have = guess.len() as u32;
            have <= n + 1 && (!entry.finished || have + 1 >= n)
        }
        None => true,
    }
}

/// The TVmaze show and season the walk found, the season it came through.
struct Found {
    via: u64,
    tvmaze_id: u64,
    season: u32,
    episodes: Vec<TvmazeEpisode>,
}

/// What the fallback has to look for: episodes, where ani.zip leaves the
/// series without regular ones and it is a season, and a logo, where it has
/// none of its own and has borrowed none yet.
fn wants(conn: &Connection, anilist_id: u64) -> Result<(bool, bool), CoreError> {
    let (want_episodes, want_logo) = missing(conn, anilist_id)?;
    let borrowed: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM fallback_shows b WHERE b.anilist_id = ?1 AND b.logo_url IS NOT NULL
                AND NOT EXISTS (SELECT 1 FROM logo_checks v WHERE v.url = b.logo_url AND v.verdict = 0))",
        params![as_i64(anilist_id)],
        |r| r.get(0),
    )?;
    Ok((
        want_episodes && entry(conn, anilist_id)?.season,
        want_logo && !borrowed,
    ))
}

/// Whether the fallback has anything to look for (`wants`).
pub fn wanted(conn: &Connection, anilist_id: u64) -> Result<bool, CoreError> {
    let (episodes, logo) = wants(conn, anilist_id)?;
    Ok(episodes || logo)
}

/// How long a walk that looked only for a logo and found none is left
/// before the next: a franchise with no logo anywhere rarely gains one, and
/// a walk asks ani.zip and AniList again.
const LOGO_WINDOW: i64 = 30 * 86_400;

/// Runs the fallback for `anilist_id` when ani.zip leaves it without regular
/// episodes or without a logo. Returns whether any row changed. An outage on
/// any request writes nothing; a walk that found nothing records that it
/// looked and keeps whatever an earlier walk wrote. `ctx` is the job running
/// it, for cancelling between requests.
pub async fn run(
    metadata: &Metadata,
    anilist_id: u64,
    now: i64,
    ctx: Option<&JobCtx>,
) -> Result<bool, CoreError> {
    let (want_episodes, want_logo) = metadata
        .services
        .store
        .read(move |c| missing(c, anilist_id))?;
    let entry = read(metadata, move |c| entry(c, anilist_id).map(Some)).unwrap_or_default();
    // A film, an OVA or a special has no TV season of its own to take
    // episodes from: it only ever borrows a logo.
    let want_episodes = want_episodes && entry.season;
    if !want_episodes && !want_logo {
        return Ok(false);
    }
    let neighbours = match chain(metadata, anilist_id, now, ctx).await {
        Ok(neighbours) => neighbours,
        Err(Stop::Outage) => return Ok(false),
        Err(Stop::Cancelled(e)) => return Err(e),
    };
    // The logo walks back the sequel chain first, the season before this
    // one and on back, then forward (#1463).
    let mut logo: Option<String> = None;
    if want_logo {
        let back = neighbours.iter().filter(|(_, place)| *place < 0);
        let forward = neighbours.iter().filter(|(_, place)| *place > 0);
        for &(id, _) in back.chain(forward) {
            if let Some(ctx) = ctx {
                ctx.checkpoint()?;
            }
            match ask(metadata, id, now).await? {
                Asked::Down => return Ok(false),
                Asked::Skip => continue,
                Asked::Known => {}
            }
            logo = read(metadata, move |c| neighbour(c, id)).logo;
            if logo.is_some() {
                break;
            }
        }
    }
    // The episodes take the nearest season that names a TheTVDB season.
    let mut target: Option<(u64, u64, i32, Neighbour)> = None;
    if want_episodes {
        for &(id, place) in &neighbours {
            if let Some(ctx) = ctx {
                ctx.checkpoint()?;
            }
            match ask(metadata, id, now).await? {
                Asked::Down => return Ok(false),
                Asked::Skip => continue,
                Asked::Known => {}
            }
            let n = read(metadata, move |c| neighbour(c, id));
            if let Some(tvdb) = n.tvdb_id
                && n.seasons.is_some()
            {
                target = Some((id, tvdb, place, n));
                break;
            }
        }
    }
    let mut found: Option<Found> = None;
    if let Some((via, tvdb, place, n)) = target {
        let show = match metadata.net.tvmaze.lookup_thetvdb(tvdb).await {
            Ok(Some(show_id)) => metadata.net.tvmaze.show(show_id).await,
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        };
        match show {
            Ok(Some(show)) => {
                let episodes = pick(&show.episodes(), &n, place, &entry);
                if let Some(season) = episodes.first().map(|e| e.season) {
                    found = Some(Found {
                        via,
                        tvmaze_id: show.id,
                        season,
                        episodes,
                    });
                } else {
                    tracing::debug!(
                        "the TVmaze fallback for AniList {anilist_id} found no season that fits"
                    );
                }
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

/// Whether the fallback should walk again: ani.zip still leaves the show
/// without what the fallback looks for (`wants`), and the last walk is older
/// than the airing window for episodes, or than `LOGO_WINDOW` when only a
/// logo is wanted, or there was none.
pub fn due(conn: &Connection, anilist_id: u64, now: i64) -> Result<bool, CoreError> {
    let (episodes, logo) = wants(conn, anilist_id)?;
    let wait = if episodes {
        window()
    } else if logo {
        LOGO_WINDOW
    } else {
        return Ok(false);
    };
    let fetched_at: Option<i64> = conn
        .query_row(
            "SELECT fetched_at FROM fallback_shows WHERE anilist_id = ?1",
            params![as_i64(anilist_id)],
            |r| r.get(0),
        )
        .optional()?;
    Ok(fetched_at.is_none_or(|at| now.saturating_sub(at) >= wait))
}

/// Runs the fallback for one show as a job of its own, after a match has
/// been reported (#1461): the match never waits on the walk. Returns the
/// job's id, or `None` when the show needs nothing from the fallback.
pub fn follow_up(metadata: &Metadata, anilist_id: u64) -> Option<u64> {
    let needed = metadata
        .services
        .store
        .read(move |c| wanted(c, anilist_id))
        .unwrap_or(false);
    let this = metadata.this()?;
    needed.then(|| {
        start_for(
            &this,
            vec![(anilist_id, series_of(&this, anilist_id))],
            false,
        )
    })
}

/// The series matched to `anilist_id`, whose cards are drawn again after a
/// walk changed something.
fn series_of(metadata: &Metadata, anilist_id: u64) -> Vec<u64> {
    read(metadata, move |c| {
        let mut stmt = c.prepare(
            "SELECT id FROM series WHERE anilist_id = ?1 AND provider = 'anilist' ORDER BY id",
        )?;
        let rows = stmt.query_map(params![as_i64(anilist_id)], |r| r.get::<_, i64>(0))?;
        Ok(rows
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(|v| v as u64)
            .collect())
    })
}

/// Starts the launch's pass over every AniList match the fallback is due
/// for, the ones matched before it existed included. Shows ani.zip was never
/// asked about are the ani.zip walk's, which runs the fallback itself.
/// Returns the job's id, or `None` when nothing is due.
pub fn start(metadata: &Arc<Metadata>) -> Option<u64> {
    let now = anibeam_foundation::time::now_secs();
    let work = metadata
        .services
        .store
        .read(move |c| due_ids(c, now))
        .ok()?;
    (!work.is_empty()).then(|| start_for(metadata, work, true))
}

/// The job behind `follow_up` and `start`. `only_due` asks `due` again right
/// before each show's walk, for the launch pass; a match's own follow-up
/// walks whatever the clock says, since the match itself is new information.
fn start_for(metadata: &Arc<Metadata>, work: Vec<(u64, Vec<u64>)>, only_due: bool) -> u64 {
    let owner = metadata.clone();
    metadata
        .services
        .jobs
        .clone()
        .start(JobKind::RefreshAiring, move |ctx| async move {
            let total = work.len() as u64;
            let mut filled = 0u64;
            for (done, (anilist_id, series)) in work.into_iter().enumerate() {
                ctx.checkpoint()?;
                ctx.progress(done as u64, Some(total), &anilist_id.to_string());
                let now = anibeam_foundation::time::now_secs();
                // Asked again right before the walk: the airing sweep starts
                // beside this pass and may have walked the season already,
                // or a match's own walk may have.
                if only_due && !read(&owner, move |c| due(c, anilist_id, now)) {
                    continue;
                }
                // One show's failure is that show's: the rest still get
                // their turn. Only a cancel ends the pass.
                match run(&owner, anilist_id, now, Some(&ctx)).await {
                    Ok(true) => {}
                    Ok(false) => continue,
                    Err(e) => {
                        ctx.checkpoint()?;
                        tracing::debug!("the fallback for AniList {anilist_id} failed: {e}");
                        continue;
                    }
                }
                filled += 1;
                let urls = owner
                    .services
                    .store
                    .write_async(move |c| anizip::image_urls(c, anilist_id))
                    .await
                    .unwrap_or_default();
                owner.net.images.ensure(&urls).await;
                for id in series {
                    match crate::apply::card_for(&owner.services, id).await {
                        Ok(Some(card)) => ctx.changed(card),
                        Ok(None) => {}
                        Err(e) => tracing::debug!("no card for series {id}: {e}"),
                    }
                }
            }
            Ok(Finished {
                plain: None,
                level: if filled == 0 {
                    Level::Debug
                } else {
                    Level::Info
                },
                message: format!("Franchise episode details or logos filled for {filled} series"),
                body: EventBody::Notice,
            })
        })
}

/// The AniList ids `due` says yes to, each with the series matched to it.
fn due_ids(conn: &Connection, now: i64) -> Result<Vec<(u64, Vec<u64>)>, CoreError> {
    let mut stmt = conn.prepare(
        "SELECT s.anilist_id, s.id FROM series s
         WHERE s.provider = 'anilist' AND s.anilist_id IS NOT NULL
           AND EXISTS (SELECT 1 FROM anizip_shows z WHERE z.anilist_id = s.anilist_id)
         ORDER BY s.anilist_id, s.id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64))
    })?;
    let mut out: Vec<(u64, Vec<u64>)> = Vec::new();
    for row in rows {
        let (anilist_id, series) = row?;
        match out.last_mut() {
            Some((id, list)) if *id == anilist_id => list.push(series),
            _ => out.push((anilist_id, vec![series])),
        }
    }
    let mut due_now = Vec::new();
    for (anilist_id, series) in out {
        if due(conn, anilist_id, now)? {
            due_now.push((anilist_id, series));
        }
    }
    Ok(due_now)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(season: u32, number: u32, airstamp: i64) -> TvmazeEpisode {
        TvmazeEpisode {
            season,
            number,
            name: Some(format!("S{season}E{number}")),
            airstamp: Some(airstamp),
            runtime: None,
            summary: None,
            image: None,
        }
    }

    const DAY: i64 = 86_400;

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

    /// Part 1 is episodes 1 to 2 of TheTVDB season 1, and season 1 goes on:
    /// Part 2 is the rest of season 1, numbered from one, not season 2.
    #[test]
    fn a_second_cour_takes_the_rest_of_its_season() {
        let start = 1000 * DAY;
        let all = [
            ep(1, 1, start - 200 * DAY),
            ep(1, 2, start - 193 * DAY),
            ep(1, 3, start),
            ep(1, 4, start + 7 * DAY),
            ep(2, 1, start + 300 * DAY),
        ];
        let part_one = Neighbour {
            seasons: Some((1, 1)),
            first_episode: Some(1),
            last_episode: Some(2),
            ..Neighbour::default()
        };
        let entry = Entry {
            season: true,
            start: Some(start),
            episodes: Some(2),
            finished: true,
        };
        let got = pick(&all, &part_one, -1, &entry);
        let names: Vec<(u32, Option<&str>)> =
            got.iter().map(|e| (e.number, e.name.as_deref())).collect();
        assert_eq!(names, [(1, Some("S1E3")), (2, Some("S1E4"))]);
    }

    /// TVmaze splits what TheTVDB keeps as one season, so the guessed
    /// season starts long before the entry: nothing is written.
    #[test]
    fn a_season_that_does_not_fit_the_entry_is_not_taken() {
        let start = 1000 * DAY;
        let all = [ep(3, 1, start - 120 * DAY), ep(3, 2, start - 113 * DAY)];
        let n = Neighbour {
            seasons: Some((2, 2)),
            first_episode: Some(1),
            last_episode: Some(12),
            ..Neighbour::default()
        };
        let entry = Entry {
            season: true,
            start: Some(start),
            episodes: Some(2),
            finished: true,
        };
        assert!(pick(&all, &n, -1, &entry).is_empty());
        // Nor when the entry has no start date to check against.
        let no_start = Entry {
            start: None,
            ..entry
        };
        let fits_now = [ep(3, 1, start), ep(3, 2, start + 7 * DAY)];
        assert!(pick(&fits_now, &n, -1, &no_start).is_empty());
        // Nor when it is far longer than AniList says the entry is.
        let long: Vec<TvmazeEpisode> = (1..=12)
            .map(|i| ep(3, i, start + i64::from(i) * 7 * DAY))
            .collect();
        let short = Entry {
            season: true,
            start: Some(start),
            episodes: Some(2),
            finished: true,
        };
        assert!(pick(&long, &n, -1, &short).is_empty());
    }
}
