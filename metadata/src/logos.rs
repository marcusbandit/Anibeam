//! Logo languages (#1470): which language each of a show's clear logos is
//! in, off TheTVDB's own pages (`anibeam_net::tvdb`), so the series page can
//! draw the logo in the language the person reads titles in.
//!
//! The logos are TheTVDB's, kept per TheTVDB series in `tvdb_logos`, and
//! `tvdb_logo_asks` says when each AniList entry last asked and with which
//! of ani.zip's clear logos. One entry asks once. It asks again only when
//! ani.zip's logos for it changed, which is TheTVDB having a new one, or
//! after `KEEP`. Two seasons of one TheTVDB series share one answer.
//!
//! It asks when a match fetches the show's artwork, when the airing refresh
//! asks ani.zip again, and once at launch for the shows matched before it
//! existed, never on a timer. A failure writes nothing, so the show keeps
//! the logo ani.zip's order picks.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::Metadata;
use crate::anizip;
use anibeam_contract::*;
use anibeam_foundation::jobs::{Finished, JobCtx};
use anibeam_foundation::store::sql::as_i64;
use anibeam_foundation::sync::recover;
use anibeam_net::tvdb::TvdbLogo;

/// How long one answer is trusted when ani.zip's logos stay the same.
pub const KEEP: i64 = 180 * 86_400;

/// ani.zip's clear logos for `anilist_id`, sorted and joined: what an answer
/// is kept against, so a set that changed asks again.
fn logo_set(conn: &Connection, anilist_id: u64) -> Result<String, CoreError> {
    let mut stmt = conn.prepare(
        "SELECT url FROM anizip_images WHERE anilist_id = ?1 AND kind = 'Clearlogo' ORDER BY url",
    )?;
    let rows = stmt.query_map(params![as_i64(anilist_id)], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?.join(";"))
}

/// What one ask is for: the TheTVDB series and ani.zip's logos at the time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ask {
    pub tvdb_id: u64,
    pub logo_set: String,
}

