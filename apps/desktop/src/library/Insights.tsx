import {
  automaticSince,
  type BetaDay,
  type BetaSummary,
  betaDays,
  betaSummary,
  buildInsights,
  dayRange,
  daysBetween,
  type InsightsCopy,
  InsightsView,
  localDay,
  type MetricsDay,
  type RangeDays,
  toCsv,
} from '@decay/ui/metrics';
import { useEffect, useRef, useState } from 'react';
import { api, type BetaContext, errorMessage, type MetricsReport } from '../api';
import type { PageProps } from './Archive';

// Insights (P2.7, F34, Designs/14): the extension's page, shared (`@decay/ui/metrics`),
// fed by the app's metrics (`get_metrics`).

const RANGE_KEY = 'decay.insightsRange';
const REFRESH_MS = 60_000;

export const APP_COPY: InsightsCopy = {
  intro:
    'How your browsing has settled since you started using Decay. Computed on this computer only.',
  privacy: 'Computed on this computer. Nothing leaves it unless you share.',
  shareWhat: "and Decay's version, the platform and the kinds of browser connected",
};

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
  const seq = useRef(0);
  const previewOpen = preview !== null;

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
          if (mine === seq.current) setError(`Couldn't load insights: ${errorMessage(e)}`);
        });
    };
    load();
    const timer = setInterval(load, REFRESH_MS);
    return () => clearInterval(timer);
  }, [range, props.version, previewOpen]);

  if (!report) {
    return (
      <p className="p-10 text-muted" aria-busy={!error} role={error ? 'alert' : undefined}>
        {error ?? 'Loading…'}
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
  const failed = (what: string) => (e: unknown) => setError(`${what}: ${errorMessage(e)}`);
  const onShare = () => {
    api
      .getBetaContext()
      .then((ctx) =>
        setPreview(JSON.stringify(buildAppBetaExport(Date.now(), ctx, report.days), null, 2)),
      )
      .catch(failed("Couldn't prepare the file"));
  };
  const onExportCsv = () => {
    // The CSV has the daily rows only: no install id is needed (or made).
    api
      .saveMetricsFile('csv', toCsv({ days: betaDays(report.days) }))
      .then((path) => path && props.notify(`Saved ${path.split(/[\\/]/).pop() ?? path}`))
      .catch(failed("Couldn't export"));
  };
  const onShareDownload = () => {
    if (preview === null) return;
    // Exactly the text the user just reviewed.
    api
      .saveMetricsFile('beta', preview)
      .then((path) => {
        if (!path) return;
        setPreview(null);
        props.notify(`Saved ${path.split(/[\\/]/).pop() ?? path}`);
      })
      .catch(failed("Couldn't save the file"));
  };
  return (
    <InsightsView
      embedded
      copy={APP_COPY}
      insights={insights}
      mode={report.mode}
      onRangeChange={onRangeChange}
      onExportCsv={onExportCsv}
      sharePreview={preview}
      onShare={onShare}
      onShareDownload={onShareDownload}
      onShareCancel={() => setPreview(null)}
      error={error}
    />
  );
}
