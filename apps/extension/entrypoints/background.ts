import { settings } from '@/lib/settings/settings';

export default defineBackground(() => {
  // Runs on every service-worker start, not only on install: it is idempotent, and this
  // recovers if the worker was killed during `onInstalled`. Opening the DB seeds default rules.
  void settings.initFirstRun();
  // Tracker (P0.2), policy (P0.3) and sweeps (P0.5) are wired in here next.
});
