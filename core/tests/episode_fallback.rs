//! The episode fallback (#1461): a season ani.zip has not mapped yet takes
//! its episode titles, stills, synopses and air dates from TVmaze, found
//! through the TheTVDB id ani.zip gives an earlier season of the franchise,
//! and borrows that season's logo, all in the match job. A placeholder never
//! wins, and an outage deletes nothing. Every id, title and address here is
//! invented.

mod common;
use common::started;
mod fixtures;

use anibeam_core::net::FakeHttp;
use anibeam_core::*;

/// The franchise: season 1 is mapped by ani.zip, season 2 is not and is
/// only known as season 3's prequel, and season 3 is the one in the library.
const S1: u64 = 900_010;
const S2: u64 = 900_011;
const S3: u64 = 900_012;
/// AniList's API, never ani.zip's, whose urls carry `anilist_id`.
const ANILIST: &str = "graphql.anilist.co";
const TVDB: u64 = 9_000_010;
const TVMAZE: u64 = 70_001;
const LOGO: &str = "https://images.example.com/s1-logo.png";
const STILL_1: &str = "https://images.example.com/tvmaze/s3e1.jpg";

fn node(id: u64) -> serde_json::Value {
    serde_json::json!({
        "id": id, "idMal": null, "type": "ANIME", "format": "TV", "status": "FINISHED",
        "seasonYear": 2031, "title": { "romaji": format!("Toudai {id}"), "english": null },
        "coverImage": null
    })
}

/// One enrichment reply, with the edges leaving the node.
fn enrichment(id: u64, edges: &[(&str, u64)], streaming: &[&str]) -> String {
    let edges: Vec<serde_json::Value> = edges
        .iter()
        .map(|(relation, to)| serde_json::json!({ "relationType": relation, "node": node(*to) }))
        .collect();
    let streaming: Vec<serde_json::Value> = streaming
        .iter()
        .map(|t| serde_json::json!({ "title": t, "thumbnail": null, "url": null, "site": "Example" }))
        .collect();
    serde_json::json!({ "data": { "Media": {
        "id": id, "idMal": null, "type": "ANIME", "format": "TV", "status": "FINISHED",
        "title": { "romaji": format!("Toudai {id}"), "english": null },
        "coverImage": null,
        "streamingEpisodes": streaming,
        "tags": [], "studios": { "edges": [] }, "characters": { "edges": [] },
        "recommendations": { "edges": [] },
        "relations": { "edges": edges }
    } } })
    .to_string()
}

fn media_json() -> String {
    serde_json::json!({ "data": { "Media": {
        "id": S3, "idMal": null,
        "title": { "romaji": "Toudai no Rei 3", "english": null, "native": null },
        "synonyms": [], "episodes": 3, "status": "FINISHED", "format": "TV",
        "seasonYear": 2033, "startDate": { "year": 2033, "month": 4, "day": 1 },
        "averageScore": 80, "coverImage": null, "studios": { "nodes": [] }
    } } })
    .to_string()
}

fn schedule_json() -> String {
    serde_json::json!({ "data": { "Media": { "id": S3, "nextAiringEpisode": null, "airingSchedule": { "nodes": [] } } } })
        .to_string()
}

/// ani.zip's answer for season 1: TheTVDB season 1, and a logo.
fn anizip_s1() -> String {
    serde_json::json!({
        "episodes": {
            "1": { "seasonNumber": 1, "episodeNumber": 1, "title": { "en": "First Light" } },
            "2": { "seasonNumber": 1, "episodeNumber": 2, "title": { "en": "Second Light" } }
        },
        "images": [ { "coverType": "Clearlogo", "url": LOGO, "lang": "en" } ],
        "mappings": { "anilist_id": S1, "thetvdb_id": TVDB }
    })
    .to_string()
}

