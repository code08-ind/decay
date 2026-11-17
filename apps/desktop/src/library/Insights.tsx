import {
  appCopy,
  automaticSince,
  type BetaDay,
  type BetaSummary,
  betaDays,
  betaSummary,
  buildInsights,
  dayRange,
  daysBetween,
  InsightsView,
  localDay,
  type MetricsDay,
  type RangeDays,
  toCsv,
} from '@decay/ui/metrics';
import { useEffect, useRef, useState } from 'react';
import { api, type BetaContext, errorMessage, type MetricsReport, type Rule } from '../api';
import { t } from '../strings';
import type { PageProps } from './Archive';

// Insights (P2.7, F34, Designs/14): the extension's page, shared (`@decay/ui/metrics`),
// fed by the app's metrics (`get_metrics`).

const RANGE_KEY = 'decay.insightsRange';
const REFRESH_MS = 60_000;

function storedRange(): RangeDays {
  try {
    const v = Number(localStorage.getItem(RANGE_KEY));
    return v === 7 || v === 14 || v === 30 ? v : 30;
  } catch {
    return 30;
  }
}

/** The app's "Share with beta team" file (format v2: the extension's is v1). */
export interface AppBetaExport {
  format: 'decay-beta-metrics';
  version: 2;
  client: 'desktop';
  /** Local date of the export (day only). */
  exportedOn: string;
  /** Random id made on this installation (lets the team count testers). */
  installId: string;
  appVersion: string;
  platform: BetaContext['platform'];
  /** Kinds of browser connected now (no profiles). */
  browsers: string[];
  setup: {
    mode: BetaContext['mode'];
    preset: string;
    thresholds: BetaContext['thresholds'];
    windowManagement: string;
    /** Enabled rules per action (no patterns). */
    rules: Record<string, number>;
    shadowStartedDaysAgo: number | null;
    automaticForDays: number | null;
  };
  summary: BetaSummary;
  days: BetaDay[];
}

export function buildAppBetaExport(
  now: number,
  ctx: BetaContext,
  metrics: readonly MetricsDay[],
): AppBetaExport {
  const days = betaDays(metrics);
  const since = automaticSince(metrics, ctx.mode);
  return {
    format: 'decay-beta-metrics',
    version: 2,
    client: 'desktop',
    exportedOn: localDay(now),
    installId: ctx.installId,
    appVersion: ctx.appVersion,
    platform: ctx.platform,
    browsers: [...ctx.browsers],
    setup: {
      mode: ctx.mode,
      preset: ctx.preset,
      thresholds: { ...ctx.thresholds },
      windowManagement: ctx.windowManagement,
      rules: { ...ctx.rules },
      shadowStartedDaysAgo:
        ctx.shadowStartedAt === null
          ? null
          : Math.max(0, Math.floor((now - ctx.shadowStartedAt) / 86_400_000)),
      automaticForDays: since ? daysBetween(since, localDay(now)) : null,
    },
    summary: betaSummary(days, metrics),
    days,
  };
}

/** Domains with an enabled "always keep" (protect) rule. */
export function protectedDomains(rules: readonly Rule[]): ReadonlySet<string> {
  return new Set(
    rules
      .filter((r) => r.enabled && r.scope === 'domain' && r.action === 'protect')
      .map((r) => r.match),
  );
}

/**
 * Always keeps a site (a domain protect rule). One already there counts, but turned off
 * it's turned on: a site isn't "kept" by a rule that doesn't apply.
 */
export async function keepSite(domain: string): Promise<void> {
  try {
    await api.createRule({ scope: 'domain', match: domain, action: 'protect' });
    return;
  } catch (e) {
    if (!(e && typeof e === 'object' && (e as { code?: unknown }).code === 'CONFLICT')) throw e;
  }
  const rules = await api.listRules();
  const rule = rules.find(
    (r) => r.scope === 'domain' && r.action === 'protect' && r.match === domain,
  );
  if (rule && !rule.enabled) {
    await api.updateRule(rule.id, {
      scope: rule.scope,
      match: rule.match,
      action: rule.action,
      thresholdHours: rule.thresholdHours,
      enabled: true,
    });
  }
}

