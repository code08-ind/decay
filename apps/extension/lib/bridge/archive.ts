// The popup's view of the app's archive (adapter mode, P1.5). While the Decay app is
// connected, the archive lives there: search, restore, batch restore and Undo go to the app
// (`archive.*` requests), so tabs the app archived are found and reopened too. These
// helpers turn the app's answers into the shapes the popup already renders.

import type { Item, ItemState, Reason, Sweep } from '../db/types';
import { type PopupData, RECENT_COUNT } from '../popup/data';
import { groupItems, type SearchResult } from '../search/search';
import type { ArchivedItem, ArchiveSearchResult, LastSweep } from './protocol';

/** An app item as the popup's `Item`. */
export function fromApp(item: ArchivedItem): Item {
  return {
    id: item.id,
    kind: item.kind,
    appId: item.appId,
    title: item.title,
    ...(item.url !== undefined ? { url: item.url } : {}),
    ...(item.domain !== undefined ? { domain: item.domain } : {}),
    ...(item.faviconUrl !== undefined ? { faviconUrl: item.faviconUrl } : {}),
    ...(item.tabIndex !== undefined ? { tabIndex: item.tabIndex } : {}),
    ...(item.groupTitle !== undefined ? { groupTitle: item.groupTitle } : {}),
    ...(item.groupColor !== undefined ? { groupColor: item.groupColor } : {}),
    pinned: item.pinned,
    state: item.state as ItemState,
    activeAgeS: item.activeAgeS,
    firstSeenAt: item.firstSeenAt,
    lastUsedAt: item.lastUsedAt,
    stateChangedAt: item.stateChangedAt,
    ...(item.archivedAt !== undefined ? { archivedAt: item.archivedAt } : {}),
    ...(item.archiveReason !== undefined ? { archiveReason: item.archiveReason as Reason } : {}),
    ...(item.lastSweepId !== undefined ? { lastSweepId: item.lastSweepId } : {}),
    restoreCount: item.restoreCount,
  };
}

/** The app's results grouped by batch (same sweep, same original window), like Phase 0. */
export function appSearchResult(result: ArchiveSearchResult, now: number): SearchResult {
  const items = result.items.map(fromApp);
  const windowOf = new Map(result.items.map((i) => [i.id, i.windowKey]));
  const groups = groupItems(items, now, windowOf);
  return {
    groups,
    items: groups.flatMap((g) => g.items),
    total: items.length,
    tookMs: 0,
  };
}

/** The popup's "last sweep" row from the app's (only real sweeps can be undone there). */
export function appLastSweep(last: LastSweep | undefined): PopupData['lastSweep'] {
  if (!last) return null;
  const sweep: Sweep = {
    id: last.id,
    startedAt: last.startedAt,
    ...(last.finishedAt !== undefined ? { finishedAt: last.finishedAt } : {}),
    trigger: 'scheduled',
    isShadow: 0,
    // A sweep still settling can't be undone yet (the popup offers Undo for `done`).
    status: last.finishedAt !== undefined ? 'done' : 'running',
    itemCount: last.itemCount,
    failedCount: 0,
    items: [],
  };
  return { sweep, remaining: last.remaining };
}

/** Popup data with the app's archive in place of the local copy. */
export function withAppArchive(data: PopupData, result: ArchiveSearchResult): PopupData {
  const archived = result.items.map(fromApp);
  return {
    ...data,
    recent: archived.slice(0, RECENT_COUNT),
    archived,
    windowOf: new Map(result.items.map((i) => [i.id, i.windowKey])),
    lastSweep: appLastSweep(result.lastSweep),
    // The app decides while connected: its mode, not the extension's own.
    mode: result.mode ?? data.mode,
  };
}
