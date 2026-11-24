//! The image cache: one file per url under `<cache_dir>/images`, one row per
//! file in `images`, and the two jobs that fill and empty it.
//!
//! Nothing here goes through a provider limiter. The posters, banners and
//! portraits come from AniList's CDN and TVmaze's static host rather than
//! either API, so the only bound on a fetch is a semaphore of four permits;
//! an error from one names AniList, whose CDN answers nearly all of them.
//!
//! A read never fetches. It reports the gap it found and the cache decides,
//! at most once every five minutes, whether that is worth a fill job.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params, params_from_iter};
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;

use crate::Net;
use crate::logo_image::{self, LogoFile};
use crate::{Http, HttpRequest};
use anibeam_contract::*;
use anibeam_foundation::atomic_file;
use anibeam_foundation::jobs::Finished;
use anibeam_foundation::store::Store;
use anibeam_foundation::store::sql::placeholders;
use anibeam_foundation::sync::recover;
use anibeam_foundation::time;

/// How the cache reads the cards of the series whose images changed, for the
/// `SeriesChanged` that tells the shells. It is the library's
/// `cards::cards_for`, which `Core::open` hands the net team, so the cache
/// names no team above it (#854).
pub type CardsFor = fn(&Connection, &Path, &[u64]) -> Result<Vec<SeriesCard>, CoreError>;

/// Fetches in flight at once.
const CONCURRENCY: usize = 4;

/// Urls per round of a fill, so a big library reports progress and notices
/// cancellation between batches rather than only at the end.
const BATCH: usize = 20;

/// How long an image no row references survives after its last use.
const ORPHAN_AGE: i64 = 30 * 86_400;

/// The row cap. Above it the least recently used non-pinned rows go.
const MAX_ROWS: u64 = 5_000;

/// `used_at` is bumped at most once a day per image, so a read that hands
/// out a hundred posters queues one narrow update rather than a hundred.
const USED_GAP: i64 = 86_400;

/// A read that finds a gap starts a fill at most this often. Without it a
/// core with no network would start one on every read it ever answers.
const FILL_INTERVAL: Duration = Duration::from_secs(300);

/// The extensions a url is allowed to name. Anything else is judged by the
/// content type instead.
const IMAGE_EXTENSIONS: [&str; 6] = ["jpg", "jpeg", "png", "gif", "webp", "avif"];

/// Every url whose image the library would draw if it had it. A TVmaze
/// show is drawn only by the series matched to it, so a show every series
/// has moved off references nothing. Airing now's cards and the shows
/// marked not interested draw theirs too.
const REFERENCED_SQL: &str =
    "SELECT cover_url AS url FROM anilist_media WHERE cover_url IS NOT NULL
     UNION SELECT banner_url FROM anilist_media WHERE banner_url IS NOT NULL
     UNION SELECT value ->> 'image_url' FROM anilist_media, json_each(anilist_media.characters)
           WHERE value ->> 'image_url' IS NOT NULL
     UNION SELECT t.poster_url FROM tvmaze_shows t JOIN series s ON s.tvmaze_id = t.id WHERE t.poster_url IS NOT NULL
     UNION SELECT t.background_url FROM tvmaze_shows t JOIN series s ON s.tvmaze_id = t.id WHERE t.background_url IS NOT NULL
     UNION SELECT cover_url FROM season_shows WHERE cover_url IS NOT NULL
     UNION SELECT cover_url FROM not_interested WHERE cover_url IS NOT NULL
     UNION SELECT a.url FROM anizip_images a JOIN series s ON s.anilist_id = a.anilist_id
     UNION SELECT z.still_url FROM anizip_episodes z JOIN series s ON s.anilist_id = z.anilist_id
           WHERE z.still_url IS NOT NULL
     UNION SELECT b.logo_url FROM fallback_shows b JOIN series s ON s.anilist_id = b.anilist_id
           WHERE b.logo_url IS NOT NULL
     UNION SELECT url FROM tvdb_logo_wanted
     UNION SELECT e.still_url FROM fallback_episodes e JOIN series s ON s.anilist_id = e.anilist_id
           WHERE e.still_url IS NOT NULL
             AND NOT EXISTS (SELECT 1 FROM anizip_episodes ze2 WHERE ze2.anilist_id = e.anilist_id AND ze2.number IS NOT NULL)";

/// The AniList ids of the series a backdrop is drawn for before anyone
/// opens their page (#1475): the ones Home's Continue may hold, played here
/// or being watched on a tracker with episodes seen. Their first background
/// is fetched up front and pinned; any other series' waits until its page
/// opens and then lives by the cap like any other image.
macro_rules! drawn_anilist_ids {
    () => {
        "SELECT s.anilist_id FROM series s LEFT JOIN anilist_media m ON m.id = s.anilist_id
         WHERE s.provider = 'anilist' AND s.anilist_id IS NOT NULL AND (
            EXISTS (SELECT 1 FROM views v WHERE v.series_id = s.id)
            OR EXISTS (SELECT 1 FROM tracker_entries t
                       WHERE t.status IN ('watching', 'repeating') AND t.progress > 0
                         AND ((t.tracker = 'anilist' AND t.media_id = s.anilist_id)
                              OR (t.tracker = 'mal' AND t.media_id = m.mal_id))))"
    };
}

/// `drawn_anilist_ids!` for the queries outside this file.
pub const DRAWN_ANILIST_IDS_SQL: &str = drawn_anilist_ids!();

