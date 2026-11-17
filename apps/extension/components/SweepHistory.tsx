import { formatDate } from '@decay/ui';
import {
  ChevronDown,
  CircleAlert,
  CircleCheck,
  Info,
  Lock,
  Plus,
  RotateCcw,
  Wind,
  X,
} from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import type { Reason, SweepSkipReason } from '@/lib/db/types';
import { localDay } from '@/lib/shadow/shadow';
import { t } from '@/lib/strings';
import type { SweepHistory as History, SweepEntry, SweepEntryItem } from '@/lib/sweep/history';
import { avatarClass, shortUrl } from './format';
import { PageHeader } from './PageHeader';

export interface SweepHistoryProps {
  history: History;
  now: number;
  faviconFor?: (url: string) => string | undefined;
  onUndo: (sweepId: string) => void;
  onSweepNow: () => void;
  /** Restore some of a sweep's archived tabs (into their original windows). */
  onRestore: (itemIds: string[]) => void;
  onOpenReview: () => void;
  /** Sweep id being undone, or 'sweepNow'. */
  busy?: string | null;
  error?: string | null;
  notice?: string | null;
}

const skipLabel = (reason: SweepSkipReason): string => t(`skip.${reason}`);

const time = (ts: number) => formatDate(ts, { hour: 'numeric', minute: '2-digit' });

export function dayLabel(ts: number, now: number): string {
  const day = localDay(ts);
  if (day === localDay(now)) return t('day.today');
  const yesterday = new Date(now);
  yesterday.setDate(yesterday.getDate() - 1);
  if (day === localDay(yesterday.getTime())) return t('day.yesterday');
  return formatDate(ts, { weekday: 'short', day: 'numeric', month: 'short' });
}

export function reasonLabel(reason: Reason | undefined): string {
  if (!reason || reason.type === 'protected') return t('reason.unused');
  return t(reason.type === 'customRule' ? 'reason.rule' : 'reason.hours', {
    n: reason.activeHours,
  });
}

function headline(entry: SweepEntry): string {
  if (entry.status === 'running') return t('sweeps.archiving');
  if (entry.status === 'nothing') return t('sweeps.nothingArchived');
  return t('sweeps.archived', { n: entry.archived });
}

type Range = '7' | '30' | 'all';
const RANGES: Range[] = ['7', '30', 'all'];

