//! Aging, the lifecycle tick, the sweep scheduler and the write-before-close executor
//! (Software Design §4.2–4.4).
//!
//! A sweep runs in steps, each driven by an ack (or its timeout):
//! 1. **Probe**: web pages that would be archived if nothing were typed in them, and whose
//!    verdict isn't cached, are checked (≤ 40 per browser, oldest first). No answer means
//!    "unknown": such a page waits for 2× the archive threshold.
//! 2. **Plan and write**: the policy decides with the verdicts; the sweep, each item's
//!    `archiving` state and archive reason, and the sweep items are committed in one
//!    durable transaction *before* anything is closed. Shadow mode records
//!    `shadow_would_archive` events instead and stops here.
//! 3. **Close**: `tabs.close` per browser, with the URL each tab is expected to show. The
//!    extension re-checks every tab (active, pinned, audible, typed input, navigated, used
//!    in the last minute) right before closing it.
//! 4. **Settle**: closed → archived; skipped → back to stale (its age is reset if it was
//!    used); no answer → left `archiving` until the next snapshot shows whether the tab is
//!    still open. Unknown state never closes and never loses a record.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use decay_bridge::kinds;
use decay_bridge::protocol::{Ack, CloseResult, Probe, ProbeAck, SkipReason, TabsClose};
use decay_core::policy::{is_excluded, is_web_url};
use decay_core::{
    Decision, EventType, FormInput, Guards, ItemKind, ItemState, LifecycleEvent, MatchTarget, Mode,
    PolicyTarget, Reason, Signals, SweepAction, SweepItemResult, SweepStatus, SweepTrigger,
    Thresholds, evaluate, next_state,
};
use decay_store::{Item, Store, Sweep, SweepItem, new_id};

use crate::engine::{
    Adapter, CommandKind, ConnId, Engine, EngineEvent, Output, Pending, PolicySettings, ProbeEntry,
    Res, timing,
};
use crate::platform::SweepWindow;
use crate::tabs::mark_used;

/// Where the sweep executor is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SweepState {
    Idle,
    /// Waiting for `waiting` probe acks.
    Probing {
        trigger: SweepTrigger,
        waiting: usize,
    },
    /// Waiting for `waiting` close acks.
    Closing {
        sweep_id: String,
        waiting: usize,
    },
}

pub(crate) enum SettleOutcome {
    /// The item was archiving and is now settled (in this sweep, if any).
    Settled(Option<String>),
    NotArchiving,
}

pub(crate) fn target(item: &Item) -> PolicyTarget<'_> {
    PolicyTarget {
        kind: item.kind,
        app_id: &item.app_id,
        url: item.url.as_deref(),
        title: &item.title,
        pinned: item.pinned,
        active_age_s: item.active_age_s,
        grace_until: item.grace_until,
    }
}

/// Guards for the lifecycle: archiving needs only `may_close` (age ≥ the archive
/// threshold the decision used, not protected, not held).
fn archive_guards(item: &Item, reason: &Reason, policy: &PolicySettings) -> Guards {
    let archive = match reason {
        Reason::Unused { threshold, .. } => *threshold,
        Reason::CustomRule { rule_id, .. } => policy
            .rules
            .iter()
            .find(|r| &r.id == rule_id)
            .and_then(|r| r.threshold_hours)
            .map_or(f64::INFINITY, |t| t.archive),
        Reason::Protected { .. } => f64::INFINITY,
    };
    Guards {
        kind: item.kind,
        active_age_s: item.active_age_s,
        thresholds: Thresholds { archive, ..policy.thresholds },
        protected: false,
        held: false,
        hideable: false,
        closable: false,
        retention_expired: false,
    }
}

/// Guards that don't matter for the transition (`Abort`, `Restore`).
pub(crate) fn plain_guards(item: &Item) -> Guards {
    Guards {
        kind: item.kind,
        active_age_s: item.active_age_s,
        thresholds: Thresholds::default(),
        protected: false,
        held: false,
        hideable: false,
        closable: false,
        retention_expired: false,
    }
}

