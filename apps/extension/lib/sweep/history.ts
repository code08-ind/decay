import type { DecayDB } from '../db/db';
import type { Item, Mode, Sweep, SweepItem } from '../db/types';
import { canUndo, recentSweeps, undoWindow } from './executor';

/** How many sweeps the Sweeps page lists. */
export const HISTORY_LIMIT = 50;

export type SweepStatusLabel = 'running' | 'archived' | 'partial' | 'undone' | 'nothing';

export interface SweepEntryItem {
  entry: SweepItem;
  item?: Item;
}

export interface SweepEntry {
  sweep: Sweep;
  /** 1-based sequence number among all real sweeps ("Sweep #12"). */
  number: number;
  /** Tabs this sweep closed. */
  archived: number;
  /** Of those, how many have been reopened (by Undo or a single restore). Deleted records don't count. */
  restored: number;
  /** Closed tabs that are still archived (what Undo would reopen). */
  remaining: number;
  status: SweepStatusLabel;
  undoable: boolean;
  /** Archived items first, in the order they were closed; then skipped ones. */
  items: SweepEntryItem[];
}

export interface SweepHistory {
  mode: Mode;
  entries: SweepEntry[];
}

export function describeSweep(
  sweep: Sweep,
  number: number,
  itemsById: ReadonlyMap<string, Item>,
  recent: readonly Sweep[],
): SweepEntry {
  const items = sweep.items.map((entry) => ({ entry, item: itemsById.get(entry.itemId) }));
  const closed = items.filter((i) => i.entry.result === 'ok');
  const remaining = closed.filter(
    (i) => i.item?.state === 'archived' && i.item.lastSweepId === sweep.id,
  ).length;
  const restored = closed.filter((i) => i.item?.state === 'restored').length;
  let status: SweepStatusLabel;
  if (sweep.status === 'running' || sweep.status === 'pending') status = 'running';
  else if (closed.length === 0) status = 'nothing';
  else if (sweep.status === 'undone' || (remaining === 0 && restored > 0)) status = 'undone';
  else if (remaining === 0)
    status = 'nothing'; // records purged since (retention, rules)
  else if (restored > 0) status = 'partial';
  else status = 'archived';
  items.sort((a, b) => Number(b.entry.result === 'ok') - Number(a.entry.result === 'ok'));
  return {
    sweep,
    number,
    archived: closed.length,
    restored,
    remaining,
    status,
    undoable: remaining > 0 && canUndo(sweep, recent),
    items,
  };
}

/** The newest real sweeps with their items, for the Sweeps page (Designs/07). */
export async function loadSweepHistory(db: DecayDB, mode: Mode): Promise<SweepHistory> {
  const [sweeps, total, window] = await Promise.all([
    recentSweeps(db, HISTORY_LIMIT),
    db.sweeps.where('isShadow').equals(0).count(),
    undoWindow(db),
  ]);
  const ids = [...new Set(sweeps.flatMap((s) => s.items.map((e) => e.itemId)))];
  const items = (await db.items.bulkGet(ids)).filter((i): i is Item => i !== undefined);
  const byId = new Map(items.map((i) => [i.id, i]));
  return {
    mode,
    entries: sweeps.map((s, i) => describeSweep(s, total - i, byId, window)),
  };
}
