//! Search, restore and Undo for the extension's popup in adapter mode (Software Design §4.5).
//!
//! - `archive.search` runs the store's FTS5 search (empty query: the most recent first).
//! - `archive.restore` reopens items in the popup's window; `archive.restoreBatch` reopens
//!   the batch an item was archived with into its original window (same browser session)
//!   or a new one; `archive.undo` reopens what a sweep archived, each tab in its own
//!   browser when that browser is connected.
//! - A reopened record becomes `restored` (an `undo` or `restored` event); the new tab's
//!   item links to it and gets 48 h of grace, so it isn't archived again right away. Undo
//!   marks the sweep undone once nothing of it is left archived; a partial undo can be
//!   retried for the rest.
//!
//! The reply (an ack with [`RestoreSummary`]) is sent once every browser answered.

use std::collections::{BTreeMap, HashMap};

use decay_bridge::ErrorCode;
use decay_bridge::kinds;
use decay_bridge::protocol::{
    Ack, ArchiveRestore, ArchiveRestoreBatch, ArchiveSearch, ArchiveSearchResult, ArchiveUndo,
    ArchivedItem, LastSweep, RestoreGroup, RestoreItem, RestoreResult, RestoreSummary, TabsRestore,
};
use decay_core::{EventType, ItemState, LifecycleEvent, SweepStatus, next_state};
use decay_store::{Item, SearchFilters, SearchQuery, Store, Sweep};

use crate::engine::{
    CommandKind, ConnId, Engine, EngineEvent, Output, Pending, Res, RestoreRefused, UndoRefused,
    timing,
};
use crate::sweep::plain_guards;
use crate::tabs::link_restored;

/// A restore request waiting for the browsers' acks.
#[derive(Debug)]
pub(crate) struct RestoreOp {
    /// The extension request to answer.
    reply: Option<(ConnId, String)>,
    waiting: usize,
    summary: RestoreSummary,
    /// Undo of this sweep.
    undo: Option<String>,
}

/// Where restored tabs go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Placement {
    /// Into this window of the requesting browser (the first tab active).
    Window(i64),
    /// Back where they were: the original window in the same browser session, else one
    /// new window per original window. `focus`: the first tab becomes active.
    Original { focus: bool },
    /// All together in one new window (the panel's ⌥↵), the first tab active.
    NewWindow,
}

fn archived_item(item: &Item, window_key: Option<String>) -> ArchivedItem {
    ArchivedItem {
        id: item.id.clone(),
        kind: item.kind,
        app_id: item.app_id.clone(),
        title: item.title.clone(),
        url: item.url.clone(),
        domain: item.domain.clone(),
        favicon_url: item.favicon_url.clone(),
        tab_index: item.tab_index,
        group_title: item.group_title.clone(),
        group_color: item.group_color.clone(),
        pinned: item.pinned,
        state: item.state,
        active_age_s: item.active_age_s,
        first_seen_at: item.first_seen_at,
        last_used_at: item.last_used_at,
        state_changed_at: item.state_changed_at,
        archived_at: item.archived_at,
        archive_reason: item.archive_reason.clone(),
        last_sweep_id: item.last_sweep_id.clone(),
        window_key,
        restore_count: item.restore_count,
    }
}

/// The sweeps Undo may revert: the last 20 real sweeps that archived something.
fn undoable(s: &Store) -> Res<Vec<Sweep>> {
    Ok(s.recent_sweeps(timing::UNDO_HISTORY * 5)?
        .into_iter()
        // Finished, real sweeps that archived something. A sweep still closing tabs isn't
        // offered: its tabs aren't archived yet, and a half undo would be marked done.
        .filter(|w| !w.is_shadow && w.item_count > 0)
        .filter(|w| matches!(w.status, SweepStatus::Done | SweepStatus::Undone))
        .take(timing::UNDO_HISTORY)
        .collect())
}

impl Engine {
    /// Ids of the sweeps Undo accepts (the Library offers Undo for exactly these).
    pub fn undoable_sweep_ids(&self) -> Res<std::collections::HashSet<String>> {
        Ok(undoable(&self.store)?.into_iter().map(|s| s.id).collect())
    }

    /// The item is being restored right now (its tab is reopening).
    pub fn is_restoring(&self, item_id: &str) -> bool {
        self.restoring.contains(item_id)
    }

