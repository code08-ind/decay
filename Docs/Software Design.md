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
| Active | `tick` | `active_age ≥ T_stale` ∧ ¬protected ∧ ¬held | Stale |
| Stale | `tick` | ¬held ∧ (protected ∨ `age < T_stale`) | Active |
| Stale | `tick` | kind=Window ∧ `age ≥ T_hidden` ∧ hideable ∧ ¬protected ∧ ¬held | Hidden |
| Stale | `begin_archive` | kind=Tab ∧ `may_close` | Archiving |
| Hidden | `begin_archive` | kind=Window ∧ `may_close` (app ∈ close allowlist ∧ state=Saved ∧ Pro) | Archiving |
| Archiving | `commit` | `may_close` re-checked right before the close, and the close succeeded | Archived |
| Archiving | `abort` | — | Stale (tab) / Hidden (window) |
| any live | `used` | — | Active (age=0) |
| Archived | `restore` | — | Restored (the reopened item is a new Active record linked via `restored_from`) |
| Archived / Restored | `retention` | `archived_at + retention < now` | Purged |

  `may_close` = `age ≥ T_archive` ∧ ¬protected ∧ ¬held ∧ (tab ∨ closable). Thresholds are the item's effective ones (a custom rule's, if one applies). A Hidden window stays Hidden on `tick` (only `used` brings it back), and a window that can't be minimised is never Hidden, so it is never closed.
  Property tests (`proptest`) assert that **no path reaches Archived while protected** and that Hidden is never skipped for windows.

- **`Policy::evaluate(item, ctx) -> Decision { action, reason }`**. Ordering: global holds → hard protections → user rules (any matching `protect` wins; otherwise the most specific custom threshold: URL glob > title > domain > app > global) → thresholds. Reasons are enums, e.g. `Reason::Unused { active_hours: 61 }`, `Reason::ProtectedByRule(rule_id)`, `Reason::UnknownSaveState`.
- **`AgingClock`:** pure accounting. Given `ActivitySample { elapsed_ms, awake, unlocked, idle_secs, audio_playing }` it returns `credit_secs`: the elapsed time (rounded to seconds) when awake ∧ unlocked ∧ (idle < cutoff ∨ audio), and 0 for the first tick or a gap longer than 3 ticks (no back-credit), as in the extension's `clock.ts`.
- **As built (P1.1, `crates/decay-core`):**
  - Lifecycle adds the write-before-close step: Stale/Hidden `BeginArchive` → **Archiving** → `Commit` → Archived, or `Abort` → back to Stale (tab) / Hidden (window). `Commit` is refused unless `may_close` passes on the pre-close re-check (the property tests feed raw events, so this is checked on the state machine itself). Restoring moves the old record to **Restored**; the reopened tab is a new Active record. Property tests: never archived while protected, windows always Hidden first, archived/restored/purged records never go live again.
  - Policy for tabs is identical to the extension's `lib/policy/policy.ts`. This is enforced by `tests/policy_parity.rs`: 6,000 seeded policy cases, 18,000 title-pattern cases and a case-folding sweep of every cased BMP character, all generated from the real TypeScript code by `apps/extension/scripts/policy-parity.ts`, and CI fails if the fixtures are stale (generated with the Node version in `.nvmrc`).
  - Policy for windows: there is no `unrestorable` check. A window gets `Hide` at the hidden threshold and `Archive` (close) only when the context is Pro, a `closeAllowed` rule matches and the document is `saved`. Unsaved or unknown documents are hidden, never closed.
  - Types serialise to the API's camelCase JSON; `SqlEnum::as_sql()` gives the snake_case `CHECK` values. Title patterns run as JS (non-`u`) regular expressions on `regress`, a backtracking ECMAScript engine. `js_regex.rs` rewrites each pattern so it matches over JS-canonicalised text: the `Canonicalize` table is generated from V8 into `js_case.rs`, and classes become sets of canonical units. Case-insensitive matching is therefore exact, including ranges and characters other engines fold differently (ſ, K, ı, Greek). Both engines refuse the same patterns: too long, inline flags, invalid, back-references, lookarounds, repeated groups, and a `backtrackingCost` over 256² (paths per start position; every choice point counts at least 4). The cost limit bounds ReDoS for bounded repeats too (`a{1,30}` × 5).
  - Rule lists are read leniently (`rules_from_json`): a rule with an unknown scope or action is skipped and malformed thresholds read as none, like the extension, so one bad rule never drops the rest. Rule domains are lowercased like JS and converted to punycode, so `bücher.de` matches.

