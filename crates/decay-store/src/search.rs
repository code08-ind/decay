//! Archive search (PRD F5, Software Design §5.1, DB Design §3.4/§4): FTS5 prefix
//! matching, ranked by bm25 (title weighs most) times a recency boost
//! `1 / (1 + days_since_archived / 7)`. Grouping by day and source is the caller's job
//! (it knows the user's calendar).

use std::fmt::Write as _;

use decay_core::{ItemKind, SqlEnum};
use rusqlite::types::Value;

use crate::items::{columns_as, from_row};
use crate::{Result, Store, fts_query};

/// Explicit filters (API `search.filters`); inline operators fill the ones left empty.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchFilters {
    /// App id, or part of an app's name (`app:chrome`).
    pub app: Option<String>,
    /// A domain and its subdomains (`site:github.com`).
    pub domain: Option<String>,
    /// Archived at or after (epoch ms).
    pub after: Option<i64>,
    /// Archived before (epoch ms).
    pub before: Option<i64>,
    pub kind: Option<ItemKind>,
    pub source_id: Option<String>,
}

/// One search request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchQuery {
    /// What the user typed, operators included.
    pub text: String,
    pub filters: SearchFilters,
    /// 1–200 (0 means the default, 50).
    pub limit: usize,
    /// Results to skip (the API's cursor).
    pub offset: usize,
    pub now: i64,
    /// The user's UTC offset, for `before:` / `after:` dates (minutes east of UTC).
    pub utc_offset_minutes: i32,
}

/// One result.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub item: crate::Item,
    /// Higher is better (relevance × recency; recency alone without search words).
    pub score: f64,
}

/// A page of results.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchPage {
    pub hits: Vec<SearchHit>,
    /// Offset of the next page, if there is one.
    pub next_offset: Option<usize>,
}

const DAY_MS: i64 = 86_400_000;

