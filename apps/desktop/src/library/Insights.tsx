import {
  appCopy,
  betaDays,
  buildInsights,
  dayRange,
  InsightsView,
  localDay,
  type RangeDays,
  toCsv,
} from '@decay/ui/metrics';
import { useEffect, useRef, useState } from 'react';
import { api, appWithProfile, errorMessage, type MetricsReport, type Rule } from '../api';
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
  const [error, setError] = useState<string | null>(null);
  const [protectedSites, setProtectedSites] = useState<ReadonlySet<string>>(new Set());
  const seq = useRef(0);

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
  }, [range, props.version]);

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
  const onExportCsv = () => {
    api
      .saveMetricsFile('csv', toCsv({ days: betaDays(report.days) }))
      .then(
        (path) =>
          path && props.notify(t('insights.saved', { file: path.split(/[\\/]/).pop() ?? path })),
      )
      .catch((e) => setError(t('insights.exportFailed', { error: errorMessage(e) })));
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
  return (
    <InsightsView
      embedded
      copy={appCopy()}
      insights={insights}
      mode={report.mode}
      onRangeChange={onRangeChange}
      onExportCsv={onExportCsv}
      onProtectSite={onProtectSite}
      protectedSites={protectedSites}
      topSources={(report.topSources ?? []).map((src) => ({
        id: `${src.appId}|${src.profile?.id ?? ''}`,
        label: appWithProfile(src.appName, src.profile),
        tabs: src.tabs,
        windows: src.windows,
      }))}
      topSourcesTotal={report.archivedTotal}
      error={error}
    />
  );
}
