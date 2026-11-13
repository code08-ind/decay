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

export type HoldDuration = '1h' | 'untilTomorrow' | 'indefinite' | { minutes: number };

export const api = {
  getStatus: () => invoke<EngineStatus>('get_status'),
  setMode: (mode: Mode) => invoke<{ mode: Mode }>('set_mode', { mode }),
  hold: (duration: HoldDuration) => invoke<{ holdUntil: number | null }>('hold', { duration }),
  releaseHold: () => invoke('release_hold'),
  runSweepNow: () => invoke('run_sweep_now'),
  undoSweep: (id?: string) => invoke('undo_sweep', { id }),
  setHotkey: (accelerator: string) => invoke('set_hotkey', { accelerator }),
  setLaunchAtLogin: (enabled: boolean) => invoke('set_launch_at_login', { enabled }),
  requestAccessibility: () => invoke('request_accessibility'),
  showWindow: (window: 'library' | 'settings', route?: string) =>
    invoke('ui_show_window', { window, route }),
  hidePanel: () => invoke('ui_hide_panel'),
  takePending: () => invoke<Pending>('ui_take_pending'),
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