export function SweepHistory(props: SweepHistoryProps) {
  const { mode } = props.history;
  const [range, setRange] = useState<Range>('30');
  const entries = useMemo(() => {
    if (range === 'all') return props.history.entries;
    const since = props.now - Number(range) * 86_400_000;
    return props.history.entries.filter((e) => e.sweep.startedAt >= since);
  }, [props.history.entries, range, props.now]);
  const [selectedId, setSelectedId] = useState<string | null>(entries[0]?.sweep.id ?? null);
  // Keep a valid selection when the list refreshes.
  useEffect(() => {
    // Closing the panel sets null: keep it closed until the user picks a sweep.
    if (selectedId !== null && !entries.some((e) => e.sweep.id === selectedId))
      setSelectedId(entries[0]?.sweep.id ?? null);
  }, [entries, selectedId]);
  const selected = entries.find((e) => e.sweep.id === selectedId) ?? null;

  const groups = useMemo(() => {
    const out: { label: string; entries: SweepEntry[] }[] = [];
    for (const entry of entries) {
      const label = dayLabel(entry.sweep.startedAt, props.now);
      const last = out[out.length - 1];
      if (last?.label === label) last.entries.push(entry);
      else out.push({ label, entries: [entry] });
    }
    return out;
  }, [entries, props.now]);

  return (
    <div className="min-h-screen bg-canvas text-ink">
      <PageHeader current="sweeps" mode={mode} />

      <main className="mx-auto max-w-6xl px-4 py-8 sm:px-10">
        <div className="flex flex-col gap-4 sm:flex-row sm:items-end sm:justify-between">
          <div>
            <h1 className="font-serif text-4xl tracking-tight sm:text-5xl">{t('sweeps.title')}</h1>
            <p className="mt-2 text-muted">{t('sweeps.subtitle')}</p>
          </div>
          <div className="flex flex-wrap items-center gap-3 self-start sm:self-auto">
            <label className="relative">
              <span className="sr-only">{t('sweeps.rangeLabel')}</span>
              <select
                value={range}
                onChange={(e) => setRange(e.target.value as Range)}
                className="appearance-none rounded-full border border-line-strong bg-surface py-3 pe-10 ps-5 text-sm"
              >
                {RANGES.map((r) => (
                  <option key={r} value={r}>
                    {t(`sweeps.range.${r}`)}
                  </option>
                ))}
              </select>
              <ChevronDown
                size={16}
                className="pointer-events-none absolute top-1/2 end-4 -translate-y-1/2 text-muted"
                aria-hidden
              />
            </label>
            {mode !== 'paused' && (
              <button
                type="button"
                onClick={props.onSweepNow}
                disabled={props.busy != null}
                className="inline-flex items-center justify-center gap-2 rounded-full bg-primary px-5 py-3 font-medium text-primary-ink disabled:opacity-60"
              >
                <Plus size={16} aria-hidden />
                {mode === 'shadow'
                  ? props.busy === 'sweepNow'
                    ? t('sweeps.simulating')
                    : t('sweeps.simulate')
                  : props.busy === 'sweepNow'
                    ? t('sweeps.sweeping')
                    : t('sweeps.sweepNow')}
              </button>
            )}
          </div>
        </div>

        {mode !== 'automatic' && (
          <p className="mt-6 flex flex-wrap items-center gap-2 rounded-tile border border-line bg-surface-2 px-4 py-3 text-sm text-muted">
            <Info size={16} className="text-accent" aria-hidden />
            {mode === 'shadow' ? t('sweeps.shadowNote') : t('sweeps.pausedNote')}
            {mode === 'shadow' && (
              <button
                type="button"
                onClick={props.onOpenReview}
                className="text-accent-text underline underline-offset-4"
              >
                {t('sweeps.openReview')}
              </button>
            )}
          </p>
        )}

        {props.error && (
          <p
            role="alert"
            className="mt-4 flex items-center gap-2 rounded-tile bg-surface-2 px-4 py-3 text-sm text-danger"
          >
            <CircleAlert size={16} aria-hidden /> {props.error}
          </p>
        )}
        {props.notice && (
          <p
            role="status"
            className="mt-4 flex items-center gap-2 rounded-tile bg-lime px-4 py-3 text-sm text-lime-ink"
          >
            <CircleCheck size={16} aria-hidden /> {props.notice}
          </p>
        )}

        <div className="mt-6 grid grid-cols-[minmax(0,1fr)] gap-6 lg:grid-cols-[minmax(0,1fr)_420px]">
          <section
            aria-label={t('sweeps.history')}
            className="min-w-0 rounded-card border border-line bg-surface p-4 shadow-[var(--shadow-card)] sm:p-6"
          >
            {entries.length === 0 ? (
              <p className="py-12 text-center text-muted">
                {mode === 'automatic' ? t('sweeps.empty.automatic') : t('sweeps.empty')}
              </p>
            ) : (
              groups.map((group) => (
                <div
                  key={group.label}
                  className="not-first:mt-4 not-first:border-t not-first:border-line not-first:pt-4"
                >
                  <h2 className="px-2 pb-2 text-xs font-medium tracking-widest text-muted uppercase">
                    {group.label}
                  </h2>
                  <ul className="space-y-1">
                    {group.entries.map((entry) => (
                      <SweepRow
                        key={entry.sweep.id}
                        entry={entry}
                        selected={entry.sweep.id === selectedId}
                        onSelect={() => setSelectedId(entry.sweep.id)}
                        onUndo={() => props.onUndo(entry.sweep.id)}
                        busy={props.busy === entry.sweep.id}
                        disabled={props.busy != null}
                      />
                    ))}
                  </ul>
                </div>
              ))
            )}
          </section>

          {selected && (
            <SweepDetail
              entry={selected}
              now={props.now}
              faviconFor={props.faviconFor}
              onUndo={() => props.onUndo(selected.sweep.id)}
              onRestore={props.onRestore}
              onClose={() => setSelectedId(null)}
              busy={props.busy === selected.sweep.id}
              disabled={props.busy != null}
            />
          )}
        </div>
      </main>

      <footer className="border-t border-line px-4 py-3 text-sm text-muted sm:px-10">
        <p className="mx-auto flex max-w-6xl items-center gap-2">
          <Lock size={14} aria-hidden /> {t('sweeps.local')}
        </p>
      </footer>
    </div>
  );
}

