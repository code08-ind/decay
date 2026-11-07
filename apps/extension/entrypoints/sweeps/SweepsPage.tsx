import { useCallback, useEffect, useRef, useState } from 'react';
import { browser } from 'wxt/browser';
import { SweepHistory } from '@/components/SweepHistory';
import { db } from '@/lib/db/db';
import { settings } from '@/lib/settings/settings';
import { type SweepHistory as History, loadSweepHistory } from '@/lib/sweep/history';
import { sendDecay } from '@/lib/sweep/messages';
import { faviconFor } from '@/lib/ui/favicon';
import { applyTheme } from '@/lib/ui/theme';

const REFRESH_MS = 15_000;
const message = (e: unknown) => (e instanceof Error ? e.message : String(e));
const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`;

export function SweepsPage() {
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
      const next = await loadSweepHistory(db, all.mode);
      if (seq !== loadSeq.current) return;
      applyTheme(all.theme);
      setHistory(next);
      setNow(Date.now());
    } catch (e) {
      if (seq === loadSeq.current) setError(`Couldn't load sweeps: ${message(e)}`);
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
        return `Restored ${restored} of ${restored + r.failed} tabs. Try Undo again for the rest.`;
      }
      return `Restored ${plural(restored, 'tab')}.`;
    });

  const onSweepNow = () =>
    act('sweepNow', async () => {
      const r = await sendDecay({ type: 'decay.sweepNow' });
      if (r.simulated !== undefined) {
        return r.simulated
          ? `Simulation: Decay would archive ${plural(r.simulated, 'more tab')}. Nothing was closed — see the shadow review.`
          : 'Simulation: no new tabs would be archived right now. Nothing was closed.';
      }
      if (!r.archived && r.failed) {
        return `Nothing archived: ${plural(r.failed, 'tab')} stayed open (in use, unsaved work, or the page changed).`;
      }
      if (!r.archived) return 'Nothing to archive right now: no tab is old enough.';
      return `Archived ${plural(r.archived, 'tab')}.`;
    });

  const onRestore = (ids: string[]) =>
    act('restore', async () => {
      const r = await sendDecay({ type: 'decay.restore', ids });
      const restored = r.restored ?? 0;
      return r.failed
        ? `Restored ${restored} of ${restored + r.failed} tabs.`
        : `Restored ${plural(restored, 'tab')}.`;
    });

  if (!history) {
    return (
      <p className="p-10 text-muted" aria-busy={!error}>
        {error ?? 'Loading…'}
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
