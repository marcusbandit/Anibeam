//! ani.zip's episode details (#1036): a match writes every episode's title
//! and still and the show's artwork, the titles merge ani.zip first, then
//! AniList's streaming titles, then Jikan's, a placeholder never wins, the
//! mapping's MAL id is what Jikan is asked with but never reaches the series
//! MAL list writes go through, a releasing show is asked again by the airing
//! refresh, and an outage deletes nothing and is not asked again inside its
//! window. Every id, title and address here is invented.

mod common;
use common::started;
mod fixtures;

use anibeam_core::net::FakeHttp;
use anibeam_core::*;

const ANILIST_ID: u64 = 900_001;
const MAL_ID: u64 = 900_101;
const LOGO_EN: &str = "https://images.example.com/series/1/clearlogo/logo-en.png";
const LOGO_JA: &str = "https://images.example.com/series/1/clearlogo/logo-ja.png";
const STILL_1: &str = "https://images.example.com/still/1.jpg";
const STILL_2: &str = "https://images.example.com/still/2.jpg";

/// AniList's `Media` reply, with no MAL id: the mapping has to supply it.
fn media_json() -> serde_json::Value {
    serde_json::json!({ "data": { "Media": {
        "id": ANILIST_ID,
        "idMal": null,
        "title": { "romaji": "Toudai no Rei", "english": "Example Lighthouse", "native": null },
        "synonyms": [],
        "episodes": 3,
        "status": "FINISHED",
        "format": "TV",
        "seasonYear": 2031,
        "startDate": { "year": 2031, "month": 4, "day": 6 },
        "averageScore": 80,
        "coverImage": null,
        "studios": { "nodes": [] }
    } } })
}

/// The enrichment, carrying a streaming title for episodes 1 and 2.
fn enrichment_json() -> serde_json::Value {
    serde_json::json!({ "data": { "Media": {
        "id": ANILIST_ID,
        "idMal": null,
        "type": "ANIME",
        "streamingEpisodes": [
            { "title": "Episode 1 - Streaming One", "thumbnail": null, "url": null, "site": "Example" },
            { "title": "Episode 2 - Streaming Two", "thumbnail": null, "url": null, "site": "Example" }
        ],
        "tags": [],
        "studios": { "edges": [] },
        "characters": { "edges": [] },
        "recommendations": { "edges": [] },
        "relations": { "edges": [] }
    } } })
}

fn schedule_json() -> serde_json::Value {
    serde_json::json!({ "data": { "Media": { "id": ANILIST_ID, "nextAiringEpisode": null, "airingSchedule": { "nodes": [] } } } })
}

/// ani.zip's reply: a real title for episode 1, a placeholder for episode
/// 2 and nothing for episode 3; two clear logos, the English one second.
fn anizip_json() -> String {
    serde_json::json!({
        "episodes": {
            "1": { "seasonNumber": 1, "episodeNumber": 1, "absoluteEpisodeNumber": 1,
                   "title": { "en": "The Keeper`s Lamp" }, "overview": "A lamp is lit.",
                   "runtime": 24, "airDate": "2031-04-06", "image": STILL_1 },
            "2": { "seasonNumber": 1, "episodeNumber": 2, "absoluteEpisodeNumber": 2,
                   "title": { "en": "Episode 2" }, "runtime": 24, "airDate": "2031-04-13",
                   "image": STILL_2 },
            "S1": { "seasonNumber": 0, "episodeNumber": 1, "title": { "en": "Harbour Recap" } }
        },
        "images": [
            { "coverType": "Fanart", "url": "https://images.example.com/fanart.jpg" },
            { "coverType": "Clearlogo", "url": LOGO_JA, "lang": "ja" },
            { "coverType": "Clearlogo", "url": LOGO_EN, "lang": "en" },
            { "coverType": "Poster", "url": "https://images.example.com/poster.jpg" }
        ],
        "mappings": { "anilist_id": ANILIST_ID, "mal_id": MAL_ID, "anidb_id": 90_001, "thetvdb_id": 9_000_001 }
    })
    .to_string()
}

