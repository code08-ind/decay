//! The engine state machine: connections, the handshake, message dispatch and the commands
//! waiting for an ack. Tab tracking is in `tabs`, aging and sweeps in `sweep`, search /
//! restore / undo in `restore`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use decay_bridge::chunk::Reassembler;
use decay_bridge::ipc::{ConnInfo, HostFrame};
use decay_bridge::protocol::{
    Ack, BrowserKind, Config, ErrorPayload, GroupInfo, Hello, ProbeResult, Welcome, WelcomeConfig,
    WindowInfo, WindowState,
};
use decay_bridge::{Envelope, ErrorCode, kinds};
use decay_core::policy::is_web_url;
use decay_core::{
    AgingClock, FormInput, Mode, PolicyContext, Rule, RuleAction, RuleScope, Thresholds,
};
use decay_store::{App, Item, Platform, RestoreKind, Store, StoreError};

use crate::control::EngineStatus;
use serde::Serialize;
use serde_json::Value;

use crate::restore::RestoreOp;
use crate::sweep::SweepState;
use crate::tabs::Tab;

/// One connection from `decay-host` (one per extension instance).
pub type ConnId = u64;

/// This engine's version, sent in `welcome`.
pub const CORE_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Oldest extension the app works with (adapter mode arrived in 0.1.0's P1.4 build).
pub const MIN_EXT_VERSION: &str = "0.1.0";

/// Timings (Software Design §4.2–4.5; same values as the extension's Phase 0 engine).
pub mod timing {
    /// Aging-clock tick.
    pub const TICK_MS: i64 = 30_000;
    /// At least this long between sweeps.
    pub const SWEEP_INTERVAL_MS: i64 = 15 * 60_000;
    /// A sweep waits for this long without a deliberate tab switch or window focus.
    pub const QUIET_MS: i64 = 30_000;
    /// An item whose tab is missing is deleted only after this long and a fresh snapshot
    /// (session restore and "reopen closed window" bring tabs back with new ids).
    pub const MISSING_GRACE_MS: i64 = 60_000;
    /// An OS sample older than this is ignored (the engine falls back to browser activity).
    pub const PLATFORM_FRESH_MS: i64 = 5_000;
    /// After the microphone and camera go quiet, sweeps wait this long more (a call that
    /// drops for a moment, the minutes right after a meeting).
    pub const MEETING_COOLDOWN_MS: i64 = 2 * 60_000;
    /// A gap this long between OS samples means the machine slept (or the app was stopped).
    pub const WAKE_GAP_MS: i64 = 90_000;
    /// After a wake or unlock, a sweep may run this long without waiting for a quiet moment
    /// ("morning cleanup").
    pub const MORNING_WINDOW_MS: i64 = 2 * 60_000;
    /// Audio from the OS (any app holding the output device) stops counting as activity
    /// after this long without input: a call app or a stream idling silently mustn't age
    /// tabs overnight. Audible browser tabs aren't capped (the browser knows they're audible).
    pub const OS_AUDIO_MAX_IDLE_S: u64 = 2 * 3_600;
    /// A quiet moment needs no typing for this long (and no app switch for `QUIET_MS`).
    pub const NO_TYPING_MS: i64 = 60_000;
    /// The first snapshot is asked for again after this long if it hasn't come.
    pub const FIRST_SNAPSHOT_RETRY_MS: i64 = 60_000;
    /// A fresh snapshot this often corrects state the events don't carry (minimised windows).
    pub const RESYNC_EVERY_MS: i64 = 10 * 60_000;
    /// A connection with no message for this long (the extension's heartbeat is 20 s) is
    /// treated as down: its tabs stop aging and aren't swept.
    pub const STALE_LINK_MS: i64 = 70_000;
    pub const HEARTBEAT_SEC: u32 = 20;
    /// Probe verdicts are reused this long: clean pages 30 min, dirty / unknown 10 min.
    pub const PROBE_CLEAN_TTL_MS: i64 = 30 * 60_000;
    pub const PROBE_OTHER_TTL_MS: i64 = 10 * 60_000;
    /// Tabs probed per connection before a sweep.
    pub const PROBE_BATCH: usize = 40;
    /// A restored tab isn't archived again for 48 h.
    pub const RESTORE_GRACE_MS: i64 = 48 * 3_600_000;
    /// Undo is offered for the last 20 sweeps that archived something.
    pub const UNDO_HISTORY: usize = 20;
    /// A close that times out stays "in flight" this much longer: the extension may still
    /// be working through it, and its late ack is still applied. Until then no snapshot
    /// settles its tabs (a tab it's about to close would otherwise be reverted, then closed).
    pub const LATE_CLOSE_MS: i64 = 5 * 60_000;
    /// Daily backups and retention, checked hourly.
    pub const MAINTENANCE_EVERY_MS: i64 = 3_600_000;

    /// How long to wait for an ack: the extension probes each tab (≤ 3 s) before closing it.
    #[allow(clippy::cast_possible_wrap, reason = "capped at 100,000 first")]
    pub const fn ack_timeout_ms(kind: super::CommandKind, items: usize) -> i64 {
        let n = if items > 100_000 { 100_000 } else { items as i64 };
        match kind {
            super::CommandKind::Probe => 5_000 + 3_500 * n,
            super::CommandKind::Close => 10_000 + 4_000 * n,
            super::CommandKind::Restore => 10_000 + 1_000 * n,
            super::CommandKind::Other => 10_000,
        }
    }
}

/// Something that happened outside the engine.
#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    /// `decay-host` connected.
    Connected(ConnId),
    /// A message from the extension, with the host's connection metadata.
    Frame(ConnId, Box<HostFrame>),
    /// The connection closed (or became unreadable).
    Disconnected(ConnId),
    /// What the OS says (idle, lock, displays, audio, frontmost app, Accessibility), about
    /// once a second.
    Platform(Box<decay_platform::PlatformSample>),
    /// Every app's standard windows (P2.1), every few seconds when they can be listed.
    Windows(Box<decay_platform::WindowList>),
    /// The window actor's answer to an `Output::Window` (P2.2).
    WindowActed { item_id: String, result: Result<(), String> },
}