/// The tab is closed: the record becomes archived.
pub(crate) fn commit(s: &Store, item: &mut Item, sweep: Option<&str>, now: i64) -> Res {
    item.state = ItemState::Archived;
    item.archived_at = Some(now);
    item.state_changed_at = now;
    item.external_id = None;
    s.update_item(item)?;
    s.append_event(&item.id, EventType::Archived, sweep, item.archive_reason.as_ref(), now)?;
    Ok(())
}

/// The close didn't happen: back to stale (tabs) / hidden (windows). The archive reason
/// and sweep stay on the record: if Decay's close lands after all (a late close), the
/// item is archived with them (see `tab.removed` in `tabs`).
fn abort(s: &Store, item: &mut Item, sweep: Option<&str>, now: i64) -> Res {
    item.state = next_state(ItemState::Archiving, LifecycleEvent::Abort, &plain_guards(item))
        .unwrap_or(ItemState::Stale);
    item.state_changed_at = now;
    s.update_item(item)?;
    s.append_event(&item.id, EventType::CloseFailed, sweep, None, now)?;
    Ok(())
}

pub(crate) fn record_result(s: &Store, item: &Item, sweep: &str, result: SweepItemResult) -> Res {
    s.put_sweep_item(&SweepItem {
        sweep_id: sweep.to_owned(),
        item_id: item.id.clone(),
        action: SweepAction::Archive,
        prev_state: match item.kind {
            ItemKind::Tab => ItemState::Stale,
            ItemKind::Window => ItemState::Hidden,
        },
        result,
    })
}

/// Settles an item left `archiving` against what the browser shows: `present` → the close
/// never happened (back to stale); gone → archived (its record was written first, so
/// nothing is lost either way).
pub(crate) fn settle(s: &Store, item: &mut Item, present: bool, now: i64) -> Res<SettleOutcome> {
    if item.state != ItemState::Archiving {
        return Ok(SettleOutcome::NotArchiving);
    }
    let sweep = item.last_sweep_id.clone();
    if present {
        abort(s, item, sweep.as_deref(), now)?;
    } else {
        commit(s, item, sweep.as_deref(), now)?;
    }
    if let Some(id) = &sweep
        && s.sweep(id)?.is_some()
    {
        let result = if present { SweepItemResult::Skipped } else { SweepItemResult::Ok };
        record_result(s, item, id, result)?;
    }
    Ok(SettleOutcome::Settled(sweep))
}

/// Marks a running sweep done once none of its items is pending.
pub(crate) fn finish_sweep_if_settled(
    s: &Store,
    sweep_id: &str,
    now: i64,
) -> Res<Option<EngineEvent>> {
    let Some(mut sweep) = s.sweep(sweep_id)? else { return Ok(None) };
    let items = s.sweep_items(sweep_id)?;
    let ok = items.iter().filter(|i| i.result == SweepItemResult::Ok).count();
    if sweep.status == SweepStatus::Done {
        // A late close changed a finished sweep's outcome: keep its counts true.
        let (count, failed) = (
            u32::try_from(ok).unwrap_or(u32::MAX),
            u32::try_from(items.len() - ok).unwrap_or(u32::MAX),
        );
        if (sweep.item_count, sweep.failed_count) != (count, failed) {
            sweep.item_count = count;
            sweep.failed_count = failed;
            s.update_sweep(&sweep)?;
        }
        return Ok(None);
    }
    if !matches!(sweep.status, SweepStatus::Pending | SweepStatus::Running) {
        return Ok(None);
    }
    if items.iter().any(|i| i.result == SweepItemResult::Pending) {
        return Ok(None);
    }
    sweep.status = SweepStatus::Done;
    sweep.finished_at = Some(now);
    sweep.item_count = u32::try_from(ok).unwrap_or(u32::MAX);
    sweep.failed_count = u32::try_from(items.len() - ok).unwrap_or(u32::MAX);
    s.update_sweep(&sweep)?;
    Ok(Some(EngineEvent::SweepCompleted {
        sweep_id: sweep.id,
        shadow: false,
        archived: sweep.item_count,
        failed: sweep.failed_count,
    }))
}

