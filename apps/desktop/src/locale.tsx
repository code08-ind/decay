import { getLocale, onLocaleChange, setLocale, setSystemLocale } from '@decay/ui';
import { listen } from '@tauri-apps/api/event';
import { type ReactNode, useSyncExternalStore } from 'react';
import { api } from './api';

/** How long the first render waits for the language (the engine answers in milliseconds). */
const FIRST_WAIT_MS = 1500;

/**
 * Follows the `locale` setting (F31) in this window, now and whenever it changes in any
 * window. Resolves once the language is known (or after a short wait), so a window is
 * drawn once, in its language. `system` is the OS language as the app reads it, so the
 * windows agree with the tray and menus.
 */
export function followLocaleSetting(): Promise<void> {
  let changed = false;
  listen<Record<string, unknown>>('settings:changed', (e) => {
    changed = true;
    setLocale(e.payload.locale);
  }).catch(() => {});
  const known = api
    .systemLanguage()
    .then((tag) => setSystemLocale(tag))
    .catch(() => {})
    .then(() => api.getSettings())
    .then((s) => {
      if (!changed) setLocale(s.locale);
    })
    .catch(() => {
      if (!changed) setLocale('system');
    });
  const wait = new Promise<void>((resolve) => setTimeout(resolve, FIRST_WAIT_MS));
  return Promise.race([known, wait]);
}

/**
 * Draws the window again in a new language when it changes, keeping its state (what's
 * open, focus, a toast being shown): `render` makes a new element, so every component
 * renders again and asks `t` anew.
 */
export function Localized(props: { render: () => ReactNode }) {
  useSyncExternalStore(onLocaleChange, getLocale);
  return <>{props.render()}</>;
}
