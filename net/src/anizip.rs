//! ani.zip, the episode side-fetch by AniList id (#1036). It maps the id to
//! AniDB and TheTVDB and answers with both sides' episode list, every
//! episode's title, synopsis, still and air date, the show's artwork and
//! its MAL id, all in one request.
//!
//! It is a side-fetch like Jikan: nothing about a match depends on it, so
//! its client retries nothing and a reply that does not come quickly is an
//! outage, which the metadata team skips for a while rather than waiting on.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};

use super::limiter::ProviderClient;
use super::{HttpRequest, null_to_default};
use anibeam_contract::CoreError;

pub const ANIZIP_API: &str = "https://api.ani.zip/v1";

/// How long one request may take. ani.zip answers in well under a second
/// when it is up, and a match waits on it.
pub const ANIZIP_TIMEOUT: Duration = Duration::from_secs(10);

/// The artwork kinds ani.zip sends, as `coverType` names them.
pub const ARTWORK_KINDS: [&str; 4] = ["Clearlogo", "Fanart", "Banner", "Poster"];

/// One field as the reply has it, or nothing when it is missing, null or of
/// a type the field cannot hold. One odd field costs that field and never
/// the whole show.
fn lenient<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let value = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(value).ok())
}

/// A count the reply may send as a number or as a numeric string.
fn lenient_u64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(match value {
        serde_json::Value::Number(n) => n.as_u64(),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    })
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct RawEpisode {
    #[serde(rename = "seasonNumber", deserialize_with = "lenient_u64")]
    season: Option<u64>,
    #[serde(rename = "episodeNumber", deserialize_with = "lenient_u64")]
    episode: Option<u64>,
    #[serde(rename = "absoluteEpisodeNumber", deserialize_with = "lenient_u64")]
    absolute: Option<u64>,
    #[serde(deserialize_with = "lenient")]
    title: Option<BTreeMap<String, Option<String>>>,
    #[serde(rename = "airDate", deserialize_with = "lenient")]
    air_date: Option<String>,
    /// AniDB's spelling of the same date, for an episode TheTVDB lacks.
    #[serde(rename = "airdate", deserialize_with = "lenient")]
    airdate: Option<String>,
    #[serde(rename = "airDateUtc", deserialize_with = "lenient")]
    air_date_utc: Option<String>,
    #[serde(deserialize_with = "lenient_u64")]
    runtime: Option<u64>,
    /// AniDB's runtime, for an episode TheTVDB lacks.
    #[serde(deserialize_with = "lenient_u64")]
    length: Option<u64>,
    #[serde(deserialize_with = "lenient")]
    overview: Option<String>,
    /// AniDB's synopsis, which ends in a `Source:` line.
    #[serde(deserialize_with = "lenient")]
    summary: Option<String>,
    #[serde(deserialize_with = "lenient")]
    image: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct RawImage {
    #[serde(rename = "coverType", deserialize_with = "lenient")]
    kind: Option<String>,
    #[serde(deserialize_with = "lenient")]
    url: Option<String>,
    #[serde(deserialize_with = "lenient")]
    lang: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct Mappings {
    #[serde(deserialize_with = "lenient_u64")]
    mal_id: Option<u64>,
    #[serde(deserialize_with = "lenient_u64")]
    anidb_id: Option<u64>,
    #[serde(deserialize_with = "lenient_u64")]
    thetvdb_id: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct Reply {
    #[serde(deserialize_with = "null_to_default")]
    episodes: BTreeMap<String, serde_json::Value>,
    #[serde(deserialize_with = "null_to_default")]
    images: Vec<serde_json::Value>,
    #[serde(deserialize_with = "null_to_default")]
    mappings: Mappings,
}

/// One episode as the core keeps it. `key` is ani.zip's own: the episode
/// number of the AniList entry for a regular episode, `S2` for a special,
/// `C1` for a credit and so on. `number` is that key read as a number, and
/// only for a regular episode.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AniZipEpisode {
    pub key: String,
    pub number: Option<u32>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub absolute: Option<u32>,
    /// The English title as ani.zip has it, AniDB's backtick apostrophes
    /// made apostrophes again.
    pub title_en: Option<String>,
    /// The romanised Japanese title, for an episode with no English one.
    pub title_romaji: Option<String>,
    pub synopsis: Option<String>,
    pub still_url: Option<String>,
    pub runtime: Option<u32>,
    /// `YYYY-MM-DD`.
    pub air_date: Option<String>,
    /// Unix seconds, when the reply gave a time as well as a day.
    pub aired_at: Option<i64>,
}

/// One piece of the show's artwork.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AniZipImage {
    /// One of `ARTWORK_KINDS`.
    pub kind: String,
    pub url: String,
    pub lang: Option<String>,
}

/// Everything one reply holds for one AniList id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AniZipShow {
    pub mal_id: Option<u64>,
    pub anidb_id: Option<u64>,
    pub tvdb_id: Option<u64>,
    /// In key order, regular episodes first by number.
    pub episodes: Vec<AniZipEpisode>,
    /// In the reply's own order.
    pub images: Vec<AniZipImage>,
}

