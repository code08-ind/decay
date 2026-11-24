//! `import.archive` (API Design §4.1, DB Design §7): the extension's Phase 0 archive moved
//! into the app when it first connects.
//!
//! - Items: archived and restored records only. Live tabs come from the extension's
//!   `snapshot` instead, which also gives them their current tab ids. App ids are mapped
//!   to this platform's catalog (the extension says `com.microsoft.Edge`, macOS knows Edge
//!   as `com.microsoft.edgemac`). Sweeps aren't imported, so `lastSweepId` is dropped;
//!   favicons are fetched later from the extension's `faviconUrl` (not stored here).
//! - Rules: user and quick-action rules; the defaults already exist in the app.
//! - Settings: the extension's `camelCase` keys map to the app's `snake_case` ones. A setting
//!   the user already changed in the app is kept; `shadow_started_at` keeps the earlier.
//!
//! Lenient (one bad record is skipped and counted, never fails the import) and idempotent
//! (an id that already exists is left alone), all in one transaction.

use decay_core::{
    ItemKind, ItemState, Mode, Reason, Rule, RuleSource, SaveState, SqlEnum, Thresholds,
};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::Value;

use crate::items::{Item, Platform};
use crate::{Result, Store, StoreError};

/// What an import did.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub items_imported: u32,
    /// Already in the app (an earlier import), unchanged.
    pub items_existing: u32,
    /// Already in the app, with a newer state from the extension (e.g. restored since).
    pub items_updated: u32,
    /// Live tabs (the snapshot brings those).
    pub items_live: u32,
    /// Couldn't be read (file imports also: an id, app or address that isn't usable, such
    /// as a tab that can't be reopened).
    pub items_invalid: u32,
    /// Archived longer ago than this installation keeps items (file imports only: the
    /// retention purge would delete them within the hour).
    pub items_expired: u32,
    /// Stored items deleted because an imported exclude rule covers them (file imports).
    pub items_purged: u32,
    pub rules_imported: u32,
    pub rules_existing: u32,
    pub rules_invalid: u32,
    /// Built-in rules the file turned off (counted in `rules_existing` too).
    pub rules_turned_off: u32,
    pub settings_imported: u32,
}

/// How a file import (P2.5) differs from the extension's migration.
#[derive(Debug, Clone, Default)]
pub struct FileOptions {
    /// Items archived before this (ms) aren't imported: retention would purge them.
    pub retention_cutoff: i64,
    /// Stored items to delete in the same transaction (an imported exclude rule covers
    /// them).
    pub purge: Vec<String>,
    /// The items come from a bookmarks file: one whose id or address is already in the
    /// archive (or earlier in the file) is left alone, never updated.
    pub bookmarks: bool,
}

/// Longest id or app id a file may bring.
const MAX_ID_LEN: usize = 256;
/// Longest title, group title or document path a file may bring.
const MAX_TEXT_LEN: usize = 4096;
/// How far past `now` a file's grace period may run (the Undo grace).
const MAX_GRACE_MS: i64 = 48 * 3_600_000;

/// An id or app id from a file: non-empty, short, no control characters or separators
/// (an app id becomes a catalog row).
fn id_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_ID_LEN
        && !s.chars().any(|c| c.is_control() || matches!(c, '/' | '\\'))
        && !s.contains("..")
}