/// Jikan's one page, with a title for episode 3 and the placeholder AniList
/// and ani.zip lose to for the rest.
fn jikan_page() -> String {
    serde_json::json!({
        "pagination": { "has_next_page": false },
        "data": [
            { "mal_id": 1, "episode": 1, "title": "Jikan One", "aired": null },
            { "mal_id": 2, "episode": 2, "title": "Jikan Two", "aired": null },
            { "mal_id": 3, "episode": 3, "title": "Jikan Three", "aired": null }
        ]
    })
    .to_string()
}

/// A finished show with episodes 1 to 3 on disk and no match.
fn show(core: &Core) -> u64 {
    let src = fixtures::insert_source(core, "/lib");
    let series = fixtures::insert_series(
        core,
        src,
        SeriesKind::Show,
        "/lib/Example Lighthouse",
        "Example Lighthouse",
    );
    for n in 1..=3 {
        fixtures::insert_episode_file(
            core,
            series,
            &format!("/lib/Example Lighthouse/{n:02}.mkv"),
            f64::from(n),
            1,
        );
    }
    series
}

fn queue_anilist(http: &FakeHttp) {
    http.push_for("anilist", 200, media_json().to_string());
    http.push_for("anilist", 200, enrichment_json().to_string());
    http.push_for("anilist", 200, schedule_json().to_string());
}

fn apply(core: &Core, c: &anibeam_core::events::Collector, series: u64) {
    let job = started(
        core,
        Call::ApplyMatch {
            series,
            target: MatchTarget::Anilist {
                id: ANILIST_ID,
                season: None,
            },
        },
    );
    let done = common::wait_job(c, job);
    assert_eq!(done.body, EventBody::MatchApplied { series }, "{done:#?}");
}

fn detail(core: &Core, series: u64) -> SeriesDetail {
    match core.call(Call::GetSeries { series }).unwrap() {
        Reply::SeriesDetail { detail } => *detail,
        other => panic!("{other:?}"),
    }
}

fn anizip_requests(http: &FakeHttp) -> usize {
    http.requests()
        .iter()
        .filter(|r| r.url.contains("api.ani.zip"))
        .count()
}

fn anizip_rows(core: &Core) -> i64 {
    core.store()
        .read(|c| {
            Ok(c.query_row(
                "SELECT count(*) FROM anizip_episodes WHERE anilist_id = ?1",
                [ANILIST_ID as i64],
                |r| r.get(0),
            )?)
        })
        .unwrap()
}

fn jikan_asked_with_mapping(http: &FakeHttp) -> usize {
    http.requests()
        .iter()
        .filter(|r| r.url.contains("jikan.moe") && r.url.contains(&format!("/anime/{MAL_ID}/")))
        .count()
}

fn series_mal(core: &Core, series: u64) -> Option<i64> {
    core.store()
        .read(move |c| {
            Ok(c.query_row(
                "SELECT mal_id FROM series WHERE id = ?1",
                [series as i64],
                |r| r.get(0),
            )?)
        })
        .unwrap()
}

