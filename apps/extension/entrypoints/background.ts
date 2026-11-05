import { db } from '@/lib/db/db';
import { settings } from '@/lib/settings/settings';
import { chromeApi, detectAppId } from '@/lib/tracker/browser-api';
import { TICK_MS } from '@/lib/tracker/clock';
import { Tracker } from '@/lib/tracker/tracker';

const TICK_ALARM = 'decay-tick';

export default defineBackground(() => {
  const tracker = new Tracker({
    db,
    api: chromeApi,
    appId: detectAppId(),
    getIdleCutoffS: () => settings.get('idleCutoffS'),
  });

  // Every handler is idempotent and logs instead of throwing, so one bad event
  // can't stop the worker from processing the next one.
  const run = (label: string, task: () => Promise<unknown>) => {
    task().catch((error) => console.error(`[decay] ${label} failed`, error));
  };

  // Runs on every service-worker start, not only on install: it is idempotent, and this
  // recovers if the worker was killed during `onInstalled`. Opening the DB seeds default rules.
  run('first run', () => settings.initFirstRun());

  // Listeners must be registered synchronously at startup so MV3 can wake the worker.
  browser.alarms.onAlarm.addListener((alarm) => {
    if (alarm.name === TICK_ALARM) run('tick', () => tracker.tick());
  });
  browser.tabs.onCreated.addListener((tab) => run('tab created', () => tracker.upsertTab(tab)));
  browser.tabs.onUpdated.addListener((tabId, change, tab) => {
    if (change.audible === true) run('tab audible', () => tracker.tabUsed(tabId));
    else if ('url' in change || 'title' in change || 'pinned' in change || 'groupId' in change) {
      run('tab updated', () => tracker.upsertTab(tab));
    }
  });
  browser.tabs.onActivated.addListener(({ tabId }) =>
    run('tab activated', () => tracker.tabUsed(tabId)),
  );
  browser.tabs.onRemoved.addListener((tabId, info) =>
    run('tab removed', () => tracker.tabRemoved(tabId, info.isWindowClosing)),
  );
  browser.tabs.onMoved.addListener((tabId) => run('tab moved', () => tracker.refreshTab(tabId)));
  browser.tabs.onAttached.addListener((tabId) =>
    run('tab attached', () => tracker.refreshTab(tabId)),
  );
  browser.tabs.onReplaced.addListener((added, removed) =>
    run('tab replaced', () => tracker.tabReplaced(added, removed)),
  );
  browser.tabGroups?.onUpdated.addListener((group) =>
    run('group updated', () => tracker.groupUpdated(group.id)),
  );
  browser.windows.onFocusChanged.addListener((windowId) =>
    run('window focus', () => tracker.windowFocused(windowId)),
  );

  // Ensure the tick alarm exists (alarms survive worker restarts but not reinstalls),
  // and reconcile right away so the archive reflects the open tabs.
  run('schedule', async () => {
    const existing = await browser.alarms.get(TICK_ALARM);
    if (!existing) {
      await browser.alarms.create(TICK_ALARM, { periodInMinutes: TICK_MS / 60_000 });
    }
    await tracker.tick();
  });
});
