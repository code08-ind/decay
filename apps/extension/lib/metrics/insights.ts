import type { Item, MetricsDay, Mode } from '../db/types';
import { ACTIVE_DAY_SECONDS, localDay } from '../shadow/shadow';
import { median } from './metrics';

/** Insights page (P0.8, F34, Designs/14): on-device success metrics for a time range. */

export const RANGES = [7, 14, 30] as const;
export type RangeDays = (typeof RANGES)[number];

/** PRD §9 targets. */
export const TARGETS = {
  regretRate: 0.05,
  searchToRestoreMs: 5_000,
  openTabsDrop: 0.5,
  autoModeDays: 14,
} as const;

export type GoalStatus = 'met' | 'notYet' | 'noData';

export interface Goal {
  id: 'regret' | 'openTabs' | 'search' | 'automatic';
  label: string;
  value: string;
  target: string;
  status: GoalStatus;
}

export interface Insights {
  range: RangeDays;
  /** One entry per calendar day in the range, oldest first; null = no samples that day. */
  days: { day: string; openTabs: number | null }[];
  /** Median of the daily open-tab medians in the range. */
  openTabs: number | null;
  /** Median open tabs in week 1: the 7 calendar days from the first sample. */
  firstWeekOpenTabs: number | null;
  /** Median open tabs in the last 7 calendar days. */
  lastWeekOpenTabs: number | null;
  /**
   * (last week − week 1) / week 1, once the first sample is ≥ 14 days old (PRD §9 "after
   * 14 days vs week 1"). Fixed windows: doesn't change with the range control.
   */
  openTabsChange: number | null;
  /** First day of the current unbroken run of automatic mode (null when off). */
  automaticSince: string | null;
  archived: number;
  restored24h: number;
  /** restored24h / archived; null without archives. */
  regretRate: number | null;
  /** Shadow mode: flagged tabs used again within 24 h / flagged. */
  shadowRegretRate: number | null;
  shadowFlagged: number;
  searches: number;
  searchToRestoreMs: number | null;
  undos: number;
  activeDays: number;
  topArchived: { domain: string; count: number }[];
  restoredMost: { domain: string; count: number }[];
  goals: Goal[];
}

const DAY_MS = 24 * 3600_000;

/** Whole calendar days from `a` to `b` (YYYY-MM-DD, local; DST-safe via noon). */
export function daysBetween(a: string, b: string): number {
  const noon = (d: string) => new Date(`${d}T12:00:00`).getTime();
  return Math.round((noon(b) - noon(a)) / DAY_MS);
}

/**
 * First day of the current unbroken run of automatic mode: active days with automatic mode
 * off break the run; days without activity don't (a weekend away keeps it going).
 */
export function automaticSince(metrics: readonly MetricsDay[], mode: Mode): string | null {
  if (mode !== 'automatic') return null;
  let start: string | null = null;
  for (const r of [...metrics].sort((a, b) => a.day.localeCompare(b.day))) {
    if (r.autoModeOn === 1) start ??= r.day;
    else if ((r.activeSeconds ?? 0) > 0) start = null;
  }
  return start;
}

/** Calendar days ending today, oldest first (local time, DST-safe via noon anchoring). */
export function dayRange(now: number, days: number): string[] {
  const noon = new Date(now);
  noon.setHours(12, 0, 0, 0);
  return Array.from({ length: days }, (_, i) => localDay(noon.getTime() - (days - 1 - i) * DAY_MS));
}

const sum = (rows: readonly MetricsDay[], key: keyof MetricsDay) =>
  rows.reduce((n, r) => n + (Number(r[key]) || 0), 0);

function topDomains(entries: Iterable<[string | undefined, number]>, limit = 5) {
  const counts = new Map<string, number>();
  for (const [domain, n] of entries)
    if (domain && n > 0) counts.set(domain, (counts.get(domain) ?? 0) + n);
  return [...counts]
    .map(([domain, count]) => ({ domain, count }))
    .sort((a, b) => b.count - a.count || a.domain.localeCompare(b.domain))
    .slice(0, limit);
}

/**
 * Restored-within-24 h / archived for the same days, capped at 100%. Approximation: restores
 * count on the day of the restore and archives on the day of the archive, so a tab archived
 * just before the range and restored inside it counts only in the numerator.
 */
export const rate = (num: number, den: number): number | null =>
  den > 0 ? Math.min(1, num / den) : null;

const pct = (r: number) => `${r > 0 && r < 0.01 ? '<1' : Math.round(r * 100)}%`;

