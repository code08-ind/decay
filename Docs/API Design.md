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
  | 'PRO_REQUIRED' | 'UNSAFE' | 'DB_ERROR' | 'TIMEOUT' | 'CONFLICT'
  | 'IO_ERROR'; // as built (P2.5): a file couldn't be read or written (e.g. disk full)
```

---

## 2. Tauri commands (UI → core)

Each command returns `Promise<T>` and rejects with `AppError`. Access is limited per window by Tauri **capabilities** (e.g. the `panel` window can call only search/restore/status).

### 2.1 Search and archive

| Command | Input | Output | Notes |
| --- | --- | --- | --- |
| `search` | `{ query: string; filters?: { app?: string; domain?: string; after?: number; before?: number; kind?: ItemKind; sourceId?: string }; limit?: number /*≤200, default 50*/ ; cursor?: string }` | `{ groups: { key: string; label: string; sourceTitle?: string; items: Item[] }[]; nextCursor?: string; tookMs: number }` | Parses inline operators `app:`, `site:`, `before:`, `after:`, `in:`. Groups by day + source |
| `list_items` | `{ state?: ItemState[]; view: 'today' \| 'week' \| 'all' \| 'protected' \| 'live'; sort?: 'recent' \| 'age' \| 'title'; cursor?: string; limit?: number }` | `{ items: Item[]; nextCursor?: string; total: number }` | Library views — **as built (P1.11):** not a command: the Library lists the archive with `search` (empty query = newest first) plus `after` (epoch ms: local midnight for Today, the start of the locale's week for This week) and `kind`. Protected / Live now views come later |
| `library_stats` | `{ todayStart; weekStart }` (epoch ms, the UI's calendar) | `{ archivedTotal; archivedToday: { tabs; windows }; restoredWeek; openTabs; mode; shadowDay: number \| null }` | P1.11 stat cards and shadow banner. `shadowDay` counts calendar days from `shadow_started_at` in the system time zone (day 1 = the day it started). `restoredWeek` counts items restored or brought back by Undo |
| `get_item` | `{ id }` | `Item & { events: { type: string; at: number; reason?: Reason }[] }` | Detail drawer timeline — **as built (P1.11):** the panel row (`id kind title url domain appName sourceId sourceTitle archivedAt reason batch`) plus `state pinned groupTitle firstSeenAt lastUsedAt activeHours sweepId sweepNumber restoreCount`, and `events: { type; at; label }[]` (newest first, ≤ 200; `label` = the reason in words) |
| `get_batch` | `{ itemId }` | `{ sweepId; sourceTitle; items: Item[] }` | Items archived together from the same source |
| `restore_items` | `{ ids: string[]; target?: 'original' \| 'newWindow' }` | `{ restored: string[]; failed: { id: string; error: AppError }[] }` | Batch restore with > 1 source requires Pro → `PRO_REQUIRED` — **as built (P2.7):** also `searchMs?: number` (first keystroke → this restore, from the panel's search), recorded for Insights once the restore reopens something |
| `restore_batch` | `{ itemId }` | same as above | Pro |
| `restore_items_in` | `{ ids; appId; profileId }` | `{ started: true }` | **As built (P2.8):** "Open in … instead": all into a new window of that connected browser profile. Only ever on that click: `restore_items` / `restore_batch` never send tabs to another profile. Their `ADAPTER_UNAVAILABLE` refusal names the missing profile ("Open Google Chrome · Work to restore this…") and carries `details: { itemIds, home: { appId, profileId, label } \| null, alternatives: [{ appId, profileId, label }] }` (the connected profiles) |
| `forget_profile` | `{ appId; profileId }` | — | **As built (P2.8):** a disconnected profile only (`INVALID_INPUT` while connected); its tabs still reopen only there; emits `profiles:changed`. Idle profiles (30 days, nothing archived) are forgotten by hourly maintenance |
| `list_profiles` | — | `{ appId; appName; profileId; name; ordinal; hasSiblings; connected; tabs; firstSeenAt; lastSeenAt }[]` | **As built (P2.8):** every browser profile seen, by browser and first sighting |
| `rename_profile` | `{ appId; profileId; name: string \| null }` | the stored name (trimmed, ≤ 40 chars; blank → `null`) | **As built (P2.8):** emits `profiles:changed`. `search` also takes `app` / `profile` (the Library's browser filter) and the `profile:` operator (id, part of a name, or an unnamed profile's number); result rows carry `profile: { id; name; ordinal } \| null` (absent for a browser's only, unnamed profile). `get_metrics` adds `topSources` (archived since the range start by app and profile) |
| `delete_items` | `{ ids: string[] }` | `{ deleted: number }` | Permanently removes from the archive — **as built:** 1–500 ids, only items that are `archived` and not being restored right now (others are skipped, not counted), one durable transaction; the UI asks first and reloads after |
| `protect_item` | `{ id; scope: 'domain' \| 'app' \| 'url' }` | `Rule` | "Always protect" quick action |

### 2.2 Sweeps

| Command | Input | Output |
| --- | --- | --- |
| `list_sweeps` | `{ input?: { limit?: number; includeShadow?: boolean } }` | `Sweep[]` — **as built (P1.11):** newest first, ≤ 200 (default 50), shadow sweeps only with `includeShadow`; each row `{ id; number /* place among sweeps of its kind (real / shadow), oldest = 1 */; startedAt; trigger; isShadow; status; itemCount /* total, kept after deletes */; tabs; windows /* of the items still stored */; restored /* archived by it, back since */; failed; undoneAt; undoable /* Undo accepts it (last 20 real sweeps that archived something) and something is still archived from it */ }`. Only items the sweep really archived count (closes the browser skipped or that failed don't) |
| `get_sweep` | `{ id }` | `Sweep & { items: Item[] }` — **as built:** the `list_sweeps` row plus `items: (panel row & { state; archived /* still archived from this sweep, not re-archived by a later one */ })[]`; a shadow sweep lists its `shadow_would_archive` items with their reasons |
| `run_sweep_now` | `{ dryRun?: boolean }` | `Sweep` — **as built (P1.7):** `{ started: true }`; the result arrives as `sweep:completed` / `sweep:nothing` (a sweep waits for probes and close acks). Refusals: `ADAPTER_UNAVAILABLE` (no browser), `CONFLICT` (already sweeping), `UNSAFE` (paused / on hold) |
| `undo_sweep` | `{ id?: string /* default: last */ }` | `{ restored: number; failed: number }` — **as built (P1.7):** `{ started: true }`; the counts arrive as `sweep:undone`. Refusals: `NOT_FOUND`, `CONFLICT` (still closing), `ADAPTER_UNAVAILABLE` (its browser isn't connected) |
| `get_shadow_report` | `{}` | `{ days: number; wouldArchive: Item[]; wouldHide: Item[]; protectedCount: number; topDomains: { domain: string; count: number }[] }` — **as built (P1.11):** `{ mode; startedAt; day; days: 7; tabs; windows; reopened; wouldArchive: (row & { wouldArchiveAt /* first time */; usedAfter /* used again since: a regret */; live })[]; kept: (row & { by: ProtectedBy; label; activeHours })[]; keptBy: { by; label; count }[] }`. Would-archive = distinct items with `shadow_would_archive` events since `shadow_started_at` (aggregated in SQL; the engine writes one such event per item per stale period, not one per sweep, and records no shadow sweep when nothing new would be archived); `day` = calendar day in the system time zone (day 1 = the day it started; the week is over from day 8); kept = live tabs at least stale-old that a protection keeps right now (engine `kept_live`). Window hiding and top domains come with Phase 2 |

### 2.3 Engine status and control

| Command | Input | Output |
| --- | --- | --- |
| `get_status` | `{}` | `{ mode: Mode; holdUntil?: number; clock: 'running' \| 'paused_idle' \| 'paused_sleep' \| 'paused_meeting' \| 'paused_hold'; staleCount: number; liveTabs: number; liveWindows: number; adapters: AdapterHealth[]; shadowDaysLeft?: number; permissions: Permissions }` |
| `set_mode` | `{ mode: Mode }` | `{ mode }` |
| `resume` | `{}` | `{ mode }` — **P1.8:** ends a pause, back to the mode before it (shadow or automatic) |
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
| `list_rules` | `{}` | `Rule[]` — **as built (P1.12):** oldest first, every source (default, user, quick action) |
| `create_rule` | `Omit<Rule,'id'\|'hitCount'\|'createdAt'>` | `Rule` (free tier: max 3 `customThreshold` → `PRO_REQUIRED`) — **as built (P1.12):** `{ rule: { scope; match; action; thresholdHours?; enabled? } }` → `{ rule: Rule; purged: number }`. Validated like the extension (domains normalized, bare public suffixes refused, safe title patterns, too-broad excludes refused, exclude not for app/global, closeAllowed only for apps, thresholds 1–720 h increasing); `INVALID_INPUT` with the reason, `CONFLICT` for an identical rule. The policy reloads at once; an enabled `exclude` deletes every stored item it matches (open tabs' records and archives, with their events) in the same transaction as the save (items a sweep is closing are deleted once the close settles) and tells the extensions. Every successful rule change (also `update_rule`, `delete_rule`, `protect_item`) emits `rules:changed`. `update_rule` that only turns a rule off skips validation. Pro limit not enforced yet (licensing) |
| `update_rule` | `{ id } & Partial<Rule>` | `Rule` — **as built:** `{ id; rule: <as create> }` → `{ rule; purged }` (all editable fields; `enabled` kept when left out) |
| `delete_rule` | `{ id }` | `{}` (`NOT_FOUND` when it's gone) |
| `preview_exclude` | `{ rule }` | `{ count }` — P1.12: how many stored items saving this exclude rule would delete, including items a sweep is closing (the UI confirms first); 0 for other actions |
| `test_rule` | `{ rule: Partial<Rule> }` | `{ matchesLive: number; matchesArchived: number; sample: Item[] }` — **as built:** `{ rule: <as create> }` (an invalid rule is refused with its reason); `sample` = up to 5 matching open tabs as panel rows, least recently used first |

### 2.5 Settings

| Command | Input | Output |
| --- | --- | --- |
| `get_settings` | `{}` | `Settings` (see DB §3.9) |
| `update_settings` | `Partial<Settings>` | `Settings` (validates thresholds: stale < hidden < archive, each 1–720 h) — **as built (P1.12, P1.13):** `theme`; `update_channel` stable / beta (a change checks the new channel at once); `auto_update` boolean; `preset` (gentle / balanced / aggressive: also sets its `thresholds`); `thresholds` (custom: sets `preset` to `custom`; not together with `preset`); `retention_days` 7–3650; `idle_cutoff_s` 60–3600; `window_management` off / hide; `crash_reports` true / false (P2.6: applied at once). Saved in one transaction; the policy reloads at once. The mode goes through `set_mode` / `resume`, the hotkey and launch at login through their commands; other keys → `INVALID_INPUT`. Every change (also `set_hotkey`, `set_launch_at_login`, an archive import) is broadcast as `settings:changed` with all settings |
| `set_hotkey` | `{ accelerator: string }` | `{ ok: true } \| AppError('CONFLICT')` |
| `set_launch_at_login` | `{ enabled: boolean }` | `{ enabled }` (debug builds only save the setting) |

### 2.6 Data and privacy

| Command | Input | Output |
| --- | --- | --- |
| `export_archive` | `{ format: 'json' \| 'html' \| 'csv'; path: string; include?: { rules: boolean; settings: boolean } }` | `{ path; count }` — **as built (P2.5):** no `path`: the app opens the system's save dialog itself (a window can't name a file to write), and returns `null` when it's cancelled. `include` (JSON only) defaults to both on. Written owner-only (0600) to a temporary file next to the target, synced, renamed into place, folder synced. Bookmarks: web pages only, one folder per local day (each item's own offset). CSV starts with a UTF-8 BOM, times are local `YYYY-MM-DD HH:MM:SS`; cells starting with `= + - @`, tab or CR are prefixed with `'`. Errors: `PERMISSION_DENIED`, `NOT_FOUND`, `IO_ERROR` |
| `import_archive` | `{ path: string; format: 'json' \| 'html' }` | `{ imported: number; skipped: number }` — **as built (P2.5):** no arguments: the app opens the system's file dialog (`null` when cancelled), reads a regular file of ≤ 64 MB (counted while reading), tells JSON (a leading BOM is fine) from bookmarks by content: a Netscape doctype or `<DT><A` entries (a saved web page is refused); otherwise JSON only when named `.json`. Returns `{ done: { fileName, itemsImported, itemsExisting, itemsUpdated, itemsLive, itemsInvalid, itemsExpired, itemsExcluded, itemsPurged, rulesImported, rulesExisting, rulesInvalid, rulesTurnedOff, settingsImported, linksCapped } }`, or — when the file's new "never record" rules would delete stored items — `{ confirm: { token, purge, fileName } }` with nothing imported (see `confirm_import`). Refused (`INVALID_INPUT`, nothing changed) unless it's a `decay-archive` v ≤ 1, a header-less archive with `items` (the extension's), or a bookmarks file with web links. A `decay-preimport-<ms>.db` backup is written first (3 kept). Items on never-recorded sites (this installation's exclude rules and the file's) are dropped; items archived longer ago than retention are skipped (`itemsExpired`); times are clamped to now, text capped, tabs must have an address that reopens, ids / app ids must be safe, the site comes from the address; a file only updates archived / restored records of the same kind with a later state, bookmarks never update and skip addresses already archived; rules pass the same checks as `create_rule`; settings must be in `update_settings`' ranges; the mode, shadow week, hold and retention never come from a file. Emits `archive:changed` and, when the file had rules, `rules:changed` |
| `confirm_import` | — | **as built (P2.5):** `{ token: number; proceed: boolean }` → the same outcome as `import_archive`, or `null` (dropped, or `token` isn't the waiting file's). Imports the file `import_archive` kept waiting, allowing exactly the confirmed number of deletions (a file that would now delete more asks again). A new `import_archive` pick replaces a waiting file. Settings window only |
| `erase_all_data` | `{ confirm: 'ERASE' }` | `{}` → app restarts into onboarding |
| `get_storage_info` | `{}` | `{ dbBytes; itemCount; oldestArchivedAt; backups: { at: number; bytes: number }[] }` |
| `restore_backup` | `{ at: number }` | `{}` |
| `export_diagnostics` | `{ path; includeUrls: boolean }` | `{ path }` — **as built (P2.6):** no arguments: the app opens the system's save dialog (`null` when cancelled) and returns the saved path. A zip (owner-only, written atomically) with `diagnostics.json` (`format: "decay-diagnostics"`, version 1: app version / build / OS / arch, the engine's state without profile ids or sweep ids, settings, counts from the store — items by kind and state, rules by source / action / on-off, sweeps by kind and status, events, sources, backups, schema version, database size) and `logs/decay.<date>.log` (the end of each, ≤ 8 MB, scrubbed again). No `includeUrls`: logs never contain addresses (scrubbed when written). Settings window only |
| `get_diagnostics_info` | — | **as built (P2.6):** `{ crashReportsAvailable: boolean; logsDir: string }` (the home folder as `~`). Settings window only |
| `open_logs_folder` | — | **as built (P2.6):** opens `…/Decay/logs` in Finder / Explorer (created if missing). Settings window only |
| `log_ui_error` | — | **as built (P2.6):** `{ message: string; stack?: string }` → nothing. A window's JavaScript error (`error`, `unhandledrejection`; a rejected command as `code: message`) written to the log at `error` level, scrubbed; at most 50 per window a run (then one line saying so), message ≤ 2,000 and stack ≤ 4,000 characters. Every window |
| `get_metrics` | `{ days?: number }` | `{ regretRate; autoModeDays; openTabsTrend: { day; p50 }[]; archived; searchToRestoreP50Ms }` — **as built (P2.7):** `{ since?: number }` (ms, default 30 days ago) → `{ mode, days: MetricsDay[], topArchived: { domain; count }[], restoredMost: { domain; count }[] }`. `days`: every stored day (≤ 400, oldest first) in the extension's `MetricsDay` shape (`day`, `activeSeconds`, `openTabsP50`, `openWindowsP50`, `archivedCount`, `restored24hCount`, `undoCount`, `searchCount`, `searchToRestoreMsP50`, `autoModeOn`, `shadowFlaggedCount`, `shadowRegretCount`, `sampledHours`, `openTabsSamples`, `openWindowsSamples`, `searchToRestoreSamples`); the counts come from `metric_events`, on the local day each was stamped with. `topArchived`: sites archived most since `since`; `restoredMost`: sites restored most, all time (top 5 each; for the page only, never exported). The page computes ranges, goals and the beta file with `@decay/ui/metrics`, shared with the extension. Library window only |
| `record_search` | — | **as built (P2.7):** one search in the panel (the panel calls it once per opening, at the first query of ≥ 2 characters). Panel window only |
| `get_beta_context` | — | **as built (P2.7):** `{ installId, appVersion, platform, browsers, mode, preset, thresholds, shadowStartedAt, windowManagement, rules }` for the "Share with beta team" file: a random install id (made once, setting `beta_install_id`, never exported elsewhere), connected browser kinds (no profiles), enabled rules per action (no patterns). Library window only |
| `save_metrics_file` | — | **as built (P2.7):** `{ kind: 'beta' \| 'csv'; content: string }` → the saved path, or `null` when the dialog is cancelled. Saves exactly the text the page showed (the beta JSON must have `format: "decay-beta-metrics"`, the CSV its header row; ≤ 4 MB), owner-only and atomically. Library window only |
| `open_link` | — | **as built (P2.7, F32):** `{ link: 'help' \| 'changelog' \| 'feedback' }` → nothing. Opens a fixed address in the default browser (a window never passes one): Help → the `decay-releases` README until a docs site exists (`help.rs` `HELP_URL`), the changelog → its Releases page, feedback → a new GitHub issue there, prefilled with questions and `Decay <version> · <OS> · <arch>` only (the version is the app's, from `tauri.conf.json`). Library and Settings windows |
| `whats_new` | — | **as built (P2.7, F32):** `{ version; show }`: `show` once the running version is newer than the one in `whats_new_seen` (a beta then its release counts); a new install or a downgrade stores the running version and shows nothing. The notes ship with the app (`apps/desktop/src/help/releases.ts`); a version without notes is marked seen quietly. Library window only |
| `whats_new_seen` | — | **as built (P2.7):** `{ later?: boolean }`. "Got it": stores the running version. `later: true` ("Later", Escape, ×) stores nothing and only stops `whats_new` showing it again until Decay restarts (the Library window is rebuilt each time it opens). Library window only |
| `ui_show_whats_new` | — | **as built (P2.7):** Help → What's New: an open Library is brought forward and shows it over its current screen (event `whats-new:open`); otherwise the Library opens at `/?whats-new`. When the running version has no notes, the newest notes are shown labelled "Latest notes". Library and Settings windows (the Help menu calls it directly) |

### 2.7 Onboarding, platform and license

| Command | Input | Output |
| --- | --- | --- |
| `detect_browsers` | `{}` | `{ id; name; installed: boolean; hostInstalled: boolean; extensionConnected: boolean; profiles: number; storeUrl: string }[]` — **as built (P1.10):** `id` is the extension's `browser` value (`chrome`, `arc`, `brave`, `edge`, `chromium`); a browser this platform doesn't know is left out unless it's connected; a connected browser counts as installed |
| `install_native_hosts` | `{ browsers?: string[] }` | `{ installed: string[] }` — **as built:** always every detected browser (`browsers` isn't taken); `NOT_FOUND` when the build has no `decay-host` next to it (dev builds), `PERMISSION_DENIED` when it failed for every browser tried. One install at a time (start-up and onboarding) |
| `open_extension_store` | `{ browser: string }` (an `id` from `detect_browsers`) | `{}` — opens the extension's store page **in that browser** (macOS `open -b <bundle id>`; Windows: the browser's own exe from its App Paths entry, no shell or console), off the main thread; `INVALID_INPUT` for another id, `NOT_FOUND` ("open the Chrome Web Store in it yourself") when it can't be opened — never another browser. P1.10; the URL is a store search until the listing is public |
| `request_accessibility` | `{}` | `{ requested: true; opened: 'prompt' \| 'settings' }` — **as built (P1.10):** the first request of a run shows the system prompt (which also lists Decay in System Settings); later ones open System Settings → Accessibility (macOS shows the prompt once). `ADAPTER_UNAVAILABLE` when the engine isn't running. The result arrives as `permission:changed` |
| `complete_onboarding` | `{ input: { preset; windowManagement: 'off' \| 'hide'; launchAtLogin: boolean } }` | `{}` — **as built (P1.10):** sets `preset` + its `thresholds`, `window_management`, `launch_at_login`, clears a hold, `mode = shadow`, `shadow_started_at = onboarded_at = now` (one durable transaction), then broadcasts `settings:changed` and applies launch at login (release builds) before returning. `set_launch_at_login` only saves the setting in debug builds; login-item changes are serialised |
| `get_license` | `{}` | `{ tier: 'free' \| 'trial' \| 'pro'; trialDaysLeft?: number; email?: string }` |
| `activate_license` | `{ key: string }` | `{ tier: 'pro'; email }` or `AppError('INVALID_INPUT')` |
| `deactivate_license` | `{}` | `{ tier }` |
| `start_trial` | `{}` | **As built (P4.2):** the four license commands answer the same status `{ tier: 'free' \| 'trial' \| 'pro'; trialDaysLeft?; trialEndsAt?; trialAvailable; email? }` (also the `license:changed` payload). `activate_license` refuses a key that doesn't verify with `INVALID_INPUT` (why: not a key / not valid / for a newer version); `start_trial` refuses a second trial with `CONFLICT`. Pro refusals elsewhere: `create_rule` / `update_rule` (a 4th custom-timing rule), `restore_batch`, `update_settings` (raising `retention_days` past 90) → `PRO_REQUIRED` |
| `check_for_updates` | `{}` | `{ available: boolean; version?; notes?; date? }` — **as built (P1.13):** returns `UpdateState` `{ current; phase: 'idle' \| 'checking' \| 'available' \| 'downloading' \| 'ready' \| 'error'; version; notes; date (RFC 3339); downloaded; total; checkedAt; error; failed: 'check' \| 'download' \| 'install' \| null; installOnQuit; installNow }`. Reads the channel's manifest (§7.1); with `auto_update` on, a new version starts downloading (verified against the updater key before it's kept). One check at a time; none during a download. Debug builds: `DECAY_UPDATE_URL` overrides the manifest |
| `get_update_state` | `{}` | `UpdateState` — P1.13 |
| `install_update` | `{ when: 'now' \| 'onQuit' }` | `{}` — **as built (P1.13):** returns `UpdateState`. `now`: installs the downloaded update and restarts (downloads first when needed; a request made during a check or download is carried out when it ends, and dropped if a newer version replaces the one it was for); `onQuit`: installs at the next quit (downloads first when needed). `NOT_FOUND` when no update was found. Automatic checks (start-up, then whenever the last check is a day old, and on a channel change) run in release builds only, or with `DECAY_UPDATE_CHECK=1`; "Check now" always checks |
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
| `update:available` | `{ version; notes }` | Settings → Updates badge — P1.13: once per new version per run (the badge isn't built yet) |
| `update:state` | `UpdateState` | Settings → Updates (P1.13): every phase change and download progress (every 256 KB) |
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
| `metrics.searched` | `{}` | **P2.7:** the popup's search was used (once per opening, while connected); the app counts it for Insights. No reply. With `searchMs` on `archive.restore` / `archive.restoreBatch`, the extension leaves search metrics to the app while connected, so an app older than P2.7 doesn't count them: the app and extension ship together |

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
| `archive.restore` | `{ itemIds, intoWindowId?, searchMs? }` (`searchMs`: P2.7) | `{ restored, failed, skipped }` (`skipped`: not archived any more). With `intoWindowId` the tabs open in that window, the first active |
| `archive.restoreBatch` | `{ itemId, searchMs? }` | Same. The item's batch (same sweep and window) reopens in its original window (same browser session) or a new one |
| `archive.undo` | `{ sweepId? }` | Same; `NOT_FOUND` when the sweep can't be undone (not among the last 20 finished ones, already undone, or still closing tabs). Without `sweepId`: the newest undoable sweep |
| `profile.rename` | `{ name: string \| null }` | **As built (P2.8):** names the asking browser profile; ack `{ name }` (as stored). `archive.search` results carry `profile: { name?; ordinal; hasSiblings }` for the popup's "This profile: …" line |

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
`GET https://github.com/code08-ind/decay-releases/releases/latest/download/latest.json` (beta: `…/releases/download/beta/latest.json`). A separate public repo: the source repo is private (P1.13, Docs/Release.md)
```json
{ "version": "1.2.0", "notes": "…", "pub_date": "2026-12-01T10:00:00Z",
  "platforms": { "darwin-aarch64": { "signature": "…", "url": "…" },
                 "darwin-x86_64":  { "signature": "…", "url": "…" },
                 "windows-x86_64": { "signature": "…", "url": "…" } } }
```

### 7.2 License issuance (serverless, not called by the app)
`POST /webhooks/purchase` (from Lemon Squeezy/Paddle, HMAC-verified) → creates a license and emails it.

**As built (P4.3, `services/license-webhook`, a Cloudflare Worker), first for Lemon Squeezy, then (24 Nov 2026) for Paddle: `Paddle-Signature: ts=…;h1=…` = HMAC-SHA256 of `<ts>:<raw body>` with the notification destination's secret, at most 5 minutes old (else `401`); only `transaction.completed`, status `completed`, with a price in `PADDLE_PRICE_IDS`; the email from `GET /customers/<customer_id>`; `licenseId` `pd-<txn id>`, `issuedAt` = `billed_at`; `Idempotency-Key: decay-<txn id>`. The Lemon Squeezy version, kept for the record:** `X-Signature` = hex HMAC-SHA256 of the raw body with the webhook's signing secret (else `401`); bodies over 256 KB → `413`; not JSON → `400`. A license is issued only for `meta.event_name = order_created` with `data.attributes.status = paid`, a variant in `LS_VARIANT_IDS` (when set), a plausible `user_email`, and `test_mode` false unless `ALLOW_TEST_MODE = 1`; anything else → `200 ignored: …` (so Lemon Squeezy doesn't retry). The key's `licenseId` is `ls-<order id>` and `issuedAt` the order's `created_at`: the same order always yields the same key. Sent with Resend (`Idempotency-Key: decay-order-<id>`); a failed send → `500` (Lemon Squeezy retries). No storage; refunds don't revoke keys (offline keys can't be).

**License key format:** `DECAY-` + base32( `payload` ‖ `ed25519_sig` ), where
`payload = { v:1, email, tier:'pro', issuedAt, licenseId }`. The app verifies offline with the embedded public key. No activation server and no device limit in v1.
