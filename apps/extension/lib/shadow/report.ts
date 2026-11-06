import type { DecayDB } from '../db/db';
import { LIVE_STATES } from '../db/db';
import type { Item, MetricsDay, Mode, ProtectedBy, Rule, Thresholds } from '../db/types';
import { type Decision, evaluate, FORM_INPUT_UNKNOWN, isBlankPage } from '../policy/policy';
import type { TabInfo } from '../tracker/tabs';
import {
  ACTIVE_DAY_SECONDS,
  countActiveDays,
  localDay,
  SHADOW_DAYS,
  SHADOW_SWEEP_INTERVAL_MS,
} from './shadow';

export interface ReportItem {
  item: Item;
  decision: Decision;
}

/** Reason groups shown in "Why things were kept". */
export type KeptGroup = 'pinned' | 'audio' | 'active' | 'form' | 'rules' | 'hold' | 'other';

export interface ShadowReport {
  mode: Mode;
  activeDays: number;
  targetDays: number;
  /** Shadow period complete (7 active days): suggest automatic mode. */
  ready: boolean;
  /** Today already has ≥ 1 h of activity (so it is included in activeDays). */
  todayCounted: boolean;
  /** "Day N of 7" for display: the day in progress, capped at targetDays. */
  dayNumber: number;
  /** Most common domains among would-archive tabs (API `get_shadow_report`). */
  topDomains: { domain: string; count: number }[];
  /**
   * Simulated archive decisions since shadow mode started. A tab that was flagged, then
   * used, then flagged again counts twice — each would have been a separate archive.
   */
  flaggedTotal: number;
  /** Flagged items used again within 24 h. */
  regretTotal: number;
  /** regretTotal / flaggedTotal, or null with nothing flagged yet. */
  regretRate: number | null;
  /** Open tabs Decay would archive at the next sweep, oldest first. */
  wouldArchive: ReportItem[];
  /** Open tabs past the archive threshold that were kept, with the reason. */
  kept: ReportItem[];
  keptByGroup: Record<KeptGroup, number>;
  /** When the next simulated sweep can run (null outside shadow mode). */
  nextSimulationAt: number | null;
}

export interface ReportInput {
  now: number;
  mode: Mode;
  holdUntil: number | null;
  thresholds: Thresholds;
  shadowStartedAt: number | null;
  rules: readonly Rule[];
  items: readonly Item[];
  metrics: readonly MetricsDay[];
  tabs: readonly TabInfo[];
  lastShadowSweepAt: number | null;
}

export function keptGroup(by: ProtectedBy): KeptGroup {
  switch (by) {
    case 'pinned':
      return 'pinned';
    case 'audible':
    case 'call':
    case 'meeting':
      return 'audio';
    case 'activeTab':
      return 'active';
    case 'formInput':
    case 'unsaved':
    case 'unknownSaveState':
      return 'form';
    case 'rule':
      return 'rules';
    case 'hold':
      return 'hold';
    default:
      return 'other';
  }
}

export function buildShadowReport(input: ReportInput): ShadowReport {
  const { now, thresholds } = input;
  const ctx = {
    now,
    mode: input.mode,
    holdUntil: input.holdUntil,
    thresholds,
    rules: input.rules,
  };
  const tabsById = new Map(input.tabs.map((t) => [String(t.id), t]));
  const wouldArchive: ReportItem[] = [];
  const kept: ReportItem[] = [];
  const keptByGroup: Record<KeptGroup, number> = {
    pinned: 0,
    audio: 0,
    active: 0,
    form: 0,
    rules: 0,
    hold: 0,
    other: 0,
  };

  for (const item of input.items) {
    if (item.kind !== 'tab' || !LIVE_STATES.includes(item.state) || item.state === 'archiving') {
      continue;
    }
    const tab = item.externalId ? tabsById.get(item.externalId) : undefined;
    const decision = evaluate(
      item,
      {
        active: tab?.active ?? false,
        audible: tab?.audible ?? false,
        formInput: FORM_INPUT_UNKNOWN,
      },
      ctx,
    );
    if (decision.action === 'archive') {
      wouldArchive.push({ item, decision });
    } else if (decision.action === 'protect' && item.activeAgeS / 3600 >= thresholds.archive) {
      kept.push({ item, decision });
      keptByGroup[keptGroup(decision.reason.by)]++;
    }
  }
  const byAge = (a: ReportItem, b: ReportItem) => b.item.activeAgeS - a.item.activeAgeS;
  // Empty new tabs go last: archiving them is right, but they're not worth reviewing.
  wouldArchive.sort(
    (a, b) => Number(isBlankPage(a.item.url)) - Number(isBlankPage(b.item.url)) || byAge(a, b),
  );
  kept.sort(byAge);

  const since = input.shadowStartedAt === null ? '' : localDay(input.shadowStartedAt);
  const period = input.metrics.filter((m) => m.day >= since);
  const flaggedTotal = period.reduce((n, m) => n + (m.shadowFlaggedCount ?? 0), 0);
  const regretTotal = period.reduce((n, m) => n + (m.shadowRegretCount ?? 0), 0);
  const activeDays = countActiveDays(input.metrics, input.shadowStartedAt);
  const today = input.metrics.find((m) => m.day === localDay(now));
  const todayCounted = (today?.activeSeconds ?? 0) >= ACTIVE_DAY_SECONDS;
  const ready = activeDays >= SHADOW_DAYS;
  const domainCounts = new Map<string, number>();
  for (const { item } of wouldArchive) {
    if (item.domain) domainCounts.set(item.domain, (domainCounts.get(item.domain) ?? 0) + 1);
  }
  const topDomains = [...domainCounts]
    .map(([domain, count]) => ({ domain, count }))
    .sort((a, b) => b.count - a.count || a.domain.localeCompare(b.domain))
    .slice(0, 5);

  return {
    mode: input.mode,
    activeDays,
    targetDays: SHADOW_DAYS,
    ready,
    todayCounted,
    dayNumber: ready ? SHADOW_DAYS : Math.min(SHADOW_DAYS, activeDays + (todayCounted ? 0 : 1)),
    topDomains,
    flaggedTotal,
    regretTotal,
    regretRate: flaggedTotal > 0 ? regretTotal / flaggedTotal : null,
    wouldArchive,
    kept,
    keptByGroup,
    nextSimulationAt:
      input.mode !== 'shadow'
        ? null
        : input.lastShadowSweepAt === null
          ? now
          : Math.max(now, input.lastShadowSweepAt + SHADOW_SWEEP_INTERVAL_MS),
  };
}

/** Gathers everything for the report from the DB and the browser. */
export async function loadShadowReport(
  db: DecayDB,
  settings: {
    mode: Mode;
    holdUntil: number | null;
    thresholds: Thresholds;
    shadowStartedAt: number | null;
  },
  tabs: readonly TabInfo[],
  now = Date.now(),
): Promise<ShadowReport> {
  const [items, rules, metrics, lastSweep] = await Promise.all([
    db.items.where('state').anyOf(LIVE_STATES).toArray(),
    db.rules.toArray(),
    db.metrics.toArray(),
    db.sweeps.where('isShadow').equals(1).last(),
  ]);
  return buildShadowReport({
    now,
    ...settings,
    rules,
    items,
    metrics,
    tabs,
    lastShadowSweepAt: lastSweep?.startedAt ?? null,
  });
}
