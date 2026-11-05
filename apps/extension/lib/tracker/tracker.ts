import { type DecayDB, isLive, LIVE_STATES } from '../db/db';
import type { Item, ItemEvent, Rule } from '../db/types';
import { isExcluded, type PolicyContext } from '../policy/policy';
import { computeCredit, type IdleState } from './clock';
import { applyLifecycle } from './lifecycle';
import { applyTick, reconcile } from './reconcile';
import {
  applyTabFields,
  type GroupInfo,
  inUseTabIds,
  markUsed,
  newItemFromTab,
  type TabInfo,
  tabUrl,
  type WindowInfo,
} from './tabs';

/** Everything the tracker needs from the browser, so tests can supply a fake. */
export interface BrowserApi {
  queryTabs(): Promise<TabInfo[]>;
  getTab(tabId: number): Promise<TabInfo | undefined>;
  getActiveTab(windowId: number): Promise<TabInfo | undefined>;
  getWindows(): Promise<WindowInfo[]>;
  getGroups(): Promise<GroupInfo[]>;
  queryIdle(cutoffS: number): Promise<IdleState>;
  /** Last tick time, kept in `storage.session` so it resets with the browser session. */
  getLastTickAt(): Promise<number | null>;
  setLastTickAt(at: number): Promise<void>;
}

export interface TrackerDeps {
  db: DecayDB;
  api: BrowserApi;
  appId: string;
  getIdleCutoffS: () => Promise<number>;
  /** Current mode, hold, thresholds and rules (read fresh for every tick and handler). */
  /** Current mode, hold and thresholds. Rules are read from the DB inside each transaction. */
  getPolicy: () => Promise<Omit<PolicyContext, 'now' | 'rules'>>;
  now?: () => number;
  /**
   * How long an activation must last to count as use. Browsers briefly activate tabs
   * while opening or restoring a window; those flickers must not reset ages.
   */
  activationSettleMs?: number;
}

export interface TickResult {
  creditS: number;
  liveCount: number;
  written: number;
  deleted: number;
}

/** How long handler writes are remembered to protect them from older tick snapshots. */
const TOUCH_MEMORY_MS = 5 * 60_000;

/**
 * Tab tracker + aging clock (P0.2, PRD F1).
 *
 * `tick()` (every 30 s) is the source of truth: it reconciles the stored live items
 * with the open tabs and credits active time. The tab event handlers keep things
 * responsive between ticks (new tabs appear and activations reset age at once).
 *
 * Handlers and ticks can interleave: a tick reads the tabs *before* its DB transaction
 * (browser APIs can't be awaited inside one). Handlers record which tab ids they wrote,
 * and the tick skips those ids if they were touched after its snapshot.
 */
export class Tracker {
  private readonly db: DecayDB;
  private readonly api: BrowserApi;
  private readonly appId: string;
  private readonly getIdleCutoffS: () => Promise<number>;
  private readonly getPolicy: () => Promise<Omit<PolicyContext, 'now' | 'rules'>>;
  private readonly now: () => number;
  private tickChain: Promise<unknown> = Promise.resolve();
  /** tab id → last time a handler wrote or removed it. In memory: same worker as ticks. */
  private readonly touched = new Map<string, number>();
  /** item id → when its tab was first found missing. */
  private missingSince = new Map<string, number>();
  /** tab id → when it was closed; late `onCreated`/`onUpdated` handlers must not revive it. */
  private readonly removed = new Map<string, number>();
  private readonly activationSettleMs: number;

  constructor(deps: TrackerDeps) {
    this.db = deps.db;
    this.api = deps.api;
    this.appId = deps.appId;
    this.getIdleCutoffS = deps.getIdleCutoffS;
    this.getPolicy = deps.getPolicy;
    this.now = deps.now ?? Date.now;
    this.activationSettleMs = deps.activationSettleMs ?? 500;
  }

  /**
   * Ticks are serialized: two overlapping ticks (worker start + alarm) would both read
   * the same `lastTickAt` and credit the same interval twice.
   */
  tick(): Promise<TickResult> {
    const next = this.tickChain.then(() => this.runTick());
    this.tickChain = next.catch(() => undefined);
    return next;
  }

