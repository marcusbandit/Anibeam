//! TheTVDB's own site, for the one thing ani.zip's artwork leaves out: which
//! language a clear logo is in (#1470). No key: the series page lists every
//! clear logo with its artwork id, and each artwork's page names its format
//! and its language. robots.txt allows both pages.
//!
//! A side-fetch like ani.zip: a page that does not read is no language data,
//! and the logo ani.zip's order picks stands. The client retries nothing and
//! is paced gently, since one series costs its page and one page per logo.

use std::collections::HashSet;
use std::time::Duration;

use super::HttpRequest;
use super::limiter::ProviderClient;
use anibeam_contract::CoreError;
use anibeam_foundation::html::plain_text;

pub const TVDB_SITE: &str = "https://thetvdb.com";

/// Where every clear logo's file lives. A link anywhere else is not one.
const ARTWORKS: &str = "https://artworks.thetvdb.com/";

/// How long one page may take.
pub const TVDB_TIMEOUT: Duration = Duration::from_secs(15);

/// The most logos of one series whose language is asked, in the page's own
/// order. A long list is mostly old uploads nobody picks.
pub const MAX_LOGOS: usize = 8;

/// One clear logo of a series.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TvdbLogo {
    pub url: String,
    /// TheTVDB's artwork id, which its artwork page is under.
    pub artwork_id: u64,
    /// `en`, `ja` or another language's code; the page's own name, lowercased,
    /// for one this does not know. `None` when the page did not say.
    pub lang: Option<String>,
}

/// One start tag's attributes, names lowercased, values with their entities
/// decoded. `tag` is the text between `<` and `>`, the name included.
fn attributes(tag: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    // Past the tag's own name.
    let mut rest = tag.trim_start_matches(|c: char| !c.is_whitespace());
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '/');
        if rest.is_empty() {
            return out;
        }
        let name_end = rest
            .find(|c: char| c.is_whitespace() || c == '=' || c == '/')
            .unwrap_or(rest.len());
        let name = rest[..name_end].to_ascii_lowercase();
        rest = rest[name_end..].trim_start();
        let value = if let Some(after) = rest.strip_prefix('=') {
            let after = after.trim_start();
            let (value, left) = match after.chars().next() {
                Some(q @ ('"' | '\'')) => match after[1..].find(q) {
                    Some(end) => (&after[1..end + 1], &after[end + 2..]),
                    None => (&after[1..], ""),
                },
                _ => {
                    let end = after.find(char::is_whitespace).unwrap_or(after.len());
                    (&after[..end], &after[end..])
                }
            };
            rest = left;
            plain_text(value)
        } else {
            String::new()
        };
        if !name.is_empty() {
            out.push((name, value));
        }
    }
}

/// Every start tag named `name` in `html`, as the text between `<` and `>`.
fn tags<'a>(html: &'a str, lower: &'a str, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
    let open = format!("<{name}");
    let mut from = 0;
    std::iter::from_fn(move || {
        loop {
            let at = from + lower[from..].find(&open)?;
            let after = at + open.len();
            from = after;
            // `<a` must be the whole name, not the start of `<abbr`.
            if !lower[after..]
                .chars()
                .next()
                .is_some_and(|c| c.is_whitespace() || c == '>' || c == '/')
            {
                continue;
            }
            let end = after + lower[after..].find('>')?;
            from = end + 1;
            return Some(&html[at + 1..end]);
        }
    })
}

/// The clear logos a series page lists, in its order, each once: every link
/// to a file under a `/clearlogo/` path of TheTVDB's artwork host that carries
/// its artwork id. The page lists older icons in the same grids; their files
/// are not under `/clearlogo/`, so they are left out (#1463).
pub fn series_logos(html: &str) -> Vec<(String, u64)> {
    let lower = html.to_ascii_lowercase();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for tag in tags(html, &lower, "a") {
        let attrs = attributes(tag);
        let get = |n: &str| attrs.iter().find(|(k, _)| k == n).map(|(_, v)| v.trim());
        let (Some(url), Some(id)) = (get("href"), get("data-id")) else {
            continue;
        };
        let Ok(id) = id.parse::<u64>() else {
            continue;
        };
        if id == 0 || !url.starts_with(ARTWORKS) || !crate::anizip::is_clear_logo(url) {
            continue;
        }
        if seen.insert(url.to_string()) {
            out.push((url.to_string(), id));
        }
    }
    out
}

