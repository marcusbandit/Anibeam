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

/// The decision itself, off a real read: Continue, and so the frame, takes
/// only a series Home's `startedOf` would put there. The detail's next up is
/// pinned to its one file, so the started test is all that decides.
#[test]
fn only_a_series_home_puts_in_continue_takes_a_frame() {
    use anibeam_core::library::frames::continue_episode;
    let (dir, core, _c) = common::open_core();
    let lib = dir.path().join("lib");
    let show = lib.join("Show");
    std::fs::create_dir_all(&show).unwrap();
    let now = anibeam_core::time::now_secs();
    let src = fixtures::insert_source(&core, &lib.to_string_lossy());
    let series = fixtures::insert_series(
        &core,
        src,
        SeriesKind::Show,
        &show.to_string_lossy(),
        "Show",
    );
    let file = fixtures::insert_episode_file(
        &core,
        series,
        &show.join("01.mkv").to_string_lossy(),
        1.0,
        now,
    );
    let mut d = detail(&core, series);
    d.next_up = Some(file);
    let with = |status: Option<ListStatus>, viewed: bool, watched: Option<u32>| {
        let mut d = d.clone();
        d.card.list_status = status;
        d.card.last_viewed_at = viewed.then(std::time::SystemTime::now);
        d.card.watched = watched;
        continue_episode(&d)
    };

    // Never started: no view here, nothing seen on the tracker
    assert_eq!(with(None, false, None), None);
    assert_eq!(with(Some(ListStatus::Watching), false, Some(0)), None);
    // Played here
    assert_eq!(with(None, true, None), Some(0));
    assert_eq!(with(Some(ListStatus::Planning), true, None), Some(0));
    // Seen on the tracker counts only while watching or rewatching there
    assert_eq!(with(Some(ListStatus::Watching), false, Some(3)), Some(0));
    assert_eq!(with(Some(ListStatus::Repeating), false, Some(3)), Some(0));
    assert_eq!(with(Some(ListStatus::Planning), false, Some(3)), None);
    assert_eq!(with(None, false, Some(3)), None);
    // Completed, dropped and paused never, even played here
    for status in [
        ListStatus::Completed,
        ListStatus::Dropped,
        ListStatus::Paused,
    ] {
        assert_eq!(with(Some(status), true, Some(3)), None, "{status:?}");
    }
}

/// An episode whose still is known but not fetched yet waits for the still:
/// the read says a still is coming for its file, and only for that one.
#[test]
fn an_episode_with_a_still_on_the_way_is_not_framed() {
    use anibeam_core::library::snapshot::Snapshot;
    let (dir, core, _c) = common::open_core();
    let src = fixtures::insert_source(&core, "/lib");
    let series = fixtures::insert_series(&core, src, SeriesKind::Show, "/lib/Show", "Show");
    let cached = fixtures::insert_episode_file(&core, series, "/lib/Show/01.mkv", 1.0, 1);
    let coming = fixtures::insert_episode_file(&core, series, "/lib/Show/02.mkv", 2.0, 1);
    let bare = fixtures::insert_episode_file(&core, series, "/lib/Show/03.mkv", 3.0, 1);
    fixtures::media(10, "Show").episodes(3).insert(&core);
    fixtures::match_series(&core, series, Some(10), None);
    core.store()
        .write(|c| {
            c.execute_batch(
                "INSERT INTO anizip_shows (anilist_id, found, fetched_at) VALUES (10, 1, 0);
                 INSERT INTO anizip_episodes (anilist_id, key, number, still_url) VALUES
                     (10, '1', 1, 'https://images.example.com/1.jpg'),
                     (10, '2', 2, 'https://images.example.com/2.jpg'),
                     (10, '3', 3, NULL);
                 INSERT INTO images (url, path, bytes, fetched_at, used_at) VALUES
                     ('https://images.example.com/1.jpg', 'aa/1.jpg', 4, 0, 0);",
            )?;
            Ok(())
        })
        .unwrap();
    let images = dir.path().join("cache").join("images");
    let (detail, stills) = core
        .store()
        .read(|c| {
            let snap = Snapshot::load(c, &images, std::time::SystemTime::now(), Some(&[series]))?;
            snap.detail_and_stills(c, series)
        })
        .unwrap()
        .unwrap();
    let still = |file: u64| {
        detail
            .episodes
            .iter()
            .find(|e| e.file == file)
            .unwrap()
            .still
            .is_some()
    };
    assert!(still(cached));
    assert!(!still(coming) && !still(bare));
    assert_eq!(stills, std::collections::HashSet::from([coming]));
}