/// What the engine wants done.
#[derive(Debug, Clone, PartialEq)]
pub enum Output {
    Send(ConnId, Envelope),
    /// Stop writing to this connection (an outdated extension, or replaced by a newer
    /// connection). The engine has already told the extension (an `error`), which ends its
    /// port and so the host; the engine ignores anything more from it.
    Close(ConnId),
    Event(EngineEvent),
    /// Hide or restore an app window (P2.2), done by the runtime's window actor.
    Window(crate::hide::WindowCommand),
}

/// Things the UI shows (P1.7 turns `SweepCompleted` into the Undo toast).
#[derive(Debug, Clone, PartialEq)]
pub enum EngineEvent {
    AdapterConnected {
        browser: BrowserKind,
        profile_id: String,
    },
    AdapterDisconnected {
        browser: BrowserKind,
        profile_id: String,
    },
    /// "Sweep now" found nothing to archive (no sweep was recorded).
    NothingToSweep {
        shadow: bool,
    },
    /// A sweep finished. In shadow mode nothing was closed: `archived` counts what would
    /// have been.
    SweepCompleted {
        sweep_id: String,
        shadow: bool,
        archived: u32,
        failed: u32,
    },
    SweepUndone {
        sweep_id: String,
        restored: u32,
        failed: u32,
    },
    /// A hide sweep finished (P2.2): windows minimised to the Dock (Undo brings them back).
    WindowsHidden {
        sweep_id: String,
        hidden: u32,
        failed: u32,
    },
    /// The Accessibility permission was granted or revoked (F33 health; first sample too).
    AccessibilityChanged {
        trusted: bool,
    },
    /// A restore the app asked for finished (`items:restored`).
    Restored {
        restored: u32,
        failed: u32,
        skipped: u32,
    },
    /// The status changed (tray badge and menu, `status:changed`).
    StatusChanged(Box<EngineStatus>),
    /// Settings changed inside the engine (an archive import): every setting, defaults
    /// filled in (`settings:changed`).
    SettingsChanged(serde_json::Map<String, serde_json::Value>),
    /// Something failed (logged); the engine carries on.
    Problem(String),
}

/// Why a restore from the app didn't start.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RestoreRefused {
    #[error("nothing to restore: it isn't in the archive any more")]
    NothingToRestore,
    /// The browser profile it came from isn't connected (it's never sent elsewhere).
    #[error("the browser it came from isn't open (or the Decay extension isn't connected)")]
    NoBrowser,
}

/// Why Undo didn't start.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UndoRefused {
    #[error("this sweep is still closing tabs; try again in a moment")]
    StillClosing,
    #[error("there's no sweep to undo")]
    NothingToUndo,
    #[error("the browser these tabs came from isn't connected")]
    NoBrowser,
    #[error("Decay can't see your windows right now (Accessibility, or window handling is off)")]
    WindowsUnknown,
}

impl UndoRefused {
    /// The message for the extension's popup.
    pub(crate) fn message(&self) -> &'static str {
        match self {
            Self::StillClosing => "this sweep is still closing tabs; try again in a moment",
            Self::NothingToUndo => "this sweep can't be undone",
            Self::NoBrowser => "the browser these tabs came from isn't connected",
            Self::WindowsUnknown => "Decay can't see your windows right now",
        }
    }
}

/// Fixed engine settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineConfig {
    pub platform: Platform,
    /// Pro features (closing allowlisted windows; batch restore is free until Phase 4).
    pub pro: bool,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            platform: if cfg!(windows) { Platform::Windows } else { Platform::MacOs },
            pro: false,
        }
    }
}

/// Why "Sweep now" didn't run.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SweepRefused {
    #[error("Decay is paused")]
    Paused,
    #[error("Decay is on hold until {0}")]
    OnHold(i64),
    #[error("a sweep is already running")]
    Busy,
    #[error("no browser is connected")]
    NoBrowser,
}

/// Settings the policy reads, reloaded every tick (and after an import).
#[derive(Debug, Clone)]
pub(crate) struct PolicySettings {
    pub mode: Mode,
    pub hold_until: Option<i64>,
    pub thresholds: Thresholds,
    pub rules: Vec<Rule>,
    pub max_items: usize,
    pub idle_cutoff_s: u64,
    pub retention_days: u64,
    pub window_management: crate::control::WindowManagement,
}

impl PolicySettings {
    pub fn load(store: &Store) -> Result<Self, StoreError> {
        Ok(Self {
            mode: store.mode()?,
            hold_until: store.hold_until()?,
            thresholds: store.thresholds()?,
            rules: store.rules()?.into_iter().filter(|r| r.enabled).collect(),
            max_items: store.setting::<usize>("max_items_per_sweep")?.unwrap_or(50).clamp(1, 500),
            idle_cutoff_s: store.setting::<u64>("idle_cutoff_s")?.unwrap_or(300).max(30),
            retention_days: store.setting::<u64>("retention_days")?.unwrap_or(90).max(1),
            // An unreadable value means "off": tracking windows needs a clear yes.
            window_management: store
                .setting::<crate::control::WindowManagement>("window_management")
                .ok()
                .flatten()
                .unwrap_or(crate::control::WindowManagement::Off),
        })
    }

    pub fn ctx(&self, now: i64, pro: bool) -> PolicyContext<'_> {
        PolicyContext {
            now,
            mode: self.mode,
            hold_until: self.hold_until,
            thresholds: self.thresholds,
            rules: &self.rules,
            pro,
        }
    }

    /// Paused, or on hold: nothing is archived and states freeze.
    pub fn held(&self, now: i64) -> bool {
        self.mode == Mode::Paused || self.hold_until.is_some_and(|t| t > now)
    }

    /// Domain exclusions the extension applies itself (it never reports those tabs).
    /// Other exclusion scopes (URL glob, title) are applied here.
    pub fn excluded_domains(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .rules
            .iter()
            .filter(|r| r.action == RuleAction::Exclude && r.scope == RuleScope::Domain)
            .map(|r| r.pattern.clone())
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// An enabled title exclusion exists: a tab without a title yet waits for it before
    /// it's stored (excluded items are never stored, F9).
    pub fn has_title_exclusions(&self) -> bool {
        self.rules
            .iter()
            .any(|r| r.action == RuleAction::Exclude && r.scope == RuleScope::TitleRegex)
    }
}

