import { uuidv7 } from 'uuidv7';
import type { DecayDB } from '../db/db';
import type {
  Item,
  ItemEvent,
  Sweep,
  SweepItem,
  SweepItemResult,
  SweepSkipReason,
  SweepTrigger,
} from '../db/types';
import type { Decision } from '../policy/policy';
import { emptyMetrics, localDay } from '../shadow/shadow';
import { type TabInfo, tabUrl } from '../tracker/tabs';

/**
 * Sweep executor (P0.5, PRD F3; Software Design §4.4 "write-before-close").
 *
 * 1. Plan (inside the tick transaction): the oldest items whose decision is `archive`
 *    become `archiving`, and a `running` sweep record with each item's previous state is
 *    written. The archive record is committed before any tab is touched.
 * 2. Close (outside any transaction): every tab is re-checked against the live browser
 *    (still the same URL, not active / pinned / audible) and closed, 5 s timeout each.
 * 3. Finalize (one transaction): closed → `archived` + `archived` event; anything else →
 *    back to its previous state + `close_failed` event (the record stays, harmless).
 *
 * A worker killed between 1 and 3 leaves `archiving` items behind; `recoverInterrupted`
 * settles them at the next tick: tab still open → revert, gone → archived.
 */

/** Real sweeps run at most every 15 min (Software Design §4.3). */
export const SWEEP_INTERVAL_MS = 15 * 60_000;
/** "Quiet moment": no tab switch or window focus for this long. */
export const QUIET_MS = 30_000;
export const CLOSE_TIMEOUT_MS = 5_000;
/** Undo is offered for the last 20 real sweeps (PRD F7). */
export const UNDO_HISTORY = 20;
/** Restored items are protected from re-archiving for 48 h (Software Design §4.5). */
export const RESTORE_GRACE_MS = 48 * 3600_000;
/** A restore within this long of archiving counts toward the regret metric. */
export const REGRET_WINDOW_MS = 24 * 3600_000;

/** Commands the executor and restore need from the browser (tests supply a fake). */
export interface TabCommands {
  removeTab(tabId: number): Promise<void>;
  createTab(opts: {
    windowId: number;
    url: string;
    index?: number;
    active?: boolean;
  }): Promise<TabInfo>;
  /** A new, unfocused window with one tab. */
  createWindow(url: string): Promise<{ windowId: number; tab: TabInfo }>;
  windowExists(windowId: number): Promise<boolean>;
  /** Groups the tabs, returns the group id. */
  groupTabs(tabIds: number[], windowId: number): Promise<number>;
  updateGroup(groupId: number, props: { title?: string; color?: string }): Promise<void>;
  /** The current browser session (kept in `storage.session`, so it resets on restart). */
  browserSession(): Promise<BrowserSession>;
}

export type CloseOutcome = {
  result: SweepItemResult;
  skipReason?: SweepSkipReason;
  /** A deliberate use seen during the sweep; applied when the item is reverted. */
  usedAt?: number;
};

export interface BrowserSession {
  id: string;
  /** When this browser session was first seen by the extension. */
  startedAt: number;
}

export interface SweepPlan {
  sweep: Sweep;
  /** Items now in state `archiving` (mutated in place; the caller writes them). */
  archiving: Item[];
}

/**
 * Picks the oldest open items whose decision is `archive` (≤ `maxItems`) and turns them
 * into `archiving` items + a `running` sweep. Returns null when there's nothing to do.
 */
export function planSweep(
  items: readonly Item[],
  decisions: ReadonlyMap<string, Decision>,
  opts: { now: number; maxItems: number; trigger: SweepTrigger; browserSession: string },
): SweepPlan | null {
  const { now } = opts;
  const candidates = items
    .filter(
      (i) =>
        (i.state === 'active' || i.state === 'stale') &&
        i.externalId !== undefined &&
        decisions.get(i.id)?.action === 'archive',
    )
    .sort((a, b) => b.activeAgeS - a.activeAgeS)
    .slice(0, Math.max(0, opts.maxItems));
  if (!candidates.length) return null;
  const id = uuidv7();
  const entries: SweepItem[] = [];
  for (const item of candidates) {
    const decision = decisions.get(item.id);
    entries.push({
      itemId: item.id,
      action: 'archive',
      prevState: item.state,
      result: 'pending',
      windowExternalId: item.windowExternalId,
    });
    item.state = 'archiving';
    item.stateChangedAt = now;
    item.lastSweepId = id;
    if (decision?.action === 'archive') item.archiveReason = decision.reason;
  }
  const sweep: Sweep = {
    id,
    startedAt: now,
    trigger: opts.trigger,
    isShadow: 0,
    status: 'running',
    itemCount: candidates.length,
    failedCount: 0,
    items: entries,
    browserSession: opts.browserSession,
  };
  return { sweep, archiving: candidates };
}

/** Why a tab must not be closed right now, or null if it's still safe (defence in depth). */
export function closeBlocker(item: Item, tab: TabInfo): SweepSkipReason | null {
  if (tab.active || tab.pinned || tab.audible) return 'inUse';
  if (tabUrl(tab) !== item.url) return 'changed';
  return null;
}

