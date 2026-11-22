//! The text, link, markdown and asset rules of storefront design §4,
//! shared by the client (lenient where forward compatibility needs it) and
//! the publishing tool (strict).

use std::collections::BTreeMap;
use std::net::Ipv4Addr;

use crate::media::MediaFormat;

/// Localized text: BCP 47 language tag → text. Every map needs `en`.
pub type Localized = BTreeMap<String, String>;

/// The languages storefront text may be written in today (the publishing
/// tool refuses others; clients accept any well-formed tag, so adding a
/// language never breaks an older client).
pub const LANGUAGES: [&str; 2] = ["en", "pl"];

/// The language every localized map must have.
pub const FALLBACK_LANGUAGE: &str = "en";

/// Longest link.
pub const MAX_URL_CHARS: usize = 512;

/// How strictly a document is checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strictness {
    /// The publishing tool and CI: the closed language set, Unicode NFC
    /// (with the `publish` feature).
    Publish,
    /// A client reading a signed document: any well-formed language tag.
    Client,
}

/// A well-formed language tag: a 2-3 letter lowercase language, then
/// optional subtags of 1-8 letters or digits (`pl`, `pl-PL`, `zh-Hant`).
pub fn is_language_tag(tag: &str) -> bool {
    let mut parts = tag.split('-');
    let Some(language) = parts.next() else {
        return false;
    };
    (2..=3).contains(&language.len())
        && language.bytes().all(|b| b.is_ascii_lowercase())
        && parts.all(|p| (1..=8).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// Characters refused in any storefront text: controls (except `\n` where
/// line breaks are allowed), bidi overrides and isolates, directional
/// marks, and zero-width characters.
fn refused_char(c: char, multiline: bool) -> Option<&'static str> {
    match c {
        '\n' if multiline => None,
        '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}' => {
            Some("a bidirectional control")
        }
        '\u{061C}' => Some("a bidirectional control"),
        '\u{2028}' | '\u{2029}' => Some("a line or paragraph separator"),
        '\u{200B}'..='\u{200D}' | '\u{2060}' | '\u{FEFF}' | '\u{180E}' => {
            Some("a zero-width character")
        }
        c if c.is_control() => Some("a control character"),
        _ => None,
    }
}

#[cfg(feature = "publish")]
fn is_nfc(text: &str) -> bool {
    use unicode_normalization::{IsNormalized, is_nfc_quick};
    match is_nfc_quick(text.chars()) {
        IsNormalized::Yes => true,
        IsNormalized::No => false,
        IsNormalized::Maybe => {
            unicode_normalization::UnicodeNormalization::nfc(text).eq(text.chars())
        }
    }
}

/// One string: its length in characters and the character rules.
pub fn check_text(
    field: &str,
    text: &str,
    min: usize,
    max: usize,
    multiline: bool,
    strictness: Strictness,
) -> Result<(), String> {
    let count = text.chars().count();
    if count < min || count > max {
        return Err(format!(
            "{field} has {count} characters; {min} to {max} are allowed"
        ));
    }
    if let Some(problem) = text.chars().find_map(|c| refused_char(c, multiline)) {
        return Err(format!("{field} contains {problem}"));
    }
    #[cfg(feature = "publish")]
    if strictness == Strictness::Publish && !is_nfc(text) {
        return Err(format!("{field} is not in Unicode NFC"));
    }
    #[cfg(not(feature = "publish"))]
    let _ = strictness;
    Ok(())
}

/// A localized map: `en` present, well-formed (or, strictly, known) tags,
/// every text within the limits.
pub fn check_localized(
    field: &str,
    text: &Localized,
    min: usize,
    max: usize,
    multiline: bool,
    strictness: Strictness,
) -> Result<(), String> {
    if !text.contains_key(FALLBACK_LANGUAGE) {
        return Err(format!("{field} has no `{FALLBACK_LANGUAGE}` text"));
    }
    for (tag, value) in text {
        let known = match strictness {
            Strictness::Publish => LANGUAGES.contains(&tag.as_str()),
            Strictness::Client => is_language_tag(tag),
        };
        if !known {
            return Err(format!(
                "{field} has the language `{tag}`; allowed: {}",
                LANGUAGES.join(", ")
            ));
        }
        check_text(
            &format!("{field}.{tag}"),
            value,
            min,
            max,
            multiline,
            strictness,
        )?;
    }
    Ok(())
}

/// The text for `locale`: the exact tag, then its language, then `en`.
pub fn localize<'a>(text: &'a Localized, locale: &str) -> Option<&'a str> {
    let language = locale.split(['-', '_']).next().unwrap_or(locale);
    text.get(locale)
        .or_else(|| text.get(&locale.replace('_', "-")))
        .or_else(|| text.get(language))
        .or_else(|| text.get(FALLBACK_LANGUAGE))
        .map(String::as_str)
}

