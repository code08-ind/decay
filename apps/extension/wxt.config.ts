import { defineConfig } from 'wxt';

// See https://wxt.dev/api/config.html
export default defineConfig({
  modules: ['@wxt-dev/module-react'],
  manifest: {
    name: 'Decay',
    description: 'Your tabs, gently put away. Archives stale tabs; search and restore anything.',
    // Phase 0 permissions (Software Design §4.7). `nativeMessaging` is added in Phase 1;
    // `<all_urls>` is requested at runtime for the form probe (P0.7).
    permissions: ['tabs', 'tabGroups', 'storage', 'alarms', 'idle', 'scripting'],
    optional_host_permissions: ['<all_urls>'],
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