function SweepRow(props: {
  entry: SweepEntry;
  selected: boolean;
  onSelect: () => void;
  onUndo: () => void;
  busy: boolean;
  disabled: boolean;
}) {
  const { entry } = props;
  const undone = entry.status === 'undone';
  const skipped = entry.sweep.items.filter(
    (e) => e.result !== 'ok' && e.result !== 'pending',
  ).length;
  return (
    <li
      className={`relative flex items-center gap-3 rounded-tile border px-3 py-3 ${
        props.selected ? 'border-accent/40 bg-chip' : 'border-transparent'
      }`}
    >
      {props.selected && (
        <span className="absolute top-3 bottom-3 start-0 w-1 rounded-full bg-accent" aria-hidden />
      )}
      <span className="grid size-10 shrink-0 place-items-center rounded-full bg-surface-3 text-muted">
        {undone ? (
          <RotateCcw className="rtl:-scale-x-100" size={18} aria-hidden />
        ) : (
          <Wind size={18} aria-hidden />
        )}
      </span>
      <button
        type="button"
        onClick={props.onSelect}
        aria-pressed={props.selected}
        className="min-w-0 flex-1 text-start"
      >
        <span className="flex flex-wrap items-center gap-2">
          <span className={`font-serif text-xl ${undone ? 'text-muted line-through' : ''}`}>
            {t('sweeps.number', { n: entry.number })}
          </span>
          {entry.archived > 0 && (
            <span className="rounded-full border border-line px-2 py-0.5 text-sm">
              {t('sweeps.tabs', { n: entry.archived })}
            </span>
          )}
          {skipped > 0 && (
            <span className="rounded-full border border-line px-2 py-0.5 text-sm text-muted">
              {t('sweeps.keptOpen', { n: skipped })}
            </span>
          )}
        </span>
        <span className="mt-0.5 block text-sm text-muted">
          {t('sweeps.meta', { time: time(entry.sweep.startedAt), trigger: entry.sweep.trigger })}
        </span>
      </button>
      <div className="flex shrink-0 flex-wrap items-center justify-end gap-2">
        {entry.status === 'partial' && (
          <span className="hidden rounded-full bg-lime px-3 py-1 text-sm text-lime-ink sm:inline">
            {t('sweeps.partly', { n: entry.restored })}
          </span>
        )}
        {entry.undoable ? (
          <button
            type="button"
            onClick={props.onUndo}
            disabled={props.disabled}
            aria-label={t('sweeps.undoLabel', {
              state: props.busy ? 'busy' : entry.status === 'partial' ? 'partial' : 'all',
              n: entry.number,
            })}
            className="rounded-full border border-line-strong bg-surface px-4 py-1.5 text-sm hover:bg-surface-2 disabled:opacity-50"
          >
            {props.busy
              ? t('sweeps.undoing')
              : entry.status === 'partial'
                ? t('sweeps.undoRest')
                : t('sweeps.undo')}
          </button>
        ) : (
          <span className="rounded-full border border-line px-3 py-1 text-sm text-faint">
            {entry.status === 'undone'
              ? t('sweeps.state.undone')
              : entry.status === 'running'
                ? t('sweeps.state.running')
                : entry.status === 'nothing'
                  ? t('sweeps.state.nothing')
                  : t('sweeps.state.old')}
          </span>
        )}
      </div>
    </li>
  );
}

