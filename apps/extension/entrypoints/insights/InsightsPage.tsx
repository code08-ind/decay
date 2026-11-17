import { setLocale } from '@decay/ui';
import { useCallback, useEffect, useRef, useState } from 'react';
import { browser } from 'wxt/browser';
import { InsightsView } from '@/components/Insights';
import { db } from '@/lib/db/db';
import type { Mode } from '@/lib/db/types';
import { buildBetaExport, getInstallId, toCsv } from '@/lib/metrics/export';
import { buildInsights, type Insights, type RangeDays } from '@/lib/metrics/insights';
import { probeAccess } from '@/lib/probe/chrome';
import { rules } from '@/lib/rules/rules';
import { settings } from '@/lib/settings/settings';
import { localDay } from '@/lib/shadow/shadow';
import { t } from '@/lib/strings';
import { detectAppId } from '@/lib/tracker/browser-api';
import { applyTheme } from '@/lib/ui/theme';

const REFRESH_MS = 60_000;
const RANGE_KEY = 'decay.insightsRange';
const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

const BROWSER: Record<string, 'chrome' | 'edge' | 'brave'> = {
  'com.google.Chrome': 'chrome',
  'com.microsoft.Edge': 'edge',
  'com.brave.Browser': 'brave',
};

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

/** Insights page (P0.8, F34): on-device metrics + "Share with beta team". */
export function InsightsPage() {
  // The tab's name, in the page's language (again after a change).
  useEffect(() => {
    document.title = t('title.insights');
  });
  const [range, setRange] = useState<RangeDays>(storedRange);
  const [insights, setInsights] = useState<Insights | null>(null);
  const [mode, setMode] = useState<Mode>('shadow');
  const [preview, setPreview] = useState<string | null>(null);
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

  // No refresh while the share preview is open (it would re-render under the reader).
  const previewOpen = preview !== null;
  useEffect(() => {
    if (previewOpen) return;
    void load();
    const timer = setInterval(() => void load(), REFRESH_MS);
    return () => clearInterval(timer);
  }, [load, previewOpen]);

  const onRangeChange = (r: RangeDays) => {
    setRange(r);
    try {
      localStorage.setItem(RANGE_KEY, String(r));
    } catch {
      // A remembered range is a convenience only.
    }
  };

  /** The CSV doesn't carry the install id, so it doesn't create one. */
  const buildExport = async (withId: boolean) => {
    const [all, metrics, rules, installId, access] = await Promise.all([
      settings.getAll(),
      db.metrics.toArray(),
      db.rules.toArray(),
      withId ? getInstallId(db) : Promise.resolve(''),
      probeAccess(),
    ]);
    return buildBetaExport({
      now: Date.now(),
      installId,
      extensionVersion: browser.runtime.getManifest().version,
      browser: BROWSER[detectAppId()] ?? 'chrome',
      mode: all.mode,
      preset: all.preset,
      thresholds: all.thresholds,
      probeAccess: access,
      rules,
      shadowStartedAt: all.shadowStartedAt,
      metrics,
    });
  };

  const stamp = () => localDay(Date.now());

  const onShare = async () => {
    try {
      setPreview(JSON.stringify(await buildExport(true), null, 2));
    } catch (e) {
      setError(t('insights.prepareFailed', { error: message(e) }));
    }
  };

  const onExportCsv = async () => {
    try {
      saveFile(`decay-metrics-${stamp()}.csv`, toCsv(await buildExport(false)), 'text/csv');
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

  const onShareDownload = () => {
    if (preview === null) return;
    // Exactly the text the user just reviewed.
    saveFile(`decay-beta-metrics-${stamp()}.json`, preview, 'application/json');
    setPreview(null);
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
      sharePreview={preview}
      onShare={() => void onShare()}
      onShareDownload={onShareDownload}
      onShareCancel={() => setPreview(null)}
      onProtectSite={onProtectSite}
      protectedSites={protectedSites}
      error={error}
    />
  );
}