/// The fields of an artwork page, as label and value: each `<strong>` label
/// and the `<span>` after it.
fn fields(html: &str) -> Vec<(String, String)> {
    let lower = html.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = lower[from..].find("<strong") {
        let at = from + at;
        let Some(open_end) = lower[at..].find('>').map(|e| at + e + 1) else {
            break;
        };
        let Some(close) = lower[open_end..].find("</strong>").map(|e| open_end + e) else {
            break;
        };
        let label = plain_text(&html[open_end..close]);
        from = close + "</strong>".len();
        // The value is the next span, unless another label comes first.
        let next_label = lower[from..].find("<strong").map(|e| from + e);
        let Some(span) = lower[from..].find("<span").map(|e| from + e) else {
            continue;
        };
        if next_label.is_some_and(|n| n < span) {
            continue;
        }
        let Some(span_end) = lower[span..].find('>').map(|e| span + e + 1) else {
            break;
        };
        let Some(span_close) = lower[span_end..].find("</span>").map(|e| span_end + e) else {
            break;
        };
        out.push((label, plain_text(&html[span_end..span_close])));
        from = span_close + "</span>".len();
    }
    out
}

/// The language a clear logo's artwork page names, as a code. `None` when the
/// page is not one for a clear logo or names no language.
pub fn artwork_language(html: &str) -> Option<String> {
    let fields = fields(html);
    let field = |name: &str| {
        fields
            .iter()
            .find(|(k, _)| k.trim().eq_ignore_ascii_case(name))
            .map(|(_, v)| v.trim())
    };
    let format = field("format")?;
    if !format.replace(' ', "").eq_ignore_ascii_case("clearlogo") {
        return None;
    }
    language_code(field("language")?)
}

/// A language's code from the name TheTVDB's pages give it in English.
/// "Chinese - China" reads as Chinese. A name this does not know is kept,
/// lowercased, so a logo in it still counts as one in some language.
pub fn language_code(name: &str) -> Option<String> {
    let name = name.trim();
    let first = name
        .split(|c: char| c.is_whitespace() || c == '-' || c == '(' || c == ',')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if first.is_empty() {
        return None;
    }
    let code = match first.as_str() {
        "english" => "en",
        "japanese" => "ja",
        "chinese" => "zh",
        "korean" => "ko",
        "german" => "de",
        "french" => "fr",
        "spanish" => "es",
        "italian" => "it",
        "portuguese" => "pt",
        "russian" => "ru",
        "polish" => "pl",
        "dutch" => "nl",
        "danish" => "da",
        "swedish" => "sv",
        "norwegian" => "no",
        "finnish" => "fi",
        "arabic" => "ar",
        "thai" => "th",
        "turkish" => "tr",
        "hungarian" => "hu",
        "czech" => "cs",
        "hebrew" => "he",
        "greek" => "el",
        "vietnamese" => "vi",
        "indonesian" => "id",
        _ => return Some(name.to_lowercase()),
    };
    Some(code.to_string())
}

/// A status that says the page is not there and will not be: any 4xx but
/// the ones a bot check, a login wall, a rate limit or a legal block answer
/// with (401, 403, 429, 451), which say nothing lasting about the page.
pub fn lasting_refusal(status: u16) -> bool {
    (400..500).contains(&status) && !matches!(status, 401 | 403 | 429 | 451)
}

pub struct TvdbSiteClient {
    client: ProviderClient,
}

impl TvdbSiteClient {
    pub fn new(client: ProviderClient) -> TvdbSiteClient {
        TvdbSiteClient { client }
    }

    /// A page, or None when the site says it is not there. `gone` is the
    /// set of statuses that mean that: a 404 for the series page, and for
    /// an artwork page any lasting refusal (`lasting_refusal`).
    async fn page(&self, url: String, gone: fn(u16) -> bool) -> Result<Option<String>, CoreError> {
        let response = self
            .client
            .send(HttpRequest::get(url).header("Accept", "text/html"))
            .await?;
        if gone(response.status) {
            return Ok(None);
        }
        if !response.is_success() {
            return Err(CoreError::provider(
                crate::Upstream::TvdbSite.provider(),
                Some(u32::from(response.status)),
                format!("TheTVDB {}", response.status),
            ));
        }
        // The whole page: `text` cuts a body short for a log line.
        Ok(Some(String::from_utf8_lossy(&response.body).into_owned()))
    }

