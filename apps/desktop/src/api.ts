// The shell's commands and events (API Design §2.3, §3). Typed by hand until the UI
// phases generate them (tauri-specta).

import { formatDate } from '@decay/ui';
import { invoke } from '@tauri-apps/api/core';
import { t } from './strings';

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
  /** What the window can offer about it (a refused restore's other profiles). */
  details?: unknown;
}

/** A connected browser profile a refused restore can go to instead (P2.8). */
export interface RestoreChoice {
  appId: string;
  profileId: string;
  label: string;
}

/**
 * A restore refused because the profile its tabs came from isn't connected: the items and
 * the connected profiles "Open in … instead" offers (at most three). `null` otherwise.
 */
export function restoreChoices(
  e: unknown,
): { itemIds: string[]; alternatives: RestoreChoice[] } | null {
  if (!e || typeof e !== 'object' || (e as AppError).code !== 'ADAPTER_UNAVAILABLE') return null;
  const d = (e as AppError).details as
    | { itemIds?: unknown; alternatives?: unknown }
    | null
    | undefined;
  if (!d || !Array.isArray(d.itemIds) || !Array.isArray(d.alternatives)) return null;
  const alternatives = (d.alternatives as RestoreChoice[]).filter(
    (c) =>
      c &&
      typeof c.appId === 'string' &&
      typeof c.profileId === 'string' &&
      typeof c.label === 'string',
  );
  const itemIds = (d.itemIds as unknown[]).filter((id): id is string => typeof id === 'string');
  if (!alternatives.length || !itemIds.length) return null;
  return { itemIds, alternatives: alternatives.slice(0, 3) };
}

/** Data waiting for this window (`ui_take_pending`). */
export interface Pending {
  prefill: string | null;
  licenseKey: string | null;
}

/** A row's browser profile (P2.8): absent for a browser's only, unnamed profile. */
export interface ProfileTag {
  id: string;
  name: string | null;
  /** 1-based by first sighting: "Profile {ordinal}" when unnamed. */
  ordinal: number;
}

/** A browser profile Decay has seen (`list_profiles`). */
export interface ProfileInfo {
  appId: string;
  appName: string;
  profileId: string;
  name: string | null;
  ordinal: number;
  /** Its browser has other profiles: unnamed, it shows as "Profile N". */
  hasSiblings: boolean;
  connected: boolean;
  tabs: number;
  firstSeenAt: number;
  lastSeenAt: number;
}

/** The profile's own words: its name, "Profile N" when it has siblings, else null. */
export function profileName(
  p: { name: string | null; ordinal: number },
  hasSiblings = true,
): string | null {
  if (p.name) return p.name;
  return hasSiblings ? t('profile.numbered', { n: p.ordinal }) : null;
}

/** "Google Chrome · Work" (or just the app when there's no profile to name). */
export function appWithProfile(appName: string, profile: ProfileTag | null | undefined): string {
  const name = profile ? profileName(profile) : null;
  return name ? t('profile.appWith', { app: appName, profile: name }) : appName;
}

