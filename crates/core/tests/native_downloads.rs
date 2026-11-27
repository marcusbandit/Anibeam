//! Subscribing, adding torrents and removing subscriptions on the core's
//! own engine (#119), against `FakeHttp`: every rule, feed and torrent goes
//! to qBittorrent directly, in anirss's own shape, and anirss is never
//! asked. anirss is set up as a remote server, so a request to it would
//! show in `FakeHttp`.
#![cfg(unix)]

mod common;
use common::started;

use std::sync::Arc;
use std::time::Duration;

use anibeam_core::events::Collector;
use anibeam_core::net::{Body, FakeHttp, HttpRequest};
use anibeam_core::*;
use serde_json::{Value, json};

const QBT: &str = "http://localhost:9090";
const FEED: &str = "https://feed.example/rss?q={query}";
const SAVE_BASE: &str = "/srv/TV";

fn settings(torrent_url: &str) -> DownloadSettings {
    DownloadSettings {
        anirss_mode: AnirssMode::Remote,
        anirss_command: "anirss".to_string(),
        jackett_command: String::new(),
        anirss_url: "http://anirss.test".to_string(),
        has_anirss_token: false,
        torrent_client: TorrentClient::Qbittorrent,
        torrent_mode: TorrentMode::Existing,
        torrent_url: torrent_url.to_string(),
        torrent_username: "me".to_string(),
        has_torrent_password: false,
        saved_clients: Vec::new(),
        active_client: String::new(),
        prune_finished_feeds: true,
        rolling: Default::default(),
        engine: DownloadsEngine::Native,
    }
}

struct Rig {
    dir: common::Dir,
    core: Arc<Core>,
    c: Arc<Collector>,
    http: Arc<FakeHttp>,
}

/// The core's engine on one endpoint added by hand, its torrent client at
/// `torrent_url` with `save_base` as its default save folder.
fn rig_at(torrent_url: &str, save_base: &str) -> Rig {
    let http = FakeHttp::new();
    let (dir, core, c) = common::open_core_with_http(http.clone());
    let reply = core
        .call(Call::SetDownloadSettings {
            settings: settings(torrent_url),
            anirss_token: Some("tok".to_string()),
            torrent_password: Some("pw".to_string()),
        })
        .unwrap();
    assert_eq!(reply, Reply::Ok);
    core.call(Call::AddCustomEndpoint {
        name: "feed".into(),
        url: FEED.into(),
        credentials: vec![],
    })
    .unwrap();
    http.push_for("app/defaultSavePath", 200, save_base.to_string());
    Rig { dir, core, c, http }
}

fn rig() -> Rig {
    rig_at(QBT, SAVE_BASE)
}

fn release(title: &str, link: &str) -> Release {
    serde_json::from_value(json!({"title": title, "link": link, "source": "feed"})).unwrap()
}

fn episode(n: u32) -> Release {
    release(
        &format!("[Group] Show - {n:02} (1080p)"),
        &format!("magnet:?xt=urn:btih:{n:040x}"),
    )
}

fn form(r: &HttpRequest) -> Vec<(String, String)> {
    match &r.body {
        Some(Body::Form(pairs)) => pairs.clone(),
        other => panic!("not a form: {other:?}"),
    }
}

fn field(r: &HttpRequest, key: &str) -> Option<String> {
    form(r).into_iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

/// The requests to the torrent client's API, by the path after `api/v2/`.
fn qbt_calls(http: &FakeHttp) -> Vec<(String, HttpRequest)> {
    http.requests()
        .into_iter()
        .filter_map(|r| {
            let path = r.url.strip_prefix(&format!("{QBT}/api/v2/"))?.to_string();
            Some((path, r))
        })
        .collect()
}

fn paths_of(http: &FakeHttp) -> Vec<String> {
    qbt_calls(http).into_iter().map(|(p, _)| p).collect()
}

fn called<'a>(calls: &'a [(String, HttpRequest)], path: &str) -> &'a HttpRequest {
    &calls
        .iter()
        .find(|(p, _)| p == path)
        .unwrap_or_else(|| panic!("no {path} in {calls:?}"))
        .1
}

/// anirss was never asked anything.
fn no_anirss(http: &FakeHttp) {
    let sent: Vec<String> = http.requests().into_iter().map(|r| r.url).collect();
    assert!(
        !sent.iter().any(|u| u.contains("anirss.test")),
        "anirss was asked: {sent:?}"
    );
}

fn failed(event: &Event) -> &CoreError {
    match &event.body {
        EventBody::JobFailed { error } => error,
        other => panic!("expected a failure, got {other:?}"),
    }
}

fn subscribe(r: &Rig, query: &str, items: Vec<Release>, name: Option<&str>) -> Event {
    let job = started(
        &r.core,
        Call::Subscribe {
            query: query.to_string(),
            endpoint: "feed".to_string(),
            items,
            name: name.map(str::to_string),
            wire_query: None,
            series: None,
        },
    );
    common::wait_job(&r.c, job)
}

fn subscribed(event: &Event) -> &Subscribed {
    match &event.body {
        EventBody::Subscribed { result } => result,
        other => panic!("{other:?}"),
    }
}

// Subscribing ----------------------------------------------------------------------

#[test]
fn subscribe_writes_the_rule_and_feed_through_qbittorrent() {
    let r = rig();
    r.http.push_for("rss/rules", 200, "{}");
    r.http.push_for("rss/addFeed", 200, "");
    r.http.push_for("rss/setRule", 200, "");

    let done = subscribe(&r, "Show -HEVC", vec![episode(1)], None);
    let feed_url = "https://feed.example/rss?q=Show";
    assert_eq!(
        subscribed(&done),
        &Subscribed {
            name: "Show".to_string(),
            // The event names no feed (#478); the requests below show the rule reads it.
            feed_url: String::new(),
            save_path: "/srv/TV/Show".to_string(),
            must_contain: String::new(),
            must_not_contain: "HEVC".to_string(),
            tags: vec!["Anime".to_string()],
        }
    );

    let calls = qbt_calls(&r.http);
    let add = called(&calls, "rss/addFeed");
    assert_eq!(
        form(add),
        vec![
            ("url".to_string(), feed_url.to_string()),
            ("path".to_string(), "Show".to_string()),
        ]
    );
    let set = called(&calls, "rss/setRule");
    assert_eq!(field(set, "ruleName").as_deref(), Some("Show"));
    let def: Value = serde_json::from_str(&field(set, "ruleDef").unwrap()).unwrap();
    assert_eq!(
        def,
        json!({
            "enabled": true, "mustContain": "", "mustNotContain": "HEVC",
            "useRegex": false, "episodeFilter": "", "smartFilter": false,
            "previouslyMatchedEpisodes": [], "affectedFeeds": [feed_url],
            "ignoreDays": 0, "lastMatch": "", "addPaused": null, "assignedCategory": "",
            "savePath": "/srv/TV/Show",
            "torrentParams": {"save_path": "/srv/TV/Show", "use_auto_tmm": false, "tags": ["Anime"]}
        })
    );
    // The feed goes in before the rule that reads it, as anirss does it
    let order = paths_of(&r.http);
    let feed_at = order.iter().position(|p| p == "rss/addFeed").unwrap();
    let rule_at = order.iter().position(|p| p == "rss/setRule").unwrap();
    assert!(feed_at < rule_at, "{order:?}");
    no_anirss(&r.http);
}

