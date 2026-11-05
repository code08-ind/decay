import type { Item, ItemEvent } from '../db/types';
import {
  applyTabFields,
  type GroupInfo,
  markUsed,
  newItemFromTab,
  type TabInfo,
  tabUrl,
} from './tabs';

/**
 * An item whose tab is gone is only deleted after it has been missing this long
 * (and for at least two ticks). Session restore, "reopen closed window" and a slow
 * browser startup bring tabs back with new ids; waiting lets them reclaim their item.
 */
export const DELETE_AFTER_MISSING_MS = 60_000;

export interface ReconcileInput {
  /** Live items (states active/stale/hidden/archiving) for this browser. */
  live: Item[];
  /** All open, non-incognito tabs, taken at `snapshotAt`. */
  tabs: readonly TabInfo[];
  groups: ReadonlyMap<number, GroupInfo>;
  appId: string;
  now: number;
  snapshotAt: number;
  /**
   * Tab ids that event handlers wrote or removed after `snapshotAt`. The snapshot is
   * older than the DB for these, so the tick leaves them alone this time.
   */
  skipTabIds?: ReadonlySet<string>;
  /** item id → when it was first seen missing (kept in memory by the tracker). */
  missingSince?: ReadonlyMap<string, number>;
}

export interface ReconcileResult {
  /** Items backing the open tabs (matched or new); these are aged by the tick. */
  items: Item[];
  /** Items to write: new ones and ones whose fields changed. */
  dirty: Set<Item>;
  /** Items whose tab has been gone long enough, and that weren't archived: hard-deleted. */
  deleteIds: string[];
  /** Updated `missingSince` for items whose tab is gone but that are still kept. */
  missingSince: Map<string, number>;
  events: ItemEvent[];
}

/**
 * Matches the open tabs to the stored live items.
 *
 * 1. Same tab id **and** same URL → same item (normal case, also after a worker restart).
 * 2. Otherwise same URL → same item. Browsers assign new tab ids after a restart, and
 *    session restore reopens the same URLs, so this keeps ages across restarts.
 *    Among several candidates, the most recently used one wins.
 * 3. Unmatched tabs become new items. Unmatched items are deleted once they've been
 *    missing for `DELETE_AFTER_MISSING_MS`, except items being archived (the sweep
 *    executor owns them), items newer than the snapshot, and items handlers just touched.
 */
export function reconcile(input: ReconcileInput): ReconcileResult {
  const { live, tabs, groups, appId, now, snapshotAt } = input;
  const skip = input.skipTabIds ?? new Set<string>();
  const prevMissing = input.missingSince ?? new Map<string, number>();
  const events: ItemEvent[] = [];
  const dirty = new Set<Item>();
  const matched = new Map<TabInfo, Item>();

  const candidates = new Set(
    live.filter((item) => !(item.externalId && skip.has(item.externalId))),
  );
  const openTabs = tabs.filter((tab) => tab.id !== undefined && !skip.has(String(tab.id)));

  // Pass 1: tab id + URL.
  const byExternalId = new Map<string, Item>();
  for (const item of candidates) if (item.externalId) byExternalId.set(item.externalId, item);
  for (const tab of openTabs) {
    const item = byExternalId.get(String(tab.id));
    if (item && candidates.has(item) && item.url === tabUrl(tab)) {
      matched.set(tab, item);
      candidates.delete(item);
    }
  }

  // Pass 2: URL only.
  const byUrl = new Map<string | undefined, Item[]>();
  for (const item of candidates) {
    if (item.state === 'archiving') continue;
    const list = byUrl.get(item.url) ?? [];
    list.push(item);
    byUrl.set(item.url, list);
  }
  for (const list of byUrl.values()) list.sort((a, b) => b.lastUsedAt - a.lastUsedAt);
  for (const tab of openTabs) {
    if (matched.has(tab)) continue;
    const item = byUrl.get(tabUrl(tab))?.find((candidate) => candidates.has(candidate));
    if (item) {
      matched.set(tab, item);
      candidates.delete(item);
    }
  }

  const items: Item[] = [];
  for (const tab of openTabs) {
    const existing = matched.get(tab);
    if (existing) {
      if (applyTabFields(existing, tab, groups)) dirty.add(existing);
      items.push(existing);
    } else {
      const item = newItemFromTab(tab, appId, groups, now);
      events.push({ itemId: item.id, type: 'seen', at: now });
      dirty.add(item);
      items.push(item);
    }
  }

  const deleteIds: string[] = [];
  const missingSince = new Map<string, number>();
  for (const item of candidates) {
    if (item.state === 'archiving' || item.firstSeenAt >= snapshotAt) continue;
    const since = prevMissing.get(item.id);
    if (since !== undefined && now - since >= DELETE_AFTER_MISSING_MS) {
      deleteIds.push(item.id);
    } else {
      missingSince.set(item.id, since ?? now);
    }
  }
  return { items, dirty, deleteIds, missingSince, events };
}

/**
 * Applies one clock tick to the items backing open tabs: in-use items are marked
 * used (only while the user is active, i.e. credit > 0); the others age by `creditS`.
 * Items being archived are left alone. Changed items are added to `dirty`.
 */
export function applyTick(
  items: readonly Item[],
  inUse: ReadonlySet<string>,
  creditS: number,
  now: number,
  dirty: Set<Item> = new Set(),
): ItemEvent[] {
  const events: ItemEvent[] = [];
  if (creditS <= 0) return events;
  for (const item of items) {
    if (item.state === 'archiving') continue;
    if (item.externalId && inUse.has(item.externalId)) {
      const event = markUsed(item, now);
      if (event) events.push(event);
    } else {
      item.activeAgeS += creditS;
    }
    dirty.add(item);
  }
  return events;
}
