import { listen } from '@tauri-apps/api/event';
import { useEffect, useState } from 'react';
import { api, describe, errorMessage } from '../api';
import { useRoute, useStatus } from '../useStatus';

// Placeholder until the Library screens (P1.11): status and the tray's actions.
export function Library() {
  const status = useStatus();
  const route = useRoute();
  const [note, setNote] = useState('');
  useEffect(() => {
    const offs = [
      listen<{ archived: number; isShadow: boolean }>('sweep:completed', (e) =>
        setNote(
          e.payload.isShadow
            ? `Shadow sweep: ${e.payload.archived} would have been archived`
            : `Archived ${e.payload.archived}`,
        ),
      ),
      listen<{ restored: number }>('sweep:undone', (e) =>
        setNote(`Restored ${e.payload.restored}`),
      ),
      listen('sweep:nothing', () => setNote('Nothing to sweep')),
    ];
    return () => {
      for (const off of offs) off.then((f) => f());
    };
  }, []);
  const run = (action: () => Promise<unknown>) => () => {
    setNote('');
    action().catch((e) => setNote(errorMessage(e)));
  };
  return (
    <main className="page">
      <header>
        <h1>Decay</h1>
        <p className="muted">{status ? describe(status) : 'Starting…'}</p>
      </header>
      {route !== '/' && <p className="chip">Opened at {route}</p>}
      {status && (
        <section className="card">
          <dl>
            <dt>Mode</dt>
            <dd>{status.mode}</dd>
            <dt>Browsers</dt>
            <dd>
              {status.browsers.length === 0
                ? 'none'
                : status.browsers.map((b) => `${b.browser} (${b.tabs} tabs)`).join(', ')}
            </dd>
          </dl>
          <div className="row">
            <button type="button" onClick={run(api.runSweepNow)}>
              Run sweep now
            </button>
            <button
              type="button"
              disabled={!status.undoableSweep}
              onClick={run(() => api.undoSweep())}
            >
              Undo last sweep
            </button>
            {status.mode === 'paused' ? (
              <button type="button" onClick={run(() => api.setMode('automatic'))}>
                Resume
              </button>
            ) : (
              <button type="button" onClick={run(() => api.setMode('paused'))}>
                Pause
              </button>
            )}
            <button type="button" onClick={run(() => api.showWindow('settings'))}>
              Settings
            </button>
          </div>
        </section>
      )}
      {note && <p role="status">{note}</p>}
    </main>
  );
}