/// The urls of a series the library actually owns. These are never evicted
/// by the cap: losing them is the one loss a user would see at once.
const PINNED_SQL: &str = concat!("SELECT m.cover_url AS url FROM anilist_media m JOIN series s ON s.anilist_id = m.id WHERE m.cover_url IS NOT NULL
     UNION SELECT m.banner_url FROM anilist_media m JOIN series s ON s.anilist_id = m.id WHERE m.banner_url IS NOT NULL
     UNION SELECT t.poster_url FROM tvmaze_shows t JOIN series s ON s.tvmaze_id = t.id WHERE t.poster_url IS NOT NULL
     UNION SELECT t.background_url FROM tvmaze_shows t JOIN series s ON s.tvmaze_id = t.id WHERE t.background_url IS NOT NULL
     UNION SELECT url FROM (SELECT url FROM (
                SELECT a.url, row_number() OVER (
                    PARTITION BY a.anilist_id
                    ORDER BY CASE WHEN lower(a.lang) LIKE 'en%' THEN 0 WHEN a.lang IS NULL THEN 1 ELSE 2 END, a.rank
                ) AS pick
                FROM anizip_images a
                WHERE a.kind = 'Clearlogo'
                  AND a.anilist_id IN (SELECT anilist_id FROM series WHERE provider = 'anilist')
                  AND NOT EXISTS (SELECT 1 FROM logo_checks v WHERE v.url = a.url AND v.verdict = 0)
             ) WHERE pick = 1)
     UNION SELECT url FROM (SELECT url FROM (
                SELECT a.url, row_number() OVER (PARTITION BY a.anilist_id ORDER BY a.rank) AS pick
                FROM anizip_images a
                WHERE a.kind = 'Fanart'
                  AND a.anilist_id IN (", drawn_anilist_ids!(), ")
             ) WHERE pick = 1)
     UNION SELECT b.logo_url FROM fallback_shows b
           WHERE b.logo_url IS NOT NULL
             AND b.anilist_id IN (SELECT anilist_id FROM series WHERE provider = 'anilist')
             AND NOT EXISTS (SELECT 1 FROM anizip_images i WHERE i.anilist_id = b.anilist_id AND i.kind = 'Clearlogo'
                               AND NOT EXISTS (SELECT 1 FROM logo_checks v WHERE v.url = i.url AND v.verdict = 0))
             AND NOT EXISTS (SELECT 1 FROM logo_checks v WHERE v.url = b.logo_url AND v.verdict = 0)
     UNION SELECT url FROM tvdb_logo_wanted");

/// What a fill goes and gets: everything an owned series draws, the covers
/// of what it recommends, the covers behind the watching list, and the
/// covers of every entry in an owned series' franchise, walked the way the
/// graph closes it (CHARACTER and OTHER edges are never crossed), so the
/// series page's Related stones have their art offline too. From ani.zip
/// (#1036), an owned series' one clear logo, English first, the first
/// background of a series Continue may show (#1475), and the stills of the
/// episodes it has a file of; the rest of its artwork waits until something
/// asks.
const MISSING_SQL: &str = concat!("WITH RECURSIVE franchise(id) AS (
        SELECT anilist_id FROM series WHERE anilist_id IS NOT NULL
        UNION SELECT r.to_id FROM relations r JOIN franchise f ON r.from_id = f.id
              WHERE r.relation NOT IN ('CHARACTER', 'OTHER')
        UNION SELECT r.from_id FROM relations r JOIN franchise f ON r.to_id = f.id
              WHERE r.relation NOT IN ('CHARACTER', 'OTHER')
     )
     SELECT DISTINCT url FROM (
        SELECT m.cover_url AS url FROM anilist_media m JOIN series s ON s.anilist_id = m.id
        UNION SELECT m.banner_url FROM anilist_media m JOIN series s ON s.anilist_id = m.id
        UNION SELECT j.value ->> 'image_url' FROM anilist_media m JOIN series s ON s.anilist_id = m.id
                     CROSS JOIN json_each(m.characters) j
        UNION SELECT rm.cover_url FROM recommendations r
                     JOIN series s ON s.anilist_id = r.anilist_id
                     JOIN anilist_media rm ON rm.id = r.recommended_id
        UNION SELECT tm.cover_url FROM tracker_entries t
                     JOIN anilist_media tm ON (t.tracker = 'anilist' AND tm.id = t.media_id)
                                           OR (t.tracker = 'mal' AND tm.mal_id = t.media_id)
        UNION SELECT fm.cover_url FROM franchise f JOIN anilist_media fm ON fm.id = f.id
        UNION SELECT ts.poster_url FROM tvmaze_shows ts JOIN series s ON s.tvmaze_id = ts.id
        UNION SELECT ts.background_url FROM tvmaze_shows ts JOIN series s ON s.tvmaze_id = ts.id
        UNION SELECT url FROM (SELECT url FROM (
                SELECT a.url, row_number() OVER (
                    PARTITION BY a.anilist_id
                    ORDER BY CASE WHEN lower(a.lang) LIKE 'en%' THEN 0 WHEN a.lang IS NULL THEN 1 ELSE 2 END, a.rank
                ) AS pick
                FROM anizip_images a
                WHERE a.kind = 'Clearlogo'
                  AND a.anilist_id IN (SELECT anilist_id FROM series WHERE provider = 'anilist')
                  AND NOT EXISTS (SELECT 1 FROM logo_checks v WHERE v.url = a.url AND v.verdict = 0)
             ) WHERE pick = 1)
        UNION SELECT url FROM (SELECT url FROM (
                SELECT a.url, row_number() OVER (PARTITION BY a.anilist_id ORDER BY a.rank) AS pick
                FROM anizip_images a
                WHERE a.kind = 'Fanart'
                  AND a.anilist_id IN (", drawn_anilist_ids!(), ")
             ) WHERE pick = 1)
        UNION SELECT ze.still_url FROM anizip_episodes ze
                     JOIN series s ON s.anilist_id = ze.anilist_id AND s.provider = 'anilist'
                     JOIN files f ON f.series_id = s.id AND f.kind != 'extra' AND f.number = ze.number
        UNION SELECT b.logo_url FROM fallback_shows b
           WHERE b.logo_url IS NOT NULL
             AND b.anilist_id IN (SELECT anilist_id FROM series WHERE provider = 'anilist')
             AND NOT EXISTS (SELECT 1 FROM anizip_images i WHERE i.anilist_id = b.anilist_id AND i.kind = 'Clearlogo'
                               AND NOT EXISTS (SELECT 1 FROM logo_checks v WHERE v.url = i.url AND v.verdict = 0))
             AND NOT EXISTS (SELECT 1 FROM logo_checks v WHERE v.url = b.logo_url AND v.verdict = 0)
        UNION SELECT url FROM tvdb_logo_wanted
        UNION SELECT fe.still_url FROM fallback_episodes fe
                     JOIN series s ON s.anilist_id = fe.anilist_id AND s.provider = 'anilist'
                     JOIN files f ON f.series_id = s.id AND f.kind != 'extra' AND f.number = fe.number
                     WHERE NOT EXISTS (SELECT 1 FROM anizip_episodes ze2 WHERE ze2.anilist_id = fe.anilist_id AND ze2.number IS NOT NULL)
     )
     WHERE url IS NOT NULL AND url NOT IN (SELECT url FROM images)
     ORDER BY url");

// ---------------------------------------------------------------------------
// The pure rules
// ---------------------------------------------------------------------------

/// A url's identity on disk: the sha256 of the url itself, lowercase hex.
/// The url is the key everywhere, so nothing has to parse a CDN path.
pub fn key(url: &str) -> String {
    let digest = Sha256::digest(url.as_bytes());
    let mut out = String::with_capacity(64);
    for byte in digest.as_slice() {
        // Writing into a String is infallible; the result exists only
        // because `Write` is one trait for files and strings alike.
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// `<first two hex>/<hash>.<ext>`, relative to the images directory. The
/// shard keeps any one directory to a few hundred files on a big library.
pub fn relative_path(url: &str, ext: &str) -> String {
    let hash = key(url);
    format!("{}/{hash}.{ext}", &hash[..2])
}

/// Where a clear logo trimmed to its mark is kept: a name of its own, so a
/// view that already drew the untrimmed file at the old path loads it anew.
pub fn trimmed_path(url: &str) -> String {
    let hash = key(url);
    format!("{}/{hash}.trim.png", &hash[..2])
}

/// What to call the file. The url's own extension wins, since AniList's CDN
/// names its files honestly; the content type answers when it does not; jpg
/// is the last resort, because the byte content is what a decoder reads and
/// the extension is only a hint to it.
pub fn extension(url: &str, content_type: Option<&str>) -> &'static str {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let name = path.rsplit('/').next().unwrap_or(path);
    if let Some((_, ext)) = name.rsplit_once('.') {
        let lower = ext.to_ascii_lowercase();
        if let Some(known) = IMAGE_EXTENSIONS.iter().find(|e| **e == lower) {
            return known;
        }
    }
    let mime = content_type.unwrap_or_default();
    let mime = mime
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match mime.as_str() {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/avif" => "avif",
        _ => "jpg",
    }
}

/// A fetch that failed. The CDN is AniList's, so that is the provider a
/// shell is told about; `status` is None when nothing answered at all.
fn cdn_error(status: Option<u32>, message: impl Into<String>) -> CoreError {
    CoreError::provider(Provider::Anilist, status, message)
}

// ---------------------------------------------------------------------------
// The cache
// ---------------------------------------------------------------------------

/// What one sweep did. Rows removed for the first three reasons are
/// `removed_rows`; rows the cap took are `evicted`; every file deleted from
/// the directory, whatever the reason, is `removed_files`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SweepReport {
    pub removed_rows: u64,
    pub removed_files: u64,
    pub evicted: u64,
}

pub struct ImageCache {
    store: Arc<Store>,
    dir: PathBuf,
    http: Arc<dyn Http>,
    permits: Arc<Semaphore>,
    /// When a fill last actually started. A coalesced call does not touch
    /// it, so the five minute gate measures real fills.
    fill_started_at: Mutex<Option<Instant>>,
    /// Urls a page asked for that no fill goes and gets by itself (#1475):
    /// the background of a series opened outside Continue. The next fill
    /// takes them, and a running one takes them before it finishes.
    wanted: Mutex<HashSet<String>>,
    /// Wanted urls that failed once, so a page that keeps opening does not
    /// start a fill for them every time. Forgotten with the process.
    refused: Mutex<HashSet<String>>,
}

impl ImageCache {
    pub fn new(store: Arc<Store>, images_dir: PathBuf, http: Arc<dyn Http>) -> Arc<ImageCache> {
        Arc::new(ImageCache {
            store,
            dir: images_dir,
            http,
            permits: Arc::new(Semaphore::new(CONCURRENCY)),
            fill_started_at: Mutex::new(None),
            wanted: Mutex::new(HashSet::new()),
            refused: Mutex::new(HashSet::new()),
        })
    }

    /// Asks the next fill for `url`, which nothing fetches up front. True
    /// when it was not asked for already and has not failed before, which
    /// is when the caller should start a fill.
    pub fn want(&self, url: &str) -> bool {
        if recover(&self.refused).contains(url) {
            return false;
        }
        recover(&self.wanted).insert(url.to_string())
    }

    fn take_wanted(&self) -> Vec<String> {
        let mut wanted: Vec<String> = recover(&self.wanted).drain().collect();
        wanted.sort_unstable();
        wanted
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Nothing in here is worth poisoning a lock over: the guarded value is
    /// one `Instant`, and a panicking job must not wedge the gate for the
    /// rest of the process.
    fn fill_gate(&self) -> MutexGuard<'_, Option<Instant>> {
        recover(&self.fill_started_at)
    }

    /// Whether a read that found a gap should ask for a fill.
    pub fn fill_is_due(&self) -> bool {
        self.fill_gate()
            .is_none_or(|at| at.elapsed() >= FILL_INTERVAL)
    }

    pub fn mark_fill_started(&self) {
        *self.fill_gate() = Some(Instant::now());
    }

    /// The local absolute path of every url, fetching what is missing, at
    /// most four at a time. A url whose row has lost its file is refetched.
    /// A failure is that url's own: the batch always comes back whole.
    ///
    /// The fetches are spawned, so this is called from inside the runtime:
    /// a job body, or a task on it. Dropping the future, which is what
    /// cancelling a job does, aborts whatever is still in flight.
    pub async fn ensure(&self, urls: &[String]) -> Vec<(String, Result<String, CoreError>)> {
        let mut wanted: Vec<String> = urls.to_vec();
        wanted.sort_unstable();
        wanted.dedup();
        if wanted.is_empty() {
            return Vec::new();
        }

        let lookup = wanted.clone();
        let known = match self.store.write_async(move |c| rows_for(c, &lookup)).await {
            Ok(known) => known,
            Err(e) => return urls.iter().map(|u| (u.clone(), Err(e.clone()))).collect(),
        };

        let mut done: HashMap<String, Result<String, CoreError>> = HashMap::new();
        let mut stale: Vec<String> = Vec::new();
        let mut fetch: Vec<String> = Vec::new();
        for url in wanted {
            match known.get(&url) {
                Some(relative) if self.dir.join(relative).is_file() => {
                    done.insert(
                        url,
                        Ok(self.dir.join(relative).to_string_lossy().into_owned()),
                    );
                }
                Some(_) => {
                    stale.push(url.clone());
                    fetch.push(url);
                }
                None => fetch.push(url),
            }
        }
        // The row goes before the refetch, so a fetch that fails leaves the
        // library telling the truth: no image rather than a broken path.
        if !stale.is_empty()
            && let Err(e) = self
                .store
                .write_async(move |c| delete_rows(c, &stale))
                .await
        {
            tracing::warn!("clearing image rows whose files were gone failed: {e}");
        }

        let mut set = tokio::task::JoinSet::new();
        for url in fetch {
            let http = self.http.clone();
            let store = self.store.clone();
            let dir = self.dir.clone();
            let permits = self.permits.clone();
            set.spawn(async move {
                let outcome = match permits.acquire_owned().await {
                    Ok(_permit) => fetch_one(http.as_ref(), &store, &dir, &url).await,
                    Err(e) => Err(CoreError::internal(format!("image permits: {e}"))),
                };
                (url, outcome)
            });
        }
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok((url, outcome)) => {
                    done.insert(url, outcome);
                }
                Err(e) => tracing::warn!("an image fetch ended without a result: {e}"),
            }
        }

        urls.iter()
            .map(|url| {
                let outcome = done
                    .get(url)
                    .cloned()
                    .unwrap_or_else(|| Err(CoreError::internal(format!("no outcome for {url}"))));
                (url.clone(), outcome)
            })
            .collect()
    }

    /// The local path of a url already cached, and only when its file is
    /// still there: a row pointing at nothing is the same as no row.
    pub fn path_for(&self, conn: &Connection, url: &str) -> Result<Option<String>, CoreError> {
        let relative: Option<String> = conn
            .query_row(
                "SELECT path FROM images WHERE url = ?1",
                params![url],
                |r| r.get(0),
            )
            .optional()?;
        Ok(relative
            .map(|relative| self.dir.join(relative))
            .filter(|path| path.is_file())
            .map(|path| path.to_string_lossy().into_owned()))
    }

    /// Marks images as used, on the writer thread, with no reply waited
    /// for: a read that hands out posters must not pay for the bookkeeping
    /// that keeps them out of the sweep.
    pub fn bump_used(&self, urls: Vec<String>) {
        if urls.is_empty() {
            return;
        }
        let now = time::now_secs();
        self.store.post(move |c| {
            let sql = format!(
                "UPDATE images SET used_at = ? WHERE url IN ({}) AND used_at < ?",
                placeholders(urls.len())
            );
            let mut values: Vec<Value> = Vec::with_capacity(urls.len() + 2);
            values.push(Value::Integer(now));
            values.extend(urls.into_iter().map(Value::Text));
            values.push(Value::Integer(now - USED_GAP));
            if let Err(e) = c.execute(&sql, params_from_iter(values)) {
                tracing::warn!("marking images used failed: {e}");
            }
        });
    }

    pub fn storage(&self, conn: &Connection) -> Result<(u64, u64), CoreError> {
        let (count, bytes): (i64, i64) = conn.query_row(
            "SELECT count(*), COALESCE(sum(bytes), 0) FROM images",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok((count.max(0) as u64, bytes.max(0) as u64))
    }

    /// The four steps, in one transaction for the rows and one walk of the
    /// directory for the files: a row whose file is gone, an orphan past
    /// its thirty days, and the cap, all decided inside the transaction;
    /// then, once it has committed, every file the surviving rows do not
    /// name is deleted, which is both the second step and the disk half of
    /// the third and fourth.
    pub fn sweep(&self, conn: &mut Connection, now: i64) -> Result<SweepReport, CoreError> {
        self.sweep_to(conn, now, MAX_ROWS)
    }

    fn sweep_to(
        &self,
        conn: &mut Connection,
        now: i64,
        cap: u64,
    ) -> Result<SweepReport, CoreError> {
        let mut report = SweepReport::default();
        let keep: HashSet<String>;
        {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let mut rows = all_rows(&tx)?;
            let referenced = urls_from(&tx, REFERENCED_SQL)?;
            let pinned = urls_from(&tx, PINNED_SQL)?;

            // (1) the file is gone, so the row is a lie.
            let mut removed: Vec<String> = Vec::new();
            rows.retain(|row| {
                if self.dir.join(&row.path).is_file() {
                    true
                } else {
                    removed.push(row.url.clone());
                    false
                }
            });
            // (3) nothing references it and nothing has used it in a month.
            rows.retain(|row| {
                if !referenced.contains(&row.url) && row.used_at < now - ORPHAN_AGE {
                    removed.push(row.url.clone());
                    false
                } else {
                    true
                }
            });
            report.removed_rows = removed.len() as u64;

            // (4) the cap, orphans first and least recently used next, and
            // never a poster or a banner of a series the library owns.
            if rows.len() as u64 > cap {
                let mut candidates: Vec<usize> = (0..rows.len())
                    .filter(|i| !pinned.contains(&rows[*i].url))
                    .collect();
                candidates.sort_by_key(|i| {
                    (
                        u8::from(referenced.contains(&rows[*i].url)),
                        rows[*i].used_at,
                    )
                });
                let over = (rows.len() as u64 - cap) as usize;
                let evicted: HashSet<String> = candidates
                    .into_iter()
                    .take(over)
                    .map(|i| rows[i].url.clone())
                    .collect();
                report.evicted = evicted.len() as u64;
                rows.retain(|row| !evicted.contains(&row.url));
                removed.extend(evicted);
            }

            delete_rows(&tx, &removed)?;
            keep = rows.into_iter().map(|row| row.path).collect();
            tx.commit()?;
        }
        // (2) every file no surviving row names, which is a file that never
        // had a row, a leftover .tmp, and the files of everything the steps
        // above just deleted.
        report.removed_files = self.delete_files_outside(&keep);
        Ok(report)
    }

    /// Deletes every file the surviving rows do not name. Run this on the
    /// writer thread, inside the same task as the transaction above: the
    /// row a fetch is about to insert is then queued behind this sweep
    /// rather than racing it. A file written by a fetch whose row has not
    /// been queued yet can still be taken, and that heals itself, since a
    /// row with no file is deleted and refetched by the next `ensure`.
    fn delete_files_outside(&self, keep: &HashSet<String>) -> u64 {
        let mut removed = 0;
        for entry in walkdir::WalkDir::new(&self.dir)
            .into_iter()
            .filter_map(Result::ok)
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let Ok(relative) = entry.path().strip_prefix(&self.dir) else {
                continue;
            };
            // Rows name files as "ab/<hash>.jpg"; Windows walks them as "ab\<hash>.jpg".
            let relative = relative
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            if keep.contains(&relative) {
                continue;
            }
            match std::fs::remove_file(entry.path()) {
                Ok(()) => removed += 1,
                // Something else got there first, which is the outcome
                // this wanted anyway.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => tracing::warn!("cannot delete {}: {e}", entry.path().display()),
            }
        }
        removed
    }
}

