// Adapter mode (Software Design §4.7, API Design §4.1–4.2): while the Decay app is
// connected, the app owns aging, policy and sweeps, and the extension is its hands and eyes
// in the browser. It reports tabs (a `snapshot`, then events) and carries out the app's
// commands. Every command is acked, and `tabs.close` re-checks safety on its own: an
// active, pinned or audible tab, one holding typed input, one that navigated or was used
// since the app decided, or one on an excluded site is never closed (defence in depth:
// the app may be acting on stale information).
//
// Nothing is reported before the app's first `config` (its excluded sites, F9): the
// snapshot follows that `config`, and later ones answer `resync`.

import { domainMatches, hostOf } from '../policy/match';
import { isRestorableUrl } from '../policy/policy';
import type { ProbeVerdict } from '../probe/probe';
import type { TabInfo } from '../tracker/tabs';
import {
  type Ack,
  type ConfigMsg,
  type Envelope,
  envelope,
  type GroupInfoMsg,
  kinds,
  type ProbeResult,
  type Snapshot,
  type TabInfoMsg,
  type TabsClose,
  type TabsRestore,
} from './protocol';

/** A browser tab as `chrome.tabs` reports it (the fields the adapter uses). */
export interface ChromeTab extends TabInfo {
  lastAccessed?: number;
}

export interface ChromeWindow {
  id?: number;
  focused: boolean;
  state?: string;
  type?: string;
  incognito: boolean;
}

export interface ChromeGroup {
  id: number;
  title?: string;
  color: string;
  windowId: number;
}

/** The browser operations the adapter needs; tests supply fakes. */
export interface AdapterDeps {
  queryTabs(): Promise<ChromeTab[]>;
  getWindows(): Promise<ChromeWindow[]>;
  getGroups(): Promise<ChromeGroup[]>;
  /** `undefined` when the tab is gone. */
  getTab(tabId: number): Promise<ChromeTab | undefined>;
  removeTab(tabId: number): Promise<void>;
  createTab(opts: {
    windowId: number;
    url: string;
    index?: number;
    active: boolean;
  }): Promise<ChromeTab>;
  /** A new window showing `url`; returns its first tab. */
  createWindow(url: string, focused: boolean): Promise<ChromeTab>;
  groupTabs(
    tabIds: number[],
    windowId: number,
    group: { title: string; color: string },
  ): Promise<void>;
  focusTab(tab: ChromeTab): Promise<void>;
  probe(tab: ChromeTab): Promise<ProbeVerdict>;
  /** Sends a message to the app (false when not connected). */
  send(env: Envelope): boolean;
  /** Still connected: a close loop stops as soon as the link drops. */
  isConnected(): boolean;
  /** This browser session's id (window ids only mean something within one). */
  session(): string;
  /** `chrome-extension://<own id>/`: Decay's own pages are never reported or closed. */
  ownOrigin: string;
  now(): number;
}

/** A tab used this recently is in use: the app's decision to close it is out of date. */
export const RECENT_USE_MS = 60_000;

type SkipReason =
  | 'active'
  | 'pinned'
  | 'audible'
  | 'formInput'
  | 'changed'
  | 'inUse'
  | 'excluded'
  | 'disconnected'
  | 'error'
  /** The app counted on the page being checked, and now it can't be. */
  | 'unchecked';

/** `tabs.restore` window keys: `<session>:<windowId>`. */
export const windowKey = (session: string, windowId: number) => `${session}:${windowId}`;

export class Adapter {
  private excludedDomains: string[] = [];
  /** The app's first `config` arrived: reporting may start. */
  private ready = false;
  /** Tabs the app knows about (reported, not removed since). */
  private reported = new Set<number>();
  /**
   * Tabs Decay closed: their `tab.removed` isn't the user's doing. Kept across reconnects:
   * a removal can be reported on the next connection, and the app must archive that tab,
   * not forget it.
   */
  private closedByDecay = new Set<number>();
  /** Bumped on every new connection: a close loop stops once its connection is gone. */
  private generation = 0;

  constructor(private readonly deps: AdapterDeps) {}

  /** A new connection: nothing is reported until the app's `config`. */
  reset(): void {
    this.ready = false;
    this.reported.clear();
    this.generation++;
  }

  /** Excluded sites (F9), incognito tabs and Decay's own pages are never reported. */
  private reportable(tab: ChromeTab): boolean {
    if (tab.incognito) return false;
    const url = tab.url || tab.pendingUrl || '';
    if (url.startsWith(this.deps.ownOrigin)) return false;
    const host = hostOf(url);
    return !this.excludedDomains.some((d) => domainMatches(host, d));
  }

