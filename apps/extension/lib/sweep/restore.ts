import { uuidv7 } from 'uuidv7';
import type { DecayDB } from '../db/db';
import type { Item, ItemEvent, Sweep } from '../db/types';
import { emptyMetrics, localDay } from '../shadow/shadow';
import { domainOf, type TabInfo, tabUrl } from '../tracker/tabs';
import {
  canUndo,
  REGRET_WINDOW_MS,
  RESTORE_GRACE_MS,
  type TabCommands,
  undoWindow,
} from './executor';

/**
 * Restore (Software Design §4.5). Tabs reopen in the background:
 * - into their original window if it still exists in this browser session,
 * - otherwise one new window per original window, in the original tab order;
 * - tab groups are rebuilt by name + colour.
 * Each restored archive record becomes `restored` and links to a new live item with a
 * 48-hour grace protection, so it isn't archived again right away.
 */

export interface RestoreResult {
  restored: number;
  failed: number;
  /** New tab ids (so the tracker can ignore stale snapshots of them). */
  tabIds: number[];
}

export class UndoError extends Error {
  constructor(
    readonly code: 'NOT_FOUND' | 'NOT_UNDOABLE',
    message: string,
  ) {
    super(message);
  }
}

interface Opened {
  item: Item;
  tab: TabInfo;
}

/** Opens the given archived items as tabs. Never throws for a single failed tab. */
export async function reopenTabs(
  items: readonly Item[],
  windowOf: ReadonlyMap<string, string | undefined>,
  commands: TabCommands,
  sameSession: boolean,
): Promise<{ opened: Opened[]; failed: Item[] }> {
  const opened: Opened[] = [];
  const failed: Item[] = [];
  // Group by original window; keep tab order.
  const byWindow = new Map<string, Item[]>();
  for (const item of items) {
    const key = windowOf.get(item.id) ?? 'none';
    const list = byWindow.get(key) ?? [];
    list.push(item);
    byWindow.set(key, list);
  }
  for (const [key, list] of byWindow) {
    list.sort((a, b) => (a.tabIndex ?? 0) - (b.tabIndex ?? 0));
    const original = Number(key);
    let windowId: number | undefined;
    if (sameSession && Number.isInteger(original)) {
      windowId = (await commands.windowExists(original).catch(() => false)) ? original : undefined;
    }
    const here: Opened[] = [];
    for (const item of list) {
      const url = item.url;
      if (!url) {
        failed.push(item);
        continue;
      }
      try {
        if (windowId === undefined) {
          const created = await commands.createWindow(url);
          windowId = created.windowId;
          here.push({ item, tab: created.tab });
        } else {
          const tab = await commands.createTab({
            windowId,
            url,
            // Original position when reopening into the same window (clamped by the browser).
            index: sameSession && windowId === original ? item.tabIndex : undefined,
            active: false,
          });
          here.push({ item, tab });
        }
      } catch {
        failed.push(item);
      }
    }
    if (windowId !== undefined) await regroup(here, windowId, commands);
    opened.push(...here);
  }
  return { opened, failed };
}

/** Rebuilds tab groups (best effort: a failure leaves the tabs ungrouped). */
async function regroup(opened: readonly Opened[], windowId: number, commands: TabCommands) {
  const groups = new Map<string, Opened[]>();
  for (const o of opened) {
    if (!o.item.groupTitle && !o.item.groupColor) continue;
    const key = `${o.item.groupTitle ?? ''}\u0000${o.item.groupColor ?? ''}`;
    const list = groups.get(key) ?? [];
    list.push(o);
    groups.set(key, list);
  }
  for (const list of groups.values()) {
    const ids = list.map((o) => o.tab.id).filter((id): id is number => id !== undefined);
    if (!ids.length) continue;
    try {
      const groupId = await commands.groupTabs(ids, windowId);
      const first = list[0]?.item;
      await commands.updateGroup(groupId, { title: first?.groupTitle, color: first?.groupColor });
    } catch {
      // Grouping is cosmetic; the tabs are open.
    }
  }
}

/**
 * Records restores: archive record → `restored` (+ `undo` or `restored` event), and a live
 * item for the new tab with grace protection. The tab's `onCreated` handler may already
 * have created that live item; it is then updated instead of duplicated.
 */