// ------------------------------------------------------------------ links

/// A link (§4 "URLs"): `https:` only, at most 512 characters, no userinfo,
/// no IP-literal host, no explicit port, a lowercase ASCII host without
/// punycode. A first-party (`deckpilot.*`) listing may only link to
/// `github.com/knittingcode/…` or `knittingcode.github.io`.
pub fn check_url(field: &str, url: &str, first_party: bool) -> Result<(), String> {
    let refuse = |why: &str| Err(format!("{field} `{url}` {why}"));
    if url.chars().count() > MAX_URL_CHARS {
        return Err(format!("{field} is longer than {MAX_URL_CHARS} characters"));
    }
    if !url.bytes().all(|b| {
        b.is_ascii_graphic()
            && !matches!(
                b,
                b'"' | b'<' | b'>' | b'\\' | b'`' | b'{' | b'}' | b'|' | b'^'
            )
    }) {
        return refuse("contains a character links may not have");
    }
    let Some(rest) = url.strip_prefix("https://") else {
        return refuse("is not an https: link");
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (host, path) = rest.split_at(end);
    if host.contains('@') {
        return refuse("carries user information");
    }
    if host.contains(':') || host.starts_with('[') {
        return refuse("names a port or an IP address");
    }
    if host.is_empty()
        || !host
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
    {
        return refuse("does not have a lowercase ASCII host");
    }
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2
        || labels
            .iter()
            .any(|l| l.is_empty() || l.len() > 63 || l.starts_with('-') || l.ends_with('-'))
    {
        return refuse("does not have a valid host name");
    }
    if labels.iter().any(|l| l.starts_with("xn--")) {
        return refuse("uses a punycode (xn--) host");
    }
    if host.parse::<Ipv4Addr>().is_ok()
        || labels
            .last()
            .is_some_and(|l| l.bytes().all(|b| b.is_ascii_digit()))
    {
        return refuse("names an IP address");
    }
    // No dot segments, plain or percent-encoded: `/a/../b` could leave the
    // allowed path prefix once a browser normalizes it.
    let path_only = path.split(['?', '#']).next().unwrap_or_default();
    if path_only.split('/').any(|segment| {
        let decoded = segment.to_ascii_lowercase().replace("%2e", ".");
        decoded == "." || decoded == ".."
    }) {
        return refuse("has a dot segment in its path");
    }
    if first_party {
        let allowed = match host {
            "github.com" => path.starts_with("/knittingcode/"),
            "knittingcode.github.io" => true,
            _ => false,
        };
        if !allowed {
            return refuse(
                "is not on github.com/knittingcode/ or knittingcode.github.io (first-party listings)",
            );
        }
    }
    Ok(())
}

// --------------------------------------------------------------- markdown

/// The safe markdown subset (§4): paragraphs, line breaks, one-level `-`
/// and `*` lists, `**bold**`, `_em_`, `` `code` `` and `[text](https://…)`
/// links under [`check_url`]. Refused: raw HTML (`<` followed by a letter,
/// `/` or `!`), headings, images and tables. Anything else renders as
/// literal text on the client.
pub fn check_markdown(field: &str, text: &str, first_party: bool) -> Result<(), String> {
    let bytes = text.as_bytes();
    for (i, window) in bytes.windows(2).enumerate() {
        if window[0] == b'<'
            && (window[1].is_ascii_alphabetic() || matches!(window[1], b'/' | b'!'))
        {
            return Err(format!("{field} contains raw HTML at byte {i}"));
        }
        if window == b"![" {
            return Err(format!("{field} contains an image"));
        }
    }
    for line in text.lines() {
        let trimmed = line.trim_start();
        let hashes = trimmed.bytes().take_while(|&b| b == b'#').count();
        if (1..=6).contains(&hashes)
            && trimmed[hashes..]
                .chars()
                .next()
                .is_none_or(char::is_whitespace)
        {
            return Err(format!("{field} contains a heading"));
        }
        if trimmed.starts_with('|') {
            return Err(format!("{field} contains a table"));
        }
        if trimmed.starts_with('>') {
            return Err(format!("{field} contains a block quote"));
        }
        let indent = line.len() - trimmed.len();
        if indent >= 2 && (trimmed.starts_with("- ") || trimmed.starts_with("* ")) {
            return Err(format!("{field} contains a nested list"));
        }
    }
    // Every link target.
    let mut rest = text;
    while let Some(at) = rest.find("](") {
        let after = &rest[at + 2..];
        let end = after
            .find(')')
            .ok_or_else(|| format!("{field} has an unterminated link"))?;
        check_url(&format!("{field} link"), &after[..end], first_party)?;
        rest = &after[end..];
    }
    Ok(())
}

// ----------------------------------------------------------------- assets

/// What an image is used for, and its §4 constraints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetKind {
    Icon,
    Screenshot,
    PromoArt,
    Banner,
}