/// A probe verdict for a tab, valid while the tab shows `url`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProbeEntry {
    pub verdict: FormInput,
    pub url: String,
    pub expires_at: i64,
}

impl ProbeEntry {
    pub fn from_result(r: &ProbeResult, url: String, now: i64) -> Self {
        let verdict = if r.dirty_form || r.before_unload || r.uploading {
            FormInput::Dirty
        } else if r.unknown {
            FormInput::Unknown
        } else {
            FormInput::Clean
        };
        let ttl = if verdict == FormInput::Clean {
            timing::PROBE_CLEAN_TTL_MS
        } else {
            timing::PROBE_OTHER_TTL_MS
        };
        Self { verdict, url, expires_at: now + ttl }
    }
}

/// One extension instance (browser + profile), known once its `hello` is accepted.
#[derive(Debug)]
pub(crate) struct Adapter {
    /// Catalog app id of the browser on this platform.
    pub app_id: String,
    pub profile: String,
    pub browser: BrowserKind,
    /// Browser session: window ids only mean something within one.
    pub session: String,
    /// The first snapshot arrived: tabs are known.
    pub ready: bool,
    pub tabs: HashMap<i64, Tab>,
    pub windows: HashMap<i64, WindowInfo>,
    pub groups: HashMap<i64, GroupInfo>,
    pub probes: HashMap<i64, ProbeEntry>,
    /// Item id → when its tab was first missed.
    pub missing: HashMap<String, i64>,
    /// Ask for a snapshot at this time (to confirm missing tabs, or settle a sweep).
    pub resync_at: Option<i64>,
    pub last_snapshot_at: i64,
    /// Sources written in this connection.
    pub sources: HashSet<String>,
    /// Tabs used while being closed: skipped, and their age reset afterwards.
    pub used_while_closing: HashSet<i64>,
    /// Tab id → archived item it reopens (the restore ack came before the tab was reported).
    pub restore_links: HashMap<i64, String>,
}

impl Adapter {
    /// `<profile>:<tabId>`: tab ids are only unique per browser profile.
    pub fn ext_id(&self, tab_id: i64) -> String {
        format!("{}:{tab_id}", self.profile)
    }

    /// The external id of an item whose tab is missing: frees its tab id for a new tab
    /// (browsers reuse ids after a restart) while keeping the item this adapter's.
    pub fn parked_ext_id(&self, item_id: &str) -> String {
        format!("{}:~{item_id}", self.profile)
    }

    /// The live item belongs to this browser profile.
    pub fn owns(&self, item: &Item) -> bool {
        item.app_id == self.app_id
            && item.external_id.as_deref().is_some_and(|e| {
                e.strip_prefix(self.profile.as_str()).is_some_and(|rest| rest.starts_with(':'))
            })
    }

    pub fn source_id(&self, window_id: i64) -> String {
        format!("{}|{}|{}", self.app_id, self.profile, self.window_key(window_id))
    }

    /// `tabs.restore` window key.
    pub fn window_key(&self, window_id: i64) -> String {
        format!("{}:{window_id}", self.session)
    }

    pub fn minimized(&self, window_id: i64) -> bool {
        self.windows.get(&window_id).is_some_and(|w| w.state == WindowState::Minimized)
    }

    /// In use right now: the active tab of a window that isn't minimised, or playing audio
    /// (the extension's `inUseTabIds`). In-use tabs don't age.
    pub fn in_use(&self, tab: &Tab) -> bool {
        (tab.info.active && !self.minimized(tab.info.window_id)) || tab.info.audible
    }

    /// The form-probe signal for a tab now: non-web pages aren't a concern; a web page
    /// without a fresh verdict for its current URL is unknown (it waits 2×).
    pub fn form_input(&self, tab: &Tab, now: i64) -> FormInput {
        if !is_web_url(&tab.info.url) {
            return FormInput::NotApplicable;
        }
        match self.probes.get(&tab.info.tab_id) {
            Some(p) if p.url == tab.info.url && p.expires_at > now => p.verdict,
            _ => FormInput::Unknown,
        }
    }
}

#[derive(Debug)]
pub(crate) struct Conn {
    pub info: Option<ConnInfo>,
    pub reassembler: Reassembler,
    pub adapter: Option<Adapter>,
    pub last_message_at: i64,
}

/// Kinds of command, for ack timeouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    Probe,
    Close,
    Restore,
    Other,
}

/// A command sent to the extension, waiting for its ack.
#[derive(Debug)]
pub(crate) enum Pending {
    /// `config` / `resync`: nothing to do with the ack.
    Plain,
    Probe {
        tab_ids: Vec<i64>,
    },
    /// `tabs.close`: (tab id, item id) of each tab.
    Close {
        sweep_id: String,
        tabs: Vec<(i64, String)>,
    },
    Restore {
        op: u64,
        item_ids: Vec<String>,
    },
}

#[derive(Debug)]
pub(crate) struct PendingCmd {
    pub conn: ConnId,
    pub what: Pending,
    pub deadline: i64,
    /// A close past its first deadline: the extension may still be closing (and will ack),
    /// so its tabs stay in flight until `deadline` (see [`timing::LATE_CLOSE_MS`]).
    pub late: bool,
}

