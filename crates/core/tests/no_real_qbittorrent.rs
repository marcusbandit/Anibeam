//! No test ever runs the real qbittorrent-nox (#1489). Anibeam's own torrent client is
//! started in a process group of its own and keeps running after the app quits, so one a
//! test found on the PATH stayed behind after every run. Every core and downloads team a
//! test opens pins the binary with `use_own_qbittorrent`: none, or a fake for the tests
//! of the start.

mod common;

use std::path::{Path, PathBuf};

/// The calls that build a core, or the downloads team on its own.
const OPENS: [&str; 4] = [
    "Core::open(",
    "Core::open_with_secrets(",
    "Core::open_with_http_and_secrets(",
    "Downloads::new(",
];
/// How far below the open the pin may come, for an open written over several lines.
const PIN_WITHIN: usize = 12;

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            // Build output, and the examples, which drive a root as the app does
            if name != "target" && name != "examples" && !name.starts_with('.') {
                rust_files(&path, out);
            }
        } else if name.ends_with(".rs") {
            out.push(path);
        }
    }
}

/// The line a file's test code starts on: all of an integration test or a test harness,
/// else whatever follows its first `#[cfg(test)]`.
fn tests_from(path: &Path, lines: &[&str]) -> Option<usize> {
    let harness = path.components().any(|c| c.as_os_str() == "tests")
        || path.file_name().is_some_and(|n| n == "testing.rs");
    if harness {
        return Some(0);
    }
    lines.iter().position(|l| l.trim() == "#[cfg(test)]")
}

#[test]
fn every_core_a_test_opens_pins_its_qbittorrent() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for top in ["crates", "apps"] {
        rust_files(&root.join(top), &mut files);
    }
    assert!(files.len() > 50, "found only {} files", files.len());
    let mut unpinned = Vec::new();
    for file in &files {
        // This file names the opens it looks for
        if file.ends_with(file!().rsplit('/').next().unwrap()) {
            continue;
        }
        let text = std::fs::read_to_string(file).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        let Some(start) = tests_from(file, &lines) else {
            continue;
        };
        for (i, line) in lines.iter().enumerate().skip(start) {
            let code = line.trim_start();
            if code.starts_with("//") || !OPENS.iter().any(|o| code.contains(o)) {
                continue;
            }
            let below = &lines[i..lines.len().min(i + PIN_WITHIN)];
            if !below.iter().any(|l| l.contains("use_own_qbittorrent(")) {
                let shown = file.strip_prefix(&root).unwrap_or(file);
                unpinned.push(format!("{}:{}", shown.display(), i + 1));
            }
        }
    }
    assert!(
        unpinned.is_empty(),
        "these test cores can start the real qbittorrent-nox, which outlives the test; \
         call use_own_qbittorrent(None) on each: {unpinned:#?}"
    );
}

/// A fresh install with nothing answering and a qbittorrent-nox on the PATH: a test's core
/// still never starts it.
#[cfg(unix)]
#[test]
fn a_tests_core_never_starts_the_qbittorrent_on_the_path() {
    use std::os::unix::fs::PermissionsExt;

    use anibeam_core::net::FakeHttp;
    use anibeam_core::{Call, Reply, TorrentMode};

    let bin = tempfile::tempdir().unwrap();
    let ran = bin.path().join("ran");
    let nox = bin.path().join("qbittorrent-nox");
    std::fs::write(
        &nox,
        format!("#!/bin/sh\ntouch '{}'\nexec sleep 5\n", ran.display()),
    )
    .unwrap();
    std::fs::set_permissions(&nox, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = std::env::var("PATH").unwrap_or_default();
    // This binary's only test that reads PATH, so nothing else sees the change
    unsafe { std::env::set_var("PATH", format!("{}:{path}", bin.path().display())) };

    let http = FakeHttp::new();
    let (_dir, core, c) = common::open_core_with_http(http);
    let job = common::started(&core, Call::TestTorrentClient);
    common::wait_job(&c, job);
    let status = match core.call(Call::GetDownloads).unwrap() {
        Reply::Downloads { status } => status,
        other => panic!("{other:?}"),
    };
    core.shutdown();
    unsafe { std::env::set_var("PATH", path) };

    assert_ne!(status.settings.torrent_mode, TorrentMode::Managed);
    assert!(!status.torrent.connected);
    // A start would have held the job for its 30 s wait on the Web UI, long after the
    // script ran
    assert!(!ran.exists(), "the qbittorrent-nox on the PATH was started");
}
