# Decay — API Design

| | |
| --- | --- |
| **Version** | 1.0 |
| **Date** | 4 Nov 2026 |
| **Related** | [Software Design](./Software%20Design.md) · [DB Design](./DB%20Design.md) |

Decay has **no public cloud API**. "API" here means the five local contracts between its parts, plus two tiny external endpoints.

| # | Interface | Between | Transport |
| --- | --- | --- | --- |
| 1 | **Tauri commands** | React UI → Rust core | Tauri IPC (`invoke`) |
| 2 | **Tauri events** | Rust core → React UI | Tauri event system |
| 3 | **Native messaging protocol** | Extension ⇄ `decay-host` | stdio, 4-byte length-prefixed JSON (≤ 1 MB per message host→ext) |
| 4 | **Host IPC** | `decay-host` ⇄ Decay app | Unix domain socket / named pipe, length-prefixed JSON |
| 5 | **Deep links** | OS / browser / email → app | `decay://` URL scheme |
| 6 | Update manifest | App → GitHub Releases | HTTPS GET (static JSON) |
| 7 | License issuance | Payment provider → serverless function | HTTPS webhook (not called by the app) |

All payloads are JSON with **camelCase** keys. Types are defined once in Rust (`serde` + `specta`) and generated into `packages/protocol` for TypeScript.

---

## 1. Shared types

```ts
type ItemKind = 'tab' | 'window';
type ItemState = 'active' | 'stale' | 'hidden' | 'archiving' | 'archived' | 'restored' | 'purged';
type Mode = 'shadow' | 'automatic' | 'paused';

interface Item {
  id: string; kind: ItemKind; appId: string; appName: string;
  title: string; url?: string; domain?: string; path?: string;
  faviconDataUrl?: string; profile?: string;
  sourceId?: string; sourceTitle?: string; tabIndex?: number;
  group?: { title: string; color: string };
  pinned: boolean; state: ItemState;
  activeAgeHours: number; lastUsedAt: number; archivedAt?: number;
  reason?: Reason; sweepId?: string; restorable: 'full' | 'partial' | 'app_only';
}

type Reason =
  | { type: 'unused'; activeHours: number; threshold: number }
  | { type: 'customRule'; ruleId: string; activeHours: number }
  | { type: 'protected'; by: 'pinned' | 'activeTab' | 'audible' | 'formInput' | 'download' | 'call' | 'unsaved' | 'unknownSaveState' | 'rule' | 'hold' | 'meeting' | 'grace' | 'unrestorable'; ruleId?: string };

// Rule semantics (decided in P0.3):
// - Any matching `protect` rule wins over a more specific `customThreshold` (safety first);
//   otherwise the most specific customThreshold applies: urlGlob > titleRegex > domain > app > global.
// - `exclude` beats `protect` (excluded = never stored). Exclude can't be global/app scope and
//   is rejected if it matches more than one unrelated sample site; creating/enabling it purges
//   stored items + events (`previewExclude` returns the count for a confirmation step).
// - Domains match host + subdomains (IPs exactly); bare public suffixes (com, github.io) rejected.
// - urlGlob: only `*` is a wildcard (`?` literal), case-insensitive, userinfo ignored.
// - titleRegex: case-insensitive, ≤ 200 chars, no quantified groups/back-refs/lookarounds,
//   ≤ 2 unbounded quantifiers; titles are matched on their first 256 chars.
// - Tabs with URLs that can't be reopened exactly (data:, blob:, file:, javascript:, none)
//   are protected as `unrestorable`; chrome://newtab and about:blank are archivable.
interface Rule {
  id: string; scope: 'app' | 'domain' | 'urlGlob' | 'titleRegex' | 'global';
  match: string; action: 'protect' | 'exclude' | 'customThreshold' | 'closeAllowed';
  thresholdHours?: { stale: number; hidden: number; archive: number };
  enabled: boolean; hitCount: number; createdAt: number;
}

interface Sweep {
  id: string; startedAt: number; finishedAt?: number;
  trigger: 'scheduled' | 'wake' | 'manual'; isShadow: boolean;
  status: 'pending' | 'running' | 'done' | 'failed' | 'undone';
  itemCount: number; failedCount: number; undoneAt?: number;
}

interface AppError { code: ErrorCode; message: string; details?: unknown }
type ErrorCode = 'NOT_FOUND' | 'INVALID_INPUT' | 'ADAPTER_UNAVAILABLE' | 'PERMISSION_DENIED'
  | 'PRO_REQUIRED' | 'UNSAFE' | 'DB_ERROR' | 'TIMEOUT' | 'CONFLICT';
```

