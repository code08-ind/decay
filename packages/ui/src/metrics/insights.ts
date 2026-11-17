import { formatNumber } from '../i18n';
import { t } from '../strings';
import type { MetricItem, MetricsDay, Mode } from './types';

/** A day with at least this much active time is an "active day". */
export const ACTIVE_DAY_SECONDS = 3600;

/** Local calendar day, `YYYY-MM-DD`. */
export function localDay(ts: number): string {
  const d = new Date(ts);
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** Median of the samples, rounded; 0 for none. */
export function median(samples: readonly number[]): number {
  const s = samples.filter((n) => Number.isFinite(n)).sort((a, b) => a - b);
  if (!s.length) return 0;
  const mid = Math.floor(s.length / 2);
  return Math.round(
    s.length % 2 ? (s[mid] as number) : ((s[mid - 1] as number) + (s[mid] as number)) / 2,
  );
}

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

/** A rate as a percentage for the locale ("5%", "5 %"), "<1%" for a sliver, a dash for none. */
export function percent(rate: number | null): string {
  if (rate === null) return t('insights.none');
  const p = (r: number) => formatNumber(r, { style: 'percent', maximumFractionDigits: 0 });
  if (rate > 0 && rate < 0.01) return t('insights.percentUnder1', { p: p(0.01) });
  return t('insights.percent', { p: p(Math.round(rate * 100) / 100) });
}

/** Milliseconds as seconds for the locale ("2.0 s"). */
const seconds = (ms: number, digits: number) =>
  t('insights.goal.seconds', {
    s: formatNumber(ms / 1000, { minimumFractionDigits: digits, maximumFractionDigits: digits }),
  });

export function buildInsights(input: {
  now: number;
  range: RangeDays;
  mode: Mode;
  metrics: readonly MetricsDay[];
  items: readonly MetricItem[];
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
      label:
        gArchived === 0 && gFlagged > 0
          ? t('insights.goal.regretShadow')
          : t('insights.goal.regret'),
      value: percent(gRegret),
      target: t('insights.goal.under', { value: percent(TARGETS.regretRate) }),
      status: gRegret === null ? 'noData' : gRegret < TARGETS.regretRate ? 'met' : 'notYet',
    },
    {
      id: 'openTabs',
      label: t('insights.goal.openTabs'),
      value:
        openTabsChange === null
          ? t('insights.none')
          : t('insights.goal.change', {
              sign: openTabsChange < 0 ? 'minus' : openTabsChange > 0 ? 'plus' : 'none',
              p: percent(Math.abs(openTabsChange)),
            }),
      target: t('insights.goal.fewer', { p: percent(TARGETS.openTabsDrop) }),
      status:
        openTabsChange === null
          ? 'noData'
          : -openTabsChange >= TARGETS.openTabsDrop
            ? 'met'
            : 'notYet',
    },
    {
      id: 'search',
      label: t('insights.goal.search'),
      value: gSearchMs === null ? t('insights.none') : seconds(gSearchMs, 1),
      target: t('insights.goal.under', { value: seconds(TARGETS.searchToRestoreMs, 0) }),
      status:
        gSearchMs === null ? 'noData' : gSearchMs < TARGETS.searchToRestoreMs ? 'met' : 'notYet',
    },
    {
      id: 'automatic',
      label: t('insights.goal.automatic'),
      value: !autoSince ? t('insights.goal.off') : t('insights.goal.onFor', { n: autoDays }),
      target: t('insights.goal.stillOn', { n: TARGETS.autoModeDays }),
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
