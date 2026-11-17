import { ACTIVE_DAY_SECONDS, median, rate } from './insights';
import type { MetricsDay } from './types';

/**
 * "Share with beta team" (F34, PRD §9): the anonymised daily rows and summary that both
 * the extension's export (format v1) and the app's (v2) carry. Counts and medians only —
 * never URLs, titles, sites, rule patterns or tab contents.
 */

export interface BetaDay {
  day: string;
  activeMinutes: number;
  /** ≥ 1 h of active browsing (an "active day"). */
  active: boolean;
  openTabsP50: number | null;
  openWindowsP50: number | null;
  archived: number;
  restoredWithin24h: number;
  undos: number;
  searches: number;
  searchToRestoreMsP50: number | null;
  autoModeOn: boolean;
  shadowFlagged: number;
  shadowRegret: number;
}

export interface BetaSummary {
  days: number;
  activeDays: number;
  archived: number;
  restoredWithin24h: number;
  regretRate: number | null;
  shadowFlagged: number;
  shadowRegretRate: number | null;
  undos: number;
  searches: number;
  searchToRestoreMsP50: number | null;
  openTabsP50: number | null;
}

const n = (v: unknown) => (typeof v === 'number' && Number.isFinite(v) ? v : 0);
const sampledOr = (samples: number[] | undefined, value: number) =>
  (samples?.length ?? 0) > 0 ? value : null;

/** One row per stored day, oldest first. */
export function betaDays(metrics: readonly MetricsDay[]): BetaDay[] {
  return [...metrics]
    .sort((a, b) => a.day.localeCompare(b.day))
    .map((m) => ({
      day: m.day,
      activeMinutes: Math.round(n(m.activeSeconds) / 60),
      active: n(m.activeSeconds) >= ACTIVE_DAY_SECONDS,
      openTabsP50: sampledOr(m.openTabsSamples, n(m.openTabsP50)),
      openWindowsP50: sampledOr(m.openWindowsSamples, n(m.openWindowsP50)),
      archived: n(m.archivedCount),
      restoredWithin24h: n(m.restored24hCount),
      undos: n(m.undoCount),
      searches: n(m.searchCount),
      searchToRestoreMsP50: sampledOr(m.searchToRestoreSamples, n(m.searchToRestoreMsP50)),
      autoModeOn: m.autoModeOn === 1,
      shadowFlagged: n(m.shadowFlaggedCount),
      shadowRegret: n(m.shadowRegretCount),
    }));
}

/** Totals over every day. */
export function betaSummary(days: readonly BetaDay[], metrics: readonly MetricsDay[]): BetaSummary {
  const total = (key: keyof BetaDay) =>
    days.reduce((s, d) => s + (typeof d[key] === 'number' ? (d[key] as number) : 0), 0);
  const archived = total('archived');
  const restored = total('restoredWithin24h');
  const flagged = total('shadowFlagged');
  const openTabs = days.map((d) => d.openTabsP50).filter((v): v is number => v !== null);
  const searchSamples = metrics.flatMap((m) => m.searchToRestoreSamples ?? []);
  return {
    days: days.length,
    activeDays: days.filter((d) => d.active).length,
    archived,
    restoredWithin24h: restored,
    regretRate: rate(restored, archived),
    shadowFlagged: flagged,
    shadowRegretRate: rate(total('shadowRegret'), flagged),
    undos: total('undos'),
    searches: total('searches'),
    searchToRestoreMsP50: searchSamples.length ? median(searchSamples) : null,
    openTabsP50: openTabs.length ? median(openTabs) : null,
  };
}

/** Daily rows as CSV (the same anonymised fields as the JSON export). */
export function toCsv(data: { days: readonly BetaDay[] }): string {
  const cols = [
    'day',
    'activeMinutes',
    'active',
    'openTabsP50',
    'openWindowsP50',
    'archived',
    'restoredWithin24h',
    'undos',
    'searches',
    'searchToRestoreMsP50',
    'autoModeOn',
    'shadowFlagged',
    'shadowRegret',
  ] as const;
  const lines = data.days.map((d) =>
    cols.map((c) => (d[c] === null ? '' : String(d[c]))).join(','),
  );
  return `${[cols.join(','), ...lines].join('\n')}\n`;
}