/// The engine (see the crate docs).
#[derive(Debug)]
pub struct Engine {
    pub(crate) store: Store,
    pub(crate) config: EngineConfig,
    pub(crate) policy: Arc<PolicySettings>,
    pub(crate) conns: BTreeMap<ConnId, Conn>,
    pub(crate) pending: HashMap<String, PendingCmd>,
    pub(crate) restore_ops: HashMap<u64, RestoreOp>,
    /// Archived items with a `tabs.restore` waiting for its ack (never sent twice).
    pub(crate) restoring: HashSet<String>,
    /// Items an exclude rule matched while a sweep was closing them: deleted as soon as
    /// the close settles (whichever way), so an excluded site never stays archived.
    pub(crate) exclude_pending: HashSet<String>,
    /// App windows (P2.1).
    pub(crate) app_windows: crate::windows::AppWindowsState,
    /// Window commands waiting for the actor's answer (P2.2), by item id.
    pub(crate) window_ops: HashMap<String, crate::hide::PendingWindowOp>,
    /// When the last hide sweep started.
    pub(crate) last_hide_at: Option<i64>,
    /// In a meeting until then (F12): no scheduled sweeps.
    pub(crate) meeting_until: Option<i64>,
    /// Why each pending hide was chosen (written with its answer).
    pub(crate) hide_reasons: HashMap<String, decay_core::Reason>,
    /// Hides that timed out: a late success is still applied until the time given.
    pub(crate) late_hides: HashMap<String, (String, i64)>,
    pub(crate) next_op: u64,
    pub(crate) sweep: SweepState,
    pub(crate) clock: AgingClock,
    pub(crate) last_tick_at: Option<i64>,
    pub(crate) next_tick_at: i64,
    /// Last deliberate browser activity (tab switch, window focus, new tab).
    pub(crate) last_activity_at: Option<i64>,
    /// The latest OS sample and when it came (see `platform`).
    pub(crate) platform: Option<(decay_platform::PlatformSample, i64)>,
    /// When the frontmost app last changed.
    pub(crate) last_app_switch_at: Option<i64>,
    /// Some moment since the last tick was locked, had the displays asleep or wasn't
    /// sampled (the machine slept or stalled): that tick earns no credit.
    pub(crate) away_since_tick: bool,
    /// When the machine last woke or was unlocked (a "morning cleanup" may run then).
    pub(crate) woke_at: Option<i64>,
    pub(crate) last_sweep_at: Option<i64>,
    pub(crate) next_maintenance_at: i64,
    /// The status last published (see `control`).
    pub(crate) last_status: Option<EngineStatus>,
    pub(crate) out: Vec<Output>,
}

/// Engine operations fail only on storage errors; the caller logs them and carries on
/// (the next snapshot reconciles whatever was left half-done).
pub(crate) type Res<T = ()> = Result<T, StoreError>;

/// Compares dotted version numbers (`0.10.1` > `0.9`); missing or odd parts count as 0.
pub(crate) fn version_at_least(version: &str, min: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> {
        v.split(['.', '-', '+'])
            .take(3)
            .map(|p| p.parse().unwrap_or(0))
            .chain(std::iter::repeat(0))
            .take(3)
            .collect()
    };
    parts(version) >= parts(min)
}

/// The browser's app id on macOS (its bundle id), for "is it still running?".
pub fn browser_bundle_id(kind: BrowserKind) -> &'static str {
    browser_app(kind, Platform::MacOs).0
}

/// The browser's catalog app id and name on this platform (matches the import mapping).
pub(crate) fn browser_app(kind: BrowserKind, platform: Platform) -> (&'static str, &'static str) {
    match (kind, platform) {
        (BrowserKind::Chrome, Platform::MacOs) => ("com.google.Chrome", "Google Chrome"),
        (BrowserKind::Chrome, Platform::Windows) => ("chrome.exe", "Google Chrome"),
        (BrowserKind::Edge, Platform::MacOs) => ("com.microsoft.edgemac", "Microsoft Edge"),
        (BrowserKind::Edge, Platform::Windows) => ("msedge.exe", "Microsoft Edge"),
        (BrowserKind::Brave, Platform::MacOs) => ("com.brave.Browser", "Brave"),
        (BrowserKind::Brave, Platform::Windows) => ("brave.exe", "Brave"),
        (BrowserKind::Arc, Platform::MacOs) => ("company.thebrowser.Browser", "Arc"),
        (BrowserKind::Arc, Platform::Windows) => ("arc.exe", "Arc"),
        (BrowserKind::Chromium | BrowserKind::Other, Platform::MacOs) => {
            ("org.chromium.Chromium", "Chromium")
        }
        (BrowserKind::Chromium | BrowserKind::Other, Platform::Windows) => {
            ("chromium.exe", "Chromium")
        }
    }
}

fn ack_payload(ok: bool, error: Option<ErrorPayload>, result: Option<Value>) -> Ack {
    Ack { ok, error, result }
}

