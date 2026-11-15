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
    const t = setInterval(() => {
      const d = startOfDay(Date.now());
      setToday((old) => (old === d ? old : d));
    }, 60_000);
    return () => clearInterval(t);
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
    const t = setTimeout(() => load(false), query ? 120 : 0);
    return () => clearTimeout(t);
  }, [query, kind, period, version, today]);

  const s = stats.data;
  const filtered = query !== '' || kind !== 'all' || period !== 'all';
  return (
    <div className="grid gap-5">
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <h1 className="font-serif text-4xl font-normal">Library</h1>
          <p className="mt-1 text-sm text-muted">
            {s ? `${s.archivedTotal} tabs and windows archived` : ' '}
            <span className="ml-2 font-serif italic">· nothing is ever lost</span>
          </p>
        </div>
        <div className="flex items-center gap-2">
          {status?.mode === 'shadow' && (
            <Button icon={<Ghost size={14} />} onClick={() => go('/shadow-report')}>
              Shadow review
            </Button>
          )}
          <IconButton
            label="Undo last sweep"
            icon={<Undo2 size={16} />}
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
            Sweep now
          </Button>
        </div>
      </div>

      {s?.mode === 'shadow' && s.shadowDay !== null && (
        <Banner
          tone="warning"
          icon={<Ghost size={16} />}
          action={<Button onClick={() => go('/shadow-report')}>Review</Button>}
        >
          Shadow mode · {shadowDaysLeft(s.shadowDay)} — nothing is being closed yet.
        </Banner>
      )}
      {stats.error && <Banner tone="danger">{stats.error}</Banner>}

      <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
        <Stat label="Open tabs" icon={<AppWindow size={14} />} value={s?.openTabs ?? '–'} />
        <Stat
          label="Archived today"
          icon={<ArchiveIcon size={14} />}
          value={s ? s.archivedToday.tabs + s.archivedToday.windows : '–'}
          detail={s && `${s.archivedToday.tabs} tabs · ${s.archivedToday.windows} windows`}
        />
        <Stat
          label="Restored this week"
          icon={<RotateCcw size={14} />}
          value={s?.restoredWeek ?? '–'}
        />
        <Stat label="In the archive" icon={<Leaf size={14} />} value={s?.archivedTotal ?? '–'} />
      </div>

      <div className={cx('grid items-start gap-4', props.itemId && 'lg:grid-cols-[1fr_360px]')}>
        <section
          aria-label="Archive"
          className="min-w-0 rounded-card border border-line bg-surface shadow-sm"
        >
          <div className="flex flex-wrap items-center gap-3 border-b border-line p-4">
            <SegmentedControl<Kind>
              label="Kind"
              value={kind}
              onChange={setKind}
              options={[
                { value: 'all', label: 'All' },
                { value: 'tab', label: 'Tabs' },
                { value: 'window', label: 'Windows' },
              ]}
            />
            <SegmentedControl<Period>
              label="Archived"
              value={period}
              onChange={setPeriod}
              options={[
                { value: 'today', label: 'Today' },
                { value: 'week', label: 'This week' },
                { value: 'all', label: 'All time' },
              ]}
            />
            <SearchInput
              className="ml-auto min-w-48 flex-1"
              label="Search the archive"
              placeholder="Search by title, site or date…"
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
              title={filtered ? 'Nothing matches' : 'Nothing archived yet'}
            >
              {filtered
                ? 'Try other words, or clear the filters.'
                : 'Tabs you stop using are archived here, ready to come back in a keystroke.'}
            </EmptyState>
          ) : (
            <table className="w-full table-fixed text-sm">
              <thead className="text-left text-[11px] tracking-wider text-muted uppercase">
                <tr>
                  <th scope="col" className="w-[44%] px-4 py-2 font-semibold">
                    Item
                  </th>
                  <th scope="col" className="hidden px-2 py-2 font-semibold md:table-cell">
                    Source
                  </th>
                  <th
                    scope="col"
                    className={cx(
                      'hidden px-2 py-2 font-semibold',
                      props.itemId ? 'xl:table-cell' : 'lg:table-cell',
                    )}
                  >
                    Reason
                  </th>
                  <th scope="col" className="px-4 py-2 text-right font-semibold">
                    Archived
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
                          className="flex w-full min-w-0 items-center gap-3 text-left"
                          onClick={() => go(open ? '/' : `/item/${item.id}`)}
                          aria-expanded={open}
                        >
                          <ItemTile item={item} />
                          <span className="min-w-0">
                            <span className="block truncate font-medium">
                              {item.title || 'Untitled'}
                            </span>
                            <span className="block truncate font-mono text-xs text-muted">
                              {item.kind === 'window'
                                ? `${item.appName} window`
                                : displayUrl(item.url)}
                            </span>
                          </span>
                        </button>
                      </td>
                      <td className="hidden truncate px-2 text-muted md:table-cell">
                        {item.sourceTitle ? `${item.appName} — ${item.sourceTitle}` : item.appName}
                      </td>
                      <td
                        className={cx(
                          'hidden px-2',
                          props.itemId ? 'xl:table-cell' : 'lg:table-cell',
                        )}
                      >
                        {item.reason && <Chip>{item.reason}</Chip>}
                      </td>
                      <td className="px-4 text-right font-mono text-xs whitespace-nowrap text-muted">
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
                ? 'Loading…'
                : filtered
                  ? `Showing ${items.length} matching`
                  : `Showing ${items.length} of ${total} archived`}
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
                  Clear filters
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
                {exporting ? 'Exporting…' : filtered ? 'Export all (JSON)' : 'Export JSON'}
              </Button>
              {cursor && (
                <Button size="sm" onClick={() => load(true)} disabled={loading}>
                  Show more
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
          {archived ? 'Archived item' : 'Item'}
        </p>
        <IconButton
          onFeature
          label="Close details"
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
            {d.title || 'Untitled'}
          </h2>
          {props.confirmRestore && archived && (
            <div className="rounded-xl bg-surface p-3 text-sm text-ink">
              A link asked to restore this {d.kind}. Restore it?
            </div>
          )}
          {d.url && (
            <div className="flex items-center gap-2 rounded-xl border border-feature-tile-line bg-feature-tile p-3">
              <span className="min-w-0 flex-1 truncate font-mono text-xs" title={d.url}>
                {displayUrl(d.url)}
              </span>
              <IconButton
                onFeature
                label="Copy address"
                icon={<Copy size={14} />}
                onClick={() =>
                  navigator.clipboard
                    .writeText(d.url ?? '')
                    .then(() => props.notify('Address copied'))
                    .catch(() => setError("Couldn't copy the address"))
                }
              />
            </div>
          )}
          <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
            <dt className="text-feature-sub">From</dt>
            <dd>{d.sourceTitle ? `${d.appName} — ${d.sourceTitle}` : d.appName}</dd>
            {d.groupTitle && (
              <>
                <dt className="text-feature-sub">Group</dt>
                <dd>{d.groupTitle}</dd>
              </>
            )}
            {d.reason && (
              <>
                <dt className="text-feature-sub">Why</dt>
                <dd>{d.reason}</dd>
              </>
            )}
            {d.sweepNumber !== null && (
              <>
                <dt className="text-feature-sub">Sweep</dt>
                <dd>#{d.sweepNumber}</dd>
              </>
            )}
          </dl>
          <section
            aria-label="Lifecycle"
            className="rounded-xl border border-feature-tile-line bg-feature-tile p-3"
          >
            <h3 className="mb-2 text-[11px] font-semibold tracking-wider text-feature-sub uppercase">
              Lifecycle
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
                <li className="text-feature-sub">No history kept for this item.</li>
              )}
            </ol>
          </section>
          {d.kind === 'tab' && (
            <p className="flex gap-2 rounded-xl bg-surface p-3 text-xs text-ink">
              <Info size={14} aria-hidden className="mt-0.5 shrink-0" />
              <span>
                <strong className="block">Decay saves the address, not the page.</strong>
                Unsaved form input won't come back.
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
                icon={<RotateCcw size={14} />}
                loading={busy === 'restore'}
                onClick={() => act('restore', () => api.restoreItems([d.id]))}
              >
                Restore
              </Button>
            )}
            {d.domain && (
              <Button
                variant="onFeatureOutline"
                icon={<ShieldCheck size={14} />}
                loading={busy === 'protect'}
                onClick={() =>
                  act('protect', () => api.protectItem(d.id), `Decay will always keep ${d.domain}`)
                }
              >
                Always keep site
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
                            r.deleted > 0
                              ? 'Deleted from the archive'
                              : 'Nothing deleted: it was restored in the meantime',
                          );
                          props.onDeleted();
                          props.onClose();
                        }),
                      )
                    }
                  >
                    Delete for good
                  </Button>
                  <Button variant="onFeatureOutline" onClick={() => setConfirmDelete(false)}>
                    Cancel
                  </Button>
                </span>
              ) : (
                <IconButton
                  onFeature
                  label="Delete from the archive"
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
