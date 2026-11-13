import { Adapter } from '@/lib/bridge/adapter';
import {
  adapterDeps,
  browserKind,
  browserSession,
  linkEnabled,
  profileId,
  readLinkStatus,
  setLinkEnabled,
  writeLinkStatus,
} from '@/lib/bridge/chrome';
import { Bridge, type BridgeState } from '@/lib/bridge/connection';
import { SYNC_KEY, type SyncRecord, syncArchive } from '@/lib/bridge/migrate';
import { localEngineMayRun } from '@/lib/bridge/ownership';
import {
  type Ack,
  type ArchiveSearchResult,
  envelope,
  kinds,
  type RestoreSummary,
} from '@/lib/bridge/protocol';
import { db } from '@/lib/db/db';
import type { Item } from '@/lib/db/types';
import { recordSearchToRestore } from '@/lib/metrics/metrics';
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
/** Reconnects to the Decay app while the link is on (Software Design §4.7 watchdog). */
const APP_WATCHDOG_ALARM = 'decay-app-watchdog';
const SWEEP_NOTIFICATION = 'decay-sweep:';
const PROTECT_NOTIFICATION = 'decay-protect:';
/** The "Always keep site?" offer is removed after a minute. */
const PROTECT_OFFER_MS = 60_000;
/** Alarm that removes an Undo toast; named after its notification id. */
const CLEAR_ALARM = 'decay-clear:';
/** The Undo toast lives 30 s (PRD F7); Sweeps history keeps undo available after that. */
const UNDO_TOAST_MS = 30_000;
/** How long the popup waits for the app (a restore waits for the browser's own ack). */
const APP_REQUEST_MS = 30_000;
const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`;

export default defineBackground(() => {
  // --- Who closes tabs: this extension or the Decay app (P1.4) ----------------------
  // While the app is connected it owns aging, policy and sweeps: the extension reports
  // tabs and carries out its commands (adapter mode). The local tracker keeps following
  // the tabs (so its ages are current if it has to take over) but is held in "paused":
  // it never plans a sweep, and every close it makes checks ownership first, so the two
  // never both close tabs. While the link is on, a fresh worker waits for the first
  // connection attempt to fail before the local engine may close anything.
  let bridge: Bridge;
  let linkOn = false;
  let linkSettled = false;
  const localMayClose = () => localEngineMayRun(linkOn, bridge.state, linkSettled);
  const linkReady = linkEnabled().then((on) => {
    linkOn = on;
  });
  const guardedCommands = {
    ...chromeCommands,
    async removeTab(tabId: number) {
      // Refused closes count as failed: the item goes back to its earlier state.
      if (!localMayClose()) throw new Error('The Decay app manages tabs now');
      await chromeCommands.removeTab(tabId);
    },
  };

  const tracker = new Tracker({
    db,
    api: chromeApi,
    appId: detectAppId(),
    commands: guardedCommands,
    prober: { probe: probeTab, load: probeStore.load, save: probeStore.save },
    ownOrigin: browser.runtime.getURL('/'),
    getIdleCutoffS: () => settings.get('idleCutoffS'),
    getPolicy: async () => {
      const [all] = await Promise.all([settings.getAll(), linkReady]);
      return {
        // "paused" while the app is (or may be) in charge: time is still counted.
        mode: localMayClose() ? all.mode : 'paused',
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

  const appId = detectAppId();
  const version = browser.runtime.getManifest().version;
  const local = {
    get: async (key: string) => (await browser.storage.local.get(key))[key],
    set: async (key: string, value: unknown) => browser.storage.local.set({ [key]: value }),
  };
  const adapter = new Adapter(
    adapterDeps(
      (env) => bridge.send(env),
      () => bridge.state === 'connected',
    ),
  );
  const adapterMode = () => bridge.state === 'connected';

  /** Sends what changed in the archive (one sync at a time); failures go to the status. */
  let syncing: Promise<unknown> | null = null;
  const syncToApp = () => {
    syncing ??= (async () => {
      const result = await syncArchive(bridge, db, local, Date.now);
      const status = await readLinkStatus();
      if (status) {
        await writeLinkStatus(
          result.ok
            ? { ...status, migratedAt: result.record.firstAt, detail: undefined }
            : { ...status, detail: `Couldn't send the archive to the app: ${result.error}` },
        );
      }
    })().finally(() => {
      syncing = null;
    });
    return syncing;
  };

  let previous: BridgeState = 'off';
  const linkStateChanged = (state: BridgeState, detail?: string) => {
    const was = previous;
    previous = state;
    if (state !== 'connecting' && state !== 'connected') linkSettled = true;
    if (state === 'connected') adapter.reset();
    run('app link status', async () => {
      const sync = (await local.get(SYNC_KEY)) as SyncRecord | undefined;
      await writeLinkStatus({
        state,
        ...(detail ? { detail } : {}),
        at: Date.now(),
        version,
        ...(sync?.firstAt ? { migratedAt: sync.firstAt } : {}),
      });
      if (state === 'connected') {
        // The snapshot follows the app's first `config` (see Adapter).
        await syncToApp();
      } else if (was === 'connected') {
        // Standalone again: the local tracker takes over, announcing any sweep (Undo).
        await tickAndAnnounce(() => tracker.tick());
      }
    });
  };
  bridge = new Bridge({
    connectNative: (name) => browser.runtime.connectNative(name),
    hello: async () => ({
      extVersion: version,
      browser: browserKind(appId),
      profileId: await profileId(),
      capabilities: ['tabGroups', 'probe'],
      session: await browserSession(),
    }),
    onMessage: (env) => run(`app ${env.type}`, () => adapter.handle(env)),
    onState: linkStateChanged,
    now: Date.now,
    setTimer: (fn, ms) => setTimeout(fn, ms),
    clearTimer: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
    setRepeating: (fn, ms) => setInterval(fn, ms),
    clearRepeating: (h) => clearInterval(h as ReturnType<typeof setInterval>),
  });
  /** `launch`: only at browser start-up or on a user action (never on a reconnect). */
  const connectApp = (launch: boolean) =>
    run('connect app', async () => {
      await linkReady;
      if (!linkOn) return;
      // An extension the app called outdated waits for an update instead of retrying.
      const status = await readLinkStatus();
      if (!launch && status?.state === 'outdated' && status.version === version) {
        linkSettled = true;
        return;
      }
      await bridge.connect({ launch });
    });
  const turnLinkOn = async () => {
    linkOn = true;
    await setLinkEnabled(true);
    await browser.alarms.create(APP_WATCHDOG_ALARM, { periodInMinutes: 1 });
    await bridge.connect({ launch: true });
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
    // The tick runs in both modes: in adapter mode the tracker only keeps its view of the
    // tabs current ("paused": it never sweeps).
    if (alarm.name === TICK_ALARM) run('tick', () => tickAndAnnounce(() => tracker.tick()));
    if (alarm.name === APP_WATCHDOG_ALARM) {
      if (bridge.state === 'connected') run('sync archive', syncToApp);
      else connectApp(false);
    }
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

  /**
   * A request to the app (adapter mode, P1.5): the archive lives there while it's
   * connected. Throws with the app's reason when it refuses.
   */
  const askApp = async <T>(type: string, payload: unknown): Promise<T> => {
    const reply = await bridge.request(envelope(type, payload), APP_REQUEST_MS);
    const ack = reply.payload as Ack | null;
    if (reply.type !== kinds.ack || ack?.ok !== true) {
      const reason = ack?.error?.message;
      throw new Error(
        reason ? `The Decay app said: ${reason}.` : "The Decay app couldn't do that.",
      );
    }
    return ack.result as T;
  };

  const handle = async (message: DecayRequest): Promise<DecayResponse> => {
    switch (message.type) {
      case 'decay.search': {
        if (!adapterMode()) return { ok: false, error: 'The Decay app is not connected.' };
        const search = await askApp<ArchiveSearchResult>(kinds.archiveSearch, {
          query: message.query,
          limit: message.limit ?? 50,
          utcOffsetMinutes: -new Date().getTimezoneOffset(),
        });
        return { ok: true, search };
      }
      case 'decay.app.connect':
        await turnLinkOn();
        return { ok: true };
      case 'decay.app.disconnect': {
        linkOn = false;
        await setLinkEnabled(false);
        bridge.disconnect();
        await browser.alarms.clear(APP_WATCHDOG_ALARM);
        return { ok: true };
      }
      case 'decay.sweepNow':
        await linkReady;
        if (!localMayClose()) {
          return {
            ok: false,
            error:
              bridge.state === 'connected'
                ? 'The Decay app runs sweeps now.'
                : 'Connecting to the Decay app. Try again in a moment.',
          };
        }
        return sweepNow();
      case 'decay.undoSweep': {
        // Sweeps the app ran live there; a Phase 0 sweep (still in this browser) is
        // undone here, as before.
        const local = message.sweepId ? await db.sweeps.get(message.sweepId) : undefined;
        if (adapterMode() && !local) {
          const r = await askApp<RestoreSummary>(
            kinds.archiveUndo,
            message.sweepId ? { sweepId: message.sweepId } : {},
          );
          return { ok: true, restored: r.restored, failed: r.failed };
        }
        const r = await tracker.undoSweep(message.sweepId);
        if (message.sweepId) {
          void browser.notifications.clear(`${SWEEP_NOTIFICATION}${message.sweepId}`);
        }
        return { ok: true, restored: r.restored, failed: r.failed };
      }
      case 'decay.restore': {
        if (adapterMode()) {
          const r = await askApp<RestoreSummary>(kinds.archiveRestore, {
            itemIds: message.ids,
            ...(message.intoWindowId !== undefined ? { intoWindowId: message.intoWindowId } : {}),
          });
          if (r.restored) await noteSearch(message.searchMs);
          return { ok: true, restored: r.restored, failed: r.failed, skipped: r.skipped };
        }
        const r = await tracker.restore(message.ids, { intoWindowId: message.intoWindowId });
        // Don't hold the reply (the popup is closing) for the follow-up notification.
        if (message.ids.length === 1) run('offer protect', () => offerProtect(r.regretted[0]));
        if (r.restored) await noteSearch(message.searchMs);
        return { ok: true, restored: r.restored, failed: r.failed, skipped: r.skipped };
      }
      case 'decay.restoreBatch': {
        if (adapterMode()) {
          const r = await askApp<RestoreSummary>(kinds.archiveRestoreBatch, {
            itemId: message.itemId,
          });
          if (r.restored) await noteSearch(message.searchMs);
          return { ok: true, restored: r.restored, failed: r.failed, skipped: r.skipped };
        }
        const r = await tracker.restoreBatch(message.itemId);
        if (r.restored) await noteSearch(message.searchMs);
        return { ok: true, restored: r.restored, failed: r.failed, skipped: r.skipped };
      }
    }
  };

  /** Popup search → restore time (P0.8); a metrics write failure never fails the restore. */
  const noteSearch = async (ms: number | undefined) => {
    if (ms !== undefined) await recordSearchToRestore(db, ms).catch(() => undefined);
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
  // Tab events go to the app in adapter mode, else to the local tracker.
  // Tab events always reach the local tracker (its view stays current); in adapter mode
  // they're also reported to the app.
  browser.tabs.onCreated.addListener((tab) => {
    if (adapterMode()) adapter.tabCreated(tab);
    run('tab created', () => tracker.upsertTab(tab));
  });
  browser.tabs.onUpdated.addListener((tabId, change, tab) => {
    if (adapterMode()) {
      if (change.audible === true) adapter.tabUsed(tabId, tab.windowId, 'audible', Date.now());
      else adapter.tabUpdated(tab, { ...change });
    }
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
  browser.tabs.onActivated.addListener(({ tabId, windowId }) => {
    if (adapterMode()) adapter.tabUsed(tabId, windowId, 'activated', Date.now());
    run('tab activated', () => tracker.tabUsed(tabId, { activatedInWindow: windowId }));
  });
  browser.tabs.onRemoved.addListener((tabId, info) => {
    if (adapterMode()) adapter.tabRemoved(tabId);
    run('tab removed', () => tracker.tabRemoved(tabId, info.isWindowClosing, info.windowId));
  });
  browser.tabs.onMoved.addListener((tabId, info) => {
    if (adapterMode()) adapter.tabMoved(tabId, info.windowId, info.toIndex);
    run('tab moved', () => tracker.refreshTab(tabId));
  });
  browser.tabs.onAttached.addListener((tabId, info) => {
    if (adapterMode()) adapter.tabMoved(tabId, info.newWindowId, info.newPosition);
    run('tab attached', () => tracker.refreshTab(tabId));
  });
  browser.tabs.onReplaced.addListener((added, removed) => {
    if (adapterMode()) {
      adapter.tabRemoved(removed);
      run('tab replaced (app)', async () => {
        const tab = await browser.tabs.get(added).catch(() => undefined);
        if (tab) adapter.tabCreated(tab);
      });
    }
    run('tab replaced', () => tracker.tabReplaced(added, removed));
  });
  browser.tabGroups?.onUpdated.addListener((group) => {
    // Group titles and colours travel in the snapshot.
    if (adapterMode()) run('group updated (app)', () => adapter.sendSnapshot());
    run('group updated', () => tracker.groupUpdated(group.id));
  });
  browser.windows.onFocusChanged.addListener((windowId) => {
    if (adapterMode()) adapter.windowFocus(windowId);
    run('window focus', () => tracker.windowFocused(windowId));
  });

  // Browser start-up may start the app (the user expects Decay to be running); a worker
  // restart only reconnects.
  browser.runtime.onStartup.addListener(() => connectApp(true));
  connectApp(false);
  run('app link watchdog', async () => {
    await linkReady;
    if (!linkOn) return;
    if (!(await browser.alarms.get(APP_WATCHDOG_ALARM))) {
      await browser.alarms.create(APP_WATCHDOG_ALARM, { periodInMinutes: 1 });
    }
    // A stale "connected" from a previous worker isn't true any more.
    const status = await readLinkStatus();
    if (status?.state === 'connected' && bridge.state !== 'connected') {
      await writeLinkStatus({ ...status, state: 'connecting', at: Date.now() });
    }
  });

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
  // Granting the app-link permission (the popup may close while Chrome asks) turns the
  // link on here, so it doesn't depend on the popup staying open.
  browser.permissions.onAdded.addListener((added) => {
    if (added.permissions?.includes('nativeMessaging')) run('connect app', turnLinkOn);
  });
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