/// A match through ani.zip: its titles win, a placeholder of its falls to
/// AniList's streaming title, an episode it has no title for falls to
/// Jikan's, which the mapped MAL id let the match ask, and the stills of
/// the episodes on disk and the English clear logo are on disk as files.
#[test]
fn a_match_writes_anizips_episodes_and_artwork_and_its_titles_win() {
    let http = FakeHttp::new();
    let (_dir, core, c) = common::open_core_with_http(http.clone());
    let series = show(&core);

    queue_anilist(&http);
    http.push_for("api.ani.zip", 200, anizip_json());
    http.push_for("jikan.moe", 200, jikan_page());
    for url in [STILL_1, STILL_2, LOGO_EN] {
        http.push_for(url, 200, b"image".to_vec());
    }
    apply(&core, &c, series);

    assert_eq!(
        http.requests()
            .iter()
            .find(|r| r.url.contains("api.ani.zip"))
            .map(|r| r.url.clone())
            .as_deref(),
        Some("https://api.ani.zip/v1/episodes?anilist_id=900001")
    );
    // The special is kept beside the regular episodes.
    assert_eq!(anizip_rows(&core), 3);
    // The mapping asked Jikan and went no further.
    assert_eq!(jikan_asked_with_mapping(&http), 1, "{:#?}", http.requests());
    assert_eq!(series_mal(&core, series), None);

    let d = detail(&core, series);
    let titles: Vec<Option<&str>> = d.episodes.iter().map(|e| e.title.as_deref()).collect();
    assert_eq!(
        titles,
        [
            Some("The Keeper's Lamp"),
            Some("Streaming Two"),
            Some("Jikan Three")
        ]
    );
    let still = d.episodes[0].still.as_deref().expect("episode 1's still");
    assert!(std::path::Path::new(still).is_file(), "{still}");
    assert!(d.episodes[1].still.is_some());
    assert_eq!(d.episodes[2].still, None);

    let logo = d.logo.as_deref().expect("the English logo");
    assert!(logo.ends_with(".png") && std::path::Path::new(logo).is_file());
    let kinds: Vec<&str> = d.artwork.iter().map(|a| a.kind.as_str()).collect();
    assert_eq!(kinds, ["Fanart", "Clearlogo", "Clearlogo", "Poster"]);
    // Only the logo is fetched up front.
    assert_eq!(d.artwork[0].path, None);
    assert_eq!(d.artwork[2].url, LOGO_EN);
    assert_eq!(d.artwork[2].path.as_deref(), Some(logo));
    assert!(!http.requests().iter().any(|r| r.url == LOGO_JA));

    core.shutdown();
}

/// An ani.zip that is down costs nothing: the match lands, every row the
/// last answer wrote stays and still wins, the MAL id it mapped still asks
/// Jikan, the warning is said once, and the next match inside the window
/// does not ask.
#[test]
fn an_anizip_outage_deletes_nothing_and_is_skipped_inside_the_window() {
    let http = FakeHttp::new();
    let (_dir, core, c) = common::open_core_with_http(http.clone());
    let series = show(&core);

    queue_anilist(&http);
    http.push_for("api.ani.zip", 200, anizip_json());
    http.push_for("jikan.moe", 200, jikan_page());
    for url in [STILL_1, STILL_2, LOGO_EN] {
        http.push_for(url, 200, b"image".to_vec());
    }
    apply(&core, &c, series);
    assert_eq!(anizip_requests(&http), 1);

    queue_anilist(&http);
    http.push_for("api.ani.zip", 503, "down");
    http.push_for("jikan.moe", 200, jikan_page());
    apply(&core, &c, series);
    assert_eq!(anizip_requests(&http), 2);
    assert_eq!(anizip_rows(&core), 3);
    assert_eq!(jikan_asked_with_mapping(&http), 2);
    assert_eq!(series_mal(&core, series), None);
    let d = detail(&core, series);
    assert_eq!(d.episodes[0].title.as_deref(), Some("The Keeper's Lamp"));
    assert!(d.logo.is_some());

    queue_anilist(&http);
    http.push_for("jikan.moe", 200, jikan_page());
    apply(&core, &c, series);
    assert_eq!(anizip_requests(&http), 2, "{:#?}", http.requests());
    let warnings = c
        .events()
        .into_iter()
        .filter(|e| e.level == Level::Warn && e.message.contains("ani.zip"))
        .count();
    assert_eq!(warnings, 1);

    core.shutdown();
}

