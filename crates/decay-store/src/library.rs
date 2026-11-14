//! Queries behind the Library window (P1.11, PRD F2/F7/F22): the stat cards, sweep
//! numbering, and the events the Shadow review and the sweep history are built from.

use decay_core::{EventType, ItemKind, SqlEnum};
use rusqlite::params;

use crate::events::EventRecord;
use crate::items::sql_enum;
use crate::sweeps::Sweep;
use crate::{Result, Store, StoreError};

/// What a rule matches an item on (see [`Store::item_targets`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemTarget {
    pub id: String,
    pub state: decay_core::ItemState,
    pub app_id: String,
    pub url: Option<String>,
    pub title: String,
}

impl Store {
    /// Archived tabs and windows (still archived) archived at or after `since`.
    pub fn archived_since(&self, since: i64) -> Result<(u32, u32)> {
        let count = |kind: ItemKind| -> Result<u32> {
            Ok(self.conn.prepare_cached(
                "SELECT COUNT(*) FROM items WHERE state = 'archived' AND kind = ?1 AND archived_at >= ?2",
            )?
            .query_row(params![kind.as_sql(), since], |r| r.get(0))?)
        };
        Ok((count(ItemKind::Tab)?, count(ItemKind::Window)?))
    }

    /// Distinct items with an event of one of `kinds` at or after `since`.
    pub fn items_with_event_since(&self, kinds: &[EventType], since: i64) -> Result<u32> {
        let mut n = 0;
        let mut seen = std::collections::HashSet::new();
        let mut stmt = self
            .conn
            .prepare_cached("SELECT DISTINCT item_id FROM events WHERE type = ?1 AND at >= ?2")?;
        for kind in kinds {
            let mut rows = stmt.query(params![kind.as_sql(), since])?;
            while let Some(r) = rows.next()? {
                if seen.insert(r.get::<_, String>(0)?) {
                    n += 1;
                }
            }
        }
        Ok(n)
    }

    /// Distinct items with an event of `kind` in `[from, to)`, by item kind (tab, window).
    pub fn items_with_event_between(
        &self,
        kind: EventType,
        from: i64,
        to: i64,
    ) -> Result<(u32, u32)> {
        let count = |item_kind: ItemKind| -> Result<u32> {
            Ok(self
                .conn
                .prepare_cached(
                    "SELECT COUNT(DISTINCT e.item_id) FROM events e JOIN items i ON i.id = e.item_id
                     WHERE e.type = ?1 AND e.at >= ?2 AND e.at < ?3 AND i.kind = ?4",
                )?
                .query_row(params![kind.as_sql(), from, to, item_kind.as_sql()], |r| r.get(0))?)
        };
        Ok((count(ItemKind::Tab)?, count(ItemKind::Window)?))
    }

    /// Distinct items with an event of any of `kinds` in `[from, to)` (an item restored
    /// and undone the same day counts once).
    pub fn items_with_any_event_between(
        &self,
        kinds: &[EventType],
        from: i64,
        to: i64,
    ) -> Result<u32> {
        let mut seen = std::collections::HashSet::new();
        let mut stmt = self.conn.prepare_cached(
            "SELECT DISTINCT item_id FROM events WHERE type = ?1 AND at >= ?2 AND at < ?3",
        )?;
        for kind in kinds {
            let mut rows = stmt.query(params![kind.as_sql(), from, to])?;
            while let Some(r) = rows.next()? {
                seen.insert(r.get::<_, String>(0)?);
            }
        }
        Ok(u32::try_from(seen.len()).unwrap_or(u32::MAX))
    }