/// At most `max` bytes of `s`, cut at a character boundary.
fn capped(mut s: String, max: usize) -> String {
    if s.len() > max {
        let mut end = max;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
    s
}

/// A file's item made safe to store: text capped, times no later than `now`, nothing
/// that can't be reopened. `None` when it can't be kept.
fn sanitize_file_item(mut ext: ExtItem, now: i64) -> Option<ExtItem> {
    if !id_ok(&ext.id) || !id_ok(&ext.app_id) {
        return None;
    }
    if ext.url.as_ref().is_some_and(|u| u.len() > crate::export::MAX_URL_LEN) {
        return None;
    }
    if ext.kind == ItemKind::Tab && !decay_core::policy::is_restorable_url(ext.url.as_deref()) {
        return None;
    }
    ext.title = capped(ext.title, MAX_TEXT_LEN);
    ext.path = ext.path.map(|p| capped(p, MAX_TEXT_LEN));
    ext.group_title = ext.group_title.map(|g| capped(g, MAX_TEXT_LEN));
    ext.group_color = ext.group_color.filter(|c| c.len() <= 32);
    ext.restored_from_id = ext.restored_from_id.filter(|r| id_ok(r));
    // Times from another machine (or a crafted file) never lie in the future: a future
    // archive time would never expire, a future state change would win every update.
    #[allow(clippy::cast_precision_loss, reason = "epoch ms fit an f64 exactly")]
    let (now_f, grace_f) = (now as f64, (now + MAX_GRACE_MS) as f64);
    let past = |v: f64| if v.is_finite() { v.clamp(0.0, now_f) } else { 0.0 };
    ext.first_seen_at = past(ext.first_seen_at);
    ext.last_used_at = past(ext.last_used_at);
    ext.state_changed_at = past(ext.state_changed_at);
    ext.archived_at = ext.archived_at.map(past);
    ext.grace_until =
        ext.grace_until.map(|g| if g.is_finite() { g.clamp(0.0, grace_f) } else { 0.0 });
    Some(ext)
}

/// An item as the extension stores it (Phase 0 `Item`).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExtItem {
    id: String,
    kind: ItemKind,
    app_id: String,
    #[serde(default)]
    title: String,
    url: Option<String>,
    domain: Option<String>,
    /// Windows: the document or folder (desktop exports).
    #[serde(default)]
    path: Option<String>,
    tab_index: Option<i64>,
    group_title: Option<String>,
    group_color: Option<String>,
    #[serde(default)]
    pinned: bool,
    state: ItemState,
    #[serde(default)]
    active_age_s: f64,
    first_seen_at: f64,
    last_used_at: f64,
    state_changed_at: f64,
    archived_at: Option<f64>,
    archive_reason: Option<Value>,
    restored_from_id: Option<String>,
    favicon_url: Option<String>,
    grace_until: Option<f64>,
    #[serde(default)]
    restore_count: f64,
}

/// A record the database refuses (a `CHECK` / foreign key / unique constraint), as opposed
/// to a failure of the database itself (disk full, I/O), which must abort the import.
fn constraint(e: &StoreError) -> bool {
    matches!(e, StoreError::Sql(s) if s.sqlite_error_code() == Some(rusqlite::ErrorCode::ConstraintViolation))
}

/// JSON numbers from JS are doubles: whole milliseconds / seconds here.
#[allow(clippy::cast_possible_truncation, reason = "clamped to the i64 range first")]
fn ms(v: f64) -> i64 {
    if v.is_finite() { v.clamp(i64::MIN as f64, i64::MAX as f64).round() as i64 } else { 0 }
}

/// Browsers' catalog ids on macOS and on Windows (the engine's `browser_app`).
const BROWSER_IDS: [(&str, &str); 6] = [
    ("com.google.Chrome", "chrome.exe"),
    ("com.microsoft.edgemac", "msedge.exe"),
    ("com.brave.Browser", "brave.exe"),
    ("company.thebrowser.Browser", "arc.exe"),
    ("org.chromium.Chromium", "chromium.exe"),
    ("org.mozilla.firefox", "firefox.exe"),
];

/// The app catalog id on this platform for an app id from the extension or from another
/// machine's export (a Windows `chrome.exe` is `com.google.Chrome` on a Mac, and back).
pub fn catalog_app_id(app_id: &str, platform: Platform) -> String {
    // The extension calls Edge `com.microsoft.Edge` on every platform.
    let app_id = if app_id == "com.microsoft.Edge" { "com.microsoft.edgemac" } else { app_id };
    for (mac, win) in BROWSER_IDS {
        if app_id == mac || app_id.eq_ignore_ascii_case(win) {
            return if platform == Platform::Windows { win } else { mac }.to_owned();
        }
    }
    app_id.to_owned()
}

/// Extension setting keys → app setting keys (DB Design §3.9).
const SETTING_KEYS: [(&str, &str); 11] = [
    ("mode", "mode"),
    ("shadowStartedAt", "shadow_started_at"),
    ("preset", "preset"),
    ("thresholds", "thresholds"),
    ("idleCutoffS", "idle_cutoff_s"),
    ("maxItemsPerSweep", "max_items_per_sweep"),
    ("retentionDays", "retention_days"),
    ("holdUntil", "hold_until"),
    ("theme", "theme"),
    ("notifications", "notifications"),
    ("locale", "locale"),
];

