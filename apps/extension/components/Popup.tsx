import {
  ArrowRight,
  ChartColumn,
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
import type { AppLinkStatus } from '@/lib/bridge/chrome';
import type { Item } from '@/lib/db/types';
import { type PopupData, RECENT_COUNT } from '@/lib/popup/data';
import {
  type ArchiveSearch,
  dayLabel,
  type SearchGroup,
  type SearchResult,
  timeLabel,
} from '@/lib/search/search';
import { localDay } from '@/lib/shadow/shadow';
import { t } from '@/lib/strings';
import { AppLink } from './AppLink';
import { avatarClass, shortUrl } from './format';
import { ProbeBanner } from './ProbeBanner';

export interface PopupProps {
  data: PopupData;
  /** Builds (or returns the cached) search index — called on the first keystroke only. */
  getSearch: () => ArchiveSearch;
  /**
   * Searches the Decay app's archive instead (adapter mode, P1.5): asynchronous, so the
   * results follow the typing after a short pause.
   */
  searchApp?: (query: string) => Promise<SearchResult>;
  now: number;
  /** Mac shows ⌘, others Ctrl (for the batch-restore hint). */
  isMac: boolean;
  faviconFor?: (url: string) => string | undefined;
  /** `fromSearch`: picked from search results (for the search → restore metric, P0.8). */
  onRestore: (item: Item, opts?: { fromSearch?: boolean }) => void;
  onRestoreBatch: (item: Item, opts?: { fromSearch?: boolean }) => void;
  /** Every change of the search text (metrics: searches, time to restore). */
  onQueryChange?: (query: string) => void;
  onUndoLastSweep: () => void;
  onOpenReview: () => void;
  onOpenSweeps: () => void;
  onOpenInsights?: () => void;
  onProtectSite: () => void;
  /** Ask for site access so pages can be checked for unsaved work (P0.7). */
  onAllowProbe?: () => void;
  /** The link to the Decay desktop app (P1.4); hidden when not given. */
  appLink?: { status: AppLinkStatus | null; enabled: boolean };
  onConnectApp?: () => void;
  onDisconnectApp?: () => void;
  onClose: () => void;
  busy?: boolean;
  status?: string | null;
  error?: string | null;
}

/** App search waits for a pause in typing. */
const APP_SEARCH_DELAY_MS = 120;

/** "2:14 PM" today, otherwise "Yesterday" / "Mon, 2 Nov". */
function whenLabel(ts: number, now: number): string {
  return localDay(ts) === localDay(now) ? timeLabel(ts) : dayLabel(ts, now);
}

function modeChip(data: PopupData, now: number): string {
  if (data.holdUntil !== null && data.holdUntil > now) {
    return t('popup.onHold', { time: timeLabel(data.holdUntil) });
  }
  if (data.mode === 'shadow') {
    return t('popup.shadowDay', { n: data.shadow.dayNumber });
  }
  return data.mode === 'automatic' ? t('popup.automatic') : t('popup.paused');
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
  const { getSearch, now, searchApp } = props;
  // App search: the answer for the latest query (older answers are dropped).
  const [appAnswer, setAppAnswer] = useState<{ q: string; result: SearchResult | null } | null>(
    null,
  );
  useEffect(() => {
    const q = deferred.trim();
    if (!searchApp || q.length < 2) return;
    let current = true;
    const timer = setTimeout(() => {
      searchApp(q).then(
        (r) => {
          if (current) setAppAnswer({ q, result: r });
        },
        () => {
          // The link dropped or the app failed: say so (not "no results").
          if (current) setAppAnswer({ q, result: null });
        },
      );
    }, APP_SEARCH_DELAY_MS);
    return () => {
      current = false;
      clearTimeout(timer);
    };
  }, [deferred, searchApp]);
  const result = useMemo(() => {
    const q = deferred.trim();
    if (!q) return null;
    // One letter matches nearly everything: wait for more unless it's a filter.
    if (q.length < 2) return { tooShort: true as const };
    if (searchApp) {
      if (appAnswer?.q !== q) return { pending: true as const };
      return appAnswer.result ?? { failed: true as const };
    }
    return getSearch().search(q, now);
  }, [deferred, getSearch, now, searchApp, appAnswer]);
  const found =
    result && !('tooShort' in result) && !('pending' in result) && !('failed' in result)
      ? result
      : null;
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
      if (query) {
        setQuery('');
        props.onQueryChange?.(''); // a new search starts the search → restore timer over
      } else props.onClose();
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
      if (e.metaKey || e.ctrlKey) props.onRestoreBatch(current, { fromSearch: true });
      else props.onRestore(current, { fromSearch: true });
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
        {props.onOpenInsights && (
          <button
            type="button"
            onClick={props.onOpenInsights}
            title={t('popup.insights')}
            className="ms-auto grid size-8 place-items-center rounded-full text-muted hover:bg-surface-2"
          >
            <ChartColumn size={16} aria-hidden />
            <span className="sr-only">{t('popup.insights')}</span>
          </button>
        )}
        <button
          type="button"
          onClick={props.onOpenSweeps}
          title={t('popup.sweepHistory')}
          className={`${props.onOpenInsights ? '' : 'ms-auto '}grid size-8 place-items-center rounded-full text-muted hover:bg-surface-2`}
        >
          <History size={16} aria-hidden />
          <span className="sr-only">{t('popup.sweepHistory')}</span>
        </button>
      </header>

      <div className="relative mt-3">
        <label htmlFor={inputId} className="sr-only">
          {t('popup.search')}
        </label>
        <Search
          size={16}
          className="pointer-events-none absolute top-1/2 start-3 -translate-y-1/2 text-faint"
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
          onChange={(e) => {
            setQuery(e.target.value);
            props.onQueryChange?.(e.target.value);
          }}
          onKeyDown={onKeyDown}
          placeholder={t('popup.search')}
          className="w-full rounded-full border border-line bg-surface-2 py-2 pe-14 ps-9 text-sm placeholder:text-faint"
        />
        {/* Decorative hint (the shortcut is in the extension's commands), so not a <kbd>. */}
        <span
          aria-hidden="true"
          className="pointer-events-none absolute top-1/2 end-2 -translate-y-1/2 rounded-md border border-line bg-surface px-1.5 font-mono text-xs text-muted"
        >
          {/* i18n-ignore: a shortcut, as the keyboard says it */}
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
        <span className="sr-only">{found ? t('popup.results', { n: found.total }) : ''}</span>
      </div>

      {!searching ? (
        <Home {...props} />
      ) : result && 'tooShort' in result ? (
        <p className="mt-4 px-1 text-sm text-muted">{t('popup.keepTyping')}</p>
      ) : result && 'failed' in result ? (
        <p className="mt-4 px-1 text-sm text-danger">{t('popup.searchFailed')}</p>
      ) : result && 'pending' in result ? (
        <p className="mt-4 px-1 text-sm text-muted" aria-busy="true">
          {t('popup.searching')}
        </p>
      ) : found ? (
        <Results
          groups={found.groups}
          total={found.total}
          listId={listId}
          optionId={optionId}
          selectedId={current?.id}
          faviconFor={props.faviconFor}
          onRestore={(item) => props.onRestore(item, { fromSearch: true })}
          onRestoreBatch={(item) => props.onRestoreBatch(item, { fromSearch: true })}
          onHover={(item) => setSelected(Math.max(0, items.indexOf(item)))}
          mod={mod}
          busy={props.busy}
          empty={props.data.archived.length === 0}
        />
      ) : null}

      {!searching && props.appLink && props.onConnectApp && props.onDisconnectApp && (
        <AppLink
          status={props.appLink.status}
          enabled={props.appLink.enabled}
          busy={props.busy}
          onConnect={props.onConnectApp}
          onDisconnect={props.onDisconnectApp}
        />
      )}

      <footer className="mt-3 flex items-center justify-between gap-2 border-t border-line pt-3 text-sm">
        {searching ? (
          <p className="flex flex-wrap gap-x-3 text-xs text-muted">
            <span>
              <Key>↵</Key> {t('popup.key.restore')}
            </span>
            <span>
              <Key>{mod}↵</Key> {t('popup.key.batch')}
            </span>
            <span>
              <Key>Esc</Key> {t('popup.key.clear')}
            </span>
          </p>
        ) : (
          <>
            <button
              type="button"
              onClick={props.onOpenSweeps}
              className="inline-flex items-center gap-1 text-muted hover:text-ink"
            >
              {t('popup.sweepsUndo')}{' '}
              <ArrowRight className="rtl:-scale-x-100" size={14} aria-hidden />
            </button>
            {data.currentSite &&
              (data.currentSite.protected ? (
                <span className="inline-flex items-center gap-1 text-xs text-muted">
                  <ShieldCheck size={14} aria-hidden />{' '}
                  {t('popup.siteKept', { host: data.currentSite.host })}
                </span>
              ) : (
                <button
                  type="button"
                  onClick={props.onProtectSite}
                  disabled={props.busy}
                  className="rounded-full border border-line-strong px-3 py-1 hover:bg-surface-2 disabled:opacity-50"
                >
                  {t('popup.neverArchive')}
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
        <Stat value={data.openTabs} label={t('popup.openTabs')} />
        <Stat
          value={automatic ? data.archivedToday : data.shadow.wouldArchive}
          label={automatic ? t('popup.archivedToday') : t('popup.wouldArchive')}
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
      <section aria-label={t('popup.recent')} className="mt-3">
        <div className="flex items-baseline justify-between px-1">
          <h2 className="text-xs font-medium tracking-widest text-muted uppercase">
            {t('popup.recent')}
          </h2>
          {/* The newest few only: a count here read like a total (QA P0). */}
          {recent.length > 0 && (
            <span className="text-xs text-muted">{t('popup.latest', { n: recent.length })}</span>
          )}
        </div>
        {recent.length === 0 ? (
          <p className="mt-2 rounded-tile border border-line bg-surface px-3 py-4 text-center text-sm text-muted">
            {automatic ? t('popup.recent.automatic') : t('popup.recent.shadow')}
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
                  aria-label={t('popup.restoreItem', { title: item.title || shortUrl(item.url) })}
                  className="grid size-7 shrink-0 place-items-center rounded-full text-muted hover:bg-surface-2 hover:text-ink disabled:opacity-50"
                >
                  <RotateCcw className="rtl:-scale-x-100" size={14} aria-hidden />
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
    eyebrow = t('popup.card.shadow');
    title = t('popup.card.shadow.title');
    body = data.shadow.ready
      ? t('popup.card.shadow.ready')
      : t('popup.card.shadow.wait', { n: left || 1 });
    action = { label: t('popup.card.shadow.action'), onClick: props.onOpenReview };
  } else if (data.mode === 'paused') {
    eyebrow = t('popup.card.paused');
    title = t('popup.card.paused.title');
    body = t('popup.card.paused.text');
    action = { label: t('popup.card.openSweeps'), onClick: props.onOpenSweeps };
  } else {
    eyebrow = t('popup.card.automatic');
    const last = data.lastSweep;
    if (last && last.sweep.itemCount > 0) {
      title = t('popup.card.archived', { n: last.sweep.itemCount });
      const reopened = last.sweep.itemCount - last.remaining;
      const time = timeLabel(last.sweep.startedAt);
      body =
        last.remaining === 0
          ? t('popup.card.allOpen', { time })
          : reopened > 0
            ? t('popup.card.stillArchived', { time, n: last.remaining })
            : t('popup.card.lastSweep', { time });
      action = { label: t('popup.card.viewSweeps'), onClick: props.onOpenSweeps };
      if (last.remaining > 0 && last.sweep.status === 'done') {
        secondary = {
          label:
            last.remaining < last.sweep.itemCount
              ? t('popup.card.undoN', { n: last.remaining })
              : t('popup.card.undo'),
          onClick: props.onUndoLastSweep,
        };
      }
    } else {
      title = t('popup.card.watching');
      body = t('popup.card.watching.text');
      action = { label: t('popup.card.viewSweeps'), onClick: props.onOpenSweeps };
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
            {action.label} <ArrowRight className="rtl:-scale-x-100" size={14} aria-hidden />
          </button>
        )}
        {secondary && (
          <button
            type="button"
            onClick={secondary.onClick}
            disabled={props.busy}
            className="inline-flex items-center gap-1.5 rounded-full border border-feature-tile-line px-4 py-2 text-sm disabled:opacity-60"
          >
            <RotateCcw className="rtl:-scale-x-100" size={14} aria-hidden /> {secondary.label}
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
        <span className="absolute top-3 end-3 size-2 rounded-full bg-accent-dot" aria-hidden />
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
        {props.empty ? t('popup.empty') : t('popup.noMatch')}
      </p>
    );
  }
  return (
    <div
      id={props.listId}
      role="listbox"
      aria-label={t('popup.list')}
      className="mt-3 max-h-[400px] overflow-y-auto"
    >
      {props.groups.map((group) => (
        // A listbox group (ARIA), not a form <fieldset>.
        // biome-ignore lint/a11y/useSemanticElements: role=group is the listbox grouping role
        <div
          key={group.key}
          role="group"
          aria-label={t('popup.group', { day: group.dayLabel, source: group.sourceLabel })}
        >
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
                {t('popup.batch', { keys: `${props.mod}↵` })}
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
                      className="absolute top-2 bottom-2 start-0 w-1 rounded-full bg-accent"
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
        <p className="px-1 py-2 text-xs text-muted">{t('popup.best', { n: props.total })}</p>
      )}
    </div>
  );
}