  private async runTick(): Promise<TickResult> {
    const [cutoff, policy] = await Promise.all([this.getIdleCutoffS(), this.getPolicy()]);
    const snapshotAt = this.now();
    const [allTabs, windows, groupList, idleState, lastTickAt] = await Promise.all([
      this.api.queryTabs(),
      this.api.getWindows(),
      this.api.getGroups(),
      this.api.queryIdle(cutoff),
      this.api.getLastTickAt(),
    ]);
    const visible = allTabs.filter((t) => !t.incognito);
    const groups = new Map(groupList.map((g) => [g.id, g]));
    const creditS = computeCredit({
      elapsedMs: lastTickAt === null ? null : snapshotAt - lastTickAt,
      idleState,
      anyAudible: visible.some((t) => t.audible),
    });

    const tables = [this.db.items, this.db.events, this.db.rules];
    const result = await this.db.transaction('rw', tables, async () => {
      // Rules are read inside the transaction, so a rule created concurrently (and its
      // purge) is either fully before or fully after this tick — never half-applied.
      const rules = await this.db.rules.toArray();
      const live = await this.liveItems();
      const liveIds = new Set(live.map((i) => i.externalId));
      // Excluded sites (F9) are never stored: to the tracker they don't exist.
      const tabs = visible.filter(
        (t) => !this.excluded(t, rules) && !this.waitForTitle(t, rules, liveIds),
      );
      const tabsById = new Map(tabs.map((t) => [String(t.id), t]));
      const inUse = inUseTabIds(tabs, windows);
      const now = this.now();
      const r = reconcile({
        live,
        tabs,
        groups,
        appId: this.appId,
        now,
        snapshotAt,
        skipTabIds: this.touchedSince(snapshotAt),
        missingSince: this.missingSince,
      });
      const tickEvents = applyTick(r.items, inUse, creditS, now, r.dirty);
      applyLifecycle(r.items, tabsById, { ...policy, rules, now }, r.dirty, tickEvents);
      await this.deleteItems(r.deleteIds);
      if (r.dirty.size) await this.db.items.bulkPut([...r.dirty]);
      await this.addEvents([...r.events, ...tickEvents]);
      return { r, liveCount: r.items.length, written: r.dirty.size };
    });
    this.missingSince = result.r.missingSince;
    // Only after a successful write, so a failed tick's time is credited by the next one.
    await this.api.setLastTickAt(snapshotAt);
    this.pruneTouched();
    return {
      creditS,
      liveCount: result.liveCount,
      written: result.written,
      deleted: result.r.deleteIds.length,
    };
  }

  /**
   * `tabs.onCreated` / `onUpdated` / `onMoved` / `onAttached`, and group changes.
   * A tab id we don't know first tries to reclaim an item whose tab is gone but has
   * the same URL ("reopen closed window", session restore racing the first tick).
   */
  async upsertTab(tab: TabInfo, opts: { used?: boolean } = {}): Promise<void> {
    if (tab.id === undefined || tab.incognito) return;
    const tabId = String(tab.id);
    const known = await this.db.findLiveByExternalId(this.appId, tabId);
    const [groups, openTabIds] = await Promise.all([
      this.groupsFor(tab),
      known ? Promise.resolve(undefined) : this.openTabIds(),
    ]);
    await this.db.transaction('rw', [this.db.items, this.db.events, this.db.rules], async () => {
      if (this.removed.has(tabId)) return; // closed while this handler was waiting
      const now = this.now();
      const rules = await this.db.rules.toArray();
      let item = await this.db.findLiveByExternalId(this.appId, tabId);
      if (this.excluded(tab, rules)) {
        // e.g. navigated to an excluded site: forget the record (F9).
        if (item && item.state !== 'archiving') await this.deleteItems([item.id]);
        this.touched.set(tabId, now);
        return;
      }
      if (!item && this.waitForTitle(tab, rules, new Set())) return;
      const events: ItemEvent[] = [];
      if (!item && openTabIds) item = await this.findOrphan(tabUrl(tab), openTabIds);
      if (item) {
        if (item.state === 'archiving') return; // the sweep executor owns it
        applyTabFields(item, tab, groups);
        this.missingSince.delete(item.id);
      } else {
        item = newItemFromTab(tab, this.appId, groups, now);
        events.push({ itemId: item.id, type: 'seen', at: now });
      }
      if (opts.used) {
        const used = markUsed(item, now);
        if (used) events.push(used);
      }
      await this.db.items.put(item);
      await this.addEvents(events);
      this.touched.set(tabId, now);
    });
  }

  /**
   * `tabs.onActivated`, or audio started: the tab is in use now — if it is still
   * active (or audible) after a short settle delay (API Design §4.1: debounced).
   */
  async tabUsed(tabId: number): Promise<void> {
    if (this.activationSettleMs > 0) {
      await new Promise((resolve) => setTimeout(resolve, this.activationSettleMs));
    }
    const tab = await this.api.getTab(tabId);
    if (!tab) return;
    await this.upsertTab(tab, { used: tab.active || tab.audible === true });
  }

  /** Re-reads a tab and updates its item (moves, attaches, group changes). */
  async refreshTab(tabId: number): Promise<void> {
    const tab = await this.api.getTab(tabId);
    if (tab) await this.upsertTab(tab);
  }

  /** `tabGroups.onUpdated`: title/colour changed for every tab in the group. */
  async groupUpdated(groupId: number): Promise<void> {
    const tabs = (await this.api.queryTabs()).filter((t) => t.groupId === groupId);
    for (const tab of tabs) await this.upsertTab(tab);
  }

