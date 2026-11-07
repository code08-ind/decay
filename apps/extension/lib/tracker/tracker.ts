import { type DecayDB, isLive, LIVE_STATES } from '../db/db';
import type { Item, ItemEvent, Rule } from '../db/types';
import { evaluate, isExcluded, type PolicyContext, type TabSignals } from '../policy/policy';
import {
  cachedVerdict,
  createLimiter,
  formSignal,
  isWebUrl,
  MAX_PROBES_PER_TICK,
  mapLimit,
  PROBE_CONCURRENCY,
  PROBE_TICK_BUDGET_MS,
  type ProbeRecord,
  type ProbeVerdict,
} from '../probe/probe';
import {
  emptyMetrics,
  localDay,
  planShadowSweep,
  SHADOW_SWEEP_INTERVAL_MS,
} from '../shadow/shadow';
import {
  closeTabs,
  finalizeSweep,
  planSweep,
  QUIET_MS,
  recoverInterrupted,
  SWEEP_INTERVAL_MS,
  type TabCommands,
} from '../sweep/executor';
import {
  findBatch,
  type RestoreItemsResult,
  type RestoreResult,
  restoreItems,
  undoSweep,
} from '../sweep/restore';
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
  /** `undefined` only when the tab is known to be gone; throws when the state is unknown. */
  getTab(tabId: number): Promise<TabInfo | undefined>;
  getActiveTab(windowId: number): Promise<TabInfo | undefined>;
  getWindows(): Promise<WindowInfo[]>;
  getGroups(): Promise<GroupInfo[]>;
  queryIdle(cutoffS: number): Promise<IdleState>;
  /** Last tick time, kept in `storage.session` so it resets with the browser session. */
  getLastTickAt(): Promise<number | null>;
  setLastTickAt(at: number): Promise<void>;
}

/** The form probe (P0.7) as the tracker sees it; tests supply a fake. */
export interface TabProber {
  /** Never throws: anything that can't be checked is `unknown`. */
  probe(tab: TabInfo): Promise<ProbeVerdict>;
  load(): Promise<ProbeRecord>;
  save(record: ProbeRecord): Promise<void>;
}

export interface TrackerDeps {
  db: DecayDB;
  api: BrowserApi;
  appId: string;
  getIdleCutoffS: () => Promise<number>;
  /**
   * Tab commands for real sweeps and undo (P0.5). Without them the tracker never closes
   * anything (tests of the clock and shadow mode).
   */
  commands?: TabCommands;
  /**
   * Decay's own extension origin (`chrome-extension://<id>/`). Its pages (Sweeps, review)
   * are never tracked, so a sweep can't close the page that started it.
   */
  ownOrigin?: string;
  /**
   * Form probe (P0.7). Without it every web page's form state is unknown, so web pages
   * archive at 2× the threshold (PRD §7.3).
   */
  prober?: TabProber;
  /** Per-tab close timeout (default 5 s; tests use less). */
  closeTimeoutMs?: number;
  /** Current mode, hold and thresholds. Rules are read from the DB inside each transaction. */
  getPolicy: () => Promise<TrackerPolicy>;
  now?: () => number;
  /**
   * How long an activation must last to count as use. Browsers briefly activate tabs
   * while opening or restoring a window; those flickers must not reset ages.
   */
  activationSettleMs?: number;
}

export type TrackerPolicy = Omit<PolicyContext, 'now' | 'rules'> & {
  /** Cap per (shadow) sweep; default 50. */
  maxItemsPerSweep?: number;
};

export interface TickResult {
  creditS: number;
  liveCount: number;
  written: number;
  deleted: number;
  shadowSweepId?: string;
  /** Items a shadow sweep flagged in this tick. */
  shadowFlagged?: number;
  /** A real sweep ran in this tick (automatic mode). */
  sweep?: { id: string; archived: number; failed: number; pending: number };
}

