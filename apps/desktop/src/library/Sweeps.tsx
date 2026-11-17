import {
  Banner,
  Button,
  Chip,
  cx,
  EmptyState,
  FeatureCard,
  formatDate,
  IconButton,
  Switch,
} from '@decay/ui';
import { Ghost, RotateCcw, Undo2, Wind, X } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { api, errorMessage, type SweepRow } from '../api';
import { timeLabel } from '../panel/results';
import { t } from '../strings';
import type { PageProps } from './Archive';
import { entryState, groupByDay, sweepAction, sweepHeadline, triggerText } from './model';
import { ItemTile, useLoad } from './parts';

const time = (at: number) => formatDate(at, { hour: 'numeric', minute: '2-digit' });

// Sweep history (Designs/07, F7): every sweep by day, Undo for any of them, and a sweep's
// items with "Restore selected".
export function Sweeps(props: PageProps & { sweepId?: string; confirmUndo: boolean }) {
  const { status, version, go, run } = props;
  // Shadow sweeps show by default in shadow mode (decided once the status is known), until
  // the switch is used.
  const [shadowChoice, setShadow] = useState<boolean | null>(null);
  const shadow = shadowChoice ?? status?.mode === 'shadow';
  const sweeps = useLoad(
    () => api.listSweeps({ limit: 100, includeShadow: shadow }),
    [version, shadow],
  );
  const now = Date.now();
  const rows = sweeps.data ?? [];
  return (
    <div className="grid gap-5">
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <h1 className="font-serif text-4xl font-normal">{t('sweeps.title')}</h1>
          <p className="mt-1 text-sm text-muted">{t('sweeps.subtitle')}</p>
        </div>
        <div className="flex items-center gap-4">
          <Switch checked={shadow} onChange={setShadow} label={t('sweeps.showShadow')} />
          <Button
            variant="primary"
            icon={<Wind size={14} />}
            disabled={
              !status || status.browsers.length === 0 || status.mode === 'paused' || status.sweeping
            }
            onClick={() => run(api.runSweepNow)}
          >
            {t('sweeps.sweepNow')}
          </Button>
        </div>
      </div>
      {sweeps.error && <Banner tone="danger">{sweeps.error}</Banner>}
      <div className={cx('grid items-start gap-4', props.sweepId && 'lg:grid-cols-[1fr_400px]')}>
        <section
          aria-label={t('sweeps.history')}
          className="rounded-card border border-line bg-surface p-4 shadow-sm"
        >
          {!sweeps.loading && rows.length === 0 ? (
            <EmptyState icon={<Wind size={20} />} title={t('sweeps.empty')}>
              {shadow ? t('sweeps.empty.shadow') : t('sweeps.empty.text')}
            </EmptyState>
          ) : (
            groupByDay(rows, now).map((g) => (
              <section key={g.label} aria-label={g.label} className="mb-3 last:mb-0">
                <h2 className="px-2 py-2 text-[11px] font-semibold tracking-wider text-muted uppercase">
                  {g.label}
                </h2>
                <ul className="grid gap-1.5">
                  {g.rows.map((s) => (
                    <SweepListRow
                      key={s.id}
                      sweep={s}
                      open={s.id === props.sweepId}
                      onOpen={() => go(s.id === props.sweepId ? '/sweeps' : `/sweeps/${s.id}`)}
                      onUndo={() => run(() => api.undoSweep(s.id))}
                    />
                  ))}
                </ul>
              </section>
            ))
          )}
        </section>
        {props.sweepId && (
          <SweepDrawer
            key={props.sweepId}
            id={props.sweepId}
            version={version}
            confirmUndo={props.confirmUndo}
            onClose={() => {
              const id = props.sweepId;
              go('/sweeps');
              setTimeout(() => {
                document.querySelector<HTMLElement>(`[data-sweep="${id}"]`)?.focus();
              });
            }}
            run={run}
          />
        )}
      </div>
    </div>
  );
}

function SweepListRow(props: {
  sweep: SweepRow;
  open: boolean;
  onOpen: () => void;
  onUndo: () => void;
}) {
  const s = props.sweep;
  const action = sweepAction(s);
  const undone = !s.isShadow && (s.status === 'undone' || s.undoneAt !== null);
  return (
    <li
      className={cx(
        'flex items-center gap-3 rounded-xl border px-3 py-2.5',
        props.open ? 'border-accent bg-accent-soft' : 'border-transparent hover:bg-surface-2',
      )}
    >
      <button
        type="button"
        data-sweep={s.id}
        aria-expanded={props.open}
        onClick={props.onOpen}
        className="flex min-w-0 flex-1 items-center gap-3 text-start"
      >
        <span
          aria-hidden
          className="grid size-9 shrink-0 place-items-center rounded-full bg-surface-2 text-muted"
        >
          {s.isShadow ? <Ghost size={16} /> : <Wind size={16} />}
        </span>
        <span className="min-w-0">
          <span className="flex flex-wrap items-center gap-2">
            <span className={cx('font-serif text-lg', undone && 'text-muted line-through')}>
              {t('sweeps.number', { n: s.number })}
            </span>
            {s.tabs > 0 && <Chip>{t('sweep.tabs', { n: s.tabs })}</Chip>}
            {s.windows > 0 && <Chip>{t('sweep.windows', { n: s.windows })}</Chip>}
            {s.isShadow && <Chip tone="warning">{t('sweeps.shadow')}</Chip>}
          </span>
          <span className="block text-xs text-muted">
            {t(s.failed > 0 ? 'sweeps.metaFailed' : 'sweeps.meta', {
              time: time(s.startedAt),
              trigger: triggerText(s.trigger),
              failed: s.failed,
            })}
          </span>
        </span>
      </button>
      {s.restored > 0 && action.undo && (
        <Chip tone="accent">{t('sweeps.partly', { n: s.restored })}</Chip>
      )}
      {action.undo ? (
        <Button
          size="sm"
          icon={<Undo2 className="rtl:-scale-x-100" size={12} />}
          aria-label={t('sweeps.actionFor', { action: action.label, n: s.number })}
          onClick={props.onUndo}
        >
          {action.label}
        </Button>
      ) : (
        <span className="text-xs text-muted">{action.label}</span>
      )}
    </li>
  );
}