### 4.2 Aging clock (runtime)

- Ticks every **30 s** (tokio interval).
- `credit = tick` if awake ∧ unlocked ∧ (idle < 300 s ∨ any audio playing); otherwise `0`.
- For every live item **not in use** at the tick, `active_age += credit`.
- "In use" = focused window/tab, a visible tab (active tab of a visible, non-minimized window), audible, or typed into within the tick.
- Sleep/wake and lock events immediately freeze the clock (no back-credit).
- Ages are persisted in batches every 5 min, plus on shutdown (crash loses ≤ 5 min of age — this is safe, because items just look newer). As built (P1.5): every tick, see below.

- **As built (P1.5, `crates/decay-engine`):** a std thread and a 1-s poll instead of a tokio interval (the store is synchronous and the engine is one state machine). Until the platform layer (P1.6) reports keyboard/mouse idle, sleep and lock, the idle signal is **browser activity**: a deliberate tab switch, typed signal or window focus within the idle cutoff. No browser activity = idle = no credit (items only look younger, the safe direction). "In use" = the active tab of a window that isn't minimised, or audible (the extension's `inUseTabIds`); the policy additionally protects every active tab. Ages are written every tick (one transaction per browser), and on disconnect. A browser that isn't connected, or hasn't sent anything for 70 s, is frozen: no aging, no sweeps.