/// A subscription made from a library show's page saves into that show's
/// folder, wherever its source is, not into a second one under the save
/// base (#267).
#[test]
fn a_subscription_for_a_library_show_saves_into_its_folder() {
    let r = rig();
    let home = common::library_show(r.dir.path(), &r.core, &r.c);
    r.http.push_for("rss/rules", 200, "{}");
    r.http.push_for("rss/addFeed", 200, "");
    r.http.push_for("rss/setRule", 200, "");

    let job = started(
        &r.core,
        Call::Subscribe {
            query: "Show".to_string(),
            endpoint: "feed".to_string(),
            items: vec![episode(1)],
            name: None,
            wire_query: None,
            series: Some(1),
        },
    );
    let done = common::wait_job(&r.c, job);
    assert_eq!(subscribed(&done).save_path, home);
    let calls = qbt_calls(&r.http);
    let def: Value =
        serde_json::from_str(&field(called(&calls, "rss/setRule"), "ruleDef").unwrap()).unwrap();
    assert_eq!(def["savePath"], json!(home));
    assert_eq!(def["torrentParams"]["save_path"], json!(home));
    no_anirss(&r.http);
}

/// A rule that saves into a library show's folder never takes its files
/// along when it goes: the folder holds episodes it did not download.
#[test]
fn removing_a_subscription_with_files_leaves_a_library_show_folder() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("lib").to_string_lossy().into_owned();
    let r = rig_at(QBT, &base);
    // The show sits inside the save base, where the old checks let its folder go.
    let home = common::library_show(dir.path(), &r.core, &r.c);
    queue_one_rule(&r.http, &home);
    let done = remove_call(&r, false, true, true);
    assert!(
        matches!(&done, Err(CoreError::Invalid { field, message })
            if field == "files" && message.contains("library folder")),
        "{done:?}"
    );
    assert!(writes(&r.http).is_empty());
    assert!(std::path::Path::new(&home).is_dir());
}

#[test]
fn subscribing_the_same_show_from_another_endpoint_suffixes_the_name() {
    let r = rig();
    r.http.push_for(
        "rss/rules",
        200,
        json!({"Show": {"affectedFeeds": ["https://other.example/rss?q=Show"]}}).to_string(),
    );
    r.http.push_for("rss/addFeed", 200, "");
    r.http.push_for("rss/setRule", 200, "");

    let done = subscribe(&r, "Show", vec![episode(1)], None);
    let result = subscribed(&done);
    assert_eq!(result.name, "Show @feed");
    assert_eq!(result.save_path, "/srv/TV/Show @feed");
    let calls = qbt_calls(&r.http);
    assert_eq!(
        field(called(&calls, "rss/addFeed"), "path").as_deref(),
        Some("Show @feed")
    );
    assert_eq!(
        field(called(&calls, "rss/setRule"), "ruleName").as_deref(),
        Some("Show @feed")
    );
    no_anirss(&r.http);
}

#[test]
fn subscribing_the_same_feed_again_keeps_the_name() {
    let r = rig();
    r.http.push_for(
        "rss/rules",
        200,
        json!({"Show": {"affectedFeeds": ["https://feed.example/rss?q=Show"]}}).to_string(),
    );
    // qBittorrent already has the feed: no failure
    r.http.push_for("rss/addFeed", 409, "Conflict");
    r.http.push_for(
        "rss/items",
        200,
        json!({"Show": {"uid": "u", "url": "https://feed.example/rss?q=Show"}}).to_string(),
    );
    r.http.push_for("rss/setRule", 200, "");

    let done = subscribe(&r, "Show", vec![episode(1)], None);
    assert_eq!(subscribed(&done).name, "Show");
    let calls = qbt_calls(&r.http);
    assert_eq!(
        field(called(&calls, "rss/setRule"), "ruleName").as_deref(),
        Some("Show")
    );
}

/// A library show followed again from another feed (another endpoint, group or quality)
/// had two rules filling its folder, and Following drew it twice (#1486). Now the new rule
/// replaces the old one: the old rule and its feed go, its episodes stay.
#[test]
fn following_a_library_show_again_replaces_its_rule() {
    let r = rig();
    let home = common::library_show(r.dir.path(), &r.core, &r.c);
    let old = json!({"affectedFeeds": ["https://other.example/rss?q=Show"], "savePath": home});
    let new = json!({"affectedFeeds": ["https://feed.example/rss?q=Show"], "savePath": home});
    // The engine names the new rule beside the old one
    r.http
        .push_for("rss/rules", 200, json!({ "Show": old }).to_string());
    r.http.push_for("rss/addFeed", 200, "");
    r.http.push_for("rss/setRule", 200, "");
    // Then reads both to find what the new one replaces
    r.http.push_for(
        "rss/rules",
        200,
        json!({ "Show": old, "Show @feed": new }).to_string(),
    );
    r.http.push_for(
        "rss/items",
        200,
        json!({
            "Show": {"uid": "a", "url": "https://other.example/rss?q=Show"},
            "Show @feed": {"uid": "b", "url": "https://feed.example/rss?q=Show"},
        })
        .to_string(),
    );
    r.http.push_for("rss/removeRule", 200, "");
    r.http.push_for("rss/removeItem", 200, "");

    let job = started(
        &r.core,
        Call::Subscribe {
            query: "Show".to_string(),
            endpoint: "feed".to_string(),
            items: vec![episode(1)],
            name: None,
            wire_query: None,
            series: Some(1),
        },
    );
    let done = common::wait_job(&r.c, job);
    let result = subscribed(&done);
    assert_eq!(result.name, "Show @feed");
    assert_eq!(result.save_path, home);
    let calls = qbt_calls(&r.http);
    assert_eq!(
        field(called(&calls, "rss/removeRule"), "ruleName").as_deref(),
        Some("Show")
    );
    assert_eq!(
        field(called(&calls, "rss/removeItem"), "path").as_deref(),
        Some("Show")
    );
    // Nothing else is taken: no torrent and no file goes with the old rule
    assert!(
        !paths_of(&r.http)
            .iter()
            .any(|p| p.starts_with("torrents/delete"))
    );
    assert!(std::path::Path::new(&home).is_dir());
    let removed: Vec<String> = calls
        .iter()
        .filter(|(p, _)| p == "rss/removeRule")
        .filter_map(|(_, req)| field(req, "ruleName"))
        .collect();
    assert_eq!(removed, vec!["Show".to_string()]);
    no_anirss(&r.http);
}