    /// The clear logos of TheTVDB series `tvdb_id`, each with its language
    /// where its page names one. A series TheTVDB does not know has none. An
    /// error is TheTVDB not answering, and nothing should be written on it.
    pub async fn logos(&self, tvdb_id: u64) -> Result<Vec<TvdbLogo>, CoreError> {
        // The dereferrer answers with a redirect to the series page, whose
        // address carries a name the id alone does not give.
        let Some(page) = self
            .page(format!("{TVDB_SITE}/dereferrer/series/{tvdb_id}"), |s| {
                s == 404
            })
            .await?
        else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for (url, artwork_id) in series_logos(&page).into_iter().take(MAX_LOGOS) {
            // An artwork page that is gone (404, 410 or another lasting
            // refusal) costs that logo its language and nothing more. Any
            // other failure fails the whole answer, which writes nothing: a
            // bot check, a rate limit or a timeout is no lasting word on it.
            let page = self
                .page(format!("{TVDB_SITE}/artwork/{artwork_id}"), lasting_refusal)
                .await?;
            let lang = page.as_deref().and_then(artwork_language);
            out.push(TvdbLogo {
                url,
                artwork_id,
                lang,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FakeHttp, Upstream};
    use std::sync::Arc;

    /// An invented series page in TheTVDB's shape: two clear logos in the
    /// grid, an old icon beside them, the first logo listed twice, a link to
    /// another host and one with no artwork id.
    const SERIES: &str = r#"<html><body>
        <a href="/series/the-lighthouse-keeper/seasons/official/1">Season 1</a>
        <div role="tabpanel" id="artwork-clearlogo" class="tab-pane fade in">
          <div class="simple-grid">
            <div><a href="https://artworks.thetvdb.com/banners/v4/series/9000001/clearlogo/aaa.png" class="lightbox"
                    rel="artwork_clearlogo" data-description="Added by someone" data-favorited=0 data-id="5001"
                    data-allowdelete="1"><img data-src="https://artworks.thetvdb.com/banners/v4/series/9000001/clearlogo/aaa.png"></a></div>
            <div><A HREF='https://artworks.thetvdb.com/banners/v4/series/9000001/clearlogo/bbb.png' DATA-ID=5002>x</A></div>
            <div><a href="https://artworks.thetvdb.com/banners/v4/series/9000001/icons/ccc.jpg" data-id="5003">icon</a></div>
            <div><a href="https://artworks.thetvdb.com/banners/v4/series/9000001/clearlogo/aaa.png" data-id="5001">again</a></div>
            <div><a href="https://images.example.com/clearlogo/ddd.png" data-id="5004">elsewhere</a></div>
            <div><a href="https://artworks.thetvdb.com/banners/v4/series/9000001/clearlogo/eee.png">no id</a></div>
            <abbr data-id="5005">not a link</abbr>
          </div>
        </div>
    </body></html>"#;

    /// An invented artwork page in TheTVDB's shape.
    fn artwork(format: &str, language: &str) -> String {
        format!(
            r#"<ul class="list-group">
              <li class="list-group-item clearfix"><strong>Format</strong>
                  <span>{format}</span></li>
              <li class="list-group-item clearfix"><strong>Resolution</strong> <span>800x310</span></li>
              <li class="list-group-item clearfix"><strong>Language</strong>
                  <span>{language}</span></li>
              <li class="list-group-item clearfix"><strong>Series</strong>
                  <span><a href="/series/the-lighthouse-keeper">The Lighthouse Keeper</a></span></li>
            </ul>"#
        )
    }

    fn client(http: Arc<FakeHttp>) -> TvdbSiteClient {
        TvdbSiteClient::new(
            ProviderClient::new(Upstream::TvdbSite, http, Duration::from_millis(1))
                .with_max_attempts(0),
        )
    }

    #[test]
    fn a_series_page_lists_its_clear_logos_once_and_no_icons() {
        assert_eq!(
            series_logos(SERIES),
            [
                (
                    "https://artworks.thetvdb.com/banners/v4/series/9000001/clearlogo/aaa.png"
                        .to_string(),
                    5001
                ),
                (
                    "https://artworks.thetvdb.com/banners/v4/series/9000001/clearlogo/bbb.png"
                        .to_string(),
                    5002
                ),
            ]
        );
        assert!(series_logos("").is_empty());
        assert!(series_logos("<a href=").is_empty());
        assert!(
            series_logos(
                "<html><a data-id=\"1\" href=\"https://artworks.thetvdb.com/x/clearlogo/1.png"
            )
            .is_empty()
        );
    }

    #[test]
    fn an_artwork_page_names_its_language_as_a_code() {
        assert_eq!(
            artwork_language(&artwork("ClearLogo", "English")).as_deref(),
            Some("en")
        );
        assert_eq!(
            artwork_language(&artwork("ClearLogo", " Japanese ")).as_deref(),
            Some("ja")
        );
        assert_eq!(
            artwork_language(&artwork("Clear Logo", "Chinese - China")).as_deref(),
            Some("zh")
        );
        assert_eq!(
            artwork_language(&artwork("ClearLogo", "Klingon")).as_deref(),
            Some("klingon")
        );
        // Not a clear logo's page, no language, or no page at all: nothing.
        assert_eq!(artwork_language(&artwork("Icon", "English")), None);
        assert_eq!(artwork_language(&artwork("ClearLogo", "")), None);
        assert_eq!(artwork_language("<strong>Language</strong>"), None);
        assert_eq!(artwork_language("<html>nothing here</html>"), None);
        assert_eq!(artwork_language(""), None);
    }

    #[tokio::test]
    async fn the_client_reads_the_series_page_then_each_logo_page() {
        let http = FakeHttp::new();
        http.push(200, SERIES);
        http.push(200, artwork("ClearLogo", "Japanese"));
        // An artwork page that is gone costs that logo its language only.
        http.push(410, "gone");
        let logos = client(http.clone()).logos(9_000_001).await.unwrap();
        let urls: Vec<String> = http.requests().iter().map(|r| r.url.clone()).collect();
        assert_eq!(
            urls,
            [
                "https://thetvdb.com/dereferrer/series/9000001",
                "https://thetvdb.com/artwork/5001",
                "https://thetvdb.com/artwork/5002",
            ]
        );
        assert_eq!(logos.len(), 2);
        assert_eq!(logos[0].lang.as_deref(), Some("ja"));
        assert_eq!(logos[0].artwork_id, 5001);
        assert_eq!(logos[1].lang, None);
    }

    #[tokio::test]
    async fn an_unknown_series_has_no_logos_and_a_down_site_is_an_error() {
        let http = FakeHttp::new();
        http.push(404, "not here");
        http.push(503, "down");
        http.push(200, SERIES);
        http.push(500, "down");
        http.push(200, SERIES);
        http.push(403, "a bot check");
        let c = client(http);
        assert_eq!(c.logos(1).await.unwrap(), Vec::new());
        assert!(c.logos(1).await.is_err());
        // Down part way through: the whole answer is an error, never half.
        assert!(c.logos(1).await.is_err());
        // So is an artwork page behind a bot check: no language for half a
        // year on the word of a challenge page.
        assert!(c.logos(1).await.is_err());
    }

    #[test]
    fn only_a_lasting_4xx_is_a_page_that_is_gone() {
        for gone in [400, 404, 410, 422] {
            assert!(lasting_refusal(gone), "{gone}");
        }
        for not in [200, 301, 401, 403, 429, 451, 500, 503] {
            assert!(!lasting_refusal(not), "{not}");
        }
    }

    /// Asks TheTVDB itself: Frieren and Mushoku Tensei's second season (one
    /// TheTVDB series with the first) each have a logo in English.
    #[tokio::test]
    #[ignore = "asks thetvdb.com over the network"]
    async fn live_frieren_and_mushoku_tensei_have_an_english_logo() {
        let http: Arc<dyn crate::Http> = Arc::new(crate::ReqwestHttp::new(TVDB_TIMEOUT).unwrap());
        let c = TvdbSiteClient::new(
            ProviderClient::new(Upstream::TvdbSite, http, Duration::from_millis(1500))
                .with_max_attempts(0),
        );
        for tvdb_id in [424_536, 371_310] {
            let logos = c.logos(tvdb_id).await.unwrap();
            println!("{tvdb_id}: {logos:#?}");
            assert!(
                logos.iter().any(|l| l.lang.as_deref() == Some("en")),
                "{tvdb_id}: {logos:?}"
            );
        }
    }
}
