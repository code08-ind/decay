//! Settings (DB Design §3.9): a JSON key-value table. Missing keys read as their defaults,
//! so only changed values are stored (and new settings need no migration).

use decay_core::{Mode, Thresholds};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::{Result, Store, StoreError};

/// The default value of a setting, or `None` for an unknown key.
pub fn default_setting(key: &str) -> Option<Value> {
    Some(match key {
        "mode" => json!("shadow"),
        "shadow_started_at" | "hold_until" | "onboarded_at" => Value::Null,
        "preset" => json!("balanced"),
        "thresholds" => json!({"stale": 12, "hidden": 36, "archive": 60}),
        "idle_cutoff_s" => json!(300),
        "max_items_per_sweep" => json!(50),
        "retention_days" => json!(90),
        "hotkey" => json!(if cfg!(target_os = "windows") { "Ctrl+Alt+D" } else { "Alt+Super+D" }),
        "theme" | "locale" => json!("system"),
        "update_channel" => json!("stable"),
        "launch_at_login" | "auto_update" => json!(true),
        "daily_summary_time" => json!("09:00"),
        "notifications" => json!({"summary": true, "sweep": false, "health": true}),
        "window_management" => json!("hide"),
        // `batch_labels` (F15, Pro): name archived batches on this computer; off until
        // turned on.
        "crash_reports" | "batch_labels" => json!(false),
        _ => return None,
    })
}

/// Every known key (for snapshots).
const KEYS: [&str; 20] = [
    "mode",
    "onboarded_at",
    "shadow_started_at",
    "preset",
    "thresholds",
    "idle_cutoff_s",
    "max_items_per_sweep",
    "retention_days",
    "hold_until",
    "hotkey",
    "theme",
    "launch_at_login",
    "update_channel",
    "auto_update",
    "daily_summary_time",
    "notifications",
    "window_management",
    "crash_reports",
    "batch_labels",
    "locale",
];

/// First run: shadow mode starts now (Software Design §5.2).
pub(crate) fn seed(conn: &Connection, now: i64) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO settings (key, value, updated_at) VALUES ('shadow_started_at', ?1, ?1)",
        [now],
    )?;
    Ok(())
}

impl Store {
    /// A setting's stored value, else its default (`None` for an unknown key). A stored
    /// value that can't be parsed reads as the default too: settings must always load.
    pub fn setting_value(&self, key: &str) -> Result<Option<Value>> {
        let stored: Option<String> = self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
            .optional()?;
        Ok(stored.and_then(|s| serde_json::from_str(&s).ok()).or_else(|| default_setting(key)))
    }

    /// A setting as `T`; a stored value of the wrong shape falls back to the default.
    pub fn setting<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let parse = |v: Value| serde_json::from_value::<T>(v).ok();
        Ok(self
            .setting_value(key)?
            .and_then(parse)
            .or_else(|| default_setting(key).and_then(parse)))
    }

    /// Stores a setting. Values that wouldn't read back (nested deeper than the JSON
    /// parser allows) are refused.
    pub fn set_setting<T: Serialize>(&self, key: &str, value: &T, now: i64) -> Result<()> {
        let json =
            serde_json::to_string(value).map_err(|e| StoreError::InvalidData(e.to_string()))?;
        if let Err(e) = serde_json::from_str::<Value>(&json) {
            return Err(StoreError::InvalidData(format!("setting {key}: {e}")));
        }
        self.conn.execute(
            "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![key, json, now],
        )?;
        Ok(())
    }

    /// Every known setting with defaults filled in, plus any stored unknown keys (not the
    /// beta install id).
    pub fn settings_snapshot(&self) -> Result<serde_json::Map<String, Value>> {
        let mut out = serde_json::Map::new();
        for key in KEYS {
            out.insert(key.to_owned(), self.setting_value(key)?.unwrap_or(Value::Null));
        }
        let mut stmt = self.conn.prepare("SELECT key, value FROM settings")?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            let key: String = r.get(0)?;
            // The removed beta export's install id (P2.7) may still be stored: it isn't a
            // setting, so it's never sent to the windows.
            if !out.contains_key(&key) && key != "beta_install_id" {
                let value: String = r.get(1)?;
                out.insert(key, serde_json::from_str(&value).unwrap_or(Value::Null));
            }
        }
        Ok(out)
    }

    /// The mode; a stored value that isn't a mode reads as `shadow` (the safe default).
    pub fn mode(&self) -> Result<Mode> {
        Ok(self.setting("mode")?.unwrap_or(Mode::Shadow))
    }

    /// The global thresholds; invalid stored ones read as the Balanced preset.
    pub fn thresholds(&self) -> Result<Thresholds> {
        Ok(self
            .setting::<Thresholds>("thresholds")?
            .filter(Thresholds::is_valid)
            .unwrap_or_default())
    }

    pub fn hold_until(&self) -> Result<Option<i64>> {
        Ok(self.setting::<Option<i64>>("hold_until")?.flatten())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::*;

    #[test]
    fn reads_defaults_and_stored_values() {
        let (store, _dir) = store();
        assert_eq!(store.mode().unwrap(), Mode::Shadow);
        assert_eq!(store.thresholds().unwrap(), Thresholds::BALANCED);
        assert_eq!(store.setting::<i64>("shadow_started_at").unwrap(), Some(NOW));
        assert_eq!(store.setting::<u32>("retention_days").unwrap(), Some(90));
        assert_eq!(store.setting_value("nope").unwrap(), None);

        store.set_setting("mode", &Mode::Automatic, NOW).unwrap();
        store.set_setting("hold_until", &Some(NOW + 60_000), NOW).unwrap();
        store
            .set_setting("thresholds", &json!({"stale": 50, "hidden": 1, "archive": 2}), NOW)
            .unwrap();
        assert_eq!(store.mode().unwrap(), Mode::Automatic);
        assert_eq!(store.hold_until().unwrap(), Some(NOW + 60_000));
        assert_eq!(store.thresholds().unwrap(), Thresholds::BALANCED, "invalid → default");

        store.set_setting("mode", &"turbo", NOW).unwrap();
        assert_eq!(store.mode().unwrap(), Mode::Shadow, "unknown → safe default");

        // Values that can't be parsed back never stop settings from loading.
        let mut deep = json!(1);
        for _ in 0..200 {
            deep = json!([deep]);
        }
        assert!(matches!(store.set_setting("mode", &deep, NOW), Err(StoreError::InvalidData(_))));
        store.conn.execute("UPDATE settings SET value = '1e999' WHERE key = 'mode'", []).unwrap();
        assert_eq!(store.mode().unwrap(), Mode::Shadow);
        assert_eq!(store.settings_snapshot().unwrap()["mode"], json!("shadow"));

        let snapshot = store.settings_snapshot().unwrap();
        assert_eq!(snapshot.len(), KEYS.len());
        assert_eq!(snapshot["retention_days"], json!(90));
    }

    #[test]
    fn shadow_start_is_kept_across_opens() {
        let (store, dir) = store();
        drop(store);
        let paths = crate::StorePaths::in_dir(dir.path());
        let (store, _) = Store::open(paths, key(), NOW + 99).unwrap();
        assert_eq!(store.setting::<i64>("shadow_started_at").unwrap(), Some(NOW));
    }
}
