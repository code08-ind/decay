import {
  Banner,
  Button,
  Card,
  FeatureCard,
  LiveRegion,
  StatusDot,
  Tabs,
  Toast,
  tabId,
} from '@decay/ui';
import { listen } from '@tauri-apps/api/event';
import { Leaf, Pause, Play, RotateCcw, Settings as SettingsIcon, Wind } from 'lucide-react';
import { useEffect, useState } from 'react';
import { api, describe, type EngineStatus, errorMessage } from '../api';
import { notesFor, RELEASES, type ReleaseNotes } from '../help/releases';
import { WhatsNew } from '../help/WhatsNew';
import { Archive } from '../library/Archive';
import { Insights } from '../library/Insights';
import { parseRoute, routeFor, type View } from '../library/model';
import { ShadowReview } from '../library/ShadowReview';
import { Summary } from '../library/Summary';
import { Sweeps } from '../library/Sweeps';
import { useRoute, useStatus } from '../useStatus';

const MODE_TEXT = { shadow: 'Shadow mode', automatic: 'Automatic', paused: 'Paused' } as const;

// The Library window (P1.11, F22): Library, Sweeps, Shadow review and the overview with
// the tray's controls. Routes come from deep links, the tray and the panel (`navigate`).
export function Library() {
  const status = useStatus();
  const [route, setRoute] = useRoute();
  const r = parseRoute(route);
  // Bumped whenever the archive may have changed: the screens reload.
  const [version, setVersion] = useState(0);
  // Each toast has its own id: a new one restarts the 30-s timer.
  const [toast, setToast] = useState<{ id: number; text: string; undo: boolean } | null>(null);
  const [error, setError] = useState('');
  const notify = (text: string, undo = false) =>
    setToast({ id: Date.now() + Math.random(), text, undo });
  // biome-ignore lint/correctness/useExhaustiveDependencies: subscribe once (notify only sets state)
  useEffect(() => {
    const bump = () => setVersion((v) => v + 1);
    const offs = [
      listen<{ archived: number; isShadow: boolean }>('sweep:completed', (e) => {
        bump();
        notify(
          e.payload.isShadow
            ? `Shadow sweep: ${e.payload.archived} would have been archived`
            : `Archived ${e.payload.archived} ${e.payload.archived === 1 ? 'item' : 'items'}`,
          !e.payload.isShadow,
        );
      }),
      listen<{ restored: number; failed: number }>('sweep:undone', (e) => {
        bump();
        notify(`Restored ${e.payload.restored} from the sweep`);
      }),
      listen<{ restored: string[]; failed: unknown[] }>('items:restored', (e) => {
        bump();
        const n = e.payload.restored?.length ?? 0;
        const failed = e.payload.failed?.length ?? 0;
        notify(failed ? `Restored ${n}; ${failed} couldn't be restored` : `Restored ${n}`);
      }),
      listen('sweep:nothing', () => notify('Nothing to sweep right now')),
      listen('settings:changed', bump),
      listen('archive:changed', bump),
    ];
    return () => {
      for (const off of offs) off.then((f) => f()).catch(() => {});
    };
  }, []);
  const run = (action: () => Promise<unknown>) => {
    setError('');
    action().catch((e) => setError(errorMessage(e)));
  };
  const go = (to: string) => {
    setError('');
    setRoute(to);
    // Keep the URL in step, so a reload of the window stays on this screen.
    try {
      history.replaceState(null, '', `#${to}`);
    } catch {
      // not available (tests)
    }
  };
  const refresh = () => setVersion((v) => v + 1);

  // What's New (F32): once after an update to a version with notes (a version without
  // any is marked seen quietly), and whenever Help → What's New asks: `whats-new:open`
  // over the current screen, or `?whats-new` when the window opens for it.
  const [whatsNew, setWhatsNew] = useState<{ notes: ReleaseNotes; latest: boolean } | null>(null);
  // Asked for: this version's notes, else the newest there are (labelled so), else the
  // changelog.
  const showAsked = () =>
    api
      .whatsNew()
      .then(({ version }) => {
        const notes = notesFor(version);
        if (notes) setWhatsNew({ notes, latest: false });
        else if (RELEASES[0]) setWhatsNew({ notes: RELEASES[0], latest: true });
        else return api.openLink('changelog');
      })
      .catch((e) => setError(errorMessage(e)));
  // biome-ignore lint/correctness/useExhaustiveDependencies: once per window
  useEffect(() => {
    api
      .whatsNew()
      .then(({ version, show }) => {
        if (!show) return;
        const notes = notesFor(version);
        if (notes) setWhatsNew({ notes, latest: false });
        else api.whatsNewSeen().catch(() => {});
      })
      .catch(() => {});
    const off = listen('whats-new:open', () => {
      showAsked();
    });
    return () => {
      off.then((f) => f()).catch(() => {});
    };
  }, []);
  const askedWhatsNew = new URLSearchParams(route.split('?')[1] ?? '').has('whats-new');
  // biome-ignore lint/correctness/useExhaustiveDependencies: only when asked (go strips it)
  useEffect(() => {
    if (!askedWhatsNew) return;
    go(routeFor(r));
    showAsked();
  }, [askedWhatsNew]);

  const page = { status, version, go, notify: (t: string) => notify(t), run, refresh };
  return (
    <div className="min-h-screen bg-canvas text-ink">
      <header className="flex flex-wrap items-center justify-between gap-4 border-b border-line px-6 py-3">
        <div className="flex items-center gap-2">
          <Leaf size={20} className="text-accent" aria-hidden />
          <span className="font-serif text-xl">Decay</span>
        </div>
        <Tabs<View>
          idPrefix="library"
          label="Library"
          current={r.view}
          onSelect={(view) => go(routeFor({ view }))}
          panelId={(id) => `view-${id}`}
          tabs={[
            { id: 'library', label: 'Library' },
            { id: 'sweeps', label: 'Sweeps' },
            { id: 'shadow', label: 'Shadow review' },
            { id: 'summary', label: 'Yesterday' },
            { id: 'insights', label: 'Insights' },
            { id: 'overview', label: 'Overview' },
          ]}
        />
        <div className="flex items-center gap-2">
          {status && (
            <StatusDot
              status={status.mode === 'paused' ? 'degraded' : 'connected'}
              label={MODE_TEXT[status.mode]}
              className="rounded-full border border-line bg-surface px-3 py-1"
            />
          )}
          <Button
            size="sm"
            icon={<SettingsIcon size={14} />}
            onClick={() => run(() => api.showWindow('settings'))}
          >
            Settings
          </Button>
        </div>
      </header>
      <main className="mx-auto max-w-[1400px] p-6">
        <div
          id={`view-${r.view}`}
          role="tabpanel"
          aria-labelledby={tabId('library', r.view)}
          className="grid gap-4"
        >
          {status && status.browsers.length === 0 && (
            <Banner tone="info">
              No browser is connected. Install the Decay extension to start.
            </Banner>
          )}
          {error && (
            <Banner tone="danger" action={<Button onClick={() => setError('')}>Dismiss</Button>}>
              {error}
            </Banner>
          )}
          {r.view === 'library' && (
            <Archive {...page} itemId={r.itemId} confirmRestore={r.confirm === 'restore'} />
          )}
          {r.view === 'sweeps' && (
            <Sweeps {...page} sweepId={r.sweepId} confirmUndo={r.confirm === 'undo'} />
          )}
          {r.view === 'shadow' && <ShadowReview {...page} />}
          {r.view === 'summary' && <Summary {...page} />}
          {r.view === 'insights' && <Insights {...page} />}
          {r.view === 'overview' && <Overview status={status} run={run} />}
        </div>
      </main>
      <LiveRegion className="fixed right-6 bottom-6">
        {toast && (
          <Toast
            key={toast.id}
            onDismiss={() => setToast(null)}
            action={
              toast.undo ? (
                <Button size="sm" onClick={() => run(() => api.undoSweep())}>
                  Undo
                </Button>
              ) : undefined
            }
          >
            {toast.text}
          </Toast>
        )}
      </LiveRegion>
      {whatsNew && (
        <WhatsNew
          notes={whatsNew.notes}
          latest={whatsNew.latest}
          onLater={() => {
            setWhatsNew(null);
            api.whatsNewSeen(true).catch(() => {});
          }}
          onGotIt={() => {
            setWhatsNew(null);
            api.whatsNewSeen().catch(() => {});
          }}
          onChangelog={() => api.openLink('changelog')}
        />
      )}
    </div>
  );
}

