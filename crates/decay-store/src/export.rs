//! Export and import files (P2.5, PRD F20, API Design §2.6):
//!
//! - **JSON** (full fidelity): `{ format: "decay-archive", version: 1, exportedAt, items,
//!   rules?, settings? }`, in exactly the shape [`Store::import_archive`] reads, so an
//!   export imports back losslessly (the extension's Phase 0 export is read the same way).
//!   Items are the archived and restored ones; live tabs and windows aren't exported.
//! - **HTML bookmarks** (Netscape format, what every browser imports): archived web pages
//!   in one folder per local day archived (each item's own offset, so DST is right);
//!   titles on one line (browsers read the file line by line).
//! - **CSV**: one row per archived or restored item, for spreadsheets; times as local
//!   `YYYY-MM-DD HH:MM:SS`, which spreadsheets read as dates. Cells that a spreadsheet
//!   would run as a formula (`= + - @`, tab, CR) are prefixed with `'`.
//! - **Import from bookmarks HTML**: each web link becomes an archived tab (first seen at
//!   `ADD_DATE`, in the archive from the import), then goes through the normal importer.

use std::fmt::Write as _;

use decay_core::SqlEnum;
use jiff::tz::TimeZone;
use serde_json::{Map, Value, json};

use crate::items::Item;
use crate::{Result, Store};

/// The settings an export carries (not the mode, shadow week or a hold: those are this
/// installation's moment, not preferences).
const EXPORTED_SETTINGS: [(&str, &str); 8] = [
    ("preset", "preset"),
    ("thresholds", "thresholds"),
    ("idle_cutoff_s", "idleCutoffS"),
    ("max_items_per_sweep", "maxItemsPerSweep"),
    ("retention_days", "retentionDays"),
    ("theme", "theme"),
    ("notifications", "notifications"),
    ("locale", "locale"),
];

/// At most this many links are read from a bookmarks file.
pub const MAX_BOOKMARKS: usize = 100_000;

/// Longest URL a bookmark (or an imported item) may have.
pub const MAX_URL_LEN: usize = 8192;

/// One item in the importer's shape.
fn item_json(i: &Item) -> Value {
    json!({
        "id": i.id,
        "kind": i.kind.as_sql(),
        "appId": i.app_id,
        "title": i.title,
        "url": i.url,
        "domain": i.domain,
        "path": i.path,
        "tabIndex": i.tab_index,
        "groupTitle": i.group_title,
        "groupColor": i.group_color,
        "pinned": i.pinned,
        "state": i.state.as_sql(),
        "activeAgeS": i.active_age_s,
        "firstSeenAt": i.first_seen_at,
        "lastUsedAt": i.last_used_at,
        "stateChangedAt": i.state_changed_at,
        "archivedAt": i.archived_at,
        "archiveReason": i.archive_reason,
        "restoredFromId": i.restored_from_id,
        "faviconUrl": i.favicon_url,
        "graceUntil": i.grace_until,
        "restoreCount": i.restore_count,
    })
}

/// HTML text / attribute escaping.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        // `&name;` with a short name, decoded if known; anything else stays as written.
        let decoded = rest.find(';').filter(|e| *e <= 10).and_then(|e| {
            let entity = &rest[1..e];
            let c = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                e if e.starts_with("#x") || e.starts_with("#X") => {
                    u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32)
                }
                e if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
                _ => None,
            };
            c.map(|c| (c, e))
        });
        if let Some((c, end)) = decoded {
            out.push(c);
            rest = &rest[end + 1..];
        } else {
            out.push('&');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

/// A CSV cell (RFC 4180 quoting; a leading formula character is neutralised).
fn csv_cell(s: &str) -> String {
    let s = if s.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{s}")
    } else {
        s.to_owned()
    };
    if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s }
}

/// `YYYY-MM-DD` in `tz` of an epoch-ms time.
fn day(ms: i64, tz: &TimeZone) -> String {
    jiff::Timestamp::from_millisecond(ms)
        .map(|t| t.to_zoned(tz.clone()).date().to_string())
        .unwrap_or_default()
}