    pub(crate) fn on_search(
        &mut self,
        conn: ConnId,
        req: &str,
        q: &ArchiveSearch,
        now: i64,
    ) -> Res {
        let page = self.store.search(&SearchQuery {
            text: q.query.clone(),
            filters: SearchFilters::default(),
            limit: q.limit.unwrap_or(50).clamp(1, 200) as usize,
            offset: 0,
            now,
            utc_offset_minutes: q.utc_offset_minutes.unwrap_or(0).clamp(-24 * 60, 24 * 60),
        })?;
        let mut keys: HashMap<String, Option<String>> = HashMap::new();
        let mut items = Vec::with_capacity(page.hits.len());
        for hit in &page.hits {
            let key = match &hit.item.source_id {
                Some(id) => {
                    if let Some(k) = keys.get(id) {
                        k.clone()
                    } else {
                        let k = self.store.source(id)?.and_then(|s| s.external_id);
                        keys.insert(id.clone(), k.clone());
                        k
                    }
                }
                None => None,
            };
            items.push(archived_item(&hit.item, key));
        }
        let last_sweep = if q.query.trim().is_empty() {
            let last = undoable(&self.store)?
                .into_iter()
                .find(|w| w.undone_at.is_none() && !crate::hide::is_hide_record(w));
            match last {
                Some(w) => Some(LastSweep {
                    remaining: u32::try_from(self.store.archived_in_sweep(&w.id)?.len())
                        .unwrap_or(u32::MAX),
                    id: w.id,
                    started_at: w.started_at,
                    finished_at: w.finished_at,
                    item_count: w.item_count,
                }),
                None => None,
            }
        } else {
            None
        };
        let result = ArchiveSearchResult {
            items,
            more: page.next_offset.is_some(),
            last_sweep,
            mode: Some(self.store.mode()?),
        };
        self.reply_ok(conn, req, &result);
        Ok(())
    }

    pub(crate) fn on_restore(
        &mut self,
        conn: ConnId,
        req: &str,
        r: ArchiveRestore,
        now: i64,
    ) -> Res {
        let mut items = Vec::new();
        let mut skipped = 0;
        let mut ids = r.item_ids;
        ids.truncate(500);
        let mut seen = std::collections::HashSet::new();
        ids.retain(|id| seen.insert(id.clone()));
        for id in &ids {
            match self.store.item(id)? {
                Some(item) if item.state == ItemState::Archived => items.push(item),
                _ => skipped += 1,
            }
        }
        let placement = match r.into_window_id {
            Some(w) => Placement::Window(w),
            None => Placement::Original { focus: true },
        };
        self.start_restore(Some((conn, req)), &items, placement, None, skipped, now)
    }

    pub(crate) fn on_restore_batch(
        &mut self,
        conn: ConnId,
        req: &str,
        r: &ArchiveRestoreBatch,
        now: i64,
    ) -> Res {
        let Some(item) = self.store.item(&r.item_id)? else {
            self.reply_err(conn, req, ErrorCode::NotFound, "no such item");
            return Ok(());
        };
        let batch = match &item.last_sweep_id {
            Some(sweep) => self.store.archived_batch(sweep, item.source_id.as_deref())?,
            None if item.state == ItemState::Archived => vec![item],
            None => Vec::new(),
        };
        self.start_restore(
            Some((conn, req)),
            &batch,
            Placement::Original { focus: true },
            None,
            0,
            now,
        )
    }

    pub(crate) fn on_undo(&mut self, conn: ConnId, req: &str, u: &ArchiveUndo, now: i64) -> Res {
        match self.find_undo(u.sweep_id.as_deref())? {
            Err(why) => {
                self.reply_err(conn, req, ErrorCode::NotFound, why.message());
                Ok(())
            }
            Ok((sweep_id, items)) if items.is_empty() && self.is_hide_sweep(&sweep_id)? => {
                match self.undo_hide_sweep(&sweep_id, now)? {
                    crate::hide::HideUndo::Done { restored, failed } => {
                        let result = serde_json::json!({ "restored": restored, "failed": failed, "skipped": 0 });
                        self.reply_ok(conn, req, &result);
                    }
                    _ => self.reply_err(
                        conn,
                        req,
                        ErrorCode::NotFound,
                        UndoRefused::WindowsUnknown.message(),
                    ),
                }
                Ok(())
            }
            Ok((sweep_id, items)) => self.start_restore(
                Some((conn, req)),
                &items,
                Placement::Original { focus: false },
                Some(sweep_id),
                0,
                now,
            ),
        }
    }

