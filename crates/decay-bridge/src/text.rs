//! Decoding messages from the browser. Page titles can hold lone UTF-16 surrogates (half
//! of an emoji cut by a site), which JSON can carry as `\ud83d` escapes but Rust strings
//! can't. They become U+FFFD, as Chrome itself does when it converts strings to UTF-8;
//! invalid UTF-8 bytes do too. Nothing else changes.

use std::borrow::Cow;

use crate::protocol::Envelope;

const fn hex_value(b: u8) -> Option<u16> {
    match b {
        b'0'..=b'9' => Some((b - b'0') as u16),
        b'a'..=b'f' => Some((b - b'a' + 10) as u16),
        b'A'..=b'F' => Some((b - b'A' + 10) as u16),
        _ => None,
    }
}

/// The code unit of a `\uXXXX` escape starting at `i` (the backslash).
fn escape_at(s: &[u8], i: usize) -> Option<u16> {
    if s.get(i) != Some(&b'\\') || s.get(i + 1) != Some(&b'u') {
        return None;
    }
    s.get(i + 2..i + 6)?.iter().try_fold(0u16, |acc, &b| Some(acc * 16 + hex_value(b)?))
}

/// JSON text with lone-surrogate escapes replaced by `\uFFFD`.
pub fn sanitize_surrogates(json: &str) -> Cow<'_, str> {
    let s = json.as_bytes();
    if !json.contains("\\u") {
        return Cow::Borrowed(json);
    }
    let mut out: Option<String> = None;
    let mut copied = 0;
    let mut i = 0;
    while i < s.len() {
        if s[i] != b'\\' {
            i += 1;
            continue;
        }
        let Some(unit) = escape_at(s, i) else {
            i += 2; // another escape (`\\`, `\"`, `\n`, …): skip both bytes
            continue;
        };
        let high = (0xD800..0xDC00).contains(&unit);
        let low = (0xDC00..0xE000).contains(&unit);
        let paired = high && escape_at(s, i + 6).is_some_and(|n| (0xDC00..0xE000).contains(&n));
        if paired {
            i += 12;
        } else if high || low {
            let buf = out.get_or_insert_with(|| String::with_capacity(json.len()));
            buf.push_str(&json[copied..i]);
            buf.push_str("\\uFFFD");
            i += 6;
            copied = i;
        } else {
            i += 6;
        }
    }
    match out {
        Some(mut buf) => {
            buf.push_str(&json[copied..]);
            Cow::Owned(buf)
        }
        None => Cow::Borrowed(json),
    }
}

/// Parses an envelope from raw bytes: invalid UTF-8 and lone surrogates become U+FFFD.
pub fn decode_envelope(bytes: &[u8]) -> Result<Envelope, serde_json::Error> {
    let text = String::from_utf8_lossy(bytes);
    serde_json::from_str(&sanitize_surrogates(&text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_only_lone_surrogates() {
        assert_eq!(sanitize_surrogates(r#""a\ud83d""#), r#""a\uFFFD""#);
        assert_eq!(sanitize_surrogates(r#""\ude00b""#), r#""\uFFFDb""#);
        assert_eq!(sanitize_surrogates(r#""\ud83d\ude00""#), r#""\ud83d\ude00""#);
        assert_eq!(sanitize_surrogates(r#""\\ud83d""#), r#""\\ud83d""#, "an escaped backslash");
        assert_eq!(sanitize_surrogates(r#""\ud83d\ud83d\ude00""#), r#""\uFFFD\ud83d\ude00""#);
        assert!(matches!(sanitize_surrogates(r#""\u00e9""#), Cow::Borrowed(_)));
    }

    #[test]
    fn decodes_titles_with_broken_text() {
        let json = r#"{"v":1,"id":"x","type":"tab.updated","payload":{"title":"Party \ud83d"}}"#;
        let env = decode_envelope(json.as_bytes()).unwrap();
        assert_eq!(env.payload["title"], "Party \u{FFFD}");
        let mut bytes = br#"{"v":1,"id":"x","type":"t","payload":"a"#.to_vec();
        bytes.extend_from_slice(&[0xED, 0xA0, 0xBD]); // a surrogate in (invalid) UTF-8
        bytes.extend_from_slice(br#""}"#);
        let env = decode_envelope(&bytes).unwrap();
        assert_eq!(env.payload, "a\u{FFFD}\u{FFFD}\u{FFFD}");
    }
}
