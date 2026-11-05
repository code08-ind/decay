import { db } from '@/lib/db/db';
import { settings } from '@/lib/settings/settings';

export default defineBackground(() => {
  browser.runtime.onInstalled.addListener(async ({ reason }) => {
    if (reason === 'install') {
      await db.open(); // triggers `populate` (default rules) on first open
      await settings.initFirstRun();
    }
  });
  // Tracker (P0.2), policy (P0.3) and sweeps (P0.5) are wired in here next.
});
