//! Item lifecycle state machine (Software Design §4.1).
//!
//! | From | Event | Guard | To |
//! | --- | --- | --- | --- |
//! | Active | `Tick` | ¬held ∧ ¬protected ∧ age ≥ stale | Stale |
//! | Stale | `Tick` | ¬held ∧ (protected ∨ age < stale) | Active |
//! | Stale | `Tick` | ¬held ∧ ¬protected ∧ window ∧ age ≥ hidden ∧ hideable | Hidden |
//! | any live | `Tick` | otherwise | unchanged |
//! | Stale | `BeginArchive` | tab ∧ [`may_close`] | Archiving |
//! | Hidden | `BeginArchive` | window ∧ [`may_close`] | Archiving |
//! | Archiving | `Commit` | [`may_close`] (re-checked; the close succeeded) | Archived |
//! | Archiving | `Abort` | — | Stale (tab) / Hidden (window) |
//! | any live | `Used` | — | Active (age 0) |
//! | Archived | `Restore` | — | Restored (the reopened item is a new Active record) |
//! | Archived / Restored | `Retention` | retention expired | Purged |
//!
//! `Archiving` is the write-before-close step (§4.4): the record is saved, then the engine
//! re-checks [`may_close`] right before closing; if it no longer holds (typing started, the
//! document became unsaved, a hold began) or the close fails, it sends `Abort`; when the
//! close succeeds, `Commit` with the guards of that re-check. `Commit` is refused when they
//! don't pass [`may_close`], so a skipped re-check is an error, not a silent close.
//!
//! A Hidden window stays Hidden on `Tick` even when it becomes protected or young again
//! (un-minimising windows on its own would be disruptive); only `Used` brings it back. A
//! window that can't be minimised is never Hidden, so it's never closed. `Tick` thresholds are the item's *effective* thresholds (a
//! custom rule's, if one applies), as in the extension's tracker. A hold or pause freezes
//! states, so they don't flip back and forth (extension `lib/tracker/lifecycle.ts`).
//!
//! Invariants (property-tested): nothing starts archiving while protected or held, nothing
//! is archived unless [`may_close`] held right before the close, a window is always Hidden
//! first, tabs are never Hidden, and archived / restored / purged records never go live.

use thiserror::Error;

use crate::types::{ItemKind, ItemState, Thresholds};

/// Something that can happen to an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LifecycleEvent {
    /// Aging-clock tick.
    Tick,
    /// A sweep picked the item: the archive record is being written.
    BeginArchive,
    /// The tab/window was closed after a passing [`may_close`] re-check (whose guards are
    /// passed with this event).
    Commit,
    /// The close didn't happen (re-check failed, in use, close failed).
    Abort,
    /// Focused, typed into, played audio, visible.
    Used,
    /// Reopened from the archive.
    Restore,
    /// Retention check.
    Retention,
}

/// Facts the guards need, evaluated by the policy and platform at event time.
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(clippy::struct_excessive_bools, reason = "each guard is an independent yes/no fact")]
pub struct Guards {
    pub kind: ItemKind,
    pub active_age_s: u64,
    /// The item's effective thresholds (a custom rule's if one applies).
    pub thresholds: Thresholds,
    /// The policy protects the item for a reason other than a hold (pinned, in use, rule…).
    pub protected: bool,
    /// A global hold or pause is on: states freeze, nothing is archived.
    pub held: bool,
    /// Windows: the platform can minimise this window.
    pub hideable: bool,
    /// Windows: may be closed (Pro, allowlisted app, document saved).
    pub closable: bool,
    /// Archived longer than the retention period.
    pub retention_expired: bool,
}

impl Guards {
    fn hours(&self) -> f64 {
        self.active_age_s as f64 / 3600.0
    }
}

/// Whether the item may be closed now: old enough, not protected or held, and for windows
/// closable. Checked when a sweep picks the item and again right before closing it.
pub fn may_close(g: &Guards) -> bool {
    !g.protected
        && !g.held
        && g.hours() >= g.thresholds.archive
        && (g.kind == ItemKind::Tab || g.closable)
}

/// A transition the table doesn't allow (the state stays unchanged).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("{event:?} is not allowed from {from:?}")]
pub struct InvalidTransition {
    pub from: ItemState,
    pub event: LifecycleEvent,
}

