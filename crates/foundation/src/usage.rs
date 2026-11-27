//! The usage log (#1532): what this Anibeam was used for, as counts per day, kept on this
//! machine in `usage.json` in the data folder. Each call by its variant name, each page the
//! shell opened by a short fixed id, and each error by its kind under the call that failed.
//! Nothing else: no paths, titles, names, accounts, addresses or install id, so the file can
//! be read, saved or sent as it is. It never leaves the machine on its own; the core's
//! `SendUsage` is the one way out.
//!
//! Only the counts not saved yet live in memory. A save takes the file's lock, reads the
//! file again, adds those counts to it and writes it back, so two Anibeams on one data
//! folder (the window and `anibeam-cli`, say) both count, and a clear in one is not undone
//! by the other's next save. What the log shows is the file plus the counts not saved yet.
//! A save happens at shutdown, before the log is shown, saved or sent, and after a change
//! once the last save is a minute old. The last 90 UTC days are kept. Switched off, nothing
//! more is counted and the file stays as it was until it is cleared.

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::sync::recover;
use anibeam_contract::CoreError;

/// The file's format. A file in another format is started over rather than misread.
pub const FORMAT: u32 = 1;

/// Days kept, today included.
pub const KEEP_DAYS: i64 = 90;

/// The longest a change waits in memory once another change comes along.
const SAVE_EVERY: Duration = Duration::from_secs(60);

/// The longest page id the log takes.
const PAGE_MAX: usize = 40;

/// The file's name in the data folder.
pub const FILE_NAME: &str = "usage.json";

/// The sibling every save and clear locks, in every process, while it reads and writes.
const LOCK_NAME: &str = "usage.json.lock";

/// One day's counts. Maps keep their keys sorted, so the same counts always read the same.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Day {
    #[serde(default)]
    pub calls: BTreeMap<String, u64>,
    #[serde(default)]
    pub pages: BTreeMap<String, u64>,
    /// Call name, then error kind, then how often.
    #[serde(default)]
    pub errors: BTreeMap<String, BTreeMap<String, u64>>,
}

impl Day {
    fn is_empty(&self) -> bool {
        self.calls.is_empty() && self.pages.is_empty() && self.errors.is_empty()
    }

    /// Calls and page opens: what the summary calls events.
    fn events(&self) -> u64 {
        self.calls.values().sum::<u64>() + self.pages.values().sum::<u64>()
    }

    /// Adds `other`'s counts to this day's.
    fn add(&mut self, other: Day) {
        for (call, n) in other.calls {
            *self.calls.entry(call).or_default() += n;
        }
        for (page, n) in other.pages {
            *self.pages.entry(page).or_default() += n;
        }
        for (call, kinds) in other.errors {
            let into = self.errors.entry(call).or_default();
            for (kind, n) in kinds {
                *into.entry(kind).or_default() += n;
            }
        }
    }
}

/// Adds every day of `from` into `into`.
fn add_days(into: &mut BTreeMap<String, Day>, from: BTreeMap<String, Day>) {
    for (date, day) in from {
        into.entry(date).or_default().add(day);
    }
}

/// The file, and the text `SendUsage` sends.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct File {
    pub format: u32,
    pub app_version: String,
    /// `desktop` or `phone`.
    pub platform: String,
    /// `linux`, `macos`, `windows`, `android` or `ios`.
    pub os: String,
    /// By UTC date, `2026-11-27`.
    #[serde(default)]
    pub days: BTreeMap<String, Day>,
}

struct Inner {
    /// Counts not in the file yet.
    pending: BTreeMap<String, Day>,
    enabled: bool,
    last_save: Option<Instant>,
}

pub struct UsageLog {
    path: PathBuf,
    lock_path: PathBuf,
    version: String,
    inner: Mutex<Inner>,
    /// Held by every read and write of the file in this process, so a save that has taken
    /// the counts out of `inner` and not written them yet is never read half done.
    io: Mutex<()>,
}

/// Which kind of Anibeam this is, for the header.
fn platform() -> &'static str {
    if cfg!(any(target_os = "android", target_os = "ios")) {
        "phone"
    } else {
        "desktop"
    }
}

