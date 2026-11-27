//! The usage log through the front door (#1532): calls and pages are counted, the text
//! the log shows is what a send carries, and nothing is sent unless `SendUsage` asks.

mod common;

use anibeam_core::net::{Body, FakeHttp, Method};
use anibeam_core::*;

fn usage(core: &Core) -> UsageLog {
    match core.call(Call::GetUsage).unwrap() {
        Reply::Usage { usage } => usage,
        other => panic!("{other:?}"),
    }
}

#[test]
fn calls_pages_and_errors_are_counted_and_nothing_is_sent() {
    let http = FakeHttp::new();
    let (dir, core, _c) = common::open_core_with_http(http.clone());
    core.call(Call::ListSources).unwrap();
    core.call(Call::NoteUsage {
        page: "library".into(),
    })
    .unwrap();
    assert!(
        core.call(Call::NoteUsage {
            page: "/srv/media/Anime".into()
        })
        .is_err()
    );
    assert!(core.call(Call::RemoveSource { source: 404 }).is_err());
    let log = usage(&core);
    assert!(log.enabled);
    let file: serde_json::Value = serde_json::from_str(&log.text).unwrap();
    let day = file["days"].as_object().unwrap().values().next().unwrap();
    assert_eq!(day["calls"]["ListSources"], 1);
    assert_eq!(day["pages"]["library"], 1);
    assert_eq!(day["errors"]["RemoveSource"]["NotFound"], 1);
    // A page open is not a call
    assert!(day["calls"].get("NoteUsage").is_none());
    assert!(!log.text.contains(dir.path().to_str().unwrap()));
    assert!(http.requests().is_empty());

    let out = dir.path().join("usage-copy.json");
    core.call(Call::ExportUsage {
        path: out.to_string_lossy().into_owned(),
        text: None,
    })
    .unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    // What was shown, and the GetUsage that showed it
    let saved_day = saved["days"].as_object().unwrap().values().next().unwrap();
    assert_eq!(saved_day["calls"]["ListSources"], 1);
    assert_eq!(saved_day["calls"]["GetUsage"], 1);
    assert_eq!(saved_day["pages"], day["pages"]);
    assert!(http.requests().is_empty());
    core.shutdown();
}

#[test]
fn a_send_posts_the_log_once_under_a_fresh_id() {
    let http = FakeHttp::new();
    let (_dir, core, _c) = common::open_core_with_http(http.clone());
    core.call(Call::ListSources).unwrap();
    http.push_for("/usage", 200, "{}");
    http.push_for("/usage", 200, "{}");
    core.call(Call::SendUsage { text: None }).unwrap();
    core.call(Call::SendUsage { text: None }).unwrap();
    let sent: Vec<_> = http.requests();
    assert_eq!(sent.len(), 2);
    let mut ids = Vec::new();
    for req in &sent {
        assert_eq!(req.method, Method::Post);
        assert_eq!(req.url, format!("{DEV_ENDPOINT}/usage"));
        let Some(Body::Json(body)) = &req.body else {
            panic!("{:?}", req.body)
        };
        let id = body["send_id"].as_str().unwrap().to_string();
        assert_eq!(id.len(), 32);
        ids.push(id);
        assert_eq!(body["usage"]["format"], 1);
    }
    assert_ne!(ids[0], ids[1]);
    // The id is never kept: the log's own text has none
    assert!(!usage(&core).text.contains(&ids[0]));

    http.push_for("/usage", 429, "");
    match core.call(Call::SendUsage { text: None }) {
        Err(CoreError::Service { code, .. }) => assert_eq!(code, "http_429"),
        other => panic!("{other:?}"),
    }
    core.shutdown();
}

#[test]
fn switched_off_it_stops_counting_and_clear_empties_it() {
    let (dir, core, _c) = common::open_core();
    core.call(Call::ListSources).unwrap();
    let Reply::Preferences { preferences } = core.call(Call::GetPreferences).unwrap() else {
        panic!()
    };
    core.call(Call::SetPreferences {
        preferences: Preferences {
            usage_log: false,
            ..preferences
        },
    })
    .unwrap();
    let before = usage(&core);
    assert!(!before.enabled);
    core.call(Call::ListSources).unwrap();
    core.call(Call::ListSources).unwrap();
    let after = usage(&core);
    assert_eq!(after.events, before.events);
    let file = dir.path().join("data").join("usage.json");
    assert!(file.exists());
    core.call(Call::ClearUsage).unwrap();
    assert!(!file.exists());
    assert_eq!(usage(&core).events, 0);
    core.shutdown();
}

#[test]
fn a_clear_with_the_log_on_leaves_it_empty() {
    let (dir, core, _c) = common::open_core();
    core.call(Call::ListSources).unwrap();
    let file = dir.path().join("data").join("usage.json");
    assert!(usage(&core).events > 0);
    assert!(file.exists());
    core.call(Call::ClearUsage).unwrap();
    let after = usage(&core);
    assert!(after.enabled);
    assert_eq!(after.events, 0);
    assert_eq!(after.days, 0);
    assert!(!file.exists());
    core.shutdown();
}

#[test]
fn a_count_reaches_the_file_from_the_runtime() {
    let (dir, core, _c) = common::open_core();
    let file = dir.path().join("data").join("usage.json");
    assert!(!file.exists());
    core.call(Call::ListSources).unwrap();
    // Nothing showed or saved the log: the save the count asked for ran on its own
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !file.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    let day = saved["days"].as_object().unwrap().values().next().unwrap();
    assert_eq!(day["calls"]["ListSources"], 1);
    core.shutdown();
}

#[test]
fn the_log_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let paths = CorePaths::under(dir.path());
    let open = || {
        let core = Core::open_with_secrets(
            paths.clone(),
            anibeam_core::trackers::Secrets::file_only(paths.secrets_path()),
        )
        .unwrap();
        core.use_own_qbittorrent(None);
        core
    };
    let core = open();
    for _ in 0..3 {
        core.call(Call::ListSources).unwrap();
    }
    core.shutdown();
    let core = open();
    let log = usage(&core);
    let file: serde_json::Value = serde_json::from_str(&log.text).unwrap();
    let day = file["days"].as_object().unwrap().values().next().unwrap();
    assert_eq!(day["calls"]["ListSources"], 3);
    core.shutdown();
}

#[test]
fn a_send_carries_the_text_the_preview_showed_and_refuses_anything_else() {
    let http = FakeHttp::new();
    let (_dir, core, _c) = common::open_core_with_http(http.clone());
    core.call(Call::ListSources).unwrap();
    let shown = usage(&core).text;
    // More happens between the preview and the press
    for _ in 0..5 {
        core.call(Call::ListSources).unwrap();
    }
    http.push_for("/usage", 200, "{}");
    core.call(Call::SendUsage {
        text: Some(shown.clone()),
    })
    .unwrap();
    let sent = http.requests();
    let Some(Body::Json(body)) = &sent[0].body else {
        panic!("{:?}", sent[0].body)
    };
    let shown: serde_json::Value = serde_json::from_str(&shown).unwrap();
    assert_eq!(body["usage"], shown);

    // Text that is not a usage log, or carries more than one, never leaves
    for text in [
        "not json".to_string(),
        r#"{"format":1,"app_version":"x","platform":"desktop","os":"linux","days":{},"path":"/srv/media"}"#.to_string(),
        r#"{"format":9,"app_version":"x","platform":"desktop","os":"linux","days":{}}"#.to_string(),
    ] {
        assert!(core.call(Call::SendUsage { text: Some(text) }).is_err());
    }
    assert_eq!(http.requests().len(), 1);
    core.shutdown();
}