/** Local midnight of a range's first day. */
function rangeStart(now: number, range: RangeDays): number {
  const first = dayRange(now, range)[0] ?? localDay(now);
  return new Date(`${first}T00:00:00`).getTime();
}

export function Insights(props: PageProps) {
  const [range, setRange] = useState<RangeDays>(storedRange);
  const [report, setReport] = useState<MetricsReport | null>(null);
  const [preview, setPreview] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [protectedSites, setProtectedSites] = useState<ReadonlySet<string>>(new Set());
  const seq = useRef(0);
  const previewOpen = preview !== null;

  // Sites already always kept: their rows say so instead of offering the button.
  // biome-ignore lint/correctness/useExhaustiveDependencies: reload when the archive changes
  useEffect(() => {
    let live = true;
    Promise.resolve()
      .then(() => api.listRules())
      .then((rules) => {
        if (live) setProtectedSites(protectedDomains(rules ?? []));
      })
      .catch(() => {
        // Only a hint: without it, rows offer the button (`keepSite` handles a duplicate).
      });
    return () => {
      live = false;
    };
  }, [props.version]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: reload on range and archive changes
  useEffect(() => {
    if (previewOpen) return; // no refresh under the share preview
    const load = () => {
      const mine = ++seq.current;
      api
        .getMetrics(rangeStart(Date.now(), range))
        .then((r) => {
          if (mine !== seq.current) return;
          setReport(r);
          setError(null);
        })
        .catch((e) => {
          if (mine === seq.current) setError(t('insights.loadFailed', { error: errorMessage(e) }));
        });
    };
    load();
    const timer = setInterval(load, REFRESH_MS);
    return () => clearInterval(timer);
  }, [range, props.version, previewOpen]);

  if (!report) {
    return (
      <p className="p-10 text-muted" aria-busy={!error} role={error ? 'alert' : undefined}>
        {error ?? t('insights.loading')}
      </p>
    );
  }
  const insights = {
    ...buildInsights({
      now: Date.now(),
      range,
      mode: report.mode,
      metrics: report.days,
      items: [],
    }),
    topArchived: report.topArchived,
    restoredMost: report.restoredMost,
  };

  const onRangeChange = (r: RangeDays) => {
    setRange(r);
    try {
      localStorage.setItem(RANGE_KEY, String(r));
    } catch {
      // A remembered range is a convenience only.
    }
  };
  const failed =
    (key: 'insights.prepareFailed' | 'insights.exportFailed' | 'insights.saveFailed') =>
    (e: unknown) =>
      setError(t(key, { error: errorMessage(e) }));
  const saved = (path: string) =>
    props.notify(t('insights.saved', { file: path.split(/[\\/]/).pop() ?? path }));
  const onShare = () => {
    api
      .getBetaContext()
      .then((ctx) =>
        setPreview(JSON.stringify(buildAppBetaExport(Date.now(), ctx, report.days), null, 2)),
      )
      .catch(failed('insights.prepareFailed'));
  };
  const onExportCsv = () => {
    // The CSV has the daily rows only: no install id is needed (or made).
    api
      .saveMetricsFile('csv', toCsv({ days: betaDays(report.days) }))
      .then((path) => path && saved(path))
      .catch(failed('insights.exportFailed'));
  };
  const onProtectSite = async (domain: string) => {
    try {
      await keepSite(domain);
    } catch (e) {
      setError(t('insights.protectFailed', { site: domain, error: errorMessage(e) }));
      throw e;
    }
    setProtectedSites((s) => new Set(s).add(domain));
    setError(null);
    props.notify(t('summary.alwaysKept', { site: domain }));
  };
  const onShareDownload = () => {
    if (preview === null) return;
    // Exactly the text the user just reviewed.
    api
      .saveMetricsFile('beta', preview)
      .then((path) => {
        if (!path) return;
        setPreview(null);
        saved(path);
      })
      .catch(failed('insights.saveFailed'));
  };
  return (
    <InsightsView
      embedded
      copy={appCopy()}
      insights={insights}
      mode={report.mode}
      onRangeChange={onRangeChange}
      onExportCsv={onExportCsv}
      sharePreview={preview}
      onShare={onShare}
      onShareDownload={onShareDownload}
      onShareCancel={() => setPreview(null)}
      onProtectSite={onProtectSite}
      protectedSites={protectedSites}
      error={error}
    />
  );
}
