// SPDX-License-Identifier: MPL-2.0
//! Conservative local text recognition; never repairs domains or performs I/O.
//! URL parsing is delegated to url; mailto/tel follow RFC 6068 / RFC 3966.
use crate::code_decode::{CodeFormat, DecodedCode, MAX_CODES, MAX_TEXT_BYTES};
use regex::{Regex, RegexBuilder};
use std::{collections::HashSet, sync::OnceLock};

const MAX_INPUT: usize = 128 * 1024;
const MAX_CANDIDATES: usize = 512;
fn expressions() -> &'static [Regex; 3] {
    static EXPRESSIONS: OnceLock<[Regex; 3]> = OnceLock::new();
    EXPRESSIONS.get_or_init(|| {
        [r#"(?i)https?://[^\s<>"'，。；：！？、（）【】]+"#,
         r"[A-Za-z0-9!#$%&'*+/=?^_`{|}~-]+(?:\.[A-Za-z0-9!#$%&'*+/=?^_`{|}~-]+)*@[A-Za-z0-9](?:[A-Za-z0-9.-]*[A-Za-z0-9])?\.[A-Za-z]{2,63}",
         r"(?:\+[0-9](?:[ -]?[0-9]){6,14}|1[3-9](?:[ -]?[0-9]){9})"]
        .map(|pattern| RegexBuilder::new(pattern).size_limit(1024 * 1024).dfa_size_limit(256 * 1024).build().expect("fixed entity expression"))
    })
}
fn adjacent(text: &str, start: usize, end: usize, extra: &str) -> bool {
    let forbidden = |c: char| c.is_ascii_alphanumeric() || extra.contains(c);
    text[..start].chars().next_back().is_some_and(forbidden)
        || text[end..].chars().next().is_some_and(forbidden)
}
fn url_candidate(mut text: &str) -> &str {
    let pairs = [('(', ')'), ('[', ']'), ('{', '}')];
    let mut excess = pairs.map(|(opening, closing)| {
        text.chars()
            .filter(|&c| c == closing)
            .count()
            .saturating_sub(text.chars().filter(|&c| c == opening).count())
    });
    loop {
        let previous = text;
        text = text.trim_end_matches([',', '.', ';', '!', '?']);
        for (index, (_, closing)) in pairs.into_iter().enumerate() {
            if text.ends_with(closing) && excess[index] > 0 {
                text = &text[..text.len() - closing.len_utf8()];
                excess[index] -= 1;
            }
        }
        if text == previous {
            return text;
        }
    }
}
pub fn email_address(text: &str) -> bool {
    if text.len() > 254 || !text.is_ascii() {
        return false;
    }
    let Some((local, domain)) = text.rsplit_once('@') else {
        return false;
    };
    if local.is_empty()
        || local.len() > 64
        || local.split('.').any(|part| part.is_empty())
        || !local
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".!#$%&'*+/=?^_`{|}~-".contains(&b))
    {
        return false;
    }
    let labels: Vec<_> = domain.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
        && labels
            .last()
            .is_some_and(|label| label.len() >= 2 && label.bytes().all(|b| b.is_ascii_alphabetic()))
}
pub fn phone_number(text: &str) -> Option<String> {
    if text.len() > 40
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || b"+ -".contains(&b))
    {
        return None;
    }
    let digits: String = text.chars().filter(|&c| c != ' ' && c != '-').collect();
    if let Some(global) = digits.strip_prefix('+') {
        if (7..=15).contains(&global.len())
            && !global.starts_with('0')
            && global.bytes().all(|b| b.is_ascii_digit())
        {
            return Some(digits);
        }
    } else if digits.len() == 11
        && digits.starts_with('1')
        && matches!(digits.as_bytes()[1], b'3'..=b'9')
        && digits.bytes().all(|b| b.is_ascii_digit())
    {
        return Some(format!("+86{digits}"));
    }
    None
}
pub fn recognize(text: &str) -> Vec<DecodedCode> {
    if text.len() > MAX_INPUT {
        return vec![];
    }
    let mut found = vec![];
    let mut urls = vec![];
    for matched in expressions()[0].find_iter(text).take(MAX_CANDIDATES) {
        if text[..matched.start()]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_alphanumeric() || "_@".contains(c))
        {
            continue;
        }
        let value = url_candidate(matched.as_str());
        // Even rejected URLs own their span; do not reinterpret credential or
        // query text within them as an independent email/telephone action.
        urls.push((matched.start(), matched.end()));
        if crate::code_open::http_url(value).is_some() {
            found.push((matched.start(), CodeFormat::Url, value.to_owned()));
        }
    }
    for (kind, expression, extra) in [
        (CodeFormat::Email, &expressions()[1], "._@+-"),
        (CodeFormat::Phone, &expressions()[2], "+-"),
    ] {
        for matched in expression.find_iter(text).take(MAX_CANDIDATES) {
            let end = if kind == CodeFormat::Email
                && text[matched.end()..].starts_with('.')
                && !text[matched.end() + 1..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || "_-.".contains(c))
            {
                matched.end() + 1
            } else {
                matched.end()
            };
            if urls
                .iter()
                .any(|&(start, end)| matched.start() >= start && matched.start() < end)
                || adjacent(text, matched.start(), end, extra)
            {
                continue;
            }
            let value = matched.as_str();
            let normalized = match kind {
                CodeFormat::Email if email_address(value) => Some(value.to_owned()),
                CodeFormat::Phone => phone_number(value),
                _ => None,
            };
            if let Some(value) = normalized {
                found.push((matched.start(), kind, value));
            }
        }
    }
    found.sort_by_key(|value| value.0);
    let mut seen = HashSet::new();
    found
        .into_iter()
        .filter_map(|(_, format, text)| {
            let identity = match format {
                CodeFormat::Email => {
                    let (local, domain) = text.rsplit_once('@').unwrap();
                    format!("mail:{local}@{}", domain.to_ascii_lowercase())
                }
                CodeFormat::Phone => format!("tel:{text}"),
                _ => format!("url:{text}"),
            };
            (text.len() <= MAX_TEXT_BYTES && seen.insert(identity))
                .then_some(DecodedCode { format, text })
        })
        .take(MAX_CODES)
        .collect()
}
/// Only a recipient is included: reserved characters cannot inject headers.
pub fn mailto(text: &str) -> Option<String> {
    if !email_address(text) {
        return None;
    }
    let mut out = String::from("mailto:");
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~@".contains(&byte) {
            out.push(byte as char);
        } else {
            use std::fmt::Write;
            let _ = write!(out, "%{byte:02X}");
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_full_domains_paths_and_balanced_url_punctuation() {
        let values = recognize("(https://example.org/A_(B)). https://[::1]/x user@gmail.com.evil USER@Example.org. +44 20 7946 0958, 138-0013-8000");
        assert_eq!(
            values.iter().map(|v| v.text.as_str()).collect::<Vec<_>>(),
            vec![
                "https://example.org/A_(B)",
                "https://[::1]/x",
                "user@gmail.com.evil",
                "USER@Example.org",
                "+442079460958",
                "+8613800138000"
            ]
        );
    }
    #[test]
    fn rejects_embedded_tokens_and_unsafe_schemes_without_repairing_ocr() {
        for text in [
            "file:///C:/x",
            "ftp://example.com",
            "https://user:pass@example.com",
            "x13800138000x",
            "991380013800099",
            "a..b@example.com",
            "a@example.com.evil_",
            "javascript:alert(1)",
        ] {
            assert!(recognize(text).is_empty(), "{text}");
        }
        assert!(
            recognize("https://example.com/?mail=user@example.com&phone=13800138000").len() == 1
        );
        assert!(!email_address("a@example.com?subject=hi"));
        assert!(!email_address("a\r\n@example.com"));
        assert!(phone_number("+123;ext=4").is_none());
        assert!(phone_number("++12345678").is_none());
    }
    #[test]
    fn encodes_recipient_reserved_bytes_and_bounds_deduplicated_results() {
        assert_eq!(
            mailto("a?b&c=d+e@example.org").unwrap(),
            "mailto:a%3Fb%26c%3Dd%2Be@example.org"
        );
        let values = recognize("a@Example.org a@example.org https://example.org/A https://example.org/a 13800138000 +86 138 0013 8000");
        assert_eq!(values.len(), 4);
        assert!(recognize(&"a".repeat(MAX_INPUT + 1)).is_empty());
        let text = (0..100)
            .map(|n| format!("a{n}@example.com "))
            .collect::<String>();
        assert_eq!(recognize(&text).len(), MAX_CODES);
    }
}