/// Followed again from a search, with no library show to save into, the new rule moves
/// into the folder the old one filled before the old one goes, so the show keeps one
/// folder as well as one rule (#1486).
#[test]
fn following_a_show_again_from_a_search_keeps_its_folder() {
    let r = rig();
    let old =
        json!({"affectedFeeds": ["https://other.example/rss?q=Show"], "savePath": "/srv/TV/Show"});
    let new = json!({"affectedFeeds": ["https://feed.example/rss?q=Show"], "savePath": "/srv/TV/Show @feed"});
    r.http
        .push_for("rss/rules", 200, json!({ "Show": old }).to_string());
    r.http.push_for("rss/addFeed", 200, "");
    r.http.push_for("rss/setRule", 200, "");
    r.http.push_for(
        "rss/rules",
        200,
        json!({ "Show": old, "Show @feed": new }).to_string(),
    );
    r.http.push_for("rss/setRule", 200, "");
    // qBittorrent lists no feed at the old address, so no feed goes
    r.http.push_for("rss/items", 200, "{}");
    r.http.push_for("rss/removeRule", 200, "");

    let done = subscribe(&r, "Show", vec![episode(1)], None);
    let result = subscribed(&done);
    assert_eq!(result.name, "Show @feed");
    assert_eq!(result.save_path, "/srv/TV/Show");
    let calls = qbt_calls(&r.http);
    let sets: Vec<&HttpRequest> = calls
        .iter()
        .filter(|(p, _)| p == "rss/setRule")
        .map(|(_, req)| req)
        .collect();
    assert_eq!(sets.len(), 2, "{calls:?}");
    assert_eq!(field(sets[1], "ruleName").as_deref(), Some("Show @feed"));
    let def: Value = serde_json::from_str(&field(sets[1], "ruleDef").unwrap()).unwrap();
    assert_eq!(def["savePath"], json!("/srv/TV/Show"));
    assert_eq!(
        field(called(&calls, "rss/removeRule"), "ruleName").as_deref(),
        Some("Show")
    );
    no_anirss(&r.http);
}

/// Followed again from the feed its plain-named rule reads, the rule keeps its name and its
/// folder, and only the `@endpoint` rule beside it goes: its folder is not the show's
/// (#1486).
#[test]
fn following_again_from_the_same_feed_keeps_the_rules_folder() {
    let r = rig();
    let plain =
        json!({"affectedFeeds": ["https://feed.example/rss?q=Show"], "savePath": "/srv/TV/Show"});
    let other = json!({"affectedFeeds": ["https://other.example/rss?q=Show"], "savePath": "/srv/TV/Show @other"});
    let both = json!({ "Show": plain, "Show @other": other }).to_string();
    r.http.push_for("rss/rules", 200, both.clone());
    r.http.push_for("rss/addFeed", 409, "Conflict");
    r.http.push_for(
        "rss/items",
        200,
        json!({"Show": {"uid": "u", "url": "https://feed.example/rss?q=Show"}}).to_string(),
    );
    r.http.push_for("rss/setRule", 200, "");
    r.http.push_for("rss/rules", 200, both);
    r.http.push_for(
        "rss/items",
        200,
        json!({
            "Show": {"uid": "u", "url": "https://feed.example/rss?q=Show"},
            "Show @other": {"uid": "v", "url": "https://other.example/rss?q=Show"},
        })
        .to_string(),
    );
    r.http.push_for("rss/removeRule", 200, "");
    r.http.push_for("rss/removeItem", 200, "");

    let done = subscribe(&r, "Show", vec![episode(1)], None);
    let result = subscribed(&done);
    assert_eq!(result.name, "Show");
    assert_eq!(result.save_path, "/srv/TV/Show");
    let calls = qbt_calls(&r.http);
    let sets = calls.iter().filter(|(p, _)| p == "rss/setRule").count();
    assert_eq!(sets, 1, "{calls:?}");
    assert_eq!(
        field(called(&calls, "rss/removeRule"), "ruleName").as_deref(),
        Some("Show @other")
    );
    assert_eq!(
        field(called(&calls, "rss/removeItem"), "path").as_deref(),
        Some("Show @other")
    );
}

/// A show that older builds followed three times keeps the rule the person picks (#1496).
/// The other two rules go; the feed only one of them read goes with it, the feed the kept
/// rule also reads stays, and no torrent or file is touched.
#[test]
fn keeping_one_of_a_shows_rules_removes_the_others_only() {
    let r = rig();
    let a = "https://other.example/rss?q=Show";
    let b = "https://feed.example/rss?q=Show";
    r.http.push_for(
        "rss/rules",
        200,
        json!({
            "Show": {"affectedFeeds": [a], "savePath": "/srv/TV/Show"},
            "Show @feed": {"affectedFeeds": [b], "savePath": "/srv/TV/Show @feed"},
            "Show @feed 2": {"affectedFeeds": [b], "savePath": "/srv/TV/Show @feed 2"},
            "Other": {"affectedFeeds": ["https://feed.example/rss?q=Other"]},
        })
        .to_string(),
    );
    r.http.push_for(
        "rss/items",
        200,
        json!({
            "Show": {"uid": "a", "url": a},
            "Show @feed": {"uid": "b", "url": b},
            "Other": {"uid": "c", "url": "https://feed.example/rss?q=Other"},
        })
        .to_string(),
    );
    r.http.push_for("rss/removeRule", 200, "");
    r.http.push_for("rss/removeRule", 200, "");
    r.http.push_for("rss/removeItem", 200, "");

    let job = started(
        &r.core,
        Call::KeepSubscription {
            title: "Show".to_string(),
            keep: "Show @feed".to_string(),
            others: vec!["Show".to_string(), "Show @feed 2".to_string()],
        },
    );
    let done = common::wait_job(&r.c, job);
    assert_eq!(
        done.body,
        EventBody::SubscriptionKept {
            title: "Show".to_string(),
            kept: "Show @feed".to_string(),
            removed: vec!["Show".to_string(), "Show @feed 2".to_string()],
        }
    );
    let calls = qbt_calls(&r.http);
    let removed: Vec<String> = calls
        .iter()
        .filter(|(p, _)| p == "rss/removeRule")
        .filter_map(|(_, req)| field(req, "ruleName"))
        .collect();
    assert_eq!(
        removed,
        vec!["Show".to_string(), "Show @feed 2".to_string()]
    );
    let feeds: Vec<String> = calls
        .iter()
        .filter(|(p, _)| p == "rss/removeItem")
        .filter_map(|(_, req)| field(req, "path"))
        .collect();
    assert_eq!(feeds, vec!["Show".to_string()]);
    assert!(
        !paths_of(&r.http)
            .iter()
            .any(|p| p.starts_with("torrents/") || p == "rss/setRule"),
        "{calls:?}"
    );
    no_anirss(&r.http);
}

