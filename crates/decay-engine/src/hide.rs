//! Hiding stale app windows (P2.2, PRD F4, Docs/Spikes.md §1): at a quiet moment, at most
//! every 15 minutes, in automatic mode, stale windows past the "hidden" threshold are
//! minimised to the Dock, a few at a time, as one recorded sweep that Undo reverts.
//!
//! - Only windows that are on screen in Accessibility's list, standard, minimisable, not
//!   full screen, not already minimised, not the focused window of the app in front, not
//!   missing, with a known number; the policy must say "hide" (pinned / protected / held
//!   windows never are) and the lifecycle must allow `Stale → Hidden`.
//! - The record is written first (a running sweep, its items pending); the platform's
//!   answer settles each item: hidden, or back to stale. No answer within
//!   [`HIDE_ANSWER_MS`] counts as failed.
//! - A hidden window the user brings back (un-minimised) is used: active, age 0.
//! - Undo un-minimises the sweep's windows that are still hidden and protects them for
//!   48 h, like restored tabs.
//! - Shadow mode hides nothing (and doesn't record would-hide events yet).
//! - Never on a guess: no hide without knowing the app in front, or right after a wake;
//!   each command carries a deadline the actor checks before acting, and the actor checks
//!   again that the window isn't the focused window of the app in front.
//! - Memory follows the store: every change is written first, then applied.

use decay_core::{
    Decision, EventType, FormInput, Guards, ItemState, LifecycleEvent, Mode, Reason, Signals,
    SweepAction, SweepItemResult, SweepStatus, SweepTrigger, evaluate, next_state,
};
use decay_platform::WindowOp;
use decay_store::{Sweep, SweepItem, new_id};

use crate::engine::{Engine, EngineEvent, Output, Res, timing};
use crate::platform::SweepWindow;
use crate::sweep::target;
use crate::windows::{UNKNOWN_PID, WINDOWS_FRESH_MS};

/// At most this many windows are hidden in one sweep.
pub(crate) const MAX_HIDE_PER_SWEEP: usize = 5;
/// The platform's answer for one window is awaited this long.
pub(crate) const HIDE_ANSWER_MS: i64 = 10_000;
/// A hide answered after its deadline is still applied for this long (the window may have
/// been minimised after all).
pub(crate) const LATE_HIDE_MS: i64 = 10 * 60_000;

/// Hide or restore one window (`Output::Window`), done by the runtime's window actor; the
/// answer comes back as `Input::WindowActed`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowCommand {
    pub op: WindowOp,
    pub item_id: String,
    pub pid: i32,
    pub window_id: u32,
    /// Epoch ms after which the actor must not act any more (answering "too late").
    pub deadline: i64,
    /// Reopen this folder instead (an archived window's restore, P4.1; `pid` and
    /// `window_id` mean nothing then).
    pub reopen: Option<String>,
}

/// What Undo did with a hide sweep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HideUndo {
    /// Not a hide sweep.
    NotHide,
    /// Windows can't be seen right now (no fresh list): nothing changed.
    Unknown,
    /// Asked to bring `restored` windows back; `failed` couldn't be (no window number).
    Done { restored: u32, failed: u32 },
}

/// A window command waiting for its answer.
#[derive(Debug, Clone)]
pub(crate) struct PendingWindowOp {
    pub op: WindowOp,
    pub sweep_id: Option<String>,
    pub deadline: i64,
    /// A folder reopening for this restore batch (P4.1).
    pub reopen_batch: Option<String>,
}

impl Engine {
    /// Starts a hide sweep if it's time (called after each tick's sweep check).
    pub(crate) fn maybe_hide_windows(&mut self, now: i64) {
        if let Err(e) = self.try_hide_windows(now) {
            self.problem("hiding windows", &e);
        }
    }

