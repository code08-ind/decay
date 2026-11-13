import { applyTheme } from '@decay/ui';
import { listen } from '@tauri-apps/api/event';
import { api } from './api';

/** Applies the `theme` setting (F19) now and whenever it changes in any window. */
export function followThemeSetting(): void {
  // An event that arrives first is newer than the initial read: the read is then ignored.
  let changed = false;
  listen<Record<string, unknown>>('settings:changed', (e) => {
    changed = true;
    applyTheme(e.payload.theme);
  }).catch(() => {});
  api
    .getSettings()
    .then((s) => changed || applyTheme(s.theme))
    .catch(() => changed || applyTheme('system'));
}