/// `YYYY-MM-DD HH:MM:SS` in `tz` (what spreadsheets read as a date and time).
fn local_time(ms: i64, tz: &TimeZone) -> String {
    jiff::Timestamp::from_millisecond(ms)
        .map(|t| t.to_zoned(tz.clone()).strftime("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default()
}

/// Whitespace and control characters collapsed to single spaces.
pub(crate) fn one_line(s: &str) -> String {
    s.split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `http(s)://…` with the scheme lowercased, or `None` for any other address.
fn web_url(u: &str) -> Option<String> {
    let lower = u.get(..8).unwrap_or(u).to_ascii_lowercase();
    let scheme = if lower.starts_with("https://") {
        8
    } else if lower.starts_with("http://") {
        7
    } else {
        return None;
    };
    Some(format!("{}{}", &lower[..scheme], &u[scheme..]))
}

impl Store {
    /// Archived and restored items, newest archived first (an item without an archive
    /// time sorts by its last state change, as the bookmarks' day folders group it).
    pub fn exportable_items(&self) -> Result<Vec<Item>> {
        let sql = format!(
            "SELECT {} FROM items WHERE state IN ('archived', 'restored')
             ORDER BY coalesce(archived_at, state_changed_at) DESC, id",
            crate::items::COLUMNS
        );
        crate::items::query_items(&self.conn, &sql, [])
    }

    /// The browser of the most recently archived tab (where imported bookmarks reopen
    /// when no browser is connected).
    pub fn latest_tab_app(&self) -> Result<Option<String>> {
        use rusqlite::OptionalExtension as _;
        Ok(self
            .conn
            .query_row(
                "SELECT app_id FROM items WHERE kind = 'tab'
                 ORDER BY coalesce(archived_at, state_changed_at) DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// The JSON export (see the module docs). Returns the document and the item count.
    pub fn export_json(&self, rules: bool, settings: bool, now: i64) -> Result<(Value, usize)> {
        let items: Vec<Value> = self.exportable_items()?.iter().map(item_json).collect();
        let count = items.len();
        let mut doc = Map::new();
        doc.insert("format".into(), json!("decay-archive"));
        doc.insert("version".into(), json!(1));
        doc.insert("exportedAt".into(), json!(now));
        doc.insert("items".into(), Value::Array(items));
        if rules {
            let all = self.rules()?;
            doc.insert("rules".into(), serde_json::to_value(all).unwrap_or(Value::Null));
        }
        if settings {
            let mut out = Map::new();
            for (key, ext) in EXPORTED_SETTINGS {
                if let Some(v) = self.setting_value(key)? {
                    out.insert(ext.into(), v);
                }
            }
            doc.insert("settings".into(), Value::Object(out));
        }
        Ok((Value::Object(doc), count))
    }

    /// Netscape bookmarks: archived web pages, one folder per local day archived.
    pub fn export_bookmarks(&self, tz: &TimeZone) -> Result<(String, usize)> {
        let mut out = String::from(
            "<!DOCTYPE NETSCAPE-Bookmark-file-1>\n<!-- This is an automatically generated file. -->\n\
             <META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html; charset=UTF-8\">\n\
             <TITLE>Bookmarks</TITLE>\n<H1>Bookmarks</H1>\n<DL><p>\n    <DT><H3>Decay archive</H3>\n    <DL><p>\n",
        );
        let mut count = 0;
        let mut current_day: Option<String> = None;
        for item in self.exportable_items()? {
            let Some(url) = item.url.as_deref().and_then(web_url) else { continue };
            let at = item.archived_at.unwrap_or(item.state_changed_at);
            let d = day(at, tz);
            if current_day.as_deref() != Some(d.as_str()) {
                if current_day.is_some() {
                    out.push_str("        </DL><p>\n");
                }
                let _ = writeln!(out, "        <DT><H3>{}</H3>\n        <DL><p>", escape(&d));
                current_day = Some(d);
            }
            let title = one_line(&item.title);
            let title = if title.is_empty() { url.clone() } else { title };
            let _ = writeln!(
                out,
                "            <DT><A HREF=\"{}\" ADD_DATE=\"{}\">{}</A>",
                escape(&one_line(&url)),
                at.div_euclid(1000),
                escape(&title)
            );
            count += 1;
        }
        if current_day.is_some() {
            out.push_str("        </DL><p>\n");
        }
        out.push_str("    </DL><p>\n</DL><p>\n");
        Ok((out, count))
    }

    /// CSV: one row per archived or restored item (times in `tz`).
    pub fn export_csv(&self, tz: &TimeZone) -> Result<(String, usize)> {
        let mut out = String::from("title,url,domain,kind,app,state,archived_at,reason\r\n");
        let items = self.exportable_items()?;
        for i in &items {
            let reason = i
                .archive_reason
                .as_ref()
                .and_then(|r| serde_json::to_value(r).ok())
                .and_then(|v| v.get("type").and_then(Value::as_str).map(str::to_owned))
                .unwrap_or_default();
            let cells = [
                csv_cell(&i.title),
                csv_cell(i.url.as_deref().or(i.path.as_deref()).unwrap_or("")),
                csv_cell(i.domain.as_deref().unwrap_or("")),
                csv_cell(i.kind.as_sql()),
                csv_cell(&i.app_id),
                csv_cell(i.state.as_sql()),
                csv_cell(&i.archived_at.map(|t| local_time(t, tz)).unwrap_or_default()),
                csv_cell(&reason),
            ];
            out.push_str(&cells.join(","));
            out.push_str("\r\n");
        }
        Ok((out, items.len()))
    }
}

/// Web links in a Netscape bookmarks file, as importer items (archived tabs of
/// `app_id`), with ids derived from the URL and date. Import them with
/// [`crate::FileOptions::bookmarks`] set: an id or URL already in the archive is left
/// alone, so importing the same file twice adds and changes nothing. Returns the items
/// and whether the file had more than [`MAX_BOOKMARKS`] links (the rest aren't read).
pub fn bookmarks_to_items(html: &str, app_id: &str, now: i64) -> (Vec<Value>, bool) {
    let mut items = Vec::new();
    let lower = html.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut from = 0;
    while let Some(start) = lower[from..].find("<a").map(|i| i + from) {
        from = start + 2;
        // `<a` then whitespace (not `<abbr`, `<address>`).
        if !bytes.get(start + 2).is_some_and(u8::is_ascii_whitespace) {
            continue;
        }
        let Some((attrs, tag_end)) = tag_attrs(html, start + 2) else { break };
        // The text runs to `</a`, or to the next `<a ` when a link was left unclosed.
        let next = lower[tag_end..].find("<a").map_or(lower.len(), |i| i + tag_end);
        let text_end = lower[tag_end..next].find("</a").map_or(next, |i| i + tag_end);
        from = text_end.max(tag_end);
        let attr = |name: &str| attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
        let Some(url) = attr("href")
            .map(str::trim)
            .filter(|u| u.len() <= MAX_URL_LEN && !u.chars().any(char::is_control))
            .and_then(web_url)
        else {
            continue;
        };
        if items.len() >= MAX_BOOKMARKS {
            return (items, true);
        }
        let raw_date = attr("add_date").unwrap_or_default();
        let added = raw_date
            .trim()
            .parse::<i64>()
            .ok()
            .map(|s| if s > 100_000_000_000 { s } else { s.saturating_mul(1000) })
            .filter(|ms| *ms > 0 && *ms <= now);
        let title = clean_text(&html[tag_end..text_end]);
        // Stable across imports: the URL and the date exactly as the file has them.
        let id = format!("bm-{:016x}", fnv(&format!("{url}\u{0}{raw_date}")));
        // In the archive from now (retention counts from the import); first seen when
        // it was bookmarked. The importer never updates an existing record from a
        // bookmark (`FileOptions::bookmarks`), so `now` here changes nothing on a re-import.
        let seen = added.unwrap_or(now);
        items.push(json!({
            "id": id,
            "kind": "tab",
            "appId": app_id,
            "title": title,
            "url": url,
            "state": "archived",
            "firstSeenAt": seen,
            "lastUsedAt": seen,
            "stateChangedAt": now,
            "archivedAt": now,
        }));
    }
    (items, false)
}

/// A tag's attributes (names lowercased, values unescaped; `"…"`, `'…'` or bare), read
/// from `at` (just after the tag name) to its `>` outside quotes. Returns them and the
/// index after the `>`; `None` when the tag never ends.
fn tag_attrs(html: &str, at: usize) -> Option<(Vec<(String, String)>, usize)> {
    let b = html.as_bytes();
    let mut i = at;
    let mut out = Vec::new();
    loop {
        while b.get(i).is_some_and(|c| c.is_ascii_whitespace() || *c == b'/') {
            i += 1;
        }
        if *b.get(i)? == b'>' {
            return Some((out, i + 1));
        }
        let name_start = i;
        while b.get(i).is_some_and(|c| !c.is_ascii_whitespace() && !matches!(c, b'=' | b'>')) {
            i += 1;
        }
        let name = html[name_start..i].to_ascii_lowercase();
        while b.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        let mut value = String::new();
        if b.get(i) == Some(&b'=') {
            i += 1;
            while b.get(i).is_some_and(u8::is_ascii_whitespace) {
                i += 1;
            }
            if let q @ (b'"' | b'\'') = b.get(i)? {
                let end = html[i + 1..].find(char::from(*q))? + i + 1;
                value = unescape(&html[i + 1..end]);
                i = end + 1;
            } else {
                let start = i;
                while b.get(i).is_some_and(|c| !c.is_ascii_whitespace() && *c != b'>') {
                    i += 1;
                }
                value = unescape(&html[start..i]);
            }
        }
        out.push((name, value));
    }
}

/// A link's text: tags dropped, entities decoded, control characters removed, spaces
/// collapsed, at most 1,000 characters.
fn clean_text(raw: &str) -> String {
    let mut text = String::with_capacity(raw.len().min(4096));
    let mut in_tag = false;
    for c in raw.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => text.push(c),
            _ => {}
        }
    }
    one_line(&unescape(&text)).chars().take(1000).collect()
}

/// FNV-1a (stable ids for imported bookmarks).
fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

#[cfg(test)]
mod tests {
    use decay_core::{ItemKind, ItemState, Reason};

    use super::*;
    use crate::FileOptions;
    use crate::items::Platform;
    use crate::test_util::*;

    fn archived(store: &Store, id: &str, url: &str, title: &str, at: i64) -> Item {
        let mut i = Item::new_live(id.into(), ItemKind::Tab, "com.google.Chrome", NOW);
        i.title = title.into();
        i.url = Some(url.into());
        i.domain = Some("example.com".into());
        i.state = ItemState::Archived;
        i.archived_at = Some(at);
        i.archive_reason = Some(Reason::Unused { active_hours: 61, threshold: 60.0 });
        store.insert_item(&i).unwrap();
        i
    }

    #[test]
    fn json_exports_import_back_losslessly() {
        let (src, _d1) = store();
        archived(&src, "x1", "https://example.com/a", "A & B", NOW);
        let mut w = Item::new_live("w1".into(), ItemKind::Window, "com.apple.TextEdit", NOW);
        w.title = "notes.txt".into();
        w.path = Some("/Users/me/notes.txt".into());
        w.state = ItemState::Archived;
        w.archived_at = Some(NOW + 1);
        src.ensure_app("com.apple.TextEdit", Platform::MacOs, ItemKind::Window).unwrap();
        src.insert_item(&w).unwrap();
        let live = Item::new_live("live".into(), ItemKind::Tab, "com.google.Chrome", NOW);
        src.insert_item(&live).unwrap();
        let (doc, count) = src.export_json(true, true, NOW).unwrap();
        assert_eq!(count, 2, "live items aren't exported");
        assert_eq!(doc["format"], "decay-archive");
        assert!(doc["settings"]["thresholds"].is_object());
        assert!(doc["settings"].get("mode").is_none());

        let (mut dst, _d2) = store();
        let report = dst.import_archive(&doc, Platform::MacOs, NOW).unwrap();
        assert_eq!(report.items_imported, 2);
        for id in ["x1", "w1"] {
            let (old, new) = (src.item(id).unwrap().unwrap(), dst.item(id).unwrap().unwrap());
            assert_eq!(
                (&old.title, &old.url, &old.path, old.state, old.archived_at, &old.archive_reason),
                (&new.title, &new.url, &new.path, new.state, new.archived_at, &new.archive_reason),
                "{id}"
            );
        }
        // Again: nothing new.
        assert_eq!(dst.import_archive(&doc, Platform::MacOs, NOW).unwrap().items_existing, 2);
    }

    #[test]
    fn bookmarks_round_trip_by_day_and_import_once() {
        let (mut a, _d) = store();
        archived(&a, "x1", "https://example.com/a?b=1&c=2", "Fish <&> \"chips\"", NOW);
        archived(&a, "x2", "https://example.com/b", "", NOW - 86_400_000);
        archived(&a, "x3", "file:///Users/me/x.html", "local", NOW);
        let (html, count) = a.export_bookmarks(&TimeZone::UTC).unwrap();
        assert_eq!(count, 2, "web pages only");
        assert!(html.starts_with("<!DOCTYPE NETSCAPE-Bookmark-file-1>"));
        assert!(html.contains("<H3>2026-11-04</H3>") && html.contains("<H3>2026-11-03</H3>"));
        assert!(html.contains("Fish &lt;&amp;&gt; &quot;chips&quot;"));
        assert!(html.contains("HREF=\"https://example.com/a?b=1&amp;c=2\""));

        let (items, capped) = bookmarks_to_items(&html, "com.google.Chrome", NOW + 1);
        assert!(!capped);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["url"], "https://example.com/a?b=1&c=2");
        assert_eq!(items[0]["title"], "Fish <&> \"chips\"");
        assert_eq!(items[0]["firstSeenAt"], json!(NOW), "bookmarked when it was archived");
        assert_eq!(items[0]["archivedAt"], json!(NOW + 1), "in the archive from the import");
        let (mut b, _d2) = store();
        let doc = json!({ "items": items });
        let opts = FileOptions { bookmarks: true, ..FileOptions::default() };
        assert_eq!(b.import_file(&doc, Platform::MacOs, NOW + 1, &opts).unwrap().items_imported, 2);
        assert_eq!(b.import_file(&doc, Platform::MacOs, NOW + 1, &opts).unwrap().items_existing, 2);
        // Decay's own bookmarks back into the archive they came from: nothing doubles.
        let report = a.import_file(&doc, Platform::MacOs, NOW + 1, &opts).unwrap();
        assert_eq!((report.items_imported, report.items_existing), (0, 2));
    }

    #[test]
    fn importing_the_same_bookmarks_later_changes_nothing() {
        let (mut b, _d) = store();
        let html = "<!DOCTYPE NETSCAPE-Bookmark-file-1><DL><DT><A HREF=\"https://a.example/\" \
            ADD_DATE=\"1700000000\">A</A><DT><A HREF=\"https://a.example/\" ADD_DATE=\"1700000001\">A again</A>";
        let opts = FileOptions { bookmarks: true, ..FileOptions::default() };
        let (items, _) = bookmarks_to_items(html, "com.google.Chrome", NOW);
        let r = b.import_file(&json!({ "items": items }), Platform::MacOs, NOW, &opts).unwrap();
        assert_eq!((r.items_imported, r.items_existing), (1, 1), "one address, once");
        let id = items[0]["id"].as_str().unwrap();
        // Restored since, then the same file again a day later.
        b.conn
            .execute(
                "UPDATE items SET state = 'restored', restore_count = 1, state_changed_at = ?2 WHERE id = ?1",
                rusqlite::params![id, NOW + 10],
            )
            .unwrap();
        let later = NOW + 86_400_000;
        let (items, _) = bookmarks_to_items(html, "com.google.Chrome", later);
        let r = b.import_file(&json!({ "items": items }), Platform::MacOs, later, &opts).unwrap();
        assert_eq!((r.items_imported, r.items_updated, r.items_existing), (0, 0, 2));
        let item = b.item(id).unwrap().unwrap();
        assert_eq!(
            (item.state, item.restore_count, item.archived_at),
            (ItemState::Restored, 1, Some(NOW))
        );
    }

    #[test]
    fn bookmark_links_past_the_limit_are_reported() {
        let one = "<DT><A HREF=\"https://x.example/\">x</A>\n";
        let html = one.repeat(MAX_BOOKMARKS + 1);
        let (items, capped) = bookmarks_to_items(&html, "c", NOW);
        assert_eq!(items.len(), MAX_BOOKMARKS);
        assert!(capped);
        let (_, capped) = bookmarks_to_items(&one.repeat(3), "c", NOW);
        assert!(!capped);
    }

    #[test]
    fn exported_titles_stay_on_one_line_and_days_follow_daylight_saving() {
        let (a, _d) = store();
        let tz = TimeZone::get("America/New_York").unwrap();
        // 2026-11-01 04:30 UTC = 00:30 EDT on Nov 1 (DST ends at 06:00 UTC that day);
        // 2026-11-02 04:30 UTC = 23:30 EST on Nov 1. Today's offset would split them.
        archived(&a, "x1", "https://example.com/a", "line1\n\tline2  ", 1_793_507_400_000);
        archived(&a, "x2", "HTTPS://example.com/b", "b", 1_793_597_400_000);
        let (html, count) = a.export_bookmarks(&tz).unwrap();
        assert_eq!(count, 2);
        assert_eq!(html.matches("<H3>2026-11-01</H3>").count(), 1, "{html}");
        assert!(html.contains(">line1 line2</A>"), "{html}");
        assert!(html.contains("HREF=\"https://example.com/b\""));
        let (csv, _) = a.export_csv(&tz).unwrap();
        assert!(csv.contains(",2026-11-01 00:30:00,"), "{csv}");
    }

    #[test]
    fn bookmarks_from_browsers_are_read_leniently() {
        let html = r#"<DL><p><DT><a href="https://x.com/" add_date="1700000000" ICON="data:x">X &#x26; Y</a>
            <DT><A HREF="javascript:alert(1)">no</A><DT><A HREF="https://y.com/">Y</A>
            <DT><A HREF="https://z.com/" ADD_DATE="99999999999999">future</A>"#;
        let (items, _) = bookmarks_to_items(html, "com.google.Chrome", NOW);
        let urls: Vec<&str> = items.iter().map(|i| i["url"].as_str().unwrap()).collect();
        assert_eq!(urls, ["https://x.com/", "https://y.com/", "https://z.com/"]);
        assert_eq!(items[0]["title"], "X & Y");
        assert_eq!(items[0]["firstSeenAt"], json!(1_700_000_000_000_i64));
        assert_eq!(items[0]["archivedAt"], json!(NOW), "retention counts from the import");
        assert_eq!(items[1]["firstSeenAt"], json!(NOW), "no date: now");
        assert_eq!(items[2]["firstSeenAt"], json!(NOW), "a date in the future: now");
        assert_eq!(
            bookmarks_to_items("<a href=\"https://unclosed", "a", NOW).0,
            Vec::<Value>::new()
        );
        // Upper-case schemes are web links too.
        let (upper, _) = bookmarks_to_items("<DT><A HREF=\"HTTPS://Up.example/X\">u</A>", "a", NOW);
        assert_eq!(upper[0]["url"], "https://Up.example/X");
        // Ids don't depend on when the file is imported (Safari files have no dates).
        let (again, _) = bookmarks_to_items(html, "com.google.Chrome", NOW + 5);
        assert_eq!(items[1]["id"], again[1]["id"]);
    }

    #[test]
    fn odd_markup_still_reads_every_link() {
        let html = "<DL><DT><A HREF='https://single.example/'>Single</A>\n\
            <DT><a\n\thref=https://bare.example/>Bare</a>\n\
            <DT><A data-href=\"https://wrong.example/\" HREF=\"https://right.example/\" TITLE=\"a > b\">Right</A>\n\
            <DT><A HREF=\"https://unclosed.example/\">Unclosed <b>bold</b>\n\
            <DT><A HREF=\"https://next.example/\">Next&#0;one</A>\n\
            <DT><abbr>not a link</abbr><address>x</address>\n\
            <DT><A HREF=\"https://ctl.example/&#0;\">control</A>";
        let (items, _) = bookmarks_to_items(html, "c", NOW);
        let got: Vec<(&str, &str)> = items
            .iter()
            .map(|i| (i["url"].as_str().unwrap(), i["title"].as_str().unwrap()))
            .collect();
        assert_eq!(
            got,
            [
                ("https://single.example/", "Single"),
                ("https://bare.example/", "Bare"),
                ("https://right.example/", "Right"),
                ("https://unclosed.example/", "Unclosed bold"),
                ("https://next.example/", "Next one"),
            ]
        );
    }

    #[test]
    fn csv_quotes_and_neutralises_formulas() {
        let (a, _d) = store();
        archived(&a, "x1", "https://example.com/a", "=HYPERLINK(\"evil\")", NOW);
        archived(&a, "x2", "https://example.com/b", "plain, with \"quotes\"\nand a line", NOW - 1);
        let (csv, count) = a.export_csv(&TimeZone::UTC).unwrap();
        assert_eq!(count, 2);
        let lines: Vec<&str> = csv.split("\r\n").collect();
        assert_eq!(lines[0], "title,url,domain,kind,app,state,archived_at,reason");
        assert!(lines[1].starts_with("\"'=HYPERLINK(\"\"evil\"\")\","), "{}", lines[1]);
        assert!(csv.contains("\"plain, with \"\"quotes\"\"\nand a line\""));
        assert!(lines[1].ends_with(",unused"), "{}", lines[1]);
        assert_eq!(csv_cell("-1"), "'-1");
        assert_eq!(csv_cell("ok"), "ok");
    }

    #[test]
    fn entities_decode_safely() {
        assert_eq!(
            unescape("a &amp; b &lt;c&gt; &#65;&#x42; &unknown; & lone"),
            "a & b <c> AB &unknown; & lone"
        );
        assert_eq!(escape("<a href=\"x\">'"), "&lt;a href=&quot;x&quot;&gt;&#39;");
    }
}