  /** `windows.onFocusChanged`: the focused window's active tab is in use. */
  async windowFocused(windowId: number): Promise<void> {
    if (windowId < 0) return; // WINDOW_ID_NONE: the browser lost focus
    const active = await this.api.getActiveTab(windowId);
    if (active?.id !== undefined) await this.tabUsed(active.id);
  }

  /** `tabs.onReplaced` (prerender / instant): the item follows the new tab id. */
  async tabReplaced(addedTabId: number, removedTabId: number): Promise<void> {
    await this.db.transaction('rw', this.db.items, async () => {
      const item = await this.db.findLiveByExternalId(this.appId, String(removedTabId));
      if (item && item.state !== 'archiving') {
        await this.db.items.update(item.id, { externalId: String(addedTabId) });
      }
      const now = this.now();
      this.touched.set(String(removedTabId), now);
      this.touched.set(String(addedTabId), now);
    });
    await this.refreshTab(addedTabId);
  }

  /**
   * `tabs.onRemoved`. A tab the user closes is forgotten together with its events
   * (we don't keep a history of what users close themselves). When the whole window
   * is closing — which also happens when the browser quits — the tick decides later,
   * so tabs that come back (session restore, reopen window) keep their age.
   */
  async tabRemoved(tabId: number, isWindowClosing: boolean): Promise<void> {
    if (isWindowClosing) return;
    await this.forgetTab(String(tabId), { closed: true });
  }

  private async forgetTab(tabId: string, opts: { closed?: boolean } = {}): Promise<void> {
    await this.db.transaction('rw', this.db.items, this.db.events, async () => {
      const item = await this.db.findLiveByExternalId(this.appId, tabId);
      if (item && item.state !== 'archiving') await this.deleteItems([item.id]);
      const now = this.now();
      this.touched.set(tabId, now);
      if (opts.closed) this.removed.set(tabId, now);
    });
  }

  /**
   * A new tab that is still loading has no real title yet. If any title-based exclude
   * rule exists, wait for the title (the next `onUpdated` or tick) before storing it.
   */
  private waitForTitle(
    tab: TabInfo,
    rules: readonly Rule[],
    liveIds: ReadonlySet<string | undefined>,
  ): boolean {
    return (
      tab.status === 'loading' &&
      !liveIds.has(String(tab.id)) &&
      rules.some((r) => r.enabled && r.action === 'exclude' && r.scope === 'titleRegex')
    );
  }

  private excluded(tab: TabInfo, rules: readonly Rule[]): boolean {
    return isExcluded({ appId: this.appId, url: tabUrl(tab), title: tab.title ?? '' }, rules);
  }

  private async liveItems(): Promise<Item[]> {
    const items = await this.db.items.where('state').anyOf(LIVE_STATES).toArray();
    return items.filter((i) => i.appId === this.appId && i.kind === 'tab' && isLive(i.state));
  }

  /** A live item for `url` whose tab no longer exists (most recently used first). */
  private async findOrphan(
    url: string | undefined,
    openTabIds: ReadonlySet<string>,
  ): Promise<Item | undefined> {
    const orphans = (await this.liveItems()).filter(
      (i) =>
        i.url === url && i.state !== 'archiving' && !(i.externalId && openTabIds.has(i.externalId)),
    );
    return orphans.sort((a, b) => b.lastUsedAt - a.lastUsedAt)[0];
  }

  private async openTabIds(): Promise<Set<string>> {
    const tabs = await this.api.queryTabs();
    return new Set(tabs.filter((t) => t.id !== undefined).map((t) => String(t.id)));
  }

  private async groupsFor(tab: TabInfo): Promise<Map<number, GroupInfo>> {
    if (tab.groupId === undefined || tab.groupId < 0) return new Map();
    const groups = await this.api.getGroups();
    return new Map(groups.map((g) => [g.id, g]));
  }

  /** Hard delete, cascading to the item's events (DB Design §3.5, §5). */
  private async deleteItems(ids: string[]): Promise<void> {
    if (!ids.length) return;
    await this.db.events.where('itemId').anyOf(ids).delete();
    await this.db.items.bulkDelete(ids);
    for (const id of ids) this.missingSince.delete(id);
  }

  private async addEvents(events: ItemEvent[]): Promise<void> {
    if (events.length) await this.db.events.bulkAdd(events);
  }

  private touchedSince(at: number): Set<string> {
    const ids = new Set<string>();
    for (const [id, when] of this.touched) if (when >= at) ids.add(id);
    return ids;
  }

  private pruneTouched(): void {
    const cutoff = this.now() - TOUCH_MEMORY_MS;
    for (const [id, when] of this.touched) if (when < cutoff) this.touched.delete(id);
    for (const [id, when] of this.removed) if (when < cutoff) this.removed.delete(id);
  }
}