impl Engine {
    /// An engine over an open store. Sweeps left unfinished by a crash are settled when
    /// their browser's next snapshot arrives.
    pub fn new(store: Store, config: EngineConfig, now: i64) -> Result<Self, StoreError> {
        let policy = Arc::new(PolicySettings::load(&store)?);
        // The last tab sweep (hide sweeps keep their own interval).
        let last_sweep_at = store
            .recent_sweeps(20)?
            .into_iter()
            .find(|s| !crate::hide::is_hide_record(s))
            .map(|s| s.started_at);
        let unfinished = store.unfinished_sweeps()?.len();
        if unfinished > 0 {
            tracing::info!(unfinished, "sweeps to settle when their browser reconnects");
        }
        let clock =
            AgingClock { tick_ms: timing::TICK_MS as u64, idle_cutoff_secs: policy.idle_cutoff_s };
        let mut engine = Self {
            store,
            config,
            policy,
            conns: BTreeMap::new(),
            pending: HashMap::new(),
            restore_ops: HashMap::new(),
            restoring: HashSet::new(),
            exclude_pending: HashSet::new(),
            app_windows: crate::windows::AppWindowsState::default(),
            window_ops: HashMap::new(),
            last_hide_at: None,
            meeting_until: None,
            hide_reasons: HashMap::new(),
            late_hides: HashMap::new(),
            next_op: 0,
            sweep: SweepState::Idle,
            clock,
            last_tick_at: None,
            next_tick_at: now + timing::TICK_MS,
            last_activity_at: None,
            platform: None,
            last_app_switch_at: None,
            away_since_tick: false,
            woke_at: None,
            last_sweep_at,
            next_maintenance_at: now,
            last_status: None,
            out: Vec::new(),
        };
        // Window handling turned off while Decay wasn't running: no window records stay.
        if !engine.wants_windows()
            && let Err(e) = engine.forget_windows(now)
        {
            engine.problem("forgetting window records", &e);
        }
        Ok(engine)
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// The store, for writes outside the engine's own (deleting archived items). Live
    /// items stay the engine's: callers only touch archived ones.
    pub fn store_mut(&mut self) -> &mut Store {
        &mut self.store
    }

    /// Gives the store back (shutdown).
    pub fn into_store(self) -> Store {
        self.store
    }

    /// Handles one input.
    pub fn handle(&mut self, input: Input, now: i64) -> Vec<Output> {
        match input {
            Input::Connected(id) => {
                self.conns.insert(
                    id,
                    Conn {
                        info: None,
                        reassembler: Reassembler::new(),
                        adapter: None,
                        last_message_at: now,
                    },
                );
            }
            Input::Frame(id, frame) => self.frame(id, *frame, now),
            Input::Disconnected(id) => self.disconnected(id, now),
            Input::Platform(sample) => self.on_platform(*sample, now),
            Input::Windows(list) => self.on_windows(*list, now),
            Input::WindowActed { item_id, result } => self.on_window_acted(&item_id, &result, now),
        }
        self.purge_settled_excludes();
        std::mem::take(&mut self.out)
    }

    /// Runs the timers: ack deadlines, resyncs, the aging tick, the sweep scheduler and
    /// maintenance. Call it every second or so.
    pub fn poll(&mut self, now: i64) -> Vec<Output> {
        for conn in self.conns.values_mut() {
            conn.reassembler.expire(now);
        }
        let expired: Vec<String> = self
            .pending
            .iter()
            .filter(|(_, p)| p.deadline <= now)
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            let Some(mut cmd) = self.pending.remove(&id) else { continue };
            if let Pending::Close { sweep_id, .. } = &cmd.what
                && !cmd.late
            {
                // Late, not lost: the sweep moves on, the tabs stay in flight a while.
                tracing::warn!(conn = cmd.conn, "close is late");
                let sweep_id = sweep_id.clone();
                self.close_progress(&sweep_id);
                cmd.late = true;
                cmd.deadline = now + timing::LATE_CLOSE_MS;
                let conn = cmd.conn;
                self.pending.insert(id, cmd);
                if let Some(a) = self.conns.get_mut(&conn).and_then(|c| c.adapter.as_mut()) {
                    a.resync_at = Some(now);
                }
                continue;
            }
            tracing::warn!(conn = cmd.conn, "command timed out");
            self.unanswered(cmd, now);
        }
        let resync: Vec<ConnId> = self
            .conns
            .iter()
            .filter_map(|(id, c)| {
                let a = c.adapter.as_ref()?;
                // No first snapshot yet (it failed, e.g. no browser window open): ask again.
                let due = if a.ready {
                    a.resync_at.is_some_and(|t| t <= now)
                        || now - a.last_snapshot_at >= timing::RESYNC_EVERY_MS
                } else {
                    now - a.last_snapshot_at >= timing::FIRST_SNAPSHOT_RETRY_MS
                };
                due.then_some(*id)
            })
            .collect();
        for id in resync {
            if let Some(a) = self.conns.get_mut(&id).and_then(|c| c.adapter.as_mut()) {
                a.resync_at = None;
                // Not again until this one is answered (or the periodic one is due).
                a.last_snapshot_at = now;
            }
            self.command(
                id,
                kinds::RESYNC,
                &serde_json::json!({}),
                Pending::Plain,
                CommandKind::Other,
                now,
            );
        }
        if now >= self.next_tick_at {
            self.next_tick_at = now + timing::TICK_MS;
            if let Err(e) = self.tick(now) {
                self.problem("aging tick", &e);
            }
            self.maybe_sweep(now, decay_core::SweepTrigger::Scheduled, false);
            self.maybe_hide_windows(now);
        }
        self.expire_window_ops(now);
        if now >= self.next_maintenance_at {
            self.next_maintenance_at = now + timing::MAINTENANCE_EVERY_MS;
            if let Err(e) = self.maintenance(now) {
                self.problem("maintenance", &e);
            }
        }
        self.purge_settled_excludes();
        self.publish_status(now);
        std::mem::take(&mut self.out)
    }

    /// "Sweep now" (`run_sweep_now`): a real sweep in automatic mode, a simulation in shadow
    /// mode; it skips the interval and the quiet-moment wait.
    pub fn sweep_now(&mut self, now: i64) -> Result<Vec<Output>, SweepRefused> {
        if let Err(e) = self.reload_policy() {
            self.problem("settings", &e);
        }
        if self.policy.mode == Mode::Paused {
            return Err(SweepRefused::Paused);
        }
        if let Some(until) = self.policy.hold_until.filter(|t| *t > now) {
            return Err(SweepRefused::OnHold(until));
        }
        if !matches!(self.sweep, SweepState::Idle) {
            return Err(SweepRefused::Busy);
        }
        if !self.conns.values().any(|c| Self::live_adapter(c, now).is_some()) {
            return Err(SweepRefused::NoBrowser);
        }
        self.maybe_sweep(now, decay_core::SweepTrigger::Manual, true);
        Ok(std::mem::take(&mut self.out))
    }

    /// Development only (`decay-engine-dev`): adds active time to every open tab of the
    /// connected browsers, so sweeps can be tried without waiting hours. Returns how many.
    #[doc(hidden)]
    pub fn dev_add_age(&mut self, secs: u64, now: i64) -> usize {
        let mut ages: Vec<(String, u64)> = Vec::new();
        for conn in self.conns.values_mut() {
            let Some(a) = conn.adapter.as_mut() else { continue };
            for item in a.tabs.values_mut().filter_map(|t| t.item.as_mut()) {
                if item.state.is_live() && item.state != decay_core::ItemState::Archiving {
                    item.active_age_s = item.active_age_s.saturating_add(secs);
                    ages.push((item.id.clone(), item.active_age_s));
                }
            }
        }
        for item in self.app_windows.tracked.values_mut().map(|t| &mut t.item) {
            if matches!(item.state, decay_core::ItemState::Active | decay_core::ItemState::Stale) {
                item.active_age_s = item.active_age_s.saturating_add(secs);
                ages.push((item.id.clone(), item.active_age_s));
            }
        }
        let refs: Vec<(&str, u64)> = ages.iter().map(|(id, a)| (id.as_str(), *a)).collect();
        if let Err(e) = self.store.set_active_ages(&refs) {
            self.problem("saving ages", &e);
        }
        // The lifecycle catches up at the next tick.
        self.next_tick_at = self.next_tick_at.min(now);
        refs.len()
    }

