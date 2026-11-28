import { Button, cx, FeatureCard, formatDate, IconButton, Shortcut, StatusDot } from '@decay/ui';
import { getVersion } from '@tauri-apps/api/app';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import {
  ExternalLink,
  Settings as Gear,
  Hourglass,
  PanelLeft,
  RotateCcw,
  Search,
} from 'lucide-react';
import { type ReactNode, useEffect, useRef, useState } from 'react';
import { api, describe, type HoldDuration, type SweepRow } from '../api';
import { DecayLeaf } from '../DecayLeaf';
import { t } from '../strings';
import { useStatus } from '../useStatus';

const MODE_TEXT = {
  shadow: 'mode.shadow',
  automatic: 'mode.automatic',
  paused: 'mode.paused',
} as const;

/** The newest sweep (shadow or not), reloaded when one completes or is undone. */
function useLastSweep(opened: number): SweepRow | null | undefined {
  const [sweep, setSweep] = useState<SweepRow | null | undefined>(undefined);
  // biome-ignore lint/correctness/useExhaustiveDependencies: `opened` reloads on every open
  useEffect(() => {
    let live = true;
    const load = () =>
      api
        .listSweeps({ limit: 1, includeShadow: true })
        .then((rows) => live && setSweep(rows[0] ?? null))
        .catch(() => live && setSweep(null));
    load();
    const offs = [listen('sweep:completed', load), listen('sweep:undone', load)];
    return () => {
      live = false;
      for (const off of offs) off.then((f) => f()).catch(() => {});
    };
  }, [opened]);
  return sweep;
}

/**
 * The tray popover (Windows, Designs/windows/02-tray): status at a glance and the tray's
 * everyday actions. Rust shows it beside the icon on a left click and hides it when it
 * loses focus (so opening another window closes it); Esc hides it too.
 */
