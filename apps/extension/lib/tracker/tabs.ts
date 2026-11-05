import { getDomain, getHostname } from 'tldts';
import { uuidv7 } from 'uuidv7';
import type { Item, ItemEvent } from '../db/types';

/** The subset of `chrome.tabs.Tab` the tracker uses. */
export interface TabInfo {
  id?: number;
  windowId: number;
  index: number;
  url?: string;
  pendingUrl?: string;
  title?: string;
  favIconUrl?: string;
  pinned: boolean;
  audible?: boolean;
  active: boolean;
  groupId?: number;
  incognito: boolean;
  status?: string; // 'loading' | 'complete' | 'unloaded'
}

export interface WindowInfo {
  id?: number;
  state?: string; // 'normal' | 'minimized' | 'maximized' | 'fullscreen' | …
  focused: boolean;
  incognito: boolean;
}

export interface GroupInfo {
  id: number;
  title?: string;
  color: string;
}

/** Minutes between sampled `used` events for one item (Docs/DB Design.md §3.5). */
export const USED_EVENT_SAMPLE_MS = 10 * 60_000;

export function tabUrl(tab: TabInfo): string | undefined {
  return tab.url || tab.pendingUrl || undefined;
}

/** eTLD+1, lowercase, for http(s) URLs; the bare host for localhost/IPs; else undefined. */
export function domainOf(url: string | undefined): string | undefined {
  if (!url || !/^https?:\/\//i.test(url)) return undefined;
  // Private suffixes (github.io, vercel.app, …) count, so each hosted site is its own domain.
  const domain = getDomain(url, { allowPrivateDomains: true }) ?? getHostname(url);
  return domain ? domain.toLowerCase() : undefined;
}

/** Fields of an item that mirror the tab's current state. */
export function tabFields(tab: TabInfo, groups: ReadonlyMap<number, GroupInfo>) {
  const url = tabUrl(tab);
  const group = tab.groupId !== undefined && tab.groupId >= 0 ? groups.get(tab.groupId) : undefined;
  return {
    externalId: String(tab.id),
    windowExternalId: String(tab.windowId),
    title: tab.title ?? '',
    url,
    domain: domainOf(url),
    faviconUrl: tab.favIconUrl || undefined,
    tabIndex: tab.index,
    pinned: tab.pinned,
    groupTitle: group?.title || undefined,
    groupColor: group?.color,
  } satisfies Partial<Item>;
}

export function newItemFromTab(
  tab: TabInfo,
  appId: string,
  groups: ReadonlyMap<number, GroupInfo>,
  now: number,
): Item {
  return {
    id: uuidv7(),
    kind: 'tab',
    appId,
    ...tabFields(tab, groups),
    state: 'active',
    activeAgeS: 0,
    firstSeenAt: now,
    lastUsedAt: now,
    stateChangedAt: now,
    restoreCount: 0,
  };
}

/** Writes the tab's current fields onto the item. Returns true if anything changed. */
export function applyTabFields(
  item: Item,
  tab: TabInfo,
  groups: ReadonlyMap<number, GroupInfo>,
): boolean {
  let changed = false;
  for (const [key, value] of Object.entries(tabFields(tab, groups))) {
    const k = key as keyof Item;
    if (item[k] !== value) {
      (item as unknown as Record<string, unknown>)[k] = value;
      changed = true;
    }
  }
  return changed;
}

/**
 * Tabs in use right now: the active tab of every non-minimized window (visible),
 * plus every audible tab. In-use tabs don't age (Docs/Software Design.md §4.2).
 */
export function inUseTabIds(tabs: readonly TabInfo[], windows: readonly WindowInfo[]): Set<string> {
  const minimized = new Set(windows.filter((w) => w.state === 'minimized').map((w) => w.id));
  const ids = new Set<string>();
  for (const tab of tabs) {
    if (tab.id === undefined || tab.incognito) continue;
    if ((tab.active && !minimized.has(tab.windowId)) || tab.audible) ids.add(String(tab.id));
  }
  return ids;
}

/**
 * Marks an item as used: age back to 0, state back to `active`.
 * Emits a `used` event at most once per item per 10 min.
 */
export function markUsed(item: Item, now: number): ItemEvent | undefined {
  let event: ItemEvent | undefined;
  if (item.lastUsedEventAt === undefined || now - item.lastUsedEventAt >= USED_EVENT_SAMPLE_MS) {
    event = { itemId: item.id, type: 'used', at: now };
    item.lastUsedEventAt = now;
  }
  item.activeAgeS = 0;
  item.lastUsedAt = now;
  if (item.state === 'stale') {
    item.state = 'active';
    item.stateChangedAt = now;
  }
  return event;
}
