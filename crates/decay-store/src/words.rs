//! Text preparation for full-text search (DB Design §3.4).

use rusqlite::Connection;
use rusqlite::functions::FunctionFlags;

/// A URL as indexed words: the scheme is dropped, `%xx` escapes are decoded (so `café`
/// matches `caf%C3%A9`), and `/ . ? = & # : +` separate words, so "pricing" matches
/// `…/pricing?plan=pro`.
pub fn url_words(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let bytes = rest.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| char::from(b).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(h), Some(l)) =
                (bytes.get(i + 1).copied().and_then(hex), bytes.get(i + 2).copied().and_then(hex))
        {
            decoded.push((h * 16 + l) as u8);
            i += 3;
            continue;
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&decoded)
        .chars()
        .map(|c| if "/.?=&#:+".contains(c) { ' ' } else { c })
        .collect()
}

/// Registers `decay_url_words(url)`, used by the `items_fts` triggers. Every connection to
/// a Decay database needs it before writing items.
pub(crate) fn register(conn: &Connection) -> rusqlite::Result<()> {
    conn.create_scalar_function(
        "decay_url_words",
        1,
        FunctionFlags::SQLITE_UTF8
            | FunctionFlags::SQLITE_DETERMINISTIC
            | FunctionFlags::SQLITE_INNOCUOUS,
        |ctx| Ok(ctx.get::<Option<String>>(0)?.map(|u| url_words(&u))),
    )
}

/// Most words used from one query (a pasted paragraph shouldn't make search slow).
pub const MAX_QUERY_TERMS: usize = 16;

/// Words of a search query, as FTS5 prefix terms (`"pric"* AND "plan"*`): same
/// separators as the tokenizer (anything but letters, digits, `-` and `_`; leading and
/// trailing `-` / `_` dropped, so `-foo` finds `foo`), each word quoted so user text can't
/// inject FTS5 syntax, at most [`MAX_QUERY_TERMS`]. `None` when there are no words.
pub fn fts_query(text: &str) -> Option<String> {
    let terms: Vec<String> = text
        .split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_'))
        .map(|w| w.trim_matches(|c| c == '-' || c == '_'))
        .filter(|w| w.chars().any(char::is_alphanumeric))
        .take(MAX_QUERY_TERMS)
        .map(|w| format!("\"{}\"*", w.replace('"', "\"\"")))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" AND "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_urls_into_words() {
        assert_eq!(url_words("https://acme.com/pricing?plan=pro"), "acme com pricing plan pro");
        assert_eq!(url_words("https://x.org/caf%C3%A9"), "x org café");
        assert_eq!(url_words("https://x.org/100%"), "x org 100%");
        assert_eq!(url_words("chrome://settings"), "settings");
    }

    #[test]
    fn builds_safe_prefix_queries() {
        assert_eq!(fts_query("Q3 plan").as_deref(), Some(r#""Q3"* AND "plan"*"#));
        assert_eq!(fts_query("a\"b OR c*").as_deref(), Some(r#""a"* AND "b"* AND "OR"* AND "c"*"#));
        assert_eq!(fts_query("--- ..."), None);
        assert_eq!(fts_query("sub-task").as_deref(), Some(r#""sub-task"*"#));
        assert_eq!(fts_query("-foo_").as_deref(), Some(r#""foo"*"#));
        assert_eq!(fts_query("🎉 ---"), None);
        let many = fts_query(&"w ".repeat(1000)).unwrap();
        assert_eq!(many.matches(" AND ").count(), MAX_QUERY_TERMS - 1);
    }
}
