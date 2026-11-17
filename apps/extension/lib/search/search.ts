import { formatDate } from '@decay/ui';
import MiniSearch from 'minisearch';
import { t } from '@/lib/strings';
import type { Item } from '../db/types';
import { localDay } from '../shadow/shadow';

/**
 * Archive search (P0.6, PRD F5). Phase 0 stand-in for the FTS5 search in Rust: a MiniSearch
 * index over archived items, rebuilt when the popup opens (≤ 20k items, DB Design §7).
 * Prefix + fuzzy matching, inline filters `site:`, `app:`, `before:`, `after:`, and a recency
 * boost `1 / (1 + days / 7)` (DB Design §3.4).
 */

export interface ParsedQuery {
  text: string;
  site?: string;
  appId?: string;
  /** Archived at or after this time. */
  after?: number;
  /** Archived before this time. */
  before?: number;
}

export interface SearchGroup {
  /** `day|sweep|window` — also the batch key. */
  key: string;
  day: string;
  /** "Today", "Yesterday", "Mon, 2 Nov". */
  dayLabel: string;
  /** Shared tab group title, or the sweep time. */
  sourceLabel: string;
  items: Item[];
}

export interface SearchResult {
  groups: SearchGroup[];
  /** Matching items in rank order (keyboard navigation). */
  items: Item[];
  total: number;
  tookMs: number;
}

const APP_ALIASES: Record<string, string> = {
  chrome: 'com.google.Chrome',
  edge: 'com.microsoft.Edge',
  brave: 'com.brave.Browser',
};

const DAY_MS = 86_400_000;

