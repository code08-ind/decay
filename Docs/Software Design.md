# Decay — Software Design Document

| | |
| --- | --- |
| **Version** | 1.0 |
| **Date** | 4 Nov 2026 |
| **Related** | [PRD](./PRD.md) · [DB Design](./DB%20Design.md) · [API Design](./API%20Design.md) · [Technical Stack](./Technical%20Stack%20Document.md) |

---

## 1. Design principles

1. **Fail by doing nothing.** Every destructive action needs positive proof of safety. Unknown means protected.
2. **Write before close.** The archive record is committed (fsync'd, WAL) before any close command is sent.
3. **The core decides, adapters obey.** All policy lives in the Rust core. Adapters (extension, platform modules) only report observations and run commands. A new browser or OS means a new adapter, not a new product.
4. **Local only.** No network except update checks and opt-in crash reports.
5. **Explainable.** Every state transition stores a machine-readable reason, shown to the user.

## 2. System context

```
┌──────────────────────── User's machine ─────────────────────────────────────────────┐
│                                                                                       │
│  ┌──────────── Chromium browser(s) ─────────┐        ┌────── Decay.app / Decay.exe ───┐ │
│  │  Decay MV3 extension                     │        │                                 │ │
│  │  • tab observer  • content probe         │ stdio  │  ┌────────┐   local IPC        │ │
│  │  • command runner • chunker              │◄──────►│  │decay-  │◄─────────────┐     │ │
│  └──────────────────────────────────────────┘ native │  │host    │ UDS / pipe   │     │ │
│                                              msging  │  └────────┘              ▼     │ │
│                                                      │  ┌───────────── Rust core ───┐ │ │
│  ┌──── Other apps (Finder, Preview, …) ────┐          │  │ Aging clock · Policy      │ │ │
│  │ windows                                  │◄─AX/────│  │ Sweep scheduler/executor  │ │ │
│  └──────────────────────────────────────────┘ Win32   │  │ Store (SQLCipher+FTS5)    │ │ │
│                                                      │  │ Platform module (mac/win) │ │ │
│                                                      │  └───────────┬───────────────┘ │ │
│                                                      │   Tauri IPC  │ commands/events │ │
│                                                      │  ┌───────────▼───────────────┐ │ │
│                                                      │  │ WebView UI (React)        │ │ │
│                                                      │  │ Panel · Library · Settings│ │ │
│                                                      │  └───────────────────────────┘ │ │
│                                                      └─────────────────────────────────┘ │
└───────────────────────────────────────────────────────────────────────────────────────┘
```

## 3. Repository layout (monorepo)

```
decay/
├─ apps/
│  ├─ desktop/                # Tauri app
│  │  ├─ src/                 # React UI (panel, library, settings, onboarding, tray popover)
│  │  └─ src-tauri/           # Tauri shell crate: commands, windows, tray, plugins wiring
│  └─ extension/              # WXT MV3 extension (Phase 0 standalone + Phase 1 adapter)
├─ crates/
│  ├─ decay-core/             # Domain: Item, State, Rules, Policy, AgingClock, Sweep planner (pure, no I/O)
│  ├─ decay-store/            # SQLite/SQLCipher, migrations, FTS, repositories, backup
│  ├─ decay-engine/           # Orchestration: scheduler, executor, adapters registry, event bus (tokio)
│  ├─ decay-platform/         # trait Platform + mac/ + windows/ impls (cfg-gated)
│  ├─ decay-bridge/           # Native messaging protocol types, chunking, IPC client/server
│  ├─ decay-host/             # Binary launched by Chrome: stdio ⇄ IPC relay
│  └─ decay-license/          # Ed25519 license verification
├─ packages/
│  ├─ ui/                     # Shared React components + design tokens (from Stitch)
│  ├─ protocol/               # Generated TS types (tauri-specta + bridge schema)
│  └─ config/                 # Shared tsconfig/biome/tailwind presets
├─ Docs/
└─ .github/workflows/
```

**Licensing split (open core):** `crates/decay-core`, `decay-store`, `decay-bridge`, `decay-host` and `apps/extension` are MIT/Apache-2.0. `decay-license`, Pro features and the app shell are proprietary (or source-available).

## 4. Components

### 4.1 `decay-core` (pure domain)

- **Entities:** `Item { id, kind: Tab|Window, source, state, active_age, last_used_at, protections, … }`, `Rule`, `Sweep`, `Event`.
- **`Lifecycle` state machine:**

| From | Event | Guard | To |
| --- | --- | --- | --- |
| Active | `tick` | `active_age ≥ T_stale` ∧ ¬protected | Stale |
| Stale | `tick` | kind=Window ∧ `age ≥ T_hidden` ∧ hideable | Hidden |
| Stale | `sweep` | kind=Tab ∧ `age ≥ T_archive` ∧ ¬protected | Archived |
| Hidden | `sweep` | `age ≥ T_archive` ∧ app ∈ close allowlist ∧ state=Saved ∧ Pro | Archived |
| any live | `used` | — | Active (age=0) |
| Archived | `restore` | — | Active (new live item linked via `restored_from`) |
| Archived | `retention` | `archived_at + retention < now` | Purged |

  Property tests (`proptest`) assert that **no path reaches Archived while protected** and that Hidden is never skipped for windows.

- **`Policy::evaluate(item, ctx) -> Decision { action, reason }`**. Ordering: global holds → hard protections → user rules (most specific first: URL glob > domain > app) → thresholds. Reasons are enums, e.g. `Reason::Unused { active_hours: 61 }`, `Reason::ProtectedByRule(rule_id)`, `Reason::UnknownSaveState`.
- **`AgingClock`:** pure accounting. Given `ActivitySample { awake, unlocked, idle_secs, audio_playing }` it returns `credit_secs` (0 or the tick length).

### 4.2 Aging clock (runtime)

- Ticks every **30 s** (tokio interval).
- `credit = tick` if awake ∧ unlocked ∧ (idle < 300 s ∨ any audio playing); otherwise `0`.
- For every live item **not in use** at the tick, `active_age += credit`.
- "In use" = focused window/tab, a visible tab (active tab of a visible, non-minimized window), audible, or typed into within the tick.
- Sleep/wake and lock events immediately freeze the clock (no back-credit).
- Ages are persisted in batches every 5 min, plus on shutdown (crash loses ≤ 5 min of age — this is safe, because items just look newer).

### 4.3 Sweep scheduler

Runs a candidate sweep when **all** are true:
- At least 15 min since the last sweep, and candidates exist.
- No typing for ≥ 60 s; no app switch for ≥ 30 s ("quiet moment"), **or** the user just unlocked or woke the machine (morning cleanup).
- No hold, meeting, screen share or permission problem.
- Not in shadow mode → real sweep; in shadow mode → simulated sweep (`sweeps.is_shadow = 1`).

Max **50 items per sweep** (configurable) to keep Undo meaningful.

### 4.4 Sweep executor (write-before-close)

```
plan = Policy.plan(candidates)               # pure
BEGIN TX
  INSERT sweep(status='pending')
  for item in plan: write restore recipe, item.state='archiving', sweep_items row
COMMIT (fsync)
for each adapter batch (per browser/profile, per app):
  send Close/Hide command (idempotent, by external id)
  await ack (timeout 5 s)
  ack ok      → item.state = archived | hidden ; event 'archived'
  ack failed  → item.state = previous ; event 'close_failed' (record kept, harmless)
  timeout     → re-query adapter for existence; if still open → previous state
UPDATE sweep(status='done', item_count, failed_count)
emit SweepCompleted → UI toast with Undo
```

Crash recovery at startup: any `archiving` items are reconciled against the adapter's live snapshot (still open → revert; gone → archived).

### 4.5 Undo and restore

- **Undo sweep:** for every item in the sweep → restore command, `sweeps.undone_at = now`, and an `undo` event per item. Restored items get a **48-hour grace protection** so they aren't re-archived immediately.
- **Restore tab:** `tabs.create({url, windowId?, index, pinned:false})`. If the original window is gone, create a new window; rebuild the group via `tabGroups.update`.
- **Batch restore:** one new window per original source window, ordered by `tab_index`, groups recreated.
- **Restore window:** Hidden → un-minimize + raise. Archived (closed) → reopen the document/folder via `open -a <app> <path>` (macOS) or `ShellExecuteW` (Windows).
- Each restore counts toward the **regret metric** if it happens within 24 h of archiving.

### 4.6 Adapters

**Adapter trait (Rust):**
```rust
#[async_trait]
trait Adapter: Send + Sync {
    fn id(&self) -> AdapterId;                         // e.g. chrome:Default, edge:Profile 1, platform
    async fn snapshot(&self) -> Result<Vec<LiveItem>>; // full sync
    fn observations(&self) -> BoxStream<'_, Observation>; // used / opened / closed / updated
    async fn execute(&self, cmd: Command) -> Result<Ack>;  // Close | Hide | Restore | Focus
    fn health(&self) -> AdapterHealth;                 // Connected | Degraded(reason) | Down
}
```

- **BrowserAdapter:** one per connected extension instance (browser + profile), over the IPC from `decay-host`.
- **PlatformAdapter:** a `MacPlatform` or `WinPlatform` implementation of `trait Platform` (enumerate, observe focus, minimize, close, document path, idle, power, meeting signals).
- When an adapter goes `Down`, all its items are frozen (no sweeps) and a banner is shown (F33).

### 4.7 Browser extension

| Module | Responsibility |
| --- | --- |
| `background/tracker.ts` | Listens to `tabs.onActivated/onUpdated/onRemoved/onCreated`, `windows.onFocusChanged`, `tabGroups.*`; tracks audible, pinned, discarded |
| `background/probe.ts` | On demand, injects a content script (`scripting.executeScript`) to check for dirty forms or `beforeunload`. Only on candidate tabs, only http(s) |
| `background/bridge.ts` | `chrome.runtime.connectNative('com.decay.host')`; heartbeat every 20 s; reconnect with backoff; chunks messages > 900 KB |
| `background/commands.ts` | Executes Close/Restore/Focus; acks with result |
| `standalone/` (Phase 0) | Full local engine in TS: Dexie DB, aging, shadow mode, search popup (`⌥⇧D`) |
| Mode switch | When the native host is reachable, the extension migrates its IndexedDB archive to the app (`ImportArchive` message) and becomes a thin adapter |

**Permissions:** `tabs`, `tabGroups`, `storage`, `alarms`, `idle`, `scripting`, `nativeMessaging`, host permissions `<all_urls>` (optional, requested at runtime for form probing). **No** `incognito` access ("spanning" not enabled), so private windows are never visible.

**MV3 service-worker lifetime:** a native messaging port keeps the worker alive; `chrome.alarms` (1 min) acts as a watchdog.

### 4.8 `decay-host` (native messaging host)

- Launched by the browser with the caller origin as an argument; rejects any origin other than the allowed extension IDs.
- Reads 4-byte little-endian length-prefixed JSON from stdin and writes to stdout (max 1 MB out).
- Connects to the main app through IPC at `~/Library/Application Support/Decay/decay.sock` or `\\.\pipe\decay-<user-SID>`.
- If the app isn't running: tries to launch it (`open -b com.decay.app` / registered path), waits 3 s, otherwise replies `app_unavailable` and the extension stays in degraded/standalone mode.

### 4.9 UI windows (Tauri)

| Window | Type | Notes |
| --- | --- | --- |
| `panel` | Frameless, transparent, always-on-top, non-activating (`NSPanel` on macOS), centered on the active screen | Pre-created hidden at launch so it appears instantly; hides on blur/Esc |
| `library` | Standard window, unified titlebar (macOS overlay titlebar / Windows Mica) | Main archive browser |
| `settings` | Standard window, fixed width 720 | Sidebar tabs |
| `onboarding` | Standard window, 640×560 | First run only |
| `tray-popover` | Small frameless window anchored to the tray icon | Quick status, hold, search |

The app runs as a **menu bar agent** (`LSUIElement = true`, no Dock icon) on macOS; while the Library or Settings window is open the activation policy switches to `Regular` so a Dock icon appears and `⌘Tab` works. Windows: tray only, and the taskbar button shows while a window is open.

### 4.10 Event bus

An internal `tokio::broadcast` channel of `CoreEvent` (`ItemChanged`, `SweepStarted/Completed/Undone`, `AdapterHealthChanged`, `ClockStateChanged`, `SettingsChanged`). The Tauri layer forwards a subset to the UI as typed events (see API Design §3).

## 5. Key sequences

### 5.1 Hotkey → search → restore
1. The global shortcut fires → Rust shows the pre-warmed `panel` and emits `panel:open`.
2. UI calls `search({query, filters, limit:50})` on each keystroke (debounced 30 ms).
3. `decay-store` runs FTS5 `MATCH` with prefix tokens + bm25 + recency boost; returns grouped results.
4. Enter → `restore_items({ids:[id]})` → engine routes to the BrowserAdapter → extension `tabs.create` → ack → `ItemChanged` → panel hides.

### 5.2 First run
1. Onboarding → create the DB key in Keychain/Credential Manager → initialize the encrypted DB.
2. Install native host manifests for every detected Chromium browser.
3. Detect the extension (wait for the host connection) → show "Connected ✓".
4. Request Accessibility on macOS (or choose tabs-only) → poll `AXIsProcessTrusted` every 1 s.
5. `settings.mode = shadow`, `shadow_started_at = now`.
6. After 7 active days → notification: "Review what Decay would have archived" → Shadow Review screen → "Turn on automatic mode".

## 6. Security and privacy design

| Threat | Control |
| --- | --- |
| Another local process reads browsing history | SQLCipher (AES-256) DB; key in Keychain/Credential Manager scoped to the app's signing identity |
| A rogue extension talks to the host | `allowed_origins` in the host manifest + origin check in `decay-host` |
| Another user on the machine connects to IPC | Socket file mode `0600` in the user's Application Support folder; named pipe ACL limited to the current user SID |
| XSS in the WebView via a malicious page title | React escaping; strict CSP (`default-src 'self'`); titles never rendered as HTML; Tauri capabilities allow only the needed commands per window |
| Malicious update | Tauri updater Ed25519 signature + OS code signing |
| Log leakage | URLs and titles redacted in logs unless "verbose diagnostics" is enabled |

## 7. Error handling and resilience

- All adapter commands are idempotent (keyed by external id + sweep id).
- DB: WAL, `synchronous=NORMAL` (FULL during executor commits), integrity check at startup, auto-restore from the latest backup on corruption.
- Panics in background tasks are caught, logged, and the task is restarted with backoff; sweeps are disabled until the next healthy tick.
- Permission revoked mid-run → `PlatformAdapter` → `Down` → no window sweeps; banner shown.

## 8. Performance plan

- The panel window is pre-created; the search runs in Rust and returns ≤ 50 rows.
- FTS5 with `prefix='2 3'` index; `items_fts` is an external-content table (no duplicated text).
- Live item observations are coalesced (100 ms) before DB writes.
- The UI uses virtualized lists; images (favicons) are cached as small blobs (≤ 4 KB).

## 9. Testing strategy

| Level | What |
| --- | --- |
| Unit (Rust) | Policy, lifecycle (proptest), aging clock, FTS ranking, chunking |
| Integration (Rust) | Engine with a fake `Adapter`; executor crash-recovery (kill between commit and close) |
| Extension | Vitest for logic; Playwright with a real Chromium + mock host |
| Platform | Manual + scripted matrix per macOS/Windows version; AX permission flows |
| UI | Vitest + Testing Library; Playwright visual diffs against Stitch references; axe a11y |
| Safety suite | "Never lose work" scenarios: audible tab, form input, pinned, incognito, unsaved doc, meeting, adapter disconnect — all must result in **no action** |

## 10. Spikes required before committing (from the spec's riskiest unknowns)

1. **macOS per-window minimize UX.** Is minimizing to the Dock acceptable as "Hidden"?
2. **Unsaved-state detection** per allowlisted app (AX `AXDocument` + window "edited" indicator), and the equivalent on Windows.
3. **`tauri-nspanel` non-activating panel** behaviour across Spaces and full-screen apps.
4. **Native messaging under Arc and Brave** (manifest paths, extension IDs).
5. **Windows meeting detection** reliability via the CapabilityAccessManager registry.