/** An activation this soon after a close in the same window is Chrome's doing. */
const AUTO_ACTIVATION_WINDOW_MS = 1500;

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
  private readonly getPolicy: () => Promise<TrackerPolicy>;
  private readonly now: () => number;
  private tickChain: Promise<unknown> = Promise.resolve();
  /** tab id → last time a handler wrote or removed it. In memory: same worker as ticks. */
  private readonly touched = new Map<string, number>();
  /** item id → when its tab was first found missing. */
  private missingSince = new Map<string, number>();
  /** tab id → when it was closed; late `onCreated`/`onUpdated` handlers must not revive it. */
  private readonly removed = new Map<string, number>();
  private readonly activationSettleMs: number;
  /** windowId → when a tab in it was last closed (Chrome then activates a neighbour). */
  private readonly lastCloseInWindow = new Map<number, number>();
  private readonly commands?: TabCommands;
  private readonly closeTimeoutMs?: number;
  private readonly ownOrigin?: string;
  /** Last deliberate tab switch / window focus, for the "quiet moment" check. */
  private lastActivityAt = Number.NEGATIVE_INFINITY;
  /**
   * tab id → last deliberate use. Handlers ignore items that are `archiving`, so a use
   * during a sweep is only visible here; the executor checks it before closing.
   */
  private readonly usedAt = new Map<number, number>();
  private readonly prober?: TabProber;
  /** Probe verdicts per tab id; loaded from `storage.session` at the first tick. */
  private probes: ProbeRecord | null = null;
  /** Caps probes in flight, also across a sweep's pre-close checks. */
  private readonly probeLimit = createLimiter(PROBE_CONCURRENCY);

  constructor(deps: TrackerDeps) {
    this.db = deps.db;
    this.api = deps.api;
    this.appId = deps.appId;
    this.getIdleCutoffS = deps.getIdleCutoffS;
    this.getPolicy = deps.getPolicy;
    this.now = deps.now ?? Date.now;
    this.activationSettleMs = deps.activationSettleMs ?? 500;
    this.commands = deps.commands;
    this.closeTimeoutMs = deps.closeTimeoutMs;
    this.ownOrigin = deps.ownOrigin;
    this.prober = deps.prober;
  }

  /** Forget every probe verdict (site access granted or revoked: re-check everything). */
  resetProbes(): Promise<void> {
    return this.exclusive(async () => {
      this.probes = {};
      await this.prober?.save({});
    });
  }

  /**
   * Ticks are serialized: two overlapping ticks (worker start + alarm) would both read
   * the same `lastTickAt` and credit the same interval twice.
   */
  tick(): Promise<TickResult> {
    return this.exclusive(() => this.runTick());
  }

  /**
   * "Sweep now" (API `run_sweep_now`): a tick that sweeps regardless of the 15-min interval
   * and the quiet-moment check. Automatic mode closes tabs; shadow mode only simulates
   * (dry run); paused does nothing. Protections always apply.
   */
  sweepNow(): Promise<TickResult> {
    return this.exclusive(() => this.runTick({ manual: true }));
  }

  /** Undo a sweep (default: the newest undoable one), serialized with ticks and sweeps. */
  undoSweep(sweepId?: string): Promise<RestoreResult> {
    const commands = this.commands;
    if (!commands) return Promise.reject(new Error('Tab commands unavailable'));
    return this.exclusive(async () => {
      const result = await undoSweep(this.db, commands, {
        appId: this.appId,
        now: this.now,
        sweepId,
        getTabs: async () => (await this.api.queryTabs()).filter((t) => !t.incognito),
      });
      const now = this.now();
      for (const id of result.tabIds) this.touched.set(String(id), now);
      return result;
    });
  }

  /**
   * Restore archived items (F6). With `intoWindowId`, they open in that window (the first
   * one active); otherwise in their original window or a new one per original window.
   */
  restore(
    ids: readonly string[],
    opts: { intoWindowId?: number; newWindow?: boolean } = {},
  ): Promise<RestoreItemsResult> {
    const commands = this.commands;
    if (!commands) return Promise.reject(new Error('Tab commands unavailable'));
    return this.exclusive(async () => {
      const result = await restoreItems(this.db, commands, {
        appId: this.appId,
        now: this.now,
        ids,
        ...opts,
        getTabs: async () => (await this.api.queryTabs()).filter((t) => !t.incognito),
      });
      const now = this.now();
      for (const id of result.tabIds) this.touched.set(String(id), now);
      return result;
    });
  }

  /** Restore the whole batch an item was archived with (same sweep, same window). */
  async restoreBatch(itemId: string): Promise<RestoreItemsResult> {
    const batch = await findBatch(this.db, itemId);
    // S4 / §4.5: a batch reopens in one new window per original window, in order.
    return this.restore(
      batch.map((i) => i.id),
      { newWindow: true },
    );
  }

  /** Runs `task` after every queued tick/sweep/undo, and before the next one. */
  private exclusive<T>(task: () => Promise<T>): Promise<T> {
    const next = this.tickChain.then(task);
    this.tickChain = next.catch(() => undefined);
    return next;
  }

  private async runTick(opts: { manual?: boolean } = {}): Promise<TickResult> {
    const [cutoff, policy] = await Promise.all([this.getIdleCutoffS(), this.getPolicy()]);
    const commands = this.commands;
    // A sweep left half-done by a killed worker is settled before anything else.
    if (commands) {
      const openTabs = async () => (await this.api.queryTabs()).filter((t) => !t.incognito);
      const session = await commands.browserSession();
      await recoverInterrupted(this.db, openTabs, this.now(), session);
    }
    const browserSession = commands ? (await commands.browserSession()).id : '';
    // Probe tabs that would be archived if nothing were typed in them (P0.7), before the
    // snapshot and the transaction (browser calls can't run inside one, and probing takes a
    // moment, so the snapshot below is fresh). A failure only means "unknown".
    if (this.prober) {
      await this.api
        .queryTabs()
        .then((tabs) =>
          this.refreshProbes(
            tabs.filter((t) => !t.incognito),
            policy,
            this.now(),
          ),
        )
        .catch((e) => console.error('[decay] probe failed', e));
    }
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

    const db = this.db;
    const tables = [db.items, db.events, db.rules, db.metrics, db.sweeps];
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
      let regrets = 0;
      const tickEvents = applyTick(r.items, inUse, creditS, now, r.dirty, () => regrets++);
      const decisions = applyLifecycle(
        r.items,
        tabsById,
        { ...policy, rules, now },
        r.dirty,
        tickEvents,
        (item, tab) => this.formSignalFor(item, tab, now),
      );

      // Shadow mode (F2): simulate a sweep at most every 15 min; nothing is closed.
      let flagged = 0;
      let sweepId: string | undefined;
      if (policy.mode === 'shadow') {
        const last = await db.sweeps.where('isShadow').equals(1).last();
        // "Sweep now" in shadow mode runs a simulation right away (API `run_sweep_now` dryRun).
        if (opts.manual || !last || now - last.startedAt >= SHADOW_SWEEP_INTERVAL_MS) {
          const plan = planShadowSweep(
            r.items,
            decisions,
            now,
            policy.maxItemsPerSweep ?? 50,
            opts.manual ? 'manual' : 'scheduled',
          );
          if (plan) {
            await db.sweeps.add(plan.sweep);
            for (const item of plan.flagged) r.dirty.add(item);
            tickEvents.push(...plan.events);
            flagged = plan.flagged.length;
            sweepId = plan.sweep.id;
          }
        }
      }

      // Automatic mode (F3): plan a real sweep and commit the archive records before
      // any tab is closed (write-before-close).
      let planned: ReturnType<typeof planSweep> = null;
      if (commands && policy.mode === 'automatic' && (opts.manual || this.isQuiet(now))) {
        const last = await db.sweeps.where('isShadow').equals(0).last();
        if (opts.manual || !last || now - last.startedAt >= SWEEP_INTERVAL_MS) {
          planned = planSweep(r.items, decisions, {
            now,
            maxItems: policy.maxItemsPerSweep ?? 50,
            trigger: opts.manual ? 'manual' : 'scheduled',
            browserSession,
          });
          if (planned) {
            await db.sweeps.add(planned.sweep);
            for (const item of planned.archiving) r.dirty.add(item);
          }
        }
      }

      await this.recordMetrics(now, {
        activeSeconds: creditS,
        flagged,
        regrets,
        automatic: policy.mode === 'automatic',
      });
      await this.deleteItems(r.deleteIds);
      if (r.dirty.size) await db.items.bulkPut([...r.dirty]);
      await this.addEvents([...r.events, ...tickEvents]);
      return { r, liveCount: r.items.length, written: r.dirty.size, sweepId, flagged, planned };
    });
    this.missingSince = result.r.missingSince;
    // Only after a successful write, so a failed tick's time is credited by the next one.
    await this.api.setLastTickAt(snapshotAt);
    this.pruneTouched();
    const sweep =
      result.planned && commands ? await this.executeSweep(result.planned, commands) : undefined;
    return {
      creditS,
      liveCount: result.liveCount,
      written: result.written,
      deleted: result.r.deleteIds.length,
      shadowSweepId: result.sweepId,
      shadowFlagged: result.flagged,
      sweep,
    };
  }

  /** Closes the planned tabs and records the outcome (Software Design §4.4). */
  private async executeSweep(
    plan: NonNullable<ReturnType<typeof planSweep>>,
    commands: TabCommands,
  ): Promise<{ id: string; archived: number; failed: number; pending: number }> {
    const id = plan.sweep.id;
    // Re-check the global switches: a pause or hold set since planning cancels the closes.
    const policy = await this.getPolicy().catch(() => null);
    const now = this.now();
    const allowed =
      policy !== null &&
      policy.mode === 'automatic' &&
      !(policy.holdUntil !== null && policy.holdUntil > now);
    const rules = allowed ? await this.db.rules.toArray() : [];
    const prober = this.prober;
    // Last check (P0.7): probe again right before closing. Decided with the fresh verdict by
    // the same policy: still `archive` → close; typed input → keep (`formInput`); a page that
    // can no longer be checked and isn't old enough for the 2× rule → keep (`changed`).
    const confirm =
      prober && policy
        ? async (item: Item, tab: TabInfo) => {
            if (!isWebUrl(item.url)) return null;
            const startedAt = this.now();
            const verdict = await this.probeLimit(() => prober.probe(tab));
            this.storeProbe(tab, verdict, startedAt);
            const decision = evaluate(
              item,
              { active: tab.active, audible: tab.audible === true, formInput: formSignal(verdict) },
              { ...policy, rules, now: this.now() },
            );
            if (decision.action === 'archive') return null;
            return verdict === 'dirty' ? ('formInput' as const) : ('changed' as const);
          }
        : undefined;
    const outcomes = allowed
      ? await closeTabs(plan.archiving, (tabId) => this.api.getTab(tabId), commands, {
          usedAt: (tabId) => this.usedAt.get(tabId),
          usedSince: plan.sweep.startedAt - QUIET_MS,
          timeoutMs: this.closeTimeoutMs,
          confirm,
        })
      : new Map(
          plan.archiving.map((i) => [
            i.id,
            { result: 'skipped' as const, skipReason: 'modeChanged' as const },
          ]),
        );
    const done = await finalizeSweep(this.db, id, outcomes, this.now());
    if (confirm && this.probes) {
      await prober?.save(this.probes).catch((e) => console.error('[decay] probe save failed', e));
    }
    return { id, ...done };
  }

  /** The policy's form signal for an item: the cached probe verdict, or unknown. */
  private formSignalFor(
    item: Item,
    tab: TabInfo | undefined,
    now: number,
  ): TabSignals['formInput'] {
    if (!this.prober || !this.probes) return 'unknown';
    return formSignal(cachedVerdict(this.probes, tab, item.lastUsedAt, now));
  }

  /** `at` is when the probe *started*: a use during the probe invalidates the verdict. */
  private storeProbe(tab: TabInfo, verdict: ProbeVerdict, at: number): void {
    if (tab.id === undefined) return;
    this.probes ??= {};
    this.probes[String(tab.id)] = { url: tabUrl(tab) ?? '', at, verdict };
  }

  /**
   * Probes the open web tabs that would be archived now if nothing were typed in them and
   * that have no valid cached verdict — oldest first, at most `MAX_PROBES_PER_TICK` per tick.
   * Typing needs the tab in front (which resets its age), so a verdict for a stale tab stays
   * good until the tab is used, navigates, or the verdict expires.
   */
  private async refreshProbes(
    tabs: readonly TabInfo[],
    policy: TrackerPolicy,
    now: number,
  ): Promise<void> {
    const prober = this.prober;
    if (!prober) return;
    this.probes ??= await prober.load();
    const record = this.probes;
    const tabsById = new Map(tabs.map((t) => [String(t.id), t]));
    let changed = false;
    for (const id of Object.keys(record)) {
      if (!tabsById.has(id)) {
        delete record[id];
        changed = true;
      }
    }
    const [items, rules] = await Promise.all([this.liveItems(), this.db.rules.toArray()]);
    const ctx = { ...policy, rules, now };
    const due = items
      .filter((item) => item.state === 'active' || item.state === 'stale')
      .map((item) => ({ item, tab: item.externalId ? tabsById.get(item.externalId) : undefined }))
      .filter(
        (c): c is { item: Item; tab: TabInfo } =>
          c.tab !== undefined &&
          isWebUrl(c.item.url) &&
          tabUrl(c.tab) === c.item.url &&
          cachedVerdict(record, c.tab, c.item.lastUsedAt, now) === undefined &&
          evaluate(
            c.item,
            { active: c.tab.active, audible: c.tab.audible === true, formInput: false },
            ctx,
          ).action === 'archive',
      )
      .sort((a, b) => b.item.activeAgeS - a.item.activeAgeS)
      .slice(0, MAX_PROBES_PER_TICK);
    // A time budget per tick (real time): the rest wait for the next tick, oldest first.
    const deadline = Date.now() + PROBE_TICK_BUDGET_MS;
    await mapLimit(due, PROBE_CONCURRENCY, async ({ tab }) => {
      if (Date.now() >= deadline) return;
      const startedAt = this.now();
      this.storeProbe(tab, await this.probeLimit(() => prober.probe(tab)), startedAt);
      changed = true;
    });
    if (changed) await prober.save(record);
  }

  private isQuiet(now: number): boolean {
    return now - this.lastActivityAt >= QUIET_MS;
  }

  /** Adds to today's on-device metrics row (PRD §9 metrics, F2 shadow stats). */
  private async recordMetrics(
    now: number,
    add: { activeSeconds: number; flagged: number; regrets: number; automatic?: boolean },
  ): Promise<void> {
    if (!add.activeSeconds && !add.flagged && !add.regrets) return;
    const day = localDay(now);
    const row = (await this.db.metrics.get(day)) ?? emptyMetrics(day);
    // Adoption metric (PRD §9 Gate 0): the day counts as "auto mode on" if it was on
    // during any active tick.
    if (add.automatic) row.autoModeOn = 1;
    row.activeSeconds = (row.activeSeconds ?? 0) + add.activeSeconds;
    row.shadowFlaggedCount = (row.shadowFlaggedCount ?? 0) + add.flagged;
    row.shadowRegretCount = (row.shadowRegretCount ?? 0) + add.regrets;
    await this.db.metrics.put(row);
  }

  /**
   * `tabs.onCreated` / `onUpdated` / `onMoved` / `onAttached`, and group changes.
   * A tab id we don't know first tries to reclaim an item whose tab is gone but has
   * the same URL ("reopen closed window", session restore racing the first tick).
   */
  async upsertTab(
    tab: TabInfo,
    opts: { used?: boolean; deliberate?: boolean } = {},
  ): Promise<void> {
    if (tab.id === undefined || tab.incognito) return;
    const tabId = String(tab.id);
    const known = await this.db.findLiveByExternalId(this.appId, tabId);
    const [groups, openTabIds] = await Promise.all([
      this.groupsFor(tab),
      known ? Promise.resolve(undefined) : this.openTabIds(),
    ]);
    const tables = [this.db.items, this.db.events, this.db.rules, this.db.metrics];
    await this.db.transaction('rw', tables, async () => {
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
        const { event, regret } = markUsed(item, now, opts.deliberate ?? true);
        if (event) events.push(event);
        if (regret) await this.recordMetrics(now, { activeSeconds: 0, flagged: 0, regrets: 1 });
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
  async tabUsed(
    tabId: number,
    opts: { deliberate?: boolean; activatedInWindow?: number } = {},
  ): Promise<void> {
    const at = this.now();
    if (opts.deliberate !== false) {
      this.lastActivityAt = at;
      this.usedAt.set(tabId, at);
    }
    // An activation right after a close in the same window is Chrome picking the
    // neighbour, not the user choosing it: it keeps the tab young but isn't regret.
    let deliberate = opts.deliberate ?? true;
    if (opts.activatedInWindow !== undefined) {
      const closedAt = this.lastCloseInWindow.get(opts.activatedInWindow);
      if (closedAt !== undefined && at - closedAt <= AUTO_ACTIVATION_WINDOW_MS) deliberate = false;
    }
    if (this.activationSettleMs > 0) {
      await new Promise((resolve) => setTimeout(resolve, this.activationSettleMs));
    }
    const tab = await this.api.getTab(tabId);
    if (!tab) return;
    await this.upsertTab(tab, {
      used: tab.active || tab.audible === true,
      deliberate: deliberate && tab.active,
    });
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
  async tabRemoved(tabId: number, isWindowClosing: boolean, windowId?: number): Promise<void> {
    if (windowId !== undefined) this.lastCloseInWindow.set(windowId, this.now());
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
    if (this.ownOrigin && tabUrl(tab)?.startsWith(this.ownOrigin)) return true;
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
    for (const [id, when] of this.usedAt) if (when < cutoff) this.usedAt.delete(id);
  }
}
