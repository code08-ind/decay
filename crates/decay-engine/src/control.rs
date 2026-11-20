//! The app's controls (P1.7, API Design §2.3): status for the tray and the UI, mode, hold
//! and Undo of the last sweep.
//!
//! [`EngineStatus`] is published as [`EngineEvent::StatusChanged`] whenever it changes
//! (checked every poll, about once a second, and right after each control call), so the
//! tray's badge and menu follow without asking.

use decay_bridge::protocol::BrowserKind;
use decay_core::{ItemState, Mode, Preset, Rule, RuleAction, RuleScope, RuleSource};
use serde::{Deserialize, Serialize};

use crate::engine::{Engine, EngineEvent, Output, Res, RestoreRefused, UndoRefused};

/// Setting: the mode "Resume" goes back to.
const MODE_BEFORE_PAUSE: &str = "mode_before_pause";

/// What Decay does with app windows (setting `window_management`; onboarding "Window
/// access"): hide stale ones (needs Accessibility on macOS), or tabs only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowManagement {
    Off,
    Hide,
}

/// A connected browser profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserStatus {
    pub browser: BrowserKind,
    pub profile_id: String,
    /// Tabs tracked in it now.
    pub tabs: u32,
}

/// What the tray and the UI show (`get_status`, `status:changed`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    pub mode: Mode,
    /// On hold until then (only while in the future).
    pub hold_until: Option<i64>,
    /// Stale or hidden tabs of the connected browsers: the menu bar badge.
    pub stale_count: u32,
    pub live_tabs: u32,
    pub browsers: Vec<BrowserStatus>,
    /// A sweep is probing or closing tabs.
    pub sweeping: bool,
    /// The sweep "Undo last sweep" would revert.
    pub undoable_sweep: Option<String>,
    /// Accessibility permission (`None`: unknown on this platform).
    pub accessibility: Option<bool>,
    /// In a call or meeting (F12): scheduled sweeps wait.
    pub in_meeting: bool,
}

impl Engine {
    /// The current status.
    pub fn status(&self, now: i64) -> EngineStatus {
        let mut browsers = Vec::new();
        let (mut stale, mut live) = (0u32, 0u32);
        for conn in self.conns.values() {
            let Some(a) = Self::live_adapter(conn, now) else { continue };
            let mut tabs = 0u32;
            for item in a.tabs.values().filter_map(|t| t.item.as_ref()) {
                tabs += 1;
                if matches!(item.state, ItemState::Stale | ItemState::Hidden) {
                    stale += 1;
                }
            }
            live += tabs;
            browsers.push(BrowserStatus {
                browser: a.browser,
                profile_id: a.profile.clone(),
                tabs,
            });
        }
        browsers.sort_by(|a, b| a.profile_id.cmp(&b.profile_id));
        let undoable_sweep = self.last_undoable().unwrap_or_else(|e| {
            tracing::warn!("reading sweeps failed: {e}");
            None
        });
        EngineStatus {
            mode: self.policy.mode,
            hold_until: self.policy.hold_until.filter(|t| *t > now),
            stale_count: stale,
            live_tabs: live,
            browsers,
            sweeping: self.sweep != crate::sweep::SweepState::Idle,
            undoable_sweep,
            accessibility: self.platform.as_ref().and_then(|(p, _)| p.accessibility),
            in_meeting: self.in_meeting(now),
        }
    }

    /// Emits [`EngineEvent::StatusChanged`] if the status differs from the last one sent.
    pub(crate) fn publish_status(&mut self, now: i64) {
        let status = self.status(now);
        if self.last_status.as_ref() != Some(&status) {
            self.last_status = Some(status.clone());
            self.out.push(Output::Event(EngineEvent::StatusChanged(Box::new(status))));
        }
    }

    /// Switches the mode (Settings, onboarding). Pausing remembers the mode to resume.
    pub fn set_mode(&mut self, mode: Mode, now: i64) -> Res<Vec<Output>> {
        let current = self.store.mode()?;
        if mode == Mode::Paused && current != Mode::Paused {
            self.store.set_setting(MODE_BEFORE_PAUSE, &current, now)?;
        }
        self.store.set_setting("mode", &mode, now)?;
        self.after_setting_change(now)
    }

    /// Ends a pause (tray "Resume"): back to the mode before it, else shadow mode (the
    /// safe one).
    pub fn resume(&mut self, now: i64) -> Res<Vec<Output>> {
        let before = self
            .store
            .setting::<Mode>(MODE_BEFORE_PAUSE)?
            .filter(|m| *m != Mode::Paused)
            .unwrap_or(Mode::Shadow);
        self.set_mode(before, now)
    }

    /// Holds everything until `until` (tray "Hold 1 h"): nothing is archived and states
    /// freeze. `None` releases the hold.
    pub fn hold(&mut self, until: Option<i64>, now: i64) -> Res<Vec<Output>> {
        let until = until.filter(|t| *t > now);
        self.store.set_setting("hold_until", &until, now)?;
        self.after_setting_change(now)
    }

    /// Undoes `sweep_id`, or the latest undoable sweep. The result comes later as
    /// [`EngineEvent::SweepUndone`].
    pub fn undo(
        &mut self,
        sweep_id: Option<&str>,
        now: i64,
    ) -> Res<Result<Vec<Output>, UndoRefused>> {
        if let Err(why) = self.undo_from_app(sweep_id, now)? {
            return Ok(Err(why));
        }
        self.publish_status(now);
        Ok(Ok(std::mem::take(&mut self.out)))
    }