// ---------------------------------------------------------------------------
// The queries
// ---------------------------------------------------------------------------

struct CachedRow {
    url: String,
    path: String,
    used_at: i64,
}

fn all_rows(conn: &Connection) -> Result<Vec<CachedRow>, CoreError> {
    let mut stmt = conn.prepare("SELECT url, path, used_at FROM images")?;
    let rows = stmt.query_map([], |r| {
        Ok(CachedRow {
            url: r.get(0)?,
            path: r.get(1)?,
            used_at: r.get(2)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

fn urls_from(conn: &Connection, sql: &str) -> Result<HashSet<String>, CoreError> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<Result<HashSet<_>, _>>()?)
}

/// Url to relative path, for the urls asked about that have a row.
fn rows_for(conn: &Connection, urls: &[String]) -> Result<HashMap<String, String>, CoreError> {
    let mut out = HashMap::new();
    for chunk in urls.chunks(500) {
        let sql = format!(
            "SELECT url, path FROM images WHERE url IN ({})",
            placeholders(chunk.len())
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(chunk.iter()), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (url, path) = row?;
            out.insert(url, path);
        }
    }
    Ok(out)
}

fn delete_rows(conn: &Connection, urls: &[String]) -> Result<(), CoreError> {
    for chunk in urls.chunks(500) {
        let sql = format!(
            "DELETE FROM images WHERE url IN ({})",
            placeholders(chunk.len())
        );
        conn.execute(&sql, params_from_iter(chunk.iter()))?;
    }
    Ok(())
}

/// The series a set of fetched urls belongs to: the ones whose card or page
/// now has a picture it did not have a moment ago.
fn series_for_urls(conn: &Connection, urls: &[String]) -> Result<Vec<u64>, CoreError> {
    if urls.is_empty() {
        return Ok(Vec::new());
    }
    let list = placeholders(urls.len());
    let sql = format!(
        "SELECT s.id FROM series s JOIN anilist_media m ON m.id = s.anilist_id
         WHERE m.cover_url IN ({list}) OR m.banner_url IN ({list})
         UNION
         SELECT s.id FROM series s JOIN tvmaze_shows t ON t.id = s.tvmaze_id
         WHERE t.poster_url IN ({list}) OR t.background_url IN ({list})
         UNION
         SELECT s.id FROM series s JOIN anizip_images a ON a.anilist_id = s.anilist_id
         WHERE a.url IN ({list})
         UNION
         SELECT s.id FROM series s JOIN anizip_episodes z ON z.anilist_id = s.anilist_id
         WHERE z.still_url IN ({list})
         UNION
         SELECT s.id FROM series s JOIN fallback_shows b ON b.anilist_id = s.anilist_id
         WHERE b.logo_url IN ({list})
         UNION
         SELECT s.id FROM series s JOIN fallback_episodes e ON e.anilist_id = s.anilist_id
         WHERE e.still_url IN ({list})
         UNION
         SELECT s.id FROM series s JOIN tvdb_logo_wanted w ON w.anilist_id = s.anilist_id
         WHERE w.url IN ({list})"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        params_from_iter(
            urls.iter()
                .chain(urls)
                .chain(urls)
                .chain(urls)
                .chain(urls)
                .chain(urls)
                .chain(urls)
                .chain(urls)
                .chain(urls),
        ),
        |r| r.get::<_, i64>(0),
    )?;
    Ok(rows
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|id| id as u64)
        .collect())
}

fn matched_series(conn: &Connection) -> Result<Vec<u64>, CoreError> {
    let mut stmt = conn.prepare(
        "SELECT id FROM series WHERE anilist_id IS NOT NULL OR tvmaze_id IS NOT NULL ORDER BY id",
    )?;
    let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
    Ok(rows
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|id| id as u64)
        .collect())
}