    /// One line about the engine's state (logs, the dev tool).
    pub fn status_line(&self) -> String {
        let adapters: Vec<String> = self
            .conns
            .values()
            .filter_map(|c| c.adapter.as_ref())
            .map(|a| {
                let tracked = a.tabs.values().filter(|t| t.item.is_some()).count();
                format!(
                    "{:?}/{}: {tracked} tabs{}",
                    a.browser,
                    a.profile,
                    if a.ready { "" } else { " (waiting)" }
                )
            })
            .collect();
        format!(
            "mode {:?}, sweep {:?}, {} pending, browsers [{}], os {:?}",
            self.policy.mode,
            self.sweep,
            self.pending.len(),
            adapters.join(", "),
            self.platform.as_ref().map(|(p, _)| p)
        )
    }

    // --- Plumbing ------------------------------------------------------------------

    pub(crate) fn problem(&mut self, what: &str, e: &dyn std::fmt::Display) {
        tracing::error!("{what} failed: {e}");
        self.out.push(Output::Event(EngineEvent::Problem(format!("{what}: {e}"))));
    }

    pub(crate) fn send(&mut self, conn: ConnId, env: Envelope) {
        self.out.push(Output::Send(conn, env));
    }

    /// Sends a command and waits for its ack.
    pub(crate) fn command(
        &mut self,
        conn: ConnId,
        kind: &str,
        payload: &impl Serialize,
        what: Pending,
        timeout: CommandKind,
        now: i64,
    ) -> Option<String> {
        let env = match Envelope::new(kind, payload) {
            Ok(env) => env,
            Err(e) => {
                self.problem("building a command", &e);
                return None;
            }
        };
        let items = match &what {
            Pending::Probe { tab_ids } => tab_ids.len(),
            Pending::Close { tabs, .. } => tabs.len(),
            Pending::Restore { item_ids, .. } => item_ids.len(),
            Pending::Plain => 0,
        };
        let id = env.id.clone();
        self.pending.insert(
            id.clone(),
            PendingCmd {
                conn,
                what,
                deadline: now + timing::ack_timeout_ms(timeout, items),
                late: false,
            },
        );
        self.send(conn, env);
        Some(id)
    }

    /// Replies to an extension request with an ack.
    pub(crate) fn reply_ack(&mut self, conn: ConnId, request_id: &str, ack: &Ack) {
        let mut env = match Envelope::new(kinds::ACK, ack) {
            Ok(env) => env,
            Err(e) => {
                self.problem("building an ack", &e);
                return;
            }
        };
        env.reply_to = Some(request_id.to_owned());
        self.send(conn, env);
    }

    pub(crate) fn reply_ok(&mut self, conn: ConnId, request_id: &str, result: &impl Serialize) {
        let result = serde_json::to_value(result).ok();
        self.reply_ack(conn, request_id, &ack_payload(true, None, result));
    }

    pub(crate) fn reply_err(&mut self, conn: ConnId, request_id: &str, code: ErrorCode, msg: &str) {
        let error = ErrorPayload { code, message: msg.to_owned() };
        self.reply_ack(conn, request_id, &ack_payload(false, Some(error), None));
    }

    /// A connected, ready adapter that has spoken recently.
    pub(crate) fn live_adapter(conn: &Conn, now: i64) -> Option<&Adapter> {
        conn.adapter
            .as_ref()
            .filter(|a| a.ready && now - conn.last_message_at < timing::STALE_LINK_MS)
    }

    /// A transaction rolled back after the adapter's caches were changed: forget what may
    /// be wrong (known sources) and rebuild the rest from a fresh snapshot.
    pub(crate) fn recover_cache(&mut self, conn: ConnId, now: i64) {
        if let Some(a) = self.conns.get_mut(&conn).and_then(|c| c.adapter.as_mut()) {
            a.sources.clear();
            a.resync_at = Some(now);
        }
    }

    pub(crate) fn reload_policy(&mut self) -> Res {
        let policy = PolicySettings::load(&self.store)?;
        self.clock.idle_cutoff_secs = policy.idle_cutoff_s;
        self.policy = Arc::new(policy);
        Ok(())
    }

    /// Sends every adapter the current exclusions (they answer with a fresh snapshot).
    pub(crate) fn broadcast_config(&mut self, now: i64) {
        let ids: Vec<ConnId> =
            self.conns.iter().filter(|(_, c)| c.adapter.is_some()).map(|(id, _)| *id).collect();
        for id in ids {
            self.send_config(id, now);
        }
    }

    fn send_config(&mut self, conn: ConnId, now: i64) {
        let config =
            Config { probe_enabled: true, excluded_domains: self.policy.excluded_domains() };
        self.command(conn, kinds::CONFIG, &config, Pending::Plain, CommandKind::Other, now);
    }

    // --- Messages ------------------------------------------------------------------

    fn frame(&mut self, id: ConnId, frame: HostFrame, now: i64) {
        let Some(conn) = self.conns.get_mut(&id) else { return };
        conn.last_message_at = now;
        if conn.info.is_none() {
            conn.info = Some(frame.conn.clone());
        }
        let env = match conn.reassembler.push(frame.msg, now) {
            Ok(Some(env)) => env,
            Ok(None) => return,
            Err(e) => {
                tracing::warn!(conn = id, "dropped a broken chunk group: {e}");
                return;
            }
        };
        if conn.adapter.is_none() {
            if env.kind == kinds::HELLO {
                self.hello(id, &env, now);
            } else {
                tracing::warn!(conn = id, kind = env.kind, "message before hello");
            }
            return;
        }
        if let Err(e) = self.message(id, &env, now) {
            self.problem(&format!("handling {}", env.kind), &e);
        }
    }