/// The next state, or an error when the event isn't allowed from `from`. `Tick` never
/// fails for a live item (it keeps the state when no guard passes); `BeginArchive`,
/// `Restore` and `Retention` fail when their guard does, so the engine can't silently skip.
pub fn next_state(
    from: ItemState,
    event: LifecycleEvent,
    g: &Guards,
) -> Result<ItemState, InvalidTransition> {
    use ItemKind::{Tab, Window};
    use ItemState::{Active, Archived, Archiving, Hidden, Purged, Restored, Stale};
    use LifecycleEvent::{Abort, BeginArchive, Commit, Restore, Retention, Tick, Used};

    let h = g.hours();
    let t = &g.thresholds;
    let free = !g.held && !g.protected;
    match (from, event) {
        (Active, Tick) if free && h >= t.stale => Ok(Stale),
        (Stale, Tick) if !g.held && (g.protected || h < t.stale) => Ok(Active),
        (Stale, Tick) if free && g.kind == Window && h >= t.hidden && g.hideable => Ok(Hidden),
        (Active | Stale | Hidden | Archiving, Tick) => Ok(from),

        (Stale, BeginArchive) if g.kind == Tab && may_close(g) => Ok(Archiving),
        (Hidden, BeginArchive) if g.kind == Window && may_close(g) => Ok(Archiving),

        (Archiving, Commit) if may_close(g) => Ok(Archived),
        (Archiving, Abort) => Ok(if g.kind == Window { Hidden } else { Stale }),

        (Active | Stale | Hidden | Archiving, Used) => Ok(Active),

        (Archived, Restore) => Ok(Restored),
        (Archived | Restored, Retention) if g.retention_expired => Ok(Purged),

        _ => Err(InvalidTransition { from, event }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ItemState::*;
    use LifecycleEvent::*;
    use proptest::prelude::*;

    const H: u64 = 3600;

    fn g(kind: ItemKind, hours: u64) -> Guards {
        Guards {
            kind,
            active_age_s: hours * H,
            thresholds: Thresholds::BALANCED,
            protected: false,
            held: false,
            hideable: true,
            closable: false,
            retention_expired: false,
        }
    }

    #[test]
    fn a_tab_goes_stale_then_archived_then_restored() {
        let tab = g(ItemKind::Tab, 61);
        assert_eq!(next_state(Active, Tick, &g(ItemKind::Tab, 11)), Ok(Active));
        assert_eq!(next_state(Active, Tick, &tab), Ok(Stale));
        assert_eq!(next_state(Stale, Tick, &tab), Ok(Stale)); // tabs are never hidden
        assert_eq!(next_state(Stale, BeginArchive, &tab), Ok(Archiving));
        assert_eq!(next_state(Archiving, Tick, &tab), Ok(Archiving));
        assert_eq!(next_state(Archiving, Commit, &tab), Ok(Archived));
        assert_eq!(next_state(Archived, Restore, &tab), Ok(Restored));
        assert!(next_state(Archived, Commit, &tab).is_err());
        let expired = Guards { retention_expired: true, ..tab };
        assert_eq!(next_state(Restored, Retention, &expired), Ok(Purged));
        assert!(next_state(Archived, Retention, &tab).is_err());
    }

    #[test]
    fn stale_tabs_go_back_to_active_when_protected_or_younger_but_not_during_a_hold() {
        let protected = Guards { protected: true, ..g(ItemKind::Tab, 100) };
        assert_eq!(next_state(Stale, Tick, &protected), Ok(Active));
        // A raised threshold (preset change, new custom rule) makes it young again.
        assert_eq!(next_state(Stale, Tick, &g(ItemKind::Tab, 5)), Ok(Active));
        // A hold freezes: no flip to Active and back.
        let held = Guards { held: true, ..g(ItemKind::Tab, 100) };
        assert_eq!(next_state(Stale, Tick, &held), Ok(Stale));
        assert_eq!(next_state(Active, Tick, &held), Ok(Active));
        assert!(next_state(Stale, BeginArchive, &held).is_err());
    }

    #[test]
    fn commit_needs_a_passing_re_check() {
        let tab = g(ItemKind::Tab, 61);
        for failing in [
            Guards { protected: true, ..tab },
            Guards { held: true, ..tab },
            g(ItemKind::Tab, 0),
            g(ItemKind::Window, 100), // not closable
        ] {
            assert!(next_state(Archiving, Commit, &failing).is_err(), "{failing:?}");
            assert_eq!(next_state(Archiving, Abort, &failing).map(|s| s == Archiving), Ok(false));
        }
    }

    #[test]
    fn may_close_is_the_pre_close_check() {
        assert!(may_close(&g(ItemKind::Tab, 60)));
        assert!(!may_close(&g(ItemKind::Tab, 59)));
        assert!(!may_close(&Guards { protected: true, ..g(ItemKind::Tab, 100) }));
        assert!(!may_close(&Guards { held: true, ..g(ItemKind::Tab, 100) }));
        // A window whose document became unsaved is no longer closable.
        assert!(!may_close(&g(ItemKind::Window, 100)));
        assert!(may_close(&Guards { closable: true, ..g(ItemKind::Window, 100) }));
    }

    #[test]
    fn a_tab_can_not_be_archived_early_while_protected_or_from_active() {
        assert!(next_state(Stale, BeginArchive, &g(ItemKind::Tab, 59)).is_err());
        let protected = Guards { protected: true, ..g(ItemKind::Tab, 100) };
        assert!(next_state(Stale, BeginArchive, &protected).is_err());
        assert_eq!(next_state(Active, Tick, &protected), Ok(Active));
        assert!(next_state(Active, BeginArchive, &g(ItemKind::Tab, 100)).is_err());
        assert_eq!(next_state(Archiving, Abort, &protected), Ok(Stale));
    }

    #[test]
    fn windows_are_hidden_before_they_can_be_closed() {
        let w = Guards { closable: true, ..g(ItemKind::Window, 61) };
        assert_eq!(next_state(Active, Tick, &w), Ok(Stale));
        assert!(next_state(Stale, BeginArchive, &w).is_err());
        assert_eq!(next_state(Stale, Tick, &w), Ok(Hidden));
        assert_eq!(next_state(Hidden, BeginArchive, &w), Ok(Archiving));
        assert_eq!(next_state(Archiving, Abort, &w), Ok(Hidden));
        let not_closable = Guards { closable: false, ..w };
        assert!(next_state(Hidden, BeginArchive, &not_closable).is_err());
        let not_hideable = Guards { hideable: false, ..w };
        assert_eq!(next_state(Stale, Tick, &not_hideable), Ok(Stale));
    }

    #[test]
    fn use_brings_any_live_item_back_to_active() {
        for s in [Active, Stale, Hidden, Archiving] {
            assert_eq!(next_state(s, Used, &g(ItemKind::Tab, 100)), Ok(Active));
        }
        for s in [Archived, Restored, Purged] {
            assert!(next_state(s, Used, &g(ItemKind::Tab, 100)).is_err());
            assert!(next_state(s, Tick, &g(ItemKind::Tab, 100)).is_err());
        }
    }

    // --- property tests -------------------------------------------------------------

    /// Biased towards the archive path.
    fn any_event() -> impl Strategy<Value = LifecycleEvent> {
        prop_oneof![
            5 => Just(Tick),
            3 => Just(BeginArchive),
            3 => Just(Commit),
            1 => Just(Abort),
            1 => Just(Used),
            1 => Just(Restore),
            1 => Just(Retention),
        ]
    }
    /// Valid thresholds (presets and custom), including fractional ones.
    fn any_thresholds() -> impl Strategy<Value = Thresholds> {
        prop_oneof![
            Just(Thresholds::BALANCED),
            Just(Thresholds::AGGRESSIVE),
            Just(Thresholds::GENTLE),
            (1.0f64..20.0, 0.5f64..30.0, 0.5f64..60.0).prop_map(|(stale, dh, da)| Thresholds {
                stale,
                hidden: stale + dh,
                archive: stale + dh + da,
            }),
        ]
    }

    /// One step: an event, the active seconds that passed since the previous step, and the
    /// guard facts at that moment (thresholds can change between steps: a preset change, a
    /// new custom rule). Ages accumulate across steps (see [`run`]), as they do for a real
    /// item, and reset when it's used.
    fn any_step(
        kind: ItemKind,
        thresholds: Vec<Thresholds>,
    ) -> impl Strategy<Value = (LifecycleEvent, Guards)> {
        (
            any_event(),
            0u64..16 * H,
            prop::sample::select(thresholds),
            prop::bool::weighted(0.15),
            prop::bool::weighted(0.1),
            prop::bool::weighted(0.8),
            prop::bool::weighted(0.6),
            any::<bool>(),
        )
            .prop_map(
                move |(e, step_s, thresholds, protected, held, hideable, closable, expired)| {
                    (
                        e,
                        Guards {
                            kind,
                            active_age_s: step_s, // an increment; made absolute in `run`
                            thresholds,
                            protected,
                            held,
                            hideable,
                            closable,
                            retention_expired: expired,
                        },
                    )
                },
            )
    }
    /// A run uses one or two thresholds, so raising them mid-run is exercised.
    fn any_run(kind: ItemKind) -> impl Strategy<Value = Vec<(LifecycleEvent, Guards)>> {
        prop::collection::vec(any_thresholds(), 1..3)
            .prop_flat_map(move |ts| prop::collection::vec(any_step(kind, ts), 1..80))
    }

    type Step = (ItemState, LifecycleEvent, ItemState, Guards);

    /// Feeds raw events to the state machine (no help from the harness: a `Commit` with
    /// failing guards must be refused by `next_state` itself). Returns the transitions.
    fn run(steps: &[(LifecycleEvent, Guards)]) -> Vec<Step> {
        let mut state = Active;
        let mut out = Vec::new();
        let mut age = 0u64;
        for &(event, step) in steps {
            age += step.active_age_s;
            let guards = Guards { active_age_s: age, ..step };
            if let Ok(next) = next_state(state, event, &guards) {
                out.push((state, event, next, guards));
                if event == Used {
                    age = 0;
                }
                state = next;
            }
        }
        out
    }

    /// Checks every invariant on a run; returns whether it reached Archived.
    fn check_invariants(
        kind: ItemKind,
        steps: &[(LifecycleEvent, Guards)],
    ) -> Result<bool, TestCaseError> {
        let mut dead = false;
        let mut archived = false;
        for (from, event, to, g) in run(steps) {
            let free = !g.protected && !g.held;
            if from == Active && to == Stale {
                prop_assert!(free && g.hours() >= g.thresholds.stale, "went stale early");
            }
            if from == Stale && to == Active && event == Tick {
                prop_assert!(
                    !g.held && (g.protected || g.hours() < g.thresholds.stale),
                    "un-staled without a reason"
                );
            }
            if to == Archiving && from != Archiving {
                prop_assert!(may_close(&g), "began archiving without may_close");
                if kind == ItemKind::Window {
                    prop_assert_eq!(from, Hidden, "window skipped Hidden");
                }
            }
            if to == Archived && from != Archived {
                prop_assert_eq!(from, Archiving);
                prop_assert!(may_close(&g), "closed without a passing re-check ({:?})", event);
                archived = true;
            }
            if kind == ItemKind::Tab {
                prop_assert_ne!(to, Hidden, "a tab was hidden");
            }
            if dead {
                prop_assert!(
                    !to.is_live(),
                    "{:?} -{:?}-> {:?}: a dead record went live",
                    from,
                    event,
                    to
                );
            }
            if matches!(to, Archived | Restored | Purged) {
                dead = true;
            }
        }
        Ok(archived)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(3000))]

        #[test]
        fn tab_invariants(steps in any_run(ItemKind::Tab)) {
            check_invariants(ItemKind::Tab, &steps)?;
        }

        #[test]
        fn window_invariants(steps in any_run(ItemKind::Window)) {
            check_invariants(ItemKind::Window, &steps)?;
        }
    }

    /// The generators must actually exercise the archive path, or the invariants above
    /// would pass vacuously.
    #[test]
    fn property_runs_reach_archived_often_enough() {
        use proptest::strategy::ValueTree;
        use proptest::test_runner::TestRunner;
        for kind in [ItemKind::Tab, ItemKind::Window] {
            let mut runner = TestRunner::deterministic();
            let strategy = any_run(kind);
            let mut reached = 0;
            for _ in 0..1000 {
                let steps = strategy.new_tree(&mut runner).unwrap().current();
                if check_invariants(kind, &steps).unwrap() {
                    reached += 1;
                }
            }
            println!("{kind:?}: {reached}/1000 runs reached Archived");
            assert!(reached >= 100, "{kind:?}: only {reached}/1000 runs reached Archived");
        }
    }
}
