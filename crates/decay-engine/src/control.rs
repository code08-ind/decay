//! The app's controls (P1.7, API Design §2.3): status for the tray and the UI, mode, hold
//! and Undo of the last sweep.
//!
//! [`EngineStatus`] is published as [`EngineEvent::StatusChanged`] whenever it changes
//! (checked every poll, about once a second, and right after each control call), so the
//! tray's badge and menu follow without asking.

use decay_bridge::protocol::BrowserKind;
use decay_core::{ItemState, Mode};
use serde::Serialize;

use crate::engine::{Engine, EngineEvent, Output, Res, UndoRefused};

/// Setting: the mode "Resume" goes back to.
const MODE_BEFORE_PAUSE: &str = "mode_before_pause";

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

    fn after_setting_change(&mut self, now: i64) -> Res<Vec<Output>> {
        self.reload_policy()?;
        self.publish_status(now);
        Ok(std::mem::take(&mut self.out))
    }
}