- **As built (P1.6, `crates/decay-platform` + `decay-engine::platform`):** the runtime samples the OS about once a second (`Input::Platform`): idle seconds and seconds since the last key press (`CGEventSourceSecondsSinceLastEventType`), locked or another user on the console (`CGSessionCopyCurrentDictionary`), main display asleep (`CGDisplayIsAsleep`), audio (default output device `kAudioDevicePropertyDeviceIsRunningSomewhere`), the active app (`GetFrontProcess` + `GetProcessPID`, name via `proc_name`; deprecated but shipped, live without a run loop or permission — the window list was wrong for windowless active apps and Stage Manager; no titles, so no Screen Recording) and Accessibility (`AXIsProcessTrusted`; `AXIsProcessTrustedWithOptions` prompts). Polling needs no run loop, so it works on the engine thread. With a fresh sample (≤ 5 s): `awake` = display not asleep, `unlocked` = not locked, idle from the OS, audio = an audible tab or the output device in use. Sleep needs no signal: a gap of more than 3 ticks gets no credit, and the tick only reads the latest sample, so anything "away" since the previous tick (a sample showing lock or display sleep, or a gap of more than 5 s between samples) voids that tick's credit. Wake detection ("morning cleanup") measures the gap on `CLOCK_MONOTONIC` (`clock_gettime_nsec_np`: runs through sleep, never jumps), so changing the wall clock isn't a wake; the wall clock is only the fallback. Audio from the OS counts only while idle < 2 h (an app holding the output device open while silent, e.g. a call app, can't age tabs overnight); audible browser tabs aren't capped. Without a fresh sample (Windows until its implementation, or a failure) the engine keeps the browser-activity fallback. Accessibility is health only: it gates app-window actions (P2.x), never tab sweeps, which go through the extension. Sampling is polling (no `NSWorkspace` notifications or `AXObserver`: those need a run loop; window focus tracking comes with P2.1).

### 4.3 Sweep scheduler

Runs a candidate sweep when **all** are true:
- At least 15 min since the last sweep, and candidates exist.
- No typing for ≥ 60 s; no app switch for ≥ 30 s ("quiet moment"), **or** the user just unlocked or woke the machine (morning cleanup).
- No hold, meeting, screen share or permission problem.
- Not in shadow mode → real sweep; in shadow mode → simulated sweep (`sweeps.is_shadow = 1`).

Max **50 items per sweep** (configurable) to keep Undo meaningful.

**As built (P1.5):** checked after every tick: ≥ 15 min since the last sweep (shadow sweeps count), quiet = no deliberate tab switch / window focus for 30 s, not paused or on hold, at least one live browser. "Sweep now" (`Engine::sweep_now`) skips the interval and the quiet check; paused / on hold / busy / no browser → refused with the reason. **P1.6:** quiet also needs no key press for 60 s and no frontmost-app change for 30 s; no scheduled sweep while locked or with the displays asleep. Within 2 min after a wake (a gap of > 90 s between OS samples), an unlock or the displays waking, the first sweep doesn't wait for a quiet moment ("morning cleanup", trigger `wake`; the 15-min interval still applies). The engine reports `AccessibilityChanged { trusted }` on the first sample and on every change (F33 health). Meetings and screen sharing aren't detected yet.

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

**As built (P1.5, `crates/decay-engine`):**
- **Probe step:** stale web tabs that would be archived if nothing were typed in them and have no fresh verdict for their current URL are probed (≤ 40 per browser, oldest first; ack timeout 5 s + 3.5 s per tab). Verdicts are cached per tab and URL (clean 30 min, dirty / unknown 10 min) and dropped when the tab navigates, is used deliberately or closes. No answer → unknown → 2× the archive threshold.
- **Plan and write:** the policy decides with the verdicts; `BeginArchive` must pass the lifecycle's `may_close`. Oldest first, at most `max_items_per_sweep`. In one durable transaction (`synchronous = FULL`): the sweep (`running`), each item `archiving` with its archive reason and `last_sweep_id`, and its `sweep_items` row (`pending`). Shadow mode instead writes a `done` shadow sweep and a `shadow_would_archive` event per item; nothing changes state.
- **Close:** one `tabs.close {sweepId, tabIds, urls}` per browser (ack timeout 10 s + 4 s per tab). The ack's `result` settles each tab, whatever `ok` says (the extension answers `ok:false` with a result when any tab is skipped): closed → `archived` (event `archived`, sweep item `ok`); skipped → back to `stale` (event `close_failed`, sweep item `skipped`), with its age reset when it was used during the close (`active`, or a `tab.used` meanwhile; `inUse` alone only defers it, because Chrome marks every tab it restores at start-up as just accessed) and a dirty verdict cached for `formInput`; neither (an error without a result, a lost link) → left `archiving` and a `resync` is requested. A close past its deadline is **late, not lost**: the sweep executor moves on, but its tabs count as in flight for 5 more minutes (no snapshot settles them, since the extension may still be closing them) and a late ack is still applied. Web tabs planned at the normal threshold on a cached *clean* verdict are sent as `requireChecked`: if the page can't be checked right before closing, the extension skips them (`unchecked`) instead of closing them unchecked.
- **Settling against a snapshot (also crash recovery):** an `archiving` item whose close isn't waiting for an ack is settled by the next snapshot of its browser: still open (same tab id and URL, or the same URL) → `stale`; gone → `archived`. A sweep becomes `done` once none of its items is `pending`. A tab the user closes while it's being closed (`tab.removed` with `byUser`) is deleted, not archived. A `tab.removed` with `byUser: false` means Decay closed it (a late close, or one that ran on across a reconnect) — that item is **archived**, never forgotten, with its sweep and archive reason (an aborted item keeps both), and the sweep's counts are corrected. The extension stops a close loop as soon as its connection is replaced, and keeps "closed by Decay" across reconnects. Exclusion rules are re-applied every tick and at planning time: a tab matching a new URL-glob or title exclusion is forgotten, never closed. Settings are re-read right before planning, so a pause during the probe stops the sweep.
- **First snapshot:** if it doesn't arrive (the browser had no window open, so the extension couldn't read its tab groups), it's asked for again every minute.
- **Events for the UI:** `SweepCompleted { sweepId, shadow, archived, failed }` (P1.7 shows the Undo toast), `NothingToSweep` (Sweep now found nothing), `SweepUndone`, `AdapterConnected` / `AdapterDisconnected`, `Problem`.
- **Tracking (Software Design §4.6):** items are keyed `<profileId>:<tabId>` (tab ids are only unique per profile). A snapshot is matched like the Phase 0 tracker: same tab id and URL, then same URL (most recently used first), so ages survive a browser restart. A tab that disappears is *parked* (`<profileId>:~<itemId>`, freeing its id) and reclaimed by a new tab with its URL within a minute ("reopen closed tab", session restore), else deleted after a minute and a fresh snapshot. Excluded tabs (F9) are never stored; with a title-exclusion rule, a loading tab waits for its title. Each browser window is a `sources` row (`<appId>|<profile>|<session>:<windowId>`) whose `external_id` is the `tabs.restore` window key.

