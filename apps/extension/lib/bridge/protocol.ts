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
