import { type BetaDay, type BetaSummary, betaDays, betaSummary } from '@decay/ui/metrics';
import type { DecayDB } from '../db/db';
import type { MetricsDay, Mode, Rule, Thresholds } from '../db/types';
import { localDay } from '../shadow/shadow';
import { automaticSince, daysBetween } from './insights';

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
  summary: BetaSummary;
  days: BetaDay[];
}

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
  const days = betaDays(input.metrics);
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
    summary: betaSummary(days, input.metrics),
    days,
  };
}

export { toCsv } from '@decay/ui/metrics';

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
