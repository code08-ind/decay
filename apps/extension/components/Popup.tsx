import {
  ArrowRight,
  CircleAlert,
  History,
  Leaf,
  RotateCcw,
  Search,
  ShieldCheck,
} from 'lucide-react';
import {
  type KeyboardEvent,
  type ReactNode,
  useDeferredValue,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from 'react';
import type { Item } from '@/lib/db/types';
import { type PopupData, RECENT_COUNT } from '@/lib/popup/data';
import { type ArchiveSearch, dayLabel, type SearchGroup, timeLabel } from '@/lib/search/search';
import { avatarClass, shortUrl } from './format';
import { ProbeBanner } from './ProbeBanner';

export interface PopupProps {
  data: PopupData;
  /** Builds (or returns the cached) search index — called on the first keystroke only. */
  getSearch: () => ArchiveSearch;
  now: number;
  /** Mac shows ⌘, others Ctrl (for the batch-restore hint). */
  isMac: boolean;
  faviconFor?: (url: string) => string | undefined;
  onRestore: (item: Item) => void;
  onRestoreBatch: (item: Item) => void;
  onUndoLastSweep: () => void;
  onOpenReview: () => void;
  onOpenSweeps: () => void;
  onProtectSite: () => void;
  /** Ask for site access so pages can be checked for unsaved work (P0.7). */
  onAllowProbe?: () => void;
  onClose: () => void;
  busy?: boolean;
  status?: string | null;
  error?: string | null;
}

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`;

/** "2:14 PM" today, otherwise "Yesterday" / "Mon, 2 Nov". */
function whenLabel(ts: number, now: number): string {
  const day = dayLabel(ts, now);
  return day === 'Today' ? timeLabel(ts) : day;
}

function modeChip(data: PopupData, now: number): string {
  if (data.holdUntil !== null && data.holdUntil > now) {
    return `On hold until ${timeLabel(data.holdUntil)}`;
  }
  if (data.mode === 'shadow') {
    return `Shadow mode · day ${data.shadow.dayNumber}`;
  }
  return data.mode === 'automatic' ? 'Automatic' : 'Paused';
}

export function Popup(props: PopupProps) {
  const { data } = props;
  const [query, setQuery] = useState('');
  const [selected, setSelected] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listId = useId();
  const inputId = useId();
  // Typing stays responsive: the search runs on the deferred value.
  const deferred = useDeferredValue(query);
  const searching = query.trim().length > 0;
  const { getSearch, now } = props;
  const result = useMemo(() => {
    const q = deferred.trim();
    if (!q) return null;
    // One letter matches nearly everything: wait for more unless it's a filter.
    if (q.length < 2) return { tooShort: true as const };
    return getSearch().search(q, now);
  }, [deferred, getSearch, now]);
  const found = result && !('tooShort' in result) ? result : null;
  const items = found?.items ?? [];
  const current = items[Math.min(selected, items.length - 1)];

  useEffect(() => {
    inputRef.current?.focus();
  }, []);
  // biome-ignore lint/correctness/useExhaustiveDependencies: reset the selection on every new query
  useEffect(() => setSelected(0), [query]);

  const mod = props.isMac ? '⌘' : 'Ctrl';
  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      if (query) setQuery('');
      else props.onClose();
      return;
    }
    if (!items.length || e.nativeEvent.isComposing) return;
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setSelected((i) => Math.min(i + 1, items.length - 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setSelected((i) => Math.max(i - 1, 0));
    } else if (e.key === 'Enter' && current) {
      e.preventDefault();
      if (e.metaKey || e.ctrlKey) props.onRestoreBatch(current);
      else props.onRestore(current);
    }
  };

  const optionId = (item: Item) => `${listId}-${item.id}`;

  return (
    <div className="w-[380px] bg-canvas p-4 text-ink">
      <header className="flex items-center gap-2">
        <span className="grid size-8 place-items-center rounded-full bg-lime text-lime-ink">
          <Leaf size={16} aria-hidden />
        </span>
        <h1 className="font-serif text-xl">Decay</h1>
        <span className="inline-flex items-center gap-1.5 rounded-full border border-line bg-surface px-2.5 py-0.5 text-xs">
          <span className="size-1.5 rounded-full bg-accent-dot" aria-hidden />
          {modeChip(data, props.now)}
        </span>
        <button
          type="button"
          onClick={props.onOpenSweeps}
          className="ml-auto grid size-8 place-items-center rounded-full text-muted hover:bg-surface-2"
        >
          <History size={16} aria-hidden />
          <span className="sr-only">Sweep history</span>
        </button>
      </header>

      <div className="relative mt-3">
        <label htmlFor={inputId} className="sr-only">
          Search archived tabs
        </label>
        <Search
          size={16}
          className="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-faint"
          aria-hidden
        />
        <input
          ref={inputRef}
          id={inputId}
          type="text"
          autoComplete="off"
          spellCheck={false}
          role="combobox"
          aria-autocomplete="list"
          aria-expanded={items.length > 0}
          aria-controls={items.length > 0 ? listId : undefined}
          aria-activedescendant={current ? optionId(current) : undefined}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={onKeyDown}
          placeholder="Search archived tabs"
          className="w-full rounded-full border border-line bg-surface-2 py-2 pr-14 pl-9 text-sm placeholder:text-faint"
        />
        {/* Decorative hint (the shortcut is in the extension's commands), so not a <kbd>. */}
        <span
          aria-hidden="true"
          className="pointer-events-none absolute top-1/2 right-2 -translate-y-1/2 rounded-md border border-line bg-surface px-1.5 font-mono text-xs text-muted"
        >
          {props.isMac ? '⌥⇧D' : 'Alt+Shift+D'}
        </span>
      </div>

      {/* Live regions stay mounted so screen readers announce changes to them. */}
      <div aria-live="assertive">
        {props.error && (
          <p className="mt-3 flex items-center gap-2 rounded-tile bg-surface-2 px-3 py-2 text-sm text-danger">
            <CircleAlert size={14} aria-hidden /> {props.error}
          </p>
        )}
      </div>
      <div aria-live="polite">
        {props.status && (
          <p className="mt-3 rounded-tile bg-lime px-3 py-2 text-sm text-lime-ink">
            {props.status}
          </p>
        )}
        <span className="sr-only">
          {found ? (found.total ? `${plural(found.total, 'result')}` : 'No results') : ''}
        </span>
      </div>

      {!searching ? (
        <Home {...props} />
      ) : result && 'tooShort' in result ? (
        <p className="mt-4 px-1 text-sm text-muted">Keep typing to search your archive…</p>
      ) : found ? (
        <Results
          groups={found.groups}
          total={found.total}
          listId={listId}
          optionId={optionId}
          selectedId={current?.id}
          faviconFor={props.faviconFor}
          onRestore={props.onRestore}
          onRestoreBatch={props.onRestoreBatch}
          onHover={(item) => setSelected(Math.max(0, items.indexOf(item)))}
          mod={mod}
          busy={props.busy}
          empty={props.data.archived.length === 0}
        />
      ) : null}

      <footer className="mt-3 flex items-center justify-between gap-2 border-t border-line pt-3 text-sm">
        {searching ? (
          <p className="flex flex-wrap gap-x-3 text-xs text-muted">
            <span>
              <Key>↵</Key> Restore
            </span>
            <span>
              <Key>{mod}↵</Key> Restore batch
            </span>
            <span>
              <Key>Esc</Key> Clear
            </span>
          </p>
        ) : (
          <>
            <button
              type="button"
              onClick={props.onOpenSweeps}
              className="inline-flex items-center gap-1 text-muted hover:text-ink"
            >
              Sweeps &amp; undo <ArrowRight size={14} aria-hidden />
            </button>
            {data.currentSite &&
              (data.currentSite.protected ? (
                <span className="inline-flex items-center gap-1 text-xs text-muted">
                  <ShieldCheck size={14} aria-hidden /> {data.currentSite.host} is always kept
                </span>
              ) : (
                <button
                  type="button"
                  onClick={props.onProtectSite}
                  disabled={props.busy}
                  className="rounded-full border border-line-strong px-3 py-1 hover:bg-surface-2 disabled:opacity-50"
                >
                  Never archive this site
                </button>
              ))}
          </>
        )}
      </footer>
    </div>
  );
}

function Key({ children }: { children: ReactNode }) {
  return (
    <kbd className="rounded border border-line bg-surface px-1 font-mono text-[11px]">
      {children}
    </kbd>
  );
}

function Home(props: PopupProps) {
  const { data } = props;
  const automatic = data.mode === 'automatic';
  const showProbeBanner = data.probeAccess !== 'all' && props.onAllowProbe !== undefined;
  // Room for the banner within Chrome's 600 px popup height: one recent tab fewer.
  const recent = showProbeBanner ? data.recent.slice(0, RECENT_COUNT - 1) : data.recent;
  return (
    <>
      <div className="mt-3 grid grid-cols-2 gap-3">
        <Stat value={data.openTabs} label="open tabs" />
        <Stat
          value={automatic ? data.archivedToday : data.shadow.wouldArchive}
          label={automatic ? 'archived today' : 'would archive now'}
          dot
        />
      </div>
      {showProbeBanner && props.onAllowProbe && (
        <ProbeBanner
          compact
          partial={data.probeAccess === 'some'}
          onAllow={props.onAllowProbe}
          busy={props.busy}
        />
      )}
      <ModeCard {...props} />
      <section aria-label="Recently archived" className="mt-3">
        <div className="flex items-baseline justify-between px-1">
          <h2 className="text-xs font-medium tracking-widest text-muted uppercase">
            Recently archived
          </h2>
          <span className="text-xs text-muted">{plural(recent.length, 'tab')}</span>
        </div>
        {recent.length === 0 ? (
          <p className="mt-2 rounded-tile border border-line bg-surface px-3 py-4 text-center text-sm text-muted">
            {automatic
              ? 'Nothing archived yet. Stale tabs are put away at a quiet moment.'
              : 'Nothing is archived in shadow mode.'}
          </p>
        ) : (
          <ul className="mt-1.5 space-y-1.5">
            {recent.map((item) => (
              <li
                key={item.id}
                className="flex items-center gap-3 rounded-tile border border-line bg-surface px-3 py-1.5"
              >
                <ItemIcon item={item} faviconFor={props.faviconFor} />
                <span className="min-w-0 flex-1 truncate text-sm" title={item.url}>
                  {item.title || shortUrl(item.url)}
                </span>
                <span className="shrink-0 font-mono text-xs text-muted">
                  {whenLabel(item.archivedAt ?? 0, props.now)}
                </span>
                <button
                  type="button"
                  onClick={() => props.onRestore(item)}
                  disabled={props.busy}
                  aria-label={`Restore ${item.title || shortUrl(item.url)}`}
                  className="grid size-7 shrink-0 place-items-center rounded-full text-muted hover:bg-surface-2 hover:text-ink disabled:opacity-50"
                >
                  <RotateCcw size={14} aria-hidden />
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>
    </>
  );
}

function ModeCard(props: PopupProps) {
  const { data } = props;
  let eyebrow: string;
  let title: string;
  let body: string;
  let action: { label: string; onClick: () => void } | null;
  let secondary: { label: string; onClick: () => void } | null = null;
  if (data.mode === 'shadow') {
    const left = Math.max(0, data.shadow.targetDays - data.shadow.dayNumber);
    eyebrow = 'Shadow mode';
    title = 'Nothing is closed yet';
    body = data.shadow.ready
      ? 'You can switch on automatic archiving.'
      : `In ${plural(left || 1, 'day')} you can switch on automatic archiving.`;
    action = { label: 'Review what Decay would do', onClick: props.onOpenReview };
  } else if (data.mode === 'paused') {
    eyebrow = 'Paused';
    title = 'Nothing is archived';
    body = 'Decay keeps tracking tabs but closes nothing while paused.';
    action = { label: 'Open sweeps', onClick: props.onOpenSweeps };
  } else {
    eyebrow = 'Automatic';
    const last = data.lastSweep;
    if (last && last.sweep.itemCount > 0) {
      title = `Archived ${plural(last.sweep.itemCount, 'tab')}`;
      const reopened = last.sweep.itemCount - last.remaining;
      body = `Last sweep at ${timeLabel(last.sweep.startedAt)}.${
        last.remaining === 0
          ? ' Everything from it is open again.'
          : reopened > 0
            ? ` ${last.remaining} still archived.`
            : ''
      }`;
      action = { label: 'View sweeps', onClick: props.onOpenSweeps };
      if (last.remaining > 0 && last.sweep.status === 'done') {
        secondary = {
          label: last.remaining < last.sweep.itemCount ? `Undo ${last.remaining}` : 'Undo',
          onClick: props.onUndoLastSweep,
        };
      }
    } else {
      title = 'Watching your tabs';
      body = 'Stale tabs are archived at a quiet moment. Every sweep can be undone.';
      action = { label: 'View sweeps', onClick: props.onOpenSweeps };
    }
  }
  return (
    <section
      aria-label={eyebrow}
      className="mt-3 rounded-card bg-feature px-4 py-3 text-feature-ink"
    >
      <p className="text-xs font-medium tracking-widest text-feature-sub uppercase">{eyebrow}</p>
      <h2 className="mt-0.5 font-serif text-xl">{title}</h2>
      <p className="mt-1 text-sm text-feature-sub">{body}</p>
      <div className="mt-2.5 flex gap-2">
        {action && (
          <button
            type="button"
            onClick={action.onClick}
            className="inline-flex flex-1 items-center justify-center gap-1.5 rounded-full bg-surface px-4 py-2 text-sm font-medium text-ink"
          >
            {action.label} <ArrowRight size={14} aria-hidden />
          </button>
        )}
        {secondary && (
          <button
            type="button"
            onClick={secondary.onClick}
            disabled={props.busy}
            className="inline-flex items-center gap-1.5 rounded-full border border-feature-tile-line px-4 py-2 text-sm disabled:opacity-60"
          >
            <RotateCcw size={14} aria-hidden /> {secondary.label}
          </button>
        )}
      </div>
    </section>
  );
}

function Stat({ value, label, dot }: { value: number; label: string; dot?: boolean }) {
  return (
    <div className="relative rounded-tile border border-line bg-surface px-4 py-2">
      {dot && (
        <span className="absolute top-3 right-3 size-2 rounded-full bg-accent-dot" aria-hidden />
      )}
      <div className="font-serif text-2xl tabular-nums">{value}</div>
      <div className="text-sm text-muted">{label}</div>
    </div>
  );
}

function ItemIcon({
  item,
  faviconFor,
}: {
  item: Item;
  faviconFor?: (url: string) => string | undefined;
}) {
  const favicon = item.url && item.faviconUrl ? faviconFor?.(item.url) : undefined;
  const key = (item.domain ?? item.title ?? '').replace(/^www\./, '');
  return (
    <span
      className={`grid size-6 shrink-0 place-items-center overflow-hidden rounded-md text-xs font-medium ${
        favicon ? 'border border-line bg-surface-2' : avatarClass(key)
      }`}
    >
      {favicon ? (
        <img src={favicon} alt="" className="size-4" />
      ) : (
        <span aria-hidden>{key.charAt(0).toUpperCase() || '·'}</span>
      )}
    </span>
  );
}

function Results(props: {
  groups: SearchGroup[];
  total: number;
  listId: string;
  optionId: (item: Item) => string;
  selectedId?: string;
  faviconFor?: (url: string) => string | undefined;
  onRestore: (item: Item) => void;
  onRestoreBatch: (item: Item) => void;
  onHover: (item: Item) => void;
  mod: string;
  busy?: boolean;
  empty: boolean;
}) {
  const selectedRef = useRef<HTMLDivElement | null>(null);
  // biome-ignore lint/correctness/useExhaustiveDependencies: scroll when the selection moves
  useEffect(() => {
    selectedRef.current?.scrollIntoView?.({ block: 'nearest' });
  }, [props.selectedId]);

  if (!props.groups.length) {
    return (
      <p className="mt-4 rounded-tile border border-line bg-surface px-3 py-6 text-center text-sm text-muted">
        {props.empty
          ? 'Your archive is empty. Archived tabs show up here.'
          : 'No archived tabs match. Try fewer words, or site:, after:, before:.'}
      </p>
    );
  }
  return (
    <div
      id={props.listId}
      role="listbox"
      aria-label="Archived tabs"
      className="mt-3 max-h-[400px] overflow-y-auto"
    >
      {props.groups.map((group) => (
        // A listbox group (ARIA), not a form <fieldset>.
        // biome-ignore lint/a11y/useSemanticElements: role=group is the listbox grouping role
        <div key={group.key} role="group" aria-label={`${group.dayLabel} · ${group.sourceLabel}`}>
          {/* Visual header; the group's aria-label carries it, and ⌘/Ctrl+Enter is the keyboard path. */}
          <div
            aria-hidden="true"
            className="sticky top-0 z-10 flex items-center justify-between bg-canvas px-1 py-1.5"
          >
            <span className="truncate text-xs text-muted">
              <span className="font-medium text-ink">{group.dayLabel}</span> · {group.sourceLabel}
            </span>
            {group.items[0] && (
              <button
                type="button"
                tabIndex={-1}
                onClick={() => group.items[0] && props.onRestoreBatch(group.items[0])}
                disabled={props.busy}
                className="shrink-0 rounded-full px-2 py-0.5 text-xs text-accent-text hover:bg-surface-2 disabled:opacity-50"
              >
                Restore batch {props.mod}↵
              </button>
            )}
          </div>
          <div>
            {group.items.map((item) => {
              const isSelected = item.id === props.selectedId;
              return (
                // Combobox pattern: focus stays in the input (aria-activedescendant), so options
                // are not focusable and keyboard handling lives on the input.
                // biome-ignore lint/a11y/useFocusableInteractive: aria-activedescendant pattern
                // biome-ignore lint/a11y/useKeyWithClickEvents: keys are handled by the combobox input
                <div
                  key={item.id}
                  id={props.optionId(item)}
                  ref={isSelected ? selectedRef : undefined}
                  role="option"
                  aria-selected={isSelected}
                  onClick={() => props.onRestore(item)}
                  onMouseMove={() => !isSelected && props.onHover(item)}
                  className={`relative flex cursor-pointer items-center gap-3 rounded-tile px-3 py-2 ${
                    isSelected ? 'bg-chip' : 'hover:bg-surface-2'
                  }`}
                >
                  {isSelected && (
                    <span
                      className="absolute top-2 bottom-2 left-0 w-1 rounded-full bg-accent"
                      aria-hidden
                    />
                  )}
                  <ItemIcon item={item} faviconFor={props.faviconFor} />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-sm">
                      {item.title || shortUrl(item.url)}
                    </span>
                    <span className="block truncate font-mono text-xs text-muted">
                      {shortUrl(item.url)}
                    </span>
                  </span>
                  <span className="shrink-0 font-mono text-xs text-muted">
                    {timeLabel(item.archivedAt ?? 0)}
                  </span>
                </div>
              );
            })}
          </div>
        </div>
      ))}
      {props.total > props.groups.reduce((n, g) => n + g.items.length, 0) && (
        <p className="px-1 py-2 text-xs text-muted">
          Showing the best 50 of {props.total}. Add words or filters to narrow it down.
        </p>
      )}
    </div>
  );
}