/// The rule to keep has gone in the meantime: nothing is removed, so the show is never left
/// with no rule at all.
#[test]
fn keeping_a_rule_that_is_gone_removes_nothing() {
    let r = rig();
    r.http.push_for(
        "rss/rules",
        200,
        json!({"Show": {"affectedFeeds": ["https://other.example/rss?q=Show"]}}).to_string(),
    );
    let job = started(
        &r.core,
        Call::KeepSubscription {
            title: "Show".to_string(),
            keep: "Show @feed".to_string(),
            others: vec!["Show".to_string()],
        },
    );
    let done = common::wait_job(&r.c, job);
    failed(&done);
    assert!(writes(&r.http).is_empty(), "{:?}", writes(&r.http));
    // Asking to keep a rule and remove nothing else is refused before any job starts
    let none = r.core.call(Call::KeepSubscription {
        title: "Show".to_string(),
        keep: "Show".to_string(),
        others: vec!["Show".to_string(), " ".to_string()],
    });
    assert!(matches!(none, Err(CoreError::Invalid { .. })), "{none:?}");
}

#[test]
fn a_feed_of_that_name_on_another_address_is_refused() {
    let r = rig();
    r.http.push_for("rss/rules", 200, "{}");
    r.http.push_for("rss/addFeed", 409, "Conflict");
    r.http.push_for(
        "rss/items",
        200,
        json!({"Show": {"uid": "u", "url": "https://feed.example/rss?q=Old"}}).to_string(),
    );

    let done = subscribe(&r, "Show", vec![episode(1)], None);
    assert!(
        matches!(failed(&done), CoreError::Invalid { field, .. } if field == "name"),
        "{done:?}"
    );
    let calls = qbt_calls(&r.http);
    assert!(
        !calls.iter().any(|(p, _)| p == "rss/setRule"),
        "no rule pointed at a feed qBittorrent never took"
    );
    no_anirss(&r.http);
}

#[test]
fn subscribe_name_defaults_to_the_show_name_and_refuses_a_path() {
    let r = rig();
    let done = subscribe(&r, "Show", vec![episode(1)], Some("TV/Show"));
    assert!(
        matches!(failed(&done), CoreError::Invalid { field, .. } if field == "name"),
        "{done:?}"
    );
    assert!(
        paths_of(&r.http).is_empty(),
        "a bad name writes nothing: {:?}",
        paths_of(&r.http)
    );

    r.http.push_for("rss/rules", 200, "{}");
    r.http.push_for("rss/addFeed", 200, "");
    r.http.push_for("rss/setRule", 200, "");
    let done = subscribe(&r, "Show", vec![episode(1)], Some("  My Show  "));
    assert_eq!(subscribed(&done).name, "My Show");
    no_anirss(&r.http);
}

/// The port of the anirss engine's narrowed subscription: a generic RSS
/// endpoint cannot answer the refined query, so the rule is pointed at the
/// feed the search read and keeps the narrowing words. The core's own
/// endpoints decide that, not the list a connect to anirss would have read,
/// which here was never read at all.
#[test]
fn a_locally_narrowed_subscription_still_narrows_on_native() {
    let r = rig();
    let narrow = "https://feed.example/rss?q=Show+1080p";
    let wide = "https://feed.example/rss?q=Show";
    let feed = "<?xml version=\"1.0\"?><rss version=\"2.0\"><channel><title>f</title>\
                <item><title>[Group] Show - 01 (1080p)</title>\
                <link>magnet:?xt=urn:btih:0000000000000000000000000000000000000001</link></item>\
                </channel></rss>";
    for _ in 0..3 {
        r.http.push_for("feed.example", 200, feed);
    }
    r.http.push_for("rss/rules", 200, "{}");
    r.http.push_for("rss/addFeed", 200, "");
    r.http.push_for("rss/setRule", 200, "");
    r.http.push_for(
        "rss/rules",
        200,
        json!({"Show": {"enabled": true, "mustContain": "", "mustNotContain": "",
                        "affectedFeeds": [narrow], "savePath": "/srv/TV/Show"}})
        .to_string(),
    );
    r.http.push_for("rss/setRule", 200, "");
    r.http.push_for(
        "rss/items",
        200,
        json!({"Show": {"uid": "u", "url": narrow}}).to_string(),
    );
    r.http.push_for("rss/removeItem", 200, "");
    r.http.push_for("rss/addFeed", 200, "");

    let job = started(
        &r.core,
        Call::Subscribe {
            query: "Show 1080p".to_string(),
            endpoint: "feed".to_string(),
            items: vec![episode(1)],
            name: None,
            wire_query: Some("Show".to_string()),
            series: None,
        },
    );
    let done = common::wait_job(&r.c, job);
    let result = subscribed(&done);
    // The event names no feed (#478); the rule's `affectedFeeds` below is `wide`.
    assert_eq!(result.feed_url, "");
    assert_eq!(result.must_contain, "1080p");

    let rss: Vec<(String, HttpRequest)> = qbt_calls(&r.http)
        .into_iter()
        .filter(|(p, _)| p.starts_with("rss/"))
        .collect();
    let order: Vec<&str> = rss.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(
        order,
        vec![
            "rss/rules",
            "rss/addFeed",
            "rss/setRule",
            "rss/rules",
            "rss/setRule",
            "rss/items",
            "rss/removeItem",
            "rss/addFeed",
            // What the new rule replaces: nothing here (#1486)
            "rss/rules"
        ]
    );
    let def: Value = serde_json::from_str(&field(&rss[4].1, "ruleDef").unwrap()).unwrap();
    assert_eq!(def["mustContain"], "1080p");
    assert_eq!(def["affectedFeeds"], json!([wide]));
    assert_eq!(
        form(&rss[7].1),
        vec![
            ("url".to_string(), wide.to_string()),
            ("path".to_string(), "Show".to_string()),
        ]
    );
    no_anirss(&r.http);
}

// Listing ----------------------------------------------------------------------------