**Phase 0 extension notes (P0.5):**
- The sweep is written as `running` directly (planning and the archive records commit in one transaction with the tick).
- Unknown state never closes and never archives: if the tab can't be read before closing → skipped; if a close times out (or fails) and the tab can't be confirmed gone → the item stays `archiving` and the sweep stays `running` until the next tick's recovery settles it.
- A tab already gone before the executor closed it was closed by the user → its record is deleted (DB Design §5), not archived.
- Deliberate tab activations during a sweep are remembered in memory (handlers ignore `archiving` items); such tabs are skipped and their age is reset.
- Sweeps from an earlier browser session are only settled 2 min after the new session starts, so session restore can bring tabs back first ("not restored yet" ≠ "gone").
- Real sweeps only at a quiet moment: no deliberate tab switch / window focus for 30 s ("Sweep now" skips this check and the 15-min interval).
- Form probe (P0.7): each tick, open web tabs that would be archived if nothing were typed in them are probed before the tick's tab snapshot (oldest first, ≤ 40 per tick, ≤ 5 s per tick, ≤ 6 probes in flight across ticks and sweeps, 3 s timeout each); verdicts are cached per tab in `storage.session` (30 min for clean; 10 min for dirty / unknown) and dropped when the tab navigates, is used, closes, or site access changes. `dirty` → protected (`formInput`), `clean` → normal threshold, `unknown` → **2× the archive threshold** (PRD §7.3). Every planned close is probed again right before `tabs.remove` and re-decided by the policy with the fresh verdict (typed input → skipped `formInput`; can no longer be checked and younger than 2× → skipped `changed`).

### 4.4b Phase 0 metrics (P0.8, F34)