/// Days since 1970-01-01 of a civil date (Howard Hinnant's days-from-civil).
pub(crate) fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// `YYYY-MM-DD` (local midnight), `7d` / `2w` (that long ago), `today`, `yesterday`.
/// `None` for anything else, including negative counts and impossible dates. One UTC
/// offset is used for every date (the caller passes today's), so a date across a DST
/// change can be an hour off: fine for day-granular filters.
fn parse_when(value: &str, now: i64, offset_min: i32) -> Option<i64> {
    // Offsets beyond ±18 h don't exist; clamping keeps the arithmetic in range.
    let offset_ms = i64::from(offset_min.clamp(-18 * 60, 18 * 60)) * 60_000;
    let now = now.clamp(0, 253_402_300_799_999); // up to 9999-12-31
    let local_midnight = |days: i64| days * DAY_MS - offset_ms;
    let today = (now + offset_ms).div_euclid(DAY_MS);
    let count = |n: &str, max: i64| n.parse::<i64>().ok().filter(|n| (0..=max).contains(n));
    match value {
        "today" => return Some(local_midnight(today)),
        "yesterday" => return Some(local_midnight(today - 1)),
        _ => {}
    }
    if let Some(n) = value.strip_suffix('d').and_then(|n| count(n, 100_000)) {
        return Some(now - n * DAY_MS);
    }
    if let Some(n) = value.strip_suffix('w').and_then(|n| count(n, 10_000)) {
        return Some(now - n * 7 * DAY_MS);
    }
    let mut parts = value.splitn(3, '-').map(|p| p.parse::<i64>().ok());
    let (Some(Some(y)), Some(Some(m)), Some(Some(d))) = (parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    ((1970..=9999).contains(&y) && (1..=12).contains(&m) && (1..=days_in_month(y, m)).contains(&d))
        .then(|| local_midnight(days_from_civil(y, m, d)))
}

/// Splits typed text into search words and inline operators: `app:`, `site:`,
/// `before:` / `after:` (see [`parse_when`]) and `in:window` / `in:tab`. An operator with a
/// value it can't use stays a search word.
pub fn parse_query(text: &str, now: i64, utc_offset_minutes: i32) -> (String, SearchFilters) {
    let mut words = Vec::new();
    let mut f = SearchFilters::default();
    for token in text.split_whitespace() {
        let Some((op, value)) = token.split_once(':') else {
            words.push(token);
            continue;
        };
        let used = !value.is_empty()
            && match op.to_ascii_lowercase().as_str() {
                "app" => {
                    f.app = Some(value.to_owned());
                    true
                }
                "site" => {
                    f.domain = Some(value.trim_start_matches("*.").to_ascii_lowercase());
                    true
                }
                "before" => {
                    parse_when(value, now, utc_offset_minutes).map(|t| f.before = Some(t)).is_some()
                }
                "after" => {
                    parse_when(value, now, utc_offset_minutes).map(|t| f.after = Some(t)).is_some()
                }
                "in" => match value.to_ascii_lowercase().as_str() {
                    "window" | "windows" => {
                        f.kind = Some(ItemKind::Window);
                        true
                    }
                    "tab" | "tabs" => {
                        f.kind = Some(ItemKind::Tab);
                        true
                    }
                    _ => false,
                },
                _ => false,
            };
        if !used {
            words.push(token);
        }
    }
    (words.join(" "), f)
}

/// `%`, `_` and `\` escaped for `LIKE … ESCAPE '\'`.
fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

impl Store {
    /// Searches archived items.
    pub fn search(&self, q: &SearchQuery) -> Result<SearchPage> {
        let (words, inline) = parse_query(&q.text, q.now, q.utc_offset_minutes);
        let f = SearchFilters {
            app: q.filters.app.clone().or(inline.app),
            domain: q.filters.domain.as_deref().map(str::to_lowercase).or(inline.domain),
            after: q.filters.after.or(inline.after),
            before: q.filters.before.or(inline.before),
            kind: q.filters.kind.or(inline.kind),
            source_id: q.filters.source_id.clone().or(inline.source_id),
        };
        let limit = if q.limit == 0 { 50 } else { q.limit.min(200) };

        let mut args: Vec<Value> = vec![Value::Integer(q.now)];
        let mut sql = String::new();
        let cols = columns_as("i");
        // Relevance: bm25 is negative, more negative = better. Recency never goes below
        // "now": an item dated in the future (clock moved back) counts as just archived.
        let recency = "(1.0 + max(0, ?1 - coalesce(i.archived_at, i.state_changed_at)) \
                       / 86400000.0 / 7.0)";
        let terms = fts_query(&words);
        if terms.is_none() && !words.trim().is_empty() {
            // Typed something, but nothing searchable (only emoji or punctuation).
            return Ok(SearchPage { hits: Vec::new(), next_offset: None });
        }
        if let Some(m) = terms {
            args.push(Value::Text(m));
            let _ = write!(
                sql,
                "SELECT {cols}, (-bm25(items_fts, 10.0, 3.0, 4.0, 2.0, 2.0, 1.0)) / {recency} AS score
                 FROM items_fts JOIN items i ON i.seq = items_fts.rowid
                 WHERE items_fts MATCH ?2 AND i.state = 'archived'"
            );
        } else {
            let _ = write!(
                sql,
                "SELECT {cols}, 1.0 / {recency} AS score FROM items i WHERE i.state = 'archived'"
            );
        }
        let mut push = |clause: &str, value: Value| {
            args.push(value);
            sql.push_str(&clause.replace('?', &format!("?{}", args.len())));
        };
        if let Some(app) = f.app {
            let pattern = format!("%{}%", like_escape(&app));
            push(" AND (i.app_id = ?", Value::Text(app));
            push(
                " OR i.app_id IN (SELECT id FROM apps WHERE name LIKE ? ESCAPE '\\'))",
                Value::Text(pattern),
            );
        }
        if let Some(domain) = f.domain {
            let pattern = format!("%.{}", like_escape(&domain));
            push(" AND (i.domain = ?", Value::Text(domain));
            push(" OR i.domain LIKE ? ESCAPE '\\')", Value::Text(pattern));
        }
        if let Some(after) = f.after {
            push(" AND i.archived_at >= ?", Value::Integer(after));
        }
        if let Some(before) = f.before {
            push(" AND i.archived_at < ?", Value::Integer(before));
        }
        if let Some(kind) = f.kind {
            push(" AND i.kind = ?", Value::Text(kind.as_sql().to_owned()));
        }
        if let Some(source) = f.source_id {
            push(" AND i.source_id = ?", Value::Text(source));
        }
        let int = |n: usize| Value::Integer(i64::try_from(n).unwrap_or(i64::MAX));
        push(" ORDER BY score DESC, i.archived_at DESC LIMIT ?", int(limit + 1));
        push(" OFFSET ?", int(q.offset));

        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query(rusqlite::params_from_iter(args))?;
        let mut hits = Vec::new();
        while let Some(row) = rows.next()? {
            hits.push(SearchHit { item: from_row(row)?, score: row.get(26)? });
        }
        let next_offset = (hits.len() > limit).then(|| q.offset.saturating_add(limit));
        hits.truncate(limit);
        Ok(SearchPage { hits, next_offset })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::Item;
    use crate::new_id;
    use crate::test_util::*;
    use decay_core::ItemState;
    use std::time::{Duration, Instant};

    fn archived(store: &Store, title: &str, url: &str, domain: &str, days_ago: i64) -> Item {
        let mut item = Item::new_live(new_id(NOW), ItemKind::Tab, "com.google.Chrome", NOW);
        item.title = title.into();
        item.url = Some(url.into());
        item.domain = Some(domain.into());
        item.state = ItemState::Archived;
        item.archived_at = Some(NOW - days_ago * DAY_MS);
        store.insert_item(&item).unwrap();
        item
    }

    fn query(text: &str) -> SearchQuery {
        SearchQuery {
            text: text.into(),
            filters: SearchFilters::default(),
            limit: 0,
            offset: 0,
            now: NOW,
            utc_offset_minutes: 0,
        }
    }

    fn titles(store: &Store, text: &str) -> Vec<String> {
        store.search(&query(text)).unwrap().hits.into_iter().map(|h| h.item.title).collect()
    }

    #[test]
    fn finds_by_prefix_url_words_and_accents() {
        let (store, _dir) = store();
        archived(&store, "Acme — Plans", "https://acme.com/pricing?plan=pro", "acme.com", 1);
        archived(&store, "Résumé draft", "https://docs.example.com/d/1", "example.com", 2);
        archived(&store, "Unrelated", "https://other.org/", "other.org", 1);
        assert_eq!(titles(&store, "pric"), ["Acme — Plans"]);
        assert_eq!(titles(&store, "resume"), ["Résumé draft"]);
        assert_eq!(titles(&store, "chrome docs"), ["Résumé draft"], "app name is indexed");
        assert_eq!(titles(&store, "\"OR\" NEAR(").len(), 0, "FTS syntax is quoted away");
    }

    #[test]
    fn ranks_title_matches_first_and_weighs_recency() {
        let (store, _dir) = store();
        archived(&store, "Notes", "https://x.com/roadmap", "x.com", 1);
        archived(&store, "Roadmap", "https://y.com/", "y.com", 1);
        archived(&store, "Roadmap (old)", "https://z.com/", "z.com", 60);
        // Same age: the title match wins. Two months old (boost 1 / (1 + 60/7) ≈ 0.1), a
        // title match drops below yesterday's URL match.
        assert_eq!(titles(&store, "roadmap"), ["Roadmap", "Notes", "Roadmap (old)"]);
    }

    #[test]
    fn applies_inline_operators_and_filters() {
        let (store, _dir) = store();
        archived(&store, "Repo", "https://github.com/a", "github.com", 1);
        archived(&store, "Gist", "https://gist.github.com/b", "gist.github.com", 10);
        archived(&store, "Repo mirror", "https://notgithub.com/c", "notgithub.com", 1);
        let mut live = Item::new_live(new_id(NOW), ItemKind::Tab, "com.google.Chrome", NOW);
        live.title = "Repo live".into();
        store.insert_item(&live).unwrap();

        assert_eq!(titles(&store, "site:github.com"), ["Repo", "Gist"]);
        assert_eq!(titles(&store, "repo site:github.com"), ["Repo"]);
        let mut recent = titles(&store, "after:3d");
        recent.sort();
        assert_eq!(recent, ["Repo", "Repo mirror"]);
        assert_eq!(titles(&store, "before:2026-10-30"), ["Gist"]);
        assert_eq!(titles(&store, "app:chrome in:tab gist"), ["Gist"]);
        assert_eq!(titles(&store, "app:figma").len(), 0);
        assert_eq!(titles(&store, "in:window").len(), 0);
        assert_eq!(titles(&store, "in:bogus").len(), 0, "kept as a search word");
    }

    #[test]
    fn pages_through_results() {
        let (store, _dir) = store();
        for i in 0..5 {
            archived(&store, &format!("Page {i}"), "https://p.com/", "p.com", i);
        }
        let mut q = query("page");
        q.limit = 2;
        let first = store.search(&q).unwrap();
        assert_eq!(first.next_offset, Some(2));
        q.offset = 4;
        let last = store.search(&q).unwrap();
        assert_eq!(last.hits.len(), 1);
        assert_eq!(last.next_offset, None);
    }

    #[test]
    fn hostile_input_never_panics_or_breaks_search() {
        let (store, _dir) = store();
        archived(&store, "Fresh", "https://a.com/", "a.com", 1);
        let future = archived(&store, "Skewed", "https://b.com/", "b.com", -7); // clock moved back
        let mut q = query("");
        q.offset = usize::MAX;
        assert_eq!(store.search(&q).unwrap().hits.len(), 0);
        let page = store.search(&query("")).unwrap();
        assert_eq!(page.hits.len(), 2, "an item dated a week ahead doesn't break search");
        assert!(page.hits.iter().all(|h| h.score.is_finite() && h.score > 0.0));
        assert_eq!(page.hits[0].item.id, future.id, "it counts as just archived");
        for text in [
            "after:-9223372036854775808d",
            "before:-9223372036854775808w",
            "after:99999999999999999999d",
            "before:-5d",
            "after:2026-02-30",
        ] {
            let (words, f) = parse_query(text, NOW, 0);
            assert_eq!((words.as_str(), f.after, f.before), (text, None, None), "{text}");
            store.search(&query(text)).unwrap();
        }
        let mut q = query("x");
        q.now = i64::MAX;
        q.utc_offset_minutes = i32::MIN;
        store.search(&q).unwrap();
        assert!(parse_query("after:today", i64::MAX, i32::MAX).1.after.is_some());
        // Only emoji or punctuation: nothing, not the whole archive.
        assert_eq!(titles(&store, "🎉").len(), 0);
        assert_eq!(titles(&store, "---").len(), 0);
        // An explicit domain filter is case-insensitive like `site:`.
        let mut q = query("");
        q.filters.domain = Some("A.COM".into());
        assert_eq!(store.search(&q).unwrap().hits.len(), 1);
    }

    #[test]
    fn parses_dates_in_the_users_time_zone() {
        // 2026-11-04 00:00 in UTC+5:30 is 2026-11-03 18:30 UTC.
        let (_, f) = parse_query("after:2026-11-04", NOW, 330);
        assert_eq!(f.after, Some(NOW - 330 * 60_000));
        let (words, f) = parse_query("x before:soon", NOW, 0);
        assert_eq!((words.as_str(), f.before), ("x before:soon", None));
        assert_eq!(parse_query("after:today", NOW + 5, 0).1.after, Some(NOW));
        assert!(parse_query("after:2028-02-29", NOW, 0).1.after.is_some(), "leap day");
        assert_eq!(parse_query("after:2026-02-29", NOW, 0).1.after, None);
    }

    #[test]
    fn stays_fast_at_50k_archived_items() {
        let (mut store, _dir) = store();
        let words = ["pricing", "roadmap", "invoice", "design", "sprint", "hiring", "budget"];
        store
            .transaction(|tx| {
                let mut item =
                    Item::new_live(String::new(), ItemKind::Tab, "com.google.Chrome", NOW);
                item.state = ItemState::Archived;
                for i in 0..50_000i64 {
                    let w = words[(i % 7) as usize];
                    let v = words[((i / 7) % 7) as usize];
                    item.id = format!("item-{i}");
                    item.title = format!("{w} {v} document {i}");
                    item.url = Some(format!("https://site{}.com/{v}/{i}", i % 500));
                    item.domain = Some(format!("site{}.com", i % 500));
                    item.archived_at = Some(NOW - (i % 365) * DAY_MS);
                    crate::items::insert_in(tx, &item)?;
                }
                Ok(())
            })
            .unwrap();
        // Includes the slowest kinds: a one-letter prefix (not in the prefix index) and a
        // word that matches every row.
        for text in [
            "pric",
            "roadmap invoice",
            "site:site42.com design",
            "document 4999",
            "d",
            "document",
            "",
        ] {
            let start = Instant::now();
            let page = store.search(&query(text)).unwrap();
            let took = start.elapsed();
            assert_ne!(page.hits.len(), 0, "{text}");
            // PRD F5: < 50 ms at 50k items (release); debug builds get headroom.
            let limit = if cfg!(debug_assertions) { 5_000 } else { 50 };
            assert!(took < Duration::from_millis(limit), "{text}: {took:?}");
        }
    }
}
