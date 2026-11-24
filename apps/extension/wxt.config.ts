import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'wxt';

/** The Firefox extension's id (F14): fixed forever, the desktop app's native host only
 * accepts this one (crates/decay-host/src/config.rs `FIREFOX_EXTENSION_ID`). */
export const FIREFOX_EXTENSION_ID = 'decay@decay-app';

// See https://wxt.dev/api/config.html. One source, two builds: Chromium (Chrome, Edge,
// Brave, Arc) and Firefox (`-b firefox --mv3`, F14).
export default defineConfig({
  modules: ['@wxt-dev/module-react'],
  vite: () => ({ plugins: [tailwindcss()] }),
  manifest: ({ browser }) => {
    const firefox = browser === 'firefox';
    return {
      // Store listing text is in public/_locales (the browser localizes the manifest).
      name: '__MSG_extName__',
      description: '__MSG_extDescription__',
      default_locale: 'en',
      // Permissions (Software Design §4.7).
      // Web-page access is optional, asked for at runtime for the form probe (P0.7): it
      // lets Decay check whether a page holds unsaved work before archiving it.
      // `favicon` serves cached favicons locally (no network requests from our pages) and
      // `tabGroups` keeps tab groups: both Chromium only.
      // `notifications` shows the Undo toast after a sweep (F7).
      permissions: [
        'tabs',
        ...(firefox ? [] : ['tabGroups']),
        'storage',
        'alarms',
        'idle',
        'scripting',
        ...(firefox ? [] : ['favicon']),
        'notifications',
      ],
      optional_host_permissions: ['http://*/*', 'https://*/*'],
      // The link to the Decay desktop app (P1.4) is optional too: asked for when the user
      // clicks "Connect to the Decay app", so people without the app never see the warning
      // and a store update never disables the extension for a new permission.
      optional_permissions: ['nativeMessaging'],
      incognito: 'not_allowed',
      // Chromium: 30 s alarm periods (the aging clock tick) need Chrome 120+. Firefox: MV3
      // content scripts in the page's own world (the unload probe) need Firefox 128+.
      ...(firefox
        ? {
            browser_specific_settings: {
              gecko: { id: FIREFOX_EXTENSION_ID, strict_min_version: '128.0' },
            },
          }
        : { minimum_chrome_version: '120' }),
      commands: {
        _execute_action: {
          suggested_key: { default: 'Alt+Shift+D' },
          description: '__MSG_commandSearch__',
        },
      },
    };
  },
});
