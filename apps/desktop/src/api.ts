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
  search: (input: { query: string; kind?: 'tab' | 'window'; limit?: number; cursor?: string }) =>
    invoke<SearchOutput>('search', { input }),
  restoreItems: (ids: string[], target: 'original' | 'newWindow' = 'original') =>
    invoke('restore_items', { ids, target }),
  restoreBatch: (itemId: string) => invoke('restore_batch', { itemId }),
  protectItem: (id: string) => invoke<{ match: string }>('protect_item', { id }),
  showItem: (id: string) => invoke('ui_show_item', { id }),
  getSettings: () => invoke<Record<string, unknown>>('get_settings'),
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
  const tabs = `${s.liveTabs} ${s.liveTabs === 1 ? 'tab' : 'tabs'} open, ${s.staleCount} stale`;
  return s.mode === 'shadow' ? `${tabs} · shadow mode` : tabs;
}
