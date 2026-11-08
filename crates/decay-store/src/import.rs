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
    /// Couldn't be read.
    pub items_invalid: u32,
    pub rules_imported: u32,
    pub rules_existing: u32,
    pub rules_invalid: u32,
    pub settings_imported: u32,
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

/// The app catalog id for an extension app id on this platform.
pub fn catalog_app_id(ext_app_id: &str, platform: Platform) -> String {
    let mapped = match (ext_app_id, platform) {
        ("com.google.Chrome", Platform::Windows) => "chrome.exe",
        ("com.microsoft.Edge", Platform::MacOs) => "com.microsoft.edgemac",
        ("com.microsoft.Edge", Platform::Windows) => "msedge.exe",
        ("com.brave.Browser", Platform::Windows) => "brave.exe",
        (other, _) => other,
    };
    mapped.to_owned()
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

/// Whether an imported value has the shape the app expects for `key`.
fn setting_ok(key: &str, value: &Value) -> bool {
    match key {
        "mode" => serde_json::from_value::<Mode>(value.clone()).is_ok(),
        "thresholds" => {
            serde_json::from_value::<Thresholds>(value.clone()).is_ok_and(|t| t.is_valid())
        }
        "shadow_started_at" | "hold_until" => value.is_null() || value.is_number(),
        "idle_cutoff_s" | "max_items_per_sweep" | "retention_days" => value.as_u64().is_some(),
        "preset" | "theme" | "locale" => value.is_string(),
        "notifications" => value.is_object(),
        _ => false,
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
        let mut report = ImportReport::default();
        let tx = self.conn.unchecked_transaction()?;
        let empty = Vec::new();
        let list = |key: &str| archive.get(key).and_then(Value::as_array).unwrap_or(&empty);

        self.import_items(list("items"), platform, &mut report)?;
        self.import_rules(list("rules"), &mut report)?;
        if let Some(settings) = archive.get("settings").and_then(Value::as_object) {
            self.import_settings(settings, now, &mut report)?;
        }
        tx.commit()?;
        Ok(report)
    }

    fn import_items(
        &self,
        items: &[Value],
        platform: Platform,
        report: &mut ImportReport,
    ) -> Result<()> {
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
            if let Some(existing) = self.item(&ext.id)? {
                // Imported before: carry over a later state change (restored, archived
                // again) made in the extension while the app wasn't running.
                if ms(ext.state_changed_at) > existing.state_changed_at {
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
            let app_id = catalog_app_id(&ext.app_id, platform);
            self.ensure_app(&app_id, platform)?;
            let reason = ext.archive_reason.and_then(|r| serde_json::from_value::<Reason>(r).ok());
            let item = Item {
                id: ext.id.clone(),
                kind: ext.kind,
                app_id,
                source_id: None,
                external_id: None,
                title: ext.title,
                url: ext.url,
                domain: ext.domain,
                path: None,
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
            };
            match self.insert_item(&item) {
                Ok(()) => report.items_imported += 1,
                Err(e) if constraint(&e) => report.items_invalid += 1,
                Err(e) => return Err(e),
            }
            if let Some(from) = ext.restored_from_id {
                links.push((ext.id, from));
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
                    self.conn.execute(
                        "UPDATE rules SET enabled = 0
                         WHERE source = 'default' AND scope = ?1 AND match = ?2 AND action = ?3",
                        params![rule.scope.as_sql(), rule.pattern, rule.action.as_sql()],
                    )?;
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
        report: &mut ImportReport,
    ) -> Result<()> {
        for (ext_key, key) in SETTING_KEYS {
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

    /// Makes sure `app_id` is in the catalog (an unknown browser gets a placeholder row).
    fn ensure_app(&self, app_id: &str, platform: Platform) -> Result<()> {
        self.conn.execute(
            "INSERT INTO apps (id, name, platform, kind, restore_kind) VALUES (?1, ?1, ?2, 'browser', 'url')
             ON CONFLICT (id) DO NOTHING",
            params![app_id, if platform == Platform::Windows { "windows" } else { "macos" }],
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
                rules_imported: 1,
                rules_existing: 1,
                rules_invalid: 1,
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
    fn maps_app_ids_per_platform() {
        assert_eq!(catalog_app_id("com.google.Chrome", Platform::Windows), "chrome.exe");
        assert_eq!(catalog_app_id("com.google.Chrome", Platform::MacOs), "com.google.Chrome");
        assert_eq!(catalog_app_id("com.brave.Browser", Platform::Windows), "brave.exe");
        assert_eq!(catalog_app_id("org.other", Platform::MacOs), "org.other");
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