/// Whether an imported value is one the app would accept for `key` (the same ranges as
/// Settings: an import never stores what the app's own checks refuse).
fn setting_ok(key: &str, value: &Value) -> bool {
    let int_in = |lo: u64, hi: u64| value.as_u64().is_some_and(|n| (lo..=hi).contains(&n));
    match key {
        "mode" => serde_json::from_value::<Mode>(value.clone()).is_ok(),
        "thresholds" => {
            serde_json::from_value::<Thresholds>(value.clone()).is_ok_and(|t| t.is_valid())
        }
        "shadow_started_at" | "hold_until" => value.is_null() || value.is_number(),
        "idle_cutoff_s" => int_in(60, 3600),
        "max_items_per_sweep" => int_in(1, 500),
        "retention_days" => int_in(7, 3650),
        "preset" => {
            value.as_str() == Some("custom")
                || serde_json::from_value::<decay_core::Preset>(value.clone()).is_ok()
        }
        "theme" => matches!(value.as_str(), Some("system" | "light" | "dark")),
        "locale" => value.as_str().is_some_and(|l| {
            !l.is_empty()
                && l.len() <= 35
                && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        }),
        "notifications" => value.as_object().is_some_and(|o| {
            const KNOWN: [&str; 6] =
                ["summary", "sweep", "sweepBigOnly", "browser", "permission", "health"];
            !o.is_empty() && o.iter().all(|(k, v)| KNOWN.contains(&k.as_str()) && v.is_boolean())
        }),
        _ => false,
    }
}

/// The stored record of an archived or restored item from a file or the extension
/// (`restored_from_id` is linked later, once every item exists).
fn new_item(ext: ExtItem, app_id: String) -> Item {
    let reason = ext.archive_reason.and_then(|r| serde_json::from_value::<Reason>(r).ok());
    Item {
        id: ext.id.clone(),
        kind: ext.kind,
        app_id,
        source_id: None,
        external_id: None,
        title: ext.title,
        url: ext.url,
        domain: ext.domain,
        path: ext.path,
        favicon: None,
        // `data:` icons can be large; they're fetched again from the page if needed.
        favicon_url: ext.favicon_url.filter(|u| !u.starts_with("data:") && u.len() <= 2048),
        tab_index: ext.tab_index,
        group_title: ext.group_title,
        group_color: ext.group_color,
        pinned: ext.pinned,
        state: ext.state,
        save_state: if ext.kind == ItemKind::Tab {
            SaveState::NotApplicable
        } else {
            SaveState::Unknown
        },
        active_age_s: u64::try_from(ms(ext.active_age_s)).unwrap_or(0),
        first_seen_at: ms(ext.first_seen_at),
        last_used_at: ms(ext.last_used_at),
        state_changed_at: ms(ext.state_changed_at),
        archived_at: ext.archived_at.map(ms),
        archive_reason: reason,
        last_sweep_id: None,
        restored_from_id: None,
        grace_until: ext.grace_until.map(ms),
        restore_count: u32::try_from(ms(ext.restore_count)).unwrap_or(0),
    }
}

impl Store {
    /// Imports the extension's archive (see the module docs). `platform` picks the app ids.
    pub fn import_archive(
        &mut self,
        archive: &Value,
        platform: Platform,
        now: i64,
    ) -> Result<ImportReport> {
        self.import_with(archive, platform, now, None)
    }

    /// Imports a file the user picked (P2.5): like [`Self::import_archive`], except that
    /// - this installation's moment and its retention — the mode, the shadow week, a
    ///   hold, how long items are kept — never come from the file (a file from another
    ///   machine mustn't switch automatic mode on, or shorten retention and so delete
    ///   this archive);
    /// - items older than `opts.retention_cutoff` aren't imported (counted as expired);
    /// - `opts.purge` is deleted in the same transaction.
    pub fn import_file(
        &mut self,
        archive: &Value,
        platform: Platform,
        now: i64,
        opts: &FileOptions,
    ) -> Result<ImportReport> {
        self.import_with(archive, platform, now, Some(opts))
    }

