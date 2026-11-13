//! Decay's engine (Software Design §4.2–4.5, P1.5): the part of the app that owns the
//! browser tabs while an extension is connected.
//!
//! - [`Engine`] is a deterministic state machine. Every input (a connection, a message from
//!   the extension, a timer) comes with the time, and the engine answers with [`Output`]s:
//!   messages to send, connections to close, events for the UI. It owns the [`Store`]
//!   (synchronous SQLite), so tests drive it directly with a fake extension.
//! - [`runtime`] runs it: the IPC server's accept loop, one reader thread per connection,
//!   and one engine thread that also polls the timers every second.
//!
//! What the engine does:
//! - **Tracking:** the extension's `snapshot` and tab events become live items, matched to
//!   the stored ones by tab id and URL (ages survive a browser restart). Items whose tab is
//!   gone are deleted after a minute and a fresh snapshot ("not restored yet" ≠ "gone").
//! - **Aging:** every 30 s the [`decay_core::AgingClock`] credits active time to every tab
//!   not in use. Until the platform layer (P1.6) reports idle time, browser activity stands
//!   in for it: no deliberate tab switch or window focus for the idle cutoff = idle. Items of
//!   a browser that isn't connected are frozen.
//! - **Sweeps:** at most every 15 min, at a quiet moment (30 s without a deliberate switch).
//!   Web pages that would be archived are probed first (typed input, uploads); then the plan
//!   is written before anything closes (write-before-close), `tabs.close` is sent, and the
//!   ack settles each tab. Shadow mode only records what it would have archived.
//! - **Crash recovery:** an item left `archiving` (crash, timeout, lost link) is settled
//!   against the next snapshot: still open → back to stale, gone → archived.
//! - **Undo, search and restore** for the extension's popup (`archive.*` requests).
//! - **Controls for the app** (P1.7): [`EngineStatus`] (published on every change), mode,
//!   hold and Undo of the last sweep.
//!
//! [`Store`]: decay_store::Store

mod control;
mod domain;
mod engine;
mod platform;
mod restore;
pub mod runtime;
mod sweep;
mod tabs;

pub use control::{BrowserStatus, EngineStatus};
pub use domain::domain_of;
pub use engine::{
    CORE_VERSION, ConnId, Engine, EngineConfig, EngineEvent, Input, MIN_EXT_VERSION, Output,
    RestoreRefused, SweepRefused, UndoRefused, timing,
};