    /// Sites (item domains) with at least `min` events of `kind` since `since`, most first:
    /// (domain, count). At most `limit`.
    pub fn domains_with_events_since(
        &self,
        kind: EventType,
        since: i64,
        min: u32,
        limit: usize,
    ) -> Result<Vec<(String, u32)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT i.domain, COUNT(*) AS n FROM events e JOIN items i ON i.id = e.item_id
             WHERE e.type = ?1 AND e.at >= ?2 AND i.domain IS NOT NULL
             GROUP BY i.domain HAVING n >= ?3 ORDER BY n DESC, i.domain LIMIT ?4",
        )?;
        let rows = stmt
            .query_map(params![kind.as_sql(), since, min, limit], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// When `item_id` last had an event of `kind`.
    pub fn last_event_at(&self, item_id: &str, kind: EventType) -> Result<Option<i64>> {
        Ok(self
            .conn
            .prepare_cached("SELECT MAX(at) FROM events WHERE item_id = ?1 AND type = ?2")?
            .query_row(params![item_id, kind.as_sql()], |r| r.get(0))?)
    }

    /// Per item with an event of `kind` at or after `since`: the first time, and the
    /// reason of its latest such event. Aggregated in SQL (shadow mode writes one event
    /// per item per stale period, but older databases repeated it every sweep).
    pub fn first_events_since(
        &self,
        kind: EventType,
        since: i64,
    ) -> Result<Vec<(String, i64, Option<decay_core::Reason>)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT e.item_id, MIN(e.at),
               (SELECT l.reason FROM events l WHERE l.item_id = e.item_id AND l.type = ?1
                  AND l.at >= ?2 ORDER BY l.at DESC, l.id DESC LIMIT 1)
             FROM events e WHERE e.type = ?1 AND e.at >= ?2
             GROUP BY e.item_id",
        )?;
        let mut rows = stmt.query(params![kind.as_sql(), since])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            let reason: Option<String> = r.get(2)?;
            out.push((
                r.get(0)?,
                r.get(1)?,
                reason
                    .map(|s| serde_json::from_str(&s))
                    .transpose()
                    .map_err(|e| StoreError::InvalidData(e.to_string()))?,
            ));
        }
        Ok(out)
    }

    /// Events of `kind` at or after `since` (optionally of one sweep), oldest first, at
    /// most `limit`.
    pub fn events_since(
        &self,
        kind: EventType,
        since: i64,
        sweep_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<EventRecord>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, item_id, type, sweep_id, reason, at FROM events
             WHERE type = ?1 AND at >= ?2 AND (?3 IS NULL OR sweep_id = ?3)
             ORDER BY at, id LIMIT ?4",
        )?;
        let mut rows = stmt.query(params![kind.as_sql(), since, sweep_id, limit])?;
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

    /// Every item's id, state, app, URL and title (what rules match on), for purging an
    /// excluded site and for testing a rule against the archive.
    pub fn item_targets(&self) -> Result<Vec<ItemTarget>> {
        let mut stmt =
            self.conn.prepare_cached("SELECT id, state, app_id, url, title FROM items")?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            out.push(ItemTarget {
                id: r.get(0)?,
                state: sql_enum(&r.get::<_, String>(1)?)?,
                app_id: r.get(2)?,
                url: r.get(3)?,
                title: r.get(4)?,
            });
        }
        Ok(out)
    }

    /// A sweep's number ("Sweep #409"): its place among the sweeps of its kind (real or
    /// shadow), oldest first.
    pub fn sweep_number(&self, sweep: &Sweep) -> Result<u32> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT COUNT(*) FROM sweeps
                 WHERE is_shadow = ?3 AND (started_at < ?1 OR (started_at = ?1 AND id <= ?2))",
            )?
            .query_row(params![sweep.started_at, sweep.id, sweep.is_shadow], |r| r.get(0))?)
    }
}

#[cfg(test)]
mod tests {
    use decay_core::{ItemState, Reason, SweepStatus, SweepTrigger};

    use crate::items::Item;
    use crate::new_id;
    use crate::test_util::*;

    use super::*;

    fn item(store: &Store, kind: ItemKind, state: ItemState, archived_at: Option<i64>) -> Item {
        let mut i = Item::new_live(new_id(NOW), kind, "com.google.Chrome", NOW);
        i.title = "t".into();
        i.state = state;
        i.archived_at = archived_at;
        store.insert_item(&i).unwrap();
        i
    }

