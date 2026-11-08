//! Aging clock (Software Design §4.2, PRD F1). Pure accounting: given what happened since
//! the last tick, how many seconds of *active work time* to add to every live item that
//! isn't in use. Matches the extension's `lib/tracker/clock.ts`.

/// What the platform observed for one tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivitySample {
    /// Milliseconds since the previous tick; `None` on the first tick after start-up. The
    /// caller passes `Some(0)` when the wall clock went backwards (the extension clamps a
    /// negative gap to 0 the same way): no credit.
    pub elapsed_ms: Option<u64>,
    /// Not asleep.
    pub awake: bool,
    /// Screen not locked.
    pub unlocked: bool,
    /// Seconds since the last keyboard / mouse input.
    pub idle_secs: u64,
    /// Any audio playing (watching a video counts as being active).
    pub audio_playing: bool,
}

/// Tick timing and the idle cutoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgingClock {
    /// Tick length in milliseconds.
    pub tick_ms: u64,
    /// No input for this long = idle (setting `idle_cutoff_s`, default 300).
    pub idle_cutoff_secs: u64,
}

impl Default for AgingClock {
    fn default() -> Self {
        Self { tick_ms: 30_000, idle_cutoff_secs: 300 }
    }
}

impl AgingClock {
    /// A gap longer than this between ticks means sleep, a closed app or a suspended
    /// process. Such gaps earn no credit (no back-credit): items only ever look newer
    /// than they are, which is the safe direction.
    pub const fn max_gap_ms(&self) -> u64 {
        self.tick_ms.saturating_mul(3)
    }

    /// Seconds of active time to credit for this tick.
    pub fn credit(&self, s: &ActivitySample) -> u64 {
        let Some(elapsed) = s.elapsed_ms else { return 0 };
        if elapsed == 0 || elapsed > self.max_gap_ms() || !s.awake || !s.unlocked {
            return 0;
        }
        if s.idle_secs < self.idle_cutoff_secs || s.audio_playing {
            elapsed.saturating_add(500) / 1000
        } else {
            0
        }
    }

    /// An item's new active age: reset to 0 when in use this tick, else aged by `credit`.
    pub const fn next_age(current_age_s: u64, credit_s: u64, in_use: bool) -> u64 {
        if in_use { 0 } else { current_age_s.saturating_add(credit_s) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACTIVE: ActivitySample = ActivitySample {
        elapsed_ms: Some(30_000),
        awake: true,
        unlocked: true,
        idle_secs: 0,
        audio_playing: false,
    };

    #[test]
    fn credits_active_time_only() {
        let c = AgingClock::default();
        assert_eq!(c.credit(&ACTIVE), 30);
        assert_eq!(c.credit(&ActivitySample { elapsed_ms: Some(31_400), ..ACTIVE }), 31);
        assert_eq!(c.credit(&ActivitySample { idle_secs: 299, ..ACTIVE }), 30);
        assert_eq!(c.credit(&ActivitySample { idle_secs: 300, ..ACTIVE }), 0);
        // Watching a video while not touching the keyboard still counts.
        assert_eq!(c.credit(&ActivitySample { idle_secs: 900, audio_playing: true, ..ACTIVE }), 30);
    }

    #[test]
    fn locked_asleep_first_tick_and_long_gaps_earn_nothing() {
        let c = AgingClock::default();
        assert_eq!(c.credit(&ActivitySample { unlocked: false, ..ACTIVE }), 0);
        assert_eq!(c.credit(&ActivitySample { awake: false, audio_playing: true, ..ACTIVE }), 0);
        assert_eq!(c.credit(&ActivitySample { elapsed_ms: None, ..ACTIVE }), 0);
        assert_eq!(c.credit(&ActivitySample { elapsed_ms: Some(0), ..ACTIVE }), 0);
        assert_eq!(c.credit(&ActivitySample { elapsed_ms: Some(90_000), ..ACTIVE }), 90);
        // Monday reset (story S1): a weekend asleep adds no age.
        assert_eq!(c.credit(&ActivitySample { elapsed_ms: Some(60 * 3_600_000), ..ACTIVE }), 0);
    }

    #[test]
    fn use_resets_age() {
        assert_eq!(AgingClock::next_age(500, 30, false), 530);
        assert_eq!(AgingClock::next_age(500, 30, true), 0);
        assert_eq!(AgingClock::next_age(u64::MAX, 30, false), u64::MAX);
    }

    #[test]
    fn extreme_tick_lengths_do_not_overflow() {
        let c = AgingClock { tick_ms: u64::MAX, idle_cutoff_secs: 300 };
        let s = ActivitySample { elapsed_ms: Some(u64::MAX), ..ACTIVE };
        assert_eq!(c.credit(&s), u64::MAX / 1000);
    }
}