#[test]
fn list_rules_lists_every_rule_with_counts() {
    let r = rig();
    let alpha = r.dir.path().join("dl").join("alpha");
    std::fs::create_dir_all(alpha.join("S1")).unwrap();
    std::fs::write(alpha.join("01.mkv"), b"x").unwrap();
    std::fs::write(alpha.join("S1").join("02.mkv"), b"x").unwrap();
    let alpha = alpha.to_string_lossy().into_owned();
    // A rule anirss or a person wrote is listed as well as one of Anibeam's
    r.http.push_for(
        "rss/rules",
        200,
        json!({
            "Beta": {"enabled": false, "affectedFeeds": ["https://other.example/feed?q=Beta%20Show"],
                     "savePath": "/srv/TV/Beta"},
            "alpha": {"affectedFeeds": ["https://feed.example/rss?q=Alpha"], "savePath": alpha},
        })
        .to_string(),
    );
    r.http.push_for(
        "torrents/info",
        200,
        json!([
            {"hash": "a1", "name": "one", "save_path": format!("{alpha}/")},
            {"hash": "a2", "name": "two", "save_path": alpha},
            {"hash": "b1", "name": "three", "save_path": "/srv/TV/Beta"},
            {"hash": "c1", "name": "four", "save_path": "/srv/TV/Other"},
        ])
        .to_string(),
    );
    assert_eq!(r.core.call(Call::ListSubscriptionRules).unwrap(), Reply::Ok);
    let read = common::wait_for(
        &r.c,
        |e| matches!(e.body, EventBody::SubscriptionsRead { .. }),
        Duration::from_secs(10),
    );
    let EventBody::SubscriptionsRead { rules } = read.body else {
        unreachable!()
    };
    let names: Vec<&str> = rules.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, vec!["alpha", "Beta"], "by name, case ignored");
    assert_eq!(rules[0].feed_url, "https://feed.example/rss?q=Alpha");
    assert_eq!(rules[0].save_path, alpha);
    assert!(rules[0].rule_enabled);
    assert_eq!(rules[0].torrent_count, 2);
    assert_eq!(rules[0].file_count, 2);
    assert_eq!(rules[0].query, "Alpha");
    assert!(!rules[1].rule_enabled);
    assert_eq!(rules[1].torrent_count, 1);
    assert_eq!(rules[1].file_count, 0, "a folder not on this disk");
    assert_eq!(rules[1].query, "Beta Show");
    // Both feeds answer RSS, or belong to no source the core has: either can follow.
    assert!(rules[0].followable && rules[1].followable);
    no_anirss(&r.http);
}

/// Installs a source whose search answers JSON (the json-feed kind) from a repo this test signs,
/// and gives back its install id.
fn install_json_source(r: &Rig) -> String {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD as B64;
    use ed25519_dalek::{Signer, SigningKey};
    let k = SigningKey::from_bytes(&[7; 32]);
    let key = format!("ed25519:{}", B64.encode(k.verifying_key().to_bytes()));
    let index = json!({"name": "Signed here", "key": key, "serial": 1, "endpoints": [
        {"id": "api", "name": "api", "version": 1, "kind": "json-feed",
         "url": "https://search.example/feed/json?q={query}"}
    ]});
    let payload = serde_json::to_vec(&index).unwrap();
    let path = r.dir.path().join("repo.json");
    let file = json!({
        "format": "anibeam-endpoint-repo",
        "version": 1,
        "payload": B64.encode(&payload),
        "signature": B64.encode(k.sign(&payload).to_bytes()),
    });
    std::fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
    let url = path.to_string_lossy().into_owned();
    let Reply::EndpointRepoPreview { preview } = r
        .core
        .call(Call::AddEndpointRepo { url: url.clone() })
        .unwrap()
    else {
        panic!()
    };
    r.core
        .call(Call::TrustEndpointRepo {
            url: url.clone(),
            key: preview.key,
        })
        .unwrap();
    let Reply::EndpointCatalog { catalog } = r
        .core
        .call(Call::InstallEndpoint {
            repo: url,
            id: "api".into(),
            credentials: vec![],
        })
        .unwrap()
    else {
        panic!()
    };
    catalog
        .installed
        .iter()
        .find(|i| i.id == "api")
        .unwrap()
        .install_id
        .clone()
}

fn listed_rules(r: &Rig) -> Vec<SubscriptionRule> {
    r.http.push_for(
        "rss/rules",
        200,
        json!({
            "Kept": {"affectedFeeds": ["https://search.example/feed/json?q=Kept"], "savePath": "/srv/TV/Kept"},
            "Fine": {"affectedFeeds": ["https://feed.example/rss?q=Fine"], "savePath": "/srv/TV/Fine"},
        })
        .to_string(),
    );
    r.http.push_for("torrents/info", 200, "[]".to_string());
    let seen = r.c.events().iter().map(|e| e.seq).max().unwrap_or(0);
    assert_eq!(r.core.call(Call::ListSubscriptionRules).unwrap(), Reply::Ok);
    let read = common::wait_for(
        &r.c,
        move |e| e.seq > seen && matches!(e.body, EventBody::SubscriptionsRead { .. }),
        Duration::from_secs(10),
    );
    let EventBody::SubscriptionsRead { rules } = read.body else {
        unreachable!()
    };
    rules
}

#[test]
fn a_rule_on_a_source_that_answers_json_cannot_follow_even_with_the_source_off() {
    let r = rig();
    let install = install_json_source(&r);
    let follows = |rules: &[SubscriptionRule]| -> Vec<(String, bool)> {
        rules
            .iter()
            .map(|r| (r.name.clone(), r.followable))
            .collect()
    };
    let want = vec![("Fine".to_string(), true), ("Kept".to_string(), false)];
    assert_eq!(follows(&listed_rules(&r)), want);
    // Switched off for search, its subscriptions are still read, and still cannot follow.
    r.core
        .call(Call::SetEndpointEnabled {
            install,
            enabled: false,
        })
        .unwrap();
    assert_eq!(follows(&listed_rules(&r)), want);
}

// Removing --------------------------------------------------------------------------

/// One rule, its feed and one torrent saving into its folder.
fn queue_one_rule(http: &FakeHttp, save_path: &str) {
    http.push_for(
        "rss/rules",
        200,
        json!({"Show": {"affectedFeeds": ["https://feed.example/rss?q=Show"], "savePath": save_path}})
            .to_string(),
    );
    http.push_for(
        "rss/items",
        200,
        json!({"TV": {"Show": {"uid": "u", "url": "https://feed.example/rss?q=Show"}}}).to_string(),
    );
    http.push_for(
        "torrents/info",
        200,
        json!([
            {"hash": "aa", "name": "[Group] Show - 01", "save_path": save_path},
            {"hash": "zz", "name": "Other", "save_path": "/srv/TV/Other"},
        ])
        .to_string(),
    );
}

const WRITES: [&str; 4] = [
    "torrents/delete",
    "rss/removeItem",
    "rss/removeRule",
    "torrents/add",
];

