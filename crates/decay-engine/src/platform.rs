//! OS signals in the engine (P1.6): the aging clock's idle / lock / display / audio input,
//! the quiet moment for sweeps (no typing for a minute, no app switch for 30 s), the
//! "morning cleanup" after a wake or unlock, and Accessibility health.
//!
//! A platform that can't tell (`None` values, or no sample for 5 s) leaves the engine on
//! its fallback: browser activity as the idle signal, which only makes items look younger.
//!
//! The aging tick looks at the latest sample, but anything "away" in between (locked,
//! displays asleep, or a gap in the samples) since the previous tick voids that tick's
//! credit, so a 30-s tick never credits time spent locked or asleep.
//!
//! Accessibility is reported as health only: it gates app-window actions (later phases),
//! never tab sweeps, which go through the extension.

use decay_core::ActivitySample;
use decay_platform::PlatformSample;

use crate::engine::{Engine, EngineEvent, Output, timing};

/// How a sweep may start now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SweepWindow {
    /// Locked, or the displays are asleep: no scheduled sweep.
    Away,
    /// In a call or meeting (microphone or camera in use, F12), or just after one.
    Meeting,
    /// Typing, an app switch or a tab switch just now.
    Busy,
    Quiet,
    /// Just woke or unlocked: a "morning cleanup" may run without waiting.
    JustWoke,
}

impl Engine {
    pub(crate) fn on_platform(&mut self, sample: PlatformSample, now: i64) {
        if let Some((prev, at)) = &self.platform {
            // A sleep is a long gap on the continuous clock when both samples have it; the
            // wall clock is only the fallback (it jumps when the time is changed).
            let gap = match (prev.continuous_ms, sample.continuous_ms) {
                (Some(a), Some(b)) => i64::try_from(b.saturating_sub(a)).unwrap_or(i64::MAX),
                _ => now - at,
            };
            if gap > timing::PLATFORM_FRESH_MS || now < *at {
                self.away_since_tick = true;
            }
            let slept = gap > timing::WAKE_GAP_MS;
            let unlocked = prev.locked == Some(true) && sample.locked == Some(false);
            let displays_on =
                prev.display_asleep == Some(true) && sample.display_asleep == Some(false);
            if slept || unlocked || displays_on {
                self.woke_at = Some(now);
            }
            let switched = match (&prev.frontmost, &sample.frontmost) {
                (Some(a), Some(b)) => a.pid != b.pid,
                _ => false,
            };
            if switched {
                self.last_app_switch_at = Some(now);
            }
        }
        if sample.mic_in_use == Some(true) || sample.camera_in_use == Some(true) {
            // A call: no sweeps until a little after it ends.
            self.meeting_until = Some(now + timing::MEETING_COOLDOWN_MS);
        }
        if sample.locked == Some(true) || sample.display_asleep == Some(true) {
            self.away_since_tick = true;
        }
        let before = self.platform.as_ref().and_then(|(p, _)| p.accessibility);
        if let Some(trusted) = sample.accessibility
            && before != Some(trusted)
        {
            self.out.push(Output::Event(EngineEvent::AccessibilityChanged { trusted }));
        }
        self.platform = Some((sample, now));
    }

    /// The latest OS sample, if it's recent.
    pub(crate) fn fresh_platform(&self, now: i64) -> Option<&PlatformSample> {
        self.platform
            .as_ref()
            .filter(|(_, at)| now - at <= timing::PLATFORM_FRESH_MS)
            .map(|(p, _)| p)
    }

    /// The aging clock's input for this tick. `browser_audio`: a tab is audible.
    pub(crate) fn activity(
        &self,
        elapsed_ms: Option<u64>,
        browser_audio: bool,
        now: i64,
    ) -> ActivitySample {
        let p = self.fresh_platform(now);
        let browser_idle = self
            .last_activity_at
            .map_or(u64::MAX, |t| u64::try_from((now - t) / 1000).unwrap_or(0));
        // Lock and display sleep always count when known (the safe direction); only an
        // unknown idle time falls back to browser activity.
        let idle_secs = p.and_then(|p| p.idle_secs).unwrap_or(browser_idle);
        let os_audio = p.is_some_and(|p| p.audio_playing == Some(true))
            && idle_secs < timing::OS_AUDIO_MAX_IDLE_S;
        ActivitySample {
            elapsed_ms,
            awake: !self.away_since_tick && p.is_none_or(|p| p.display_asleep != Some(true)),
            unlocked: p.is_none_or(|p| p.locked != Some(true)),
            idle_secs,
            audio_playing: browser_audio || os_audio,
        }
    }

    /// In a call or meeting (F12): the microphone or a camera was in use within the last
    /// [`timing::MEETING_COOLDOWN_MS`]. Scheduled sweeps of tabs and windows wait;
    /// "Sweep now" (asked by the user) doesn't.
    pub fn in_meeting(&self, now: i64) -> bool {
        self.meeting_until.is_some_and(|t| now < t)
    }

    /// Whether a scheduled sweep may start now (Software Design §4.3).
    pub(crate) fn sweep_window(&self, now: i64) -> SweepWindow {
        let platform = self.fresh_platform(now);
        if platform.is_some_and(|p| p.locked == Some(true) || p.display_asleep == Some(true)) {
            return SweepWindow::Away;
        }
        if self.in_meeting(now) {
            return SweepWindow::Meeting;
        }
        // Woke or unlocked just now, and no sweep since.
        let woke = self.woke_at.is_some_and(|w| {
            now - w <= timing::MORNING_WINDOW_MS && self.last_sweep_at.is_none_or(|s| s < w)
        });
        if woke {
            return SweepWindow::JustWoke;
        }
        let recent = |t: Option<i64>, ms: i64| t.is_some_and(|t| now - t < ms);
        let typing = platform
            .and_then(|p| p.secs_since_key)
            .is_some_and(|s| s.saturating_mul(1000) < timing::NO_TYPING_MS.unsigned_abs());
        let busy = typing
            || recent(self.last_activity_at, timing::QUIET_MS)
            || recent(self.last_app_switch_at, timing::QUIET_MS);
        if busy { SweepWindow::Busy } else { SweepWindow::Quiet }
    }
}