    fn try_hide_windows(&mut self, now: i64) -> Res {
        let policy = std::sync::Arc::clone(&self.policy);
        let fresh = self.app_windows.last_list_at.is_some_and(|t| now - t < WINDOWS_FRESH_MS);
        let due = self.last_hide_at.is_none_or(|t| now - t >= timing::SWEEP_INTERVAL_MS);
        if !self.wants_windows()
            || policy.mode != Mode::Automatic
            || policy.held(now)
            || !fresh
            || !due
            || !self.window_ops.is_empty()
            // A truly quiet moment only (not right after a wake, when the user may be
            // about to type): minimising is visible on screen.
            || self.sweep_window(now) != SweepWindow::Quiet
        {
            return Ok(());
        }
        let trigger = SweepTrigger::Scheduled;
        let mut picked = self.hide_candidates(now);
        if picked.is_empty() {
            return Ok(());
        }
        // The oldest first.
        picked.sort_by(|a, b| b.4.cmp(&a.4).then_with(|| a.0.cmp(&b.0)));
        picked.truncate(MAX_HIDE_PER_SWEEP);
        let sweep_id = new_id(now);
        let sweep = Sweep {
            id: sweep_id.clone(),
            started_at: now,
            finished_at: None,
            trigger,
            is_shadow: false,
            status: SweepStatus::Running,
            item_count: u32::try_from(picked.len()).unwrap_or(u32::MAX),
            failed_count: 0,
            reason: Some(r#"{"kind":"hide"}"#.into()),
            undone_at: None,
        };
        self.store.atomically(true, |s| {
            s.insert_sweep(&sweep)?;
            for (id, ..) in &picked {
                s.put_sweep_item(&SweepItem {
                    sweep_id: sweep_id.clone(),
                    item_id: id.clone(),
                    action: SweepAction::Hide,
                    prev_state: ItemState::Stale,
                    result: SweepItemResult::Pending,
                })?;
            }
            Ok(())
        })?;
        for (id, pid, window_id, reason, _) in picked {
            // Kept with the op (memory only until the answer is written).
            self.hide_reasons.insert(id.clone(), reason);
            self.window_ops.insert(
                id.clone(),
                PendingWindowOp {
                    op: WindowOp::Hide,
                    sweep_id: Some(sweep_id.clone()),
                    deadline: now + HIDE_ANSWER_MS,
                    reopen_batch: None,
                },
            );
            self.out.push(Output::Window(WindowCommand {
                op: WindowOp::Hide,
                item_id: id,
                pid,
                window_id,
                deadline: now + HIDE_ANSWER_MS,
                reopen: None,
            }));
        }
        self.last_hide_at = Some(now);
        Ok(())
    }

    /// Windows a hide sweep may take now: (item id, pid, window number, reason, age).
    fn hide_candidates(&self, now: i64) -> Vec<(String, i32, u32, Reason, u64)> {
        let policy = std::sync::Arc::clone(&self.policy);
        // Without knowing the app in front, any window might be the one in use: none.
        let Some(front) =
            self.fresh_platform(now).and_then(|p| p.frontmost.as_ref()).map(|f| f.pid)
        else {
            return Vec::new();
        };
        let ctx = policy.ctx(now, self.config.pro);
        let mut picked: Vec<(String, i32, u32, Reason, u64)> = Vec::new();
        for (id, t) in &self.app_windows.tracked {
            let Some(window_id) = t.window_id else { continue };
            let eligible = t.item.state == ItemState::Stale
                && t.visible
                && t.can_minimize
                && !t.full_screen
                && !t.minimized
                && t.pid != UNKNOWN_PID
                && !(t.focused && front == t.pid)
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
            let decision = evaluate(&target(&t.item), &signals, &ctx);
            let (threshold, reason) = match &decision {
                Decision::Hide { threshold, active_hours, rule_id, .. } => {
                    let hours = active_hours.floor() as u64;
                    let reason = match rule_id {
                        Some(id) => Reason::CustomRule { rule_id: id.clone(), active_hours: hours },
                        None => Reason::Unused { active_hours: hours, threshold: threshold.hidden },
                    };
                    (*threshold, reason)
                }
                // Archive (closing allowlisted windows, Pro) is a later step: hide them.
                Decision::Archive { reason } => (ctx.thresholds, reason.clone()),
                _ => continue,
            };
            let guards = Guards {
                kind: t.item.kind,
                active_age_s: t.item.active_age_s,
                thresholds: threshold,
                protected: false,
                held: false,
                hideable: true,
                closable: false,
                retention_expired: false,
            };
            if next_state(ItemState::Stale, LifecycleEvent::Tick, &guards) != Ok(ItemState::Hidden)
            {
                continue;
            }
            picked.push((id.clone(), t.pid, window_id, reason, t.item.active_age_s));
        }
        picked
    }

    /// The platform's answer to a window command.
    pub(crate) fn on_window_acted(&mut self, item_id: &str, result: &Result<(), String>, now: i64) {
        if let Err(e) = self.window_acted(item_id, result, now) {
            self.problem("window answer", &e);
        }
    }

    fn window_acted(&mut self, item_id: &str, result: &Result<(), String>, now: i64) -> Res {
        if let Err(why) = result {
            tracing::info!(item = %item_id, "window not changed: {why}");
        }
        let (op, late) = match self.window_ops.get(item_id) {
            Some(op) => (op.clone(), false),
            None => match self.late_hides.get(item_id) {
                // A hide answered after its deadline: only a success changes anything.
                Some((sweep, _)) if result.is_ok() => (
                    PendingWindowOp {
                        op: WindowOp::Hide,
                        sweep_id: Some(sweep.clone()),
                        deadline: now,
                        reopen_batch: None,
                    },
                    true,
                ),
                _ => return Ok(()),
            },
        };
        if let Some(batch) = &op.reopen_batch {
            return self.reopen_answered(item_id, batch, result, now);
        }
        if op.op == WindowOp::Close {
            let sweep = op.sweep_id.clone().unwrap_or_default();
            return self.window_close_answered(item_id, &sweep, result, now);
        }
        if op.op != WindowOp::Hide {
            if result.is_err() {
                // Undo couldn't bring it back: it's still in the Dock, so it's still hidden
                // (another Undo, or the user, can bring it back).
                let back = self.app_windows.tracked.get(item_id).map(|t| {
                    let mut item = t.item.clone();
                    item.state = ItemState::Hidden;
                    item.state_changed_at = now;
                    item
                });
                if let Some(item) = &back {
                    self.store.atomically(true, |s| s.update_item(item))?;
                }
                if let (Some(item), Some(t)) = (back, self.app_windows.tracked.get_mut(item_id)) {
                    t.item = item;
                    t.minimized = true;
                }
            }
            self.window_ops.remove(item_id);
            return Ok(());
        }
        let Some(sweep_id) = op.sweep_id else {
            self.window_ops.remove(item_id);
            return Ok(());
        };
        let hidden = result.is_ok();
        // The record as it will be, written first; memory follows only once it's saved.
        let record = self.app_windows.tracked.get(item_id).map(|t| {
            let mut item = t.item.clone();
            if hidden && item.state == ItemState::Stale {
                item.state = ItemState::Hidden;
                item.state_changed_at = now;
                item.last_sweep_id = Some(sweep_id.clone());
                if let Some(reason) = self.hide_reasons.get(item_id) {
                    item.archive_reason = Some(reason.clone());
                }
            }
            item
        });
        let ok = hidden && record.as_ref().is_some_and(|i| i.state == ItemState::Hidden);
        let event = self.store.atomically(true, |s| {
            if let Some(item) = &record {
                s.update_item(item)?;
                if ok {
                    s.append_event(
                        &item.id,
                        EventType::Hidden,
                        Some(&sweep_id),
                        item.archive_reason.as_ref(),
                        now,
                    )?;
                }
            }
            s.put_sweep_item(&SweepItem {
                sweep_id: sweep_id.clone(),
                item_id: item_id.to_owned(),
                action: SweepAction::Hide,
                prev_state: ItemState::Stale,
                result: if ok { SweepItemResult::Ok } else { SweepItemResult::Failed },
            })?;
            finish_hide_sweep(s, &sweep_id, now)
        })?;
        self.window_ops.remove(item_id);
        self.hide_reasons.remove(item_id);
        if late || ok {
            self.late_hides.remove(item_id);
        }
        if let (Some(item), Some(t)) = (record, self.app_windows.tracked.get_mut(item_id)) {
            if ok {
                t.minimized = true;
            }
            t.item = item;
        }
        if let Some(e) = event {
            self.out.push(Output::Event(e));
        }
        Ok(())
    }

    /// Commands nobody answered in time count as failed.
    pub(crate) fn expire_window_ops(&mut self, now: i64) {
        let late: Vec<String> = self
            .window_ops
            .iter()
            .filter(|(_, op)| op.deadline <= now)
            .map(|(id, _)| id.clone())
            .collect();
        for id in late {
            // A close nobody confirmed proves nothing: the next window list settles it.
            if self.window_ops.get(&id).is_some_and(|op| op.op == WindowOp::Close) {
                self.window_ops.remove(&id);
                continue;
            }
            // Recorded as failed, but a late success is still applied for a while.
            if let Some(PendingWindowOp { op: WindowOp::Hide, sweep_id: Some(sweep), .. }) =
                self.window_ops.get(&id)
            {
                self.late_hides.insert(id.clone(), (sweep.clone(), now + LATE_HIDE_MS));
            }
            self.on_window_acted(&id, &Err("no answer".into()), now);
        }
        self.late_hides.retain(|_, (_, until)| *until > now);
    }

    /// Whether a sweep hid windows (P2.2) rather than archiving tabs.
    pub(crate) fn is_hide_sweep(&self, sweep_id: &str) -> Res<bool> {
        Ok(self.store.sweep_items(sweep_id)?.iter().any(|i| i.action == SweepAction::Hide))
    }

    /// Undo of a hide sweep: its windows still hidden (from the store) come back
    /// (un-minimised, raised) and are protected for 48 h. Refused while windows can't be
    /// seen (no fresh list): nothing is marked restored that can't be.
    pub(crate) fn undo_hide_sweep(&mut self, sweep_id: &str, now: i64) -> Res<HideUndo> {
        let Some(mut sweep) = self.store.sweep(sweep_id)? else { return Ok(HideUndo::NotHide) };
        if !self.is_hide_sweep(sweep_id)? {
            return Ok(HideUndo::NotHide);
        }
        let fresh = self.app_windows.loaded
            && self.app_windows.last_list_at.is_some_and(|t| now - t < WINDOWS_FRESH_MS);
        if !fresh {
            return Ok(HideUndo::Unknown);
        }
        let hidden: Vec<decay_store::Item> = self
            .store
            .live_items()?
            .into_iter()
            .filter(|i| {
                i.state == ItemState::Hidden && i.last_sweep_id.as_deref() == Some(sweep_id)
            })
            .collect();
        let mut back = Vec::new();
        let mut commands = Vec::new();
        let mut failed = 0u32;
        for mut item in hidden {
            let target = self
                .app_windows
                .tracked
                .get(&item.id)
                .filter(|t| t.pid != UNKNOWN_PID)
                .and_then(|t| t.window_id.map(|n| (t.pid, n)));
            let Some((pid, window_id)) = target else {
                failed += 1; // its window can't be addressed: it stays where it is (the Dock)
                continue;
            };
            item.state = ItemState::Active;
            item.state_changed_at = now;
            item.active_age_s = 0;
            item.last_used_at = now;
            item.grace_until = Some(now + timing::RESTORE_GRACE_MS);
            commands.push(WindowCommand {
                op: WindowOp::Restore,
                item_id: item.id.clone(),
                pid,
                window_id,
                deadline: now + HIDE_ANSWER_MS,
                reopen: None,
            });
            back.push(item);
        }
        sweep.status = SweepStatus::Undone;
        sweep.undone_at = Some(now);
        self.store.atomically(true, |s| {
            for item in &back {
                s.update_item(item)?;
                s.append_event(&item.id, EventType::Undo, Some(sweep_id), None, now)?;
            }
            s.update_sweep(&sweep)?;
            Ok(())
        })?;
        for item in back {
            if let Some(t) = self.app_windows.tracked.get_mut(&item.id) {
                t.minimized = false;
                t.item = item;
            }
        }
        let restored = u32::try_from(commands.len()).unwrap_or(u32::MAX);
        for c in commands {
            self.window_ops.insert(
                c.item_id.clone(),
                PendingWindowOp {
                    op: WindowOp::Restore,
                    sweep_id: None,
                    deadline: c.deadline,
                    reopen_batch: None,
                },
            );
            self.out.push(Output::Window(c));
        }
        self.out.push(Output::Event(EngineEvent::SweepUndone {
            sweep_id: sweep_id.to_owned(),
            restored,
            failed,
        }));
        Ok(HideUndo::Done { restored, failed })
    }

    /// After a restart: hide sweeps left running (Decay quit or crashed mid-hide) are
    /// settled against the first window list: a window that's minimised now was hidden,
    /// the others weren't.
    pub(crate) fn settle_hide_sweeps(&mut self, now: i64) -> Res {
        for sweep in self.store.unfinished_sweeps()? {
            if !self.is_hide_sweep(&sweep.id)? {
                continue;
            }
            let pending: Vec<String> = self
                .store
                .sweep_items(&sweep.id)?
                .into_iter()
                .filter(|i| i.result == SweepItemResult::Pending)
                .map(|i| i.item_id)
                .collect();
            let mut updates = Vec::new();
            let mut results = Vec::new();
            for id in pending {
                let seen = self.app_windows.tracked.get(&id);
                let minimised = seen.is_some_and(|t| t.visible && t.minimized);
                if minimised && let Some(t) = seen {
                    let mut item = t.item.clone();
                    if item.state == ItemState::Stale {
                        item.state = ItemState::Hidden;
                        item.state_changed_at = now;
                        item.last_sweep_id = Some(sweep.id.clone());
                        updates.push(item);
                    }
                }
                results.push((id, minimised));
            }
            let event = self.store.atomically(true, |s| {
                for item in &updates {
                    s.update_item(item)?;
                    s.append_event(&item.id, EventType::Hidden, Some(&sweep.id), None, now)?;
                }
                for (id, ok) in &results {
                    s.put_sweep_item(&SweepItem {
                        sweep_id: sweep.id.clone(),
                        item_id: id.clone(),
                        action: SweepAction::Hide,
                        prev_state: ItemState::Stale,
                        result: if *ok { SweepItemResult::Ok } else { SweepItemResult::Failed },
                    })?;
                }
                finish_hide_sweep(s, &sweep.id, now)
            })?;
            for item in updates {
                if let Some(t) = self.app_windows.tracked.get_mut(&item.id) {
                    t.item = item;
                }
            }
            if let Some(e) = event {
                self.out.push(Output::Event(e));
            }
        }
        Ok(())
    }
}

/// Whether a sweep record is a hide sweep (by its reason, without reading its items).
pub(crate) fn is_hide_record(sweep: &Sweep) -> bool {
    sweep.reason.as_deref().is_some_and(|r| r.contains(r#""kind":"hide""#))
}

/// Marks a hide sweep done once no item is pending; the event for the UI.
fn finish_hide_sweep(s: &decay_store::Store, sweep_id: &str, now: i64) -> Res<Option<EngineEvent>> {
    let Some(mut sweep) = s.sweep(sweep_id)? else { return Ok(None) };
    let items = s.sweep_items(sweep_id)?;
    let ok = items.iter().filter(|i| i.result == SweepItemResult::Ok).count();
    let (hidden, failed) = (
        u32::try_from(ok).unwrap_or(u32::MAX),
        u32::try_from(items.len() - ok).unwrap_or(u32::MAX),
    );
    if sweep.status == SweepStatus::Done
        && (sweep.item_count, sweep.failed_count) != (hidden, failed)
    {
        // A late success changed a finished sweep: keep its counts true.
        sweep.item_count = hidden;
        sweep.failed_count = failed;
        s.update_sweep(&sweep)?;
        return Ok(None);
    }
    if sweep.status != SweepStatus::Running
        || items.iter().any(|i| i.result == SweepItemResult::Pending)
    {
        return Ok(None);
    }
    sweep.status = SweepStatus::Done;
    sweep.finished_at = Some(now);
    sweep.item_count = hidden;
    sweep.failed_count = failed;
    s.update_sweep(&sweep)?;
    Ok(Some(EngineEvent::WindowsHidden { sweep_id: sweep_id.to_owned(), hidden, failed }))
}
