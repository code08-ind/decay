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
  | 'grace';

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
}

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
}