/// Excluded by the current rules (F9): never planned, and forgotten at the next tick
/// (a rule added since the tab was reported).
pub(crate) fn excluded(item: &Item, policy: &PolicySettings) -> bool {
    let target = MatchTarget { app_id: &item.app_id, url: item.url.as_deref(), title: &item.title };
    is_excluded(&target, &policy.rules)
}

/// The reason stored with a `stale` event (the extension's `staleReason`).
fn stale_reason(decision: &Decision) -> Option<Reason> {
    match decision {
        Decision::Archive { reason } => Some(reason.clone()),
        Decision::Stale { active_hours, threshold, rule_id } => {
            let active_hours = active_hours.floor() as u64;
            Some(match rule_id {
                Some(id) => Reason::CustomRule { rule_id: id.clone(), active_hours },
                None => Reason::Unused { active_hours, threshold: threshold.stale },
            })
        }
        _ => None,
    }
}

/// One aging tick for one adapter's tabs.
fn age_adapter(
    s: &Store,
    a: &mut Adapter,
    policy: &PolicySettings,
    pro: bool,
    credit: u64,
    now: i64,
) -> Res {
    let ctx = policy.ctx(now, pro);
    let in_use: HashSet<i64> =
        a.tabs.values().filter(|t| a.in_use(t)).map(|t| t.info.tab_id).collect();
    let forms: HashMap<i64, FormInput> =
        a.tabs.values().map(|t| (t.info.tab_id, a.form_input(t, now))).collect();
    let mut ages: Vec<(String, u64)> = Vec::new();
    for tab in a.tabs.values_mut() {
        let tab_id = tab.info.tab_id;
        let (active, audible) = (tab.info.active, tab.info.audible);
        let Some(item) = tab.item.as_mut() else { continue };
        if !matches!(item.state, ItemState::Active | ItemState::Stale) {
            continue; // archiving: the executor owns it
        }
        if excluded(item, policy) {
            let id = item.id.clone();
            s.delete_item(&id)?;
            tab.item = None;
            continue;
        }
        let mut age_changed = false;
        if credit > 0 {
            if in_use.contains(&tab_id) {
                mark_used(s, item, now)?;
            } else {
                item.active_age_s = item.active_age_s.saturating_add(credit);
                age_changed = true;
            }
        }
        let signals = Signals {
            in_use: active,
            audible,
            form_input: forms.get(&tab_id).copied().unwrap_or(FormInput::Unknown),
            save_state: item.save_state,
        };
        let decision = evaluate(&target(item), &signals, &ctx);
        // A hold or pause is temporary: states freeze instead of flipping back and forth.
        let held = matches!(decision, Decision::Protect { by: decay_core::ProtectedBy::Hold, .. });
        let next = match decision {
            Decision::Archive { .. } | Decision::Stale { .. } | Decision::Hide { .. } => {
                ItemState::Stale
            }
            Decision::Keep { .. } | Decision::Protect { .. } => ItemState::Active,
        };
        if !held && next != item.state {
            item.state = next;
            item.state_changed_at = now;
            s.update_item(item)?;
            if next == ItemState::Stale {
                s.append_event(
                    &item.id,
                    EventType::Stale,
                    None,
                    stale_reason(&decision).as_ref(),
                    now,
                )?;
            }
        } else if age_changed {
            ages.push((item.id.clone(), item.active_age_s));
        }
    }
    let refs: Vec<(&str, u64)> = ages.iter().map(|(id, age)| (id.as_str(), *age)).collect();
    s.set_active_ages(&refs)?;
    Ok(())
}