/// TVmaze's show, with three seasons; season 3 is this entry.
fn tvmaze_show() -> String {
    let ep = |season: u32, number: u32, name: &str, summary: Option<&str>, image: Option<&str>| {
        serde_json::json!({
            "name": name, "season": season, "number": number,
            "airstamp": format!("2033-04-{:02}T15:00:00+00:00", number),
            "runtime": 24, "summary": summary,
            "image": image.map(|u| serde_json::json!({ "medium": u, "original": u }))
        })
    };
    serde_json::json!({
        "id": TVMAZE, "name": "Example Lighthouse", "type": "Animation", "status": "Ended",
        "externals": { "thetvdb": TVDB },
        "_embedded": { "images": [], "episodes": [
            ep(1, 1, "First Light", None, None),
            ep(2, 1, "Deep Water", None, None),
            ep(3, 1, "Harbour Lights", Some("<p>The lamp is <b>lit</b> again.</p>"), Some(STILL_1)),
            ep(3, 2, "TBA", Some("<p>Fog rolls in.</p>"), None),
            ep(3, 3, "Episode 3", None, None)
        ] }
    })
    .to_string()
}

/// Season 3 on disk, episodes 1 to 3, and no match yet.
fn show(core: &Core) -> u64 {
    let src = fixtures::insert_source(core, "/lib");
    let series = fixtures::insert_series(core, src, SeriesKind::Show, "/lib/Toudai 3", "Toudai 3");
    for n in 1..=3 {
        fixtures::insert_episode_file(
            core,
            series,
            &format!("/lib/Toudai 3/{n:02}.mkv"),
            f64::from(n),
            1,
        );
    }
    series
}

/// The match's three AniList replies, then the two the walk asks for the
/// seasons whose edges the graph does not hold yet.
fn queue_anilist(http: &FakeHttp, walk: bool) {
    http.push_for(ANILIST, 200, media_json());
    http.push_for(
        ANILIST,
        200,
        enrichment(S3, &[("PREQUEL", S2)], &["Episode 2 - Streaming Two"]),
    );
    http.push_for(ANILIST, 200, schedule_json());
    if walk {
        http.push_for(
            ANILIST,
            200,
            enrichment(S2, &[("PREQUEL", S1), ("SEQUEL", S3)], &[]),
        );
        http.push_for(ANILIST, 200, enrichment(S1, &[("SEQUEL", S2)], &[]));
    }
}

fn apply(core: &Core, c: &anibeam_core::events::Collector, series: u64) {
    let job = started(
        core,
        Call::ApplyMatch {
            series,
            target: MatchTarget::Anilist {
                id: S3,
                season: None,
            },
        },
    );
    let done = common::wait_job(c, job);
    assert_eq!(done.body, EventBody::MatchApplied { series }, "{done:#?}");
}

/// Waits for the `n`th fallback job to end: the match reports first, and the
/// walk runs after it in a job of its own.
fn wait_fallbacks(c: &anibeam_core::events::Collector, n: usize) {
    let done = |events: &[Event]| {
        events
            .iter()
            .filter(|e| {
                e.job.as_ref().is_some_and(|j| {
                    j.kind == JobKind::RefreshAiring && j.phase == JobPhase::Finished
                })
            })
            .count()
            >= n
    };
    assert!(
        c.wait_for(|events| done(events), std::time::Duration::from_secs(30)),
        "{:#?}",
        c.events()
    );
}

fn detail(core: &Core, series: u64) -> SeriesDetail {
    match core.call(Call::GetSeries { series }).unwrap() {
        Reply::SeriesDetail { detail } => *detail,
        other => panic!("{other:?}"),
    }
}

fn fallback_rows(core: &Core) -> i64 {
    core.store()
        .read(|c| {
            Ok(c.query_row(
                "SELECT count(*) FROM fallback_episodes WHERE anilist_id = ?1",
                [S3 as i64],
                |r| r.get(0),
            )?)
        })
        .unwrap()
}

fn not_found(http: &FakeHttp, id: u64) {
    http.push_for(
        &format!("api.ani.zip/v1/episodes?anilist_id={id}"),
        404,
        "\"Not Found\"",
    );
}