const TIMEOUT = Symbol('timeout');

function withTimeout<T>(task: Promise<T>, ms: number): Promise<T | typeof TIMEOUT> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => resolve(TIMEOUT), ms);
    task.then(
      (v) => {
        clearTimeout(timer);
        resolve(v);
      },
      (e) => {
        clearTimeout(timer);
        reject(e);
      },
    );
  });
}

export interface CloseOptions {
  timeoutMs?: number;
  /** When the user last deliberately used this tab (activations the item didn't record). */
  usedAt?: (tabId: number) => number | undefined;
  /** Uses at or after this time block the close (the sweep's planning time minus the quiet period). */
  usedSince?: number;
}

/**
 * Closes the planned tabs. Every tab is re-read and re-checked first.
 * `getTab` resolves `undefined` only when the tab is known to be gone and throws when the
 * browser can't say — unknown state never closes anything and never archives anything:
 * - read fails before closing → skipped (nothing was closed);
 * - tab already gone before we closed it → `closedByUser` (the record is dropped);
 * - close times out or fails and the tab can't be read → `pending`: the item stays
 *   `archiving` and the next tick's recovery settles it against the live tabs.
 */
export async function closeTabs(
  items: readonly Item[],
  getTab: (tabId: number) => Promise<TabInfo | undefined>,
  commands: TabCommands,
  opts: CloseOptions = {},
): Promise<Map<string, CloseOutcome>> {
  const timeoutMs = opts.timeoutMs ?? CLOSE_TIMEOUT_MS;
  const outcomes = new Map<string, CloseOutcome>();
  await Promise.all(
    items.map(async (item) => {
      const set = (o: CloseOutcome) => outcomes.set(item.id, o);
      const tabId = Number(item.externalId);
      if (!Number.isInteger(tabId)) return set({ result: 'skipped', skipReason: 'changed' });
      let tab: TabInfo | undefined;
      try {
        tab = await getTab(tabId);
      } catch {
        return set({ result: 'skipped', skipReason: 'error' });
      }
      if (!tab) return set({ result: 'skipped', skipReason: 'closedByUser' });
      const usedAt = opts.usedAt?.(tabId);
      if (usedAt !== undefined && usedAt >= (opts.usedSince ?? Number.POSITIVE_INFINITY)) {
        return set({ result: 'skipped', skipReason: 'inUse', usedAt });
      }
      const blocker = closeBlocker(item, tab);
      if (blocker) return set({ result: 'skipped', skipReason: blocker });
      let timedOut = false;
      try {
        timedOut = (await withTimeout(commands.removeTab(tabId), timeoutMs)) === TIMEOUT;
        if (!timedOut) return set({ result: 'ok' });
      } catch {
        // Fall through: find out whether the tab is still there.
      }
      let still: TabInfo | undefined;
      try {
        still = await getTab(tabId);
      } catch {
        return set({ result: 'pending' });
      }
      if (!still) return set({ result: 'ok' }); // our close went through
      // Still open: a timed-out close may still complete later, so don't revert yet.
      return set(timedOut ? { result: 'pending' } : { result: 'failed', skipReason: 'error' });
    }),
  );
  return outcomes;
}

export interface FinalizeResult {
  /** Tabs this sweep has archived in total. */
  archived: number;
  /** Planned tabs that were kept open (or dropped because the user closed them). */
  failed: number;
  /** Tabs whose outcome is still unknown; recovery settles them at the next tick. */
  pending: number;
}

/**
 * Writes the outcome of a sweep in one transaction. Items no longer `archiving` for this
 * sweep (already settled) are left alone; missing outcomes count as failed (reverted).
 * The sweep stays `running` while any item is `pending`.
 */