    fn sweep(store: &Store, id: &str, at: i64) -> Sweep {
        let s = Sweep {
            id: id.into(),
            started_at: at,
            finished_at: Some(at),
            trigger: SweepTrigger::Scheduled,
            is_shadow: true,
            status: SweepStatus::Done,
            item_count: 1,
            failed_count: 0,
            reason: None,
            undone_at: None,
        };
        store.insert_sweep(&s).unwrap();
        s
    }

    #[test]
    fn counts_archived_items_by_kind_since() {
        let (store, _dir) = store();
        item(&store, ItemKind::Tab, ItemState::Archived, Some(NOW + 10));
        item(&store, ItemKind::Tab, ItemState::Archived, Some(NOW + 20));
        item(&store, ItemKind::Window, ItemState::Archived, Some(NOW + 20));
        item(&store, ItemKind::Tab, ItemState::Archived, Some(NOW - 10)); // before
        item(&store, ItemKind::Tab, ItemState::Restored, Some(NOW + 10)); // not archived now
        item(&store, ItemKind::Tab, ItemState::Active, None);
        assert_eq!(store.archived_since(NOW).unwrap(), (2, 1));
        assert_eq!(store.archived_since(NOW + 15).unwrap(), (1, 1));
    }

    #[test]
    fn events_by_type_time_and_sweep() {
        let (store, _dir) = store();
        let a = item(&store, ItemKind::Tab, ItemState::Stale, None);
        let b = item(&store, ItemKind::Tab, ItemState::Stale, None);
        let reason = Reason::Unused { active_hours: 61, threshold: 60.0 };
        let shadow = EventType::ShadowWouldArchive;
        for id in ["s0", "s1", "s2"] {
            sweep(&store, id, NOW);
        }
        store.append_event(&a.id, shadow, Some("s1"), Some(&reason), NOW + 1).unwrap();
        store.append_event(&a.id, shadow, Some("s2"), Some(&reason), NOW + 2).unwrap();
        store.append_event(&b.id, shadow, Some("s2"), None, NOW + 3).unwrap();
        store.append_event(&b.id, EventType::Restored, None, None, NOW + 4).unwrap();
        store.append_event(&a.id, shadow, Some("s0"), None, NOW - 5).unwrap();

        let all = store.events_since(shadow, NOW, None, 100).unwrap();
        assert_eq!(all.iter().map(|e| e.at).collect::<Vec<_>>(), [NOW + 1, NOW + 2, NOW + 3]);
        assert_eq!(all[0].reason, Some(reason));
        let s2 = store.events_since(shadow, NOW, Some("s2"), 100).unwrap();
        assert_eq!(s2.len(), 2);
        assert_eq!(store.events_since(shadow, NOW, None, 1).unwrap().len(), 1);
        assert_eq!(store.items_with_event_since(&[shadow], NOW).unwrap(), 2);
        assert_eq!(store.items_with_event_since(&[EventType::Restored], NOW).unwrap(), 1);
        assert_eq!(store.items_with_event_since(&[EventType::Restored], NOW + 5).unwrap(), 0);
        // Several kinds: each item once.
        store.append_event(&b.id, EventType::Undo, None, None, NOW + 6).unwrap();
        store.append_event(&a.id, EventType::Undo, None, None, NOW + 6).unwrap();
        let back = [EventType::Restored, EventType::Undo];
        assert_eq!(store.items_with_event_since(&back, NOW).unwrap(), 2);
        assert_eq!(store.last_event_at(&a.id, shadow).unwrap(), Some(NOW + 2));
        assert_eq!(store.last_event_at(&a.id, EventType::Hidden).unwrap(), None);
        // First time per item, with the latest reason.
        let mut firsts = store.first_events_since(shadow, NOW).unwrap();
        firsts.sort_by_key(|f| f.1);
        assert_eq!(firsts.len(), 2);
        assert_eq!((firsts[0].0.as_str(), firsts[0].1), (a.id.as_str(), NOW + 1));
        assert_eq!(firsts[0].2, Some(Reason::Unused { active_hours: 61, threshold: 60.0 }));
        assert_eq!((firsts[1].1, &firsts[1].2), (NOW + 3, &None));
    }