export async function recordRestores(
  db: DecayDB,
  opened: readonly Opened[],
  opts: { appId: string; now: number; sweepId?: string; asUndo: boolean },
): Promise<void> {
  const { now } = opts;
  await db.transaction('rw', [db.items, db.events, db.metrics], async () => {
    const events: ItemEvent[] = [];
    let regrets = 0;
    for (const { item: snapshot, tab } of opened) {
      const record = await db.items.get(snapshot.id);
      if (!record) continue;
      if (record.archivedAt !== undefined && now - record.archivedAt <= REGRET_WINDOW_MS) regrets++;
      await db.items.update(record.id, {
        state: 'restored',
        stateChangedAt: now,
        restoreCount: (record.restoreCount ?? 0) + 1,
      });
      events.push({
        itemId: record.id,
        type: opts.asUndo ? 'undo' : 'restored',
        sweepId: opts.sweepId,
        at: now,
      });
      if (tab.id === undefined) continue;
      const tabId = String(tab.id);
      const existing = await db.findLiveByExternalId(opts.appId, tabId);
      const fields: Partial<Item> = {
        restoredFromId: record.id,
        graceUntil: now + RESTORE_GRACE_MS,
        activeAgeS: 0,
        lastUsedAt: now,
        // The new tab may still be loading: keep the archived title and group until it updates.
        title: existing?.title || record.title,
        groupTitle: existing?.groupTitle ?? record.groupTitle,
        groupColor: existing?.groupColor ?? record.groupColor,
        faviconUrl: existing?.faviconUrl ?? record.faviconUrl,
      };
      if (existing) {
        await db.items.update(existing.id, fields);
      } else {
        const url = tabUrl(tab) ?? record.url;
        const live: Item = {
          ...record,
          ...fields,
          id: uuidv7(),
          url,
          domain: domainOf(url),
          externalId: tabId,
          windowExternalId: String(tab.windowId),
          tabIndex: tab.index,
          pinned: false,
          state: 'active',
          firstSeenAt: now,
          stateChangedAt: now,
          restoreCount: 0,
        };
        delete live.archivedAt;
        delete live.archiveReason;
        delete live.lastSweepId;
        delete live.shadowFlaggedAt;
        delete live.lastUsedEventAt;
        await db.items.add(live);
        events.push({ itemId: live.id, type: 'seen', at: now });
      }
    }
    if (events.length) await db.events.bulkAdd(events);
    if (opened.length) {
      const day = localDay(now);
      const row = (await db.metrics.get(day)) ?? emptyMetrics(day);
      row.restored24hCount = (row.restored24hCount ?? 0) + regrets;
      if (opts.asUndo) row.undoCount = (row.undoCount ?? 0) + 1;
      await db.metrics.put(row);
    }
  });
}

/** The sweep Undo acts on: the given one, or the newest undoable one. */
export async function findUndoTarget(db: DecayDB, sweepId?: string): Promise<Sweep> {
  const recent = await undoWindow(db);
  const sweep = sweepId ? await db.sweeps.get(sweepId) : recent.find((s) => canUndo(s, recent));
  if (!sweep) throw new UndoError('NOT_FOUND', 'There is no sweep to undo.');
  if (!canUndo(sweep, recent)) {
    throw new UndoError(
      'NOT_UNDOABLE',
      sweep.status === 'undone'
        ? 'This sweep was already undone.'
        : 'Only the last 20 sweeps can be undone.',
    );
  }
  return sweep;
}

/**
 * Undo a sweep (PRD F7): reopens every item of the sweep that is still archived.
 * The sweep becomes `undone` once nothing of it is left archived; if some tabs failed
 * to open, it stays `done` so Undo can be retried for the rest.
 *
 * An undo attempt is marked on the sweep before any tab opens. If a previous attempt was
 * interrupted (worker killed after opening tabs, before recording them), records whose
 * URL is already open are linked to that tab instead of being opened a second time.
 */
export async function undoSweep(
  db: DecayDB,
  commands: TabCommands,
  opts: {
    appId: string;
    now: () => number;
    sweepId?: string;
    getTabs: () => Promise<readonly TabInfo[]>;
  },
): Promise<RestoreResult> {
  const sweep = await findUndoTarget(db, opts.sweepId);
  const okIds = sweep.items.filter((e) => e.result === 'ok').map((e) => e.itemId);
  const records = (await db.items.bulkGet(okIds)).filter(
    (i): i is Item => i !== undefined && i.state === 'archived' && i.lastSweepId === sweep.id,
  );
  const windowOf = new Map(sweep.items.map((e) => [e.itemId, e.windowExternalId]));
  const session = await commands.browserSession();
  const sameSession = sweep.browserSession !== undefined && sweep.browserSession === session.id;

  const adopted: Opened[] = [];
  let toOpen = records;
  if (sweep.undoStartedAt !== undefined) {
    const open = [...(await opts.getTabs())];
    toOpen = [];
    for (const record of records) {
      const i = open.findIndex((t) => tabUrl(t) === record.url);
      const tab = i >= 0 ? open.splice(i, 1)[0] : undefined;
      if (tab) adopted.push({ item: record, tab });
      else toOpen.push(record);
    }
  }
  await db.sweeps.update(sweep.id, { undoStartedAt: opts.now() });
  const { opened, failed } = await reopenTabs(toOpen, windowOf, commands, sameSession);
  opened.push(...adopted);
  const now = opts.now();
  await recordRestores(db, opened, { appId: opts.appId, now, sweepId: sweep.id, asUndo: true });
  await db.sweeps.update(
    sweep.id,
    failed.length
      ? { undoStartedAt: undefined }
      : { status: 'undone', undoneAt: now, undoStartedAt: undefined },
  );
  return {
    restored: opened.length,
    failed: failed.length,
    tabIds: opened.map((o) => o.tab.id).filter((id): id is number => id !== undefined),
  };
}