/// The ask `anilist_id` is owed, if any: ani.zip mapped it to a TheTVDB
/// series and gave it a clear logo, and it never asked, or asked about
/// another series or another set of logos, or asked longer than `KEEP` ago.
pub fn owed(conn: &Connection, anilist_id: u64, now: i64) -> Result<Option<Ask>, CoreError> {
    let id = as_i64(anilist_id);
    let tvdb_id: Option<i64> = conn
        .query_row(
            "SELECT tvdb_id FROM anizip_shows WHERE anilist_id = ?1 AND found = 1",
            params![id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let Some(tvdb_id) = tvdb_id.filter(|v| *v > 0) else {
        return Ok(None);
    };
    let set = logo_set(conn, anilist_id)?;
    if set.is_empty() {
        return Ok(None);
    }
    let asked: Option<(i64, String, i64)> = conn
        .query_row(
            "SELECT tvdb_id, logo_set, fetched_at FROM tvdb_logo_asks WHERE anilist_id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let fresh =
        asked.is_some_and(|(t, s, at)| t == tvdb_id && s == set && now.saturating_sub(at) < KEEP);
    Ok((!fresh).then_some(Ask {
        tvdb_id: tvdb_id as u64,
        logo_set: set,
    }))
}

/// When another entry asked about the same series with the same logos,
/// recently enough to trust: its answer is this one's too.
fn shared(
    conn: &Connection,
    anilist_id: u64,
    ask: &Ask,
    now: i64,
) -> Result<Option<i64>, CoreError> {
    Ok(conn
        .query_row(
            "SELECT max(fetched_at) FROM tvdb_logo_asks
             WHERE anilist_id != ?1 AND tvdb_id = ?2 AND logo_set = ?3 AND fetched_at > ?4",
            params![
                as_i64(anilist_id),
                as_i64(ask.tvdb_id),
                ask.logo_set,
                now - KEEP
            ],
            |r| r.get::<_, Option<i64>>(0),
        )
        .optional()?
        .flatten())
}

fn stamp(tx: &Transaction, anilist_id: u64, ask: &Ask, at: i64) -> Result<(), CoreError> {
    tx.execute(
        "INSERT INTO tvdb_logo_asks (anilist_id, tvdb_id, logo_set, fetched_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(anilist_id) DO UPDATE SET
            tvdb_id = excluded.tvdb_id, logo_set = excluded.logo_set, fetched_at = excluded.fetched_at",
        params![as_i64(anilist_id), as_i64(ask.tvdb_id), ask.logo_set, at],
    )?;
    Ok(())
}

/// Writes TheTVDB's answer for `ask`: its series' logos, replaced whole, and
/// `anilist_id`'s ask stamped.
pub fn write(
    tx: &Transaction,
    anilist_id: u64,
    ask: &Ask,
    logos: &[TvdbLogo],
    now: i64,
) -> Result<(), CoreError> {
    let tvdb = as_i64(ask.tvdb_id);
    tx.execute("DELETE FROM tvdb_logos WHERE tvdb_id = ?1", params![tvdb])?;
    let mut insert = tx.prepare(
        "INSERT OR IGNORE INTO tvdb_logos (tvdb_id, url, lang, rank) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (rank, logo) in logos.iter().enumerate() {
        insert.execute(params![tvdb, logo.url, logo.lang, rank as i64])?;
    }
    stamp(tx, anilist_id, ask, now)
}

impl Metadata {
    /// Whether TheTVDB failed inside the outage window, when it is not asked.
    fn tvdb_down(&self) -> bool {
        recover(&self.tvdb_outage).is_some_and(|at| at.elapsed() < crate::OUTAGE_WINDOW)
    }
}

/// Asks TheTVDB about `anilist_id`'s logos if it is owed an ask. True when
/// it wrote something the page reads differently now.
pub async fn run(metadata: &Metadata, anilist_id: u64) -> Result<bool, CoreError> {
    let now = anibeam_foundation::time::now_secs();
    let store = &metadata.services.store;
    let Some(ask) = store.read(move |c| owed(c, anilist_id, now))? else {
        return Ok(false);
    };
    let reuse = {
        let ask = ask.clone();
        store.read(move |c| shared(c, anilist_id, &ask, now))?
    };
    if let Some(at) = reuse {
        store
            .tx_async(move |tx| stamp(tx, anilist_id, &ask, at))
            .await?;
        return Ok(true);
    }
    if metadata.tvdb_down() {
        return Ok(false);
    }
    let logos = match metadata.net.tvdb.logos(ask.tvdb_id).await {
        Ok(logos) => logos,
        Err(e) => {
            tracing::debug!("TheTVDB's logos for AniList {anilist_id} did not read: {e}");
            if turned_away(&e) {
                // TheTVDB is down, or turns this client away (a bot check):
                // nothing is asked of it for a while.
                *recover(&metadata.tvdb_outage) = Some(Instant::now());
            } else {
                // A refusal for this series alone (a 410, a 400) is asked
                // again only after `KEEP`, and the series' logos stay as
                // they are.
                store
                    .tx_async(move |tx| stamp(tx, anilist_id, &ask, now))
                    .await?;
            }
            return Ok(false);
        }
    };
    if logos.is_empty() {
        // ani.zip lists logos for the show, so a page that lists none did
        // not read (TheTVDB changed its pages): the ask waits for `KEEP`,
        // and the logos other seasons read before stay.
        store
            .tx_async(move |tx| stamp(tx, anilist_id, &ask, now))
            .await?;
        return Ok(false);
    }
    store
        .tx_async(move |tx| write(tx, anilist_id, &ask, &logos, now))
        .await?;
    Ok(true)
}

/// Whether a failure says TheTVDB is down or turns this client away as a
/// whole, rather than that one page was bad.
fn turned_away(e: &CoreError) -> bool {
    crate::team::is_outage(e)
        || matches!(
            e,
            CoreError::Provider {
                status: Some(401 | 403 | 451),
                ..
            }
        )
}

/// Fetches the images `anilist_id` wants now. A logo that turns out to be
/// none on arrival gives way to the next, so this asks again while new
/// urls come up, a few times at most.
async fn ensure_logos(metadata: &Metadata, anilist_id: u64) {
    let mut asked: HashSet<String> = HashSet::new();
    for _ in 0..4 {
        let urls: Vec<String> = metadata
            .services
            .store
            .write_async(move |c| anizip::image_urls(c, anilist_id))
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|u| !asked.contains(u))
            .collect();
        if urls.is_empty() {
            return;
        }
        asked.extend(urls.iter().cloned());
        metadata.net.images.ensure(&urls).await;
    }
}

/// One show a job asks about: its AniList id, the series matched to it,
/// whose cards are drawn again, and the name its progress line shows.
struct Work {
    anilist_id: u64,
    series: Vec<u64>,
    name: String,
}

/// The AniList ids a series is matched to that are owed an ask.
fn owed_ids(conn: &Connection, now: i64) -> Result<Vec<Work>, CoreError> {
    let mut stmt = conn.prepare(
        "SELECT s.anilist_id, s.id, s.folder_name FROM series s
         WHERE s.provider = 'anilist' AND s.anilist_id IS NOT NULL
           AND EXISTS (SELECT 1 FROM anizip_images z WHERE z.anilist_id = s.anilist_id AND z.kind = 'Clearlogo')
         ORDER BY s.anilist_id, s.id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)? as u64,
            r.get::<_, i64>(1)? as u64,
            r.get::<_, String>(2)?,
        ))
    })?;
    let mut out: Vec<Work> = Vec::new();
    for row in rows {
        let (anilist_id, series, name) = row?;
        match out.last_mut() {
            Some(w) if w.anilist_id == anilist_id => w.series.push(series),
            _ => out.push(Work {
                anilist_id,
                series: vec![series],
                name,
            }),
        }
    }
    let mut owed_out = Vec::with_capacity(out.len());
    for w in out {
        if owed(conn, w.anilist_id, now)?.is_some() {
            owed_out.push(w);
        }
    }
    Ok(owed_out)
}