fn writes(http: &FakeHttp) -> Vec<String> {
    paths_of(http)
        .into_iter()
        .filter(|p| WRITES.contains(&p.as_str()))
        .collect()
}

/// Removes the subscription `Show` through the downloads backend, the way
/// `RemoveShow` and the prune reach it; with `dry_run` it only plans. No call
/// passes `torrents`, `files` or `dry_run` any more; #858 takes them out.
fn remove_call(
    r: &Rig,
    dry_run: bool,
    torrents: bool,
    files: bool,
) -> Result<Vec<RemovePlan>, CoreError> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let backend =
            anibeam_core::downloads::backend::DownloadsBackend::open(r.core.downloads()).await?;
        backend
            .remove(&["Show".to_string()], torrents, files, dry_run)
            .await
    })
}

#[test]
fn remove_plan_matches_remove() {
    let r = rig();
    queue_one_rule(&r.http, "/srv/TV/Show");
    let planned = remove_call(&r, true, true, false);
    let plans = &planned.unwrap();
    assert_eq!(
        plans,
        &vec![RemovePlan {
            name: "Show".to_string(),
            feed_url: "https://feed.example/rss?q=Show".to_string(),
            save_path: "/srv/TV/Show".to_string(),
            torrent_count_in_qb: 1,
            size_bytes: 0,
            torrents: vec!["[Group] Show - 01".to_string()],
        }]
    );
    assert!(writes(&r.http).is_empty(), "a plan changes nothing");

    queue_one_rule(&r.http, "/srv/TV/Show");
    r.http.push_for("app/defaultSavePath", 200, SAVE_BASE);
    r.http.push_for("torrents/delete", 200, "");
    r.http.push_for("rss/removeItem", 200, "");
    r.http.push_for("rss/removeRule", 200, "");
    let done = remove_call(&r, false, true, false);
    assert!(
        matches!(&done, Ok(plans) if plans.first().is_some_and(|p| p.name == "Show")),
        "{done:?}"
    );
    assert_eq!(
        writes(&r.http),
        vec!["torrents/delete", "rss/removeItem", "rss/removeRule"]
    );
    let calls = qbt_calls(&r.http);
    let delete = called(&calls, "torrents/delete");
    assert_eq!(field(delete, "hashes").as_deref(), Some("aa"));
    assert_eq!(field(delete, "deleteFiles").as_deref(), Some("false"));
    assert_eq!(
        field(called(&calls, "rss/removeItem"), "path").as_deref(),
        Some("TV\\Show")
    );
    assert_eq!(
        field(called(&calls, "rss/removeRule"), "ruleName").as_deref(),
        Some("Show")
    );
    no_anirss(&r.http);
}

#[test]
fn remove_unknown_name_changes_nothing() {
    let r = rig();
    r.http.push_for("rss/rules", 200, "{}");
    let done = remove_call(&r, false, true, true);
    assert!(
        matches!(&done, Err(CoreError::Service { code, message, .. })
            if code == "not_found" && message.contains("Show")),
        "{done:?}"
    );
    assert!(writes(&r.http).is_empty());
    no_anirss(&r.http);
}

#[test]
fn remove_with_files_refuses_outside_the_save_base() {
    let r = rig();
    queue_one_rule(&r.http, "/elsewhere/Show");
    let done = remove_call(&r, true, true, true);
    assert!(
        matches!(&done, Err(CoreError::Invalid { field, .. }) if field == "files"),
        "a dry run refuses too: {done:?}"
    );
    queue_one_rule(&r.http, "/elsewhere/Show");
    r.http.push_for("app/defaultSavePath", 200, SAVE_BASE);
    let done = remove_call(&r, false, true, true);
    assert!(
        matches!(&done, Err(CoreError::Invalid { field, .. }) if field == "files"),
        "{done:?}"
    );
    assert!(writes(&r.http).is_empty());
}

/// On this machine the torrents go without their files, and the folder
/// goes to the trash: nothing is deleted for good.
#[test]
fn remove_with_files_trashes_and_never_deletes() {
    let dir = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var("XDG_DATA_HOME", dir.path().join("share")) };
    let base = dir.path().join("dl");
    let show = base.join("Show");
    std::fs::create_dir_all(&show).unwrap();
    std::fs::write(show.join("01.mkv"), b"abcd").unwrap();
    let base = base.to_string_lossy().into_owned();
    let show_path = show.to_string_lossy().into_owned();
    let r = rig_at(QBT, &base);

    queue_one_rule(&r.http, &show_path);
    let planned = remove_call(&r, true, true, true);
    let plans = &planned.unwrap();
    assert_eq!(plans[0].size_bytes, 4);
    assert!(show.exists(), "a plan takes nothing");

    queue_one_rule(&r.http, &show_path);
    r.http.push_for("app/defaultSavePath", 200, base.clone());
    r.http.push_for("torrents/delete", 200, "");
    r.http.push_for("rss/removeItem", 200, "");
    r.http.push_for("rss/removeRule", 200, "");
    let done = remove_call(&r, false, true, true);
    assert!(done.is_ok(), "{done:?}");
    let calls = qbt_calls(&r.http);
    assert_eq!(
        field(called(&calls, "torrents/delete"), "deleteFiles").as_deref(),
        Some("false"),
        "never the client's own delete with files on this machine"
    );
    assert!(!show.exists(), "the folder went to the trash");
    no_anirss(&r.http);
}

/// A client on another machine keeps its files out of the trash's reach:
/// its own delete takes them, and files without the torrents are refused.
#[test]
fn a_remote_client_deletes_the_files_with_its_torrents() {
    let remote = "http://192.0.2.1:9090";
    let r = rig_at(remote, SAVE_BASE);
    let on_remote = |http: &FakeHttp, path: &str| {
        http.requests()
            .into_iter()
            .find(|q| q.url == format!("{remote}/api/v2/{path}"))
    };
    let done = remove_call(&r, false, false, true);
    assert!(
        matches!(&done, Err(CoreError::Invalid { field, .. }) if field == "files"),
        "{done:?}"
    );

    queue_one_rule(&r.http, "/srv/TV/Show");
    r.http.push_for("torrents/delete", 200, "");
    r.http.push_for("rss/removeItem", 200, "");
    r.http.push_for("rss/removeRule", 200, "");
    let done = remove_call(&r, false, true, true);
    assert!(done.is_ok(), "{done:?}");
    let delete = on_remote(&r.http, "torrents/delete").expect("torrents deleted");
    assert_eq!(field(&delete, "deleteFiles").as_deref(), Some("true"));
    no_anirss(&r.http);
}

// Adding torrents --------------------------------------------------------------------

fn added(event: &Event) -> &AddedTorrents {
    match &event.body {
        EventBody::TorrentsAdded { result } => result,
        other => panic!("{other:?}"),
    }
}