export async function finalizeSweep(
  db: DecayDB,
  sweepId: string,
  outcomes: ReadonlyMap<string, CloseOutcome>,
  now: number,
): Promise<FinalizeResult> {
  return db.transaction('rw', [db.items, db.events, db.sweeps, db.metrics], async () => {
    const sweep = await db.sweeps.get(sweepId);
    if (!sweep) return { archived: 0, failed: 0, pending: 0 };
    const items = await db.items.bulkGet(sweep.items.map((e) => e.itemId));
    const events: ItemEvent[] = [];
    const writes: Item[] = [];
    const deletes: string[] = [];
    let newlyArchived = 0;
    sweep.items.forEach((entry, i) => {
      const item = items[i];
      if (item?.state !== 'archiving' || item.lastSweepId !== sweepId) {
        if (entry.result === 'pending') {
          entry.result = 'skipped';
          entry.skipReason = 'changed';
        }
        return;
      }
      const outcome: CloseOutcome = outcomes.get(item.id) ?? {
        result: 'failed',
        skipReason: 'error',
      };
      if (outcome.result === 'pending') return; // still unknown: leave it `archiving`
      entry.result = outcome.result;
      if (outcome.skipReason) entry.skipReason = outcome.skipReason;
      if (outcome.skipReason === 'closedByUser') {
        // The user closed it themselves: we don't keep a history of those (DB Design §5).
        deletes.push(item.id);
        return;
      }
      item.stateChangedAt = now;
      if (outcome.result === 'ok') {
        item.state = 'archived';
        item.archivedAt = now;
        delete item.shadowFlaggedAt;
        // Archived records drop their live ids (browsers reuse tab ids).
        delete item.externalId;
        delete item.windowExternalId;
        events.push({
          itemId: item.id,
          type: 'archived',
          sweepId,
          reason: item.archiveReason,
          at: now,
        });
        newlyArchived++;
      } else {
        item.state = entry.prevState;
        if (outcome.usedAt !== undefined) {
          // A use the handlers couldn't record while the item was `archiving`.
          item.activeAgeS = 0;
          item.lastUsedAt = Math.max(item.lastUsedAt, outcome.usedAt);
          item.state = 'active';
        }
        events.push({ itemId: item.id, type: 'close_failed', sweepId, at: now });
      }
      writes.push(item);
    });
    if (writes.length) await db.items.bulkPut(writes);
    if (deletes.length) {
      await db.events.where('itemId').anyOf(deletes).delete();
      await db.items.bulkDelete(deletes);
    }
    if (events.length) await db.events.bulkAdd(events);
    const archived = sweep.items.filter((e) => e.result === 'ok').length;
    const pending = sweep.items.filter((e) => e.result === 'pending').length;
    const failed = sweep.items.length - archived - pending;
    if (!pending) {
      sweep.status = 'done';
      sweep.finishedAt = now;
    }
    sweep.itemCount = archived;
    sweep.failedCount = failed;
    await db.sweeps.put(sweep);
    if (newlyArchived) {
      const day = localDay(now);
      const row = (await db.metrics.get(day)) ?? emptyMetrics(day);
      row.archivedCount = (row.archivedCount ?? 0) + newlyArchived;
      await db.metrics.put(row);
    }
    return { archived, failed, pending };
  });
}

/** After a browser restart, wait this long for session restore before settling old sweeps. */
export const RESTORE_SETTLE_MS = 2 * 60_000;

/**
 * Crash recovery (Software Design §4.4): settles sweeps left `running` — by a worker that
 * died between commit and finalize, or by closes whose outcome was unknown. A tab counts as
 * "still open" if its id still shows the same URL, or any open tab has that URL (tab ids
 * change across browser restarts). Sweeps from an earlier browser session wait until
 * session restore has had time to reopen tabs: before that, "gone" can't be told apart
 * from "not restored yet".
 */
export async function recoverInterrupted(
  db: DecayDB,
  getTabs: () => Promise<readonly TabInfo[]>,
  now: number,
  session: BrowserSession,
): Promise<number> {
  const running = await db.sweeps.where('status').equals('running').toArray();
  const due = running.filter(
    (s) =>
      s.isShadow === 0 &&
      (s.browserSession === session.id || now - session.startedAt >= RESTORE_SETTLE_MS),
  );
  if (!due.length) return 0;
  const tabs = await getTabs();
  const openUrls = new Set(tabs.map((t) => tabUrl(t)));
  const byId = new Map(tabs.map((t) => [String(t.id), t]));
  for (const sweep of due) {
    const sameSession = sweep.browserSession === session.id;
    const items = await db.items.bulkGet(sweep.items.map((e) => e.itemId));
    const outcomes = new Map<string, CloseOutcome>();
    for (const item of items) {
      if (!item) continue;
      const sameTab = sameSession && item.externalId ? byId.get(item.externalId) : undefined;
      const open = (sameTab && tabUrl(sameTab) === item.url) || openUrls.has(item.url);
      outcomes.set(item.id, open ? { result: 'skipped', skipReason: 'error' } : { result: 'ok' });
    }
    await finalizeSweep(db, sweep.id, outcomes, now);
  }
  return due.length;
}

const closedSomething = (s: Sweep) => s.items.some((e) => e.result === 'ok');

/** The newest real sweeps, newest first (for history). */
export async function recentSweeps(db: DecayDB, limit: number): Promise<Sweep[]> {
  return db.sweeps
    .orderBy('startedAt')
    .reverse()
    .filter((s) => s.isShadow === 0)
    .limit(limit)
    .toArray();
}

/** The undo window: the last 20 sweeps that closed something (PRD F7). */
export async function undoWindow(db: DecayDB): Promise<Sweep[]> {
  return db.sweeps
    .orderBy('startedAt')
    .reverse()
    .filter((s) => s.isShadow === 0 && closedSomething(s))
    .limit(UNDO_HISTORY)
    .toArray();
}

/** Whether a sweep can still be undone: done, archived something, within the undo window. */
export function canUndo(sweep: Sweep, window: readonly Sweep[]): boolean {
  return (
    sweep.isShadow === 0 &&
    sweep.status === 'done' &&
    closedSomething(sweep) &&
    window.some((s) => s.id === sweep.id)
  );
}
