//! Sweeps and their items (DB Design §3.6–3.7).

use decay_core::{ItemState, SqlEnum, SweepAction, SweepItemResult, SweepStatus, SweepTrigger};
use rusqlite::{OptionalExtension, Row, params};

use crate::items::sql_enum;
use crate::{Result, Store, StoreError};

/// One sweep (or a shadow-mode simulation of one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sweep {
    pub id: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub trigger: SweepTrigger,
    pub is_shadow: bool,
    pub status: SweepStatus,
    pub item_count: u32,
    pub failed_count: u32,
    /// Summary JSON, e.g. `{"preset":"balanced"}`.
    pub reason: Option<String>,
    pub undone_at: Option<i64>,
}

/// What a sweep did to one item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SweepItem {
    pub sweep_id: String,
    pub item_id: String,
    pub action: SweepAction,
    /// The state to go back to on Undo.
    pub prev_state: ItemState,
    pub result: SweepItemResult,
}

fn sweep_from_row(r: &Row<'_>) -> Result<Sweep> {
    Ok(Sweep {
        id: r.get(0)?,
        started_at: r.get(1)?,
        finished_at: r.get(2)?,
        trigger: sql_enum(&r.get::<_, String>(3)?)?,
        is_shadow: r.get(4)?,
        status: sql_enum(&r.get::<_, String>(5)?)?,
        item_count: r.get(6)?,
        failed_count: r.get(7)?,
        reason: r.get(8)?,
        undone_at: r.get(9)?,
    })
}

const SWEEP_COLUMNS: &str = "id, started_at, finished_at, trigger, is_shadow, status, item_count, failed_count, reason, undone_at";

impl Store {
    pub fn insert_sweep(&self, sweep: &Sweep) -> Result<()> {
        self.conn.execute(
            &format!("INSERT INTO sweeps ({SWEEP_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"),
            params![
                sweep.id,
                sweep.started_at,
                sweep.finished_at,
                sweep.trigger.as_sql(),
                sweep.is_shadow,
                sweep.status.as_sql(),
                sweep.item_count,
                sweep.failed_count,
                sweep.reason,
                sweep.undone_at
            ],
        )?;
        Ok(())
    }

    /// Saves a sweep's progress fields (finish time, status, counts, undo time).
    pub fn update_sweep(&self, sweep: &Sweep) -> Result<()> {
        let n = self.conn.execute(
            "UPDATE sweeps SET finished_at = ?2, status = ?3, item_count = ?4, failed_count = ?5,
               reason = ?6, undone_at = ?7 WHERE id = ?1",
            params![
                sweep.id,
                sweep.finished_at,
                sweep.status.as_sql(),
                sweep.item_count,
                sweep.failed_count,
                sweep.reason,
                sweep.undone_at
            ],
        )?;
        if n == 0 { Err(StoreError::NotFound) } else { Ok(()) }
    }

    pub fn sweep(&self, id: &str) -> Result<Option<Sweep>> {
        self.conn
            .query_row(&format!("SELECT {SWEEP_COLUMNS} FROM sweeps WHERE id = ?1"), [id], |r| {
                Ok(sweep_from_row(r))
            })
            .optional()?
            .transpose()
    }

    /// The newest sweeps first.
    pub fn recent_sweeps(&self, limit: usize) -> Result<Vec<Sweep>> {
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT {SWEEP_COLUMNS} FROM sweeps ORDER BY started_at DESC, id DESC LIMIT ?1"
        ))?;
        let mut rows = stmt.query([limit])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            out.push(sweep_from_row(r)?);
        }
        Ok(out)
    }

    /// Sweeps left `pending` / `running` by a crash (to resume or fail at start-up).
    pub fn unfinished_sweeps(&self) -> Result<Vec<Sweep>> {
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT {SWEEP_COLUMNS} FROM sweeps WHERE status IN ('pending', 'running') ORDER BY started_at"
        ))?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            out.push(sweep_from_row(r)?);
        }
        Ok(out)
    }

    /// Adds or replaces one item of a sweep.
    pub fn put_sweep_item(&self, item: &SweepItem) -> Result<()> {
        self.conn
            .prepare_cached(
                "INSERT INTO sweep_items (sweep_id, item_id, action, prev_state, result)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (sweep_id, item_id) DO UPDATE SET action = excluded.action,
                   prev_state = excluded.prev_state, result = excluded.result",
            )?
            .execute(params![
                item.sweep_id,
                item.item_id,
                item.action.as_sql(),
                item.prev_state.as_sql(),
                item.result.as_sql()
            ])?;
        Ok(())
    }

    pub fn sweep_items(&self, sweep_id: &str) -> Result<Vec<SweepItem>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT sweep_id, item_id, action, prev_state, result FROM sweep_items
             WHERE sweep_id = ?1 ORDER BY rowid",
        )?;
        let mut rows = stmt.query([sweep_id])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            out.push(SweepItem {
                sweep_id: r.get(0)?,
                item_id: r.get(1)?,
                action: sql_enum(&r.get::<_, String>(2)?)?,
                prev_state: sql_enum(&r.get::<_, String>(3)?)?,
                result: sql_enum(&r.get::<_, String>(4)?)?,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::Item;
    use crate::new_id;
    use crate::test_util::*;
    use decay_core::ItemKind;

    #[test]
    fn records_a_sweep_and_its_items() {
        let (store, _dir) = store();
        let item = Item::new_live(new_id(NOW), ItemKind::Tab, "com.google.Chrome", NOW);
        store.insert_item(&item).unwrap();
        let mut sweep = Sweep {
            id: new_id(NOW),
            started_at: NOW,
            finished_at: None,
            trigger: SweepTrigger::Scheduled,
            is_shadow: false,
            status: SweepStatus::Running,
            item_count: 0,
            failed_count: 0,
            reason: Some(r#"{"preset":"balanced"}"#.into()),
            undone_at: None,
        };
        store.insert_sweep(&sweep).unwrap();
        assert_eq!(store.unfinished_sweeps().unwrap(), vec![sweep.clone()]);
        let mut si = SweepItem {
            sweep_id: sweep.id.clone(),
            item_id: item.id.clone(),
            action: SweepAction::Archive,
            prev_state: ItemState::Stale,
            result: SweepItemResult::Pending,
        };
        store.put_sweep_item(&si).unwrap();
        si.result = SweepItemResult::Ok;
        store.put_sweep_item(&si).unwrap();
        assert_eq!(store.sweep_items(&sweep.id).unwrap(), vec![si]);

        sweep.status = SweepStatus::Done;
        sweep.finished_at = Some(NOW + 10);
        sweep.item_count = 1;
        store.update_sweep(&sweep).unwrap();
        assert_eq!(store.sweep(&sweep.id).unwrap(), Some(sweep.clone()));
        assert_eq!(store.unfinished_sweeps().unwrap().len(), 0);
        assert_eq!(store.recent_sweeps(5).unwrap(), vec![sweep]);
    }
}