- **Collection (on-device):** each tick with active credit samples open tabs (every tab in normal windows, incl. `chrome://` and extension pages) and normal windows once per local hour (daily medians; hours with no open tab are skipped). The popup counts one search per opening (query ≥ 2 chars) and times first keystroke → restore of a search result; clearing the query (Backspace or Esc) restarts the timer, a restore attempt stops it, and times over 120 s are dropped. The time travels with the restore request and the worker records it once the restore succeeds (restoring an active tab closes the popup before the reply arrives). Archives, restores within 24 h, undos, shadow flags/regret and auto-mode days were already recorded.
- **Definitions:** regret = restored within 24 h ÷ archived over the same days, capped at 100 % (approximation: restores count on their own day, archives on theirs, so a range edge can shift a few). Week 1 = the 7 calendar days from the first sampled day; the open-tabs comparison appears once that day is ≥ 14 days old. Automatic-mode days = calendar days since the current unbroken run started (active days with automatic mode off break it; days without browsing don't; a same-day off/on doesn't) — Insights and the export's `setup.automaticForDays` use this one definition.
- **Insights page (`insights.html`, Designs/14 adapted):** range 7/14/30 days; tiles (median open tabs vs week 1, regret rate — shadow regret before any real archive, items archived, most tidied site); open-tabs bar chart with first-week reference line, per-day hover/focus readout and table view; "Beta goals" card with the PRD §9 targets (regret < 5 %, open tabs ≥ 50 % fewer, search → restore < 5 s, automatic mode 14 days) as On target / Not yet / No data yet; archived-most and restored-most site lists. Linked from the popup header; Shadow review, Sweeps and Insights share a header with links between them.
- **Share with beta team:** builds `decay-beta-metrics` v1 JSON (random install id, version, browser, mode/preset/thresholds, probe access level, rule counts per action, summary, daily rows) — counts and medians only, no URLs, titles, domains or rule patterns; shown in full in a preview dialog (which says what the file holds, including that the install id links one tester's exports), saved as a file the user sends themselves. Export CSV saves the same daily rows for all days, whatever range is selected.

### 4.5 Undo and restore

- **Undo sweep:** for every item in the sweep → restore command, `sweeps.undone_at = now`, and an `undo` event per item. Restored items get a **48-hour grace protection** so they aren't re-archived immediately. The undo window is the last 20 sweeps that archived something. If some tabs fail to reopen, the sweep stays `done` and Undo can be retried for the rest; an interrupted undo is marked on the sweep so a retry links already-reopened tabs instead of opening duplicates.
- **As built (P1.5):** Undo reopens what's still archived of one of the last 20 finished real sweeps that archived something (a sweep still closing tabs is refused with "try again in a moment"), each tab in its own browser when that browser is connected (else in the one that asked), into its original window (same browser session) at its index and in its group. Each reopened record becomes `restored` (event `undo`); the new tab's item links to it (`restored_from_id`) with 48 h of grace and age 0 — whether the tab is reported before or after the ack. An item whose restore is on its way isn't sent again (a double Enter, two popups): it counts as `skipped`. A restore that isn't answered counts as failed and the items stay archived. The sweep becomes `undone` once nothing of it is left archived; a partial undo can be retried for the rest. Not yet: the "interrupted undo" marker (a crash between sending `tabs.restore` and its ack can reopen a tab twice on retry), and the regret metric / "Always keep" offer (P1.11 with the app UI).
- **Phase 0 (extension) search & restore (P0.6):** the popup builds a MiniSearch index over archived items when it opens (title ×3, domain ×2, tab group ×2, URL words; prefix + fuzzy for words > 3 letters; `site:`, `app:chrome|edge|brave`, `before:` / `after:` with `YYYY-MM-DD`, `today`, `yesterday`, `3d`, `2w`), score × (1 + recency boost). Results are grouped by day + batch (same sweep, same original window). Enter reopens one item as the active tab of the current window; ⌘/Ctrl+Enter restores its batch into the original window (same session) or a new window. **Batch restore is free in the Phase 0 extension** (licensing arrives in Phase 4; PRD §7.4 makes it Pro in the desktop app). A restore within 24 h of archiving offers "Always keep <site>" as a notification (creates a `quick_action` protect rule).
- **"Sweep now" (`run_sweep_now`):** automatic mode → real sweep; shadow mode → dry run (a simulated sweep right away, nothing closed); paused / hold → refused with the reason.
- **Notifications:** the Undo toast after a real sweep is always shown (F7). The `notifications.sweep` setting (off by default) is reserved for the optional "sweep done" summary (F25), not the Undo toast.
- **Phase 0 (extension):** the Undo "toast" is a system notification with Undo / View buttons, cleared after 30 s by an alarm; the Sweeps page (`sweeps.html`) lists sweeps and offers Undo for each undoable one. Tabs reopen in the original window only within the same browser session (window ids aren't stable across restarts).
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
| Phase 0 probe (`lib/probe`, P0.7) | With optional site access — all http(s) pages, or only the sites picked under Chrome's "On specific sites" — two tiny scripts are registered for the granted origins at `document_start` in all frames: `typing.js` (isolated world) remembers *which* elements got trusted `input`/`change` events (never the text), answers a liveness ping (listeners erased by `document.open()` → untracked); `unload-watch.js` (main world) counts `window.addEventListener('beforeunload')` registrations (fired `{once}` listeners forgotten) and uploads in progress (XHR `send` / `fetch`, incl. `Request` objects, with a file, FormData with a file, a stream, or ≥ 256 KB string/binary body; XHR counted from live `readyState`). Wrappers are Proxies of the native methods (on `Window.prototype` for add/removeEventListener, forwarding at call time so zone.js-style later patches still apply; on `XMLHttpRequest.prototype` and `fetch`). The probe (`inspectForms` isolated + `inspectUnload` main, all frames): typed text still in a text field / textarea / file picker / rich-text editor (also inside open/closed shadow roots; search boxes, checkboxes, radios, selects don't count) → dirty; `beforeunload` handler + sticky user activation → dirty; upload running → dirty. Tracked frames after reload / Back-Forward / discarded-tab reload: any untyped field whose text differs from its default → unknown (Chrome refills fields after `load` without events, indistinguishable from a script setting a value; such pages wait 2× until navigated). Untracked frames: value ≠ default → dirty; other non-empty field / editor, or user activation without the watcher → unknown. Discarded/unloaded tabs, no access for the page's site, blocked injection, timeout, malformed top-frame answer → unknown. With partial access, an embedded page (iframe) from a site Decay may not check → unknown (the browser skips such frames silently). Registration is diffed (no gap on worker restarts). Only booleans, counts and embedded-frame origins leave the page. Known limits: uploads from Web/Service Workers aren't seen; the fetch wrapper marks an upload's promise as handled; native errors from wrapped methods show the script URL in their stack; the page can detect the wrappers |
| `background/bridge.ts` | `chrome.runtime.connectNative('com.decay.host')`; heartbeat every 20 s; reconnect with backoff; chunks messages > 900 KB |
| `background/commands.ts` | Executes Close/Restore/Focus; acks with result |
| `standalone/` (Phase 0) | Full local engine in TS: Dexie DB, aging, shadow mode, search popup (`⌥⇧D`) |
| Mode switch | When the native host is reachable, the extension migrates its IndexedDB archive to the app (`ImportArchive` message) and becomes a thin adapter |

**Permissions:** `tabs`, `tabGroups`, `storage`, `alarms`, `idle`, `scripting`, `nativeMessaging`, optional host permissions `http://*/*` + `https://*/*` (requested at runtime from the popup / Shadow review banner for form probing; without them web pages archive at 2×). **No** `incognito` access ("spanning" not enabled), so private windows are never visible.

**MV3 service-worker lifetime:** a native messaging port keeps the worker alive; `chrome.alarms` (1 min) acts as a watchdog.

**As built (P1.4, `apps/extension/lib/bridge/`):**
- **Permission:** `nativeMessaging` is an **optional** permission. The popup's "Connect" (a one-line "Decay app" row) requests it, so users without the desktop app never see the prompt, and a store update can't disable the extension for a new permission. Turning the link on is remembered (`decay.appLinkEnabled`); "Disconnect" turns it off.
- **`connection.ts` (`Bridge`):**
  - `hello` → `welcome` handshake: 10 s, then `unavailable`; `VERSION_MISMATCH` → `outdated`.
  - A heartbeat at the app's `heartbeatSec` while connected.
  - Chunking both ways, cut at code point boundaries (`chunk.ts`, the same format and limits as `decay-bridge`), and request/reply matching by `replyTo`.
  - `hello.launch` is true only at browser start-up (`runtime.onStartup`) and on the popup's Connect. Worker restarts and the 1-minute watchdog reconnect without it, so an app the user quit stays closed.
- **Who closes tabs** (`ownership.ts`, background): never both the app and the local tracker.
  - **While the app is connected:** the tracker keeps following tab events and ticks, so its ages are current if it has to take over. Its policy reads `paused`, though, so it never plans a sweep, and every close it makes goes through a guard that refuses while the app owns the tabs.
  - **While the link is on:** a fresh worker (after a restart, or at browser start-up) keeps the local engine paused until a connection attempt has actually failed.
  - **When the app goes away:** the tracker's takeover tick shows the Undo toast like any sweep.
  - **Sweep now:** the popup's button says the app runs sweeps (or that it's still connecting).