/// A series matched before ani.zip was asked gets its details from the
/// launch's walk, which keeps the MAL id AniList left empty for Jikan alone
/// and leaves the series owed Jikan's titles; a walk with nothing left to ask
/// starts no job.
#[test]
fn the_walk_fills_a_series_matched_before_anizip_was_asked() {
    let http = FakeHttp::new();
    let (_dir, core, c) = common::open_core_with_http(http.clone());
    let series = show(&core);
    fixtures::media(ANILIST_ID, "Toudai no Rei")
        .episodes(3)
        .insert(&core);
    fixtures::match_series(&core, series, Some(ANILIST_ID), None);

    http.push_for("api.ani.zip", 200, anizip_json());
    for url in [STILL_1, STILL_2, LOGO_EN] {
        http.push_for(url, 200, b"image".to_vec());
    }
    let job = anibeam_core::metadata::anizip::start(core.metadata()).expect("a series to ask");
    common::wait_job(&c, job);

    assert_eq!(anizip_rows(&core), 3);
    assert_eq!(series_mal(&core, series), None);
    let owed: Vec<u64> = core
        .store()
        .read(|c| Ok(anibeam_core::metadata::owed::owed(c)?.into_iter().collect()))
        .unwrap();
    assert_eq!(owed, [ANILIST_ID]);
    let d = detail(&core, series);
    assert_eq!(d.episodes[0].title.as_deref(), Some("The Keeper's Lamp"));
    assert!(d.episodes[0].still.is_some());
    assert!(d.logo.is_some());

    assert!(anibeam_core::metadata::anizip::start(core.metadata()).is_none());

    core.shutdown();
}

/// A releasing show whose ani.zip answer is older than the airing window is
/// asked again by the airing refresh, so an episode that aired after the
/// match gets its title and still; one asked inside the window is not.
#[test]
fn the_airing_refresh_asks_again_for_a_releasing_show() {
    let http = FakeHttp::new();
    let (_dir, core, c) = common::open_core_with_http(http.clone());
    let series = show(&core);
    fixtures::media(ANILIST_ID, "Toudai no Rei")
        .episodes(12)
        .status("RELEASING")
        .insert(&core);
    fixtures::match_series(&core, series, Some(ANILIST_ID), None);
    let now = time::now_secs();
    // An answer from a day ago that knew of episode 1 only.
    core.store()
        .write(move |c| {
            c.execute_batch(&format!(
                "INSERT INTO anizip_shows (anilist_id, found, mal_id, fetched_at) VALUES ({ANILIST_ID}, 1, {MAL_ID}, {});
                 INSERT INTO anizip_episodes (anilist_id, key, number, title) VALUES ({ANILIST_ID}, '1', 1, 'The Keeper''s Lamp');",
                now - 86_400
            ))?;
            Ok(())
        })
        .unwrap();

    let schedule = serde_json::json!({ "data": { "Media": { "id": ANILIST_ID, "status": "RELEASING",
        "nextAiringEpisode": { "episode": 4, "airingAt": now + 86_400 },
        "airingSchedule": { "nodes": [] } } } });
    http.push_for("anilist", 200, schedule.to_string());
    http.push_for("jikan.moe", 200, jikan_page());
    http.push_for("api.ani.zip", 200, anizip_json());
    for url in [STILL_1, STILL_2, LOGO_EN] {
        http.push_for(url, 200, b"image".to_vec());
    }
    let job = started(&core, Call::RefreshAiring { series });
    let done = common::wait_job(&c, job);
    assert!(
        matches!(done.body, EventBody::AiringRefreshed { updated: true, .. }),
        "{done:?}"
    );
    assert_eq!(anizip_requests(&http), 1);
    assert_eq!(anizip_rows(&core), 3);
    let d = detail(&core, series);
    assert!(d.episodes[1].still.is_some(), "{:#?}", d.episodes);
    assert!(d.logo.is_some());

    // Asked just now, so the next refresh leaves ani.zip alone.
    http.push_for("anilist", 200, schedule.to_string());
    http.push_for("jikan.moe", 200, jikan_page());
    let job = started(&core, Call::RefreshAiring { series });
    common::wait_job(&c, job);
    assert_eq!(anizip_requests(&http), 1, "{:#?}", http.requests());

    core.shutdown();
}
