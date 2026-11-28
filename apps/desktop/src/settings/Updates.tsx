import { Banner, Button, formatDate, SegmentedControl, Switch } from '@decay/ui';
import { listen } from '@tauri-apps/api/event';
import { RefreshCw, RotateCcw } from 'lucide-react';
import { useEffect, useState } from 'react';
import { api, errorMessage, type UpdateState } from '../api';
import { DecayLeaf } from '../DecayLeaf';
import { t } from '../strings';
import { Group, Row, TabHeader, type TabProps } from './parts';

/** "Last checked today at 9:02" / "… on 12 Nov at 9:02" / "Not checked yet". */
export function checkedText(at: number | null, now = Date.now()): string {
  if (at === null) return t('updates.notChecked');
  const d = new Date(at);
  const time = formatDate(d, { hour: 'numeric', minute: '2-digit' });
  const today = new Date(now).toDateString() === d.toDateString();
  return today
    ? t('updates.checkedToday', { time })
    : t('updates.checkedOn', { date: formatDate(d, { day: 'numeric', month: 'short' }), time });
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
      <TabHeader title={t('settings.tab.updates')}>{t('updates.subtitle')}</TabHeader>
      <div className="grid gap-4">
        {error && <Banner tone="danger">{error}</Banner>}
        {waiting && state && (
          <section
            aria-labelledby="release-h"
            aria-live="polite"
            className="rounded-card bg-feature p-5 text-feature-ink shadow-sm"
          >
            <p className="mb-1 flex items-center gap-2 text-xs font-semibold tracking-wider uppercase">
              <DecayLeaf size={14} aria-hidden /> {t('updates.newRelease')}
            </p>
            <h2 id="release-h" className="font-serif text-2xl">
              {phase === 'ready'
                ? t('updates.ready', { version: state.version })
                : t('updates.available', { version: state.version })}
            </h2>
            {state.notes && <p className="mt-1 text-sm whitespace-pre-line">{state.notes}</p>}
            {failedLate && state.error && (
              <p role="alert" className="mt-2 text-sm text-danger">
                {t('updates.failed', { step: state.failed ?? 'download', error: state.error })}
              </p>
            )}
            {phase === 'downloading' && (
              <div className="mt-3">
                <div
                  role="progressbar"
                  aria-label={t('updates.downloadingLabel')}
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
                <p className="mt-1 text-xs">
                  {pct !== null ? t('updates.downloadingPct', { pct }) : t('updates.downloading')}
                </p>
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
                  ? t('updates.restarting')
                  : phase === 'ready'
                    ? t('updates.restart')
                    : failedLate
                      ? t('updates.retry')
                      : t('updates.downloadRestart')}
              </Button>
              {state.installOnQuit ? (
                <span className="text-sm">{t('updates.onQuitSet')}</span>
              ) : (
                <Button onClick={() => act(api.installUpdate('onQuit'))}>
                  {t('updates.onQuit')}
                </Button>
              )}
            </div>
          </section>
        )}
        <Group title={t('updates.current')}>
          <Row
            label={state ? t('updates.version', { version: state.current }) : 'Decay'}
            description={
              <span aria-live="polite">
                {phase === 'checking'
                  ? t('updates.checking')
                  : phase === 'error' && state?.failed === 'check' && state.error
                    ? t('updates.checkFailed', { error: state.error })
                    : phase === 'idle' && state?.checkedAt
                      ? t('updates.upToDate', { checked: checkedText(state.checkedAt) })
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
              {t('updates.checkNow')}
            </Button>
          </Row>
        </Group>
        <Group title={t('updates.preferences')}>
          <Switch
            label={t('updates.auto')}
            description={t('updates.auto.text')}
            checked={settings?.auto_update !== false}
            onChange={(on) =>
              save({ auto_update: on }, on ? t('updates.autoOn') : t('updates.autoOff'))
            }
          />
          <Row
            label={t('updates.channel')}
            description={
              channel === 'beta' ? t('updates.channel.beta.text') : t('updates.channel.stable.text')
            }
          >
            <SegmentedControl<'stable' | 'beta'>
              label={t('updates.channel')}
              value={channel}
              onChange={(c) => {
                save({ update_channel: c }, t('updates.channel.saved', { channel: c }));
              }}
              options={[
                { value: 'stable', label: t('updates.channel.stable') },
                { value: 'beta', label: t('updates.channel.beta') },
              ]}
            />
          </Row>
        </Group>
      </div>
    </>
  );
}