function SweepDrawer(props: {
  id: string;
  version: number;
  confirmUndo: boolean;
  onClose: () => void;
  run: (action: () => Promise<unknown>) => void;
}) {
  const sweep = useLoad(() => api.getSweep(props.id), [props.id, props.version]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [error, setError] = useState('');
  const heading = useRef<HTMLHeadingElement>(null);
  // biome-ignore lint/correctness/useExhaustiveDependencies: focus once loaded
  useEffect(() => {
    if (sweep.data) heading.current?.focus();
  }, [sweep.data?.id]);
  const d = sweep.data;
  const action = d ? sweepAction(d) : null;
  const toggle = (id: string) =>
    setSelected((old) => {
      const next = new Set(old);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  const [restoring, setRestoring] = useState(false);
  const restoreSelected = () => {
    setError('');
    setRestoring(true);
    api
      .restoreItems([...selected])
      .then(() => setSelected(new Set()))
      .catch((e) => setError(errorMessage(e)))
      .finally(() => setRestoring(false));
  };
  // Escape closes the drawer.
  // biome-ignore lint/correctness/useExhaustiveDependencies: subscribe once per drawer
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !e.defaultPrevented) props.onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);
  return (
    <FeatureCard className="sticky top-4 grid gap-4">
      <div className="flex items-start justify-between gap-2">
        <p className="text-[11px] font-semibold tracking-wider text-feature-sub uppercase">
          {d
            ? t('sweeps.drawer.title', { n: d.number, time: timeLabel(d.startedAt, Date.now()) })
            : t('sweeps.drawer.sweep')}
        </p>
        <IconButton
          onFeature
          label={t('sweeps.drawer.close')}
          icon={<X size={16} />}
          onClick={props.onClose}
        />
      </div>
      {sweep.error && <p role="alert">{sweep.error}</p>}
      {d && action && (
        <>
          <h2
            ref={heading}
            tabIndex={-1}
            className="font-serif text-2xl leading-tight outline-none"
          >
            {sweepHeadline(d)}
          </h2>
          {props.confirmUndo && action.undo && (
            <div className="rounded-xl bg-surface p-3 text-sm text-ink">
              {t('sweeps.drawer.confirmUndo')}
            </div>
          )}
          <p className="rounded-xl bg-feature-tile px-3 py-2 text-sm">
            {t(d.isShadow ? 'sweeps.drawer.triggerShadow' : 'sweeps.drawer.trigger', {
              trigger: d.trigger,
            })}
          </p>
          <section
            aria-label={t('sweeps.drawer.items')}
            className="rounded-xl border border-feature-tile-line bg-feature-tile p-3"
          >
            <h3 className="mb-2 text-[11px] font-semibold tracking-wider text-feature-sub uppercase">
              {t('sweeps.drawer.items')}
            </h3>
            <ul className="grid max-h-80 gap-1 overflow-y-auto">
              {d.items.map((i) => (
                <li key={i.id} className="flex items-center gap-2.5 py-1">
                  {!d.isShadow && i.archived ? (
                    <input
                      type="checkbox"
                      className="size-4 accent-current"
                      aria-label={t('sweeps.drawer.select', {
                        title: i.title || t('sweeps.drawer.untitledLower'),
                      })}
                      checked={selected.has(i.id)}
                      onChange={() => toggle(i.id)}
                    />
                  ) : (
                    <span className="size-4" />
                  )}
                  <ItemTile item={i} size="sm" />
                  <span className="min-w-0 flex-1 truncate text-sm">
                    {i.title || t('sweeps.drawer.untitled')}
                  </span>
                  <span className="shrink-0 font-mono text-[11px] text-feature-sub">
                    {d.isShadow || i.archived ? (i.reason ?? '') : entryState(i.state)}
                  </span>
                </li>
              ))}
              {d.items.length === 0 && (
                <li className="text-sm text-feature-sub">{t('sweeps.drawer.empty')}</li>
              )}
            </ul>
          </section>
          {!d.isShadow && <p className="text-xs text-feature-sub">{t('sweeps.drawer.saved')}</p>}
          {error && (
            <p role="alert" className="rounded-xl bg-surface p-3 text-sm text-danger">
              {error}
            </p>
          )}
          {!d.isShadow && (
            <div className="flex flex-wrap gap-2">
              <Button
                variant="onFeature"
                icon={<Undo2 className="rtl:-scale-x-100" size={14} />}
                disabled={!action.undo}
                onClick={() => props.run(() => api.undoSweep(d.id))}
              >
                {action.undo
                  ? d.restored > 0
                    ? t('sweeps.drawer.undoRest')
                    : t('sweeps.drawer.undoAll')
                  : action.label}
              </Button>
              <Button
                variant="onFeatureOutline"
                icon={<RotateCcw className="rtl:-scale-x-100" size={14} />}
                disabled={selected.size === 0}
                loading={restoring}
                onClick={restoreSelected}
              >
                {selected.size > 0
                  ? t('sweeps.drawer.restoreSelectedN', { n: selected.size })
                  : t('sweeps.drawer.restoreSelected')}
              </Button>
            </div>
          )}
        </>
      )}
    </FeatureCard>
  );
}
