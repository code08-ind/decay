import { db } from '@/lib/db/db';
import { settings } from '@/lib/settings/settings';
import { type DecayResponse, isDecayRequest } from '@/lib/sweep/messages';
import { chromeApi, chromeCommands, detectAppId } from '@/lib/tracker/browser-api';
import { TICK_MS } from '@/lib/tracker/clock';
import { type TickResult, Tracker } from '@/lib/tracker/tracker';

const TICK_ALARM = 'decay-tick';
const SWEEP_NOTIFICATION = 'decay-sweep:';
/** Alarm that removes an Undo toast; named after its notification id. */
const CLEAR_ALARM = 'decay-clear:';
/** The Undo toast lives 30 s (PRD F7); Sweeps history keeps undo available after that. */
const UNDO_TOAST_MS = 30_000;
const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`;

export default defineBackground(() => {
  const tracker = new Tracker({
    db,
    api: chromeApi,
    appId: detectAppId(),
    commands: chromeCommands,
    ownOrigin: browser.runtime.getURL('/'),
    getIdleCutoffS: () => settings.get('idleCutoffS'),
    getPolicy: async () => {
      const all = await settings.getAll();
      return {
        mode: all.mode,
        holdUntil: all.holdUntil,
        thresholds: all.thresholds,
        maxItemsPerSweep: all.maxItemsPerSweep,
      };
    },
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
  const sweepsPage = () => browser.runtime.getURL('/sweeps.html');

  /** The Undo "toast": a notification with Undo / View buttons, cleared after 30 s. */
  const announce = async (result: TickResult) => {
    const sweep = result.sweep;
    if (!sweep?.archived) return;
    const id = `${SWEEP_NOTIFICATION}${sweep.id}`;
    await browser.notifications.create(id, {
      type: 'basic',
      iconUrl: browser.runtime.getURL('/icon/128.png'),
      title: `Archived ${plural(sweep.archived, 'tab')}`,
      message: "Tabs you hadn't used in a while. Undo brings them all back.",
      buttons: [{ title: 'Undo' }, { title: 'View' }],
      // Stays on screen (OS banners otherwise vanish after ~5 s); the alarm removes it.
      requireInteraction: true,
      priority: 0,
    });
    // An alarm, not setTimeout: MV3 may stop the worker before a timer fires.
    await browser.alarms.create(`${CLEAR_ALARM}${id}`, { when: Date.now() + UNDO_TOAST_MS });
  };

  const tickAndAnnounce = async (task: () => Promise<TickResult>) => {
    const result = await task();
    await announce(result).catch((e) => console.error('[decay] notification failed', e));
    return result;
  };

  browser.alarms.onAlarm.addListener((alarm) => {
    if (alarm.name === TICK_ALARM) run('tick', () => tickAndAnnounce(() => tracker.tick()));
    if (alarm.name.startsWith(CLEAR_ALARM)) {
      run('clear toast', () => browser.notifications.clear(alarm.name.slice(CLEAR_ALARM.length)));
    }
  });

  browser.notifications.onButtonClicked.addListener((notificationId, buttonIndex) => {
    if (!notificationId.startsWith(SWEEP_NOTIFICATION)) return;
    const sweepId = notificationId.slice(SWEEP_NOTIFICATION.length);
    void browser.notifications.clear(notificationId);
    if (buttonIndex === 0) {
      run('undo from notification', async () => {
        const r = await tracker.undoSweep(sweepId);
        await browser.notifications.create(`decay-undone:${sweepId}`, {
          type: 'basic',
          iconUrl: browser.runtime.getURL('/icon/128.png'),
          title: r.failed
            ? `Restored ${r.restored} of ${r.restored + r.failed} tabs`
            : `Restored ${plural(r.restored, 'tab')}`,
          message: r.failed
            ? 'Some tabs could not be reopened. Try Undo again from Sweeps.'
            : 'They are back in their windows.',
          priority: 0,
        });
      });
    } else {
      run('open sweeps', () => browser.tabs.create({ url: sweepsPage() }));
    }
  });
  browser.notifications.onClicked.addListener((notificationId) => {
    if (!notificationId.startsWith(SWEEP_NOTIFICATION)) return;
    void browser.notifications.clear(notificationId);
    run('open sweeps', () => browser.tabs.create({ url: sweepsPage() }));
  });

  // Requests from our own pages (popup, Sweeps page). Other senders are ignored.
  browser.runtime.onMessage.addListener((message, sender, sendResponse) => {
    // Only our own extension pages: content scripts (the P0.7 probe) share the extension
    // id, but their sender URL is the web page.
    const fromOurPage =
      sender.id === browser.runtime.id &&
      (sender.url ?? '').startsWith(browser.runtime.getURL('/'));
    if (!fromOurPage || !isDecayRequest(message)) return false;
    const reply = (response: DecayResponse) => sendResponse(response);
    const fail = (e: unknown) =>
      reply({ ok: false, error: e instanceof Error ? e.message : String(e) });
    if (message.type === 'decay.sweepNow') {
      settings
        .get('mode')
        .then(async (mode) => {
          if (mode !== 'automatic') {
            reply({ ok: false, error: 'Sweeps only close tabs in automatic mode.' });
            return;
          }
          const holdUntil = await settings.get('holdUntil');
          if (holdUntil !== null && holdUntil > Date.now()) {
            const until = new Date(holdUntil).toLocaleTimeString(undefined, {
              hour: 'numeric',
              minute: '2-digit',
            });
            reply({ ok: false, error: `Decay is on hold until ${until}. Nothing is closed.` });
            return;
          }
          const r = await tickAndAnnounce(() => tracker.sweepNow());
          reply({ ok: true, archived: r.sweep?.archived ?? 0, failed: r.sweep?.failed ?? 0 });
        })
        .catch(fail);
    } else {
      tracker
        .undoSweep(message.sweepId)
        .then((r) => {
          if (message.sweepId) {
            void browser.notifications.clear(`${SWEEP_NOTIFICATION}${message.sweepId}`);
          }
          reply({ ok: true, restored: r.restored, failed: r.failed });
        })
        .catch(fail);
    }
    return true; // async response
  });
  browser.tabs.onCreated.addListener((tab) => run('tab created', () => tracker.upsertTab(tab)));
  browser.tabs.onUpdated.addListener((tabId, change, tab) => {
    if (change.audible === true) {
      // Audio keeps a tab young, but autoplay isn't a deliberate choice (no regret).
      run('tab audible', () => tracker.tabUsed(tabId, { deliberate: false }));
    }
    if (
      change.audible !== true &&
      ('url' in change ||
        'title' in change ||
        'pinned' in change ||
        'groupId' in change ||
        change.status === 'complete')
    ) {
      run('tab updated', () => tracker.upsertTab(tab));
    }
  });
  browser.tabs.onActivated.addListener(({ tabId, windowId }) =>
    run('tab activated', () => tracker.tabUsed(tabId, { activatedInWindow: windowId })),
  );
  browser.tabs.onRemoved.addListener((tabId, info) =>
    run('tab removed', () => tracker.tabRemoved(tabId, info.isWindowClosing, info.windowId)),
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
    await tickAndAnnounce(() => tracker.tick());
  });
});