impl Engine {
    /// The 30-s aging tick (Software Design §4.2).
    pub(crate) fn tick(&mut self, now: i64) -> Res {
        self.reload_policy()?;
        let elapsed = self.last_tick_at.map(|t| u64::try_from(now - t).unwrap_or(0));
        self.last_tick_at = Some(now);
        // Until the platform layer (P1.6) reports keyboard / mouse idle time, deliberate
        // browser activity stands in for it. Without any, nothing ages: items only ever
        // look younger than they are, the safe direction.
        let live: Vec<ConnId> = self
            .conns
            .iter()
            .filter(|(_, c)| Self::live_adapter(c, now).is_some())
            .map(|(id, _)| *id)
            .collect();
        let audio_playing = live.iter().any(|id| {
            self.conns[id].adapter.as_ref().is_some_and(|a| a.tabs.values().any(|t| t.info.audible))
        });
        // Real idle, lock, display and audio from the OS when it can tell; otherwise
        // deliberate browser activity stands in for input (items only look younger).
        let credit = self.clock.credit(&self.activity(elapsed, audio_playing, now));
        self.away_since_tick = false;
        let policy = Arc::clone(&self.policy);
        let pro = self.config.pro;
        for id in live {
            let Some(a) = self.conns.get_mut(&id).and_then(|c| c.adapter.as_mut()) else {
                continue;
            };
            self.store.atomically(false, |s| age_adapter(s, a, &policy, pro, credit, now))?;
        }
        Ok(())
    }

    /// Daily backup and the retention purge (checked hourly).
    pub(crate) fn maintenance(&mut self, now: i64) -> Res {
        if self.store.backup_due(now) {
            self.store.backup_now(now)?;
        }
        let days = i64::try_from(self.policy.retention_days).unwrap_or(i64::MAX / 86_400_001);
        let cutoff = now.saturating_sub(days.saturating_mul(86_400_000));
        let purged = self.store.purge_archived_before(cutoff)?;
        if purged > 0 {
            tracing::info!(purged, "archive retention");
        }
        Ok(())
    }

    /// Starts a sweep if it's time (or `force`: "Sweep now").
    pub(crate) fn maybe_sweep(&mut self, now: i64, trigger: SweepTrigger, force: bool) {
        if self.sweep != SweepState::Idle || self.policy.held(now) {
            return;
        }
        let mut trigger = trigger;
        if !force {
            let too_soon = self.last_sweep_at.is_some_and(|t| now - t < timing::SWEEP_INTERVAL_MS);
            if too_soon {
                return;
            }
            match self.sweep_window(now) {
                SweepWindow::Away | SweepWindow::Busy => return,
                SweepWindow::JustWoke => trigger = SweepTrigger::Wake,
                SweepWindow::Quiet => {}
            }
        }
        let policy = Arc::clone(&self.policy);
        let ctx = policy.ctx(now, self.config.pro);
        let mut probes: Vec<(ConnId, Vec<i64>)> = Vec::new();
        for (id, conn) in &self.conns {
            let Some(a) = Self::live_adapter(conn, now) else { continue };
            let mut need: Vec<(u64, i64)> = a
                .tabs
                .values()
                .filter_map(|tab| {
                    let item = tab.item.as_ref().filter(|i| i.state == ItemState::Stale)?;
                    if !is_web_url(&tab.info.url) || excluded(item, &policy) {
                        return None;
                    }
                    let cached = a
                        .probes
                        .get(&tab.info.tab_id)
                        .is_some_and(|p| p.url == tab.info.url && p.expires_at > now);
                    if cached {
                        return None;
                    }
                    let clean = Signals {
                        in_use: tab.info.active,
                        audible: tab.info.audible,
                        form_input: FormInput::Clean,
                        save_state: item.save_state,
                    };
                    matches!(evaluate(&target(item), &clean, &ctx), Decision::Archive { .. })
                        .then_some((item.active_age_s, tab.info.tab_id))
                })
                .collect();
            if need.is_empty() {
                continue;
            }
            need.sort_by(|x, y| y.cmp(x));
            need.truncate(timing::PROBE_BATCH);
            probes.push((*id, need.into_iter().map(|(_, t)| t).collect()));
        }
        if probes.is_empty() {
            self.execute_sweep(now, trigger);
            return;
        }
        let mut waiting = 0;
        for (conn, tab_ids) in probes {
            let payload = Probe { tab_ids: tab_ids.clone() };
            let pending = Pending::Probe { tab_ids };
            if self
                .command(conn, kinds::PROBE, &payload, pending, CommandKind::Probe, now)
                .is_some()
            {
                waiting += 1;
            }
        }
        if waiting == 0 {
            self.execute_sweep(now, trigger);
        } else {
            self.sweep = SweepState::Probing { trigger, waiting };
        }
    }