/// Whether `page` is a page id the log takes: lowercase letters, digits and dashes, a
/// letter first, short. A title or a path never is.
pub fn page_ok(page: &str) -> bool {
    let mut chars = page.chars();
    page.len() <= PAGE_MAX
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Days since the Unix epoch on this machine's clock, in UTC.
fn today() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| (d.as_secs() / 86_400) as i64)
}

/// `2026-11-27` for a day number, the proleptic Gregorian calendar's civil date.
pub fn date_of(day: i64) -> String {
    let z = day + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

impl UsageLog {
    /// The log in `data_dir`. Its file is read on every save and show, never kept: a file
    /// that cannot be read, or is in another format, reads as an empty log, and the next
    /// save replaces it.
    pub fn open(data_dir: &Path, version: &str, enabled: bool) -> UsageLog {
        UsageLog {
            path: data_dir.join(FILE_NAME),
            lock_path: data_dir.join(LOCK_NAME),
            version: version.to_string(),
            inner: Mutex::new(Inner {
                pending: BTreeMap::new(),
                enabled,
                last_save: None,
            }),
            io: Mutex::new(()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn enabled(&self) -> bool {
        recover(&self.inner).enabled
    }

    /// Switched off, nothing more is counted; what is counted stays until `clear`.
    pub fn set_enabled(&self, on: bool) {
        recover(&self.inner).enabled = on;
    }

    /// One call by its variant name, and the kind of error it ended in, if any.
    pub fn record_call(&self, call: &'static str, error: Option<&'static str>) {
        self.record_at(today(), |day| {
            *day.calls.entry(call.to_string()).or_default() += 1;
            if let Some(kind) = error {
                *day.errors
                    .entry(call.to_string())
                    .or_default()
                    .entry(kind.to_string())
                    .or_default() += 1;
            }
        });
    }

    /// A page the shell opened. Refused unless it is a page id (`page_ok`).
    pub fn record_page(&self, page: &str) -> Result<(), CoreError> {
        if !page_ok(page) {
            return Err(CoreError::invalid(
                "page",
                anibeam_contract::i18n::tr("core-usage-bad-page", &[]),
            ));
        }
        self.record_at(today(), |day| {
            *day.pages.entry(page.to_string()).or_default() += 1;
        });
        Ok(())
    }

    /// Counts into the day `day`, drops the days past `KEEP_DAYS`, and saves when the last
    /// save is a minute old.
    fn record_at(&self, day: i64, count: impl FnOnce(&mut Day)) {
        let due = {
            let mut inner = recover(&self.inner);
            if !inner.enabled {
                return;
            }
            count(inner.pending.entry(date_of(day)).or_default());
            prune(&mut inner.pending, day);
            inner.last_save.is_none_or(|at| at.elapsed() >= SAVE_EVERY)
        };
        if due {
            self.flush();
        }
    }

    /// The file's lock, held until the returned file is dropped. A filesystem that cannot
    /// lock gets the save anyway, unlocked, rather than losing it.
    fn file_lock(&self) -> Option<std::fs::File> {
        let open = || {
            if let Some(dir) = self.lock_path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let file = OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(&self.lock_path)?;
            file.lock()?;
            Ok::<_, std::io::Error>(file)
        };
        open()
            .inspect_err(|e| tracing::warn!("could not lock the usage log: {e}"))
            .ok()
    }

    /// What the file holds now.
    fn read_file(&self) -> BTreeMap<String, Day> {
        std::fs::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<File>(&bytes).ok())
            .filter(|f| f.format == FORMAT)
            .map(|f| f.days)
            .unwrap_or_default()
    }

    /// Adds the counts not saved yet to the file, under its lock.
    pub fn flush(&self) {
        let _io = recover(&self.io);
        let pending = {
            let mut inner = recover(&self.inner);
            inner.last_save = Some(Instant::now());
            std::mem::take(&mut inner.pending)
        };
        if pending.is_empty() {
            return;
        }
        let _lock = self.file_lock();
        let mut days = self.read_file();
        add_days(&mut days, pending.clone());
        prune(&mut days, today());
        if let Err(e) = crate::atomic_file::write(&self.path, self.text_of(&days)) {
            // The counts go back, and the next save tries again
            tracing::warn!("could not save the usage log: {e}");
            add_days(&mut recover(&self.inner).pending, pending);
        }
    }

    /// The file plus the counts not saved yet, past days dropped.
    fn days_now(&self) -> BTreeMap<String, Day> {
        let _io = recover(&self.io);
        let mut days = self.read_file();
        add_days(&mut days, recover(&self.inner).pending.clone());
        prune(&mut days, today());
        days
    }

    fn text_of(&self, days: &BTreeMap<String, Day>) -> String {
        let file = File {
            format: FORMAT,
            app_version: self.version.clone(),
            platform: platform().to_string(),
            os: std::env::consts::OS.to_string(),
            days: days.clone(),
        };
        serde_json::to_string_pretty(&file).unwrap_or_default()
    }

    /// `text` read back as a usage log and written out again, so only the log's own
    /// fields can leave: anything else, or another format, is refused.
    pub fn checked(text: &str) -> Result<String, String> {
        let file: File = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if file.format != FORMAT {
            return Err(format!("format {}", file.format));
        }
        serde_json::to_string_pretty(&file).map_err(|e| e.to_string())
    }

    /// The log's text, after a save, so what is shown is what is on disk.
    pub fn text(&self) -> String {
        self.flush();
        self.text_of(&self.days_now())
    }

    /// Days with anything in them, and the calls and page opens over all of them.
    pub fn summary(&self) -> (u64, u64) {
        let days = self.days_now();
        let with = days.values().filter(|d| !d.is_empty()).count() as u64;
        let events = days.values().map(Day::events).sum();
        (with, events)
    }

    /// Empties the log: removes its file and the counts not saved yet, under the file's
    /// lock, so no save in another process puts back what was there.
    pub fn clear(&self) -> Result<(), CoreError> {
        let _io = recover(&self.io);
        let _lock = self.file_lock();
        match std::fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(CoreError::io_at(self.path.to_string_lossy(), e)),
        }
        recover(&self.inner).pending.clear();
        Ok(())
    }
}

/// Drops every day more than `KEEP_DAYS` before `today`. ISO dates sort as text.
fn prune(days: &mut BTreeMap<String, Day>, today: i64) {
    let oldest = date_of(today - (KEEP_DAYS - 1));
    days.retain(|date, _| *date >= oldest);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log(dir: &Path) -> UsageLog {
        UsageLog::open(dir, "9.9.9", true)
    }

    #[test]
    fn dates_are_civil_utc_dates() {
        assert_eq!(date_of(0), "1970-01-01");
        assert_eq!(date_of(19_723), "2024-01-01");
        assert_eq!(date_of(19_782), "2024-02-29");
        assert_eq!(date_of(20_784), "2026-11-27");
    }

    #[test]
    fn calls_pages_and_errors_count_per_day() {
        let dir = tempfile::tempdir().unwrap();
        let log = log(dir.path());
        log.record_call("Scan", None);
        log.record_call("Scan", Some("Io"));
        log.record_call("ListSources", None);
        log.record_page("library").unwrap();
        log.record_page("library").unwrap();
        let file: File = serde_json::from_str(&log.text()).unwrap();
        let day = &file.days[&date_of(today())];
        assert_eq!(day.calls["Scan"], 2);
        assert_eq!(day.calls["ListSources"], 1);
        assert_eq!(day.pages["library"], 2);
        assert_eq!(day.errors["Scan"]["Io"], 1);
        assert_eq!(log.summary(), (1, 5));
        assert_eq!(file.format, FORMAT);
        assert_eq!(file.app_version, "9.9.9");
        assert_eq!(file.os, std::env::consts::OS);
    }

    #[test]
    fn a_new_day_gets_its_own_counts() {
        let dir = tempfile::tempdir().unwrap();
        let log = log(dir.path());
        let count = |d: &mut Day| *d.calls.entry("About".into()).or_default() += 1;
        log.record_at(today() - 1, count);
        log.record_at(today(), count);
        log.record_at(today(), count);
        let days = log.days_now();
        assert_eq!(days[&date_of(today() - 1)].calls["About"], 1);
        assert_eq!(days[&date_of(today())].calls["About"], 2);
    }

    #[test]
    fn days_past_ninety_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let log = log(dir.path());
        let count = |d: &mut Day| *d.calls.entry("About".into()).or_default() += 1;
        let now = today();
        log.record_at(now - KEEP_DAYS, count);
        log.record_at(now - (KEEP_DAYS - 1), count);
        log.record_at(now, count);
        let days: Vec<String> = log.days_now().keys().cloned().collect();
        assert_eq!(days, vec![date_of(now - (KEEP_DAYS - 1)), date_of(now)]);
    }

    #[test]
    fn switched_off_counts_nothing_and_keeps_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let log = log(dir.path());
        log.record_call("Scan", None);
        log.flush();
        let before = std::fs::read_to_string(log.path()).unwrap();
        log.set_enabled(false);
        log.record_call("Scan", None);
        log.record_page("library").unwrap();
        log.flush();
        assert_eq!(std::fs::read_to_string(log.path()).unwrap(), before);
        assert_eq!(log.summary(), (1, 1));
        log.clear().unwrap();
        assert!(!log.path().exists());
        assert_eq!(log.summary(), (0, 0));
    }

    #[test]
    fn the_file_is_read_back_and_saved_once_a_minute() {
        let dir = tempfile::tempdir().unwrap();
        {
            let log = log(dir.path());
            // The first change saves at once, the next waits for a minute or a flush
            log.record_call("Scan", None);
            assert!(log.path().exists());
            log.record_call("Scan", None);
            let on_disk: File =
                serde_json::from_slice(&std::fs::read(log.path()).unwrap()).unwrap();
            assert_eq!(on_disk.days[&date_of(today())].calls["Scan"], 1);
            log.flush();
        }
        let again = log(dir.path());
        assert_eq!(again.summary(), (1, 2));
    }

    /// The window and the CLI on one data folder: both count, the file holds the sum, and a
    /// clear in one is not undone by the other's next save.
    #[test]
    fn two_logs_on_one_folder_add_up_and_a_clear_holds() {
        let dir = tempfile::tempdir().unwrap();
        let a = log(dir.path());
        let b = log(dir.path());
        for _ in 0..2 {
            a.record_call("Scan", None);
            b.record_call("Scan", None);
        }
        b.record_call("Lookup", None);
        a.flush();
        b.flush();
        let on_disk: File = serde_json::from_slice(&std::fs::read(a.path()).unwrap()).unwrap();
        let day = &on_disk.days[&date_of(today())];
        assert_eq!(day.calls["Scan"], 4);
        assert_eq!(day.calls["Lookup"], 1);
        assert_eq!(a.summary(), (1, 5));
        assert_eq!(b.summary(), (1, 5));

        a.clear().unwrap();
        b.flush();
        assert!(!a.path().exists());
        assert_eq!(b.summary(), (0, 0));
        b.record_call("About", None);
        b.flush();
        let on_disk: File = serde_json::from_slice(&std::fs::read(a.path()).unwrap()).unwrap();
        let day = &on_disk.days[&date_of(today())];
        assert_eq!(day.calls.len(), 1);
        assert_eq!(day.calls["About"], 1);
        assert_eq!(a.summary(), (1, 1));
    }

    #[test]
    fn a_page_that_is_not_an_id_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let log = log(dir.path());
        for bad in [
            "",
            "/srv/media/Anime",
            "C:\\Users\\someone",
            "Some Show Title",
            "someone@example.com",
            "Library",
            &"a".repeat(PAGE_MAX + 1),
        ] {
            assert!(log.record_page(bad).is_err(), "{bad}");
        }
        assert!(log.record_page("settings-about").is_ok());
    }

    /// Whatever is counted, the text carries no path, address or mail-like string: only
    /// fixed names, dates and numbers.
    #[test]
    fn the_payload_holds_nothing_path_like() {
        let dir = tempfile::tempdir().unwrap();
        let log = log(dir.path());
        log.record_call("AddSource", Some("Io"));
        log.record_call("Lookup", Some("NotFound"));
        let _ = log.record_page("/srv/media/Anime");
        log.record_page("series").unwrap();
        let text = log.text();
        for needle in ["/", "\\", "@", "srv", "media", "Anime"] {
            assert!(!text.contains(needle), "{needle} in {text}");
        }
    }
}
