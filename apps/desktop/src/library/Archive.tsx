import {
  Banner,
  Button,
  Chip,
  cx,
  EmptyState,
  FeatureCard,
  IconButton,
  SearchInput,
  SegmentedControl,
} from '@decay/ui';
import {
  AppWindow,
  Archive as ArchiveIcon,
  Copy,
  Ghost,
  Info,
  Leaf,
  RotateCcw,
  ShieldCheck,
  Trash2,
  Undo2,
  Wind,
  X,
} from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { api, type EngineStatus, errorMessage, exportSummary, type PanelItem } from '../api';
import { displayUrl, timeLabel } from '../panel/results';
import { t } from '../strings';
import { eventText, firstDayOfWeek, shadowDaysLeft, startOfDay, startOfWeek } from './model';
import { ItemTile, Stat, useLoad } from './parts';

type Kind = 'all' | 'tab' | 'window';
type Period = 'today' | 'week' | 'all';
const PAGE = 50;

export interface PageProps {
  status: EngineStatus | null;
  /** Bumped whenever the archive may have changed (sweeps, restores, undo). */
  version: number;
  go: (route: string) => void;
  /** Shows a short message (toast). */
  notify: (text: string) => void;
  /** Runs an action, showing its error. */
  run: (action: () => Promise<unknown>) => void;
  /** The archive changed here (a delete): reload the screens. */
  refresh: () => void;
}

/** Local midnight now, updated when the day changes (a window left open overnight). */
export function useToday(): number {
  const [today, setToday] = useState(() => startOfDay(Date.now()));
  useEffect(() => {
    const timer = setInterval(() => {
      const d = startOfDay(Date.now());
      setToday((old) => (old === d ? old : d));
    }, 60_000);
    return () => clearInterval(timer);
  }, []);
  return today;
}

