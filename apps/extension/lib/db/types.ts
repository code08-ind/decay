// Phase 0 record types for the extension's IndexedDB archive.
// Field names mirror the SQL schema in Docs/DB Design.md §3 (camelCase) and enum values
// follow Docs/API Design.md. The Phase 1 `import.archive` migration still needs a small
// mapping step: camelCase enums → snake_case CHECK values (urlGlob → url_glob, …),
// settings keys → snake_case, `faviconUrl` → fetched `favicon` BLOB, and defaults for
// columns tabs don't use (`saveState = 'n/a'`, `path = NULL`). `profile` lives on `sources`.
// IndexedDB cannot index booleans, so indexed flags are stored as 0 | 1.

export type ItemKind = 'tab' | 'window';

export type ItemState =
  | 'active'
  | 'stale'
  | 'hidden'
  | 'archiving'
  | 'archived'
  | 'restored'
  | 'purged';

export type Mode = 'shadow' | 'automatic' | 'paused';

export type ProtectedBy =
  | 'pinned'
  | 'activeTab'
  | 'audible'
  | 'formInput'
  | 'download'
  | 'call'
  | 'unsaved'
  | 'unknownSaveState'
  | 'rule'
  | 'hold'
  | 'meeting'
  | 'grace'
  /** Phase 0 addition: the URL can't be reliably reopened (data:, blob:, file:, …). */
  | 'unrestorable';

/** Machine-readable reason for every state transition (Docs/API Design.md). */
export type Reason =
  | { type: 'unused'; activeHours: number; threshold: number }
  | { type: 'customRule'; ruleId: string; activeHours: number }
  | { type: 'protected'; by: ProtectedBy; ruleId?: string };

export interface Thresholds {
  /** Active hours until an item is marked stale. */
  stale: number;
  /** Active hours until a window is hidden (unused for tabs). */
  hidden: number;
  /** Active hours until an item is archived. */
  archive: number;
}

export interface Item {
  id: string; // UUIDv7
  kind: ItemKind;
  appId: string; // e.g. 'com.google.Chrome'
  sourceId?: string; // browser window record
  externalId?: string; // tabId while live; not stable across restarts
  windowExternalId?: string; // browser windowId while live (Phase 0 stand-in for sources)
  profile?: string;
  title: string;
  url?: string;
  domain?: string; // eTLD+1, lowercase
  faviconUrl?: string;
  tabIndex?: number;
  groupTitle?: string;
  groupColor?: string;
  pinned: boolean;
  state: ItemState;
  activeAgeS: number;
  firstSeenAt: number;
  lastUsedAt: number;
  /** Phase 0 only: when a shadow sweep last flagged this item as "would archive". */
  shadowFlaggedAt?: number;
  /** Phase 0 only: set while a restore of this archived record is in progress (crash-safe retry). */
  restoreStartedAt?: number;
  /** Phase 0 only: when the last sampled `used` event was written (≤ 1 per 10 min). */
  lastUsedEventAt?: number;
  stateChangedAt: number;
  archivedAt?: number;
  archiveReason?: Reason;
  lastSweepId?: string;
  restoredFromId?: string;
  graceUntil?: number;
  restoreCount: number;
}

export type EventType =
  | 'seen'
  | 'used'
  | 'stale'
  | 'hidden'
  | 'archived'
  | 'close_failed'
  | 'restored'
  | 'undo'
  | 'protected'
  | 'purged'
  | 'shadow_would_archive';

export interface ItemEvent {
  id?: number; // auto-increment
  itemId: string;
  type: EventType;
  sweepId?: string;
  reason?: Reason;
  at: number;
}

export type SweepTrigger = 'scheduled' | 'wake' | 'manual';
export type SweepStatus = 'pending' | 'running' | 'done' | 'failed' | 'undone';
export type SweepAction = 'archive' | 'hide' | 'close';
export type SweepItemResult = 'pending' | 'ok' | 'failed' | 'skipped';

export interface SweepItem {
  itemId: string;
  action: SweepAction;
  prevState: ItemState;
  result: SweepItemResult;
  /** Phase 0: browser window id at archive time, so Undo can reopen into the same window. */
  windowExternalId?: string;
  /** Phase 0: why a planned close didn't happen (`skipped` / `failed`). */
  skipReason?: SweepSkipReason;
}

export type SweepSkipReason =
  | 'inUse'
  | 'changed'
  | 'modeChanged'
  | 'error'
  | 'closedByUser'
  /** P0.7: the pre-close probe found typed input or a "Leave site?" prompt. */
  | 'formInput';

export interface Sweep {
  id: string; // UUIDv7
  startedAt: number;
  finishedAt?: number;
  trigger: SweepTrigger;
  isShadow: 0 | 1;
  status: SweepStatus;
  itemCount: number;
  failedCount: number;
  /** Embedded instead of a separate sweep_items table (Phase 0 only). */
  items: SweepItem[];
  reason?: Record<string, unknown>;
  undoneAt?: number;
  /**
   * Phase 0: browser session the sweep ran in. Window ids are only meaningful within one
   * browser session, so Undo reuses the original window only in the same session.
   */
  browserSession?: string;
  /** Phase 0: set while an undo is in progress (crash-safe retry without duplicates). */
  undoStartedAt?: number;
}

export type RuleScope = 'app' | 'domain' | 'urlGlob' | 'titleRegex' | 'global';
export type RuleAction = 'protect' | 'exclude' | 'customThreshold' | 'closeAllowed';
export type RuleSource = 'user' | 'default' | 'quick_action';

export interface Rule {
  id: string;
  scope: RuleScope;
  match: string;
  action: RuleAction;
  thresholdHours?: Thresholds;
  enabled: boolean;
  source: RuleSource;
  createdAt: number;
  hitCount: number;
}

export interface SettingRecord {
  key: string;
  value: unknown;
  updatedAt: number;
}

export interface MetricsDay {
  day: string; // YYYY-MM-DD, local
  activeSeconds: number;
  openTabsP50: number;
  openWindowsP50: number;
  archivedCount: number;
  restored24hCount: number;
  undoCount: number;
  searchCount: number;
  searchToRestoreMsP50: number;
  autoModeOn: 0 | 1;
  /** Phase 0: items a shadow sweep flagged as "would archive" this day. */
  shadowFlaggedCount: number;
  /** Phase 0: flagged items used again within 24 h (shadow regret). */
  shadowRegretCount: number;
  /** Phase 0 (P0.8): local hours already sampled for open tabs / windows. */
  sampledHours?: number[];
  /** Phase 0 (P0.8): hourly samples behind `openTabsP50` / `openWindowsP50`. */
  openTabsSamples?: number[];
  openWindowsSamples?: number[];
  /** Phase 0 (P0.8): samples behind `searchToRestoreMsP50`. */
  searchToRestoreSamples?: number[];
}
