import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'wxt';

// See https://wxt.dev/api/config.html
export default defineConfig({
  modules: ['@wxt-dev/module-react'],
  vite: () => ({ plugins: [tailwindcss()] }),
  manifest: {
    name: 'Decay',
    description: 'Your tabs, gently put away. Archives stale tabs; search and restore anything.',
    // Permissions (Software Design §4.7).
    // Web-page access is optional, asked for at runtime for the form probe (P0.7): it lets
    // Decay check whether a page holds unsaved work before archiving it.
    // `favicon` serves cached favicons locally (no network requests from our pages).
    // `notifications` shows the Undo toast after a sweep (F7).
    permissions: [
      'tabs',
      'tabGroups',
      'storage',
      'alarms',
      'idle',
      'scripting',
      'favicon',
      'notifications',
    ],
    optional_host_permissions: ['http://*/*', 'https://*/*'],
    // The link to the Decay desktop app (P1.4) is optional too: asked for when the user
    // clicks "Connect to the Decay app", so people without the app never see the warning
    // and a store update never disables the extension for a new permission.
    optional_permissions: ['nativeMessaging'],
    incognito: 'not_allowed',
    // 30 s alarm periods (the aging clock tick) need Chrome 120+.
    minimum_chrome_version: '120',
    commands: {
      _execute_action: {
        suggested_key: { default: 'Alt+Shift+D' },
        description: 'Search archived tabs',
      },
    },
  },
});