    /// Undo from the app (tray, toast, deep link): `sweep_id` or the latest undoable sweep.
    /// The result comes as [`EngineEvent::SweepUndone`].
    pub(crate) fn undo_from_app(
        &mut self,
        sweep_id: Option<&str>,
        now: i64,
    ) -> Res<Result<(), UndoRefused>> {
        let (sweep_id, items) = match self.find_undo(sweep_id)? {
            Ok(found) => found,
            Err(why) => return Ok(Err(why)),
        };
        // A hide sweep (P2.2): its windows come back through the window actor.
        if items.is_empty() {
            match self.undo_hide_sweep(&sweep_id, now)? {
                crate::hide::HideUndo::Done { .. } => return Ok(Ok(())),
                crate::hide::HideUndo::Unknown => return Ok(Err(UndoRefused::WindowsUnknown)),
                crate::hide::HideUndo::NotHide => {}
            }
        }
        // Nothing left archived (restored one by one): the undo just marks the sweep undone.
        let reachable =
            items.is_empty() || items.iter().any(|i| matches!(self.home_conn(i, now), Ok(Some(_))));
        if !reachable {
            return Ok(Err(UndoRefused::NoBrowser));
        }
        self.start_restore(
            None,
            &items,
            Placement::Original { focus: false },
            Some(sweep_id),
            0,
            now,
        )?;
        Ok(Ok(()))
    }

    /// Restores archived items from the app: back where they were, or all into one new
    /// window. Unknown or already restored ids count as skipped.
    pub(crate) fn restore_from_app(
        &mut self,
        ids: &[String],
        new_window: bool,
        now: i64,
    ) -> Res<Result<(), RestoreRefused>> {
        let mut items = Vec::new();
        let mut skipped = 0u32;
        let mut seen = std::collections::HashSet::new();
        for id in ids.iter().take(500).filter(|id| seen.insert(id.as_str())) {
            match self.store.item(id)? {
                Some(item) if item.state == ItemState::Archived => items.push(item),
                _ => skipped += 1,
            }
        }
        if items.is_empty() {
            return Ok(Err(RestoreRefused::NothingToRestore));
        }
        if !items.iter().any(|i| matches!(self.home_conn(i, now), Ok(Some(_)))) {
            return Ok(Err(RestoreRefused::NoBrowser));
        }
        // Several from one window come back in their original order.
        items.sort_by(|a, b| {
            (a.source_id.as_deref(), a.tab_index).cmp(&(b.source_id.as_deref(), b.tab_index))
        });
        let placement =
            if new_window { Placement::NewWindow } else { Placement::Original { focus: true } };
        self.start_restore(None, &items, placement, None, skipped, now)?;
        Ok(Ok(()))
    }

    /// Restores the batch an item was archived with (same sweep, same window), in order.
    pub(crate) fn restore_batch_from_app(
        &mut self,
        item_id: &str,
        now: i64,
    ) -> Res<Result<(), RestoreRefused>> {
        let Some(item) = self.store.item(item_id)? else {
            return Ok(Err(RestoreRefused::NothingToRestore));
        };
        let batch = match &item.last_sweep_id {
            Some(sweep) => self.store.archived_batch(sweep, item.source_id.as_deref())?,
            None if item.state == ItemState::Archived => vec![item],
            None => Vec::new(),
        };
        let ids: Vec<String> = batch.into_iter().map(|i| i.id).collect();
        if ids.is_empty() {
            return Ok(Err(RestoreRefused::NothingToRestore));
        }
        self.restore_from_app(&ids, false, now)
    }

    /// The sweep to undo and its archived items.
    fn find_undo(&self, sweep_id: Option<&str>) -> Res<Result<(String, Vec<Item>), UndoRefused>> {
        if let Some(id) = sweep_id
            && let Some(w) = self.store.sweep(id)?
            && matches!(w.status, SweepStatus::Pending | SweepStatus::Running)
        {
            return Ok(Err(UndoRefused::StillClosing));
        }
        let candidates = undoable(&self.store)?;
        let sweep = match sweep_id {
            Some(id) => candidates.into_iter().find(|w| w.id == id),
            None => candidates.into_iter().find(|w| w.undone_at.is_none()),
        };
        let Some(sweep) = sweep.filter(|w| w.undone_at.is_none()) else {
            return Ok(Err(UndoRefused::NothingToUndo));
        };
        let items = self.store.archived_in_sweep(&sweep.id)?;
        Ok(Ok((sweep.id, items)))
    }

