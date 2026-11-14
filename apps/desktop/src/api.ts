// The shell's commands and events (API Design §2.3, §3). Typed by hand until the UI
// phases generate them (tauri-specta).
import { invoke } from '@tauri-apps/api/core';

export type Mode = 'shadow' | 'automatic' | 'paused';

export interface BrowserStatus {
  browser: string;
  profileId: string;
  tabs: number;
}

export interface EngineStatus {
  mode: Mode;
  holdUntil: number | null;
  staleCount: number;
  liveTabs: number;
  browsers: BrowserStatus[];
  sweeping: boolean;
  undoableSweep: string | null;
  accessibility: boolean | null;
  /** In a call or meeting (mic or camera in use): scheduled sweeps wait (F12). */
  inMeeting: boolean;
}

export interface AppError {
  code: string;
  message: string;
}

/** Data waiting for this window (`ui_take_pending`). */
export interface Pending {
  prefill: string | null;
  licenseKey: string | null;
}

export interface PanelItem {
  id: string;
  kind: 'tab' | 'window';
  title: string;
  url: string | null;
  domain: string | null;
  appName: string;
  sourceId: string | null;
  sourceTitle: string | null;
  archivedAt: number | null;
  reason: string | null;
  batch: boolean;
}

export interface SearchOutput {
  items: PanelItem[];
  nextCursor: string | null;
  totalArchived: number;
  tookMs: number;
}

export type Preset = 'gentle' | 'balanced' | 'aggressive';
export type WindowManagement = 'hide' | 'off';

/** A browser as onboarding sees it (`detect_browsers`). */
export interface DetectedBrowser {
  id: string;
  name: string;
  installed: boolean;
  hostInstalled: boolean;
  extensionConnected: boolean;
  profiles: number;
  storeUrl: string;
}

export type ItemState =
  | 'active'
  | 'stale'
  | 'hidden'
  | 'archiving'
  | 'archived'
  | 'restored'
  | 'purged';

export interface LibraryStats {
  archivedTotal: number;
  archivedToday: { tabs: number; windows: number };
  restoredWeek: number;
  openTabs: number;
  mode: Mode;
  shadowDay: number | null;
}

export interface TimelineEvent {
  type: string;
  at: number;
  label: string | null;
}

export interface ItemDetail extends PanelItem {
  state: ItemState;
  pinned: boolean;
  groupTitle: string | null;
  firstSeenAt: number;
  lastUsedAt: number;
  activeHours: number;
  sweepId: string | null;
  sweepNumber: number | null;
  restoreCount: number;
  events: TimelineEvent[];
}

export interface SweepRow {
  id: string;
  number: number;
  startedAt: number;
  trigger: 'scheduled' | 'wake' | 'manual';
  isShadow: boolean;
  status: 'pending' | 'running' | 'done' | 'failed' | 'undone';
  /** Items it archived (or would have), including ones deleted since. */
  itemCount: number;
  tabs: number;
  windows: number;
  restored: number;
  failed: number;
  undoneAt: number | null;
  undoable: boolean;
}

export interface SweepEntry extends PanelItem {
  state: ItemState;
  archived: boolean;
}

export interface SweepDetail extends SweepRow {
  items: SweepEntry[];
}

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
  | 'unrestorable';

export interface ShadowItem extends PanelItem {
  wouldArchiveAt: number;
  usedAfter: boolean;
  live: boolean;
}

export interface KeptItem extends PanelItem {
  by: ProtectedBy;
  label: string;
  activeHours: number;
}

export interface ShadowReport {
  mode: Mode;
  startedAt: number | null;
  day: number;
  days: number;
  tabs: number;
  windows: number;
  reopened: number;
  wouldArchive: ShadowItem[];
  kept: KeptItem[];
  keptBy: { by: ProtectedBy; label: string; count: number }[];
}

export type RuleScope = 'app' | 'domain' | 'urlGlob' | 'titleRegex' | 'global';
export type RuleAction = 'protect' | 'exclude' | 'customThreshold' | 'closeAllowed';

export interface Thresholds {
  stale: number;
  hidden: number;
  archive: number;
}

export interface Rule {
  id: string;
  scope: RuleScope;
  match: string;
  action: RuleAction;
  thresholdHours?: Thresholds;
  enabled: boolean;
  source: 'user' | 'default' | 'quick_action';
  createdAt: number;
  hitCount: number;
}

/** What `create_rule` / `update_rule` / `test_rule` take. */
export interface RuleInput {
  scope: RuleScope;
  match: string;
  action: RuleAction;
  thresholdHours?: Thresholds;
  enabled?: boolean;
}

export interface RuleTestResult {
  matchesLive: number;
  matchesArchived: number;
  sample: PanelItem[];
}

/** The daily summary (`get_daily_summary`, P2.4, Designs/11). */
export interface DailySummary {
  archivedTabs: number;
  hiddenWindows: number;
  restored: number;
  hints: { kind: 'alwaysKeep' | 'keepLonger'; domain: string; count: number }[];
  items: PanelItem[];
}

/** The updater (`get_update_state`, `update:state`). */
export interface UpdateState {
  current: string;
  phase: 'idle' | 'checking' | 'available' | 'downloading' | 'ready' | 'error';
  version: string | null;
  notes: string | null;
  date: string | null;
  downloaded: number;
  total: number | null;
  checkedAt: number | null;
  error: string | null;
  /** What failed, when `phase` is `error`. */
  failed: 'check' | 'download' | 'install' | null;
  installOnQuit: boolean;
  /** "Restart now" asked: installs as soon as the download is ready. */
  installNow: boolean;
}