fn missing_urls(conn: &Connection) -> Result<Vec<String>, CoreError> {
    let mut stmt = conn.prepare(MISSING_SQL)?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

// ---------------------------------------------------------------------------
// The fetch
// ---------------------------------------------------------------------------

/// A clear logo's file judged off the runtime (`logo_image`).
struct Judged {
    /// The bytes to keep: the file cut to its mark, or the file as it was.
    body: Vec<u8>,
    /// What `logo_checks.verdict` records.
    verdict: i64,
    /// Whether `body` is a trimmed file, which goes under `trimmed_path`.
    trimmed: bool,
}

async fn judge_logo(body: Vec<u8>) -> Option<Judged> {
    let judged = tokio::task::spawn_blocking(move || {
        let file = logo_image::inspect(&body);
        let verdict = file.check();
        match file {
            LogoFile::Clear {
                trimmed: Some(trimmed),
            } => Judged {
                body: trimmed,
                verdict,
                trimmed: true,
            },
            _ => Judged {
                body,
                verdict,
                trimmed: false,
            },
        }
    })
    .await;
    judged
        .inspect_err(|e| tracing::warn!("judging a logo failed: {e}"))
        .ok()
}

/// Records what the cache found a clear logo to be, apart from the cached
/// file, so the verdict outlives it.
fn record_verdict(c: &Connection, url: &str, verdict: i64, now: i64) -> Result<(), CoreError> {
    c.execute(
        "INSERT INTO logo_checks (url, verdict, checked_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(url) DO UPDATE SET verdict = excluded.verdict, checked_at = excluded.checked_at",
        params![url, verdict, now],
    )?;
    Ok(())
}

impl ImageCache {
    /// Judges the clear logos cached before the cache judged them as they
    /// land (#1470): each is trimmed to its mark in place, or marked as not
    /// a logo. Returns the urls whose file or standing changed.
    pub async fn check_logos(&self) -> Vec<String> {
        let rows: Vec<(String, String)> = self
            .store
            .read(|c| {
                let mut stmt = c.prepare(
                    "SELECT i.url, i.path FROM images i
                     WHERE instr(lower(i.url), '/clearlogo/') > 0
                       AND NOT EXISTS (SELECT 1 FROM logo_checks v WHERE v.url = i.url)",
                )?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                Ok(rows.collect::<Result<Vec<_>, _>>()?)
            })
            .unwrap_or_default();
        let mut changed = Vec::new();
        for (url, relative) in rows {
            let old = self.dir.join(&relative);
            let Ok(body) = tokio::fs::read(&old).await else {
                continue;
            };
            let Some(judged) = judge_logo(body).await else {
                continue;
            };
            // A trimmed file goes under a name of its own, and the row
            // moves to it, so a view that drew the old file reloads.
            let relative = if judged.trimmed {
                let trimmed = trimmed_path(&url);
                if let Err(e) =
                    atomic_file::write_async(&self.dir.join(&trimmed), judged.body.clone()).await
                {
                    tracing::warn!("trimming the logo {url} failed: {e}");
                    continue;
                }
                trimmed
            } else {
                relative
            };
            let (row_url, row_path, verdict) = (url.clone(), relative.clone(), judged.verdict);
            let bytes = judged.body.len() as i64;
            let now = time::now_secs();
            if let Err(e) = self
                .store
                .write_async(move |c| {
                    c.execute(
                        "UPDATE images SET path = ?2, bytes = ?3 WHERE url = ?1",
                        params![row_url, row_path, bytes],
                    )?;
                    record_verdict(c, &row_url, verdict, now)
                })
                .await
            {
                tracing::warn!("recording the logo {url} failed: {e}");
                continue;
            }
            if judged.trimmed {
                let _ = tokio::fs::remove_file(&old).await;
            }
            if judged.verdict != 1 || judged.trimmed {
                changed.push(url);
            }
        }
        changed
    }
}

/// One url: get it, write it beside its final name, rename it into place,
/// then record the row. The rename is what makes a half-written file
/// impossible to read as a cached image, since only a complete file ever
/// wears the real name.
async fn fetch_one(
    http: &dyn Http,
    store: &Store,
    dir: &Path,
    url: &str,
) -> Result<String, CoreError> {
    let request = HttpRequest::get(url);
    let response = http
        .send(request)
        .await
        .map_err(|e| cdn_error(None, e.message))?;
    if !response.is_success() {
        return Err(cdn_error(
            Some(u32::from(response.status)),
            format!("image fetch failed: {}", response.text()),
        ));
    }
    let mut relative = relative_path(url, extension(url, response.header("content-type")));
    let (body, verdict) = if crate::anizip::is_clear_logo(url) {
        let Some(judged) = judge_logo(response.body).await else {
            return Err(CoreError::internal(format!(
                "judging the logo {url} failed"
            )));
        };
        if judged.trimmed {
            relative = trimmed_path(url);
        }
        (judged.body, Some(judged.verdict))
    } else {
        (response.body, None)
    };
    let path = dir.join(&relative);
    let bytes = body.len() as i64;
    atomic_file::write_async(&path, body).await?;

    let now = time::now_secs();
    let (row_url, row_path) = (url.to_string(), relative);
    store
        .write_async(move |c| {
            c.execute(
                "INSERT OR REPLACE INTO images (url, path, bytes, fetched_at, used_at) VALUES (?1, ?2, ?3, ?4, ?4)",
                params![row_url, row_path, bytes, now],
            )?;
            if let Some(verdict) = verdict {
                record_verdict(c, &row_url, verdict, now)?;
            }
            Ok(())
        })
        .await?;
    Ok(path.to_string_lossy().into_owned())
}

// ---------------------------------------------------------------------------
// The jobs
// ---------------------------------------------------------------------------

/// The fill: every url an owned series, its recommendations or the tracker
/// lists would draw, that has no row yet. It runs one at a time, so a read
/// asking for a second one is handed the running job's id.
pub fn start_fill(net: &Arc<Net>) -> u64 {
    // A coalesced call is not a fill starting, so it must not reset the
    // gate the reads consult.
    let coalesced = net.services.jobs.running(JobKind::FillImages).is_some();
    let owner = net.clone();
    let id = net
        .services
        .jobs
        .clone()
        .start(JobKind::FillImages, move |ctx| async move {
            // The logos cached before they were judged are judged first, so
            // the urls a logo that is none gives way to are among the missing.
            let judged = owner.images.check_logos().await;
            let mut urls = owner
                .services
                .store
                .write_async(|c| missing_urls(c))
                .await?;
            // A fill of only what pages asked for is chatter, not news.
            let asked_only = urls.is_empty();
            urls.extend(owner.images.take_wanted());
            let (mut fetched, mut failed) = (0u64, 0u64);
            let mut touched: HashSet<u64> = HashSet::new();
            if !judged.is_empty() {
                touched.extend(
                    owner
                        .services
                        .store
                        .write_async(move |c| series_for_urls(c, &judged))
                        .await?,
                );
            }

            // What a page asks for while this runs is taken on before it
            // ends, so no ask waits for the next fill.
            while !urls.is_empty() {
                let total = urls.len() as u64;
                for (round, batch) in urls.chunks(BATCH).enumerate() {
                    ctx.checkpoint()?;
                    ctx.progress(
                        (round * BATCH) as u64,
                        Some(total),
                        &anibeam_contract::i18n::tr("core-images-progress-fetching", &[]),
                    );
                    let mut arrived: Vec<String> = Vec::new();
                    for (url, outcome) in owner.images.ensure(batch).await {
                        match outcome {
                            Ok(_) => {
                                fetched += 1;
                                arrived.push(url);
                            }
                            Err(e) => {
                                failed += 1;
                                tracing::warn!("image {url} was not fetched: {e}");
                                recover(&owner.images.refused).insert(url);
                            }
                        }
                    }
                    if !arrived.is_empty() {
                        touched.extend(
                            owner
                                .services
                                .store
                                .write_async(move |c| series_for_urls(c, &arrived))
                                .await?,
                        );
                    }
                }
                urls = owner.images.take_wanted();
            }

            if !touched.is_empty() {
                let mut ids: Vec<u64> = touched.into_iter().collect();
                ids.sort_unstable();
                let dir = owner.services.paths.images_dir();
                let cards_for = owner.cards_for;
                let cards = owner
                    .services
                    .store
                    .write_async(move |c| cards_for(c, &dir, &ids))
                    .await?;
                ctx.emit(
                    Level::Debug,
                    format!("fetched {fetched} images"),
                    EventBody::SeriesChanged { series: cards },
                );
            }
            Ok(Finished {
                plain: None,
                // Every launch runs this, and on most of them there is
                // nothing to fill: a line saying so belongs in the trace
                // rather than in a log of state changes.
                level: if asked_only || (fetched == 0 && failed == 0) {
                    Level::Debug
                } else {
                    Level::Info
                },
                message: format!("Images filled: {fetched} fetched, {failed} failed"),
                body: EventBody::Notice,
            })
        });
    if !coalesced {
        net.images.mark_fill_started();
    }
    id
}

/// `GetStorage`: two counts off one table, answered off the reader
/// connection rather than as a job.
pub fn storage_call(net: &Net) -> Result<Reply, CoreError> {
    let (image_count, image_bytes) = net.services.store.read(|c| net.images.storage(c))?;
    Ok(Reply::Storage {
        image_count,
        image_bytes,
    })
}

/// Clear images: every row and every file, and a card for every matched
/// series so a shell showing posters drops them at once.
pub fn start_clear(net: &Net) -> u64 {
    let owner = net.services.clone();
    let cards_for = net.cards_for;
    net.services
        .jobs
        .clone()
        .start(JobKind::ClearImages, move |ctx| async move {
            let removed = owner
                .store
                .write_async(|c| {
                    let count: i64 =
                        c.query_row("SELECT count(*) FROM images", [], |r| r.get(0))?;
                    c.execute("DELETE FROM images", [])?;
                    Ok(count.max(0) as u64)
                })
                .await?;

            // The whole tree goes rather than the listed files: a file with no
            // row is exactly what this call is also for.
            let dir = owner.paths.images_dir();
            match tokio::fs::remove_dir_all(&dir).await {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(CoreError::io_at(dir.to_string_lossy(), e)),
            }
            tokio::fs::create_dir_all(&dir)
                .await
                .map_err(|e| CoreError::io_at(dir.to_string_lossy(), e))?;

            let ids = owner.store.write_async(|c| matched_series(c)).await?;
            if !ids.is_empty() {
                let images_dir = dir.clone();
                let cards = owner
                    .store
                    .write_async(move |c| cards_for(c, &images_dir, &ids))
                    .await?;
                ctx.emit(
                    Level::Debug,
                    format!("{} series lost their images", cards.len()),
                    EventBody::SeriesChanged { series: cards },
                );
            }
            Ok(Finished {
                plain: None,
                level: Level::Info,
                message: format!("Images cleared: {removed} removed"),
                body: EventBody::ImagesCleared { removed },
            })
        })
}

/// The sweep every job that wrote records owes, and the launch owes once.
/// A record brings covers, banners and portraits in, and this is what keeps
/// the directory from only ever growing. Its report is bookkeeping rather
/// than a state change, so it goes to the trace log and never to the
/// activity log, and a failure costs nothing but disk.
pub async fn sweep_after(net: &Net, what: &str) {
    let cache = net.images.clone();
    let now = anibeam_foundation::time::now_secs();
    match net
        .services
        .store
        .write_async(move |c| cache.sweep(c, now))
        .await
    {
        Ok(report) => tracing::debug!(
            "image sweep after {what}: {} rows removed, {} evicted, {} files removed",
            report.removed_rows,
            report.evicted,
            report.removed_files
        ),
        Err(e) => tracing::debug!("the image sweep after {what} failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FakeHttp;

    #[test]
    fn the_key_is_the_urls_sha256_and_the_path_shards_on_its_first_two_hex() {
        let k = key("https://img/xl.jpg");
        assert_eq!(k.len(), 64);
        assert!(
            k.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase())
        );
        assert_eq!(
            key("https://img/xl.jpg"),
            k,
            "the same url always hashes the same"
        );
        assert_ne!(key("https://img/other.jpg"), k);
        assert_eq!(
            relative_path("https://img/xl.jpg", "jpg"),
            format!("{}/{k}.jpg", &k[..2])
        );
    }

    #[test]
    fn the_extension_comes_from_the_url_then_the_content_type_then_jpg() {
        assert_eq!(extension("https://img/a.png", None), "png");
        assert_eq!(extension("https://img/a.PNG", None), "png");
        assert_eq!(extension("https://img/a.jpeg", Some("image/png")), "jpeg");
        assert_eq!(extension("https://img/a.webp?v=2", None), "webp");
        assert_eq!(extension("https://img/a.avif#x", None), "avif");
        assert_eq!(extension("https://img/a.bin", Some("image/gif")), "gif");
        assert_eq!(
            extension("https://img/nodot", Some("image/webp; charset=binary")),
            "webp"
        );
        assert_eq!(
            extension("https://img/nodot", Some("application/octet-stream")),
            "jpg"
        );
        assert_eq!(extension("https://img/nodot", None), "jpg");
        assert_eq!(
            extension("https://cdn.example.com/no-extension-here", None),
            "jpg"
        );
    }

    // The sweep is easier to judge against rows written by hand than
    // against a fill, so this builds the four cases directly.
    struct Bench {
        _dir: tempfile::TempDir,
        store: Arc<Store>,
        cache: Arc<ImageCache>,
        now: i64,
    }

    fn bench() -> Bench {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("data").join("anibeam.db")).unwrap();
        let images = dir.path().join("images");
        std::fs::create_dir_all(&images).unwrap();
        let cache = ImageCache::new(store.clone(), images, FakeHttp::new());
        Bench {
            _dir: dir,
            store,
            cache,
            now: 1_700_000_000,
        }
    }

    impl Bench {
        /// A row and, unless `on_disk` is false, the file it names.
        fn add(&self, url: &str, used_at: i64, on_disk: bool) {
            let relative = relative_path(url, "jpg");
            if on_disk {
                let path = self.cache.dir().join(&relative);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, b"xx").unwrap();
            }
            let (url, relative) = (url.to_string(), relative);
            self.store
                .write(move |c| {
                    c.execute(
                        "INSERT INTO images (url, path, bytes, fetched_at, used_at) VALUES (?1, ?2, 2, ?3, ?3)",
                        params![url, relative, used_at],
                    )?;
                    Ok(())
                })
                .unwrap();
        }

        fn urls(&self) -> Vec<String> {
            self.store
                .read(|c| {
                    let mut stmt = c.prepare("SELECT url FROM images ORDER BY url")?;
                    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
                    Ok(rows.collect::<Result<Vec<_>, _>>()?)
                })
                .unwrap()
        }

        fn files(&self) -> usize {
            walkdir::WalkDir::new(self.cache.dir())
                .into_iter()
                .filter_map(Result::ok)
                .filter(|e| e.file_type().is_file())
                .count()
        }
    }

    #[test]
    fn the_sweep_drops_lost_rows_stray_files_and_old_orphans_then_caps_what_is_left() {
        let b = bench();
        let (now, day) = (b.now, 86_400);
        // A matched series pins its cover and banner; a second media row is
        // referenced but owned by nothing.
        b.store
            .write(move |c| {
                c.execute(
                    "INSERT INTO anilist_media (id, cover_url, banner_url, characters) VALUES (1, 'pinned-cover', 'pinned-banner', '[{\"image_url\":\"portrait\"}]')",
                    [],
                )?;
                c.execute("INSERT INTO anilist_media (id, cover_url) VALUES (2, 'loose-cover')", [])?;
                c.execute("INSERT INTO sources (path, added_at) VALUES ('/lib', 1)", [])?;
                c.execute(
                    "INSERT INTO series (source_id, kind, path, folder_name, added_at, anilist_id) VALUES (1, 'show', '/lib/A', 'A', 1, 1)",
                    [],
                )?;
                Ok(())
            })
            .unwrap();

        b.add("pinned-cover", now, true);
        b.add("pinned-banner", now, false); // (1) the file is gone
        b.add("portrait", now - day, true);
        b.add("loose-cover", now - 10 * day, true);
        b.add("old-orphan", now - 40 * day, true); // (3) unreferenced and stale
        b.add("new-orphan", now, true); // young enough to survive step 3
        // (2) a file under the directory that no row names
        std::fs::write(b.cache.dir().join("stray.jpg"), b"x").unwrap();

        let cache = b.cache.clone();
        // (4) the cap, small enough to bite: four rows survive the first
        // three steps and two of them have to go.
        let report = b.store.write(move |c| cache.sweep_to(c, now, 2)).unwrap();

        assert_eq!(report.removed_rows, 2, "the lost row and the old orphan");
        assert_eq!(report.evicted, 2);
        // The old orphan's, the new orphan's, the loose cover's, the stray.
        assert_eq!(report.removed_files, 4);
        // The orphan goes before anything referenced, then the least
        // recently used, and the pinned cover is never a candidate.
        assert_eq!(
            b.urls(),
            vec!["pinned-cover".to_string(), "portrait".to_string()]
        );
        assert_eq!(b.files(), 2);
    }

    /// A TVmaze show no series points at any more, one the series moved
    /// off or cleared, references nothing: its pictures age out like any
    /// other orphan, while a show a series is matched to keeps its own.
    #[test]
    fn a_tvmaze_show_no_series_points_at_lets_its_images_go() {
        let b = bench();
        let (now, day) = (b.now, 86_400);
        b.store
            .write(move |c| {
                c.execute(
                    "INSERT INTO tvmaze_shows (id, poster_url, background_url, fetched_at) VALUES (1, 'owned-poster', 'owned-background', 0)",
                    [],
                )?;
                c.execute(
                    "INSERT INTO tvmaze_shows (id, poster_url, background_url, fetched_at) VALUES (2, 'left-poster', 'left-background', 0)",
                    [],
                )?;
                c.execute("INSERT INTO sources (path, added_at) VALUES ('/lib', 1)", [])?;
                c.execute(
                    "INSERT INTO series (source_id, kind, path, folder_name, added_at, provider, tvmaze_id) VALUES (1, 'show', '/lib/A', 'A', 1, 'tvmaze', 1)",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        for url in [
            "owned-poster",
            "owned-background",
            "left-poster",
            "left-background",
        ] {
            b.add(url, now - 40 * day, true);
        }
        let cache = b.cache.clone();
        let report = b.store.write(move |c| cache.sweep(c, now)).unwrap();
        assert_eq!(report.removed_rows, 2);
        assert_eq!(
            b.urls(),
            vec!["owned-background".to_string(), "owned-poster".to_string()]
        );
    }

    #[test]
    fn a_fill_wants_the_covers_of_an_owned_series_franchise_but_not_across_a_cameo() {
        let b = bench();
        b.store
            .write(|c| {
                c.execute_batch(
                    "INSERT INTO sources (id, path, added_at) VALUES (1, '/lib', 0);
                     INSERT INTO anilist_media (id, cover_url) VALUES (1, 'own'), (2, 'sequel'), (3, 'novel'), (4, 'cameo'), (5, 'cameo-sequel');
                     INSERT INTO series (id, source_id, kind, path, folder_name, added_at, anilist_id) VALUES (1, 1, 'show', '/lib/a', 'a', 0, 1);
                     INSERT INTO relations (from_id, to_id, relation) VALUES (1, 2, 'SEQUEL'), (3, 2, 'ADAPTATION'), (1, 4, 'CHARACTER'), (4, 5, 'SEQUEL');",
                )?;
                Ok(())
            })
            .unwrap();
        b.add("sequel", b.now, true);
        let want = b.store.read(missing_urls).unwrap();
        assert_eq!(want, vec!["novel".to_string(), "own".to_string()]);
    }

    #[test]
    fn a_sweep_of_an_untouched_cache_changes_nothing() {
        let b = bench();
        b.add("kept", b.now, true);
        let cache = b.cache.clone();
        let now = b.now;
        let report = b.store.write(move |c| cache.sweep(c, now)).unwrap();
        assert_eq!(report, SweepReport::default());
        assert_eq!(b.urls(), vec!["kept".to_string()]);
        assert_eq!(b.files(), 1);
    }

    #[test]
    fn bump_used_only_moves_an_image_a_day_stale_and_path_for_ignores_a_lost_file() {
        let b = bench();
        // `bump_used` reads the real clock, so these rows are placed
        // against it rather than against the bench's fixed instant.
        let now = time::now_secs();
        b.add("fresh", now, true);
        b.add("stale", now - 5 * 86_400, true);
        b.add("lost", now - 5 * 86_400, false);
        b.cache
            .bump_used(vec!["fresh".into(), "stale".into(), "lost".into()]);
        // The next write runs after the posted one, so this sees it.
        let used: Vec<(String, i64)> = b
            .store
            .write(|c| {
                let mut stmt = c.prepare("SELECT url, used_at FROM images ORDER BY url")?;
                let rows =
                    stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
                Ok(rows.collect::<Result<Vec<_>, _>>()?)
            })
            .unwrap();
        let at = |url: &str| {
            used.iter()
                .find(|(u, _)| u == url)
                .map(|(_, at)| *at)
                .unwrap()
        };
        assert_eq!(
            at("fresh"),
            now,
            "a bump inside the day is not worth a write"
        );
        assert!(at("stale") >= now, "a day-stale row is moved up to now");

        let cache = b.cache.clone();
        let (kept, lost) = b
            .store
            .read(|c| Ok((cache.path_for(c, "fresh")?, cache.path_for(c, "lost")?)))
            .unwrap();
        assert!(kept.is_some_and(|p| p.ends_with(".jpg")));
        assert_eq!(lost, None, "a row whose file is gone is the same as no row");
    }

    #[tokio::test]
    async fn ensure_returns_the_path_once_and_never_fetches_a_cached_url_twice() {
        let b = bench();
        let http = FakeHttp::new();
        http.push_for("one.jpg", 200, vec![1, 2, 3]);
        http.push_for("two.jpg", 500, b"boom".to_vec());
        let cache = ImageCache::new(b.store.clone(), b.cache.dir().to_path_buf(), http.clone());

        let urls = vec![
            "https://img/one.jpg".to_string(),
            "https://img/two.jpg".to_string(),
        ];
        let out = cache.ensure(&urls).await;
        assert_eq!(out.len(), 2);
        let one = out[0].1.clone().unwrap();
        assert!(std::path::Path::new(&one).is_file());
        assert!(matches!(
            out[1].1,
            Err(CoreError::Provider {
                status: Some(500),
                ..
            })
        ));
        assert_eq!(http.requests().len(), 2);

        // The cached url comes back off the row; the failed one is asked
        // for again, since nothing was written for it.
        http.push_for("two.jpg", 200, vec![9]);
        let out = cache.ensure(&urls).await;
        assert_eq!(out[0].1.clone().unwrap(), one);
        assert!(out[1].1.is_ok());
        assert_eq!(http.requests().len(), 3);
    }
}
