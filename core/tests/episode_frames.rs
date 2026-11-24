//! An episode with no still gets a frame from its own file (#1469): the
//! episode a Continue card shows is framed off the read that finds it bare,
//! the series is announced again, and the next read hands the frame out.

mod common;
mod fixtures;
use anibeam_core::*;
use std::process::{Command, Stdio};
use std::time::Duration;

/// A short colourful clip at `path`, or false without ffmpeg.
fn clip(path: &std::path::Path) -> bool {
    Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"])
        .args(["-f", "lavfi", "-i", "testsrc2=s=320x180:r=25:d=10"])
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p"])
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .is_ok_and(|o| o.status.success())
}

fn detail(core: &Core, series: u64) -> SeriesDetail {
    match core.call(Call::GetSeries { series }).unwrap() {
        Reply::SeriesDetail { detail } => *detail,
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_next_episode_with_no_still_gets_a_frame_from_its_own_file() {
    let (dir, core, c) = common::open_core();
    let lib = dir.path().join("lib");
    let show = lib.join("Show");
    std::fs::create_dir_all(&show).unwrap();
    let next = show.join("02.mkv");
    if !clip(&next) {
        eprintln!("skipped: no ffmpeg");
        return;
    }
    let now = anibeam_core::time::now_secs();
    let src = fixtures::insert_source(&core, &lib.to_string_lossy());
    let series = fixtures::insert_series(
        &core,
        src,
        SeriesKind::Show,
        &show.to_string_lossy(),
        "Show",
    );
    fixtures::insert_episode_file(
        &core,
        series,
        &show.join("01.mkv").to_string_lossy(),
        1.0,
        now,
    );
    let file = fixtures::insert_episode_file(&core, series, &next.to_string_lossy(), 2.0, now);
    fixtures::insert_completed(&core, series, "1", now - 10);
    fixtures::insert_view(&core, series, "1", now - 10);

    let first = detail(&core, series);
    assert_eq!(first.next_up, Some(file));
    assert!(first.episodes.iter().all(|e| e.frame.is_none()));

    common::wait_for(
        &c,
        |e| matches!(&e.body, EventBody::SeriesChanged { series: cards } if cards.iter().any(|s| s.id == series)),
        Duration::from_secs(60),
    );
    let again = detail(&core, series);
    let ep = again.episodes.iter().find(|e| e.file == file).unwrap();
    let frame = ep.frame.as_deref().expect("a frame");
    assert!(frame.ends_with(".jpg"), "{frame}");
    assert_eq!(&std::fs::read(frame).unwrap()[..2], &[0xff, 0xd8]);
    // Only the episode Continue shows is framed
    assert!(again.episodes.iter().filter(|e| e.frame.is_some()).count() == 1);
}

#[test]
fn a_series_never_started_takes_no_frame() {
    let (dir, core, _c) = common::open_core();
    let lib = dir.path().join("lib");
    let show = lib.join("Show");
    std::fs::create_dir_all(&show).unwrap();
    let first = show.join("01.mkv");
    if !clip(&first) {
        eprintln!("skipped: no ffmpeg");
        return;
    }
    let now = anibeam_core::time::now_secs();
    let src = fixtures::insert_source(&core, &lib.to_string_lossy());
    let series = fixtures::insert_series(
        &core,
        src,
        SeriesKind::Show,
        &show.to_string_lossy(),
        "Show",
    );
    fixtures::insert_episode_file(&core, series, &first.to_string_lossy(), 1.0, now);
    let d = detail(&core, series);
    assert!(d.episodes.iter().all(|e| e.frame.is_none()));
    let frames = std::path::Path::new(&dir.path().join("cache")).join("frames");
    assert!(!frames.exists() || std::fs::read_dir(&frames).unwrap().next().is_none());
}
