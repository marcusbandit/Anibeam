//! Logo languages (#1470): TheTVDB's pages name each clear logo's language,
//! and the series page draws the one the logo language setting wants,
//! following the title language unless the person picked one. Every id,
//! title, page and address here is invented.

mod common;
mod fixtures;

use anibeam_core::net::FakeHttp;
use anibeam_core::*;

const SHOW: u64 = 900_020;
const TVDB: u64 = 9_000_020;
const LOGO_JA: &str = "https://artworks.thetvdb.com/banners/v4/series/9000020/clearlogo/ja.png";
const LOGO_EN: &str = "https://artworks.thetvdb.com/banners/v4/series/9000020/clearlogo/en.png";

/// The series page: the Japanese logo first, then the English one.
fn series_page() -> String {
    format!(
        r#"<div id="artwork-clearlogo">
             <a href="{LOGO_JA}" class="lightbox" data-id="7001"><img data-src="{LOGO_JA}"></a>
             <a href="{LOGO_EN}" class="lightbox" data-id="7002"><img data-src="{LOGO_EN}"></a>
           </div>"#
    )
}

fn artwork_page(language: &str) -> String {
    format!(
        "<li><strong>Format</strong> <span>ClearLogo</span></li>
         <li><strong>Language</strong> <span>{language}</span></li>"
    )
}

fn detail(core: &Core, series: u64) -> SeriesDetail {
    match core.call(Call::GetSeries { series }).unwrap() {
        Reply::SeriesDetail { detail } => *detail,
        other => panic!("{other:?}"),
    }
}

/// Whether the series page draws the file the image cache keeps for `url`.
fn logo_is(core: &Core, series: u64, url: &'static str) -> bool {
    let path: String = core
        .store()
        .read(move |c| {
            Ok(
                c.query_row("SELECT path FROM images WHERE url = ?1", [url], |r| {
                    r.get(0)
                })?,
            )
        })
        .unwrap();
    detail(core, series)
        .logo
        .is_some_and(|logo| logo.ends_with(&path))
}

fn set_logo_language(core: &Core, logo_language: LogoLanguage) {
    let Reply::Preferences { preferences } = core.call(Call::GetPreferences).unwrap() else {
        panic!("no preferences");
    };
    core.call(Call::SetPreferences {
        preferences: Preferences {
            logo_language,
            ..preferences
        },
    })
    .unwrap();
}

#[test]
fn the_logo_follows_the_title_language_and_the_setting() {
    let http = FakeHttp::new();
    let (_dir, core, c) = common::open_core_with_http(http.clone());
    let src = fixtures::insert_source(&core, "/lib");
    let series = fixtures::insert_series(&core, src, SeriesKind::Show, "/lib/Toudai", "Toudai");
    fixtures::media(SHOW, "Toudai").insert(&core);
    fixtures::match_series(&core, series, Some(SHOW), None);
    let now = time::now_secs();
    // ani.zip knows the English logo only, and says it is English.
    core.store()
        .write(move |c| {
            c.execute_batch(&format!(
                "INSERT INTO anizip_shows (anilist_id, found, tvdb_id, fetched_at) VALUES ({SHOW}, 1, {TVDB}, {now});
                 INSERT INTO anizip_images (anilist_id, kind, url, lang, rank) VALUES
                    ({SHOW}, 'Clearlogo', '{LOGO_EN}', 'en', 0);"
            ))?;
            Ok(())
        })
        .unwrap();
    http.push_for("thetvdb.com/dereferrer/series/9000020", 200, series_page());
    http.push_for("thetvdb.com/artwork/7001", 200, artwork_page("Japanese"));
    http.push_for("thetvdb.com/artwork/7002", 200, artwork_page("English"));
    http.push_for(LOGO_JA, 200, b"image".to_vec());
    http.push_for(LOGO_EN, 200, b"image".to_vec());

    let job = anibeam_core::metadata::logos::start(core.metadata()).expect("a show owed an ask");
    common::wait_job(&c, job);

    // Romaji titles, the default, want the Japanese logo.
    assert!(logo_is(&core, series, LOGO_JA));
    set_logo_language(&core, LogoLanguage::English);
    assert!(logo_is(&core, series, LOGO_EN));
    set_logo_language(&core, LogoLanguage::Titles);
    let Reply::Preferences { preferences } = core.call(Call::GetPreferences).unwrap() else {
        panic!("no preferences");
    };
    core.call(Call::SetPreferences {
        preferences: Preferences {
            title_language: TitleLanguage::English,
            ..preferences
        },
    })
    .unwrap();
    assert!(logo_is(&core, series, LOGO_EN));

    // One ask: the same logos never ask again, and no walk starts.
    let asked = http.requests().len();
    assert_eq!(anibeam_core::metadata::logos::start(core.metadata()), None);
    assert_eq!(http.requests().len(), asked);

    core.shutdown();
}

#[test]
fn a_tvdb_that_does_not_answer_leaves_the_ani_zip_pick() {
    let http = FakeHttp::new();
    let (_dir, core, c) = common::open_core_with_http(http.clone());
    let src = fixtures::insert_source(&core, "/lib");
    let series = fixtures::insert_series(&core, src, SeriesKind::Show, "/lib/Toudai", "Toudai");
    fixtures::media(SHOW, "Toudai").insert(&core);
    fixtures::match_series(&core, series, Some(SHOW), None);
    let now = time::now_secs();
    core.store()
        .write(move |c| {
            c.execute_batch(&format!(
                "INSERT INTO anizip_shows (anilist_id, found, tvdb_id, fetched_at) VALUES ({SHOW}, 1, {TVDB}, {now});
                 INSERT INTO anizip_images (anilist_id, kind, url, lang, rank) VALUES
                    ({SHOW}, 'Clearlogo', '{LOGO_EN}', 'en', 0);"
            ))?;
            Ok(())
        })
        .unwrap();
    http.push_for("thetvdb.com/dereferrer", 503, "down");
    core.store()
        .write(move |c| {
            c.execute(
                "INSERT INTO images (url, path, bytes, fetched_at, used_at) VALUES (?1, 'aa/en.png', 5, ?2, ?2)",
                rusqlite::params![LOGO_EN, now],
            )?;
            Ok(())
        })
        .unwrap();
    core.call(Call::SetPreferences {
        preferences: Preferences {
            logo_language: LogoLanguage::Japanese,
            ..Preferences::default()
        },
    })
    .unwrap();

    let job = anibeam_core::metadata::logos::start(core.metadata()).expect("a show owed an ask");
    common::wait_job(&c, job);
    let asks: i64 = core
        .store()
        .read(|c| Ok(c.query_row("SELECT count(*) FROM tvdb_logo_asks", [], |r| r.get(0))?))
        .unwrap();
    assert_eq!(asks, 0, "an outage writes nothing");
    // ani.zip's English logo, on disk, is what the page draws.
    assert!(logo_is(&core, series, LOGO_EN));

    core.shutdown();
}