/** The tray's controls and the connected browsers. */
function Overview(props: {
  status: EngineStatus | null;
  run: (a: () => Promise<unknown>) => void;
}) {
  const { status, run } = props;
  const paused = status?.mode === 'paused';
  return (
    <div className="grid gap-4 md:grid-cols-[2fr_1fr]">
      <h1 className="font-serif text-4xl font-normal md:col-span-2">Overview</h1>
      <FeatureCard>
        <p className="text-sm text-feature-sub">Right now</p>
        <p className="mt-1 font-serif text-4xl">{status ? status.staleCount : '–'} stale</p>
        <p className="mt-1 text-sm text-feature-sub">{status ? describe(status) : 'Starting…'}</p>
        <div className="mt-5 flex flex-wrap gap-2">
          <Button
            variant="onFeature"
            icon={<Wind size={14} />}
            disabled={!status || status.browsers.length === 0 || paused || status.sweeping}
            onClick={() => run(api.runSweepNow)}
          >
            Sweep now
          </Button>
          <Button
            variant="onFeatureOutline"
            icon={<RotateCcw size={14} />}
            disabled={!status?.undoableSweep}
            onClick={() => run(() => api.undoSweep())}
          >
            Undo last sweep
          </Button>
        </div>
      </FeatureCard>
      <Card as="section" className="grid content-start gap-3">
        <h2 className="font-serif text-lg">Browsers</h2>
        {status?.browsers.length ? (
          status.browsers.map((b) => (
            <div key={b.profileId} className="flex items-center justify-between text-sm">
              <span className="capitalize">{b.browser}</span>
              <StatusDot status="connected" label={`${b.tabs} tabs`} />
            </div>
          ))
        ) : (
          <StatusDot status="down" />
        )}
        <Button
          icon={paused ? <Play size={14} /> : <Pause size={14} />}
          onClick={() => run(() => (paused ? api.resume() : api.setMode('paused')))}
        >
          {paused ? 'Resume' : 'Pause'}
        </Button>
      </Card>
    </div>
  );
}