    /// The latest sweep Undo would revert (the tray's "Undo last sweep").
    pub(crate) fn last_undoable(&self) -> Res<Option<String>> {
        Ok(undoable(&self.store)?.into_iter().find(|w| w.undone_at.is_none()).map(|w| w.id))
    }

    /// The connected adapter an item came from (Undo puts tabs back into their browser).
    fn home_conn(&self, item: &Item, now: i64) -> Res<Option<ConnId>> {
        let Some(source) = item.source_id.as_deref() else { return Ok(None) };
        let Some(source) = self.store.source(source)? else { return Ok(None) };
        Ok(self.conns.iter().find_map(|(id, c)| {
            let a = Self::live_adapter(c, now)?;
            (a.app_id == item.app_id && Some(&a.profile) == source.profile.as_ref()).then_some(*id)
        }))
    }

    fn start_restore(
        &mut self,
        reply: Option<(ConnId, &str)>,
        items: &[Item],
        placement: Placement,
        undo: Option<String>,
        skipped: u32,
        now: i64,
    ) -> Res {
        let mut summary = RestoreSummary { restored: 0, failed: 0, skipped };
        let mut per_conn: BTreeMap<ConnId, Vec<RestoreItem>> = BTreeMap::new();
        let mut window_keys: HashMap<String, Option<String>> = HashMap::new();
        for item in items {
            if self.restoring.contains(&item.id) {
                // Already on its way back (a double Enter, a second popup): one tab only.
                summary.skipped += 1;
                continue;
            }
            let Some(url) = item.url.clone().filter(|u| !u.is_empty()) else {
                summary.failed += 1;
                continue;
            };
            // The browser profile it came from (any session of it); else the requesting
            // popup's own browser. The app never sends a tab to another browser or profile
            // (other cookies, other accounts): such items fail and stay archived.
            let conn = match placement {
                Placement::Window(_) => reply.map(|(c, _)| c),
                Placement::Original { .. } | Placement::NewWindow => {
                    self.home_conn(item, now)?.or_else(|| reply.map(|(c, _)| c))
                }
            };
            let Some(conn) = conn else {
                summary.failed += 1; // the app asked, and its browser isn't connected
                continue;
            };
            let (window_key, index, active) = match placement {
                Placement::Window(w) => {
                    let key = self
                        .conns
                        .get(&conn)
                        .and_then(|c| c.adapter.as_ref())
                        .map(|a| a.window_key(w));
                    (key, None, true)
                }
                Placement::Original { focus } => {
                    let key = match &item.source_id {
                        Some(id) => {
                            if let Some(k) = window_keys.get(id) {
                                k.clone()
                            } else {
                                let k = self.store.source(id)?.and_then(|s| s.external_id);
                                window_keys.insert(id.clone(), k.clone());
                                k
                            }
                        }
                        None => None,
                    };
                    (key, item.tab_index, focus)
                }
                Placement::NewWindow => (None, None, true),
            };
            let group = item.group_title.clone().map(|title| RestoreGroup {
                title,
                color: item.group_color.clone().unwrap_or_else(|| "grey".to_owned()),
            });
            // Focus goes to the first tab actually sent to each browser (an earlier item may
            // have been skipped).
            let batch = per_conn.entry(conn).or_default();
            let active = active && batch.is_empty();
            batch.push(RestoreItem {
                item_id: item.id.clone(),
                url,
                window_key,
                index,
                group,
                active: active.then_some(true),
            });
        }
        let op_id = self.next_op;
        self.next_op += 1;
        self.restore_ops.insert(
            op_id,
            RestoreOp { reply: reply.map(|(c, r)| (c, r.to_owned())), waiting: 0, summary, undo },
        );
        let mut waiting = 0;
        for (conn, items) in per_conn {
            let item_ids = items.iter().map(|i| i.item_id.clone()).collect::<Vec<_>>();
            let count = item_ids.len();
            let new_window = (placement == Placement::NewWindow).then_some(true);
            let payload = TabsRestore { items, new_window };
            let pending = Pending::Restore { op: op_id, item_ids: item_ids.clone() };
            if self
                .command(conn, kinds::TABS_RESTORE, &payload, pending, CommandKind::Restore, now)
                .is_some()
            {
                waiting += 1;
                self.restoring.extend(item_ids);
            } else if let Some(op) = self.restore_ops.get_mut(&op_id) {
                op.summary.failed += u32::try_from(count).unwrap_or(u32::MAX);
            }
        }
        if let Some(op) = self.restore_ops.get_mut(&op_id) {
            op.waiting = waiting;
        }
        if waiting == 0 {
            self.finish_restore(op_id, now)?;
        }
        Ok(())
    }