/** How a known profile is named next to its app ("Google Chrome · Work"). */
export function profileLabel(p: ProfileInfo): string {
  return appWithProfile(
    p.appName,
    p.name || p.hasSiblings ? { id: p.profileId, name: p.name, ordinal: p.ordinal } : null,
  );
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
  profile?: ProfileTag | null;
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
    /** Only this app and browser profile (ids): the Library's browser filter. */
    app?: string;
    profile?: string;
  }) => invoke<SearchOutput>('search', { input }),
  restoreItems: (ids: string[], target: 'original' | 'newWindow' = 'original', searchMs?: number) =>
    invoke('restore_items', { ids, target, searchMs: searchMs ?? null }),
  restoreBatch: (itemId: string) => invoke('restore_batch', { itemId }),
  /** "Open in … instead": into a new window of another connected profile (P2.8). */
  restoreItemsIn: (ids: string[], appId: string, profileId: string) =>
    invoke('restore_items_in', { ids, appId, profileId }),
  /** Insights (P2.7): every stored day, and the top sites since `since` (ms). */
  getMetrics: (since: number) => invoke<MetricsReport>('get_metrics', { since }),
  /** One search in the panel (once per opening). */
  recordSearch: () => invoke<void>('record_search'),
  /** What the beta export says about this installation (makes the install id). */
  getBetaContext: () => invoke<BetaContext>('get_beta_context'),
  /** Saves the beta JSON or the daily CSV where the user picks; `null` when cancelled. */
  saveMetricsFile: (kind: 'beta' | 'csv', content: string) =>
    invoke<string | null>('save_metrics_file', { kind, content }),
  /** Help and feedback (P2.7): opens a fixed link (no address leaves the page). */
  openLink: (link: 'help' | 'changelog' | 'feedback') => invoke<void>('open_link', { link }),
  /** The running version, and whether its "What's New" is still to be shown. */
  whatsNew: () => invoke<{ version: string; show: boolean }>('whats_new'),
  /** "Got it": not shown again for this version; `later`: only for the rest of this run. */
  whatsNewSeen: (later = false) => invoke<void>('whats_new_seen', { later }),
  /** Help → What's New: over the Library's current screen (opened if needed). */
  showWhatsNew: () => invoke<void>('ui_show_whats_new'),
  listProfiles: () => invoke<ProfileInfo[]>('list_profiles'),
  /** Names a profile (null or blank: unnamed); returns the name as stored. */
  renameProfile: (appId: string, profileId: string, name: string | null) =>
    invoke<string | null>('rename_profile', { appId, profileId, name }),
  /** Forgets a profile that isn't connected (P2.8). */
  forgetProfile: (appId: string, profileId: string) =>
    invoke<void>('forget_profile', { appId, profileId }),
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
  /** The OS language as the app reads it (the windows' `system` language). */
  systemLanguage: () => invoke<string>('system_language'),
  getDailySummary: (dayStart: number, dayEnd: number) =>
    invoke<DailySummary>('get_daily_summary', { input: { dayStart, dayEnd } }),
  getUpdateState: () => invoke<UpdateState>('get_update_state'),
  checkForUpdates: () => invoke<UpdateState>('check_for_updates'),
  installUpdate: (when: 'now' | 'onQuit') => invoke<UpdateState>('install_update', { when }),
  /** Opens a save dialog and writes the export there; `null` when cancelled. */
  exportArchive: (format: ExportFormat) =>
    invoke<{ path: string; count: number } | null>('export_archive', { format }),
  /**
   * Opens a file dialog and imports the picked file; `null` when cancelled. A file whose
   * "never record" rules would delete stored items comes back as `{ confirm }` instead,
   * imported only after `confirmImport(token, true)`.
   */
  importArchive: () => invoke<ImportOutcome | null>('import_archive'),
  /** Imports (`proceed`) or drops the file waiting for confirmation. */
  confirmImport: (token: number, proceed: boolean) =>
    invoke<ImportOutcome | null>('confirm_import', { token, proceed }),
  /** Whether this build can send crash reports, and where the logs are. */
  getDiagnosticsInfo: () =>
    invoke<{ crashReportsAvailable: boolean; logsDir: string }>('get_diagnostics_info'),
  openLogsFolder: () => invoke<void>('open_logs_folder'),
  /** Opens a save dialog and writes a diagnostics zip there; `null` when cancelled. */
  exportDiagnostics: () => invoke<string | null>('export_diagnostics'),
  /** Writes a window's JavaScript error to Decay's log (scrubbed there). */
  logUiError: (message: string, stack?: string) =>
    invoke<void>('log_ui_error', { message, stack: stack ?? null }),
  updateSettings: (patch: Record<string, unknown>) =>
    invoke<Record<string, unknown>>('update_settings', { patch }),
};

/** `get_metrics` (P2.7): days in the shared `MetricsDay` shape. */
export interface MetricsReport {
  mode: 'shadow' | 'automatic' | 'paused';
  days: import('@decay/ui/metrics').MetricsDay[];
  topArchived: { domain: string; count: number }[];
  restoredMost: { domain: string; count: number }[];
  /** Archived since the range's first day by app and browser profile (P2.8). */
  topSources?: {
    appId: string;
    appName: string;
    profile: ProfileTag | null;
    tabs: number;
    windows: number;
  }[];
  /** Everything archived since the range's first day (the shares are of this). */
  archivedTotal?: number;
}

/** `get_beta_context` (P2.7). */
export interface BetaContext {
  installId: string;
  appVersion: string;
  platform: 'macos' | 'windows';
  browsers: string[];
  mode: 'shadow' | 'automatic' | 'paused';
  preset: string;
  thresholds: { stale: number; hidden: number; archive: number };
  shadowStartedAt: number | null;
  windowManagement: string;
  rules: Record<string, number>;
}