- **Adapter mode** (`adapter.ts`, while connected): the app owns aging, policy and sweeps. The extension reports tabs and carries out commands; the protocol details are in API Design §4.2 "As built (P1.4)".
  - **Reporting to the app:** a `snapshot` after connecting, on `resync` and on tab-group changes; `tab.created / updated / removed / moved`, `tab.used` (activated, audible) and `window.focus`.
  - **Not reported:** incognito tabs, and tabs on the app's excluded domains (`config`). A tab that navigates to an excluded site is reported as removed.
  - **Commands:** all are acked.
    - `probe` replies with one `probe.result` per tab.
    - `tabs.close` re-checks active / pinned / audible / unsaved typing right before closing. Any skip makes the ack `UNSAFE`, with per-tab reasons, and a tab already gone counts as closed. Its `tab.removed` says `byUser: false`.
    - `tabs.restore` reuses the original window if it's open, else one new window per `windowKey`, regroups tabs, and refuses non-web URLs (`javascript:` etc.).
    - `tabs.focus`, `resync` and `config` are handled too.
- **Back to standalone:** when the app goes away, the tracker takes over at once with an announced tick; its view of the tabs is already current.
- **Archive sync** (`migrate.ts`): the extension keeps working while the app isn't running, so this isn't a one-off migration.
  - **When:** on every connection, and every minute while connected.
  - **What:** items archived or restored since the last batch the app confirmed (by `stateChangedAt`), in acked batches of ≤ 2,000, so archives of any size fit. Rules and settings go too when they've changed; the app keeps settings the user changed there.
  - **Progress:** it's recorded per batch in `decay.archiveSync`, and failures show in the link status.
  - **Favicons:** `data:` favicons are dropped.
  - **Local copy:** it's kept as the standalone fallback.
  - **Not yet:** the popup still searches and restores from the local archive in adapter mode. Its restores reach the app through the next sync and the tab events. Routing search to the app, and showing the app's mode in the popup, come with the app UI.
- **Connection races:** a second `connect` while one is running is absorbed. A `launch` request during a quiet attempt is remembered: if that attempt finds no app, it retries once, allowed to start it. The profile id is created once. Granting the permission turns the link on from the background (`permissions.onAdded`), so the popup may close while Chrome asks. An `outdated` link stops retrying until the extension version changes.
- **Cross-language check:** `scripts/bridge-fixtures.ts` builds real chunks and a real archive with this code. `decay-bridge` and `decay-store` tests read them, and CI fails if they're stale.
- **Waiting on the engine (P1.5):** the app side of the commands (who sends `probe` / `tabs.close`, and how acks drive the lifecycle).

