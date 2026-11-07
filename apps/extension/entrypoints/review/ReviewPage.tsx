import { useCallback, useEffect, useRef, useState } from 'react';
import { browser } from 'wxt/browser';
import { ShadowReview } from '@/components/ShadowReview';
import { db } from '@/lib/db/db';
import type { Mode } from '@/lib/db/types';
import { probeAccess as loadProbeAccess, probeStore, requestProbeAccess } from '@/lib/probe/chrome';
import { RuleError, rules } from '@/lib/rules/rules';
import { settings } from '@/lib/settings/settings';
import { protectRuleFor } from '@/lib/shadow/actions';
import { loadShadowReport, type ReportItem, type ShadowReport } from '@/lib/shadow/report';
import { AUTOMATIC_MODE_AVAILABLE } from '@/lib/shadow/shadow';
import type { TabInfo } from '@/lib/tracker/tabs';
import { faviconFor } from '@/lib/ui/favicon';
import { applyTheme } from '@/lib/ui/theme';

const REFRESH_MS = 30_000;
const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

export function ReviewPage() {
  const [report, setReport] = useState<ShadowReport | null>(null);
  const [liveTabs, setLiveTabs] = useState(0);
  const [now, setNow] = useState(Date.now());
  const [busy, setBusy] = useState<string | null>(null);
  const [modeBusy, setModeBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Only the newest load may update state (loads can overlap: interval + actions).
  const loadSeq = useRef(0);

  const load = useCallback(async () => {
    const seq = ++loadSeq.current;
    try {
      const [all, tabs, probes, probeAccess] = await Promise.all([
        settings.getAll(),
        browser.tabs.query({}) as Promise<TabInfo[]>,
        probeStore.load(),
        loadProbeAccess(),
      ]);
      const open = tabs.filter((tab) => !tab.incognito);
      const t = Date.now();
      const next = await loadShadowReport(db, all, open, t, { probes, probeAccess });
      if (seq !== loadSeq.current) return;
      applyTheme(all.theme);
      setReport(next);
      setLiveTabs(open.length);
      setNow(t);
      setError(null);
    } catch (e) {
      if (seq === loadSeq.current) setError(`Couldn't load the review: ${message(e)}`);
    }
  }, []);

  useEffect(() => {
    void load();
    const timer = setInterval(() => void load(), REFRESH_MS);
    return () => clearInterval(timer);
  }, [load]);

  const onProtect = async ({ item }: ReportItem) => {
    const input = protectRuleFor(item);
    if (!input || busy) return;
    setBusy(item.id);
    try {
      await rules.create(input);
    } catch (e) {
      if (!(e instanceof RuleError && e.code === 'DUPLICATE_RULE')) setError(message(e));
    } finally {
      setBusy(null);
    }
    await load();
  };

  const setMode = async (mode: Mode) => {
    if (modeBusy) return;
    setModeBusy(true);
    try {
      await settings.set('mode', mode);
      await load();
    } catch (e) {
      setError(message(e));
    } finally {
      setModeBusy(false);
    }
  };

  // `permissions.request` needs the click's user gesture: it must be the first await.
  const [probeBusy, setProbeBusy] = useState(false);
  const onAllowProbe = async () => {
    if (probeBusy) return;
    setProbeBusy(true); // one permission request at a time (double clicks)
    try {
      await requestProbeAccess();
    } catch (e) {
      setError(message(e));
    } finally {
      setProbeBusy(false);
    }
    await load();
  };

  const onKeepWatching = async () => {
    try {
      const tab = await browser.tabs.getCurrent();
      if (tab?.id !== undefined) await browser.tabs.remove(tab.id);
      else window.close();
    } catch {
      window.close();
    }
  };

  if (!report) {
    return (
      <p className="p-10 text-muted" aria-busy={!error}>
        {error ?? 'Loading…'}
      </p>
    );
  }
  return (
    <ShadowReview
      report={report}
      now={now}
      liveTabs={liveTabs}
      automaticAvailable={AUTOMATIC_MODE_AVAILABLE}
      faviconFor={(url) => faviconFor(url)}
      onProtect={onProtect}
      onAllowProbe={() => void onAllowProbe()}
      probeBusy={probeBusy}
      onTurnOnAutomatic={() => void setMode('automatic')}
      onBackToShadow={() => void setMode('shadow')}
      onKeepWatching={() => void onKeepWatching()}
      busyItemId={busy}
      modeBusy={modeBusy}
      error={error}
    />
  );
}