/// The match walks back two seasons to the one ani.zip has mapped, finds
/// the TVmaze show by its TheTVDB id, and takes season 3: titles, a still,
/// plain-text synopses and the borrowed logo arrive with the match. TVmaze's
/// placeholders lose to AniList's streaming title and to nothing at all.
#[test]
fn a_season_ani_zip_has_not_mapped_takes_tvmazes_episodes_and_a_franchise_logo() {
    let http = FakeHttp::new();
    let (_dir, core, c) = common::open_core_with_http(http.clone());
    let series = show(&core);

    queue_anilist(&http, true);
    not_found(&http, S3);
    not_found(&http, S2);
    http.push_for(
        &format!("api.ani.zip/v1/episodes?anilist_id={S1}"),
        200,
        anizip_s1(),
    );
    http.push_for(&format!("lookup/shows?thetvdb={TVDB}"), 200, tvmaze_show());
    http.push_for(
        &format!("api.tvmaze.com/shows/{TVMAZE}"),
        200,
        tvmaze_show(),
    );
    for url in [LOGO, STILL_1] {
        http.push_for(url, 200, b"image".to_vec());
    }
    apply(&core, &c, series);
    wait_fallbacks(&c, 1);

    assert_eq!(
        fallback_rows(&core),
        3,
        "{:#?}",
        http.requests()
            .iter()
            .map(|r| r.url.clone())
            .collect::<Vec<_>>()
    );
    let d = detail(&core, series);
    let titles: Vec<Option<&str>> = d.episodes.iter().map(|e| e.title.as_deref()).collect();
    assert_eq!(
        titles,
        [Some("Harbour Lights"), Some("Streaming Two"), None]
    );
    assert_eq!(
        d.episodes[0].synopsis.as_deref(),
        Some("The lamp is lit again.")
    );
    assert_eq!(d.episodes[1].synopsis.as_deref(), Some("Fog rolls in."));
    assert_eq!(d.episodes[2].synopsis, None);
    let still = d.episodes[0].still.as_deref().expect("episode 1's still");
    assert!(std::path::Path::new(still).is_file(), "{still}");
    assert!(d.episodes[0].air_date.is_some());
    let logo = d.logo.as_deref().expect("season 1's logo");
    assert!(std::path::Path::new(logo).is_file(), "{logo}");
    // The season's own artwork list stays its own.
    assert!(d.artwork.is_empty());

    core.shutdown();
}

/// TVmaze down on a later match writes nothing and deletes nothing: the
/// rows the first walk wrote stay and still read.
#[test]
fn a_tvmaze_outage_keeps_what_the_fallback_wrote() {
    let http = FakeHttp::new();
    let (_dir, core, c) = common::open_core_with_http(http.clone());
    let series = show(&core);

    queue_anilist(&http, true);
    not_found(&http, S3);
    not_found(&http, S2);
    http.push_for(
        &format!("api.ani.zip/v1/episodes?anilist_id={S1}"),
        200,
        anizip_s1(),
    );
    http.push_for(&format!("lookup/shows?thetvdb={TVDB}"), 200, tvmaze_show());
    http.push_for(
        &format!("api.tvmaze.com/shows/{TVMAZE}"),
        200,
        tvmaze_show(),
    );
    for url in [LOGO, STILL_1] {
        http.push_for(url, 200, b"image".to_vec());
    }
    apply(&core, &c, series);
    wait_fallbacks(&c, 1);
    assert_eq!(fallback_rows(&core), 3);

    // The franchise is known now, so only the match and TVmaze are asked.
    queue_anilist(&http, false);
    not_found(&http, S3);
    http.push_for(&format!("lookup/shows?thetvdb={TVDB}"), 503, "down");
    apply(&core, &c, series);
    wait_fallbacks(&c, 2);
    assert!(
        http.requests()
            .iter()
            .filter(|r| r.url.contains("lookup/shows?thetvdb="))
            .count()
            == 2
    );
    assert_eq!(fallback_rows(&core), 3);
    let d = detail(&core, series);
    assert_eq!(d.episodes[0].title.as_deref(), Some("Harbour Lights"));
    assert!(d.logo.is_some());

    core.shutdown();
}