    /// A `tabs.restore` ack (`None`: no answer; its items count as failed and stay
    /// archived, so they can be restored again).
    pub(crate) fn restore_answered(
        &mut self,
        conn: ConnId,
        op_id: u64,
        item_ids: &[String],
        ack: Option<&Ack>,
        now: i64,
    ) -> Res {
        let result: RestoreResult = ack
            .and_then(|a| a.result.clone())
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        for id in item_ids {
            self.restoring.remove(id);
        }
        let undo = self.restore_ops.get(&op_id).and_then(|op| op.undo.clone());
        let mut adapter = self.conns.get_mut(&conn).and_then(|c| c.adapter.as_mut());
        let mut restored = 0u32;
        let mut already = 0u32;
        let written = self.store.atomically(false, |s| {
            for r in result.restored.iter().filter(|r| item_ids.contains(&r.item_id)) {
                let Some(mut old) = s.item(&r.item_id)? else { continue };
                if old.state != ItemState::Archived {
                    already += 1; // restored meanwhile (another popup, another request)
                    continue;
                }
                old.state = next_state(old.state, LifecycleEvent::Restore, &plain_guards(&old))
                    .unwrap_or(ItemState::Restored);
                old.state_changed_at = now;
                old.restore_count = old.restore_count.saturating_add(1);
                s.update_item(&old)?;
                let event = if undo.is_some() { EventType::Undo } else { EventType::Restored };
                s.append_event(&old.id, event, undo.as_deref(), None, now)?;
                restored += 1;
                // Link the new tab's item (now, or when the tab is reported).
                if let Some(a) = adapter.as_deref_mut() {
                    match a.tabs.get_mut(&r.tab_id).and_then(|t| t.item.as_mut()) {
                        Some(item) => link_restored(s, item, &old.id, now)?,
                        None => {
                            a.restore_links.insert(r.tab_id, old.id.clone());
                        }
                    }
                }
            }
            Ok(())
        });
        if written.is_err() {
            // Rolled back: nothing was recorded (the items stay archived, so they can be
            // restored again), but the request is still answered.
            (restored, already) = (0, 0);
            self.recover_cache(conn, now);
        }
        let answered = u32::try_from(item_ids.len()).unwrap_or(u32::MAX);
        if let Some(op) = self.restore_ops.get_mut(&op_id) {
            op.summary.restored += restored;
            op.summary.skipped += already;
            op.summary.failed += answered.saturating_sub(restored + already);
            op.waiting = op.waiting.saturating_sub(1);
            if op.waiting == 0 {
                self.finish_restore(op_id, now)?;
            }
        }
        written
    }

    fn finish_restore(&mut self, op_id: u64, now: i64) -> Res {
        let Some(op) = self.restore_ops.remove(&op_id) else { return Ok(()) };
        if let Some(sweep_id) = &op.undo {
            if let Some(mut sweep) = self.store.sweep(sweep_id)?
                && sweep.status == SweepStatus::Done
                && self.store.archived_in_sweep(sweep_id)?.is_empty()
                && sweep.undone_at.is_none()
            {
                sweep.status = SweepStatus::Undone;
                sweep.undone_at = Some(now);
                self.store.update_sweep(&sweep)?;
            }
            self.out.push(Output::Event(EngineEvent::SweepUndone {
                sweep_id: sweep_id.clone(),
                restored: op.summary.restored,
                failed: op.summary.failed,
            }));
        }
        match op.reply {
            Some((conn, req)) => {
                if self.conns.contains_key(&conn) {
                    self.reply_ok(conn, &req, &op.summary);
                }
            }
            // The app asked (panel, Library): it hears the result as an event.
            None if op.undo.is_none() => {
                self.out.push(Output::Event(EngineEvent::Restored {
                    restored: op.summary.restored,
                    failed: op.summary.failed,
                    skipped: op.summary.skipped,
                }));
            }
            None => {}
        }
        Ok(())
    }
}
