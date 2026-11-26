import { setLocale } from '@decay/ui';
import { betaDays, toCsv } from '@decay/ui/metrics';
import { useCallback, useEffect, useRef, useState } from 'react';
import { InsightsView } from '@/components/Insights';
import { db } from '@/lib/db/db';
import type { Mode } from '@/lib/db/types';
import { buildInsights, type Insights, type RangeDays } from '@/lib/metrics/insights';
import { rules } from '@/lib/rules/rules';
import { settings } from '@/lib/settings/settings';
import { localDay } from '@/lib/shadow/shadow';
import { t } from '@/lib/strings';
import { applyTheme } from '@/lib/ui/theme';

const REFRESH_MS = 60_000;
const RANGE_KEY = 'decay.insightsRange';
const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

function storedRange(): RangeDays {
  try {
    const v = Number(localStorage.getItem(RANGE_KEY));
    return v === 7 || v === 14 || v === 30 ? v : 30;
  } catch {
    return 30;
  }
}

/** Saves `text` as a file in the user's Downloads (no downloads permission needed). */
function saveFile(name: string, text: string, type: string) {
  const url = URL.createObjectURL(new Blob([text], { type }));
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 10_000);
}

/** Insights page (P0.8, F34): on-device metrics and a CSV export. */
export function InsightsPage() {
  // The tab's name, in the page's language (again after a change).
  useEffect(() => {
    document.title = t('title.insights');
  });
  const [range, setRange] = useState<RangeDays>(storedRange);
  const [insights, setInsights] = useState<Insights | null>(null);
  const [mode, setMode] = useState<Mode>('shadow');
  const [error, setError] = useState<string | null>(null);
  const [protectedSites, setProtectedSites] = useState<ReadonlySet<string>>(new Set());
  const loadSeq = useRef(0);

  const load = useCallback(async () => {
    const seq = ++loadSeq.current;
    try {
      const [all, metrics, items, ruleRows] = await Promise.all([
        settings.getAll(),
        db.metrics.toArray(),
        db.items.toArray(),
        db.rules.toArray(),
      ]);
      if (seq !== loadSeq.current) return;
      applyTheme(all.theme);
      setLocale(all.locale);
      setMode(all.mode);
      setProtectedSites(
        new Set(
          ruleRows
            .filter((r) => r.enabled && r.scope === 'domain' && r.action === 'protect')
            .map((r) => r.match),
        ),
      );
      setInsights(buildInsights({ now: Date.now(), range, mode: all.mode, metrics, items }));
      setError(null);
    } catch (e) {
      if (seq === loadSeq.current) setError(t('load.insights', { error: message(e) }));
    }
  }, [range]);

  useEffect(() => {
    void load();
    const timer = setInterval(() => void load(), REFRESH_MS);
    return () => clearInterval(timer);
  }, [load]);

  const onRangeChange = (r: RangeDays) => {
    setRange(r);
    try {
      localStorage.setItem(RANGE_KEY, String(r));
    } catch {
      // A remembered range is a convenience only.
    }
  };

  const stamp = () => localDay(Date.now());

  const onExportCsv = async () => {
    try {
      saveFile(
        `decay-metrics-${stamp()}.csv`,
        toCsv({ days: betaDays(await db.metrics.toArray()) }),
        'text/csv',
      );
    } catch (e) {
      setError(t('insights.exportFailed', { error: message(e) }));
    }
  };

  /**
   * "Always keep" on a restored-most row: a protect rule for the site (a quick action, as
   * from the popup). One already there counts, but turned off it's turned on.
   */
  const onProtectSite = async (domain: string) => {
    try {
      await rules.alwaysKeep(domain);
    } catch (e) {
      setError(t('insights.protectFailed', { site: domain, error: message(e) }));
      throw e;
    }
    setProtectedSites((s) => new Set(s).add(domain));
    setError(null);
  };

  if (!insights) {
    return (
      <p className="p-10 text-muted" aria-busy={!error}>
        {error ?? t('common.loading')}
      </p>
    );
  }
  return (
    <InsightsView
      insights={insights}
      mode={mode}
      onRangeChange={onRangeChange}
      onExportCsv={() => void onExportCsv()}
      onProtectSite={onProtectSite}
      protectedSites={protectedSites}
      error={error}
    />
  );
}