function SweepDetail(props: {
  entry: SweepEntry;
  now: number;
  faviconFor?: (url: string) => string | undefined;
  onUndo: () => void;
  onRestore: (itemIds: string[]) => void;
  onClose: () => void;
  busy: boolean;
  disabled: boolean;
}) {
  const { entry } = props;
  const [picked, setPicked] = useState<ReadonlySet<string>>(new Set());
  // A different sweep, or items restored meanwhile: drop stale picks.
  useEffect(() => {
    setPicked((prev) => {
      const restorable = new Set(
        entry.items.filter((r) => isRestorable(r, entry)).map((r) => r.entry.itemId),
      );
      const next = new Set([...prev].filter((id) => restorable.has(id)));
      return next.size === prev.size ? prev : next;
    });
  }, [entry]);
  const restorableIds = entry.items
    .filter((r) => isRestorable(r, entry))
    .map((r) => r.entry.itemId);
  const toggle = (id: string) =>
    setPicked((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  const when = t('sweeps.detail.when', {
    day: dayLabel(entry.sweep.startedAt, props.now),
    time: time(entry.sweep.startedAt),
  });
  return (
    <aside
      aria-label={t('sweeps.number', { n: entry.number })}
      className="min-w-0 self-start rounded-card bg-feature p-6 text-feature-ink"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <p className="text-xs font-medium tracking-widest text-feature-sub uppercase">
            {t('sweeps.detail.title', { n: entry.number, when })}
          </p>
          <h2 className="mt-2 font-serif text-3xl">{headline(entry)}</h2>
        </div>
        <button
          type="button"
          onClick={props.onClose}
          className="grid size-9 shrink-0 place-items-center rounded-full border border-feature-tile-line"
        >
          <X size={16} aria-hidden />
          <span className="sr-only">{t('sweeps.detail.close')}</span>
        </button>
      </div>

      <p className="mt-4 rounded-tile bg-feature-tile px-4 py-3 text-sm">
        {t('sweeps.detail.trigger', { trigger: entry.sweep.trigger })}
      </p>

      <div className="mt-4 rounded-tile border border-feature-tile-line bg-feature-tile p-4">
        <div className="flex items-center justify-between gap-2">
          <h3 className="text-xs font-medium tracking-widest text-feature-sub uppercase">
            {t('sweeps.detail.items')}
          </h3>
          {restorableIds.length > 1 && (
            <button
              type="button"
              onClick={() =>
                setPicked(picked.size === restorableIds.length ? new Set() : new Set(restorableIds))
              }
              className="text-xs text-feature-sub underline underline-offset-4"
            >
              {picked.size === restorableIds.length
                ? t('sweeps.detail.none')
                : t('sweeps.detail.all')}
            </button>
          )}
        </div>
        <ul className="mt-2 max-h-[420px] space-y-1 overflow-y-auto">
          {entry.items.map((row) => (
            <DetailItem
              key={row.entry.itemId}
              row={row}
              faviconFor={props.faviconFor}
              selectable={isRestorable(row, entry)}
              picked={picked.has(row.entry.itemId)}
              onToggle={() => toggle(row.entry.itemId)}
            />
          ))}
        </ul>
      </div>

      <p className="mt-4 text-sm text-feature-sub">{t('sweeps.detail.saved')}</p>

      {restorableIds.length > 0 && (
        <div className="mt-4 flex flex-wrap gap-2 border-t border-feature-tile-line pt-4">
          {entry.undoable && (
            <button
              type="button"
              onClick={props.onUndo}
              disabled={props.disabled}
              className="inline-flex items-center gap-2 rounded-full bg-surface px-5 py-2.5 font-medium text-ink disabled:opacity-60"
            >
              <RotateCcw className="rtl:-scale-x-100" size={16} aria-hidden />
              {props.busy
                ? t('sweeps.undoing')
                : entry.status === 'partial'
                  ? t('sweeps.detail.undoOther', { n: entry.remaining })
                  : t('sweeps.detail.undoAll')}
            </button>
          )}
          <button
            type="button"
            onClick={() => props.onRestore([...picked])}
            disabled={props.disabled || picked.size === 0}
            className="rounded-full border border-feature-tile-line px-5 py-2.5 font-medium disabled:opacity-50"
          >
            {picked.size
              ? t('sweeps.detail.restoreN', { n: picked.size })
              : t('sweeps.detail.restore')}
          </button>
        </div>
      )}
    </aside>
  );
}

const isRestorable = (row: SweepEntryItem, entry: SweepEntry) =>
  row.entry.result === 'ok' &&
  row.item?.state === 'archived' &&
  row.item.lastSweepId === entry.sweep.id;

function DetailItem({
  row,
  faviconFor,
  selectable,
  picked,
  onToggle,
}: {
  row: SweepEntryItem;
  faviconFor?: (url: string) => string | undefined;
  selectable: boolean;
  picked: boolean;
  onToggle: () => void;
}) {
  const { item, entry } = row;
  const title = item?.title || shortUrl(item?.url) || t('sweeps.item.deleted');
  const key = (item?.domain ?? title).replace(/^www\./, '');
  const favicon = item?.url && item.faviconUrl ? faviconFor?.(item.url) : undefined;
  let note: string;
  if (entry.result === 'pending') note = t('sweeps.item.closing');
  else if (entry.result !== 'ok')
    note = entry.skipReason ? skipLabel(entry.skipReason) : t('sweeps.item.keptOpen');
  else if (item && item.state !== 'archived') note = t('sweeps.item.restored');
  else note = reasonLabel(item?.archiveReason);
  return (
    <li className="flex items-center gap-3 py-1.5">
      {selectable ? (
        <input
          type="checkbox"
          checked={picked}
          onChange={onToggle}
          aria-label={t('sweeps.item.select', { title })}
          className="size-4 shrink-0 accent-[var(--lime)]"
        />
      ) : (
        <span className="size-4 shrink-0" aria-hidden />
      )}
      <span
        className={`grid size-7 shrink-0 place-items-center overflow-hidden rounded-lg text-xs font-medium ${
          favicon ? 'bg-surface' : avatarClass(key)
        }`}
      >
        {favicon ? (
          <img src={favicon} alt="" className="size-4" />
        ) : (
          key.charAt(0).toUpperCase() || '·'
        )}
      </span>
      <span
        className={`min-w-0 flex-1 truncate ${entry.result === 'ok' ? '' : 'text-feature-sub'}`}
        title={item?.url}
      >
        {title}
      </span>
      <span
        className="max-w-[45%] shrink-0 truncate text-end font-mono text-xs text-feature-sub"
        title={note}
      >
        {note}
      </span>
    </li>
  );
}