export function buildInsights(input: {
  now: number;
  range: RangeDays;
  mode: Mode;
  metrics: readonly MetricsDay[];
  items: readonly Item[];
}): Insights {
  const { now, range } = input;
  const days = dayRange(now, range);
  const since = days[0] ?? localDay(now);
  const byDay = new Map(input.metrics.map((m) => [m.day, m]));
  const rows = days.map((d) => byDay.get(d)).filter((r): r is MetricsDay => r !== undefined);
  const sampled = (r: MetricsDay | undefined) =>
    r && (r.openTabsSamples?.length ?? 0) > 0 ? r.openTabsP50 : null;

  const series = days.map((day) => ({ day, openTabs: sampled(byDay.get(day)) }));
  const inRange = series.map((s) => s.openTabs).filter((n): n is number => n !== null);
  const openTabs = inRange.length ? median(inRange) : null;
  // Goals use fixed windows (PRD §9), independent of the range control.
  const today = localDay(now);
  const allSampled = [...input.metrics]
    .sort((a, b) => a.day.localeCompare(b.day))
    .filter((r) => sampled(r) !== null);
  const firstDay = allSampled[0]?.day;
  const firstWeek = firstDay
    ? allSampled.filter((r) => daysBetween(firstDay, r.day) < 7).map((r) => r.openTabsP50)
    : [];
  const firstWeekOpenTabs = firstWeek.length ? median(firstWeek) : null;
  const lastWeek = allSampled
    .filter((r) => daysBetween(r.day, today) < 7)
    .map((r) => r.openTabsP50);
  const lastWeekOpenTabs = lastWeek.length ? median(lastWeek) : null;
  const openTabsChange =
    firstDay &&
    daysBetween(firstDay, today) >= TARGETS.autoModeDays &&
    firstWeekOpenTabs !== null &&
    firstWeekOpenTabs > 0 &&
    lastWeekOpenTabs !== null
      ? (lastWeekOpenTabs - firstWeekOpenTabs) / firstWeekOpenTabs
      : null;
  const last14 = dayRange(now, 14)
    .map((d) => byDay.get(d))
    .filter((r): r is MetricsDay => r !== undefined);

  const archived = sum(rows, 'archivedCount');
  const restored24h = sum(rows, 'restored24hCount');
  const shadowFlagged = sum(rows, 'shadowFlaggedCount');
  const shadowRegret = sum(rows, 'shadowRegretCount');
  const searchSamples = rows.flatMap((r) => r.searchToRestoreSamples ?? []);
  const searchToRestoreMs = searchSamples.length ? median(searchSamples) : null;

  const autoSince = automaticSince(input.metrics, input.mode);
  const autoDays = autoSince ? daysBetween(autoSince, today) : 0;

  const sinceTs = new Date(`${since}T00:00:00`).getTime();
  const topArchived = topDomains(
    input.items
      .filter((i) => i.archivedAt !== undefined && i.archivedAt >= sinceTs)
      .map((i): [string | undefined, number] => [i.domain, 1]),
  );
  // All time (includes Undo): sites worth protecting.
  const restoredMost = topDomains(
    input.items.map((i): [string | undefined, number] => [i.domain, i.restoreCount ?? 0]),
  );

  const regretRate = rate(restored24h, archived);
  const shadowRegretRate = rate(shadowRegret, shadowFlagged);
  // Goals: last 14 days.
  const gArchived = sum(last14, 'archivedCount');
  const gFlagged = sum(last14, 'shadowFlaggedCount');
  const gRegret =
    gArchived > 0
      ? rate(sum(last14, 'restored24hCount'), gArchived)
      : rate(sum(last14, 'shadowRegretCount'), gFlagged);
  const gSearch = last14.flatMap((r) => r.searchToRestoreSamples ?? []);
  const gSearchMs = gSearch.length ? median(gSearch) : null;
  const goals: Goal[] = [
    {
      id: 'regret',
      label: gArchived === 0 && gFlagged > 0 ? 'Regret (shadow, 14 days)' : 'Regret (14 days)',
      value: gRegret === null ? '—' : pct(gRegret),
      target: `under ${pct(TARGETS.regretRate)}`,
      status: gRegret === null ? 'noData' : gRegret < TARGETS.regretRate ? 'met' : 'notYet',
    },
    {
      id: 'openTabs',
      label: 'Open tabs, last week vs week 1',
      value:
        openTabsChange === null
          ? '—'
          : `${openTabsChange <= 0 ? '−' : '+'}${pct(Math.abs(openTabsChange))}`,
      target: `${pct(TARGETS.openTabsDrop)} fewer`,
      status:
        openTabsChange === null
          ? 'noData'
          : -openTabsChange >= TARGETS.openTabsDrop
            ? 'met'
            : 'notYet',
    },
    {
      id: 'search',
      label: 'Search to restore (14 days)',
      value: gSearchMs === null ? '—' : `${(gSearchMs / 1000).toFixed(1)} s`,
      target: `under ${TARGETS.searchToRestoreMs / 1000} s`,
      status:
        gSearchMs === null ? 'noData' : gSearchMs < TARGETS.searchToRestoreMs ? 'met' : 'notYet',
    },
    {
      id: 'automatic',
      label: 'Automatic mode',
      value: !autoSince ? 'Off' : `On for ${autoDays} day${autoDays === 1 ? '' : 's'}`,
      target: `still on after ${TARGETS.autoModeDays} days`,
      status: autoDays >= TARGETS.autoModeDays ? 'met' : 'notYet',
    },
  ];

  return {
    range,
    days: series,
    openTabs,
    firstWeekOpenTabs,
    lastWeekOpenTabs,
    openTabsChange,
    automaticSince: autoSince,
    archived,
    restored24h,
    regretRate,
    shadowRegretRate,
    shadowFlagged,
    searches: sum(rows, 'searchCount'),
    searchToRestoreMs,
    undos: sum(rows, 'undoCount'),
    activeDays: rows.filter((r) => (r.activeSeconds ?? 0) >= ACTIVE_DAY_SECONDS).length,
    topArchived,
    restoredMost,
    goals,
  };
}
