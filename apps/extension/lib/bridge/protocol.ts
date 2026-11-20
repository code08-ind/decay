// The native messaging protocol (Docs/API Design.md §4), mirrored from the Rust
// `decay-bridge` crate (crates/decay-bridge/src/protocol.rs). Field names are camelCase on
// both sides; unknown fields are ignored by the app, and unknown enum values read as
// `unknown` / `other` there.

export const HOST_NAME = 'com.decay.host';
export const PROTOCOL_VERSION = 1;

export interface Chunk {
  seq: number;
  total: number;
  groupId: string;
}

export interface Envelope<T = unknown> {
  v: number;
  id: string;
  type: string;
  replyTo?: string;
  chunk?: Chunk;
  payload: T;
}

export const kinds = {
  // Extension → core.
  hello: 'hello',
  snapshot: 'snapshot',
  tabUsed: 'tab.used',
  tabCreated: 'tab.created',
  tabUpdated: 'tab.updated',
  tabRemoved: 'tab.removed',
  tabMoved: 'tab.moved',
  windowFocus: 'window.focus',
  probeResult: 'probe.result',
  ack: 'ack',
  heartbeat: 'heartbeat',
  importArchive: 'import.archive',
  /** The popup's search was used (once per opening; P2.7, the app's Insights count it). */
  metricsSearched: 'metrics.searched',
  // Popup requests in adapter mode (the archive lives in the app), answered with an ack.
  archiveSearch: 'archive.search',
  archiveRestore: 'archive.restore',
  archiveRestoreBatch: 'archive.restoreBatch',
  archiveUndo: 'archive.undo',
  /** The popup names this browser profile (P2.8), answered with `ack { name }`. */
  profileRename: 'profile.rename',
  // Core → extension.
  welcome: 'welcome',
  probe: 'probe',
  tabsClose: 'tabs.close',
  tabsRestore: 'tabs.restore',
  tabsFocus: 'tabs.focus',
  resync: 'resync',
  config: 'config',
  // Both ways.
  error: 'error',
} as const;

export type ErrorCode =
  | 'VERSION_MISMATCH'
  | 'APP_UNAVAILABLE'
  | 'UNSAFE'
  | 'NOT_FOUND'
  | 'TIMEOUT'
  | 'BAD_MESSAGE';

export interface ErrorPayload {
  code: ErrorCode | string;
  message?: string;
}

export type BrowserKind = 'chrome' | 'edge' | 'brave' | 'arc' | 'chromium';

export interface Hello {
  extVersion: string;
  browser: BrowserKind;
  /** Random UUID kept in `chrome.storage.local`, one per browser profile. */
  profileId: string;
  capabilities: string[];
  /** Start the app if it isn't running: only at browser start-up or on a user action. */
  launch: boolean;
  /** This browser session's id: `tabs.restore` window keys are `<session>:<windowId>`. */
  session?: string;
}

export interface Welcome {
  coreVersion: string;
  minExtVersion: string;
  mode: 'shadow' | 'automatic' | 'paused';
  config: { probeEnabled: boolean; heartbeatSec: number };
}

export interface TabInfoMsg {
  tabId: number;
  windowId: number;
  index: number;
  url: string;
  title: string;
  favIconUrl?: string;
  pinned: boolean;
  audible: boolean;
  discarded: boolean;
  groupId: number;
  active: boolean;
  status: string;
  lastAccessed?: number;
}

export interface WindowInfoMsg {
  id: number;
  focused: boolean;
  state: string;
  incognito: false;
  tabs: TabInfoMsg[];
}

export interface GroupInfoMsg {
  id: number;
  title: string;
  color: string;
  windowId: number;
}

export interface Snapshot {
  windows: WindowInfoMsg[];
  groups: GroupInfoMsg[];
}

export interface ProbeResult {
  tabId: number;
  dirtyForm: boolean;
  beforeUnload: boolean;
  uploading: boolean;
  unknown?: boolean;
}

export interface Ack {
  ok: boolean;
  error?: ErrorPayload;
  result?: unknown;
}

export interface TabsClose {
  sweepId: string;
  tabIds: number[];
  /** The URL the app expects each tab to show (by tab id); a tab that navigated is skipped. */
  urls?: Record<string, string>;
  /**
   * Tabs the app planned on an earlier "clean" check: if the page can't be checked right
   * before closing, they're skipped (`unchecked`) instead of closed at the normal threshold.
   */
  requireChecked?: number[];
}

export interface RestoreItem {
  itemId: string;
  url: string;
  windowKey?: string;
  index?: number;
  group?: { title: string; color: string };
  active?: boolean;
}

export interface TabsRestore {
  items: RestoreItem[];
  newWindow?: boolean;
}

export interface ConfigMsg {
  probeEnabled: boolean;
  excludedDomains: string[];
}

/** `import.archive`: the Phase 0 archive moved into the app (records as stored). */
export interface ImportArchive {
  items: unknown[];
  rules: unknown[];
  settings: Record<string, unknown>;
}

/** `archive.search`: the app's archive (empty query: most recent first). */
export interface ArchiveSearch {
  query: string;
  limit?: number;
  /** Minutes east of UTC (for `before:` / `after:` dates). */
  utcOffsetMinutes?: number;
}

/** An archived item as the app sends it (the popup's `Item` fields). */
export interface ArchivedItem {
  id: string;
  kind: 'tab' | 'window';
  appId: string;
  title: string;
  url?: string;
  domain?: string;
  faviconUrl?: string;
  tabIndex?: number;
  groupTitle?: string;
  groupColor?: string;
  pinned: boolean;
  state: string;
  activeAgeS: number;
  firstSeenAt: number;
  lastUsedAt: number;
  stateChangedAt: number;
  archivedAt?: number;
  archiveReason?: unknown;
  lastSweepId?: string;
  /** The original window, `<session>:<windowId>` (search groups batches by it). */
  windowKey?: string;
  restoreCount: number;
}

export interface LastSweep {
  id: string;
  startedAt: number;
  finishedAt?: number;
  itemCount: number;
  /** Items of that sweep still archived. */
  remaining: number;
}

export interface ArchiveSearchResult {
  items: ArchivedItem[];
  more: boolean;
  /** Only for an empty query: the sweep the popup's Undo reverts. */
  lastSweep?: LastSweep;
  /** The app's mode: it decides while connected. */
  mode?: 'shadow' | 'automatic' | 'paused';
  /** This browser profile as the app names it (P2.8). */
  profile?: PopupProfile;
}

/** This browser profile: its name, else "Profile {ordinal}" when its browser has several. */
export interface PopupProfile {
  name?: string;
  ordinal: number;
  hasSiblings: boolean;
}

/** `archive.restore` / `archive.restoreBatch` / `archive.undo` ack result. */
export interface RestoreSummary {
  restored: number;
  failed: number;
  /** Requested items that weren't archived any more. */
  skipped: number;
}

export function newId(): string {
  return crypto.randomUUID();
}

export function envelope<T>(type: string, payload: T, replyTo?: string): Envelope<T> {
  return { v: PROTOCOL_VERSION, id: newId(), type, ...(replyTo ? { replyTo } : {}), payload };
}

/** Whether a value received from the host has the envelope's shape. */
export function isEnvelope(value: unknown): value is Envelope {
  if (typeof value !== 'object' || value === null) return false;
  const v = value as Record<string, unknown>;
  return typeof v.v === 'number' && typeof v.id === 'string' && typeof v.type === 'string';
}
