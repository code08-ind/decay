//! What "Export diagnostics" (P2.6, PRD F29) says about the database: counts only. No
//! URL, title, path, site, rule pattern or id ever leaves through here.

use serde::Serialize;
use serde_json::{Map, Value};

use crate::backup::BackupKind;
use crate::{Result, Store};

/// Counts that describe the database without its content.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreDiagnostics {
    pub schema_version: u32,
    /// Database plus its WAL.
    pub db_bytes: u64,
    /// `"<kind> <state>"` → items (e.g. `"tab archived"`).
    pub items: Map<String, Value>,
    /// `"<source> <action> on|off"` → rules (e.g. `"user exclude on"`).
    pub rules: Map<String, Value>,
    /// `"<real|shadow> <status>"` → sweeps.
    pub sweeps: Map<String, Value>,
    /// Sweeps that started in the last 7 days, and how many items they failed on.
    pub sweeps_last_week: u64,
    pub sweep_failures_last_week: u64,
    pub events: u64,
    pub sources: u64,
    /// Every backup: what it was for, when, how large.
    pub backups: Vec<BackupSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupSummary {
    pub kind: &'static str,
    pub at: i64,
    pub bytes: u64,
}

impl Store {
    /// Counts for a diagnostics report (see the module docs).
    pub fn diagnostics(&self, now: i64) -> Result<StoreDiagnostics> {
        let grouped = |sql: &str| -> Result<Map<String, Value>> {
            let mut stmt = self.conn.prepare(sql)?;
            let mut out = Map::new();
            for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
                let (key, n) = row?;
                out.insert(key, Value::from(n));
            }
            Ok(out)
        };
        let count = |sql: &str, arg: i64| -> Result<u64> {
            let n: i64 = self.conn.query_row(sql, [arg], |r| r.get(0))?;
            Ok(u64::try_from(n).unwrap_or(0))
        };
        let total = |sql: &str| -> Result<u64> {
            let n: i64 = self.conn.query_row(sql, [], |r| r.get(0))?;
            Ok(u64::try_from(n).unwrap_or(0))
        };
        let week_ago = now - 7 * 86_400_000;
        let backups = self
            .backups()?
            .into_iter()
            .map(|b| BackupSummary {
                kind: match b.kind {
                    BackupKind::Daily => "daily",
                    BackupKind::PreMigration => "preMigration",
                    BackupKind::PreRestore => "preRestore",
                    BackupKind::PreImport => "preImport",
                },
                at: b.at,
                bytes: b.bytes,
            })
            .collect();
        Ok(StoreDiagnostics {
            schema_version: crate::SCHEMA_VERSION,
            db_bytes: self.db_bytes()?,
            items: grouped(
                "SELECT kind || ' ' || state, count(*) FROM items GROUP BY kind, state ORDER BY 1",
            )?,
            rules: grouped(
                "SELECT source || ' ' || action || CASE WHEN enabled THEN ' on' ELSE ' off' END,
                        count(*) FROM rules GROUP BY 1 ORDER BY 1",
            )?,
            sweeps: grouped(
                "SELECT CASE WHEN is_shadow THEN 'shadow ' ELSE 'real ' END || status, count(*)
                 FROM sweeps GROUP BY 1 ORDER BY 1",
            )?,
            sweeps_last_week: count(
                "SELECT count(*) FROM sweeps WHERE started_at >= ?1",
                week_ago,
            )?,
            sweep_failures_last_week: count(
                "SELECT coalesce(sum(failed_count), 0) FROM sweeps WHERE started_at >= ?1",
                week_ago,
            )?,
            events: total("SELECT count(*) FROM events")?,
            sources: total("SELECT count(*) FROM sources")?,
            backups,
        })
    }
}

#[cfg(test)]
mod tests {
    use decay_core::{ItemKind, ItemState};

    use crate::items::Item;
    use crate::test_util::*;

    #[test]
    fn counts_without_content() {
        let (store, _dir) = store();
        let mut tab = Item::new_live("t1".into(), ItemKind::Tab, "com.google.Chrome", NOW);
        tab.title = "My bank statement".into();
        tab.url = Some("https://bank.example/secret".into());
        store.insert_item(&tab).unwrap();
        tab.id = "t2".into();
        tab.state = ItemState::Archived;
        store.insert_item(&tab).unwrap();
        let d = store.diagnostics(NOW).unwrap();
        assert_eq!(d.items["tab active"], 1);
        assert_eq!(d.items["tab archived"], 1);
        assert!(d.rules.keys().any(|k| k.starts_with("default protect")), "{:?}", d.rules);
        assert!(d.db_bytes > 0);
        let text = serde_json::to_string(&d).unwrap();
        for secret in ["bank", "statement", "t1", "zoom"] {
            assert!(!text.contains(secret), "{secret} in {text}");
        }
    }
}
