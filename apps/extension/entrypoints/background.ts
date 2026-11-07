import { db } from '@/lib/db/db';
import type { Item } from '@/lib/db/types';
import { ruleMatches } from '@/lib/policy/match';
import { probeStore, probeTab, syncProbeScripts } from '@/lib/probe/chrome';
import { RuleError, rules } from '@/lib/rules/rules';
import { settings } from '@/lib/settings/settings';
import { protectRuleFor, protectTargetFor } from '@/lib/shadow/actions';
import { type DecayRequest, type DecayResponse, isDecayRequest } from '@/lib/sweep/messages';
import { chromeApi, chromeCommands, detectAppId } from '@/lib/tracker/browser-api';
import { TICK_MS } from '@/lib/tracker/clock';
import { type TickResult, Tracker } from '@/lib/tracker/tracker';

const TICK_ALARM = 'decay-tick';
const SWEEP_NOTIFICATION = 'decay-sweep:';
const PROTECT_NOTIFICATION = 'decay-protect:';
/** The "Always keep site?" offer is removed after a minute. */
const PROTECT_OFFER_MS = 60_000;
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
    prober: { probe: probeTab, load: probeStore.load, save: probeStore.save },
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
    if (notificationId.startsWith(PROTECT_NOTIFICATION)) {
      void browser.notifications.clear(notificationId);
      if (buttonIndex !== 0) return;
      const itemId = notificationId.slice(PROTECT_NOTIFICATION.length);
      run('always keep site', async () => {
        const item = await db.items.get(itemId);
        const input = item ? protectRuleFor(item) : null;
        if (!input) return;
        try {
          await rules.create({ ...input, source: 'quick_action' });
        } catch (e) {
          if (!(e instanceof RuleError && e.code === 'DUPLICATE_RULE')) throw e;
        }
      });
      return;
    }
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
    if (notificationId.startsWith(PROTECT_NOTIFICATION)) {
      void browser.notifications.clear(notificationId);
      return;
    }
    if (!notificationId.startsWith(SWEEP_NOTIFICATION)) return;
    void browser.notifications.clear(notificationId);
    run('open sweeps', () => browser.tabs.create({ url: sweepsPage() }));
  });

  const handle = async (message: DecayRequest): Promise<DecayResponse> => {
    switch (message.type) {
      case 'decay.sweepNow':
        return sweepNow();
      case 'decay.undoSweep': {
        const r = await tracker.undoSweep(message.sweepId);
        if (message.sweepId) {
          void browser.notifications.clear(`${SWEEP_NOTIFICATION}${message.sweepId}`);
        }
        return { ok: true, restored: r.restored, failed: r.failed };
      }
      case 'decay.restore': {
        const r = await tracker.restore(message.ids, { intoWindowId: message.intoWindowId });
        // Don't hold the reply (the popup is closing) for the follow-up notification.
        if (message.ids.length === 1) run('offer protect', () => offerProtect(r.regretted[0]));
        return { ok: true, restored: r.restored, failed: r.failed, skipped: r.skipped };
      }
      case 'decay.restoreBatch': {
        const r = await tracker.restoreBatch(message.itemId);
        return { ok: true, restored: r.restored, failed: r.failed, skipped: r.skipped };
      }
    }
  };

  /** "Sweep now": real sweep in automatic mode, a simulation in shadow mode. */
  const sweepNow = async (): Promise<DecayResponse> => {
    const all = await settings.getAll();
    if (all.mode === 'paused') {
      return { ok: false, error: 'Decay is paused. Resume it to run a sweep.' };
    }
    if (all.holdUntil !== null && all.holdUntil > Date.now()) {
      const until = new Date(all.holdUntil).toLocaleTimeString(undefined, {
        hour: 'numeric',
        minute: '2-digit',
      });
      return { ok: false, error: `Decay is on hold until ${until}. Nothing is closed.` };
    }
    const r = await tickAndAnnounce(() => tracker.sweepNow());
    if (all.mode === 'shadow') return { ok: true, simulated: r.shadowFlagged ?? 0 };
    return { ok: true, archived: r.sweep?.archived ?? 0, failed: r.sweep?.failed ?? 0 };
  };

  /**
   * Restored within 24 h of archiving (regret): offer "Always keep <site>" in one click
   * (PRD S5, Designs/10 follow-up).
   */
  const offerProtect = async (item: Item | undefined) => {
    const target = item ? protectTargetFor(item) : undefined;
    if (!item || target?.kind !== 'site') return;
    // Already protected (e.g. a rule added since it was archived): nothing to offer.
    const all = await db.rules.toArray();
    const match = { appId: item.appId, url: item.url, title: item.title };
    if (all.some((r) => r.enabled && r.action === 'protect' && ruleMatches(r, match))) return;
    const id = `${PROTECT_NOTIFICATION}${item.id}`;
    const minutes = Math.max(
      1,
      Math.round((Date.now() - (item.archivedAt ?? Date.now())) / 60_000),
    );
    const ago = minutes < 60 ? plural(minutes, 'minute') : plural(Math.round(minutes / 60), 'hour');
    await browser.notifications
      .create(id, {
        type: 'basic',
        iconUrl: browser.runtime.getURL('/icon/128.png'),
        title: `Always keep ${target.label} open?`,
        message: `You brought this back ${ago} after it was archived.`,
        buttons: [{ title: `Always keep ${target.label}` }, { title: 'Not now' }],
        priority: 0,
      })
      .catch((e) => console.error('[decay] notification failed', e));
    await browser.alarms.create(`${CLEAR_ALARM}${id}`, { when: Date.now() + PROTECT_OFFER_MS });
  };

  // Requests from our own pages (popup, Sweeps page). Other senders are ignored.
  browser.runtime.onMessage.addListener((message, sender, sendResponse) => {
    // Only our own extension pages: content scripts (the P0.7 probe) share the extension
    // id, but their sender URL is the web page.
    const fromOurPage =
      sender.id === browser.runtime.id &&
      (sender.url ?? '').startsWith(browser.runtime.getURL('/'));
    if (!fromOurPage || !isDecayRequest(message)) return false;
    handle(message)
      .then(sendResponse)
      .catch((e: unknown) =>
        sendResponse({ ok: false, error: e instanceof Error ? e.message : String(e) }),
      );
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

  // Form probe (P0.7): the typing / beforeunload trackers run only while site access is
  // granted. Checked on every worker start (cheap: correct registrations are left alone) and
  // whenever access changes; then old verdicts are dropped so every page is checked again.
  run('probe scripts', () => syncProbeScripts());
  const accessChanged = () =>
    run('probe access changed', async () => {
      await syncProbeScripts();
      await tracker.resetProbes();
      await tickAndAnnounce(() => tracker.tick());
    });
  browser.permissions.onAdded.addListener(accessChanged);
  browser.permissions.onRemoved.addListener(accessChanged);

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
