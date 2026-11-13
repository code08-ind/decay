import {
  Banner,
  Button,
  Card,
  Chip,
  EmptyState,
  FeatureCard,
  LiveRegion,
  StatusDot,
  Tabs,
  Toast,
  tabId,
} from '@decay/ui';
import { listen } from '@tauri-apps/api/event';
import { Ghost, Leaf, Pause, Play, RotateCcw, Settings as SettingsIcon, Wind } from 'lucide-react';
import { useEffect, useState } from 'react';
import { api, describe, errorMessage } from '../api';
import { useRoute, useStatus } from '../useStatus';

type View = 'overview' | 'archive' | 'sweeps';

// The Library window, built from the shared components. Its screens (archive, live now,
// sweeps, shadow review) arrive in P1.11; this is the overview with the tray's actions.
export function Library() {
  const status = useStatus();
  const route = useRoute();
  const [view, setView] = useState<View>('overview');
  // Each toast has its own id: a new one restarts the 30-s timer.
  const [toast, setToast] = useState<{ id: number; text: string; undo: boolean } | null>(null);
  const [error, setError] = useState('');
  useEffect(() => {
    const show = (text: string, undo = false) =>
      setToast({ id: Date.now() + Math.random(), text, undo });
    const offs = [
      listen<{ archived: number; isShadow: boolean }>('sweep:completed', (e) =>
        e.payload.isShadow
          ? show(`Shadow sweep: ${e.payload.archived} would have been archived`)
          : show(`Archived ${e.payload.archived} tabs`, true),
      ),
      listen<{ restored: number }>('sweep:undone', (e) =>
        show(`Restored ${e.payload.restored} tabs`),
      ),
      listen('sweep:nothing', () => show('Nothing to sweep right now')),
    ];
    return () => {
      for (const off of offs) off.then((f) => f());
    };
  }, []);
  const run = (action: () => Promise<unknown>) => () => {
    setError('');
    action().catch((e) => setError(errorMessage(e)));
  };
  const paused = status?.mode === 'paused';
  return (
    <div className="min-h-screen bg-canvas">
      <header className="flex items-center justify-between gap-4 border-b border-line px-6 py-3">
        <div className="flex items-center gap-2">
          <Leaf size={20} className="text-accent" aria-hidden />
          <h1 className="font-serif text-xl font-normal">Decay</h1>
        </div>
        <Tabs
          idPrefix="library"
          label="Library"
          current={view}
          onSelect={setView}
          panelId={(id) => `view-${id}`}
          tabs={[
            { id: 'overview', label: 'Overview' },
            { id: 'archive', label: 'Archive' },
            { id: 'sweeps', label: 'Sweeps' },
          ]}
        />
        <Button
          size="sm"
          icon={<SettingsIcon size={14} />}
          onClick={run(() => api.showWindow('settings'))}
        >
          Settings
        </Button>
      </header>
      <main>
        <div
          id={`view-${view}`}
          role="tabpanel"
          aria-labelledby={tabId('library', view)}
          className="grid gap-4 p-6"
        >
          {status?.mode === 'shadow' && (
            <Banner tone="warning" icon={<Ghost size={16} />}>
              Shadow mode: Decay shows what it would archive, but closes nothing yet.
            </Banner>
          )}
          {status && status.browsers.length === 0 && (
            <Banner tone="info">
              No browser is connected. Install the Decay extension to start.
            </Banner>
          )}
          {route !== '/' && <Chip tone="accent">Opened at {route}</Chip>}
          {view === 'overview' ? (
            <div className="grid gap-4 md:grid-cols-[2fr_1fr]">
              <FeatureCard>
                <p className="text-sm text-feature-sub">Right now</p>
                <p className="mt-1 font-serif text-4xl">{status ? status.staleCount : '–'} stale</p>
                <p className="mt-1 text-sm text-feature-sub">
                  {status ? describe(status) : 'Starting…'}
                </p>
                <div className="mt-5 flex flex-wrap gap-2">
                  <Button
                    variant="onFeature"
                    icon={<Wind size={14} />}
                    disabled={!status || status.browsers.length === 0 || paused || status.sweeping}
                    onClick={run(api.runSweepNow)}
                  >
                    Sweep now
                  </Button>
                  <Button
                    variant="onFeatureOutline"
                    icon={<RotateCcw size={14} />}
                    disabled={!status?.undoableSweep}
                    onClick={run(() => api.undoSweep())}
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
                  onClick={run(() => (paused ? api.resume() : api.setMode('paused')))}
                >
                  {paused ? 'Resume' : 'Pause'}
                </Button>
              </Card>
            </div>
          ) : (
            <Card>
              <EmptyState icon={<Leaf size={20} />} title="Coming in the Library screens">
                The archive and sweep history arrive with the next UI phases.
              </EmptyState>
            </Card>
          )}
          {error && (
            <Banner tone="danger" action={<Button onClick={() => setError('')}>Dismiss</Button>}>
              {error}
            </Banner>
          )}
        </div>
      </main>
      <LiveRegion className="fixed right-6 bottom-6">
        {toast && (
          <Toast
            key={toast.id}
            onDismiss={() => setToast(null)}
            action={
              toast.undo ? (
                <Button size="sm" onClick={run(() => api.undoSweep())}>
                  Undo
                </Button>
              ) : undefined
            }
          >
            {toast.text}
          </Toast>
        )}
      </LiveRegion>
    </div>
  );
}