/// The series matched to `anilist_id`, and the first one's name.
fn work_of(conn: &Connection, anilist_id: u64) -> Result<Work, CoreError> {
    let mut stmt = conn.prepare(
        "SELECT id, folder_name FROM series WHERE anilist_id = ?1 AND provider = 'anilist' ORDER BY id",
    )?;
    let rows = stmt.query_map(params![as_i64(anilist_id)], |r| {
        Ok((r.get::<_, i64>(0)? as u64, r.get::<_, String>(1)?))
    })?;
    let mut work = Work {
        anilist_id,
        series: Vec::new(),
        name: anilist_id.to_string(),
    };
    for (at, row) in rows.enumerate() {
        let (id, name) = row?;
        if at == 0 {
            work.name = name;
        }
        work.series.push(id);
    }
    Ok(work)
}

/// After a match or a fresh ani.zip answer: asks about `anilist_id`'s logos
/// in a job of its own when it is owed an ask, so nothing waits on TheTVDB.
pub fn follow_up(metadata: &Metadata, anilist_id: u64) -> Option<u64> {
    let now = anibeam_foundation::time::now_secs();
    let owed_now = metadata
        .services
        .store
        .read(move |c| owed(c, anilist_id, now))
        .ok()
        .flatten()
        .is_some();
    if !owed_now {
        return None;
    }
    let this = metadata.this()?;
    let work = metadata
        .services
        .store
        .read(move |c| work_of(c, anilist_id))
        .ok()?;
    Some(start_for(&this, vec![work], false))
}

/// The launch's walk over every matched show owed an ask, unless one is
/// running or none is owed. It reads the list again before it ends, so the
/// shows ani.zip's own walk answers meanwhile get their turn too.
pub fn start(metadata: &Arc<Metadata>) -> Option<u64> {
    let now = anibeam_foundation::time::now_secs();
    let work = metadata
        .services
        .store
        .read(move |c| owed_ids(c, now))
        .ok()?;
    if work.is_empty() || metadata.tvdb_walking.swap(true, Ordering::SeqCst) {
        return None;
    }
    Some(start_for(metadata, work, true))
}

/// Holds `tvdb_walking` up for as long as the walk's job body lives.
struct Walking(Arc<Metadata>);

impl Drop for Walking {
    fn drop(&mut self) {
        self.0.tvdb_walking.store(false, Ordering::SeqCst);
    }
}

fn start_for(metadata: &Arc<Metadata>, work: Vec<Work>, walk: bool) -> u64 {
    let owner = metadata.clone();
    let walking = walk.then(|| Walking(metadata.clone()));
    metadata
        .services
        .jobs
        .clone()
        .start(JobKind::RefreshAiring, move |ctx| async move {
            let _walking = walking;
            let named = ask_all(&owner, &ctx, work, walk).await?;
            Ok(Finished {
                plain: None,
                // One show after its match is no news for the log; the
                // launch walk over the library is.
                level: if named == 0 || !walk {
                    Level::Debug
                } else {
                    Level::Info
                },
                message: format!("TheTVDB named the logo languages for {named} series"),
                body: EventBody::Notice,
            })
        })
}

/// Asks about each show in turn, then, for the launch walk, about any show
/// that came to be owed meanwhile. Stops early when TheTVDB is down.
async fn ask_all(
    metadata: &Metadata,
    ctx: &Arc<JobCtx>,
    mut work: Vec<Work>,
    walk: bool,
) -> Result<u64, CoreError> {
    let mut named = 0u64;
    let mut tried: HashSet<u64> = HashSet::new();
    loop {
        let total = work.len() as u64;
        for (
            done,
            Work {
                anilist_id,
                series,
                name,
            },
        ) in work.into_iter().enumerate()
        {
            ctx.checkpoint()?;
            ctx.progress(done as u64, Some(total), &name);
            tried.insert(anilist_id);
            if metadata.tvdb_down() {
                return Ok(named);
            }
            match run(metadata, anilist_id).await {
                Ok(true) => {}
                Ok(false) => continue,
                Err(e) => {
                    tracing::debug!("the logo languages for AniList {anilist_id} failed: {e}");
                    continue;
                }
            }
            named += 1;
            ensure_logos(metadata, anilist_id).await;
            for id in series {
                match crate::apply::card_for(&metadata.services, id).await {
                    Ok(Some(card)) => ctx.changed(card),
                    Ok(None) => {}
                    Err(e) => tracing::debug!("no card for series {id}: {e}"),
                }
            }
        }
        if !walk {
            return Ok(named);
        }
        let now = anibeam_foundation::time::now_secs();
        work = metadata
            .services
            .store
            .read(move |c| owed_ids(c, now))
            .unwrap_or_default()
            .into_iter()
            .filter(|w| !tried.contains(&w.anilist_id))
            .collect();
        if work.is_empty() {
            return Ok(named);
        }
    }
}
