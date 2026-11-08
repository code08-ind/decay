//! Decay's pure domain core (Software Design §4.1): types, rule matching, the policy, the
//! item lifecycle and the aging clock. No I/O, no clocks, no threads: every input is a
//! parameter, so the same logic runs in the desktop engine, tests and tools.

pub mod aging;
mod js_case;
mod js_regex;
pub mod lifecycle;
pub mod matching;
pub mod policy;
pub mod types;

pub use aging::{ActivitySample, AgingClock};
pub use lifecycle::{Guards, InvalidTransition, LifecycleEvent, may_close, next_state};
pub use matching::{MatchTarget, rule_matches};
pub use policy::{Decision, FormInput, PolicyContext, PolicyTarget, Signals, evaluate};
pub use types::*;