---

## 2. Tauri commands (UI → core)

Each command returns `Promise<T>` and rejects with `AppError`. Access is limited per window by Tauri **capabilities** (e.g. the `panel` window can call only search/restore/status).

### 2.1 Search and archive

| Command | Input | Output | Notes |
| --- | --- | --- | --- |
| `search` | `{ query: string; filters?: { app?: string; domain?: string; after?: number; before?: number; kind?: ItemKind; sourceId?: string }; limit?: number /*≤200, default 50*/ ; cursor?: string }` | `{ groups: { key: string; label: string; sourceTitle?: string; items: Item[] }[]; nextCursor?: string; tookMs: number }` | Parses inline operators `app:`, `site:`, `before:`, `after:`, `in:`. Groups by day + source |
| `list_items` | `{ state?: ItemState[]; view: 'today' \| 'week' \| 'all' \| 'protected' \| 'live'; sort?: 'recent' \| 'age' \| 'title'; cursor?: string; limit?: number }` | `{ items: Item[]; nextCursor?: string; total: number }` | Library views |
| `get_item` | `{ id }` | `Item & { events: { type: string; at: number; reason?: Reason }[] }` | Detail drawer timeline |
| `get_batch` | `{ itemId }` | `{ sweepId; sourceTitle; items: Item[] }` | Items archived together from the same source |
| `restore_items` | `{ ids: string[]; target?: 'original' \| 'newWindow' }` | `{ restored: string[]; failed: { id: string; error: AppError }[] }` | Batch restore with > 1 source requires Pro → `PRO_REQUIRED` |
| `restore_batch` | `{ itemId }` | same as above | Pro |
| `delete_items` | `{ ids: string[] }` | `{ deleted: number }` | Permanently removes from the archive |
| `protect_item` | `{ id; scope: 'domain' \| 'app' \| 'url' }` | `Rule` | "Always protect" quick action |

### 2.2 Sweeps

