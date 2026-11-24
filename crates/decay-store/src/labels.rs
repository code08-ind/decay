//! Batch labels (F15, Pro, off by default): a short name for each archived batch, made on
//! this computer. A batch is what one sweep archived from one window (`last_sweep_id`,
//! `source_id`), as ⌘↵ restores it.

use rusqlite::{OptionalExtension, params};

use crate::{Result, Store};

/// One archived item, as the labeller sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchItem {
    pub title: String,
    pub domain: Option<String>,
}

/// An archived batch without a label yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch {
    pub sweep_id: String,
    pub source_id: Option<String>,
    pub items: Vec<BatchItem>,
}

/// Batches this small aren't named (a single tab names itself).
pub const MIN_BATCH_ITEMS: usize = 2;
/// Items read per batch for its label.
const MAX_ITEMS_READ: usize = 30;

impl Store {
    /// Archived batches of at least [`MIN_BATCH_ITEMS`] items archived since `since`
    /// without a label, newest first, at most `limit`.
    pub fn unlabeled_batches(&self, since: i64, limit: usize) -> Result<Vec<Batch>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT i.last_sweep_id, i.source_id
             FROM items i
             WHERE i.state = 'archived' AND i.last_sweep_id IS NOT NULL AND i.archived_at >= ?1
               AND NOT EXISTS (SELECT 1 FROM batch_labels b
                               WHERE b.sweep_id = i.last_sweep_id
                                 AND b.source_key = coalesce(i.source_id, ''))
             GROUP BY i.last_sweep_id, i.source_id
             HAVING count(*) >= ?2
             ORDER BY max(i.archived_at) DESC
             LIMIT ?3",
        )?;
        let keys: Vec<(String, Option<String>)> = stmt
            .query_map(params![since, MIN_BATCH_ITEMS, limit], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        let mut items = self.conn.prepare_cached(
            "SELECT title, domain FROM items
             WHERE last_sweep_id = ?1 AND source_id IS ?2 AND state = 'archived'
             ORDER BY tab_index LIMIT ?3",
        )?;
        keys.into_iter()
            .map(|(sweep_id, source_id)| {
                let items = items
                    .query_map(params![sweep_id, source_id, MAX_ITEMS_READ], |r| {
                        Ok(BatchItem { title: r.get(0)?, domain: r.get(1)? })
                    })?
                    .collect::<std::result::Result<_, _>>()?;
                Ok(Batch { sweep_id, source_id, items })
            })
            .collect()
    }

    /// Saves (or replaces) a batch's label.
    pub fn set_batch_label(
        &self,
        sweep_id: &str,
        source_id: Option<&str>,
        label: &str,
        model: &str,
        now: i64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO batch_labels (sweep_id, source_key, label, model, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (sweep_id, source_key) DO UPDATE SET label = excluded.label,
               model = excluded.model, created_at = excluded.created_at",
            params![sweep_id, source_id.unwrap_or(""), label, model, now],
        )?;
        Ok(())
    }

    /// A batch's label, if it has one.
    pub fn batch_label(&self, sweep_id: &str, source_id: Option<&str>) -> Result<Option<String>> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT label FROM batch_labels WHERE sweep_id = ?1 AND source_key = ?2",
            )?
            .query_row(params![sweep_id, source_id.unwrap_or("")], |r| r.get(0))
            .optional()?)
    }

    /// Forgets labels whose batch has nothing archived any more (restored or purged).
    pub fn prune_batch_labels(&self) -> Result<usize> {
        Ok(self.conn.execute(
            "DELETE FROM batch_labels WHERE NOT EXISTS (
               SELECT 1 FROM items i WHERE i.last_sweep_id = batch_labels.sweep_id
                 AND coalesce(i.source_id, '') = batch_labels.source_key
                 AND i.state = 'archived')",
            [],
        )?)
    }

    /// Forgets every label (labels turned off).
    pub fn clear_batch_labels(&self) -> Result<usize> {
        Ok(self.conn.execute("DELETE FROM batch_labels", [])?)
    }
}

