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
): Promise<number> {
  const { now } = opts;
  return db.transaction('rw', [db.items, db.events, db.metrics], async () => {
    const events: ItemEvent[] = [];
    let regrets = 0;
    let recorded = 0;
    for (const { item: snapshot, tab } of opened) {
      const record = await db.items.get(snapshot.id);
      // Re-checked inside the transaction: a concurrent restore already counted it.
      if (record?.state !== 'archived') continue;
      recorded++;
      if (record.archivedAt !== undefined && now - record.archivedAt <= REGRET_WINDOW_MS) regrets++;
      await db.items.update(record.id, {
        state: 'restored',
        stateChangedAt: now,
        restoreCount: (record.restoreCount ?? 0) + 1,
        restoreStartedAt: undefined,
      });
      events.push({
        itemId: record.id,
        type: opts.asUndo ? 'undo' : 'restored',
        sweepId: opts.sweepId,
        at: now,
      });
      // Private windows are never tracked (like `upsertTab`).
      if (tab.id === undefined || tab.incognito) continue;
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
    if (recorded) {
      const day = localDay(now);
      const row = (await db.metrics.get(day)) ?? emptyMetrics(day);
      row.restored24hCount = (row.restored24hCount ?? 0) + regrets;
      if (opts.asUndo) row.undoCount = (row.undoCount ?? 0) + 1;
      await db.metrics.put(row);
    }
    return recorded;
  });
}

/** The sweep Undo acts on: the given one, or the newest undoable one. */
export async function findUndoTarget(db: DecayDB, sweepId?: string): Promise<Sweep> {
  const recent = await undoWindow(db);
  let sweep: Sweep | undefined;
  if (sweepId) sweep = await db.sweeps.get(sweepId);
  else {
    // The newest sweep that still has archived tabs (one may have been restored by hand).
    for (const s of recent) {
      if (!canUndo(s, recent)) continue;
      const ids = s.items.filter((e) => e.result === 'ok').map((e) => e.itemId);
      const items = await db.items.bulkGet(ids);
      if (items.some((i) => i?.state === 'archived' && i.lastSweepId === s.id)) {
        sweep = s;
        break;
      }
    }
  }
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
  if (!records.length) {
    throw new UndoError('NOT_UNDOABLE', 'Everything from this sweep is already open again.');
  }
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

export interface RestoreItemsResult extends RestoreResult {
  /** Requested items that are already open again (restored earlier). */
  skipped: number;
  /** Records restored within 24 h of archiving (regret): candidates for "Always keep site". */
  regretted: Item[];
}

/**
 * Restore single items or a batch (PRD F6, Software Design §4.5).
 * - `intoWindowId`: items reopen in the window the user is in, the first one active
 *   (search → Enter);
 * - `newWindow`: one new window per original window, in the original order (batch, S4);
 * - otherwise like an undo: the original window within the same browser session.
 *
 * Crash-safe like undo: records are marked before tabs open, and a retry after an
 * interrupted attempt links an already-open tab with the same URL instead of opening a
 * second one.
 */
export async function restoreItems(
  db: DecayDB,
  commands: TabCommands,
  opts: {
    appId: string;
    now: () => number;
    ids: readonly string[];
    intoWindowId?: number;
    newWindow?: boolean;
    getTabs: () => Promise<readonly TabInfo[]>;
  },
): Promise<RestoreItemsResult> {
  const unique = [...new Set(opts.ids)];
  const found = await db.items.bulkGet(unique);
  const records = found.filter((i): i is Item => i?.state === 'archived');
  const skipped = found.filter((i) => i !== undefined && i.state !== 'archived').length;
  let failed = unique.length - records.length - skipped;

  // A previous attempt was interrupted: adopt tabs it already opened.
  const adopted: Opened[] = [];
  let toOpen = records;
  if (records.some((r) => r.restoreStartedAt !== undefined)) {
    const open = [...(await opts.getTabs())];
    toOpen = [];
    for (const record of records) {
      const i =
        record.restoreStartedAt !== undefined
          ? open.findIndex((t) => tabUrl(t) === record.url)
          : -1;
      const tab = i >= 0 ? open.splice(i, 1)[0] : undefined;
      if (tab) adopted.push({ item: record, tab });
      else toOpen.push(record);
    }
  }
  const startedAt = opts.now();
  await db.items.bulkUpdate(
    toOpen.map((r) => ({ key: r.id, changes: { restoreStartedAt: startedAt } })),
  );

  const opened: Opened[] = [...adopted];
  const notOpened: Item[] = [];
  if (opts.intoWindowId !== undefined) {
    for (const [i, record] of toOpen.entries()) {
      try {
        if (!record.url) throw new Error('no url');
        const tab = await commands.createTab({
          windowId: opts.intoWindowId,
          url: record.url,
          active: i === 0,
        });
        opened.push({ item: record, tab });
      } catch {
        notOpened.push(record);
      }
    }
  } else {
    const session = await commands.browserSession();
    const bySweep = new Map<string, Item[]>();
    for (const record of toOpen) {
      const key = record.lastSweepId ?? '';
      bySweep.set(key, [...(bySweep.get(key) ?? []), record]);
    }
    for (const [sweepId, list] of bySweep) {
      const sweep = sweepId ? await db.sweeps.get(sweepId) : undefined;
      const windowOf = new Map(sweep?.items.map((e) => [e.itemId, e.windowExternalId]) ?? []);
      const sameSession =
        !opts.newWindow &&
        sweep?.browserSession !== undefined &&
        sweep.browserSession === session.id;
      const r = await reopenTabs(list, windowOf, commands, sameSession);
      opened.push(...r.opened);
      notOpened.push(...r.failed);
    }
  }
  const now = opts.now();
  const recorded = await recordRestores(db, opened, { appId: opts.appId, now, asUndo: false });
  if (notOpened.length) {
    await db.items.bulkUpdate(
      notOpened.map((r) => ({ key: r.id, changes: { restoreStartedAt: undefined } })),
    );
  }
  failed += notOpened.length;
  return {
    restored: recorded,
    failed,
    skipped: skipped + (opened.length - recorded),
    tabIds: opened.map((o) => o.tab.id).filter((id): id is number => id !== undefined),
    regretted: opened
      .map((o) => o.item)
      .filter((i) => i.archivedAt !== undefined && now - i.archivedAt <= REGRET_WINDOW_MS),
  };
}

/** The batch an archived item belongs to: still-archived items of its sweep from the same window. */
export async function findBatch(db: DecayDB, itemId: string): Promise<Item[]> {
  const item = await db.items.get(itemId);
  if (item?.state !== 'archived') return [];
  if (!item.lastSweepId) return [item];
  const sweep = await db.sweeps.get(item.lastSweepId);
  const window = sweep?.items.find((e) => e.itemId === itemId)?.windowExternalId;
  const ids = sweep?.items
    .filter((e) => e.result === 'ok' && e.windowExternalId === window)
    .map((e) => e.itemId) ?? [itemId];
  const items = await db.items.bulkGet(ids);
  return items.filter(
    (i): i is Item =>
      i !== undefined && i.state === 'archived' && i.lastSweepId === item.lastSweepId,
  );
}