// The Library screen (Designs/05, F22): stat cards, the archive list with kind / period
// filters and search, and the detail drawer.
export function Archive(props: PageProps & { itemId?: string; confirmRestore: boolean }) {
  const { status, version, go, run } = props;
  const today = useToday();
  const now = Date.now();
  const weekStart = startOfWeek(today, firstDayOfWeek());
  const stats = useLoad(() => api.libraryStats({ todayStart: today, weekStart }), [version, today]);
  const [kind, setKind] = useState<Kind>('all');
  const [period, setPeriod] = useState<Period>('all');
  const [query, setQuery] = useState('');
  const [items, setItems] = useState<PanelItem[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [total, setTotal] = useState(0);
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(true);
  const [exporting, setExporting] = useState(false);
  const asked = useRef(0);

  // A typed `after:` / `before:` wins over the period buttons.
  const typedDate = /\b(after|before):/i.test(query);
  const after = typedDate || period === 'all' ? undefined : period === 'today' ? today : weekStart;
  const load = (more: boolean) => {
    const n = ++asked.current;
    setLoading(true);
    api
      .search({
        query,
        kind: kind === 'all' ? undefined : kind,
        after,
        limit: PAGE,
        cursor: more ? (cursor ?? undefined) : undefined,
      })
      .then((r) => {
        if (n !== asked.current) return; // a newer search is on its way
        // Paging by offset: rows archived meanwhile shift the pages, so skip repeats.
        setItems((old) => {
          if (!more) return r.items;
          const seen = new Set(old.map((i) => i.id));
          return [...old, ...r.items.filter((i) => !seen.has(i.id))];
        });
        setCursor(r.nextCursor);
        setTotal(r.totalArchived);
        setError('');
      })
      .catch((e) => n === asked.current && setError(errorMessage(e)))
      .finally(() => n === asked.current && setLoading(false));
  };
  // Typing waits a moment; filters and changes elsewhere load at once.
  // biome-ignore lint/correctness/useExhaustiveDependencies: reloads on these inputs only
  useEffect(() => {
    const timer = setTimeout(() => load(false), query ? 120 : 0);
    return () => clearTimeout(timer);
  }, [query, kind, period, version, today]);

  const s = stats.data;
  const filtered = query !== '' || kind !== 'all' || period !== 'all';
  return (
    <div className="grid gap-5">
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <h1 className="font-serif text-4xl font-normal">{t('archive.title')}</h1>
          <p className="mt-1 text-sm text-muted">
            {s ? t('archive.total', { n: s.archivedTotal }) : ' '}
            <span className="ms-2 font-serif italic">{t('archive.neverLost')}</span>
          </p>
        </div>
        <div className="flex items-center gap-2">
          {status?.mode === 'shadow' && (
            <Button icon={<Ghost size={14} />} onClick={() => go('/shadow-report')}>
              {t('archive.shadowReview')}
            </Button>
          )}
          <IconButton
            label={t('archive.undo')}
            icon={<Undo2 className="rtl:-scale-x-100" size={16} />}
            outlined
            disabled={!status?.undoableSweep}
            onClick={() => run(() => api.undoSweep())}
          />
          <Button
            variant="primary"
            icon={<Wind size={14} />}
            disabled={
              !status || status.browsers.length === 0 || status.mode === 'paused' || status.sweeping
            }
            onClick={() => run(api.runSweepNow)}
          >
            {t('archive.sweepNow')}
          </Button>
        </div>
      </div>

      {s?.mode === 'shadow' && s.shadowDay !== null && (
        <Banner
          tone="warning"
          icon={<Ghost size={16} />}
          action={<Button onClick={() => go('/shadow-report')}>{t('archive.review')}</Button>}
        >
          {t('archive.shadowBanner', { left: shadowDaysLeft(s.shadowDay) })}
        </Banner>
      )}
      {stats.error && <Banner tone="danger">{stats.error}</Banner>}

      <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
        <Stat
          label={t('archive.stat.open')}
          icon={<AppWindow size={14} />}
          value={s?.openTabs ?? '–'}
        />
        <Stat
          label={t('archive.stat.today')}
          icon={<ArchiveIcon size={14} />}
          value={s ? s.archivedToday.tabs + s.archivedToday.windows : '–'}
          detail={
            s &&
            t('archive.stat.todayDetail', {
              tabs: s.archivedToday.tabs,
              windows: s.archivedToday.windows,
            })
          }
        />
        <Stat
          label={t('archive.stat.restored')}
          icon={<RotateCcw className="rtl:-scale-x-100" size={14} />}
          value={s?.restoredWeek ?? '–'}
        />
        <Stat
          label={t('archive.stat.total')}
          icon={<Leaf size={14} />}
          value={s?.archivedTotal ?? '–'}
        />
      </div>

      <div className={cx('grid items-start gap-4', props.itemId && 'lg:grid-cols-[1fr_360px]')}>
        <section
          aria-label={t('archive.region')}
          className="min-w-0 rounded-card border border-line bg-surface shadow-sm"
        >
          <div className="flex flex-wrap items-center gap-3 border-b border-line p-4">
            <SegmentedControl<Kind>
              label={t('archive.kind')}
              value={kind}
              onChange={setKind}
              options={[
                { value: 'all', label: t('archive.kind.all') },
                { value: 'tab', label: t('archive.kind.tab') },
                { value: 'window', label: t('archive.kind.window') },
              ]}
            />
            <SegmentedControl<Period>
              label={t('archive.period')}
              value={period}
              onChange={setPeriod}
              options={[
                { value: 'today', label: t('archive.period.today') },
                { value: 'week', label: t('archive.period.week') },
                { value: 'all', label: t('archive.period.all') },
              ]}
            />
            <SearchInput
              className="ms-auto min-w-48 flex-1"
              label={t('archive.search')}
              placeholder={t('archive.searchPlaceholder')}
              value={query}
              onValueChange={setQuery}
              onEscape={() => setQuery('')}
            />
          </div>
          {error && (
            <Banner tone="danger" className="m-4">
              {error}
            </Banner>
          )}
          {!loading && items.length === 0 && !error ? (
            <EmptyState
              icon={<Leaf size={20} />}
              title={filtered ? t('archive.noMatch') : t('archive.empty')}
            >
              {filtered ? t('archive.noMatch.text') : t('archive.empty.text')}
            </EmptyState>
          ) : (
            <table className="w-full table-fixed text-sm">
              <thead className="text-start text-[11px] tracking-wider text-muted uppercase">
                <tr>
                  <th scope="col" className="w-[44%] px-4 py-2 font-semibold">
                    {t('archive.col.item')}
                  </th>
                  <th scope="col" className="hidden px-2 py-2 font-semibold md:table-cell">
                    {t('archive.col.source')}
                  </th>
                  <th
                    scope="col"
                    className={cx(
                      'hidden px-2 py-2 font-semibold',
                      props.itemId ? 'xl:table-cell' : 'lg:table-cell',
                    )}
                  >
                    {t('archive.col.reason')}
                  </th>
                  <th scope="col" className="px-4 py-2 text-end font-semibold">
                    {t('archive.col.archived')}
                  </th>
                </tr>
              </thead>
              <tbody>
                {items.map((item) => {
                  const open = item.id === props.itemId;
                  return (
                    <tr
                      key={item.id}
                      className={cx(
                        'border-t border-line',
                        open ? 'bg-accent-soft' : 'hover:bg-surface-2',
                      )}
                    >
                      <td className="px-4 py-2.5">
                        <button
                          type="button"
                          data-item={item.id}
                          className="flex w-full min-w-0 items-center gap-3 text-start"
                          onClick={() => go(open ? '/' : `/item/${item.id}`)}
                          aria-expanded={open}
                        >
                          <ItemTile item={item} />
                          <span className="min-w-0">
                            <span className="block truncate font-medium">
                              {item.title || t('item.untitled')}
                            </span>
                            <span className="block truncate font-mono text-xs text-muted">
                              {item.kind === 'window'
                                ? t('item.appWindow', { app: item.appName })
                                : displayUrl(item.url)}
                            </span>
                          </span>
                        </button>
                      </td>
                      <td className="hidden truncate px-2 text-muted md:table-cell">
                        {item.sourceTitle
                          ? t('archive.source', { app: item.appName, source: item.sourceTitle })
                          : item.appName}
                      </td>
                      <td
                        className={cx(
                          'hidden px-2',
                          props.itemId ? 'xl:table-cell' : 'lg:table-cell',
                        )}
                      >
                        {item.reason && <Chip>{item.reason}</Chip>}
                      </td>
                      <td className="px-4 text-end font-mono text-xs whitespace-nowrap text-muted">
                        {item.archivedAt ? timeLabel(item.archivedAt, now) : ''}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          )}
          <footer className="flex items-center justify-between gap-3 border-t border-line px-4 py-3 text-xs text-muted">
            <span>
              {loading
                ? t('archive.loading')
                : filtered
                  ? t('archive.showingMatching', { n: items.length })
                  : t('archive.showing', { n: items.length, total })}
            </span>
            <span className="flex gap-2">
              {filtered && (
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => {
                    setQuery('');
                    setKind('all');
                    setPeriod('all');
                  }}
                >
                  {t('archive.clear')}
                </Button>
              )}
              <Button
                size="sm"
                variant="ghost"
                aria-disabled={exporting}
                onClick={() => {
                  if (exporting) return;
                  setExporting(true);
                  run(() =>
                    api
                      .exportArchive('json')
                      .then((r) => {
                        if (r) props.notify(exportSummary(r));
                      })
                      .finally(() => setExporting(false)),
                  );
                }}
              >
                {exporting
                  ? t('archive.exporting')
                  : filtered
                    ? t('archive.exportAll')
                    : t('archive.export')}
              </Button>
              {cursor && (
                <Button size="sm" onClick={() => load(true)} disabled={loading}>
                  {t('archive.more')}
                </Button>
              )}
            </span>
          </footer>
        </section>
        {props.itemId && (
          <ItemDrawer
            key={props.itemId}
            id={props.itemId}
            version={version}
            confirmRestore={props.confirmRestore}
            onClose={() => {
              const id = props.itemId;
              go('/');
              // Back to the row that opened it.
              setTimeout(() => {
                document.querySelector<HTMLElement>(`[data-item="${id}"]`)?.focus();
              });
            }}
            onDeleted={props.refresh}
            notify={props.notify}
          />
        )}
      </div>
    </div>
  );
}

/** The detail drawer (Designs/05): address, lifecycle, restore / keep / delete. */
export function ItemDrawer(props: {
  id: string;
  version: number;
  confirmRestore: boolean;
  onClose: () => void;
  onDeleted: () => void;
  notify: (text: string) => void;
}) {
  const item = useLoad(() => api.getItem(props.id), [props.id, props.version]);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState('');
  const [confirmDelete, setConfirmDelete] = useState(false);
  const heading = useRef<HTMLHeadingElement>(null);
  // biome-ignore lint/correctness/useExhaustiveDependencies: focus once the item has loaded
  useEffect(() => {
    if (item.data) heading.current?.focus();
  }, [item.data?.id]);
  // Escape closes the drawer.
  // biome-ignore lint/correctness/useExhaustiveDependencies: onClose is stable enough per item
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !e.defaultPrevented) props.onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  const act = (what: string, action: () => Promise<unknown>, done?: string) => {
    setBusy(what);
    setError('');
    action()
      .then(() => done && props.notify(done))
      .catch((e) => setError(errorMessage(e)))
      .finally(() => setBusy(''));
  };
  const d = item.data;
  const archived = d?.state === 'archived';
  return (
    <FeatureCard className="sticky top-4 grid gap-4">
      <div className="flex items-start justify-between gap-2">
        <p className="text-[11px] font-semibold tracking-wider text-feature-sub uppercase">
          {archived ? t('drawer.archived') : t('drawer.item')}
        </p>
        <IconButton
          onFeature
          label={t('drawer.close')}
          icon={<X size={16} />}
          onClick={props.onClose}
        />
      </div>
      {item.error && <p role="alert">{item.error}</p>}
      {d && (
        <>
          <h2
            ref={heading}
            tabIndex={-1}
            className="font-serif text-2xl leading-tight outline-none"
          >
            {d.title || t('item.untitled')}
          </h2>
          {props.confirmRestore && archived && (
            <div className="rounded-xl bg-surface p-3 text-sm text-ink">
              {t('drawer.confirmRestore', { kind: d.kind })}
            </div>
          )}
          {d.url && (
            <div className="flex items-center gap-2 rounded-xl border border-feature-tile-line bg-feature-tile p-3">
              <span className="min-w-0 flex-1 truncate font-mono text-xs" title={d.url}>
                {displayUrl(d.url)}
              </span>
              <IconButton
                onFeature
                label={t('drawer.copy')}
                icon={<Copy size={14} />}
                onClick={() =>
                  navigator.clipboard
                    .writeText(d.url ?? '')
                    .then(() => props.notify(t('drawer.copied')))
                    .catch(() => setError(t('drawer.copyFailed')))
                }
              />
            </div>
          )}
          <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
            <dt className="text-feature-sub">{t('drawer.from')}</dt>
            <dd>
              {d.sourceTitle
                ? t('archive.source', { app: d.appName, source: d.sourceTitle })
                : d.appName}
            </dd>
            {d.groupTitle && (
              <>
                <dt className="text-feature-sub">{t('drawer.group')}</dt>
                <dd>{d.groupTitle}</dd>
              </>
            )}
            {d.reason && (
              <>
                <dt className="text-feature-sub">{t('drawer.why')}</dt>
                <dd>{d.reason}</dd>
              </>
            )}
            {d.sweepNumber !== null && (
              <>
                <dt className="text-feature-sub">{t('drawer.sweep')}</dt>
                <dd>{t('drawer.sweepNumber', { n: d.sweepNumber })}</dd>
              </>
            )}
          </dl>
          <section
            aria-label={t('drawer.lifecycle')}
            className="rounded-xl border border-feature-tile-line bg-feature-tile p-3"
          >
            <h3 className="mb-2 text-[11px] font-semibold tracking-wider text-feature-sub uppercase">
              {t('drawer.lifecycle')}
            </h3>
            <ol className="grid gap-2 text-sm">
              {[...d.events]
                .reverse()
                .slice(-8)
                .map((e) => (
                  <li key={`${e.type}-${e.at}`} className="flex justify-between gap-3">
                    <span>
                      {eventText(e.type)}
                      {e.label && <span className="text-feature-sub"> · {e.label}</span>}
                    </span>
                    <time
                      className="font-mono text-xs whitespace-nowrap text-feature-sub"
                      dateTime={new Date(e.at).toISOString()}
                    >
                      {timeLabel(e.at, Date.now())}
                    </time>
                  </li>
                ))}
              {d.events.length === 0 && (
                <li className="text-feature-sub">{t('drawer.noHistory')}</li>
              )}
            </ol>
          </section>
          {d.kind === 'tab' && (
            <p className="flex gap-2 rounded-xl bg-surface p-3 text-xs text-ink">
              <Info size={14} aria-hidden className="mt-0.5 shrink-0" />
              <span>
                <strong className="block">{t('drawer.savesAddress')}</strong>
                {t('drawer.formsLost')}
              </span>
            </p>
          )}
          {error && (
            <p role="alert" className="rounded-xl bg-surface p-3 text-sm text-danger">
              {error}
            </p>
          )}
          <div className="flex flex-wrap gap-2">
            {archived && (
              <Button
                variant="onFeature"
                icon={<RotateCcw className="rtl:-scale-x-100" size={14} />}
                loading={busy === 'restore'}
                onClick={() => act('restore', () => api.restoreItems([d.id]))}
              >
                {t('drawer.restore')}
              </Button>
            )}
            {d.domain && (
              <Button
                variant="onFeatureOutline"
                icon={<ShieldCheck size={14} />}
                loading={busy === 'protect'}
                onClick={() =>
                  act(
                    'protect',
                    () => api.protectItem(d.id),
                    t('drawer.alwaysKept', { site: d.domain }),
                  )
                }
              >
                {t('drawer.alwaysKeep')}
              </Button>
            )}
            {archived &&
              (confirmDelete ? (
                <span className="flex gap-2">
                  <Button
                    variant="danger"
                    icon={<Trash2 size={14} />}
                    loading={busy === 'delete'}
                    autoFocus
                    onClick={() =>
                      act('delete', () =>
                        api.deleteItems([d.id]).then((r) => {
                          props.notify(
                            r.deleted > 0 ? t('drawer.deleted') : t('drawer.notDeleted'),
                          );
                          props.onDeleted();
                          props.onClose();
                        }),
                      )
                    }
                  >
                    {t('drawer.deleteForGood')}
                  </Button>
                  <Button variant="onFeatureOutline" onClick={() => setConfirmDelete(false)}>
                    {t('common.cancel')}
                  </Button>
                </span>
              ) : (
                <IconButton
                  onFeature
                  label={t('drawer.delete')}
                  icon={<Trash2 size={14} />}
                  onClick={() => setConfirmDelete(true)}
                />
              ))}
          </div>
        </>
      )}
    </FeatureCard>
  );
}
