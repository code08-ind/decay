//! The append-only event log (DB Design §3.5).

use decay_core::{EventType, Reason, SqlEnum};
use rusqlite::params;

use crate::items::sql_enum;
use crate::{Result, Store, StoreError};

/// `used` events are sampled: at most one per item per 10 minutes.
pub const USED_SAMPLE_MS: i64 = 10 * 60 * 1000;

/// One logged event.
#[derive(Debug, Clone, PartialEq)]
pub struct EventRecord {
    pub id: i64,
    pub item_id: String,
    pub kind: EventType,
    pub sweep_id: Option<String>,
    pub reason: Option<Reason>,
    pub at: i64,
}

impl Store {
    /// Appends an event.
    pub fn append_event(
        &self,
        item_id: &str,
        kind: EventType,
        sweep_id: Option<&str>,
        reason: Option<&Reason>,
        at: i64,
    ) -> Result<i64> {
        let reason = reason
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| StoreError::InvalidData(e.to_string()))?;
        self.conn
            .prepare_cached(
                "INSERT INTO events (item_id, type, sweep_id, reason, at) VALUES (?1, ?2, ?3, ?4, ?5)",
            )?
            .execute(params![item_id, kind.as_sql(), sweep_id, reason, at])?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Logs a `used` event unless the item already has one in the last 10 minutes.
    /// Returns whether it was logged.
    pub fn record_used(&self, item_id: &str, at: i64) -> Result<bool> {
        let recent: bool = self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM events WHERE item_id = ?1 AND type = 'used' AND at > ?2)",
            params![item_id, at - USED_SAMPLE_MS],
            |r| r.get(0),
        )?;
        if recent {
            return Ok(false);
        }
        self.append_event(item_id, EventType::Used, None, None, at)?;
        Ok(true)
    }

    /// An item's events, newest first.
    pub fn events_for_item(&self, item_id: &str, limit: usize) -> Result<Vec<EventRecord>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, item_id, type, sweep_id, reason, at FROM events
             WHERE item_id = ?1 ORDER BY at DESC, id DESC LIMIT ?2",
        )?;
        let mut rows = stmt.query(params![item_id, limit])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            let reason: Option<String> = r.get(4)?;
            out.push(EventRecord {
                id: r.get(0)?,
                item_id: r.get(1)?,
                kind: sql_enum(&r.get::<_, String>(2)?)?,
                sweep_id: r.get(3)?,
                reason: reason
                    .map(|s| serde_json::from_str(&s))
                    .transpose()
                    .map_err(|e| StoreError::InvalidData(e.to_string()))?,
                at: r.get(5)?,
            });
        }
        Ok(out)
    }

    /// Weekly compaction: deletes `used` / `seen` events older than `cutoff`.
    pub fn compact_events(&self, cutoff: i64) -> Result<usize> {
        Ok(self
            .conn
            .execute("DELETE FROM events WHERE type IN ('used', 'seen') AND at < ?1", [cutoff])?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::Item;
    use crate::new_id;
    use crate::test_util::*;
    use decay_core::{ItemKind, ProtectedBy};

    #[test]
    fn logs_samples_and_compacts_events() {
        let (store, _dir) = store();
        let item = Item::new_live(new_id(NOW), ItemKind::Tab, "com.google.Chrome", NOW);
        store.insert_item(&item).unwrap();
        let reason = Reason::Protected { by: ProtectedBy::Pinned, rule_id: None };
        store.append_event(&item.id, EventType::Protected, None, Some(&reason), NOW).unwrap();
        assert!(store.record_used(&item.id, NOW).unwrap());
        assert!(!store.record_used(&item.id, NOW + USED_SAMPLE_MS - 1).unwrap());
        assert!(store.record_used(&item.id, NOW + USED_SAMPLE_MS + 1).unwrap());

        let events = store.events_for_item(&item.id, 10).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[2].reason, Some(reason));
        assert_eq!(store.compact_events(NOW + 1).unwrap(), 1);

        store.delete_item(&item.id).unwrap();
        assert_eq!(store.events_for_item(&item.id, 10).unwrap().len(), 0, "cascade");
    }
}
