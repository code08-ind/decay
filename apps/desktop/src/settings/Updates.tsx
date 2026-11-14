import { Banner, Button, SegmentedControl, Switch } from '@decay/ui';
import { listen } from '@tauri-apps/api/event';
import { Leaf, RefreshCw, RotateCcw } from 'lucide-react';
import { useEffect, useState } from 'react';
import { api, errorMessage, type UpdateState } from '../api';
import { Group, Row, TabHeader, type TabProps } from './parts';

/** "Last checked today at 9:02" / "… on 12 Nov at 9:02" / "Not checked yet". */
export function checkedText(at: number | null, now = Date.now()): string {
  if (at === null) return 'Not checked yet';
  const d = new Date(at);
  const time = d.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
  const today = new Date(now).toDateString() === d.toDateString();
  return today
    ? `Last checked today at ${time}`
    : `Last checked ${d.toLocaleDateString([], { day: 'numeric', month: 'short' })} at ${time}`;
}

/** Download progress, 0–100, or null when the size isn't known. */
export function percent(s: Pick<UpdateState, 'downloaded' | 'total'>): number | null {
  if (!s.total) return null;
  return Math.min(100, Math.round((s.downloaded / s.total) * 100));
}

// Updates (Designs/09g, F18): the current version, a waiting release with "Restart to
// update" / "Install on quit", checking now, automatic downloads and the channel.
export function Updates(props: TabProps) {
  const { settings, save } = props;
  const [state, setState] = useState<UpdateState | null>(null);
  const [error, setError] = useState('');
  useEffect(() => {
    api
      .getUpdateState()
      .then(setState)
      .catch((e) => setError(errorMessage(e)));
    const off = listen<UpdateState>('update:state', (e) => setState(e.payload));
    return () => {
      off.then((f) => f()).catch(() => {});
    };
  }, []);
  const act = (p: Promise<UpdateState>) => {
    setError('');
    p.then(setState).catch((e) => setError(errorMessage(e)));
  };
  const channel = settings?.update_channel === 'beta' ? 'beta' : 'stable';
  const phase = state?.phase ?? 'idle';
  // A release stays on screen while it's found, downloading, ready, or its download /
  // install failed (with a way to try again).
  const failedLate = phase === 'error' && state?.failed !== 'check';
  const waiting =
    state?.version && (['available', 'downloading', 'ready'].includes(phase) || failedLate);
  const restarting = state?.installNow === true;
  const pct = state ? percent(state) : null;
  return (
    <>
      <TabHeader title="Updates">Decay updates itself quietly in the background.</TabHeader>
      <div className="grid gap-4">
        {error && <Banner tone="danger">{error}</Banner>}
        {waiting && state && (
          <section
            aria-labelledby="release-h"
            aria-live="polite"
            className="rounded-card bg-feature p-5 text-feature-ink shadow-sm"
          >
            <p className="mb-1 flex items-center gap-2 text-xs font-semibold tracking-wider uppercase">
              <Leaf size={14} aria-hidden /> New release
            </p>
            <h2 id="release-h" className="font-serif text-2xl">
              {phase === 'ready'
                ? `Decay ${state.version} is ready`
                : `Decay ${state.version} is available`}
            </h2>
            {state.notes && <p className="mt-1 text-sm whitespace-pre-line">{state.notes}</p>}
            {failedLate && state.error && (
              <p role="alert" className="mt-2 text-sm text-danger">
                {state.failed === 'install' ? 'Couldn’t install' : 'Couldn’t download'}:{' '}
                {state.error}
              </p>
            )}
            {phase === 'downloading' && (
              <div className="mt-3">
                <div
                  role="progressbar"
                  aria-label="Downloading the update"
                  aria-valuemin={0}
                  aria-valuemax={100}
                  aria-valuenow={pct ?? undefined}
                  className="h-1.5 overflow-hidden rounded-full bg-feature-tile"
                >
                  <div
                    className="h-full rounded-full bg-[var(--accent)] transition-[width]"
                    style={{ width: `${pct ?? 10}%` }}
                  />
                </div>
                <p className="mt-1 text-xs">Downloading{pct !== null ? ` · ${pct}%` : '…'}</p>
              </div>
            )}
            <div className="mt-4 flex flex-wrap items-center gap-2">
              <Button
                variant="primary"
                icon={<RotateCcw size={14} aria-hidden />}
                loading={restarting}
                disabled={restarting}
                onClick={() => act(api.installUpdate('now'))}
              >
                {restarting
                  ? 'Restarting when ready…'
                  : phase === 'ready'
                    ? 'Restart to update'
                    : failedLate
                      ? 'Try again and restart'
                      : 'Download and restart'}
              </Button>
              {state.installOnQuit ? (
                <span className="text-sm">Installs when you quit Decay</span>
              ) : (
                <Button onClick={() => act(api.installUpdate('onQuit'))}>Install on quit</Button>
              )}
            </div>
          </section>
        )}
        <Group title="Current version">
          <Row
            label={state ? `Decay ${state.current}` : 'Decay'}
            description={
              <span aria-live="polite">
                {phase === 'checking'
                  ? 'Checking…'
                  : phase === 'error' && state?.failed === 'check' && state.error
                    ? `Couldn’t check: ${state.error}`
                    : phase === 'idle' && state?.checkedAt
                      ? `Up to date · ${checkedText(state.checkedAt)}`
                      : checkedText(state?.checkedAt ?? null)}
              </span>
            }
          >
            <Button
              size="sm"
              icon={<RefreshCw size={14} aria-hidden />}
              loading={phase === 'checking'}
              disabled={phase === 'checking' || phase === 'downloading'}
              onClick={() => act(api.checkForUpdates())}
            >
              Check now
            </Button>
          </Row>
        </Group>
        <Group title="Preferences">
          <Switch
            label="Download updates automatically"
            description="New versions download in the background; nothing installs until you restart or quit."
            checked={settings?.auto_update !== false}
            onChange={(on) =>
              save(
                { auto_update: on },
                on ? 'Updates download automatically' : 'Updates wait for you',
              )
            }
          />
          <Row
            label="Channel"
            description={
              channel === 'beta'
                ? 'Beta gets new versions early, and every stable release too.'
                : 'Stable gets tested releases only.'
            }
          >
            <SegmentedControl<'stable' | 'beta'>
              label="Channel"
              value={channel}
              onChange={(c) => {
                save({ update_channel: c }, c === 'beta' ? 'Beta channel' : 'Stable channel');
              }}
              options={[
                { value: 'stable', label: 'Stable' },
                { value: 'beta', label: 'Beta' },
              ]}
            />
          </Row>
        </Group>
      </div>
    </>
  );
}