#[cfg(test)]
mod tests {
    use decay_core::{ItemKind, ItemState, SweepStatus, SweepTrigger};

    use super::*;
    use crate::test_util::{NOW, store};
    use crate::{Item, Source, Sweep};

    const CHROME: &str = "com.google.Chrome";

    fn setup(s: &Store) {
        for id in ["w1", "w2"] {
            s.upsert_source(&Source {
                id: id.into(),
                app_id: CHROME.into(),
                profile: None,
                external_id: None,
                title: None,
                first_seen_at: NOW,
                last_seen_at: NOW,
                closed_at: None,
            })
            .unwrap();
        }
        s.insert_sweep(&Sweep {
            id: "s1".into(),
            started_at: NOW,
            finished_at: Some(NOW),
            trigger: SweepTrigger::Scheduled,
            is_shadow: false,
            status: SweepStatus::Done,
            item_count: 4,
            failed_count: 0,
            reason: None,
            undone_at: None,
        })
        .unwrap();
        // Window 1: three tabs; window 2: one tab (too small to name).
        for (i, (id, source, title)) in [
            ("a", "w1", "Flights to Lisbon"),
            ("b", "w1", "Hotels in Alfama"),
            ("c", "w1", "Lisbon tram map"),
            ("d", "w2", "Rust book"),
        ]
        .into_iter()
        .enumerate()
        {
            let mut item = Item::new_live(id.into(), ItemKind::Tab, CHROME, NOW);
            item.title = title.into();
            item.domain = Some("example.com".into());
            item.source_id = Some(source.into());
            item.tab_index = i64::try_from(i).ok();
            item.state = ItemState::Archived;
            item.archived_at = Some(NOW);
            item.last_sweep_id = Some("s1".into());
            s.insert_item(&item).unwrap();
        }
    }

    #[test]
    fn batches_are_found_named_and_forgotten() {
        let (s, _dir) = store();
        setup(&s);
        let batches = s.unlabeled_batches(NOW - 1, 10).unwrap();
        assert_eq!(batches.len(), 1, "the one-tab window isn't a batch to name");
        let b = &batches[0];
        assert_eq!((b.sweep_id.as_str(), b.source_id.as_deref()), ("s1", Some("w1")));
        let titles: Vec<&str> = b.items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, ["Flights to Lisbon", "Hotels in Alfama", "Lisbon tram map"]);
        assert_eq!(s.unlabeled_batches(NOW + 1, 10).unwrap(), Vec::new(), "older than `since`");

        s.set_batch_label("s1", Some("w1"), "Lisbon trip", "apple", NOW).unwrap();
        assert_eq!(s.batch_label("s1", Some("w1")).unwrap().as_deref(), Some("Lisbon trip"));
        assert_eq!(s.batch_label("s1", None).unwrap(), None);
        assert_eq!(s.unlabeled_batches(NOW - 1, 10).unwrap(), Vec::new(), "labelled");
        s.set_batch_label("s1", Some("w1"), "Lisbon", "keywords", NOW).unwrap();
        assert_eq!(s.batch_label("s1", Some("w1")).unwrap().as_deref(), Some("Lisbon"));
        assert!(s.set_batch_label("s1", None, "", "apple", NOW).is_err(), "empty label");
        assert!(s.set_batch_label("s1", None, "x", "gpt", NOW).is_err(), "unknown model");

        assert_eq!(s.prune_batch_labels().unwrap(), 0, "still archived");
        for id in ["a", "b", "c"] {
            s.set_item_state(id, ItemState::Restored, NOW).unwrap();
        }
        assert_eq!(s.prune_batch_labels().unwrap(), 1, "nothing archived any more");
        s.set_batch_label("s1", Some("w2"), "Rust", "keywords", NOW).unwrap();
        assert_eq!(s.clear_batch_labels().unwrap(), 1);
    }
}