    #[test]
    fn item_targets_list_every_item() {
        let (store, _dir) = store();
        let a = item(&store, ItemKind::Tab, ItemState::Archived, Some(NOW));
        item(&store, ItemKind::Window, ItemState::Active, None);
        let t = store.item_targets().unwrap();
        assert_eq!(t.len(), 2);
        let ta = t.iter().find(|x| x.id == a.id).unwrap();
        assert_eq!(
            (ta.state, ta.app_id.as_str(), ta.title.as_str()),
            (ItemState::Archived, "com.google.Chrome", "t")
        );
    }

    #[test]
    fn sweeps_are_numbered_oldest_first() {
        let (store, _dir) = store();
        let first = sweep(&store, "b", NOW);
        let same_time = sweep(&store, "c", NOW);
        let later = sweep(&store, "a", NOW + 1);
        assert_eq!(store.sweep_number(&first).unwrap(), 1);
        assert_eq!(store.sweep_number(&same_time).unwrap(), 2);
        assert_eq!(store.sweep_number(&later).unwrap(), 3);
        // Real sweeps are numbered on their own.
        let real = Sweep { id: "r".into(), is_shadow: false, started_at: NOW + 2, ..first.clone() };
        store.insert_sweep(&real).unwrap();
        assert_eq!(store.sweep_number(&real).unwrap(), 1);
    }

    #[test]
    fn daily_summary_queries_keep_to_their_range() {
        let (store, _dir) = store();
        let day = NOW;
        let end = day + 86_400_000;
        let tab = item(&store, ItemKind::Tab, ItemState::Archived, Some(day));
        let win = item(&store, ItemKind::Window, ItemState::Hidden, None);
        let late = item(&store, ItemKind::Tab, ItemState::Archived, Some(end)); // next day
        let gone = item(&store, ItemKind::Tab, ItemState::Restored, Some(day + 5));
        store.append_event(&tab.id, EventType::Archived, None, None, day).unwrap(); // first ms: in
        store.append_event(&late.id, EventType::Archived, None, None, end).unwrap(); // `to`: out
        store.append_event(&win.id, EventType::Hidden, None, None, day + 1).unwrap();
        store.append_event(&gone.id, EventType::Restored, None, None, day + 2).unwrap();
        store.append_event(&gone.id, EventType::Undo, None, None, day + 3).unwrap();
        assert_eq!(store.items_with_event_between(EventType::Archived, day, end).unwrap(), (1, 0));
        assert_eq!(store.items_with_event_between(EventType::Hidden, day, end).unwrap(), (0, 1));
        let reopened = store
            .items_with_any_event_between(&[EventType::Restored, EventType::Undo], day, end)
            .unwrap();
        assert_eq!(reopened, 1, "restored and undone the same day: once");
        let listed = store.archived_between(day, end, 10).unwrap();
        assert_eq!(listed.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), [tab.id.as_str()]);
        assert_eq!(store.archived_between(day, end, 0).unwrap().len(), 0);
    }

    #[test]
    fn sites_with_repeated_events_skip_items_without_a_site() {
        let (store, _dir) = store();
        let mut ids = Vec::new();
        for (domain, n) in [(Some("a.com"), 3), (Some("b.com"), 1), (None, 5)] {
            let mut i = item(&store, ItemKind::Tab, ItemState::Restored, Some(NOW));
            i.domain = domain.map(Into::into);
            store.update_item(&i).unwrap();
            for k in 0..n {
                store.append_event(&i.id, EventType::Restored, None, None, NOW + k).unwrap();
            }
            ids.push(i.id);
        }
        let hits = store.domains_with_events_since(EventType::Restored, NOW, 2, 5).unwrap();
        assert_eq!(hits, vec![("a.com".to_owned(), 3)]);
        assert_eq!(
            store.domains_with_events_since(EventType::Restored, NOW, 1, 1).unwrap().len(),
            1
        );
        assert_eq!(
            store.domains_with_events_since(EventType::Restored, NOW + 10, 1, 5).unwrap(),
            vec![]
        );
    }
}