| Command | Input | Output |
| --- | --- | --- |
| `list_sweeps` | `{ limit?: number; includeShadow?: boolean }` | `Sweep[]` |
| `get_sweep` | `{ id }` | `Sweep & { items: Item[] }` |
| `run_sweep_now` | `{ dryRun?: boolean }` | `Sweep` — **as built (P1.7):** `{ started: true }`; the result arrives as `sweep:completed` / `sweep:nothing` (a sweep waits for probes and close acks). Refusals: `ADAPTER_UNAVAILABLE` (no browser), `CONFLICT` (already sweeping), `UNSAFE` (paused / on hold) |
| `undo_sweep` | `{ id?: string /* default: last */ }` | `{ restored: number; failed: number }` — **as built (P1.7):** `{ started: true }`; the counts arrive as `sweep:undone`. Refusals: `NOT_FOUND`, `CONFLICT` (still closing), `ADAPTER_UNAVAILABLE` (its browser isn't connected) |
| `get_shadow_report` | `{}` | `{ days: number; wouldArchive: Item[]; wouldHide: Item[]; protectedCount: number; topDomains: { domain: string; count: number }[] }` |

### 2.3 Engine status and control

| Command | Input | Output |
| --- | --- | --- |
| `get_status` | `{}` | `{ mode: Mode; holdUntil?: number; clock: 'running' \| 'paused_idle' \| 'paused_sleep' \| 'paused_meeting' \| 'paused_hold'; staleCount: number; liveTabs: number; liveWindows: number; adapters: AdapterHealth[]; shadowDaysLeft?: number; permissions: Permissions }` |
| `set_mode` | `{ mode: Mode }` | `{ mode }` |
| `hold` | `{ duration: '1h' \| 'untilTomorrow' \| 'indefinite' \| { minutes: number } }` | `{ holdUntil?: number }` |
| `release_hold` | `{}` | `{}` |

**As built (P1.7):** `get_status` returns `{ mode; holdUntil: number | null; staleCount; liveTabs; browsers: { browser; profileId; tabs }[]; sweeping: boolean; undoableSweep: string | null; accessibility: boolean | null }` (the engine's `EngineStatus`). `clock`, `liveWindows`, `adapters: AdapterHealth[]`, `shadowDaysLeft` and `permissions` are added with the screens that show them (P1.10–P1.11); until then `adapter:health` and `permission:changed` carry that information. `hold`'s `untilTomorrow` ends at the next 06:00 local time at least 2 h away; `indefinite` stores 9999-12-31. `set_mode('paused')` remembers the mode before it, and the tray's Resume returns to it.
| `list_live_items` | `{ appId?: string }` | `Item[]` (current tabs/windows with age and protection reasons) |

```ts
interface AdapterHealth { id: string; kind: 'browser' | 'platform'; name: string; profile?: string;
  status: 'connected' | 'degraded' | 'down'; reason?: string; version?: string; lastSeenAt: number }
interface Permissions { accessibility: 'granted' | 'denied' | 'notRequired'; notifications: 'granted' | 'denied' | 'unknown' }
```

### 2.4 Rules

| Command | Input | Output |
| --- | --- | --- |
| `list_rules` | `{}` | `Rule[]` |
| `create_rule` | `Omit<Rule,'id'\|'hitCount'\|'createdAt'>` | `Rule` (free tier: max 3 `customThreshold` → `PRO_REQUIRED`) |
| `update_rule` | `{ id } & Partial<Rule>` | `Rule` |
| `delete_rule` | `{ id }` | `{}` |
| `test_rule` | `{ rule: Partial<Rule> }` | `{ matchesLive: number; matchesArchived: number; sample: Item[] }` |

### 2.5 Settings

| Command | Input | Output |
| --- | --- | --- |
| `get_settings` | `{}` | `Settings` (see DB §3.9) |
| `update_settings` | `Partial<Settings>` | `Settings` (validates thresholds: stale < hidden < archive, each 1–720 h) |
| `set_hotkey` | `{ accelerator: string }` | `{ ok: true } \| AppError('CONFLICT')` |
| `set_launch_at_login` | `{ enabled: boolean }` | `{ enabled }` |

### 2.6 Data and privacy

| Command | Input | Output |
| --- | --- | --- |
| `export_archive` | `{ format: 'json' \| 'html' \| 'csv'; path: string; include?: { rules: boolean; settings: boolean } }` | `{ path; count }` |
| `import_archive` | `{ path: string; format: 'json' \| 'html' }` | `{ imported: number; skipped: number }` |
| `erase_all_data` | `{ confirm: 'ERASE' }` | `{}` → app restarts into onboarding |
| `get_storage_info` | `{}` | `{ dbBytes; itemCount; oldestArchivedAt; backups: { at: number; bytes: number }[] }` |
| `restore_backup` | `{ at: number }` | `{}` |
| `export_diagnostics` | `{ path; includeUrls: boolean }` | `{ path }` |
| `get_metrics` | `{ days?: number }` | `{ regretRate; autoModeDays; openTabsTrend: { day; p50 }[]; archived; searchToRestoreP50Ms }` |

### 2.7 Onboarding, platform and license

| Command | Input | Output |
| --- | --- | --- |
| `detect_browsers` | `{}` | `{ id; name; installed: boolean; hostInstalled: boolean; extensionConnected: boolean; storeUrl: string }[]` |
| `install_native_hosts` | `{ browsers?: string[] }` | `{ installed: string[] }` |
| `request_accessibility` | `{}` | `{ status }` (opens System Settings pane) |
| `complete_onboarding` | `{ preset; windowManagement: 'off' \| 'hide'; launchAtLogin: boolean }` | `{}` |
| `get_license` | `{}` | `{ tier: 'free' \| 'trial' \| 'pro'; trialDaysLeft?: number; email?: string }` |
| `activate_license` | `{ key: string }` | `{ tier: 'pro'; email }` or `AppError('INVALID_INPUT')` |
| `deactivate_license` | `{}` | `{ tier }` |
| `check_for_updates` | `{}` | `{ available: boolean; version?; notes?; date? }` |
| `install_update` | `{ when: 'now' \| 'onQuit' }` | `{}` |
| `ui_show_window` | `{ window: 'library' \| 'settings' \| 'onboarding'; route?: string }` | `{}` |
| `ui_hide_panel` | `{}` | `{}` |
| `ui_take_pending` | `{}` | `{ prefill: string \| null; licenseKey: string \| null }` — what's waiting for the calling window (`panel`: a `decay://search` prefill; `settings`: a license key to confirm). Each value is handed out once; windows call it on load and on `panel:open` / `pending` (P1.7) |

---

## 3. Tauri events (core → UI)

| Event | Payload | Consumers |
| --- | --- | --- |
| `status:changed` | `get_status` result | Tray popover, Library header |
| `item:changed` | `{ ids: string[]; change: 'state' \| 'updated' \| 'deleted' }` | Invalidate TanStack queries |
| `sweep:started` | `{ sweepId; isShadow; count }` | — |
| `sweep:completed` | `Sweep & { preview: Item[] /* ≤5 */ }` — **as built (P1.7):** `{ id; isShadow; archived; failed }` (the Undo toast, P1.11, adds the preview) | Undo toast (30 s) |
| `sweep:undone` | `{ sweepId; restored }` | Toast |
| `adapter:health` | `AdapterHealth` | Health banner |
| `permission:changed` | `Permissions` | Banner / onboarding |
| `panel:open` | `{}` (P1.7: the prefill is taken with `ui_take_pending`, so a panel opened cold doesn't miss it) | Panel focuses input |
| `pending` | `{}` | Settings takes a waiting license key (`ui_take_pending`) |
| `sweep:nothing` | `{ isShadow }` | "Nothing to sweep" after Sweep now |
| `shadow:complete` | `{ reportReady: true }` | Shadow Review screen |
| `update:available` | `{ version; notes }` | Settings → Updates badge |
| `license:changed` | `{ tier }` | Pro gates |
| `navigate` | `{ window; route }` | Deep links / tray menu |

---

## 4. Native messaging protocol (extension ⇄ host ⇄ core)

**Host name:** `com.decay.host` · **Envelope:**

```ts
interface Envelope<T = unknown> {
  v: 1;                       // protocol version
  id: string;                 // message id (UUID), echoed in replies
  type: string;               // see tables below
  replyTo?: string;
  chunk?: { seq: number; total: number; groupId: string }; // for payloads > 900 KB
  payload: T;
}
```

**Handshake:**
1. Ext → `hello { extVersion, browser: 'chrome'|'edge'|'brave'|'arc'|'chromium', profileId (random UUID stored in chrome.storage, ≤ 128 chars), capabilities: ['tabGroups','probe'], launch?: boolean }`. `launch: true` (only at browser start-up or on a user action) lets the host start the app; reconnects omit it, so an app the user quit isn't reopened. Unknown enum values from a newer side read as `unknown` / `other` instead of failing the message
2. Core → `welcome { coreVersion, minExtVersion, mode, config: { probeEnabled, heartbeatSec: 20 } }`
3. If the versions are incompatible → `error { code:'VERSION_MISMATCH' }` and the extension shows an "update" badge.

### 4.1 Extension → core

| Type | Payload | When |
| --- | --- | --- |
| `snapshot` | `{ windows: { id, focused, state: 'normal'\|'minimized'\|'maximized'\|'fullscreen', incognito:false, tabs: TabInfo[] }[]; groups: { id, title, color, windowId }[] }` | After the handshake, and on `resync` request (chunked) |
| `tab.used` | `{ tabId, windowId, at, signal: 'activated' \| 'visible' \| 'audible' \| 'typed' }` | Debounced |
| `tab.created` / `tab.updated` / `tab.removed` / `tab.moved` | `TabInfo` / `{ tabId, changes }` / `{ tabId, byUser: boolean }` / `{ tabId, windowId, index }` | Real-time |
| `window.focus` | `{ windowId \| -1 }` | — |
| `probe.result` | `{ tabId, dirtyForm: boolean, beforeUnload: boolean, uploading: boolean }` | Reply to `probe`. Phase 0 (P0.7) computes `dirtyForm` and `beforeUnload` (only counted after user activation); `uploading` = file/large-body XHR or fetch in progress (needs the watcher); in Phase 0 uploads fold into the `dirty` verdict (protected as `formInput`, shown as "unsaved work"). Phase 0 adds `unknown?: boolean` to the payload (`{ tabId, dirtyForm, beforeUnload, uploading, unknown? }`): true when the page couldn't be checked → the 2× threshold applies |
| `ack` | `{ ok: boolean; error?: { code; message }; result?: unknown }` | Reply to any command |
| `heartbeat` | `{ at }` | Every 20 s |
| `import.archive` | `{ items, rules, settings }` (chunked) | Phase 0 → app migration |

```ts
interface TabInfo { tabId: number; windowId: number; index: number; url: string; title: string;
  favIconUrl?: string; pinned: boolean; audible: boolean; mutedInfo?: { muted: boolean };
  discarded: boolean; groupId: number; active: boolean; status: 'loading' | 'complete';
  lastAccessed?: number }
```

### 4.2 Core → extension (commands — all idempotent, all acked)

| Type | Payload | Extension action |
| --- | --- | --- |
| `probe` | `{ tabIds: number[] }` | Inject the probe script; reply `probe.result` per tab |
| `tabs.close` | `{ sweepId, tabIds: number[] }` | Re-verify not active/pinned/audible → `tabs.remove`; ack per tab with `skipped` reasons |
| `tabs.restore` | `{ items: { itemId, url, windowKey?: string, index?, group?: {title,color}, active?: boolean }[]; newWindow?: boolean }` | `windows.create` / `tabs.create` / `tabs.group` |
| `tabs.focus` | `{ tabId }` | Activate the tab and focus its window |
| `resync` | `{}` | Send a full `snapshot` |
| `config` | `{ probeEnabled, excludedDomains: string[] }` | Update local filters (excluded domains are never reported) |

**Extension-side safety re-check:** before `tabs.close`, the extension independently re-checks `active`, `pinned`, `audible`, and the probe result. A disagreement → `ack { ok:false, error:{ code:'UNSAFE' } }`. Defence in depth.

**As built (P1.4):**
- **Handshake and reporting:**
  - `hello` may carry `session`, a random id per browser session.
  - The app sends `config` right after `welcome`. The extension reports nothing before that first `config` (so excluded sites never leak), then sends the `snapshot`.
  - A tab moving onto an excluded site is reported as `tab.removed` once; one leaving it, as `tab.created`.
  - Decay's own extension pages are never reported.
- **`probe`:** the verdicts come back in the ack, `{ ok: true, result: { results: ProbeResult[], missing: number[] } }`, and no separate `probe.result` messages are sent, so the reply is unambiguous.
- **`tabs.close`:**
  - It may carry `urls: { [tabId]: expectedUrl }`.
  - Each tab is re-read after the probe and skipped with a reason: `active`, `pinned`, `audible`, `formInput`, `changed` (navigated since the app decided), `inUse` (`lastAccessed` within the last 60 s), `excluded`, `disconnected` (the link dropped mid-command) or `error`.
  - The ack is `{ ok, error?: UNSAFE, result: { closed, skipped: [{tabId, reason}], unchecked } }`. `unchecked` lists tabs closed although their page couldn't be checked; the app's 2× rule already applied to them.
  - The probe runs whatever `probeEnabled` says.
- **`tabs.restore`:** `windowKey` is `<session>:<windowId>` and matches a normal window only within the same browser session; otherwise the items go into one new window. The new window is focused when an item is `active`. URLs follow Phase 0's restore rule (http(s), browser pages, `about:blank`); `javascript:`, `data:` and `file:` are refused.
- **Unknown commands:** they get `ack { ok:false, error: BAD_MESSAGE }`.
- **`tabs.close` `requireChecked`** (P1.5): tab ids the app planned at the normal threshold on an earlier clean check; if the page can't be checked right before closing, the tab is skipped with reason `unchecked`. The close loop stops when a new connection replaces the one it came on (`disconnected`), and a tab Decay closed is reported as `byUser: false` even on the next connection.
- **A replaced connection** (a newer `hello` for the same profile) gets `error { code: APP_UNAVAILABLE }` and is ignored from then on; the extension ends that port.
- **Acks with results:** `tabs.close` and `tabs.restore` answer `ok:false` *with* a `result` when any tab is skipped or fails; the app settles from the `result`. `tabs.restore`'s result is `{ restored: [{itemId, tabId}], failed: [itemId] }`.

**Popup requests (P1.5, extension → app, answered with `ack`):** while the app is connected its archive is the real one, so the popup asks it.

| Type | Payload | Ack `result` |
| --- | --- | --- |
| `archive.search` | `{ query, limit? (1–200, default 50), utcOffsetMinutes? }` | `{ items: ArchivedItem[], more, lastSweep? }`. Empty query: most recent first, plus `lastSweep { id, startedAt, finishedAt?, itemCount, remaining }` (the newest real sweep Undo can revert). Always `mode`: the app's mode, which the popup shows while connected |
| `archive.restore` | `{ itemIds, intoWindowId? }` | `{ restored, failed, skipped }` (`skipped`: not archived any more). With `intoWindowId` the tabs open in that window, the first active |
| `archive.restoreBatch` | `{ itemId }` | Same. The item's batch (same sweep and window) reopens in its original window (same browser session) or a new one |
| `archive.undo` | `{ sweepId? }` | Same; `NOT_FOUND` when the sweep can't be undone (not among the last 20 finished ones, already undone, or still closing tabs). Without `sweepId`: the newest undoable sweep |

`ArchivedItem` has the extension `Item`'s fields the popup uses (`id, kind, appId, title, url, domain, faviconUrl, tabIndex, groupTitle, groupColor, pinned, state, activeAgeS, firstSeenAt, lastUsedAt, stateChangedAt, archivedAt, archiveReason, lastSweepId, restoreCount`) plus `windowKey` (`<session>:<windowId>` of its original window). The background answers the popup's `decay.search` / `decay.restore` / `decay.restoreBatch` / `decay.undoSweep` through these while connected; a Phase 0 sweep (still in the extension's IndexedDB) is undone locally as before.
- **`import.archive`:** it's sent in batches of ≤ 2,000 items on every connection and every minute while connected, with only what changed since the last confirmed batch (see Software Design §4.7).

### 4.3 Chunking
- Max outgoing message from the host is 1 MB (Chrome limit). Payloads are split at 900 KB into `chunk.seq/total` sharing a `groupId`; the receiver reassembles them within 10 s or discards them.
- **As built (P1.3, `decay-bridge::chunk`):**
  - **Chunk shape:** the whole original envelope is serialised to JSON and cut at **code point** boundaries. The extension must cut the same way, never between the two halves of a surrogate pair, because a lone surrogate in a piece is replaced by U+FFFD. Each piece is sent as `{v, id: <new>, type: <original type>, chunk: {seq, total, groupId}, payload: "<piece>"}`. A piece's escaped length is at most 900 KB, so every chunk stays under 1 MB.
  - **Reassembly:** the receiver joins pieces in `seq` order and parses the original envelope.
  - **Limits:** at most 160 chunks per group (enough for a 64 MiB message, which can double in size when escaped again), 64 groups pending, group ids ≤ 64 bytes, chunked types ≤ 64 bytes, and 160 MB pending in all, counting about 1 KB of overhead per group. A sender refuses to produce a group the receiver would reject. Duplicate chunks are ignored. A group with a chunk out of range or inconsistent with the rest is dropped, and so is a chunk nested inside a chunk.
  - **Direction:** the host chunks app → extension messages. Extension → app chunks pass through the host unchanged and are reassembled by the app.
- **Text:** lone UTF-16 surrogates (`\ud83d` escapes) and invalid UTF-8 from the browser become U+FFFD before parsing, as Chrome itself does when converting to UTF-8.

### 4.4 Errors
`VERSION_MISMATCH`, `APP_UNAVAILABLE`, `UNSAFE`, `NOT_FOUND` (tab already gone → treated as success for close), `TIMEOUT`, `BAD_MESSAGE`.

---

## 5. Host IPC (decay-host ⇄ app)

- Socket: macOS `~/Library/Application Support/Decay/run/decay.sock` (mode `0600`); Windows `\\.\pipe\decay-{userSid}` (ACL: current user only).
- Framing: `u32 LE length + JSON`, the same envelope as §4, wrapped as `{ conn: { browser, profileId, pid }, msg: Envelope }`.
- The host is a dumb relay: it adds `conn` metadata and enforces `allowed_origins`.
- **As built (P1.3, `decay-bridge::ipc`):**
  - **Frames:** host → app frames are `{ conn: { browser, profileId, pid, origin }, msg }`. App → host frames are plain envelopes.
  - **macOS socket:** the `run/` folder is `0700` and must be owned by the user. The socket is chmodded to `0600` right after binding (macOS can't set it at creation). The app refuses peers whose effective uid differs. A stale socket left by a crash is replaced; a live one means another app instance (`AddrInUse`).
  - **Windows pipe:** `\\.\pipe\decay-<random>`, a new name each time the app starts. The app publishes the name in `%APPDATA%\Decay\run\pipe-name`, the user's own profile folder, and the host reads it from there. Reasons:
    - A fixed name, whether built from the user's SID as specified or from the user name, could be created first by another user to receive this user's tabs ("pipe squatting").
    - The SID would need raw Win32 calls, which `forbid(unsafe_code)` rules out.

    The protected DACL `D:P(A;;GA;;;OW)(A;;GA;;;SY)` (owner and SYSTEM only) keeps other users from connecting. The app must not run elevated: its pipe's owner would then be Administrators, and the host would be refused.
  - **Both ends check the user:** on Unix the host checks that the socket it connects to is served by a process of the same user (peer credentials), and the app checks every peer the same way. The socket is replaced only if it's stale, and removed on shutdown only if it's still the app's own.
  - **Messages are forwarded as received:** the host checks that an extension message is a JSON object, with invalid UTF-8 and lone surrogates replaced, but doesn't rebuild it. A message that would exceed the IPC frame limit gets `BAD_MESSAGE`. Frames are written without per-frame flushes: on Windows a flush waits for the reader.
  - **Overrides:** `DECAY_IPC_ENDPOINT` and `DECAY_SUPPORT_DIR` override the endpoint and the support folder, in debug builds only (tests, development). Release builds ignore them, so the browser's environment can't redirect the host.

---

## 6. Deep links (`decay://`)

| URL | Action |
| --- | --- |
| `decay://search?q=pricing` | Open the panel with the query prefilled |
| `decay://restore/{itemId}` | Restore the item (from notifications) |
| `decay://sweep/{sweepId}` | Open Library → Sweep detail |
| `decay://undo/{sweepId}` | Undo that sweep (requires confirmation if > 1 h old) |
| `decay://settings/{tab}` | Open Settings at a tab |
| `decay://license/activate?key=…` | Prefill license activation (from purchase email) |
| `decay://shadow-report` | Open the Shadow Review |

Deep links never perform a destructive action without UI confirmation.

---

## 7. External endpoints

### 7.1 Update manifest (GitHub Releases)
`GET https://github.com/<org>/decay/releases/latest/download/latest.json` (beta: `…/beta/latest.json`)
```json
{ "version": "1.2.0", "notes": "…", "pub_date": "2026-12-01T10:00:00Z",
  "platforms": { "darwin-aarch64": { "signature": "…", "url": "…" },
                 "darwin-x86_64":  { "signature": "…", "url": "…" },
                 "windows-x86_64": { "signature": "…", "url": "…" } } }
```

### 7.2 License issuance (serverless, not called by the app)
`POST /webhooks/purchase` (from Lemon Squeezy/Paddle, HMAC-verified) → creates a license and emails it.

**License key format:** `DECAY-` + base32( `payload` ‖ `ed25519_sig` ), where
`payload = { v:1, email, tier:'pro', issuedAt, licenseId }`. The app verifies offline with the embedded public key. No activation server and no device limit in v1.