  static tabMsg(tab: ChromeTab): TabInfoMsg {
    return {
      tabId: tab.id ?? -1,
      windowId: tab.windowId,
      index: tab.index,
      url: tab.url || tab.pendingUrl || '',
      title: tab.title ?? '',
      ...(tab.favIconUrl ? { favIconUrl: tab.favIconUrl } : {}),
      pinned: tab.pinned,
      audible: tab.audible === true,
      discarded: tab.discarded === true,
      groupId: tab.groupId ?? -1,
      active: tab.active,
      status: tab.status ?? 'complete',
      ...(tab.lastAccessed !== undefined ? { lastAccessed: tab.lastAccessed } : {}),
    };
  }

  /** Every normal window with its reportable tabs, and the tab groups. */
  async snapshot(): Promise<Snapshot> {
    const [tabs, windows, groups] = await Promise.all([
      this.deps.queryTabs(),
      this.deps.getWindows(),
      this.deps.getGroups(),
    ]);
    const normal = windows.filter(
      (w) => w.id !== undefined && !w.incognito && (w.type ?? 'normal') === 'normal',
    );
    const included = tabs.filter(
      (t) => normal.some((w) => w.id === t.windowId) && this.reportable(t),
    );
    this.reported = new Set(
      included.map((t) => t.id).filter((id): id is number => id !== undefined),
    );
    return {
      windows: normal.map((w) => ({
        id: w.id as number,
        focused: w.focused,
        state: w.state ?? 'normal',
        incognito: false as const,
        tabs: included.filter((t) => t.windowId === w.id).map(Adapter.tabMsg),
      })),
      groups: groups
        .filter((g) => normal.some((w) => w.id === g.windowId))
        .map(
          (g): GroupInfoMsg => ({
            id: g.id,
            title: g.title ?? '',
            color: g.color,
            windowId: g.windowId,
          }),
        ),
    };
  }

  sendSnapshot = async (): Promise<boolean> => {
    if (!this.ready) return false;
    return this.deps.send(envelope(kinds.snapshot, await this.snapshot()));
  };

  // --- Events (extension → app) ----------------------------------------------------

  tabCreated(tab: ChromeTab): void {
    if (!this.ready || tab.id === undefined || !this.reportable(tab)) return;
    this.reported.add(tab.id);
    this.deps.send(envelope(kinds.tabCreated, Adapter.tabMsg(tab)));
  }

  /**
   * A tab that moves onto an excluded site is reported as gone (once); one that leaves it
   * is reported as new, with everything the app needs to know about it.
   */
  tabUpdated(tab: ChromeTab, changes: Record<string, unknown>): void {
    if (!this.ready || tab.id === undefined) return;
    const known = this.reported.has(tab.id);
    if (!this.reportable(tab)) {
      if (known) {
        this.reported.delete(tab.id);
        this.deps.send(envelope(kinds.tabRemoved, { tabId: tab.id, byUser: true }));
      }
      return;
    }
    if (!known) {
      this.tabCreated(tab);
      return;
    }
    this.deps.send(envelope(kinds.tabUpdated, { tabId: tab.id, changes }));
  }

  tabRemoved(tabId: number): void {
    const byDecay = this.closedByDecay.delete(tabId);
    if (!this.ready || !this.reported.delete(tabId)) return;
    this.deps.send(envelope(kinds.tabRemoved, { tabId, byUser: !byDecay }));
  }

  tabMoved(tabId: number, windowId: number, index: number): void {
    if (this.ready && this.reported.has(tabId)) {
      this.deps.send(envelope(kinds.tabMoved, { tabId, windowId, index }));
    }
  }

  tabUsed(
    tabId: number,
    windowId: number,
    signal: 'activated' | 'visible' | 'audible' | 'typed',
    at: number,
  ): void {
    if (this.ready && this.reported.has(tabId)) {
      this.deps.send(envelope(kinds.tabUsed, { tabId, windowId, at, signal }));
    }
  }

  windowFocus(windowId: number): void {
    if (this.ready) this.deps.send(envelope(kinds.windowFocus, { windowId }));
  }

  // --- Commands (app → extension) --------------------------------------------------