#[test]
fn download_adds_magnets_and_urls_with_savepath_and_tags() {
    let r = rig();
    r.http.push_for("torrents/add", 200, "Ok.");
    let items = vec![
        episode(1),
        release(
            "[Group] Show - 02 (1080p)",
            "https://dl.example/[Group] Show - 02.torrent",
        ),
    ];
    let job = started(&r.core, Call::DownloadReleases { items, name: None });
    let done = common::wait_job(&r.c, job);
    assert_eq!(
        added(&done),
        &AddedTorrents {
            count: 2,
            save_path: "/srv/TV/Show".to_string(),
            tags: vec!["Anime".to_string()],
            // A magnet's hash is known up front; a link qBittorrent fetches is not
            hashes: vec![format!("{:040x}", 1)],
        }
    );
    let calls = qbt_calls(&r.http);
    let add = called(&calls, "torrents/add");
    assert_eq!(
        form(add),
        vec![
            (
                "urls".to_string(),
                format!(
                    "magnet:?xt=urn:btih:{:040x}\nhttps://dl.example/%5BGroup%5D%20Show%20-%2002.torrent",
                    1
                )
            ),
            ("savepath".to_string(), "/srv/TV/Show".to_string()),
            ("tags".to_string(), "Anime".to_string()),
        ]
    );
    no_anirss(&r.http);
}

#[test]
fn a_failed_add_body_is_an_error() {
    let r = rig();
    r.http.push_for(
        "torrents/add",
        200,
        json!({"success_count": 0, "failure_count": 1}).to_string(),
    );
    let job = started(
        &r.core,
        Call::DownloadReleases {
            items: vec![episode(1)],
            name: Some("Show".to_string()),
        },
    );
    let done = common::wait_job(&r.c, job);
    assert!(
        matches!(failed(&done), CoreError::Service { code, .. } if code == "add_failed"),
        "{done:?}"
    );
    no_anirss(&r.http);
}

#[test]
fn movie_adds_into_the_movie_path() {
    let r = rig();
    r.http.push_for("torrents/add", 200, "Ok.");
    let link = format!("magnet:?xt=urn:btih:{:040x}", 7);
    let job = started(
        &r.core,
        Call::DownloadMovie {
            link: link.clone(),
            needs_video: false,
        },
    );
    let done = common::wait_job(&r.c, job);
    let result = added(&done);
    // No films' folder set anywhere: the default save folder, no subfolder
    assert_eq!(result.save_path, SAVE_BASE);
    assert_eq!(result.hashes, vec![format!("{:040x}", 7)]);
    let calls = qbt_calls(&r.http);
    let add = called(&calls, "torrents/add");
    assert_eq!(field(add, "urls"), Some(link));
    assert_eq!(field(add, "savepath").as_deref(), Some(SAVE_BASE));
    no_anirss(&r.http);
}

/// A rolling pick of forty single episodes adds the first twelve now, and
/// the add's hashes come back one per release, so the roll can follow
/// each part in qBittorrent.
#[test]
fn download_with_rolling_works_on_native() {
    let r = rig();
    r.http.push_for("torrents/add", 200, "Ok.");
    let items: Vec<Release> = (1..=40)
        .rev()
        .map(|n| {
            release(
                &format!("[Group] Show - {n:03} [1080p].mkv"),
                &format!("magnet:?xt=urn:btih:{n:040x}"),
            )
        })
        .collect();
    let job = started(
        &r.core,
        Call::DownloadReleasesWith {
            items,
            name: Some("Show".to_string()),
            options: DownloadOptions {
                rolling: Some(true),
                ..DownloadOptions::default()
            },
        },
    );
    let done = common::wait_job(&r.c, job);
    let result = added(&done);
    let hashes: Vec<String> = (1..=12).map(|n| format!("{n:040x}")).collect();
    assert_eq!(result.count, 12);
    assert_eq!(result.hashes, hashes);
    let calls = qbt_calls(&r.http);
    let urls = field(called(&calls, "torrents/add"), "urls").unwrap();
    assert_eq!(urls.lines().count(), 12);
    no_anirss(&r.http);
}

/// A small but whole .torrent file.
const TORRENT: &[u8] = b"d8:announce15:http://x.test/a4:infod6:lengthi1e4:name5:a.mkvee";

fn torrent_hash() -> String {
    anibeam_core::releases::stats::info_hash(TORRENT)
        .unwrap()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A web link's .torrent is fetched here and uploaded, so its hash is
/// known and qBittorrent never fetches it itself.
#[test]
fn a_torrent_url_is_fetched_and_uploaded_with_its_hash() {
    let r = rig();
    r.http
        .push_for("dl.example/a.torrent", 200, TORRENT.to_vec());
    r.http.push_for("torrents/info", 200, "[]");
    r.http.push_for("torrents/add", 200, "Ok.");
    let job = started(
        &r.core,
        Call::DownloadReleases {
            items: vec![release(
                "[Group] Show - 01 (1080p)",
                "https://dl.example/a.torrent",
            )],
            name: Some("Show".to_string()),
        },
    );
    let done = common::wait_job(&r.c, job);
    assert_eq!(added(&done).hashes, vec![torrent_hash()]);
    let calls = qbt_calls(&r.http);
    assert!(
        calls
            .iter()
            .any(|(p, _)| *p == format!("torrents/info?hashes={}", torrent_hash())),
        "{calls:?}"
    );
    let add = called(&calls, "torrents/add");
    let Some(Body::Multipart { fields, files }) = &add.body else {
        panic!("not an upload: {:?}", add.body)
    };
    assert_eq!(
        fields,
        &vec![
            ("savepath".to_string(), "/srv/TV/Show".to_string()),
            ("tags".to_string(), "Anime".to_string()),
        ]
    );
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].field, "torrents");
    assert_eq!(files[0].file_name, "a.torrent");
    assert_eq!(files[0].bytes, TORRENT);
    assert_eq!(
        calls.iter().filter(|(p, _)| p == "torrents/add").count(),
        1,
        "no second add by URL"
    );
    no_anirss(&r.http);
}

#[test]
fn a_duplicate_upload_is_skipped() {
    let r = rig();
    r.http
        .push_for("dl.example/a.torrent", 200, TORRENT.to_vec());
    r.http.push_for(
        "torrents/info",
        200,
        json!([{"hash": torrent_hash(), "name": "a.mkv"}]).to_string(),
    );
    let job = started(
        &r.core,
        Call::DownloadReleases {
            items: vec![release(
                "[Group] Show - 01 (1080p)",
                "https://dl.example/a.torrent",
            )],
            name: Some("Show".to_string()),
        },
    );
    let done = common::wait_job(&r.c, job);
    let result = added(&done);
    assert_eq!(result.count, 1);
    assert_eq!(result.hashes, vec![torrent_hash()]);
    assert!(
        !paths_of(&r.http).iter().any(|p| p == "torrents/add"),
        "qBittorrent already has it: {:?}",
        paths_of(&r.http)
    );
    no_anirss(&r.http);
}