fn clean(text: Option<String>) -> Option<String> {
    text.map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

/// AniDB writes an apostrophe as a backtick.
fn apostrophes(text: String) -> String {
    text.replace('`', "'")
}

/// AniDB's synopsis without the `Source:` line it closes on.
fn without_source(text: String) -> String {
    match text.rfind("\nSource:") {
        Some(at) => text[..at].trim_end().to_string(),
        None => text,
    }
}

fn u32_of(v: Option<u64>) -> Option<u32> {
    v.and_then(|v| u32::try_from(v).ok())
}

fn parse_utc(s: &str) -> Option<i64> {
    time::OffsetDateTime::parse(s.trim(), &time::format_description::well_known::Rfc3339)
        .ok()
        .map(|t| t.unix_timestamp())
}

/// A `YYYY-MM-DD`, or nothing for anything that is not one.
fn day_of(s: Option<String>) -> Option<String> {
    let s = s?.trim().to_string();
    let b = s.as_bytes();
    let ok = b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit());
    ok.then_some(s)
}

fn episode_of(key: &str, raw: RawEpisode) -> AniZipEpisode {
    let mut titles = raw.title.unwrap_or_default();
    let mut title = |lang: &str| clean(titles.remove(lang).flatten()).map(apostrophes);
    let title_en = title("en");
    let title_romaji = title("x-jat");
    AniZipEpisode {
        key: key.to_string(),
        number: key.parse::<u32>().ok().filter(|n| *n > 0),
        season: u32_of(raw.season),
        episode: u32_of(raw.episode),
        absolute: u32_of(raw.absolute),
        title_en,
        title_romaji,
        synopsis: clean(raw.overview)
            .or_else(|| clean(raw.summary).map(without_source))
            .filter(|s| !s.is_empty()),
        still_url: clean(raw.image).filter(|u| u.starts_with("https://")),
        runtime: u32_of(raw.runtime.or(raw.length)).filter(|m| *m > 0),
        air_date: day_of(raw.air_date).or_else(|| day_of(raw.airdate)),
        aired_at: raw.air_date_utc.as_deref().and_then(parse_utc),
    }
}

/// Whether a `Clearlogo` url is TheTVDB's clear logo art, a transparent
/// name wider than it is tall, rather than the square icon ani.zip also
/// files under that kind.
pub fn is_clear_logo(url: &str) -> bool {
    url.to_ascii_lowercase().contains("/clearlogo/")
}

fn show_of(reply: Reply) -> AniZipShow {
    let mut episodes: Vec<AniZipEpisode> = reply
        .episodes
        .into_iter()
        .filter(|(key, _)| !key.trim().is_empty())
        .map(|(key, value)| {
            let raw: RawEpisode = serde_json::from_value(value).unwrap_or_default();
            episode_of(key.trim(), raw)
        })
        .collect();
    // Regular episodes by number, then every other key in its own order.
    episodes.sort_by(|a, b| match (a.number, b.number) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.key.cmp(&b.key),
    });
    let mut images: Vec<AniZipImage> = Vec::new();
    for value in reply.images {
        let raw: RawImage = serde_json::from_value(value).unwrap_or_default();
        let (Some(kind), Some(url)) = (clean(raw.kind), clean(raw.url)) else {
            continue;
        };
        let Some(kind) = ARTWORK_KINDS.iter().find(|k| k.eq_ignore_ascii_case(&kind)) else {
            continue;
        };
        if !url.starts_with("https://") || images.iter().any(|i| i.kind == *kind && i.url == url) {
            continue;
        }
        // ani.zip labels TheTVDB's square icons Clearlogo too (#1463). Only
        // TheTVDB's own clear logo art is the show's name drawn as a logo.
        if *kind == "Clearlogo" && !is_clear_logo(&url) {
            continue;
        }
        images.push(AniZipImage {
            kind: kind.to_string(),
            url,
            lang: clean(raw.lang),
        });
    }
    AniZipShow {
        mal_id: reply.mappings.mal_id.filter(|id| *id > 0),
        anidb_id: reply.mappings.anidb_id.filter(|id| *id > 0),
        tvdb_id: reply.mappings.thetvdb_id.filter(|id| *id > 0),
        episodes,
        images,
    }
}

/// Reads one reply body. A body that is not a JSON object fails; anything
/// inside one that does not read is left out.
pub fn parse(body: &[u8]) -> Result<AniZipShow, CoreError> {
    let reply: Reply = serde_json::from_slice(body).map_err(|e| CoreError::Internal {
        message: format!("ani.zip json: {e}"),
    })?;
    Ok(show_of(reply))
}

pub struct AniZipClient {
    client: ProviderClient,
}

impl AniZipClient {
    pub fn new(client: ProviderClient) -> AniZipClient {
        AniZipClient { client }
    }