  /** Handles a command from the app; every command is acked. */
  async handle(env: Envelope): Promise<void> {
    const ack = (payload: Ack) => this.deps.send(envelope(kinds.ack, payload, env.id));
    try {
      switch (env.type) {
        case kinds.probe:
          ack({ ok: true, result: await this.probe(env.payload) });
          return;
        case kinds.tabsClose:
          ack(await this.close(env.payload as TabsClose, this.deps.now()));
          return;
        case kinds.tabsRestore:
          ack(await this.restore(env.payload as TabsRestore));
          return;
        case kinds.tabsFocus: {
          const tabId = (env.payload as { tabId?: unknown } | null)?.tabId;
          const tab = typeof tabId === 'number' ? await this.deps.getTab(tabId) : undefined;
          if (!tab) {
            ack({ ok: false, error: { code: 'NOT_FOUND', message: 'tab is gone' } });
            return;
          }
          await this.deps.focusTab(tab);
          ack({ ok: true });
          return;
        }
        case kinds.resync:
          ack({ ok: await this.sendSnapshot() });
          return;
        case kinds.config: {
          const config = (env.payload ?? {}) as Partial<ConfigMsg>;
          this.excludedDomains = Array.isArray(config.excludedDomains)
            ? config.excludedDomains.filter((d): d is string => typeof d === 'string')
            : [];
          ack({ ok: true });
          // The first config opens reporting: the app learns about the tabs now. Later
          // ones may change the exclusions, so the app gets a fresh snapshot.
          this.ready = true;
          await this.sendSnapshot();
          return;
        }
        default:
          ack({
            ok: false,
            error: { code: 'BAD_MESSAGE', message: `unknown command ${env.type}` },
          });
      }
    } catch (e) {
      ack({
        ok: false,
        error: { code: 'BAD_MESSAGE', message: e instanceof Error ? e.message : String(e) },
      });
    }
  }

  /** `probe`: the verdicts are the ack's result (`results`, and `missing` tab ids). */
  private async probe(payload: unknown): Promise<{ results: ProbeResult[]; missing: number[] }> {
    const ids = (payload as { tabIds?: unknown } | null)?.tabIds;
    const tabIds = Array.isArray(ids) ? ids.filter((n): n is number => Number.isInteger(n)) : [];
    const results: ProbeResult[] = [];
    const missing: number[] = [];
    for (const tabId of tabIds) {
      const tab = await this.deps.getTab(tabId);
      if (!tab) {
        missing.push(tabId);
        continue;
      }
      const verdict = await this.deps.probe(tab);
      results.push({
        tabId,
        dirtyForm: verdict === 'dirty',
        beforeUnload: false,
        uploading: false,
        unknown: verdict === 'unknown',
      });
    }
    return { results, missing };
  }

  /** Why `tab` mustn't be closed now, or null. */
  private blocker(
    tab: ChromeTab,
    expectedUrl: string | undefined,
    since: number,
  ): SkipReason | null {
    if (tab.active) return 'active';
    if (tab.pinned) return 'pinned';
    if (tab.audible) return 'audible';
    if (!this.reportable(tab)) return 'excluded';
    const url = tab.url || tab.pendingUrl || '';
    if (expectedUrl !== undefined && url !== expectedUrl) return 'changed';
    if (tab.lastAccessed !== undefined && tab.lastAccessed > since - RECENT_USE_MS) return 'inUse';
    return null;
  }