/// Largest icon.
pub const MAX_ICON_BYTES: u64 = 256 * 1024;

fn ratio_within(width: u32, height: u32, wide: u32, high: u32) -> bool {
    // |w/h − wide/high| ≤ 1% of wide/high, in integers.
    let lhs = (u64::from(width) * u64::from(high)).abs_diff(u64::from(height) * u64::from(wide));
    lhs * 100 <= u64::from(height) * u64::from(wide)
}

impl AssetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Icon => "icon",
            Self::Screenshot => "screenshot",
            Self::PromoArt => "promo art",
            Self::Banner => "banner art",
        }
    }

    pub fn max_bytes(self) -> u64 {
        match self {
            Self::Icon => MAX_ICON_BYTES,
            _ => crate::media::MAX_MEDIA_BYTES as u64,
        }
    }

    /// Format, dimensions, aspect and byte size.
    pub fn check(
        self,
        format: MediaFormat,
        width: u32,
        height: u32,
        length: u64,
    ) -> Result<(), String> {
        let what = self.as_str();
        if length == 0 || length > self.max_bytes() {
            return Err(format!(
                "the {what} is {length} bytes; at most {} are allowed",
                self.max_bytes()
            ));
        }
        if self == Self::Icon && format == MediaFormat::Jpeg {
            return Err(format!("the {what} is a JPEG; icons are PNG or WebP"));
        }
        let pixels = u64::from(width) * u64::from(height);
        if pixels > crate::media::MAX_MEDIA_PIXELS {
            return Err(format!("the {what} has {pixels} pixels; at most 8388608"));
        }
        let ok = match self {
            Self::Icon => width == height && (256..=1024).contains(&width),
            Self::Screenshot => {
                (1280..=2560).contains(&width)
                    && (ratio_within(width, height, 16, 10) || ratio_within(width, height, 16, 9))
            }
            Self::PromoArt => (1600..=3200).contains(&width) && ratio_within(width, height, 2, 1),
            Self::Banner => (1200..=3600).contains(&width) && ratio_within(width, height, 3, 1),
        };
        if !ok {
            let rule = match self {
                Self::Icon => "square, 256 to 1024 pixels",
                Self::Screenshot => "16:10 or 16:9 (±1%), 1280 to 2560 pixels wide",
                Self::PromoArt => "2:1 (±1%), 1600 to 3200 pixels wide",
                Self::Banner => "3:1 (±1%), 1200 to 3600 pixels wide",
            };
            return Err(format!("the {what} is {width}×{height}; it must be {rule}"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_tags() {
        for ok in ["en", "pl", "pl-PL", "zh-Hant", "sr-Latn-RS"] {
            assert!(is_language_tag(ok), "{ok}");
        }
        for bad in ["", "EN", "e", "english", "en_", "en-", "en--US", "en US"] {
            assert!(!is_language_tag(bad), "{bad}");
        }
    }

    #[test]
    fn locale_fallback() {
        let text: Localized = [("en", "Hello"), ("pl", "Cześć")]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        assert_eq!(localize(&text, "pl-PL"), Some("Cześć"));
        assert_eq!(localize(&text, "pl_PL"), Some("Cześć"));
        assert_eq!(localize(&text, "pl"), Some("Cześć"));
        assert_eq!(localize(&text, "de-DE"), Some("Hello"));
        assert_eq!(localize(&Localized::new(), "en"), None);
    }

    #[test]
    fn ratios() {
        assert!(ratio_within(2560, 1600, 16, 10));
        assert!(ratio_within(1920, 1080, 16, 9));
        assert!(ratio_within(1920, 1090, 16, 9)); // 0.9% off
        assert!(!ratio_within(1920, 1100, 16, 9));
        assert!(!ratio_within(1920, 1920, 16, 9));
    }
}