export type ExportFormat = 'json' | 'html' | 'csv';

/** `import_archive`: what the import did (API Design §2.6, as built in P2.5). */
export interface ImportResult {
  fileName: string;
  itemsImported: number;
  /** Already here, unchanged. */
  itemsExisting: number;
  /** Already here, with a newer state from the file. */
  itemsUpdated: number;
  /** Open tabs in the file (not imported: they come from the browser). */
  itemsLive: number;
  /** Unreadable, or not usable (e.g. a tab whose address can't be reopened). */
  itemsInvalid: number;
  /** Archived longer ago than this Mac keeps items. */
  itemsExpired: number;
  /** On sites Decay never records. */
  itemsExcluded: number;
  /** Stored items deleted because an imported "never record" rule covers them. */
  itemsPurged: number;
  rulesImported: number;
  rulesExisting: number;
  /** Unreadable, or refused by the same checks as adding a rule by hand. */
  rulesInvalid: number;
  /** Built-in protections the file turned off. */
  rulesTurnedOff: number;
  settingsImported: number;
  /** A bookmarks file with more links than Decay reads (100,000). */
  linksCapped: boolean;
}

/** `import_archive` / `confirm_import`: done, or the file's rules would delete stored items. */
export type ImportOutcome =
  | { done: ImportResult }
  | { confirm: { token: number; purge: number; fileName: string } };

/** One sentence about an import, for a toast. */
export function importSummary(r: ImportResult): string {
  const parts = [t('import.items', { n: r.itemsImported })];
  if (r.rulesImported) parts.push(t('import.rules', { n: r.rulesImported }));
  if (r.settingsImported) parts.push(t('import.settings', { n: r.settingsImported }));
  const text = parts.join(t('common.listSeparator'));
  const notes: string[] = [];
  if (r.itemsExisting) notes.push(t('import.note.existing', { n: r.itemsExisting }));
  if (r.itemsUpdated) notes.push(t('import.note.updated', { n: r.itemsUpdated }));
  if (r.itemsExpired) notes.push(t('import.note.expired', { n: r.itemsExpired }));
  if (r.itemsExcluded) notes.push(t('import.note.excluded', { n: r.itemsExcluded }));
  if (r.itemsLive) notes.push(t('import.note.live', { n: r.itemsLive }));
  if (r.itemsInvalid) notes.push(t('import.note.invalid', { n: r.itemsInvalid }));
  if (r.rulesInvalid) notes.push(t('import.note.rulesInvalid', { n: r.rulesInvalid }));
  if (r.rulesTurnedOff) notes.push(t('import.note.rulesOff', { n: r.rulesTurnedOff }));
  if (r.itemsPurged) notes.push(t('import.note.purged', { n: r.itemsPurged }));
  if (r.linksCapped) notes.push(t('import.note.capped', { n: 100_000 }));
  if (!notes.length) return text;
  return t('import.withNotes', { text, notes: notes.join(t('common.noteSeparator')) });
}

/** One sentence about an export, for a toast. */
export function exportSummary(r: { path: string; count: number }): string {
  const name = r.path.split(/[\\/]/).pop() ?? r.path;
  return t('export.done', { n: r.count, name });
}

/** The message of a rejected command. */
export function errorMessage(e: unknown): string {
  if (e && typeof e === 'object' && 'message' in e) return String((e as AppError).message);
  return String(e);
}

/** One line about the status, like the tray's. */
export function describe(s: EngineStatus, now = Date.now()): string {
  if (s.mode === 'paused') return t('status.paused');
  if (s.holdUntil !== null && s.holdUntil > now) {
    const until = new Date(s.holdUntil);
    if (until.getUTCFullYear() >= 9999) return t('status.onHold');
    return t('status.onHoldUntil', {
      time: formatDate(until, { hour: '2-digit', minute: '2-digit' }),
    });
  }
  if (s.browsers.length === 0) return t('status.noBrowser');
  if (s.sweeping) return t('status.sweeping');
  if (s.inMeeting) return t('status.inMeeting');
  const p = { tabs: s.liveTabs, stale: s.staleCount };
  return s.mode === 'shadow' ? t('status.tabsShadow', p) : t('status.tabs', p);
}