    /// Restores archived items (the panel's ↵, or ⌥↵ with `new_window`). The result comes
    /// later as [`EngineEvent::Restored`].
    pub fn restore(
        &mut self,
        ids: &[String],
        new_window: bool,
        now: i64,
    ) -> Res<Result<Vec<Output>, RestoreRefused>> {
        if let Err(why) = self.restore_from_app(ids, new_window, now)? {
            return Ok(Err(why));
        }
        Ok(Ok(std::mem::take(&mut self.out)))
    }

    /// [`Self::restore`] picked from a search: `search_ms` (first keystroke → restore) is
    /// recorded for Insights once something reopens.
    pub fn restore_searched(
        &mut self,
        ids: &[String],
        new_window: bool,
        search_ms: Option<u64>,
        now: i64,
    ) -> Res<Result<Vec<Output>, RestoreRefused>> {
        self.restore_search_ms = search_ms;
        let result = self.restore(ids, new_window, now);
        self.restore_search_ms = None;
        result
    }

    /// Restores items into a new window of the given connected browser profile: the
    /// user's choice when the profile they came from isn't connected (P2.8).
    pub fn restore_into(
        &mut self,
        ids: &[String],
        app_id: &str,
        profile_id: &str,
        now: i64,
    ) -> Res<Result<Vec<Output>, RestoreRefused>> {
        if let Err(why) = self.restore_into_from_app(ids, app_id, profile_id, now)? {
            return Ok(Err(why));
        }
        Ok(Ok(std::mem::take(&mut self.out)))
    }

    /// Restores the batch `item_id` was archived with (⌘↵).
    pub fn restore_batch(
        &mut self,
        item_id: &str,
        now: i64,
    ) -> Res<Result<Vec<Output>, RestoreRefused>> {
        if let Err(why) = self.restore_batch_from_app(item_id, now)? {
            return Ok(Err(why));
        }
        Ok(Ok(std::mem::take(&mut self.out)))
    }

    /// "Always keep" an item's site (the panel's ⌘P): a protect rule for its domain, made
    /// as a quick action. `None` when the item has no domain; an existing identical rule
    /// is returned as is.
    pub fn protect_site(&mut self, item_id: &str, now: i64) -> Res<Option<Rule>> {
        let Some(domain) = self.store.item(item_id)?.and_then(|i| i.domain) else {
            return Ok(None);
        };
        let existing = self.store.rules()?.into_iter().find(|r| {
            r.scope == RuleScope::Domain && r.action == RuleAction::Protect && r.pattern == domain
        });
        if let Some(mut rule) = existing {
            // A turned-off rule protects nothing: "Always keep" turns it back on.
            if !rule.enabled {
                rule.enabled = true;
                self.store.update_rule(&rule)?;
                self.reload_policy()?;
            }
            return Ok(Some(rule));
        }
        let rule = Rule {
            id: decay_store::new_id(now),
            scope: RuleScope::Domain,
            pattern: domain,
            action: RuleAction::Protect,
            threshold_hours: None,
            enabled: true,
            source: RuleSource::QuickAction,
            created_at: now,
            hit_count: 0,
        };
        self.store.insert_rule(&rule)?;
        self.reload_policy()?;
        Ok(Some(rule))
    }

    /// Finishes onboarding (F21, Software Design §5.2 step 5), in one transaction: the
    /// chosen pace, window handling and launch at login, and a fresh week of shadow mode starting now, whatever happened before
    /// (nothing is closed until the user turns automatic mode on). Running it again (a
    /// second onboarding after an erase) starts the week over.
    pub fn complete_onboarding(
        &mut self,
        preset: Preset,
        window_management: WindowManagement,
        launch_at_login: bool,
        now: i64,
    ) -> Res<Vec<Output>> {
        // Durable: finishing onboarding must survive a power loss right after it.
        self.store.atomically(true, |s| {
            s.set_setting("launch_at_login", &launch_at_login, now)?;
            s.set_setting("preset", &preset, now)?;
            s.set_setting("thresholds", &preset.thresholds(), now)?;
            s.set_setting("window_management", &window_management, now)?;
            s.set_setting("mode", &Mode::Shadow, now)?;
            s.set_setting("shadow_started_at", &now, now)?;
            s.set_setting("hold_until", &Option::<i64>::None, now)?;
            s.set_setting("onboarded_at", &now, now)
        })?;
        self.after_setting_change(now)
    }

    /// Whether onboarding has been completed.
    pub fn onboarded(&self) -> Res<bool> {
        Ok(self.store.setting::<i64>("onboarded_at")?.is_some())
    }

    /// Settings were written to the store directly (`update_settings`): the policy and
    /// the status follow at once.
    pub fn settings_changed(&mut self, now: i64) -> Res<Vec<Output>> {
        self.after_setting_change(now)
    }

    fn after_setting_change(&mut self, now: i64) -> Res<Vec<Output>> {
        self.reload_policy()?;
        if !self.wants_windows() {
            self.forget_windows(now)?;
        }
        self.publish_status(now);
        Ok(std::mem::take(&mut self.out))
    }
}