    /// Everything ani.zip has for `anilist_id`. A 404 is an id ani.zip has
    /// no entry for, which is an answer rather than a failure.
    pub async fn show(&self, anilist_id: u64) -> Result<Option<AniZipShow>, CoreError> {
        let response = self
            .client
            .send(
                HttpRequest::get(format!("{ANIZIP_API}/episodes?anilist_id={anilist_id}"))
                    .header("Accept", "application/json"),
            )
            .await?;
        if response.status == 404 {
            return Ok(None);
        }
        if !response.is_success() {
            return Err(CoreError::provider(
                crate::Upstream::AniZip.provider(),
                Some(u32::from(response.status)),
                format!("ani.zip {}: {}", response.status, response.text()),
            ));
        }
        parse(&response.body).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FakeHttp, Upstream};
    use std::sync::Arc;

    /// An invented reply in ani.zip's shape: two regular episodes, a
    /// special, a placeholder title, AniDB-only fields and an odd field.
    const SHOW: &str = include_str!("../tests/fixtures/anizip/episodes-example.json");

    fn client(http: Arc<FakeHttp>) -> AniZipClient {
        AniZipClient::new(
            ProviderClient::new(Upstream::AniZip, http, Duration::from_millis(1))
                .with_max_attempts(0),
        )
    }

    #[tokio::test]
    async fn a_reply_reads_the_mappings_episodes_and_artwork() {
        let http = FakeHttp::new();
        http.push(200, SHOW);
        let show = client(http.clone()).show(900_001).await.unwrap().unwrap();
        assert_eq!(
            http.requests()[0].url,
            "https://api.ani.zip/v1/episodes?anilist_id=900001"
        );
        assert_eq!(show.mal_id, Some(900_101));
        assert_eq!(show.anidb_id, Some(90_001));
        assert_eq!(show.tvdb_id, Some(9_000_001));

        let keys: Vec<&str> = show.episodes.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, ["1", "2", "3", "S1"]);
        let first = &show.episodes[0];
        assert_eq!(
            first,
            &AniZipEpisode {
                key: "1".into(),
                number: Some(1),
                season: Some(1),
                episode: Some(1),
                absolute: Some(1),
                title_en: Some("The Lighthouse's First Night".into()),
                title_romaji: Some("Toudai no Hajimari".into()),
                synopsis: Some("A keeper arrives at an island lighthouse.".into()),
                still_url: Some("https://images.example.com/still/1.jpg".into()),
                runtime: Some(24),
                air_date: Some("2031-04-06".into()),
                aired_at: Some(1_933_254_000),
            }
        );
        // AniDB's synopsis loses its source line, its length stands in for
        // the runtime, and a numeric string reads as a number.
        let second = &show.episodes[1];
        assert_eq!(second.synopsis.as_deref(), Some("The storm comes in."));
        assert_eq!(second.runtime, Some(23));
        assert_eq!(second.season, Some(1));
        assert_eq!(second.air_date.as_deref(), Some("2031-04-13"));
        assert_eq!(second.still_url, None);
        // A placeholder comes through as written; the metadata team drops it.
        assert_eq!(show.episodes[2].title_en.as_deref(), Some("Episode 3"));
        let special = &show.episodes[3];
        assert_eq!(special.number, None);
        assert_eq!(special.season, Some(0));

        let kinds: Vec<&str> = show.images.iter().map(|i| i.kind.as_str()).collect();
        assert_eq!(kinds, ["Banner", "Clearlogo", "Clearlogo", "Fanart"]);
        assert_eq!(show.images[2].lang.as_deref(), Some("en"));
        // The icon filed as an English Clearlogo is not one, and is gone:
        // the English logo kept is the clear logo art.
        assert!(
            show.images[2].url.ends_with("/clearlogo/logo-en.png"),
            "{:?}",
            show.images
        );
        assert!(is_clear_logo(
            "https://example.com/series/1/ClearLogo/a.png"
        ));
        assert!(!is_clear_logo("https://example.com/series/1/icons/a.jpg"));
    }

    #[tokio::test]
    async fn a_404_is_no_show_and_an_error_status_is_an_error() {
        let http = FakeHttp::new();
        http.push(404, "\"Not Found\"");
        http.push(503, "down");
        let c = client(http);
        assert_eq!(c.show(1).await.unwrap(), None);
        let err = c.show(1).await.err().unwrap();
        assert!(
            matches!(
                err,
                CoreError::Provider {
                    status: Some(503),
                    ..
                }
            ),
            "{err:?}"
        );
    }

    #[test]
    fn an_empty_or_null_reply_reads_as_nothing_and_garbage_fails() {
        assert_eq!(parse(b"{}").unwrap(), AniZipShow::default());
        assert_eq!(
            parse(br#"{"episodes": null, "images": null, "mappings": null}"#).unwrap(),
            AniZipShow::default()
        );
        assert!(parse(b"not json").is_err());
    }
}