    fn hello(&mut self, id: ConnId, env: &Envelope, now: i64) {
        let hello: Hello = match env.payload_as() {
            Ok(h) => h,
            Err(e) => {
                let err = Envelope::error(Some(env), ErrorCode::BadMessage, e.to_string());
                self.send(id, err);
                self.reject(id);
                return;
            }
        };
        let profile_ok = !hello.profile_id.is_empty()
            && hello.profile_id.len() <= decay_bridge::protocol::MAX_PROFILE_ID_LEN
            && !hello.profile_id.contains(':');
        if !profile_ok {
            let err = Envelope::error(Some(env), ErrorCode::BadMessage, "bad profileId");
            self.send(id, err);
            self.reject(id);
            return;
        }
        if !version_at_least(&hello.ext_version, MIN_EXT_VERSION) {
            let msg = format!("Decay needs extension {MIN_EXT_VERSION} or newer");
            self.send(id, Envelope::error(Some(env), ErrorCode::VersionMismatch, msg));
            self.reject(id);
            return;
        }
        let (app_id, app_name) = browser_app(hello.browser, self.config.platform);
        if let Err(e) = self.ensure_app(app_id, app_name) {
            self.problem("registering the browser", &e);
        }
        // One connection per browser profile: a newer one (worker restart) replaces it.
        let stale: Vec<ConnId> = self
            .conns
            .iter()
            .filter(|(other, c)| {
                **other != id
                    && c.adapter
                        .as_ref()
                        .is_some_and(|a| a.app_id == app_id && a.profile == hello.profile_id)
            })
            .map(|(other, _)| *other)
            .collect();
        for other in stale {
            self.disconnected(other, now);
            // The writer is dropped but the host keeps the socket open while it reads: this
            // tells the extension to end its port, and the host then exits.
            let bye =
                Envelope::error(None, ErrorCode::AppUnavailable, "replaced by a newer connection");
            self.send(other, bye);
            self.out.push(Output::Close(other));
        }
        let session = hello
            .session
            .clone()
            .filter(|s| !s.is_empty() && s.len() <= 128)
            .unwrap_or_else(|| format!("conn{id}"));
        let adapter = Adapter {
            app_id: app_id.to_owned(),
            profile: hello.profile_id.clone(),
            browser: hello.browser,
            session,
            ready: false,
            tabs: HashMap::new(),
            windows: HashMap::new(),
            groups: HashMap::new(),
            probes: HashMap::new(),
            missing: HashMap::new(),
            resync_at: None,
            last_snapshot_at: now,
            sources: HashSet::new(),
            used_while_closing: HashSet::new(),
            restore_links: HashMap::new(),
        };
        if let Some(conn) = self.conns.get_mut(&id) {
            conn.adapter = Some(adapter);
        }
        if let Err(e) = self.reload_policy() {
            self.problem("settings", &e);
        }
        let welcome = Welcome {
            core_version: CORE_VERSION.to_owned(),
            min_ext_version: MIN_EXT_VERSION.to_owned(),
            mode: self.policy.mode,
            config: WelcomeConfig { probe_enabled: true, heartbeat_sec: timing::HEARTBEAT_SEC },
        };
        match env.reply(kinds::WELCOME, &welcome) {
            Ok(reply) => self.send(id, reply),
            Err(e) => self.problem("building welcome", &e),
        }
        // The extension reports nothing before this `config`, then sends its snapshot.
        self.send_config(id, now);
        self.out.push(Output::Event(EngineEvent::AdapterConnected {
            browser: hello.browser,
            profile_id: hello.profile_id,
        }));
    }

    /// Refuses a connection whose `hello` failed (the error was sent): forgotten here, and
    /// the extension ends its port on that error, which ends the host.
    fn reject(&mut self, id: ConnId) {
        self.conns.remove(&id);
        self.out.push(Output::Close(id));
    }

    fn ensure_app(&self, id: &str, name: &str) -> Res {
        if self.store.app(id)?.is_none() {
            self.store.upsert_app(&App {
                id: id.to_owned(),
                name: name.to_owned(),
                platform: self.config.platform,
                is_browser: true,
                close_allowed: false,
                restore_kind: RestoreKind::Url,
                icon_png: None,
            })?;
        }
        Ok(())
    }