/// Regression for #1404: on Linux, `zbus-secret-service-keyring-store` runs
/// `zbus::utils::block_on`, which panics with "Cannot start a runtime from
/// within a runtime" if called directly on a Tokio worker thread. Native
/// download jobs and background feed/rule tasks must work without panicking
/// both on a cold keyring probe and when reading stored endpoint credentials.
#[test]
fn native_download_jobs_do_not_panic_with_runtime_blocking_keyring() {
    use anibeam_core::trackers::Secrets;
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD as B64;
    use ed25519_dalek::{Signer, SigningKey};

    let dir = tempfile::tempdir().unwrap();
    let paths = CorePaths::under(dir.path());
    let http = FakeHttp::new();

    // Seed settings and a rate-limited endpoint with a secret credential in
    // one core, then drop it so the second core starts with a cold keyring
    // probe and a stored credential to read.
    {
        let secrets = Secrets::runtime_blocking_for_test(paths.secrets_path());
        let core = Core::open_with_http_and_secrets(paths.clone(), http.clone(), secrets).unwrap();
        core.use_own_qbittorrent(None);
        core.call(Call::SetDownloadSettings {
            settings: settings(QBT),
            anirss_token: Some("tok".to_string()),
            torrent_password: Some("pw".to_string()),
        })
        .unwrap();

        let k = SigningKey::from_bytes(&[9; 32]);
        let key = format!("ed25519:{}", B64.encode(k.verifying_key().to_bytes()));
        let index = json!({"name": "Signed here", "key": key, "serial": 1, "endpoints": [
            {
                "id": "authed",
                "name": "authed",
                "version": 1,
                "kind": "rss",
                "url": "https://feed.example/rss?q={query}&apikey={apikey}",
                "rate": {"per_minute": 30},
                "auth": [{"key": "apikey", "label": "API key", "secret": true}]
            }
        ]});
        let payload = serde_json::to_vec(&index).unwrap();
        let repo_path = dir.path().join("authed-repo.json");
        let file = json!({
            "format": "anibeam-endpoint-repo",
            "version": 1,
            "payload": B64.encode(&payload),
            "signature": B64.encode(k.sign(&payload).to_bytes()),
        });
        std::fs::write(&repo_path, serde_json::to_vec(&file).unwrap()).unwrap();
        let repo_url = repo_path.to_string_lossy().into_owned();
        let Reply::EndpointRepoPreview { preview } = core
            .call(Call::AddEndpointRepo {
                url: repo_url.clone(),
            })
            .unwrap()
        else {
            panic!()
        };
        core.call(Call::TrustEndpointRepo {
            url: repo_url.clone(),
            key: preview.key,
        })
        .unwrap();
        core.call(Call::InstallEndpoint {
            repo: repo_url,
            id: "authed".into(),
            credentials: vec![EndpointCredential {
                key: "apikey".into(),
                value: "secret-key".into(),
            }],
        })
        .unwrap();
        core.shutdown();
    }

    // Reopen on the same root with an unwarmed runtime-blocking keyring.
    let http = FakeHttp::new();
    let secrets = Secrets::runtime_blocking_for_test(paths.secrets_path());
    let core = Core::open_with_http_and_secrets(paths, http.clone(), secrets).unwrap();
    core.use_own_qbittorrent(None);
    let c = Arc::new(Collector::default());
    let _sub = core.subscribe(c.clone());

    // 1. DownloadMovie on a magnet link (cold probe + credential read inside Native::for_core).
    http.push_for("app/defaultSavePath", 200, SAVE_BASE.to_string());
    http.push_for("torrents/add", 200, "Ok.");
    let link = format!("magnet:?xt=urn:btih:{:040x}&dn=Example.Film.2026.1080p", 7);
    let job = started(
        &core,
        Call::DownloadMovie {
            link: link.clone(),
            needs_video: false,
        },
    );
    let done = common::wait_job(&c, job);
    assert_eq!(added(&done).hashes, vec![format!("{:040x}", 7)]);

    // 2. ListSubscriptionRules (Native::for_core + follow_rules/fillers inside spawned task).
    http.push_for("rss/rules", 200, "{}");
    http.push_for("torrents/info", 200, "[]");
    assert_eq!(core.call(Call::ListSubscriptionRules).unwrap(), Reply::Ok);
    common::wait_for(
        &c,
        |e| matches!(e.body, EventBody::SubscriptionsRead { .. }),
        Duration::from_secs(10),
    );

    // 3. ReadFeed (endpoint_http with credentialed rate-limited endpoint inside spawned task).
    let rss = "<?xml version=\"1.0\"?><rss version=\"2.0\"><channel><title>f</title>\
               <item><title>[Group] Show - 01 (1080p)</title>\
               <link>magnet:?xt=urn:btih:0000000000000000000000000000000000000001</link></item>\
               </channel></rss>";
    http.push_for("feed.example/rss", 200, rss);
    assert_eq!(
        core.call(Call::ReadFeed {
            url: "https://feed.example/rss?q=Show".into(),
            tag: "show".into(),
        })
        .unwrap(),
        Reply::Ok
    );
    common::wait_for(
        &c,
        |e| matches!(e.body, EventBody::FeedRead { .. }),
        Duration::from_secs(10),
    );

    // 4. InspectRelease (endpoint_http inside spawned task).
    http.push_for("dl.example/a.torrent", 200, TORRENT.to_vec());
    assert_eq!(
        core.call(Call::InspectRelease {
            link: "https://dl.example/a.torrent".into(),
        })
        .unwrap(),
        Reply::Ok
    );
    common::wait_for(
        &c,
        |e| matches!(e.body, EventBody::ReleaseInspected { .. }),
        Duration::from_secs(10),
    );

    // 5. RepairSubscription (endpoint_http inside JobKind::RepairSubscription).
    http.push_for(
        "rss/rules",
        200,
        json!({"Show": {"affectedFeeds": ["https://feed.example/rss?q=Show"], "mustContain": ""}})
            .to_string(),
    );
    http.push_for("feed.example/rss", 200, rss);
    let repair = started(
        &core,
        Call::RepairSubscription {
            name: "Show".into(),
        },
    );
    let repaired = common::wait_job(&c, repair);
    assert!(
        matches!(
            repaired.body,
            EventBody::SubscriptionRepaired { found: 1, .. }
        ),
        "{repaired:?}"
    );
}