    fn import_with(
        &mut self,
        archive: &Value,
        platform: Platform,
        now: i64,
        file: Option<&FileOptions>,
    ) -> Result<ImportReport> {
        let mut report = ImportReport::default();
        let tx = self.conn.unchecked_transaction()?;
        let empty = Vec::new();
        let list = |key: &str| archive.get(key).and_then(Value::as_array).unwrap_or(&empty);

        if let Some(opts) = file {
            for id in &opts.purge {
                if self.delete_item(id)? {
                    report.items_purged += 1;
                }
            }
        }
        self.import_items(list("items"), platform, now, file, &mut report)?;
        self.import_rules(list("rules"), &mut report)?;
        if let Some(settings) = archive.get("settings").and_then(Value::as_object) {
            self.import_settings(settings, now, file.is_some(), &mut report)?;
        }
        tx.commit()?;
        Ok(report)
    }

    fn import_items(
        &self,
        items: &[Value],
        platform: Platform,
        now: i64,
        file: Option<&FileOptions>,
        report: &mut ImportReport,
    ) -> Result<()> {
        let cutoff = file.map(|o| o.retention_cutoff);
        let bookmarks = file.is_some_and(|o| o.bookmarks);
        // Bookmarks: addresses already archived (or earlier in the file) aren't added again.
        let mut known_urls = std::collections::HashSet::new();
        if bookmarks {
            let mut stmt = self.conn.prepare(
                "SELECT url FROM items WHERE url IS NOT NULL AND state IN ('archived', 'restored')",
            )?;
            for url in stmt.query_map([], |r| r.get::<_, String>(0))? {
                known_urls.insert(url?);
            }
        }
        // Items (restored_from_id is set in a second pass, once every item exists).
        let mut links = Vec::new();
        for raw in items {
            let Ok(ext) = ExtItem::deserialize(raw) else {
                report.items_invalid += 1;
                continue;
            };
            if !matches!(ext.state, ItemState::Archived | ItemState::Restored) {
                report.items_live += 1;
                continue;
            }
            let ext = if file.is_some() {
                let Some(ext) = sanitize_file_item(ext, now) else {
                    report.items_invalid += 1;
                    continue;
                };
                ext
            } else {
                ext
            };
            if bookmarks && ext.url.as_ref().is_some_and(|u| known_urls.contains(u)) {
                report.items_existing += 1;
                continue;
            }
            if let Some(existing) = self.item(&ext.id)? {
                // Imported before: carry over a later state change (restored, archived
                // again) made in the extension while the app wasn't running. A file only
                // updates an archived or restored record of the same kind (never a live
                // tab or window), and a bookmark never does.
                let updatable = file.is_none()
                    || (!bookmarks
                        && existing.kind == ext.kind
                        && matches!(existing.state, ItemState::Archived | ItemState::Restored));
                if updatable && ms(ext.state_changed_at) > existing.state_changed_at {
                    self.conn.execute(
                        "UPDATE items SET state = ?2, state_changed_at = ?3, archived_at = ?4,
                           restore_count = ?5, grace_until = ?6 WHERE id = ?1",
                        params![
                            ext.id,
                            ext.state.as_sql(),
                            ms(ext.state_changed_at),
                            ext.archived_at.map(ms),
                            u32::try_from(ms(ext.restore_count)).unwrap_or(0),
                            ext.grace_until.map(ms),
                        ],
                    )?;
                    report.items_updated += 1;
                } else {
                    report.items_existing += 1;
                }
                if let Some(from) = ext.restored_from_id {
                    links.push((ext.id, from));
                }
                continue;
            }
            if cutoff.is_some_and(|c| ms(ext.archived_at.unwrap_or(ext.state_changed_at)) < c) {
                report.items_expired += 1;
                continue;
            }
            if let Some(url) = ext.url.as_ref().filter(|_| bookmarks) {
                known_urls.insert(url.clone());
            }
            let app_id = catalog_app_id(&ext.app_id, platform);
            self.ensure_app(&app_id, platform, ext.kind)?;
            let restored_from = ext.restored_from_id.clone();
            let item = new_item(ext, app_id);
            match self.insert_item(&item) {
                Ok(()) => report.items_imported += 1,
                Err(e) if constraint(&e) => report.items_invalid += 1,
                Err(e) => return Err(e),
            }
            if let Some(from) = restored_from {
                links.push((item.id, from));
            }
        }
        for (id, from) in links {
            self.conn.execute(
                "UPDATE items SET restored_from_id = ?2
             WHERE id = ?1 AND EXISTS (SELECT 1 FROM items WHERE id = ?2)",
                params![id, from],
            )?;
        }
        Ok(())
    }

