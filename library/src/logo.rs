//! Which of a show's logos the series page draws, by language (#1470).
//! TheTVDB names each logo's language (`anibeam_metadata::logos`), and the
//! person picks one in Appearance, or lets it follow the title language.

use anibeam_contract::{LogoLanguage, TitleLanguage};

/// The language a logo is wanted in, `en` or `ja`. Following the titles,
/// English titles want the English logo, and romaji or native ones the
/// Japanese logo: a romaji title is the Japanese name, written out.
pub fn wanted(logo: LogoLanguage, titles: TitleLanguage) -> &'static str {
    match logo {
        LogoLanguage::English => "en",
        LogoLanguage::Japanese => "ja",
        LogoLanguage::Titles => match titles {
            TitleLanguage::English => "en",
            TitleLanguage::Romaji | TitleLanguage::Native => "ja",
        },
    }
}

/// How well a logo in `lang` fits `want`: 0 in it, 1 in the other of
/// English and Japanese, 2 in any other language or in none named.
fn fit(lang: Option<&str>, want: &str) -> u8 {
    let other = if want == "en" { "ja" } else { "en" };
    match lang.map(|l| l.trim().to_ascii_lowercase()) {
        Some(l) if l == want => 0,
        Some(l) if l == other => 1,
        _ => 2,
    }
}

/// Which of `langs`, the logos' languages in their source's order, to draw:
/// the first in the wanted language, else the first in the other of English
/// and Japanese, else the first at all. `None` for no logos.
pub fn pick(langs: &[Option<&str>], want: &str) -> Option<usize> {
    (0..langs.len()).min_by_key(|&i| (fit(langs[i], want), i))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn following_the_titles_english_wants_english_and_the_rest_japanese() {
        use LogoLanguage::*;
        assert_eq!(wanted(Titles, TitleLanguage::English), "en");
        assert_eq!(wanted(Titles, TitleLanguage::Romaji), "ja");
        assert_eq!(wanted(Titles, TitleLanguage::Native), "ja");
        // A language picked by name wins over the titles.
        assert_eq!(wanted(English, TitleLanguage::Native), "en");
        assert_eq!(wanted(Japanese, TitleLanguage::English), "ja");
    }

    #[test]
    fn the_wanted_language_then_the_other_then_any() {
        let langs = [Some("zh"), Some("ja"), None, Some("en"), Some("en")];
        assert_eq!(pick(&langs, "en"), Some(3));
        assert_eq!(pick(&langs, "ja"), Some(1));
        // No Japanese logo: the English one, before a logo in Chinese.
        assert_eq!(pick(&[Some("zh"), Some("EN")], "ja"), Some(1));
        // Neither: the first, named or not.
        assert_eq!(pick(&[None, Some("ko")], "en"), Some(0));
        assert_eq!(pick(&[Some("ko"), None], "ja"), Some(0));
        assert_eq!(pick(&[], "en"), None);
    }
}
