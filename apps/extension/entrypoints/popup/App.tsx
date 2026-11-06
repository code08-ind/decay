import { useCallback, useEffect, useState } from 'react';
import { db } from '@/lib/db/db';
import type { Sweep } from '@/lib/db/types';
import { type Settings, settings } from '@/lib/settings/settings';
import { recentSweeps } from '@/lib/sweep/executor';
import { sendDecay } from '@/lib/sweep/messages';

// Placeholder until the search popup (P0.6) is built from Designs/*/15-extension-popup-*.
export function App() {
  const [current, setCurrent] = useState<Settings | null>(null);
  const [last, setLast] = useState<Sweep | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    const [all, [sweep]] = await Promise.all([settings.getAll(), recentSweeps(db, 1)]);
    setCurrent(all);
    setLast(sweep ?? null);
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const open = (path: '/review.html' | '/sweeps.html') =>
    browser.tabs.create({ url: browser.runtime.getURL(path) });

  const undo = async () => {
    if (!last || busy) return;
    setBusy(true);
    try {
      const r = await sendDecay({ type: 'decay.undoSweep', sweepId: last.id });
      setStatus(`Restored ${r.restored ?? 0} tab${r.restored === 1 ? '' : 's'}.`);
    } catch (e) {
      setStatus(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
      await load();
    }
  };

  const undoable = last?.status === 'done' && last.itemCount > 0;
  return (
    <main style={{ width: 360, padding: 16, fontFamily: 'system-ui, sans-serif' }}>
      <h1 style={{ fontSize: 18, margin: 0 }}>Decay</h1>
      {current && (
        <p style={{ fontSize: 13 }}>
          Mode: <strong>{current.mode}</strong> · Preset: <strong>{current.preset}</strong> (
          {current.thresholds.stale}/{current.thresholds.archive} active h)
        </p>
      )}
      {last && (
        <p style={{ fontSize: 13 }}>
          Last sweep: {last.status === 'undone' ? 'undone' : `archived ${last.itemCount}`}{' '}
          {undoable && (
            <button type="button" onClick={() => void undo()} disabled={busy}>
              {busy ? 'Undoing…' : 'Undo'}
            </button>
          )}
        </p>
      )}
      {status && (
        <p role="status" style={{ fontSize: 13 }}>
          {status}
        </p>
      )}
      <p style={{ display: 'flex', gap: 8 }}>
        <button type="button" onClick={() => open('/review.html')}>
          Shadow review
        </button>
        <button type="button" onClick={() => open('/sweeps.html')}>
          Sweeps
        </button>
      </p>
    </main>
  );
}
