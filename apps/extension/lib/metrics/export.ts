import type { DecayDB } from '../db/db';
import type { MetricsDay, Mode, Rule, Thresholds } from '../db/types';
import { ACTIVE_DAY_SECONDS, localDay } from '../shadow/shadow';
import { automaticSince, daysBetween, rate } from './insights';
import { median } from './metrics';

/**
 * "Share with beta team" export (P0.8, PRD F34 / §9): an anonymised JSON file the user
 * downloads and sends themselves. Counts and medians only — never URLs, titles, sites,
 * rule patterns or tab contents. The user sees the exact file before saving it.
 */

export interface BetaExport {
  format: 'decay-beta-metrics';
  version: 1;
  /** Local date of the export (day only, not a timestamp). */
  exportedOn: string;
  /** Random id made on this device for the export (lets the team count testers). */
  installId: string;
  extensionVersion: string;
  browser: 'chrome' | 'edge' | 'brave';
  setup: {
    mode: Mode;
    preset: string;
    thresholds: Thresholds;
    /** Form-probe site access: all pages, some sites, or none. */
    probeAccess: 'all' | 'some' | 'none';
    /** Number of rules per action (no patterns). */
    rules: Record<Rule['action'], number>;
    shadowStartedDaysAgo: number | null;
    /**
     * Calendar days since the current unbroken run of automatic mode started (null when
     * off); the same number Insights shows. Per-day detail: `days[].autoModeOn`.
     */
    automaticForDays: number | null;
  };
  summary: {
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
  };
  days: {
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
  }[];
}

const n = (v: unknown) => (typeof v === 'number' && Number.isFinite(v) ? v : 0);
const sampledOr = (samples: number[] | undefined, value: number) =>
  (samples?.length ?? 0) > 0 ? value : null;

export function buildBetaExport(input: {
  now: number;
  installId: string;
  extensionVersion: string;
  browser: BetaExport['browser'];
  mode: Mode;
  preset: string;
  thresholds: Thresholds;
  probeAccess: BetaExport['setup']['probeAccess'];
  rules: readonly Rule[];
  shadowStartedAt: number | null;
  metrics: readonly MetricsDay[];
}): BetaExport {
  const days = [...input.metrics]
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
  const total = (key: keyof (typeof days)[number]) =>
    days.reduce((s, d) => s + (typeof d[key] === 'number' ? (d[key] as number) : 0), 0);
  const archived = total('archived');
  const restored = total('restoredWithin24h');
  const flagged = total('shadowFlagged');
  const openTabs = days.map((d) => d.openTabsP50).filter((v): v is number => v !== null);
  const searchSamples = input.metrics.flatMap((m) => m.searchToRestoreSamples ?? []);
  const rules: Record<Rule['action'], number> = {
    protect: 0,
    exclude: 0,
    customThreshold: 0,
    closeAllowed: 0,
  };
  for (const r of input.rules) if (r.enabled && r.action in rules) rules[r.action]++;

  return {
    format: 'decay-beta-metrics',
    version: 1,
    exportedOn: localDay(input.now),
    installId: input.installId,
    extensionVersion: input.extensionVersion,
    browser: input.browser,
    setup: {
      mode: input.mode,
      preset: input.preset,
      thresholds: { ...input.thresholds },
      probeAccess: input.probeAccess,
      rules,
      shadowStartedDaysAgo:
        input.shadowStartedAt === null
          ? null
          : Math.max(0, Math.floor((input.now - input.shadowStartedAt) / 86_400_000)),
      automaticForDays: (() => {
        const since = automaticSince(input.metrics, input.mode);
        return since ? daysBetween(since, localDay(input.now)) : null;
      })(),
    },
    summary: {
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
    },
    days,
  };
}

/** Daily rows as CSV (same anonymised fields as the JSON export). */
export function toCsv(data: BetaExport): string {
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

/**
 * The export's random install id, made once and kept in the settings table (not part of
 * `Settings`: `settings.getAll()` returns known keys only). One transaction, so two
 * concurrent exports can't make two ids.
 */
export async function getInstallId(db: DecayDB): Promise<string> {
  return db.transaction('rw', db.settings, async () => {
    const row = await db.settings.get(INSTALL_ID_KEY);
    if (typeof row?.value === 'string' && row.value) return row.value;
    const id = crypto.randomUUID();
    await db.settings.put({ key: INSTALL_ID_KEY, value: id, updatedAt: Date.now() });
    return id;
  });
}

export const INSTALL_ID_KEY = 'betaInstallId';
