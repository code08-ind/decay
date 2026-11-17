import { setLocale } from '@decay/ui';
import { useCallback, useEffect, useRef, useState } from 'react';
import { browser } from 'wxt/browser';
import { SweepHistory } from '@/components/SweepHistory';
import { db } from '@/lib/db/db';
import { settings } from '@/lib/settings/settings';
import { t } from '@/lib/strings';
import { type SweepHistory as History, loadSweepHistory } from '@/lib/sweep/history';
import { sendDecay } from '@/lib/sweep/messages';
import { faviconFor } from '@/lib/ui/favicon';
import { applyTheme } from '@/lib/ui/theme';

const REFRESH_MS = 15_000;
const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

export function SweepsPage() {
  // The tab's name, in the page's language (again after a change).
  useEffect(() => {
    document.title = t('title.sweeps');
  });
  const [history, setHistory] = useState<History | null>(null);
  const [now, setNow] = useState(Date.now());
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const loadSeq = useRef(0);

  const load = useCallback(async () => {
    const seq = ++loadSeq.current;
    try {
      const all = await settings.getAll();
      setLocale(all.locale); // before the history: it's made of words
      const next = await loadSweepHistory(db, all.mode);
      if (seq !== loadSeq.current) return;
      applyTheme(all.theme);
      setHistory(next);
      setNow(Date.now());
    } catch (e) {
      if (seq === loadSeq.current) setError(t('load.sweeps', { error: message(e) }));
    }
  }, []);

  useEffect(() => {
    void load();
    const timer = setInterval(() => void load(), REFRESH_MS);
    return () => clearInterval(timer);
  }, [load]);

  const act = async (key: string, task: () => Promise<string>) => {
    if (busy) return;
    setBusy(key);
    setError(null);
    setNotice(null);
    try {
      setNotice(await task());
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(null);
      await load();
    }
  };

  const onUndo = (sweepId: string) =>
    act(sweepId, async () => {
      const r = await sendDecay({ type: 'decay.undoSweep', sweepId });
      const restored = r.restored ?? 0;
      if (r.failed) {
        return t('sweepsPage.restoredSome', { n: restored, total: restored + r.failed });
      }
      return t('popup.restored', { n: restored });
    });

  const onSweepNow = () =>
    act('sweepNow', async () => {
      const r = await sendDecay({ type: 'decay.sweepNow' });
      if (r.simulated !== undefined) {
        return r.simulated
          ? t('sweepsPage.simulated', { n: r.simulated })
          : t('sweepsPage.simulatedNone');
      }
      if (!r.archived && r.failed) {
        return t('sweepsPage.stayedOpen', { n: r.failed });
      }
      if (!r.archived) return t('sweepsPage.nothing');
      return t('sweepsPage.archived', { n: r.archived });
    });

  const onRestore = (ids: string[]) =>
    act('restore', async () => {
      const r = await sendDecay({ type: 'decay.restore', ids });
      const restored = r.restored ?? 0;
      return r.failed
        ? t('popup.restoredSome', { n: restored, total: restored + r.failed })
        : t('popup.restored', { n: restored });
    });

  if (!history) {
    return (
      <p className="p-10 text-muted" aria-busy={!error}>
        {error ?? t('common.loading')}
      </p>
    );
  }
  return (
    <SweepHistory
      history={history}
      now={now}
      faviconFor={(url) => faviconFor(url)}
      onUndo={(id) => void onUndo(id)}
      onSweepNow={() => void onSweepNow()}
      onRestore={(ids) => void onRestore(ids)}
      onOpenReview={() => void browser.tabs.create({ url: browser.runtime.getURL('/review.html') })}
      busy={busy}
      error={error}
      notice={notice}
    />
  );
}