export type HoldDuration = '1h' | 'untilTomorrow' | 'indefinite' | { minutes: number };

export const api = {
  getStatus: () => invoke<EngineStatus>('get_status'),
  setMode: (mode: Mode) => invoke<{ mode: Mode }>('set_mode', { mode }),
  /** Ends a pause: back to the mode before it (shadow or automatic). */
  resume: () => invoke<{ mode: Mode }>('resume'),
  hold: (duration: HoldDuration) => invoke<{ holdUntil: number | null }>('hold', { duration }),
  releaseHold: () => invoke('release_hold'),
  runSweepNow: () => invoke('run_sweep_now'),
  undoSweep: (id?: string) => invoke('undo_sweep', { id }),
  setHotkey: (accelerator: string) => invoke('set_hotkey', { accelerator }),
  setLaunchAtLogin: (enabled: boolean) => invoke('set_launch_at_login', { enabled }),
  /** The system prompt the first time in a run, then System Settings → Accessibility. */
  requestAccessibility: () =>
    invoke<{ requested: boolean; opened: 'prompt' | 'settings' }>('request_accessibility'),
  detectBrowsers: () => invoke<DetectedBrowser[]>('detect_browsers'),
  installNativeHosts: () => invoke<{ installed: string[] }>('install_native_hosts'),
  openExtensionStore: (browser: string) => invoke('open_extension_store', { browser }),
  completeOnboarding: (input: {
    preset: Preset;
    windowManagement: WindowManagement;
    launchAtLogin: boolean;
  }) => invoke('complete_onboarding', { input }),
  showWindow: (window: 'library' | 'settings' | 'onboarding', route?: string) =>
    invoke('ui_show_window', { window, route }),
  hidePanel: () => invoke('ui_hide_panel'),
  takePending: () => invoke<Pending>('ui_take_pending'),
  search: (input: {
    query: string;
    kind?: 'tab' | 'window';
    limit?: number;
    cursor?: string;
    after?: number;
  }) => invoke<SearchOutput>('search', { input }),
  restoreItems: (ids: string[], target: 'original' | 'newWindow' = 'original') =>
    invoke('restore_items', { ids, target }),
  restoreBatch: (itemId: string) => invoke('restore_batch', { itemId }),
  protectItem: (id: string) => invoke<{ match: string }>('protect_item', { id }),
  showItem: (id: string) => invoke('ui_show_item', { id }),
  libraryStats: (input: { todayStart: number; weekStart: number }) =>
    invoke<LibraryStats>('library_stats', { input }),
  getItem: (id: string) => invoke<ItemDetail>('get_item', { id }),
  deleteItems: (ids: string[]) => invoke<{ deleted: number }>('delete_items', { ids }),
  listSweeps: (input: { limit?: number; includeShadow?: boolean } = {}) =>
    invoke<SweepRow[]>('list_sweeps', { input }),
  getSweep: (id: string) => invoke<SweepDetail>('get_sweep', { id }),
  getShadowReport: () => invoke<ShadowReport>('get_shadow_report'),
  listRules: () => invoke<Rule[]>('list_rules'),
  createRule: (rule: RuleInput) => invoke<{ rule: Rule; purged: number }>('create_rule', { rule }),
  updateRule: (id: string, rule: RuleInput) =>
    invoke<{ rule: Rule; purged: number }>('update_rule', { id, rule }),
  deleteRule: (id: string) => invoke('delete_rule', { id }),
  testRule: (rule: RuleInput) => invoke<RuleTestResult>('test_rule', { rule }),
  previewExclude: (rule: RuleInput) => invoke<{ count: number }>('preview_exclude', { rule }),
  getSettings: () => invoke<Record<string, unknown>>('get_settings'),
  getDailySummary: (dayStart: number, dayEnd: number) =>
    invoke<DailySummary>('get_daily_summary', { input: { dayStart, dayEnd } }),
  getUpdateState: () => invoke<UpdateState>('get_update_state'),
  checkForUpdates: () => invoke<UpdateState>('check_for_updates'),
  installUpdate: (when: 'now' | 'onQuit') => invoke<UpdateState>('install_update', { when }),
  updateSettings: (patch: Record<string, unknown>) =>
    invoke<Record<string, unknown>>('update_settings', { patch }),
};

/** The message of a rejected command. */
export function errorMessage(e: unknown): string {
  if (e && typeof e === 'object' && 'message' in e) return String((e as AppError).message);
  return String(e);
}

/** One line about the status, like the tray's. */
export function describe(s: EngineStatus, now = Date.now()): string {
  if (s.mode === 'paused') return 'Paused';
  if (s.holdUntil !== null && s.holdUntil > now) {
    const until = new Date(s.holdUntil);
    if (until.getUTCFullYear() >= 9999) return 'On hold';
    return `On hold until ${until.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`;
  }
  if (s.browsers.length === 0) return 'No browser connected';
  if (s.sweeping) return 'Sweeping…';
  if (s.inMeeting) return 'In a meeting · sweeps wait';
  const tabs = `${s.liveTabs} ${s.liveTabs === 1 ? 'tab' : 'tabs'} open, ${s.staleCount} stale`;
  return s.mode === 'shadow' ? `${tabs} · shadow mode` : tabs;
}
