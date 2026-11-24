//! anibeam://add-repo?url=... (#1406): a guide site's one-tap link to a source list. The
//! link only opens the add-source-list flow with the address filled in and its preview
//! started; the list is added when the person taps Trust there, never by the link.
//!
//! On Android a browser hands the link to `LinkActivity.java`, which keeps it and brings
//! the app forward. The app takes it when it comes to the front, and the clock looks for one
//! each second as well, since an app already in front (split screen) may not be resumed
//! again; a link that comes before the core is open waits for it.

/// Why a link was refused, worded when it is shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// No `url` in the link, or an empty one.
    NoUrl,
    /// The link, or the address in it, could not be read.
    Unreadable,
    /// The address is not https.
    NotHttps,
}

impl Refusal {
    pub fn text(&self) -> String {
        match self {
            Refusal::NoUrl => tr!("repos-link-no-url"),
            Refusal::Unreadable => tr!("repos-link-unreadable"),
            Refusal::NotHttps => tr!("repos-link-not-https"),
        }
    }
}

/// The source list address an anibeam://add-repo link carries, decoded, or why it is refused.
pub fn parse(link: &str) -> Result<String, Refusal> {
    let link = url::Url::parse(link.trim()).map_err(|_| Refusal::Unreadable)?;
    if !link.scheme().eq_ignore_ascii_case("anibeam") || link.host_str() != Some("add-repo") {
        return Err(Refusal::Unreadable);
    }
    // Percent-decoded the way the desktop reads it (apps/desktop/src/args.rs), not form
    // decoded, so a literal + stays a + and both apps get the same address from one link.
    let Some(raw) = link
        .query()
        .unwrap_or("")
        .split('&')
        .find_map(|pair| pair.strip_prefix("url="))
    else {
        return Err(Refusal::NoUrl);
    };
    let address = percent_encoding::percent_decode_str(raw)
        .decode_utf8()
        .map_err(|_| Refusal::Unreadable)?;
    let address = address.trim();
    if address.is_empty() {
        return Err(Refusal::NoUrl);
    }
    let parsed = url::Url::parse(address).map_err(|_| Refusal::Unreadable)?;
    if parsed.scheme() != "https" {
        return Err(Refusal::NotHttps);
    }
    if parsed.host_str().is_none_or(str::is_empty) {
        return Err(Refusal::Unreadable);
    }
    // https://good.example@other.example/ reads as the first and goes to the second, so a
    // login part is refused, as the desktop does
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(Refusal::Unreadable);
    }
    Ok(address.to_string())
}

/// The app came to the front: a link may have brought it.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub fn resumed() {
    poll();
}

/// Takes a waiting link once the core is open, and opens what it asks for. Called on the UI
/// loop, from the clock each second and on resume. Looking costs one static field read
/// (`take_link`), and a link stays where it is until it is taken.
pub fn poll() {
    if crate::local::core().is_none() {
        return;
    }
    let Some(link) = crate::platform::take_link() else {
        return;
    };
    crate::local::log("opened by an add-repo link");
    crate::repos::open_link(parse(&link));
}

#[cfg(test)]
mod tests {
    use super::{Refusal, parse};

    #[test]
    fn a_link_hands_over_its_decoded_https_address() {
        assert_eq!(
            parse("anibeam://add-repo?url=https%3A%2F%2Fexample.com%2Frepo.json").as_deref(),
            Ok("https://example.com/repo.json")
        );
        assert_eq!(
            parse("anibeam://add-repo?url=https%3A%2F%2Fexample.com%2Fa%2Findex.json%3Fv%3D2")
                .as_deref(),
            Ok("https://example.com/a/index.json?v=2")
        );
    }

    #[test]
    fn a_plus_in_the_address_stays_a_plus() {
        assert_eq!(
            parse("anibeam://add-repo?url=https%3A%2F%2Fexample.com%2Fa+b.json").as_deref(),
            Ok("https://example.com/a+b.json")
        );
        assert_eq!(
            parse("anibeam://add-repo?from=guide&url=https://example.com/c%2Bd.json").as_deref(),
            Ok("https://example.com/c+d.json")
        );
    }

    #[test]
    fn an_address_with_a_login_part_is_refused() {
        assert_eq!(
            parse("anibeam://add-repo?url=https%3A%2F%2Fgood.example%40other.example%2Frepo.json"),
            Err(Refusal::Unreadable)
        );
        assert_eq!(
            parse("anibeam://add-repo?url=https%3A%2F%2Fme%3Asecret%40example.com%2Frepo.json"),
            Err(Refusal::Unreadable)
        );
        assert_eq!(
            parse("anibeam://add-repo?url=https%3A%2F%2F%3Asecret%40example.com%2Frepo.json"),
            Err(Refusal::Unreadable)
        );
        // An @ in the path is not a login part
        assert_eq!(
            parse("anibeam://add-repo?url=https%3A%2F%2Fexample.com%2F%40me%2Frepo.json")
                .as_deref(),
            Ok("https://example.com/@me/repo.json")
        );
    }

    #[test]
    fn a_link_with_no_address_is_refused_as_such() {
        assert_eq!(parse("anibeam://add-repo"), Err(Refusal::NoUrl));
        assert_eq!(parse("anibeam://add-repo?url="), Err(Refusal::NoUrl));
        assert_eq!(parse("anibeam://add-repo?other=1"), Err(Refusal::NoUrl));
    }

    #[test]
    fn an_address_that_is_not_https_is_refused() {
        assert_eq!(
            parse("anibeam://add-repo?url=http%3A%2F%2Fexample.com%2Fr.json"),
            Err(Refusal::NotHttps)
        );
        assert_eq!(
            parse("anibeam://add-repo?url=file%3A%2F%2F%2Fsdcard%2Fr.json"),
            Err(Refusal::NotHttps)
        );
    }

    #[test]
    fn an_unreadable_link_or_address_is_refused() {
        assert_eq!(parse("not a link"), Err(Refusal::Unreadable));
        assert_eq!(
            parse("anibeam://add-repo?url=no%20address"),
            Err(Refusal::Unreadable)
        );
        assert_eq!(
            parse("anibeam://rooms?url=https%3A%2F%2Fexample.com%2Fr.json"),
            Err(Refusal::Unreadable)
        );
        assert_eq!(
            parse("https://add-repo?url=https%3A%2F%2Fexample.com%2Fr.json"),
            Err(Refusal::Unreadable)
        );
    }
}
