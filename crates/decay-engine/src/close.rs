//! Closing hidden folder windows (P4.1, PRD F11, Software Design §4.4): at a quiet
//! moment, at most every 15 minutes, in automatic mode, with Pro, hidden windows the
//! policy says to archive (an allowlisted app, a saved document, past the archive
//! threshold) are closed, a few at a time, as one recorded sweep that Undo reverts by
//! reopening them.
//!
//! - Only windows Decay hid that are still minimised, with a known number and process,
//!   not missing, showing a document (the folder: what reopens them); the lifecycle must
//!   allow `Hidden → Archiving`. The platform only ever closes a file manager's folder
//!   window (Finder, Explorer) and re-checks it right before closing.
//! - Write before close: each record is `archiving` (and the sweep `running`) before the
//!   command is sent.
//! - A confirmed close archives the record at once. A refusal, an error or no answer
//!   proves nothing (the window may close a moment later): the record stays `archiving`
//!   until a window list settles it, back to hidden if the window is still there,
//!   archived once it's gone. The same settles closes a quit or crash interrupted.
//! - Restoring an archived window (the Library, Undo) reopens its folder; the record is
//!   `restored` only once the folder opened.

use decay_core::{
    Decision, EventType, FormInput, ItemKind, ItemState, LifecycleEvent, Mode, Signals,
    SweepItemResult, SweepStatus, SweepTrigger, evaluate, next_state,
};
use decay_platform::WindowOp;
use decay_store::{Item, Sweep, new_id};

use crate::engine::{Engine, EngineEvent, Output, Res, timing};
use crate::hide::{PendingWindowOp, WindowCommand};
use crate::platform::SweepWindow;
use crate::sweep::{abort, archive_guards, commit, plain_guards, record_result, target};
use crate::windows::{UNKNOWN_PID, WINDOWS_FRESH_MS};

/// At most this many windows are closed in one sweep.
pub(crate) const MAX_CLOSE_PER_SWEEP: usize = 5;
/// The platform's answer to a close is awaited this long (after that, lists decide).
pub(crate) const CLOSE_ANSWER_MS: i64 = 10_000;
/// A folder must reopen within this long, or the restore counts as failed.
pub(crate) const REOPEN_ANSWER_MS: i64 = 10_000;

/// A restore of archived windows waiting for their folders to open.
#[derive(Debug, Clone, Default)]
pub(crate) struct ReopenBatch {
    /// Undo of this sweep (else a restore the app asked for).
    pub undo: Option<String>,
    pub waiting: u32,
    pub restored: u32,
    pub failed: u32,
    pub skipped: u32,
}

