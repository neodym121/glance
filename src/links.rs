//! Finding a web link in recognized text and opening it safely. Only http and https
//! URLs are ever handed to the shell; recognized text is never executed.

use crate::error::{Error, Result};

const MAX_URL_LEN: usize = 2048;

/// OCR of monospaced text splits a scheme into tokens ("https : / /x.com"); this joins
/// them back so the link reads as one and can be opened.
pub fn repair_schemes(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let skip_spaces = |mut at: usize| {
        while lower[at..].starts_with(' ') {
            at += 1;
        }
        at
    };
    let mut out = String::with_capacity(text.len());
    let mut done = 0;
    while let Some(offset) = lower[done..].find("http") {
        let start = done + offset;
        let scheme_end = start + if lower[start + 4..].starts_with('s') { 5 } else { 4 };
        let colon = skip_spaces(scheme_end);
        let slash1 = if lower[colon..].starts_with(':') { skip_spaces(colon + 1) } else { colon };
        let slash2 = if lower[slash1..].starts_with('/') { skip_spaces(slash1 + 1) } else { slash1 };
        let host = if lower[slash2..].starts_with('/') { skip_spaces(slash2 + 1) } else { slash2 };
        let complete = colon < slash1 && slash1 < slash2 && slash2 < host;
        let next = lower[host..].chars().next();
        if complete && host - scheme_end > 3 && next.is_some_and(|c| !c.is_whitespace()) {
            out.push_str(&text[done..scheme_end]);
            out.push_str("://");
            done = host;
        } else {
            out.push_str(&text[done..start + 4]);
            done = start + 4;
        }
    }
    out.push_str(&text[done..]);
    out
}

/// Returns the first http(s) link in `text`, without trailing punctuation.
pub fn find_link(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let mut search_from = 0;
    while let Some(offset) = lower[search_from..].find("http") {
        let start = search_from + offset;
        let rest = &text[start..];
        let lower_rest = &lower[start..];
        if lower_rest.starts_with("https://") || lower_rest.starts_with("http://") {
            let end = rest
                .char_indices()
                .find(|&(_, c)| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '`' | '«' | '»'))
                .map_or(rest.len(), |(i, _)| i);
            let candidate = trim_trailing(&rest[..end]);
            if let Some(url) = validate(candidate) {
                return Some(url);
            }
        }
        search_from = start + 4;
    }
    None
}

fn trim_trailing(mut url: &str) -> &str {
    loop {
        let Some(last) = url.chars().last() else { return url };
        let unbalanced = |open: char, close: char| {
            last == close && url.matches(open).count() < url.matches(close).count()
        };
        if matches!(last, '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '…')
            || unbalanced('(', ')')
            || unbalanced('[', ']')
            || unbalanced('{', '}')
        {
            url = &url[..url.len() - last.len_utf8()];
        } else {
            return url;
        }
    }
}

/// Accepts only well-formed http(s) URLs with a host and no characters that could
/// change how the shell interprets the string.
pub fn validate(url: &str) -> Option<String> {
    if url.is_empty() || url.len() > MAX_URL_LEN {
        return None;
    }
    let lower = url.to_ascii_lowercase();
    let rest = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://"))?;
    if url.chars().any(|c| {
        c.is_control() || c.is_whitespace() || matches!(c, '"' | '<' | '>' | '\\' | '^' | '`' | '{' | '}' | '|')
    }) {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit('@').next().unwrap_or("");
    let host = if host_port.starts_with('[') {
        host_port.split(']').next().map(|h| &h[1..])?
    } else {
        host_port.split(':').next().unwrap_or("")
    };
    if host.is_empty()
        || host.starts_with('.')
        || host.starts_with('-')
        || !host.chars().all(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ':'))
    {
        return None;
    }
    Some(url.to_owned())
}

pub fn open(url: &str) -> Result<()> {
    let url = validate(url).ok_or_else(|| Error::new("This link can't be opened safely."))?;
    crate::win32::shell_open_url(&url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_links_in_text() {
        assert_eq!(find_link("see https://example.com/a?b=1."), Some("https://example.com/a?b=1".into()));
        assert_eq!(find_link("(https://example.com/x)"), Some("https://example.com/x".into()));
        assert_eq!(
            find_link("wiki https://en.wikipedia.org/wiki/Rust_(language), ok"),
            Some("https://en.wikipedia.org/wiki/Rust_(language)".into())
        );
        assert_eq!(find_link("HTTPS://Example.COM"), Some("HTTPS://Example.COM".into()));
        assert_eq!(find_link("http://localhost:8080/"), Some("http://localhost:8080/".into()));
        assert_eq!(find_link("no links here, just http and https words"), None);
        assert_eq!(find_link("broken https:// link"), None);
        assert_eq!(find_link("first https:// then https://ok.example"), Some("https://ok.example".into()));
    }

    #[test]
    fn repairs_schemes_split_by_ocr() {
        assert_eq!(repair_schemes("https : / /www.youtube.com/watch"), "https://www.youtube.com/watch");
        assert_eq!(repair_schemes("see http :// x.com now"), "see http://x.com now");
        assert_eq!(repair_schemes("HTTPS: //Example.com"), "HTTPS://Example.com");
        assert_eq!(repair_schemes("https://ok.example and https://ok.too"), "https://ok.example and https://ok.too");
        assert_eq!(repair_schemes("http and https words"), "http and https words");
        assert_eq!(repair_schemes("https : / /\nnext"), "https : / /\nnext");
        assert_eq!(repair_schemes("ссылка: https : / /пример.рф"), "ссылка: https://пример.рф");
        assert_eq!(repair_schemes("http"), "http");
        assert_eq!(find_link(&repair_schemes("https : / /www.youtube.com/")).as_deref(), Some("https://www.youtube.com/"));
    }

    #[test]
    fn rejects_other_schemes_and_odd_characters() {
        for url in [
            "file:///C:/Windows/System32/calc.exe",
            "javascript:alert(1)",
            "ms-settings:privacy",
            "https://exa mple.com",
            "https://example.com/\"onload",
            "https:///path",
            "https://.example.com",
            "https://example.com\\..\\x",
            "https://example.com/\u{0}",
            "ftp://example.com",
        ] {
            assert_eq!(validate(url), None, "{url}");
        }
    }

    #[test]
    fn accepts_regular_urls() {
        for url in [
            "https://example.com",
            "http://example.com:8080/path?q=1#frag",
            "https://user@example.com/",
            "https://[::1]:443/",
            "https://пример.рф/путь",
        ] {
            assert_eq!(validate(url).as_deref(), Some(url), "{url}");
        }
    }
}