    /// A probe's verdicts (none when it wasn't answered: those pages stay unknown).
    pub(crate) fn probe_answered(
        &mut self,
        conn: ConnId,
        tab_ids: &[i64],
        ack: Option<&Ack>,
        now: i64,
    ) {
        let verdicts: Option<ProbeAck> = ack
            .filter(|a| a.ok)
            .and_then(|a| a.result.clone())
            .and_then(|v| serde_json::from_value(v).ok());
        let adapter = self.conns.get_mut(&conn).and_then(|c| c.adapter.as_mut());
        if let (None, Some(a)) = (&verdicts, adapter) {
            // No answer: those pages count as unknown (2×) for a while, like a verdict, so
            // they aren't probed again at every tick.
            for tab_id in tab_ids {
                if let Some(tab) = a.tabs.get(tab_id) {
                    let entry = ProbeEntry {
                        verdict: FormInput::Unknown,
                        url: tab.info.url.clone(),
                        expires_at: now + timing::PROBE_OTHER_TTL_MS,
                    };
                    a.probes.insert(*tab_id, entry);
                }
            }
        }
        if let (Some(verdicts), Some(a)) =
            (verdicts, self.conns.get_mut(&conn).and_then(|c| c.adapter.as_mut()))
        {
            if !verdicts.missing.is_empty() {
                // Tabs that are gone: a fresh snapshot stops them being probed again.
                a.resync_at = Some(now);
            }
            for r in verdicts.results.iter().filter(|r| tab_ids.contains(&r.tab_id)) {
                if let Some(tab) = a.tabs.get(&r.tab_id) {
                    let entry = ProbeEntry::from_result(r, tab.info.url.clone(), now);
                    a.probes.insert(r.tab_id, entry);
                }
            }
        }
        if let SweepState::Probing { trigger, waiting } = &mut self.sweep {
            *waiting = waiting.saturating_sub(1);
            if *waiting == 0 {
                let trigger = *trigger;
                self.sweep = SweepState::Idle;
                self.execute_sweep(now, trigger);
            }
        }
    }

    /// Plans with the current verdicts, writes the plan, and sends the closes.
    fn execute_sweep(&mut self, now: i64, trigger: SweepTrigger) {
        if let Err(e) = self.try_execute_sweep(now, trigger) {
            self.sweep = SweepState::Idle;
            self.problem("sweep", &e);
        }
    }