  /**
   * `tabs.close`: each tab is checked, probed for typed input, read again and checked
   * again right before closing. A tab that's gone counts as closed (NOT_FOUND is success
   * for a close). Any tab skipped for safety makes the ack `UNSAFE`, with per-tab reasons;
   * tabs whose page couldn't be checked are closed and listed in `unchecked` (the app's
   * policy already waited for them: the 2× rule), unless the app listed them in
   * `requireChecked` (it decided on an earlier clean check): those are skipped. The loop
   * stops if the link drops or a new connection replaces this one.
   */
  private async close(cmd: TabsClose, receivedAt: number): Promise<Ack> {
    // The loop belongs to this connection: after a reconnect the app has moved on (it may
    // already have put these tabs back), so nothing more is closed.
    const generation = this.generation;
    const linked = () => this.deps.isConnected() && this.generation === generation;
    const closed: number[] = [];
    const unchecked: number[] = [];
    const skipped: { tabId: number; reason: SkipReason }[] = [];
    const urls = (cmd as { urls?: Record<string, string> } | null)?.urls ?? {};
    const requireChecked = new Set(
      Array.isArray(cmd?.requireChecked)
        ? cmd.requireChecked.filter((n) => Number.isInteger(n))
        : [],
    );
    const tabIds = Array.isArray(cmd?.tabIds) ? cmd.tabIds.filter((n) => Number.isInteger(n)) : [];
    for (const tabId of tabIds) {
      if (!linked()) {
        skipped.push({ tabId, reason: 'disconnected' });
        continue;
      }
      const expected = typeof urls[String(tabId)] === 'string' ? urls[String(tabId)] : undefined;
      let tab = await this.deps.getTab(tabId);
      if (!tab) {
        closed.push(tabId);
        continue;
      }
      const early = this.blocker(tab, expected, receivedAt);
      if (early) {
        skipped.push({ tabId, reason: early });
        continue;
      }
      const verdict = await this.deps.probe(tab);
      if (verdict === 'dirty') {
        skipped.push({ tabId, reason: 'formInput' });
        continue;
      }
      // The app decided at the normal threshold because the page was checked clean before;
      // it can't be checked now, so it waits (the 2× rule) rather than closing unchecked.
      if (verdict === 'unknown' && requireChecked.has(tabId)) {
        skipped.push({ tabId, reason: 'unchecked' });
        continue;
      }
      // The check took a moment: look again before closing.
      tab = await this.deps.getTab(tabId);
      if (!tab) {
        closed.push(tabId);
        continue;
      }
      const late = this.blocker(tab, expected, receivedAt) ?? (linked() ? null : 'disconnected');
      if (late) {
        skipped.push({ tabId, reason: late });
        continue;
      }
      try {
        this.closedByDecay.add(tabId);
        await this.deps.removeTab(tabId);
        closed.push(tabId);
        if (verdict === 'unknown') unchecked.push(tabId);
      } catch {
        this.closedByDecay.delete(tabId);
        skipped.push({ tabId, reason: 'error' });
      }
    }
    const safety = skipped.some((s) => s.reason !== 'error');
    return {
      ok: skipped.length === 0,
      ...(safety ? { error: { code: 'UNSAFE', message: 'some tabs are in use or changed' } } : {}),
      result: { closed, skipped, unchecked },
    };
  }

  /**
   * `tabs.restore`: items with the same `windowKey` go to the same window — the normal
   * window with that id if it's still open in this browser session, else one new window.
   * `newWindow` puts everything into one new window. Tabs with a group are grouped again.
   * Only URLs Decay can reopen exactly are opened (no `javascript:`, `data:`, `file:` …).
   */
  private async restore(cmd: TabsRestore): Promise<Ack> {
    const windows = await this.deps.getWindows();
    const session = this.deps.session();
    const live = new Set(
      windows
        .filter((w) => w.id !== undefined && !w.incognito && (w.type ?? 'normal') === 'normal')
        .map((w) => windowKey(session, w.id as number)),
    );
    const target = new Map<string, number>();
    const grouped = new Map<
      string,
      { windowId: number; tabIds: number[]; group: { title: string; color: string } }
    >();
    const restored: { itemId: string; tabId: number }[] = [];
    const failed: string[] = [];
    for (const item of Array.isArray(cmd?.items) ? cmd.items : []) {
      const url = typeof item?.url === 'string' ? item.url.trim() : '';
      if (!url || !isRestorableUrl(url) || url.startsWith(this.deps.ownOrigin)) {
        failed.push(String(item?.itemId));
        continue;
      }
      const key = cmd.newWindow ? '__new__' : (item.windowKey ?? '__new__');
      try {
        let windowId = target.get(key);
        let tab: ChromeTab;
        if (windowId === undefined && !cmd.newWindow && live.has(key)) {
          windowId = Number(key.slice(key.lastIndexOf(':') + 1));
        }
        if (windowId === undefined) {
          tab = await this.deps.createWindow(url, item.active === true);
          windowId = tab.windowId;
        } else {
          tab = await this.deps.createTab({
            windowId,
            url,
            ...(item.index !== undefined ? { index: item.index } : {}),
            active: item.active === true,
          });
        }
        target.set(key, windowId);
        if (tab.id !== undefined) {
          restored.push({ itemId: item.itemId, tabId: tab.id });
          if (item.group?.title !== undefined) {
            const gk = `${windowId}:${item.group.title}:${item.group.color}`;
            const g = grouped.get(gk) ?? { windowId, tabIds: [], group: item.group };
            g.tabIds.push(tab.id);
            grouped.set(gk, g);
          }
        }
      } catch {
        failed.push(item.itemId);
      }
    }
    for (const g of grouped.values()) {
      await this.deps.groupTabs(g.tabIds, g.windowId, g.group).catch(() => undefined);
    }
    return { ok: failed.length === 0, result: { restored, failed } };
  }
}