function startOfDay(ts: number): number {
  const d = new Date(ts);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

/** Start of the local day `days` calendar days before `ts` (DST-safe: not 24 h steps). */
function daysBefore(ts: number, days: number): number {
  const d = new Date(ts);
  d.setDate(d.getDate() - days);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

/**
 * A date operand: `YYYY-MM-DD`, `today`, `yesterday`, or relative `3d` / `2w` / `12h`.
 * Calendar dates mean the start of that local day; relative ones mean "that long ago".
 */
export function parseDate(value: string, now: number): number | undefined {
  const v = value.toLowerCase();
  if (v === 'today') return startOfDay(now);
  if (v === 'yesterday') return daysBefore(now, 1);
  const rel = /^(\d{1,4})([hdw])$/.exec(v);
  if (rel) {
    const n = Number(rel[1]);
    const unit = rel[2] === 'h' ? 3_600_000 : rel[2] === 'd' ? DAY_MS : 7 * DAY_MS;
    return now - n * unit;
  }
  const abs = /^(\d{4})-(\d{1,2})-(\d{1,2})$/.exec(v);
  if (abs) {
    const d = new Date(Number(abs[1]), Number(abs[2]) - 1, Number(abs[3]));
    if (d.getMonth() === Number(abs[2]) - 1) return d.getTime();
  }
  return undefined;
}

/** Splits inline filters from the free text. Unknown or invalid operators stay as text. */
export function parseQuery(raw: string, now: number): ParsedQuery {
  const out: ParsedQuery = { text: '' };
  const words: string[] = [];
  for (const token of raw.trim().split(/\s+/)) {
    const m = /^(site|app|before|after):(.+)$/i.exec(token);
    const op = m?.[1]?.toLowerCase();
    const value = m?.[2] ?? '';
    if (op === 'site') {
      out.site = value.toLowerCase().replace(/^www\./, '');
    } else if (op === 'app' && APP_ALIASES[value.toLowerCase()]) {
      out.appId = APP_ALIASES[value.toLowerCase()];
    } else if ((op === 'before' || op === 'after') && parseDate(value, now) !== undefined) {
      out[op] = parseDate(value, now);
    } else if (token) {
      words.push(token);
    }
  }
  out.text = words.join(' ');
  return out;
}

/** URL words: `stripe.com/pricing?plan=pro` → `stripe com pricing plan pro`. */
export function urlWords(url: string | undefined): string {
  if (!url) return '';
  return url
    .replace(/^[a-z-]+:\/\//i, '')
    .split(/[/.?=&#:+%_-]+/)
    .filter(Boolean)
    .join(' ');
}

const hostOf = (url: string | undefined) => {
  try {
    return url ? new URL(url).hostname.toLowerCase().replace(/^www\./, '') : '';
  } catch {
    return '';
  }
};

export function matchesFilters(item: Item, q: ParsedQuery): boolean {
  if (q.appId && item.appId !== q.appId) return false;
  const at = item.archivedAt ?? 0;
  if (q.after !== undefined && at < q.after) return false;
  if (q.before !== undefined && at >= q.before) return false;
  if (q.site) {
    const host = hostOf(item.url);
    const exact = host === q.site || host.endsWith(`.${q.site}`) || item.domain === q.site;
    // `site:github` (no dot) matches any host with that label: github.com, docs.github.io.
    const label = !q.site.includes('.') && host.split('.').includes(q.site);
    if (!exact && !label) return false;
  }
  return true;
}

export class ArchiveSearch {
  private readonly index: MiniSearch<Item>;
  private readonly byId: Map<string, Item>;
  /** item id → original window at archive time (from the sweep), for grouping. */
  private readonly windowOf: ReadonlyMap<string, string | undefined>;

  constructor(
    items: readonly Item[],
    windowOf: ReadonlyMap<string, string | undefined> = new Map(),
  ) {
    const archived = items.filter((i) => i.state === 'archived');
    this.byId = new Map(archived.map((i) => [i.id, i]));
    this.windowOf = windowOf;
    this.index = new MiniSearch<Item>({
      fields: ['title', 'url', 'domain', 'groupTitle'],
      extractField: (doc, field) => {
        if (field === 'url') return urlWords(doc.url);
        const value = doc[field as keyof Item];
        return typeof value === 'string' ? value : '';
      },
      searchOptions: {
        prefix: true,
        // Max edits: 1 for 4–5 letters, 2 from 6 (a swapped pair like "stirpe" is 2 edits).
        fuzzy: (term) => (term.length >= 6 ? 2 : term.length > 3 ? 1 : 0),
        boost: { title: 3, domain: 2, groupTitle: 2 },
        combineWith: 'AND',
      },
    });
    this.index.addAll(archived);
  }

  get size(): number {
    return this.byId.size;
  }

  search(raw: string, now: number, limit = 50): SearchResult {
    const started = performance.now();
    const q = parseQuery(raw, now);
    let ranked: Item[];
    if (q.text) {
      const recency = (item: Item) => 1 / (1 + (now - (item.archivedAt ?? now)) / (7 * DAY_MS));
      ranked = this.index
        .search(q.text)
        .map((hit) => ({ item: this.byId.get(String(hit.id)), score: hit.score }))
        .filter((h): h is { item: Item; score: number } => !!h.item && matchesFilters(h.item, q))
        .map((h) => ({ item: h.item, score: h.score * (1 + recency(h.item)) }))
        .sort((a, b) => b.score - a.score)
        .map((h) => h.item);
    } else {
      ranked = [...this.byId.values()]
        .filter((i) => matchesFilters(i, q))
        .sort((a, b) => (b.archivedAt ?? 0) - (a.archivedAt ?? 0));
    }
    const total = ranked.length;
    const top = ranked.slice(0, limit);
    const groups = groupItems(top, now, this.windowOf);
    return {
      groups,
      // Navigation order follows the groups as displayed.
      items: groups.flatMap((g) => g.items),
      total,
      tookMs: performance.now() - started,
    };
  }
}

export function dayLabel(ts: number, now: number): string {
  const day = localDay(ts);
  if (day === localDay(now)) return t('day.today');
  if (day === localDay(daysBefore(now, 1))) return t('day.yesterday');
  return formatDate(ts, { weekday: 'short', day: 'numeric', month: 'short' });
}

export const timeLabel = (ts: number) => formatDate(ts, { hour: 'numeric', minute: '2-digit' });

/**
 * Groups by batch (same sweep, same original window — exactly what "Restore batch"
 * restores, PRD S4), in order of each group's best item. Labelled with the sweep's day.
 */
export function groupItems(
  items: readonly Item[],
  now: number,
  windowOf: ReadonlyMap<string, string | undefined>,
): SearchGroup[] {
  const groups = new Map<string, SearchGroup>();
  for (const item of items) {
    const at = item.archivedAt ?? 0;
    const day = localDay(at);
    const key = `${item.lastSweepId ?? item.id}|${windowOf.get(item.id) ?? ''}`;
    let group = groups.get(key);
    if (!group) {
      group = { key, day, dayLabel: dayLabel(at, now), sourceLabel: '', items: [] };
      groups.set(key, group);
    }
    group.items.push(item);
  }
  // One sweep can archive from several windows: number them so batches are distinguishable.
  const perSweep = new Map<string, SearchGroup[]>();
  for (const group of groups.values()) {
    const sweepKey = group.key.split('|')[0] ?? '';
    perSweep.set(sweepKey, [...(perSweep.get(sweepKey) ?? []), group]);
  }
  for (const group of groups.values()) {
    const titles = new Set(group.items.map((i) => i.groupTitle ?? ''));
    const [only] = titles;
    const first = group.items[0];
    group.sourceLabel =
      titles.size === 1 && only
        ? t('search.group', { title: only })
        : t('search.sweepAt', { time: timeLabel(first?.archivedAt ?? 0) });
    const siblings = perSweep.get(group.key.split('|')[0] ?? '') ?? [];
    if (siblings.length > 1) {
      group.sourceLabel = t('search.window', {
        label: group.sourceLabel,
        n: siblings.indexOf(group) + 1,
      });
    }
  }
  return [...groups.values()];
}