    fn import_rules(&self, rules: &[Value], report: &mut ImportReport) -> Result<()> {
        // Rules.
        for raw in rules {
            let Ok(rule) = serde_json::from_value::<Rule>(raw.clone()) else {
                report.rules_invalid += 1;
                continue;
            };
            if rule.source == RuleSource::Default {
                // The defaults exist in the app; keep the user's choice to turn one off.
                if !rule.enabled {
                    let changed = self.conn.execute(
                        "UPDATE rules SET enabled = 0
                         WHERE source = 'default' AND scope = ?1 AND match = ?2 AND action = ?3
                           AND enabled = 1",
                        params![rule.scope.as_sql(), rule.pattern, rule.action.as_sql()],
                    )?;
                    report.rules_turned_off += u32::try_from(changed).unwrap_or(0);
                }
                report.rules_existing += 1;
                continue;
            }
            let exists: bool = self.conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM rules WHERE id = ?1)",
                [&rule.id],
                |r| r.get(0),
            )?;
            if exists {
                report.rules_existing += 1;
                continue;
            }
            match self.insert_rule(&rule) {
                Ok(()) => report.rules_imported += 1,
                Err(StoreError::DuplicateRule) => report.rules_existing += 1,
                Err(e) if constraint(&e) => report.rules_invalid += 1,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    fn import_settings(
        &self,
        settings: &serde_json::Map<String, Value>,
        now: i64,
        from_file: bool,
        report: &mut ImportReport,
    ) -> Result<()> {
        for (ext_key, key) in SETTING_KEYS {
            if from_file
                && matches!(key, "mode" | "shadow_started_at" | "hold_until" | "retention_days")
            {
                continue;
            }
            let Some(value) = settings.get(ext_key).filter(|v| setting_ok(key, v)) else {
                continue;
            };
            let stored: Option<String> = self
                .conn
                .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
                .optional()?;
            let value = match (key, stored) {
                ("shadow_started_at", Some(current)) => {
                    // The earlier start wins: shadow mode began when the extension did.
                    let current =
                        serde_json::from_str::<Value>(&current).ok().and_then(|v| v.as_f64());
                    match (current, value.as_f64()) {
                        (Some(c), Some(v)) if v < c => Value::from(ms(v)),
                        _ => continue,
                    }
                }
                (_, Some(_)) => continue, // changed in the app: the app's value stays
                ("shadow_started_at" | "hold_until", None) => {
                    value.as_f64().map_or(Value::Null, |v| Value::from(ms(v)))
                }
                (_, None) => value.clone(),
            };
            self.set_setting(key, &value, now)?;
            report.settings_imported += 1;
        }
        Ok(())
    }

    /// Makes sure `app_id` is in the catalog: an unknown browser (tabs) or app (windows,
    /// which Decay can't reopen until it knows the app) gets a placeholder row.
    pub(crate) fn ensure_app(
        &self,
        app_id: &str,
        platform: Platform,
        kind: ItemKind,
    ) -> Result<()> {
        let (app_kind, restore) =
            if kind == ItemKind::Tab { ("browser", "url") } else { ("app", "none") };
        self.conn.execute(
            "INSERT INTO apps (id, name, platform, kind, restore_kind) VALUES (?1, ?1, ?2, ?3, ?4)
             ON CONFLICT (id) DO NOTHING",
            params![
                app_id,
                if platform == Platform::Windows { "windows" } else { "macos" },
                app_kind,
                restore
            ],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::*;
    use serde_json::json;

    fn archive() -> Value {
        json!({
            "items": [
                {"id": "a1", "kind": "tab", "appId": "com.microsoft.Edge", "title": "Pricing",
                 "url": "https://acme.com/pricing", "domain": "acme.com", "pinned": false,
                 "state": "archived", "activeAgeS": 220_000.4, "firstSeenAt": 1.0, "lastUsedAt": 2.0,
                 "stateChangedAt": 3.0, "archivedAt": 1_793_750_000_000.0, "lastSweepId": "s-gone",
                 "archiveReason": {"type": "unused", "activeHours": 61, "threshold": 60},
                 "restoreCount": 0, "faviconUrl": "https://acme.com/f.ico", "windowExternalId": "9"},
                {"id": "a2", "kind": "tab", "appId": "com.vivaldi.Vivaldi", "title": "Docs",
                 "state": "restored", "activeAgeS": 0, "firstSeenAt": 1, "lastUsedAt": 2,
                 "stateChangedAt": 3, "archivedAt": 4, "restoredFromId": "a1", "restoreCount": 1},
                {"id": "live", "kind": "tab", "appId": "com.google.Chrome", "title": "Open",
                 "state": "active", "activeAgeS": 5, "firstSeenAt": 1, "lastUsedAt": 2, "stateChangedAt": 3},
                {"id": "bad", "kind": "spaceship"},
                "not even an object"
            ],
            "rules": [
                {"id": "u1", "scope": "domain", "match": "figma.com", "action": "protect",
                 "enabled": true, "source": "user", "createdAt": 5, "hitCount": 2},
                {"id": "d1", "scope": "domain", "match": "zoom.us", "action": "protect",
                 "enabled": true, "source": "default", "createdAt": 0, "hitCount": 0},
                {"id": "x", "scope": "galaxy", "match": "?", "action": "protect", "enabled": true}
            ],
            "settings": {"mode": "automatic", "shadowStartedAt": 100.0, "retentionDays": 30,
                         "thresholds": {"stale": 50, "hidden": 1, "archive": 2}, "theme": "dark",
                         "betaInstallId": "not-a-setting"}
        })
    }

    #[test]
    fn imports_the_archive_once() {
        let (mut store, _dir) = store();
        store.set_setting("theme", &"light", NOW).unwrap(); // changed in the app: kept
        let report = store.import_archive(&archive(), Platform::MacOs, NOW).unwrap();
        assert_eq!(
            report,
            ImportReport {
                items_imported: 2,
                items_existing: 0,
                items_updated: 0,
                items_live: 1,
                items_invalid: 2,
                items_expired: 0,
                items_purged: 0,
                rules_imported: 1,
                rules_existing: 1,
                rules_invalid: 1,
                rules_turned_off: 0,
                settings_imported: 3, // mode, shadow_started_at (earlier), retention_days
            }
        );
        let a1 = store.item("a1").unwrap().unwrap();
        assert_eq!(a1.app_id, "com.microsoft.edgemac");
        assert_eq!(
            (a1.active_age_s, a1.state, a1.last_sweep_id),
            (220_000, ItemState::Archived, None)
        );
        assert_eq!(a1.archive_reason, Some(Reason::Unused { active_hours: 61, threshold: 60.0 }));
        let a2 = store.item("a2").unwrap().unwrap();
        assert_eq!(a2.restored_from_id.as_deref(), Some("a1"));
        assert!(store.app("com.vivaldi.Vivaldi").unwrap().is_some(), "placeholder app");
        assert_eq!(store.mode().unwrap(), Mode::Automatic);
        assert_eq!(store.setting::<i64>("shadow_started_at").unwrap(), Some(100));
        assert_eq!(store.setting::<String>("theme").unwrap().as_deref(), Some("light"));
        assert_eq!(store.thresholds().unwrap(), Thresholds::BALANCED, "invalid thresholds skipped");
        // Searchable like any archived item.
        let q = crate::SearchQuery {
            text: "pricing".into(),
            filters: crate::SearchFilters::default(),
            limit: 0,
            offset: 0,
            now: NOW,
            utc_offset_minutes: 0,
        };
        assert_eq!(store.search(&q).unwrap().hits.len(), 1);

        // Again: nothing new.
        let again = store.import_archive(&archive(), Platform::MacOs, NOW).unwrap();
        assert_eq!((again.items_imported, again.items_existing), (0, 2));
        assert_eq!((again.rules_imported, again.rules_existing), (0, 2));
    }

    #[test]
    fn later_state_changes_and_disabled_defaults_carry_over() {
        let (mut store, _dir) = store();
        store.import_archive(&archive(), Platform::MacOs, NOW).unwrap();
        assert_eq!(
            store.item("a1").unwrap().unwrap().favicon_url.as_deref(),
            Some("https://acme.com/f.ico")
        );
        // Restored in the extension later, and the zoom default turned off.
        let mut later = archive();
        later["items"][0]["state"] = json!("restored");
        later["items"][0]["stateChangedAt"] = json!(99.0);
        later["items"][0]["restoreCount"] = json!(1);
        later["rules"][1]["enabled"] = json!(false);
        let report = store.import_archive(&later, Platform::MacOs, NOW).unwrap();
        assert_eq!((report.items_updated, report.items_existing), (1, 1));
        assert_eq!(report.rules_turned_off, 1);
        let again = store.import_archive(&later, Platform::MacOs, NOW).unwrap();
        assert_eq!(again.rules_turned_off, 0, "already off");
        let a1 = store.item("a1").unwrap().unwrap();
        assert_eq!((a1.state, a1.restore_count), (ItemState::Restored, 1));
        let zoom = store.rules().unwrap().into_iter().find(|r| r.id == "default-zoom").unwrap();
        assert!(!zoom.enabled);
        // An older state never overwrites a newer one.
        let report = store.import_archive(&archive(), Platform::MacOs, NOW).unwrap();
        assert_eq!(report.items_updated, 0);
        assert_eq!(store.item("a1").unwrap().unwrap().state, ItemState::Restored);
    }

    #[test]
    fn a_file_never_sets_the_mode_shadow_week_or_hold() {
        let (mut store, _dir) = store();
        let mut file = archive();
        file["settings"]["holdUntil"] = json!(NOW + 1);
        let report =
            store.import_file(&file, Platform::MacOs, NOW, &FileOptions::default()).unwrap();
        assert_eq!(report.settings_imported, 1, "the theme only");
        assert_eq!(store.mode().unwrap(), Mode::Shadow);
        assert_eq!(store.setting::<i64>("hold_until").unwrap(), None);
        assert_eq!(store.setting::<u64>("retention_days").unwrap(), Some(90), "never shortened");
    }

    #[test]
    fn a_file_skips_expired_items_and_purges_in_the_same_transaction() {
        let (mut store, _dir) = store();
        store.import_archive(&archive(), Platform::MacOs, NOW).unwrap(); // a1, a2
        let mut file = archive();
        for (i, item) in file["items"].as_array_mut().unwrap().iter_mut().enumerate() {
            if item.is_object() {
                item["id"] = json!(format!("f{i}"));
            }
        }
        // f0 was archived at 1_793_750_000_000; f1 at 4 (long ago). A file's tabs need an
        // address they can reopen.
        file["items"][1]["url"] = json!("https://acme.com/docs");
        let opts =
            FileOptions { retention_cutoff: 1_000_000, purge: vec!["a1".into()], bookmarks: false };
        let report = store.import_file(&file, Platform::MacOs, NOW, &opts).unwrap();
        assert_eq!((report.items_imported, report.items_expired, report.items_purged), (1, 1, 1));
        assert!(store.item("a1").unwrap().is_none() && store.item("f0").unwrap().is_some());
    }

    #[test]
    fn a_file_never_touches_live_records_or_sets_future_times() {
        let (mut store, _dir) = store();
        let live = Item::new_live("tab-1".into(), ItemKind::Tab, "com.google.Chrome", NOW);
        store.insert_item(&live).unwrap();
        let far = 9.0e15;
        let file = json!({"items": [
            {"id": "tab-1", "kind": "tab", "appId": "com.google.Chrome", "title": "x",
             "url": "https://a.example/", "state": "archived", "firstSeenAt": 1,
             "lastUsedAt": 1, "stateChangedAt": far, "archivedAt": far},
            {"id": "new", "kind": "tab", "appId": "com.google.Chrome", "title": "y",
             "url": "https://b.example/", "state": "archived", "firstSeenAt": far,
             "lastUsedAt": far, "stateChangedAt": far, "archivedAt": far, "graceUntil": far}
        ]});
        let report =
            store.import_file(&file, Platform::MacOs, NOW, &FileOptions::default()).unwrap();
        assert_eq!((report.items_imported, report.items_existing, report.items_updated), (1, 1, 0));
        assert_eq!(
            store.item("tab-1").unwrap().unwrap().state,
            ItemState::Active,
            "live stays live"
        );
        let new = store.item("new").unwrap().unwrap();
        assert_eq!(
            (new.archived_at, new.state_changed_at, new.first_seen_at),
            (Some(NOW), NOW, NOW)
        );
        assert_eq!(new.grace_until, Some(NOW + MAX_GRACE_MS));
    }

    #[test]
    fn a_file_brings_only_items_that_can_be_kept() {
        let (mut store, _dir) = store();
        let tab = |id: &str, app: &str, url: &str| {
            json!({"id": id, "kind": "tab", "appId": app, "title": "t", "url": url,
                   "state": "archived", "firstSeenAt": 1, "lastUsedAt": 1,
                   "stateChangedAt": 1, "archivedAt": NOW})
        };
        let mut big = tab("big", "com.google.Chrome", "https://ok.example/");
        big["title"] = json!("x".repeat(2_000_000));
        let file = json!({"items": [
            tab("js", "com.google.Chrome", "javascript:alert(1)"),
            tab("data", "com.google.Chrome", "data:text/html,hi"),
            tab("file", "com.google.Chrome", "file:///etc/passwd"),
            tab("", "com.google.Chrome", "https://ok.example/"),
            tab("dots", "../../etc", "https://ok.example/"),
            tab("long", "com.google.Chrome", &format!("https://ok.example/{}", "a".repeat(9000))),
            big,
        ]});
        let report =
            store.import_file(&file, Platform::MacOs, NOW, &FileOptions::default()).unwrap();
        assert_eq!((report.items_imported, report.items_invalid), (1, 6));
        assert_eq!(store.item("big").unwrap().unwrap().title.len(), MAX_TEXT_LEN);
        assert!(store.app("../../etc").unwrap().is_none(), "no catalog row from a bad app id");
        // The extension's own migration is unchanged.
        assert_eq!(store.import_archive(&file, Platform::MacOs, NOW).unwrap().items_imported, 6);
    }

    #[test]
    fn imported_settings_must_be_ones_the_app_accepts() {
        for (key, good, bad) in [
            ("idle_cutoff_s", json!(300), json!(0)),
            ("max_items_per_sweep", json!(50), json!(999_999_999)),
            ("theme", json!("dark"), json!("<script>")),
            ("locale", json!("en-GB"), json!("x\"y")),
            ("preset", json!("custom"), json!("turbo")),
            ("notifications", json!({"summary": true}), json!({"evil": true})),
            ("retention_days", json!(30), json!(1)),
        ] {
            assert!(setting_ok(key, &good), "{key} {good}");
            assert!(!setting_ok(key, &bad), "{key} {bad}");
        }
    }

    #[test]
    fn unknown_window_apps_get_an_app_row() {
        let (store, _dir) = store();
        store.ensure_app("com.apple.TextEdit", Platform::MacOs, ItemKind::Window).unwrap();
        let app = store.app("com.apple.TextEdit").unwrap().unwrap();
        assert_eq!(app.restore_kind, crate::RestoreKind::None);
    }

    #[test]
    fn maps_app_ids_per_platform() {
        assert_eq!(catalog_app_id("com.google.Chrome", Platform::Windows), "chrome.exe");
        assert_eq!(catalog_app_id("com.google.Chrome", Platform::MacOs), "com.google.Chrome");
        assert_eq!(catalog_app_id("com.brave.Browser", Platform::Windows), "brave.exe");
        assert_eq!(catalog_app_id("org.other", Platform::MacOs), "org.other");
        // Another machine's export.
        assert_eq!(catalog_app_id("chrome.exe", Platform::MacOs), "com.google.Chrome");
        assert_eq!(catalog_app_id("com.microsoft.edgemac", Platform::Windows), "msedge.exe");
        assert_eq!(catalog_app_id("Arc.exe", Platform::MacOs), "company.thebrowser.Browser");
        assert_eq!(catalog_app_id("msedge.exe", Platform::Windows), "msedge.exe");
    }

    #[test]
    fn an_empty_or_odd_archive_imports_nothing() {
        let (mut store, _dir) = store();
        for odd in [json!({}), json!({"items": 5, "rules": "x", "settings": []}), json!(null)] {
            assert_eq!(
                store.import_archive(&odd, Platform::MacOs, NOW).unwrap(),
                ImportReport::default()
            );
        }
    }
}