    fn message(&mut self, id: ConnId, env: &Envelope, now: i64) -> Res {
        use decay_bridge::protocol as p;
        let kind = env.kind.as_str();
        // Payloads that don't parse are dropped (requests get an error ack).
        macro_rules! payload {
            ($t:ty) => {
                match env.payload_as::<$t>() {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::warn!(conn = id, "bad {kind} payload: {e}");
                        return Ok(());
                    }
                }
            };
        }
        macro_rules! request {
            ($t:ty) => {
                match env.payload_as::<$t>() {
                    Ok(p) => p,
                    Err(e) => {
                        self.reply_err(id, &env.id, ErrorCode::BadMessage, &e.to_string());
                        return Ok(());
                    }
                }
            };
        }
        match kind {
            // Already connected; liveness; verdicts come in the probe's ack (P1.4).
            kinds::HELLO | kinds::HEARTBEAT | kinds::PROBE_RESULT => {}
            kinds::SNAPSHOT => self.on_snapshot(id, payload!(p::Snapshot), now)?,
            kinds::TAB_CREATED => self.on_tab_created(id, payload!(p::TabInfo), now)?,
            kinds::TAB_UPDATED => self.on_tab_updated(id, &payload!(p::TabUpdated), now)?,
            kinds::TAB_REMOVED => self.on_tab_removed(id, payload!(p::TabRemoved), now)?,
            kinds::TAB_MOVED => self.on_tab_moved(id, payload!(p::TabMoved), now)?,
            kinds::TAB_USED => self.on_tab_used(id, &payload!(p::TabUsed), now)?,
            kinds::WINDOW_FOCUS => self.on_window_focus(id, payload!(p::WindowFocus), now)?,
            kinds::ACK => {
                let ack = payload!(Ack);
                if let Some(reply_to) = env.reply_to.as_deref()
                    && let Some(cmd) = self.pending.remove(reply_to)
                {
                    if cmd.conn == id {
                        self.answered(cmd, &ack, now);
                    } else {
                        self.pending.insert(reply_to.to_owned(), cmd);
                    }
                }
            }
            kinds::ERROR => {
                let err = env.payload_as::<ErrorPayload>().ok();
                tracing::warn!(conn = id, ?err, "error from the extension");
            }
            kinds::IMPORT_ARCHIVE => self.import(id, env, now),
            kinds::ARCHIVE_SEARCH => {
                self.on_search(id, &env.id, &request!(p::ArchiveSearch), now)?;
            }
            kinds::ARCHIVE_RESTORE => {
                self.on_restore(id, &env.id, request!(p::ArchiveRestore), now)?;
            }
            kinds::ARCHIVE_RESTORE_BATCH => {
                self.on_restore_batch(id, &env.id, &request!(p::ArchiveRestoreBatch), now)?;
            }
            kinds::ARCHIVE_UNDO => self.on_undo(id, &env.id, &request!(p::ArchiveUndo), now)?,
            other => tracing::debug!(conn = id, kind = other, "ignored message"),
        }
        Ok(())
    }

    fn import(&mut self, id: ConnId, env: &Envelope, now: i64) {
        let before = self.policy.excluded_domains();
        match self.store.import_archive(&env.payload, self.config.platform, now) {
            Ok(report) => {
                self.reply_ok(id, &env.id, &report);
                self.after_import(&report, &before, now);
            }
            Err(e) => {
                self.problem("importing the archive", &e);
                self.reply_err(id, &env.id, ErrorCode::BadMessage, &e.to_string());
            }
        }
    }

    /// After an import: settings broadcast, the policy reloaded, new exclusions sent to
    /// every extension before more is reported.
    pub(crate) fn after_import(
        &mut self,
        report: &decay_store::ImportReport,
        before: &[String],
        now: i64,
    ) {
        if report.settings_imported > 0 {
            match self.store.settings_snapshot() {
                Ok(all) => self.out.push(Output::Event(EngineEvent::SettingsChanged(all))),
                Err(e) => self.problem("settings", &e),
            }
        }
        if let Err(e) = self.reload_policy() {
            self.problem("settings", &e);
        }
        if self.policy.excluded_domains() != before {
            self.broadcast_config(now);
        }
    }

    /// Where imported bookmarks will reopen: the connected browser heard from most
    /// recently (ties: by app id, so the choice never depends on map order); with none
    /// connected, the browser of the most recently archived tab; else Chrome.
    pub fn import_browser_app(&self, now: i64) -> String {
        self.conns
            .values()
            .filter_map(|c| Self::live_adapter(c, now).map(|a| (c.last_message_at, &a.app_id)))
            .max_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(a.1)))
            .map(|(_, app)| app.clone())
            .or_else(|| self.store.latest_tab_app().ok().flatten())
            .unwrap_or_else(|| browser_app(BrowserKind::Chrome, self.config.platform).0.to_owned())
    }

    fn answered(&mut self, cmd: PendingCmd, ack: &Ack, now: i64) {
        let PendingCmd { conn, what, late, .. } = cmd;
        if let Pending::Close { sweep_id, .. } = &what
            && !late
        {
            self.close_progress(sweep_id);
        }
        let result = match what {
            Pending::Plain => Ok(()),
            Pending::Probe { tab_ids } => {
                self.probe_answered(conn, &tab_ids, Some(ack), now);
                Ok(())
            }
            Pending::Close { sweep_id, tabs } => {
                self.close_answered(conn, &sweep_id, &tabs, Some(ack), now)
            }
            Pending::Restore { op, item_ids } => {
                self.restore_answered(conn, op, &item_ids, Some(ack), now)
            }
        };
        if let Err(e) = result {
            self.problem("handling an ack", &e);
        }
    }

    /// A command that will never be answered (timeout, lost connection).
    fn unanswered(&mut self, cmd: PendingCmd, now: i64) {
        let PendingCmd { conn, what, late, .. } = cmd;
        if let Pending::Close { sweep_id, .. } = &what
            && !late
        {
            self.close_progress(sweep_id);
        }
        let result = match what {
            Pending::Plain => Ok(()),
            Pending::Probe { tab_ids } => {
                self.probe_answered(conn, &tab_ids, None, now);
                Ok(())
            }
            Pending::Close { sweep_id, tabs } => {
                self.close_answered(conn, &sweep_id, &tabs, None, now)
            }
            Pending::Restore { op, item_ids } => {
                self.restore_answered(conn, op, &item_ids, None, now)
            }
        };
        if let Err(e) = result {
            self.problem("handling a lost command", &e);
        }
    }

    fn disconnected(&mut self, id: ConnId, now: i64) {
        let Some(conn) = self.conns.remove(&id) else { return };
        // Persist the ages earned since the last tick write (nothing is lost but < 30 s).
        if let Some(a) = &conn.adapter {
            let ages: Vec<(&str, u64)> = a
                .tabs
                .values()
                .filter_map(|t| t.item.as_ref())
                .map(|i| (i.id.as_str(), i.active_age_s))
                .collect();
            if let Err(e) = self.store.set_active_ages(&ages) {
                self.problem("saving ages", &e);
            }
            self.out.push(Output::Event(EngineEvent::AdapterDisconnected {
                browser: a.browser,
                profile_id: a.profile.clone(),
            }));
        }
        let lost: Vec<String> =
            self.pending.iter().filter(|(_, p)| p.conn == id).map(|(k, _)| k.clone()).collect();
        for key in lost {
            if let Some(cmd) = self.pending.remove(&key) {
                self.unanswered(cmd, now);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert!(version_at_least("0.1.0", "0.1.0"));
        assert!(version_at_least("0.10.0", "0.9.9"));
        assert!(version_at_least("1", "0.9"));
        assert!(!version_at_least("0.0.9", "0.1.0"));
        assert!(!version_at_least("garbage", "0.1.0"));
        assert!(version_at_least("0.1.0-beta", "0.1.0"));
    }

    #[test]
    fn browser_ids_match_the_catalog() {
        for kind in [BrowserKind::Chrome, BrowserKind::Edge, BrowserKind::Brave] {
            for platform in [Platform::MacOs, Platform::Windows] {
                let (id, _) = browser_app(kind, platform);
                let ext = match kind {
                    BrowserKind::Chrome => "com.google.Chrome",
                    BrowserKind::Edge => "com.microsoft.Edge",
                    _ => "com.brave.Browser",
                };
                // Live items and imported ones share the app id.
                assert_eq!(id, decay_store::catalog_app_id(ext, platform));
            }
        }
    }
}
