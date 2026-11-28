//! Dev QML file watcher: when `ANIBEAM_QML_DIR` is active, watches `.qml` and `.js` files
//! under that directory and notifies `Shell.qmlChanged` so `Main.qml` can clear the engine's
//! component cache and rebuild `Frame.qml` in place without restarting the process.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use notify_debouncer_full::notify::event::{AccessKind, AccessMode, ModifyKind};
use notify_debouncer_full::notify::{EventKind, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, new_debouncer};

static DEV_QML_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Records the validated `ANIBEAM_QML_DIR` so `Shell::initialize` can watch it.
#[cfg(any(debug_assertions, feature = "dev-qml"))]
pub fn install_dir(dir: PathBuf) {
    let abs = std::fs::canonicalize(&dir).unwrap_or(dir);
    let _ = DEV_QML_DIR.set(abs);
}

/// The active dev QML directory, if `ANIBEAM_QML_DIR` was installed.
pub fn dir() -> Option<&'static Path> {
    DEV_QML_DIR.get().map(PathBuf::as_path)
}

/// True when `path` is a non-hidden `.qml` or `.js` source file.
pub fn is_qml_source(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(OsStr::to_str) else {
        return false;
    };
    if name.starts_with('.') {
        return false;
    }
    matches!(
        path.extension().and_then(OsStr::to_str),
        Some(ext) if ext.eq_ignore_ascii_case("qml") || ext.eq_ignore_ascii_case("js")
    )
}

/// True when `path` is `Main.qml` directly under `dir`.
pub fn is_main_qml(dir: &Path, path: &Path) -> bool {
    path.strip_prefix(dir)
        .ok()
        .is_some_and(|rel| rel == Path::new("Main.qml"))
}

/// Live watching is on whenever `ANIBEAM_QML_DIR` is active on an interactive run, unless
/// `ANIBEAM_QML_WATCH=0` turns it off.
pub fn should_watch(is_shoot: bool, is_drive: bool, env_watch: Option<&OsStr>) -> bool {
    if is_shoot || is_drive {
        return false;
    }
    env_watch != Some(OsStr::new("0"))
}

/// True only for events that actually change a file on disk — never reads/opens/metadata
/// touches, which Qt itself triggers while loading QML.
pub fn is_write_event(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_)
            | EventKind::Remove(_)
            | EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Name(_) | ModifyKind::Any)
            | EventKind::Access(AccessKind::Close(AccessMode::Write))
    )
}

/// Extracts whether a debounced batch contains any `.qml`/`.js` edits and whether `Main.qml`
/// itself was touched.
fn classify_batch(dir: &Path, result: &DebounceEventResult) -> (bool, bool) {
    let Ok(events) = result else {
        return (false, false);
    };
    let mut any_qml = false;
    let mut main_touched = false;
    for ev in events {
        if !is_write_event(&ev.kind) {
            continue;
        }
        for p in &ev.paths {
            if is_qml_source(p) {
                any_qml = true;
                if is_main_qml(dir, p) {
                    main_touched = true;
                }
            }
        }
    }
    (any_qml, main_touched)
}

/// Starts watching `ANIBEAM_QML_DIR` on a background task and emits `Shell.qmlChanged` on the
/// Qt thread whenever a `.qml` or `.js` file changes.
pub fn start(shell: cxx_qt::CxxQtThread<crate::bridge::shell::qobject::Shell>) {
    let Some(dir) = dir() else {
        return;
    };
    let args = crate::runtime::args();
    if !should_watch(
        args.shoot.is_some(),
        args.drive.is_some(),
        std::env::var_os("ANIBEAM_QML_WATCH").as_deref(),
    ) {
        return;
    }
    let dir = dir.to_path_buf();
    crate::runtime::runtime().spawn(async move {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<bool>();
        let watch_dir = dir.clone();
        let mut debouncer = match new_debouncer(
            Duration::from_millis(150),
            None,
            move |res: DebounceEventResult| {
                let (any_qml, main_touched) = classify_batch(&watch_dir, &res);
                if any_qml {
                    let _ = tx.send(main_touched);
                }
            },
        ) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("anibeam: QML watcher could not start: {e}");
                return;
            }
        };
        if let Err(e) = debouncer.watch(&dir, RecursiveMode::Recursive) {
            eprintln!("anibeam: QML watcher could not watch {}: {e}", dir.display());
            return;
        }
        eprintln!("anibeam: watching {} for QML changes", dir.display());
        while let Some(main_touched) = rx.recv().await {
            if main_touched {
                eprintln!(
                    "anibeam: Main.qml changed (restart required for Window-level edits); reloading frame"
                );
            } else {
                eprintln!("anibeam: QML changed, reloading");
            }
            if shell.queue(|s| s.qml_changed()).is_err() {
                break;
            }
        }
        drop(debouncer);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_qml_and_js_sources_only() {
        assert!(is_qml_source(Path::new("/tmp/qml/pages/HomePage.qml")));
        assert!(is_qml_source(Path::new("/tmp/qml/ui/helper.js")));
        assert!(!is_qml_source(Path::new(
            "/tmp/qml/pages/.HomePage.qml.swp"
        )));
        assert!(!is_qml_source(Path::new("/tmp/qml/pages/HomePage.qml~")));
        assert!(!is_qml_source(Path::new("/tmp/qml/pages/HomePage.qml.tmp")));
        assert!(!is_qml_source(Path::new("/tmp/qml/README.md")));
        assert!(!is_qml_source(Path::new("/tmp/qml/.hidden.qml")));
    }

    #[test]
    fn detects_main_qml_only_at_root() {
        let dir = Path::new("/tmp/qml");
        assert!(is_main_qml(dir, Path::new("/tmp/qml/Main.qml")));
        assert!(!is_main_qml(dir, Path::new("/tmp/qml/shell/Main.qml")));
        assert!(!is_main_qml(dir, Path::new("/tmp/qml/shell/Frame.qml")));
    }

    #[test]
    fn skips_watching_for_captures_or_opt_out() {
        assert!(should_watch(false, false, None));
        assert!(should_watch(false, false, Some(OsStr::new("1"))));
        assert!(!should_watch(false, false, Some(OsStr::new("0"))));
        assert!(!should_watch(true, false, None));
        assert!(!should_watch(false, true, None));
    }

    #[test]
    fn ignores_read_and_metadata_events() {
        use notify_debouncer_full::notify::event::{
            CreateKind, DataChange, MetadataKind, RemoveKind, RenameMode,
        };

        assert!(is_write_event(&EventKind::Create(CreateKind::File)));
        assert!(is_write_event(&EventKind::Remove(RemoveKind::File)));
        assert!(is_write_event(&EventKind::Modify(ModifyKind::Data(
            DataChange::Content
        ))));
        assert!(is_write_event(&EventKind::Modify(ModifyKind::Name(
            RenameMode::To
        ))));
        assert!(is_write_event(&EventKind::Access(AccessKind::Close(
            AccessMode::Write
        ))));

        assert!(!is_write_event(&EventKind::Access(AccessKind::Open(
            AccessMode::Read
        ))));
        assert!(!is_write_event(&EventKind::Access(AccessKind::Close(
            AccessMode::Read
        ))));
        assert!(!is_write_event(&EventKind::Access(AccessKind::Read)));
        assert!(!is_write_event(&EventKind::Modify(ModifyKind::Metadata(
            MetadataKind::AccessTime
        ))));
    }
}
