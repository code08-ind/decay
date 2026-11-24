//! The keyword labeller (F15 fallback: no on-device model): a batch's name from the words
//! its titles share and the site most of it came from, e.g. `github.com · Tokio, async`.
//! Deterministic, instant, nothing leaves the computer.

use std::collections::HashMap;

use decay_store::BatchItem;

/// Words that say nothing about what a batch is about.
const STOP: &[&str] = &[
    "the", "and", "for", "with", "from", "your", "you", "are", "was", "how", "what", "why", "when",
    "who", "this", "that", "into", "about", "new", "all", "not", "but", "can", "our", "out", "get",
    "via", "per", "its", "has", "have", "will", "home", "page", "login", "sign", "inbox",
    "untitled", "tab", "www", "http", "https", "com", "org", "net", "html", "pdf", "der", "die",
    "das", "und", "les", "des", "une", "los", "las", "del",
];

/// At most this many words in a label.
const MAX_WORDS: usize = 3;
/// Labels are cut to this many characters.
pub const MAX_LABEL: usize = 80;

/// The site without `www.`.
fn site(domain: &str) -> &str {
    domain.strip_prefix("www.").unwrap_or(domain)
}

/// Words worth keeping from a title (3+ letters, not a stop word, not the name of a site
/// in `sites`).
fn words<'a>(title: &'a str, sites: &[&str]) -> Vec<&'a str> {
    title
        .split(|c: char| !c.is_alphanumeric() && c != '\'' && c != '+' && c != '#')
        .map(|w| w.trim_matches('\''))
        .filter(|w| w.chars().count() >= 3 && w.chars().any(char::is_alphabetic))
        .filter(|w| !STOP.contains(&w.to_lowercase().as_str()))
        .filter(|w| {
            let lower = w.to_lowercase();
            !sites.iter().any(|s| s.split('.').next().is_some_and(|name| name == lower))
        })
        .collect()
}

/// A label for `items`, or `None` when they share nothing worth naming.
pub fn label(items: &[BatchItem]) -> Option<String> {
    if items.is_empty() {
        return None;
    }
    // The site most of the batch came from (more than half of it).
    let mut by_site: HashMap<&str, usize> = HashMap::new();
    for d in items.iter().filter_map(|i| i.domain.as_deref()) {
        *by_site.entry(site(d)).or_default() += 1;
    }
    let main_site = by_site
        .iter()
        .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
        .filter(|(_, n)| **n * 2 > items.len())
        .map(|(s, _)| *s);
    // The shown site's own name isn't repeated as a word ("github.com · GitHub").
    let sites: Vec<&str> = main_site.into_iter().collect();

    // Words in at least two titles (each title counts once), most shared first, then in
    // order of first appearance; shown as first written.
    let mut count: HashMap<String, (usize, usize, &str)> = HashMap::new();
    let mut order = 0;
    for item in items {
        let mut seen = std::collections::HashSet::new();
        for w in words(&item.title, &sites) {
            let key = w.to_lowercase();
            if seen.insert(key.clone()) {
                let entry = count.entry(key).or_insert((0, order, w));
                entry.0 += 1;
                order += 1;
            }
        }
    }
    let mut shared: Vec<(usize, usize, &str)> =
        count.into_values().filter(|(n, ..)| *n >= 2).collect();
    shared.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let picked: Vec<&str> = shared.iter().take(MAX_WORDS).map(|(_, _, w)| *w).collect();

    let words = (!picked.is_empty()).then(|| {
        let mut first = picked.join(", ");
        if let Some(c) = first.get(..1) {
            first = c.to_uppercase() + &first[1..];
        }
        first
    });
    let text = match (main_site, words) {
        (Some(s), Some(w)) => format!("{s} · {w}"),
        (Some(s), None) => s.to_owned(),
        (None, Some(w)) => w,
        (None, None) => return None,
    };
    Some(cut(&text))
}

/// At most [`MAX_LABEL`] characters (on a character boundary).
pub fn cut(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= MAX_LABEL {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(MAX_LABEL - 1).collect();
    out = out.trim_end().to_owned();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(list: &[(&str, &str)]) -> Vec<BatchItem> {
        list.iter()
            .map(|(t, d)| BatchItem {
                title: (*t).into(),
                domain: (!d.is_empty()).then(|| (*d).to_owned()),
            })
            .collect()
    }

    #[test]
    fn names_a_batch_by_its_shared_words_and_site() {
        let b = items(&[
            ("tokio::spawn - Rust async runtime docs", "docs.rs"),
            ("Async in depth | Tokio", "tokio.rs"),
            ("tokio/tokio: A runtime for writing async apps - GitHub", "github.com"),
        ]);
        assert_eq!(label(&b).as_deref(), Some("Tokio, async, runtime"));

        let gh = items(&[
            ("Pull request #12: fix parser · decay/core", "github.com"),
            ("Issue #40: parser crash · decay/core", "github.com"),
            ("decay/core: README", "www.github.com"),
        ]);
        assert_eq!(label(&gh).as_deref(), Some("github.com · Decay, core, parser"));
    }

    #[test]
    fn a_site_alone_or_nothing() {
        let same_site = items(&[("Inbox", "mail.google.com"), ("Sign in", "mail.google.com")]);
        assert_eq!(label(&same_site).as_deref(), Some("mail.google.com"));
        let unrelated = items(&[("Weather", "a.com"), ("Recipes", "b.com"), ("Football", "c.com")]);
        assert_eq!(label(&unrelated), None);
        assert_eq!(label(&[]), None);
    }

    #[test]
    fn labels_stay_short_and_whole() {
        assert_eq!(cut("  short "), "short");
        let long = "é".repeat(100);
        let c = cut(&long);
        assert_eq!(c.chars().count(), MAX_LABEL);
        assert!(c.ends_with('…'));
    }
}