    #[allow(clippy::too_many_lines, reason = "one linear plan → write → close sequence")]
    fn try_execute_sweep(&mut self, now: i64, trigger: SweepTrigger) -> Res {
        // Settings as they are now: a pause or a new rule written during the probe counts.
        self.reload_policy()?;
        let policy = Arc::clone(&self.policy);
        if policy.held(now) {
            return Ok(());
        }
        let ctx = policy.ctx(now, self.config.pro);
        // (conn, tab id, item, reason)
        let mut plan: Vec<(ConnId, i64, Item, Reason)> = Vec::new();
        // Planned at the normal threshold on an earlier clean check.
        let mut require_checked: HashSet<String> = HashSet::new();
        for (id, conn) in &self.conns {
            let Some(a) = Self::live_adapter(conn, now) else { continue };
            for tab in a.tabs.values() {
                let Some(item) = tab.item.as_ref().filter(|i| i.state == ItemState::Stale) else {
                    continue;
                };
                if excluded(item, &policy) {
                    continue;
                }
                let form_input = a.form_input(tab, now);
                let signals = Signals {
                    in_use: tab.info.active,
                    audible: tab.info.audible,
                    form_input,
                    save_state: item.save_state,
                };
                let Decision::Archive { reason } = evaluate(&target(item), &signals, &ctx) else {
                    continue;
                };
                let guards = archive_guards(item, &reason, &policy);
                if next_state(item.state, LifecycleEvent::BeginArchive, &guards)
                    != Ok(ItemState::Archiving)
                {
                    continue;
                }
                if form_input == FormInput::Clean {
                    require_checked.insert(item.id.clone());
                }
                plan.push((*id, tab.info.tab_id, item.clone(), reason));
            }
        }
        plan.sort_by(|x, y| {
            y.2.active_age_s.cmp(&x.2.active_age_s).then_with(|| x.2.id.cmp(&y.2.id))
        });
        plan.truncate(policy.max_items);
        let shadow = policy.mode == Mode::Shadow;
        if plan.is_empty() {
            if trigger == SweepTrigger::Manual {
                self.out.push(Output::Event(EngineEvent::NothingToSweep { shadow }));
            }
            return Ok(());
        }
        let sweep_id = new_id(now);
        self.last_sweep_at = Some(now);
        let count = u32::try_from(plan.len()).unwrap_or(u32::MAX);
        let mut sweep = Sweep {
            id: sweep_id.clone(),
            started_at: now,
            finished_at: None,
            trigger,
            is_shadow: shadow,
            status: SweepStatus::Running,
            item_count: 0,
            failed_count: 0,
            reason: None,
            undone_at: None,
        };

        if shadow {
            // Shadow mode: record what would have been archived; nothing changes.
            sweep.status = SweepStatus::Done;
            sweep.finished_at = Some(now);
            sweep.item_count = count;
            self.store.atomically(false, |s| {
                s.insert_sweep(&sweep)?;
                for (_, _, item, reason) in &plan {
                    s.append_event(
                        &item.id,
                        EventType::ShadowWouldArchive,
                        Some(&sweep_id),
                        Some(reason),
                        now,
                    )?;
                }
                Ok(())
            })?;
            self.out.push(Output::Event(EngineEvent::SweepCompleted {
                sweep_id,
                shadow: true,
                archived: count,
                failed: 0,
            }));
            return Ok(());
        }

        // Write before close: the archive records are durable before any tab closes.
        let mut written: Vec<(ConnId, i64, Item)> = Vec::with_capacity(plan.len());
        self.store.atomically(true, |s| {
            s.insert_sweep(&sweep)?;
            for (conn, tab_id, item, reason) in &plan {
                let mut item = item.clone();
                item.state = ItemState::Archiving;
                item.state_changed_at = now;
                item.archive_reason = Some(reason.clone());
                item.last_sweep_id = Some(sweep_id.clone());
                s.update_item(&item)?;
                s.put_sweep_item(&SweepItem {
                    sweep_id: sweep_id.clone(),
                    item_id: item.id.clone(),
                    action: SweepAction::Archive,
                    prev_state: ItemState::Stale,
                    result: SweepItemResult::Pending,
                })?;
                written.push((*conn, *tab_id, item));
            }
            Ok(())
        })?;

        let mut per_conn: BTreeMap<ConnId, (TabsClose, Vec<(i64, String)>)> = BTreeMap::new();
        for (conn, tab_id, item) in written {
            let entry = per_conn.entry(conn).or_insert_with(|| {
                let close = TabsClose {
                    sweep_id: sweep_id.clone(),
                    tab_ids: Vec::new(),
                    urls: BTreeMap::new(),
                    require_checked: Vec::new(),
                };
                (close, Vec::new())
            });
            entry.0.tab_ids.push(tab_id);
            if require_checked.contains(&item.id) {
                entry.0.require_checked.push(tab_id);
            }
            entry.0.urls.insert(tab_id.to_string(), item.url.clone().unwrap_or_default());
            entry.1.push((tab_id, item.id.clone()));
            if let Some(tab) = self
                .conns
                .get_mut(&conn)
                .and_then(|c| c.adapter.as_mut())
                .and_then(|a| a.tabs.get_mut(&tab_id))
            {
                tab.item = Some(item);
            }
        }
        let mut waiting = 0;
        for (conn, (close, tabs)) in per_conn {
            let pending = Pending::Close { sweep_id: sweep_id.clone(), tabs };
            if self
                .command(conn, kinds::TABS_CLOSE, &close, pending, CommandKind::Close, now)
                .is_some()
            {
                waiting += 1;
            }
        }
        self.sweep =
            if waiting == 0 { SweepState::Idle } else { SweepState::Closing { sweep_id, waiting } };
        Ok(())
    }