### 4.8 `decay-host` (native messaging host)

- Launched by the browser with the caller origin as an argument; rejects any origin other than the allowed extension IDs.
- Reads 4-byte little-endian length-prefixed JSON from stdin and writes to stdout (max 1 MB out).
- Connects to the main app through IPC at `~/Library/Application Support/Decay/run/decay.sock` or the Windows pipe the app published (API Design §5).
- If the app isn't running: tries to launch it (`open -b com.decay.app` / registered path), waits 3 s, otherwise replies `app_unavailable` and the extension stays in degraded/standalone mode.
- **As built (P1.3, `crates/decay-host`):**
  - **Allowed callers:** the built-in store IDs (to be added when the extension is published), plus `allowedOrigins` in `<support>/host/host-config.json`, which the app writes when it installs the host. An origin must be `chrome-extension://` + 32 letters a–p + `/` and be in that list. Anything else exits with status 1 without reading or writing a byte.
  - **First message:** it must be `hello`, which gives `conn.browser` and `conn.profileId`; otherwise the host sends `error BAD_MESSAGE` and exits.
  - **Starting the app:** only when `hello.launch` is set (browser start-up or a user action), and not after the user quit Decay (the app writes `run/quit-by-user` on quit and removes it at start). The host then starts the app once (`open -g -b com.decay.app --args --background` on macOS, or `appPath --background` on Windows, detached from the browser's job object) and keeps retrying for 3 s. Otherwise the extension gets `APP_UNAVAILABLE` right away.
  - **Extension → app:** invalid JSON or an oversized message gets `error BAD_MESSAGE` and isn't forwarded. Messages are forwarded as received, not rebuilt.
  - **App → extension:** messages over 1 MB are chunked. An invalid frame from the app is skipped; it doesn't end the session.
  - **When the app quits:** everything it already sent is delivered first (up to 3 s), then the extension gets `error APP_UNAVAILABLE` and the host exits.
  - **Exiting:** the host exits holding the stdout lock, so no frame is cut off. When the browser closes the port, it exits too.
  - **Manifests:** `install` / `uninstall` write `com.decay.host.json` for every detected browser:
    - macOS: `~/Library/Application Support/<browser>/NativeMessagingHosts/`, for Chrome (and Beta / Canary), Chromium, Edge, Brave and Arc (`Arc/User Data`, unverified: spike 4).
    - Windows: `HKCU\Software\<vendor>\NativeMessagingHosts\com.decay.host`, pointing at `<support>\host\com.decay.host.json`.

    A browser counts as detected when its folder or registry key exists. The manifest's `allowed_origins` is the same list the host checks.
  - **When to install:** the app runs `install` at every start; it's cheap and idempotent, so it picks up browsers installed later and an app that moved.
  - **What install refuses:**
    - invalid origins, or an empty origin list;
    - a host path inside a disk image or an App Translocation copy (the app must be moved to Applications first).
  - **Failures:** one browser failing is reported and doesn't stop the others; uninstall works the same way.
  - **Unverified (spike 4):** Brave on Windows may read Chrome's registry key instead of its own.

### 4.9 UI windows (Tauri)

| Window | Type | Notes |
| --- | --- | --- |
| `panel` | Frameless, transparent, always-on-top, non-activating (`NSPanel` on macOS), centered on the active screen | Pre-created hidden at launch so it appears instantly; hides on blur/Esc |
| `library` | Standard window, unified titlebar (macOS overlay titlebar / Windows Mica) | Main archive browser |
| `settings` | Standard window, 920×640 (min 820×520) | Sidebar tabs (P1.12): General, Aging, Protection (built-in protections + rules with a live test), Browsers, Privacy & data (retention, never-recorded sites), Shortcuts (shortcut recorder), About. Own capability (no Library commands) |
| `onboarding` | Standard window, 840×600, fixed size | First run only (`onboarded_at` unset): a plain launch, the tray's Library, the Dock, a second launch and Library links open it instead of the Library (a Library request made before start-up has read the setting waits for it; an item route asked for before onboarding is dropped). Browsers count as installed when their user data folder has a `Local State` file (macOS) / their registry key exists (Windows); Chrome Beta / Canary count as Chrome |
| `tray-popover` | Small frameless window anchored to the tray icon | Quick status, hold, search |

The app runs as a **menu bar agent** (`LSUIElement = true`, no Dock icon) on macOS; while the Library or Settings window is open the activation policy switches to `Regular` so a Dock icon appears and `⌘Tab` works. Windows: tray only, and the taskbar button shows while a window is open.

- **As built (P1.7, `apps/desktop`):** one Vite/React bundle for every window (the window label picks the view; real screens come in P1.8–P1.11). `src-tauri` (crate `decay-app`, bin `decay`): `core` starts the engine runtime and calls into the engine thread (`ask` waits ≤ 5 s off the main thread; tray actions don't wait); `menu` is the pure tray model (badge = stale + hidden tabs of connected browsers, ≤ "99+"; items disabled when they'd do nothing); `deeplink` parses and validates `decay://` links (ids `[A-Za-z0-9-]{1,64}`, known settings tabs, query ≤ 500 chars) and maps each to a window + route — restore / undo / license open a confirming screen, never act; `keys` keeps the DB key in the Keychain (service `com.decay.app`, account `database-key`), creates it only when no database exists, and stops with an error instead of replacing a missing key. Debug builds use `<support>/dev-app/` with a key file (`DECAY_USE_KEYCHAIN=1` to test the Keychain) and never register a login item. The panel is a hidden, undecorated, always-on-top window for now (the non-activating `NSPanel` is P1.9). Engine events become UI events (`status:changed`, `sweep:completed`, `sweep:undone`, `sweep:nothing`, `permission:changed`, `adapter:health`). Commands get per-window capabilities (`build.rs` app manifest). Release packaging (P1.13): `decay-host` is built and signed by `scripts/sidecar.mjs` (`beforeBundleCommand`) and bundled at `Contents/MacOS/decay-host` (`bundle.macOS.files`); updates via tauri-plugin-updater (`update.rs`, Docs/Release.md). Not yet: tauri-specta (TS types are hand-written until the UI phases), a Windows tray badge (the count is in the tooltip).

- **As built (P1.9, search panel):** on macOS the panel is an `NSPanel` (`tauri-nspanel` 2.1, `tauri_panel!` class `SearchPanel`): `NonactivatingPanel` style (typing in it doesn't deactivate the browser), level `ModalPanel`, collection behaviour `canJoinAllSpaces | fullScreenAuxiliary | transient`, transparent (`macOSPrivateApi`, fine for direct distribution), hidden on `windowDidResignKey`. It's 712×512 logical incl. a 16-px shadow margin, centred horizontally and ⅓ down the screen under the pointer. Windows uses an undecorated, transparent, always-on-top window. Search runs in the store (FTS5 prefix + bm25 × recency; `app:` `site:` `before:` `after:`; fuzzy matching is not in yet); the page groups the ranked hits by local day + source window and labels them with `Intl`. Rows use monogram tiles instead of favicons (no network, CSP stays `img-src 'self' data:`). Restores (panel, Undo) only go to the item's own browser **profile** (any session of it): never to another browser or profile (other cookies and accounts). If it isn't connected the restore is refused (`ADAPTER_UNAVAILABLE`, "the browser it came from isn't open") and the items stay archived; a multi-item restore puts each window's tabs back in their original order, and the first tab actually sent to each browser gets focus. Results arrive as `items:restored`; the panel stays open ("Restoring…") until then, closes on success, and says what failed otherwise. ⌘↵ / "Restore all" restore exactly the group on screen (day + window). The panel pages in 50 at a time as it scrolls, runs an Enter pressed before the current results arrive on those results, and leaves keys to an IME while it composes. Highlighting folds case and accents like FTS5; URLs show decoded hosts (IDN) and paths. The panel's event handler replaces Tauri's window delegate (tauri-nspanel doesn't forward), so Tauri window events don't fire for it; its placement is computed in logical points (mixed-DPI monitors). ⌘P adds a domain `protect` rule (`quick_action`).

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
| Another user on the machine connects to IPC | Socket file mode `0600` in a `0700` folder owned by the user, plus a peer-uid check on both ends (macOS); a named pipe ACL limited to the owner (Windows) |
| Another user creates the IPC endpoint first to receive tabs ("squatting") | macOS: the host checks the socket's peer uid. Windows: a random pipe name per app start, published in the user's own profile folder |
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
- FTS5 with `prefix='2 3'` index; `items_fts` is contentless (`contentless_delete=1`), so no text is duplicated (DB Design §6.1).
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
