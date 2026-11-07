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
| `run_sweep_now` | `{ dryRun?: boolean }` | `Sweep` |
| `undo_sweep` | `{ id?: string /* default: last */ }` | `{ restored: number; failed: number }` |
| `get_shadow_report` | `{}` | `{ days: number; wouldArchive: Item[]; wouldHide: Item[]; protectedCount: number; topDomains: { domain: string; count: number }[] }` |

### 2.3 Engine status and control

| Command | Input | Output |
| --- | --- | --- |
| `get_status` | `{}` | `{ mode: Mode; holdUntil?: number; clock: 'running' \| 'paused_idle' \| 'paused_sleep' \| 'paused_meeting' \| 'paused_hold'; staleCount: number; liveTabs: number; liveWindows: number; adapters: AdapterHealth[]; shadowDaysLeft?: number; permissions: Permissions }` |
| `set_mode` | `{ mode: Mode }` | `{ mode }` |
| `hold` | `{ duration: '1h' \| 'untilTomorrow' \| 'indefinite' \| { minutes: number } }` | `{ holdUntil?: number }` |
| `release_hold` | `{}` | `{}` |
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

---

## 3. Tauri events (core → UI)

| Event | Payload | Consumers |
| --- | --- | --- |
| `status:changed` | `get_status` result | Tray popover, Library header |
| `item:changed` | `{ ids: string[]; change: 'state' \| 'updated' \| 'deleted' }` | Invalidate TanStack queries |
| `sweep:started` | `{ sweepId; isShadow; count }` | — |
| `sweep:completed` | `Sweep & { preview: Item[] /* ≤5 */ }` | Undo toast (30 s) |
| `sweep:undone` | `{ sweepId; restored }` | Toast |
| `adapter:health` | `AdapterHealth` | Health banner |
| `permission:changed` | `Permissions` | Banner / onboarding |
| `panel:open` | `{ prefill?: string }` | Panel focuses input |
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
1. Ext → `hello { extVersion, browser: 'chrome'|'edge'|'brave'|'arc', profileId (random UUID stored in chrome.storage), capabilities: ['tabGroups','probe'] }`
2. Core → `welcome { coreVersion, minExtVersion, mode, config: { probeEnabled, heartbeatSec: 20 } }`
3. If the versions are incompatible → `error { code:'VERSION_MISMATCH' }` and the extension shows an "update" badge.

### 4.1 Extension → core

| Type | Payload | When |
| --- | --- | --- |
| `snapshot` | `{ windows: { id, focused, state: 'normal'\|'minimized'\|'maximized'\|'fullscreen', incognito:false, tabs: TabInfo[] }[]; groups: { id, title, color, windowId }[] }` | After the handshake, and on `resync` request (chunked) |
| `tab.used` | `{ tabId, windowId, at, signal: 'activated' \| 'visible' \| 'audible' \| 'typed' }` | Debounced |
| `tab.created` / `tab.updated` / `tab.removed` / `tab.moved` | `TabInfo` / `{ tabId, changes }` / `{ tabId, byUser: boolean }` / `{ tabId, windowId, index }` | Real-time |
| `window.focus` | `{ windowId \| -1 }` | — |
| `probe.result` | `{ tabId, dirtyForm: boolean, beforeUnload: boolean, uploading: boolean }` | Reply to `probe`. Phase 0 (P0.7) computes `dirtyForm` and `beforeUnload` (only counted after user activation); `uploading` isn't detected yet (always `false`). Phase 0 adds `unknown?: boolean` to the payload (`{ tabId, dirtyForm, beforeUnload, uploading, unknown? }`): true when the page couldn't be checked → the 2× threshold applies |
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

### 4.3 Chunking
- Max outgoing message from the host is 1 MB (Chrome limit). Payloads are split at 900 KB into `chunk.seq/total` sharing a `groupId`; the receiver reassembles them within 10 s or discards them.

### 4.4 Errors
`VERSION_MISMATCH`, `APP_UNAVAILABLE`, `UNSAFE`, `NOT_FOUND` (tab already gone → treated as success for close), `TIMEOUT`, `BAD_MESSAGE`.

---

## 5. Host IPC (decay-host ⇄ app)

- Socket: macOS `~/Library/Application Support/Decay/run/decay.sock` (mode `0600`); Windows `\\.\pipe\decay-{userSid}` (ACL: current user only).
- Framing: `u32 LE length + JSON`, the same envelope as §4, wrapped as `{ conn: { browser, profileId, pid }, msg: Envelope }`.
- The host is a dumb relay: it adds `conn` metadata and enforces `allowed_origins`.

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