    /// One close command of the current sweep is answered (or late, or lost): the sweep
    /// executor is free once all are. Done before anything that can fail, so a storage
    /// error never leaves the engine stuck in `Closing`.
    pub(crate) fn close_progress(&mut self, sweep_id: &str) {
        if let SweepState::Closing { sweep_id: current, waiting } = &mut self.sweep
            && current == sweep_id
        {
            *waiting = waiting.saturating_sub(1);
            if *waiting == 0 {
                self.sweep = SweepState::Idle;
            }
        }
    }

    /// Settles a close from its ack; `None` (timeout, lost link, an error without a
    /// result) leaves the items archiving until a snapshot shows where they are.
    pub(crate) fn close_answered(
        &mut self,
        conn: ConnId,
        sweep_id: &str,
        tabs: &[(i64, String)],
        ack: Option<&Ack>,
        now: i64,
    ) -> Res {
        let result: Option<CloseResult> =
            ack.and_then(|a| a.result.clone()).and_then(|v| serde_json::from_value(v).ok());
        let mut adapter = self.conns.get_mut(&conn).and_then(|c| c.adapter.as_mut());
        let mut unsettled = false;
        let written = self.store.atomically(false, |s| {
            for (tab_id, item_id) in tabs {
                let Some(mut item) = s.item(item_id)? else { continue }; // closed by the user
                if item.state != ItemState::Archiving {
                    continue;
                }
                let skipped = result
                    .as_ref()
                    .and_then(|r| r.skipped.iter().find(|x| x.tab_id == *tab_id))
                    .map(|x| x.reason);
                let closed = result.as_ref().is_some_and(|r| r.closed.contains(tab_id));
                if closed {
                    commit(s, &mut item, Some(sweep_id), now)?;
                    record_result(s, &item, sweep_id, SweepItemResult::Ok)?;
                    if let Some(a) = adapter.as_deref_mut() {
                        let ours = a
                            .tabs
                            .get(tab_id)
                            .and_then(|t| t.item.as_ref())
                            .is_some_and(|i| i.id == item.id);
                        if ours {
                            a.tabs.remove(tab_id);
                        }
                        a.probes.remove(tab_id);
                        a.used_while_closing.remove(tab_id);
                    }
                } else if let Some(reason) = skipped {
                    abort(s, &mut item, Some(sweep_id), now)?;
                    record_result(s, &item, sweep_id, SweepItemResult::Skipped)?;
                    if let Some(a) = adapter.as_deref_mut() {
                        // `inUse` alone isn't a use: Chrome sets `lastAccessed` on every tab
                        // it restores at start-up, so it only defers the close.
                        let used =
                            a.used_while_closing.remove(tab_id) || reason == SkipReason::Active;
                        if used {
                            mark_used(s, &mut item, now)?;
                        }
                        if reason == SkipReason::FormInput {
                            let url =
                                a.tabs.get(tab_id).map(|t| t.info.url.clone()).unwrap_or_default();
                            a.probes.insert(
                                *tab_id,
                                ProbeEntry {
                                    verdict: FormInput::Dirty,
                                    url,
                                    expires_at: now + timing::PROBE_OTHER_TTL_MS,
                                },
                            );
                        }
                        if let Some(tab) = a.tabs.get_mut(tab_id)
                            && tab.item.as_ref().is_some_and(|i| i.id == item.id)
                        {
                            tab.item = Some(item);
                        }
                    }
                } else {
                    unsettled = true;
                }
            }
            Ok(())
        });
        if written.is_err() {
            self.recover_cache(conn, now);
        }
        written?;
        if unsettled {
            // Ask where the tabs are; a lost link settles on the next connection instead.
            if let Some(a) = self.conns.get_mut(&conn).and_then(|c| c.adapter.as_mut()) {
                a.resync_at = Some(now);
            }
        }
        if let Some(event) = finish_sweep_if_settled(&self.store, sweep_id, now)? {
            self.out.push(Output::Event(event));
        }
        Ok(())
    }
}