/// Whether a sweep record is a close sweep (by its reason, without reading its items).
pub(crate) fn is_close_record(sweep: &Sweep) -> bool {
    sweep.reason.as_deref().is_some_and(|r| r.contains(r#""kind":"close""#))
}

impl Engine {
    /// Starts a close sweep if it's time (called after each tick's hide check).
    pub(crate) fn maybe_close_windows(&mut self, now: i64) {
        if let Err(e) = self.try_close_windows(now) {
            self.problem("closing windows", &e);
        }
    }

    fn try_close_windows(&mut self, now: i64) -> Res {
        let policy = std::sync::Arc::clone(&self.policy);
        let fresh = self.app_windows.last_list_at.is_some_and(|t| now - t < WINDOWS_FRESH_MS);
        let due = self.last_close_at.is_none_or(|t| now - t >= timing::SWEEP_INTERVAL_MS);
        if !self.config.pro
            || !self.wants_windows()
            || policy.mode != Mode::Automatic
            || policy.held(now)
            || !fresh
            || !due
            || !self.window_ops.is_empty()
            || self.sweep_window(now) != SweepWindow::Quiet
        {
            return Ok(());
        }
        let mut picked = self.close_candidates(now);
        if picked.is_empty() {
            return Ok(());
        }
        // The oldest first.
        picked.sort_by(|a, b| b.0.active_age_s.cmp(&a.0.active_age_s).then(a.0.id.cmp(&b.0.id)));
        picked.truncate(MAX_CLOSE_PER_SWEEP);
        let sweep_id = new_id(now);
        let sweep = Sweep {
            id: sweep_id.clone(),
            started_at: now,
            finished_at: None,
            trigger: SweepTrigger::Scheduled,
            is_shadow: false,
            status: SweepStatus::Running,
            item_count: u32::try_from(picked.len()).unwrap_or(u32::MAX),
            failed_count: 0,
            reason: Some(r#"{"kind":"close"}"#.into()),
            undone_at: None,
        };
        let items: Vec<Item> = picked
            .iter()
            .map(|(item, ..)| {
                let mut item = item.clone();
                item.state = ItemState::Archiving;
                item.state_changed_at = now;
                item.last_sweep_id = Some(sweep_id.clone());
                item
            })
            .collect();
        self.store.atomically(true, |s| {
            s.insert_sweep(&sweep)?;
            for item in &items {
                s.update_item(item)?;
                record_result(s, item, &sweep_id, SweepItemResult::Pending)?;
            }
            Ok(())
        })?;
        for (item, (_, pid, window_id)) in items.into_iter().zip(picked) {
            let id = item.id.clone();
            if let Some(t) = self.app_windows.tracked.get_mut(&id) {
                t.item = item;
            }
            let deadline = now + CLOSE_ANSWER_MS;
            self.window_ops.insert(
                id.clone(),
                PendingWindowOp {
                    op: WindowOp::Close,
                    sweep_id: Some(sweep_id.clone()),
                    deadline,
                    reopen_batch: None,
                },
            );
            self.out.push(Output::Window(WindowCommand {
                op: WindowOp::Close,
                item_id: id,
                pid,
                window_id,
                deadline,
                reopen: None,
            }));
        }
        self.last_close_at = Some(now);
        Ok(())
    }

    /// Hidden windows a close sweep may take now: (record with its archive reason, pid,
    /// window number).
    fn close_candidates(&self, now: i64) -> Vec<(Item, i32, u32)> {
        let policy = std::sync::Arc::clone(&self.policy);
        let ctx = policy.ctx(now, self.config.pro);
        let mut picked = Vec::new();
        for (id, t) in &self.app_windows.tracked {
            let Some(window_id) = t.window_id else { continue };
            let eligible = t.item.state == ItemState::Hidden
                && t.item.kind == ItemKind::Window
                && t.visible
                && t.minimized
                && t.pid != UNKNOWN_PID
                && t.item.path.is_some()
                && !self.app_windows.missing.contains_key(id);
            if !eligible {
                continue;
            }
            let signals = Signals {
                in_use: false,
                audible: false,
                form_input: FormInput::NotApplicable,
                save_state: t.item.save_state,
            };
            let Decision::Archive { reason } = evaluate(&target(&t.item), &signals, &ctx) else {
                continue;
            };
            let guards =
                decay_core::Guards { closable: true, ..archive_guards(&t.item, &reason, &policy) };
            if next_state(ItemState::Hidden, LifecycleEvent::BeginArchive, &guards)
                != Ok(ItemState::Archiving)
            {
                continue;
            }
            let mut item = t.item.clone();
            item.archive_reason = Some(reason);
            picked.push((item, t.pid, window_id));
        }
        picked
    }

    /// The actor's answer to a close. Only a confirmed close archives; anything else
    /// leaves the record `archiving` for the next window list to settle.
    pub(crate) fn window_close_answered(
        &mut self,
        item_id: &str,
        sweep_id: &str,
        result: &Result<(), String>,
        now: i64,
    ) -> Res {
        self.window_ops.remove(item_id);
        if let Err(why) = result {
            tracing::info!(item = %item_id, "window not closed (yet): {why}");
            return Ok(());
        }
        let Some(mut item) = self.app_windows.tracked.get(item_id).map(|t| t.item.clone()) else {
            return Ok(());
        };
        if item.state != ItemState::Archiving {
            return Ok(());
        }
        let event = self.store.atomically(true, |s| {
            commit(s, &mut item, Some(sweep_id), now)?;
            record_result(s, &item, sweep_id, SweepItemResult::Ok)?;
            finish_close_sweep(s, sweep_id, now)
        })?;
        self.app_windows.tracked.remove(item_id);
        self.app_windows.missing.remove(item_id);
        if let Some(e) = event {
            self.out.push(Output::Event(e));
        }
        Ok(())
    }

    /// Settles closes nobody confirmed against the latest list (after every list): a
    /// window still listed wasn't closed (back to hidden); one that's gone was (archived).
    /// A window that's neither (its app didn't answer) waits for a later list.
    pub(crate) fn settle_closes(&mut self, now: i64) -> Res {
        let mut back = Vec::new();
        let mut closed = Vec::new();
        for (id, t) in &self.app_windows.tracked {
            if t.item.state != ItemState::Archiving || self.window_ops.contains_key(id) {
                continue;
            }
            if t.visible {
                back.push(t.item.clone());
            } else if self.app_windows.missing.contains_key(id) {
                closed.push(t.item.clone());
            }
        }
        if back.is_empty() && closed.is_empty() {
            return Ok(());
        }
        let events = self.store.atomically(true, |s| {
            let mut sweeps = std::collections::BTreeSet::new();
            for item in &mut back {
                let sweep = item.last_sweep_id.clone();
                abort(s, item, sweep.as_deref(), now)?;
                if let Some(sweep) = sweep {
                    record_result(s, item, &sweep, SweepItemResult::Failed)?;
                    sweeps.insert(sweep);
                }
            }
            for item in &mut closed {
                let sweep = item.last_sweep_id.clone();
                commit(s, item, sweep.as_deref(), now)?;
                if let Some(sweep) = sweep {
                    record_result(s, item, &sweep, SweepItemResult::Ok)?;
                    sweeps.insert(sweep);
                }
            }
            let mut events = Vec::new();
            for sweep in &sweeps {
                events.extend(finish_close_sweep(s, sweep, now)?);
            }
            Ok(events)
        })?;
        for item in back {
            if let Some(t) = self.app_windows.tracked.get_mut(&item.id) {
                t.item = item;
            }
        }
        for item in closed {
            self.app_windows.tracked.remove(&item.id);
            self.app_windows.missing.remove(&item.id);
        }
        self.out.extend(events.into_iter().map(Output::Event));
        Ok(())
    }

    /// Reopens archived windows (their folders). `undo`: the sweep they're undoing. The
    /// result comes as [`EngineEvent::SweepUndone`] (Undo) or [`EngineEvent::Restored`].
    pub(crate) fn start_reopen(
        &mut self,
        items: &[Item],
        undo: Option<String>,
        skipped: u32,
        now: i64,
    ) {
        self.next_op += 1;
        let batch_id = format!("reopen-{}", self.next_op);
        let mut batch = ReopenBatch { undo, skipped, ..ReopenBatch::default() };
        for item in items {
            let in_flight = self.window_ops.contains_key(&item.id);
            let path = item.path.clone().filter(|_| item.state == ItemState::Archived);
            let Some(path) = path.filter(|_| !in_flight) else {
                if in_flight {
                    batch.skipped += 1;
                } else {
                    batch.failed += 1;
                }
                continue;
            };
            let deadline = now + REOPEN_ANSWER_MS;
            self.window_ops.insert(
                item.id.clone(),
                PendingWindowOp {
                    op: WindowOp::Restore,
                    sweep_id: None,
                    deadline,
                    reopen_batch: Some(batch_id.clone()),
                },
            );
            self.out.push(Output::Window(WindowCommand {
                op: WindowOp::Restore,
                item_id: item.id.clone(),
                pid: UNKNOWN_PID,
                window_id: 0,
                deadline,
                reopen: Some(path),
            }));
            batch.waiting += 1;
        }
        let done = batch.waiting == 0;
        self.reopen_batches.insert(batch_id.clone(), batch);
        if done && let Err(e) = self.finish_reopen(&batch_id, now) {
            self.problem("reopening windows", &e);
        }
    }

    /// A folder opened (or didn't): the record is restored only when it did.
    pub(crate) fn reopen_answered(
        &mut self,
        item_id: &str,
        batch_id: &str,
        result: &Result<(), String>,
        now: i64,
    ) -> Res {
        self.window_ops.remove(item_id);
        let undo = self.reopen_batches.get(batch_id).and_then(|b| b.undo.clone());
        let mut restored = false;
        if let Err(why) = result {
            tracing::info!(item = %item_id, "folder not reopened: {why}");
        } else if let Some(mut item) = self.store.item(item_id)?
            && item.state == ItemState::Archived
        {
            item.state = next_state(item.state, LifecycleEvent::Restore, &plain_guards(&item))
                .unwrap_or(ItemState::Restored);
            item.state_changed_at = now;
            item.restore_count = item.restore_count.saturating_add(1);
            let event = if undo.is_some() { EventType::Undo } else { EventType::Restored };
            self.store.atomically(true, |s| {
                s.update_item(&item)?;
                s.append_event(&item.id, event, undo.as_deref(), None, now)
            })?;
            restored = true;
        }
        let Some(batch) = self.reopen_batches.get_mut(batch_id) else { return Ok(()) };
        if restored {
            batch.restored += 1;
        } else {
            batch.failed += 1;
        }
        batch.waiting = batch.waiting.saturating_sub(1);
        if batch.waiting == 0 {
            self.finish_reopen(batch_id, now)?;
        }
        Ok(())
    }

    fn finish_reopen(&mut self, batch_id: &str, now: i64) -> Res {
        let Some(batch) = self.reopen_batches.remove(batch_id) else { return Ok(()) };
        match batch.undo {
            Some(sweep_id) => {
                if let Some(mut sweep) = self.store.sweep(&sweep_id)?
                    && sweep.status == SweepStatus::Done
                    && sweep.undone_at.is_none()
                    && self.store.archived_in_sweep(&sweep_id)?.is_empty()
                {
                    sweep.status = SweepStatus::Undone;
                    sweep.undone_at = Some(now);
                    self.store.update_sweep(&sweep)?;
                }
                self.out.push(Output::Event(EngineEvent::SweepUndone {
                    sweep_id,
                    restored: batch.restored,
                    failed: batch.failed,
                }));
            }
            None => self.out.push(Output::Event(EngineEvent::Restored {
                restored: batch.restored,
                failed: batch.failed,
                skipped: batch.skipped,
            })),
        }
        Ok(())
    }
}

/// Marks a close sweep done once no item is pending; the event for the UI.
fn finish_close_sweep(
    s: &decay_store::Store,
    sweep_id: &str,
    now: i64,
) -> Res<Option<EngineEvent>> {
    let Some(mut sweep) = s.sweep(sweep_id)? else { return Ok(None) };
    let items = s.sweep_items(sweep_id)?;
    if sweep.status != SweepStatus::Running
        || items.iter().any(|i| i.result == SweepItemResult::Pending)
    {
        return Ok(None);
    }
    let ok = items.iter().filter(|i| i.result == SweepItemResult::Ok).count();
    let closed = u32::try_from(ok).unwrap_or(u32::MAX);
    let failed = u32::try_from(items.len() - ok).unwrap_or(u32::MAX);
    sweep.status = SweepStatus::Done;
    sweep.finished_at = Some(now);
    sweep.item_count = closed;
    sweep.failed_count = failed;
    s.update_sweep(&sweep)?;
    Ok(Some(EngineEvent::WindowsClosed { sweep_id: sweep_id.to_owned(), closed, failed }))
}