export function Popover() {
  const status = useStatus();
  const [opened, setOpened] = useState(0);
  const sweep = useLastSweep(opened);
  const [version, setVersion] = useState('');
  const [hotkey, setHotkey] = useState('');
  const [busy, setBusy] = useState(false);
  const searchRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    getVersion()
      .then(setVersion)
      .catch(() => {});
    const loadHotkey = () =>
      api
        .getSettings()
        .then((s) => setHotkey(typeof s.hotkey === 'string' ? s.hotkey : ''))
        .catch(() => {});
    loadHotkey();
    const off = listen('popover:open', () => {
      setOpened((n) => n + 1);
      loadHotkey();
      searchRef.current?.focus();
    });
    searchRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') getCurrentWindow().hide();
    };
    window.addEventListener('keydown', onKey);
    return () => {
      off.then((f) => f()).catch(() => {});
      window.removeEventListener('keydown', onKey);
    };
  }, []);

  const run = (action: () => Promise<unknown>) => {
    setBusy(true);
    action()
      .catch(() => {})
      .finally(() => setBusy(false));
  };
  const hold = (d: HoldDuration) => run(() => api.hold(d));
  const now = Date.now();
  const held = status?.holdUntil != null && status.holdUntil > now;
  const paused = status?.mode === 'paused';

  return (
    <main className="flex h-screen select-none flex-col gap-3 bg-canvas p-4 text-ink">
      <header className="flex items-center gap-2">
        <span className="grid size-6 place-items-center rounded-md bg-accent-soft text-accent">
          <DecayLeaf size={16} aria-hidden />
        </span>
        <span className="font-serif text-lg font-medium">Decay</span>
        {version && (
          <span className="pt-0.5 font-mono text-[11px] text-muted">
            {t('popover.version', { version })}
          </span>
        )}
        <span className="flex-1" />
        {status && (
          <StatusDot
            status={paused ? 'degraded' : 'connected'}
            label={t(MODE_TEXT[status.mode])}
            className="rounded-full border border-line bg-surface-2 px-2.5 py-0.5 text-[11px]"
          />
        )}
        <IconButton
          size="sm"
          label={t('popover.settings')}
          icon={<Gear size={16} />}
          onClick={() => api.showWindow('settings')}
        />
      </header>

      <button
        ref={searchRef}
        type="button"
        onClick={() => api.openSearch()}
        className="flex items-center gap-2 rounded-md border border-line bg-surface px-3 py-1.5 text-start text-xs text-muted hover:bg-surface-2 hover:text-ink"
      >
        <Search size={14} aria-hidden />
        <span className="flex-1">{t('popover.search')}</span>
        {hotkey && <Shortcut accelerator={hotkey} />}
      </button>

      <div className="grid grid-cols-2 gap-2.5">
        <Tile label={t('popover.openTabs')} icon={<PanelLeft size={14} />}>
          {status?.liveTabs ?? '–'}
        </Tile>
        <Tile label={t('popover.stale')} icon={<Hourglass size={14} />}>
          {status?.staleCount ?? '–'}
        </Tile>
      </div>

      {status && <p className="text-xs text-muted">{describe(status, now)}</p>}

      {sweep !== undefined && (
        <FeatureCard className="grid gap-2">
          {sweep ? (
            <>
              <p className="text-[11px] font-medium uppercase tracking-wide opacity-80">
                {t(sweep.isShadow ? 'popover.shadowSweep' : 'popover.lastSweep', {
                  time: formatDate(new Date(sweep.startedAt), {
                    hour: 'numeric',
                    minute: '2-digit',
                  }),
                })}
              </p>
              <p className="font-serif text-xl">
                {sweep.status === 'undone'
                  ? t('popover.undone')
                  : t(sweep.isShadow ? 'popover.wouldArchive' : 'popover.archived', {
                      n: sweep.itemCount,
                    })}
              </p>
              <p className="text-xs opacity-80">
                {t('popover.parts', { tabs: sweep.tabs, windows: sweep.windows })}
              </p>
              <div className="flex gap-2">
                {status?.undoableSweep === sweep.id && (
                  <Button
                    size="sm"
                    variant="onFeature"
                    icon={<RotateCcw className="rtl:-scale-x-100" size={12} aria-hidden />}
                    loading={busy}
                    onClick={() => run(() => api.undoSweep(sweep.id))}
                  >
                    {t('popover.undo')}
                  </Button>
                )}
                <Button
                  size="sm"
                  variant="onFeatureOutline"
                  onClick={() => api.showWindow('library', `/sweeps/${sweep.id}`)}
                >
                  {t('popover.view')}
                </Button>
              </div>
            </>
          ) : (
            <p className="text-sm">{t('popover.noSweep')}</p>
          )}
        </FeatureCard>
      )}

      <section className="grid gap-1.5">
        <h2 className="text-xs text-muted">{t('popover.pause')}</h2>
        {held || paused ? (
          <Button
            size="sm"
            disabled={busy}
            onClick={() => run(() => (paused ? api.resume() : api.releaseHold()))}
          >
            {t('popover.resume')}
          </Button>
        ) : (
          <div className="grid grid-cols-3 gap-2">
            <HoldButton disabled={busy || !status} onClick={() => hold('1h')}>
              {t('popover.hold1h')}
            </HoldButton>
            <HoldButton disabled={busy || !status} onClick={() => hold('untilTomorrow')}>
              {t('popover.holdTomorrow')}
            </HoldButton>
            <HoldButton disabled={busy || !status} onClick={() => hold('indefinite')}>
              {t('popover.holdIndefinite')}
            </HoldButton>
          </div>
        )}
      </section>

      <div className="mt-auto grid gap-2 border-t border-line pt-3">
        <Button
          variant="primary"
          size="lg"
          icon={<ExternalLink className="rtl:-scale-x-100" size={14} aria-hidden />}
          onClick={() => api.showWindow('library')}
          className="justify-start"
        >
          {t('popover.open')}
        </Button>
        <nav className="flex justify-center gap-4 text-xs text-muted">
          <FooterLink onClick={() => api.showWindow('library', '/shadow-report')}>
            {t('library.tab.shadow')}
          </FooterLink>
          <FooterLink onClick={() => api.showWindow('settings')}>
            {t('popover.settings')}
          </FooterLink>
          <FooterLink onClick={() => api.quit()}>{t('popover.quit')}</FooterLink>
        </nav>
      </div>
    </main>
  );
}

function Tile(props: { label: string; icon: ReactNode; children: ReactNode }) {
  return (
    <div className="rounded-lg border border-line bg-surface p-2.5">
      <div className="flex items-center justify-between text-[11px] text-muted">
        <span>{props.label}</span>
        <span aria-hidden>{props.icon}</span>
      </div>
      <p className="mt-1 font-serif text-2xl leading-none">{props.children}</p>
    </div>
  );
}

function HoldButton(props: { children: ReactNode; disabled?: boolean; onClick: () => void }) {
  return (
    <button
      type="button"
      disabled={props.disabled}
      onClick={props.onClick}
      className="rounded-md border border-line bg-surface px-2 py-1.5 text-xs hover:bg-surface-2 disabled:opacity-50"
    >
      {props.children}
    </button>
  );
}

function FooterLink(props: { children: ReactNode; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={props.onClick}
      className={cx('rounded px-1 hover:text-ink hover:underline')}
    >
      {props.children}
    </button>
  );
}
