import { Chip, cx, detectPlatform, EmptyState, getLocale, Kbd, SearchInput } from '@decay/ui';
import { listen } from '@tauri-apps/api/event';
import { Archive, CircleAlert, Clock, History } from 'lucide-react';
import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { api, errorMessage, type PanelItem, restoreChoices } from '../api';
import {
  displayUrl,
  groupResults,
  highlight,
  monogram,
  type PanelAction,
  panelAction,
  type ResultGroup,
  searchWords,
  timeLabel,
} from '../panel/results';
import { t } from '../strings';

type Kind = 'all' | 'tab' | 'window';

const KINDS: Kind[] = ['all', 'tab', 'window'];

const TINT = {
  1: 'bg-avatar-1 text-avatar-1-ink',
  2: 'bg-avatar-2 text-avatar-2-ink',
  3: 'bg-avatar-3 text-avatar-3-ink',
  4: 'bg-avatar-4 text-avatar-4-ink',
} as const;

const MAC = detectPlatform() === 'mac';
const MOD = MAC ? '⌘' : 'Ctrl+';

/** How long typing waits before searching (Software Design §5.1: 30 ms). */
const DEBOUNCE_MS = 30;
const PAGE = 50;
/** A restore that hasn't answered by then is reported as such (the engine times out too). */
const RESTORE_WAIT_MS = 12_000;

interface Results {
  /** The query and filter these results are for. */
  key: string;
  items: PanelItem[];
  cursor: string | null;
}

type Note = {
  text: string;
  tone: 'info' | 'error' | 'busy';
  choices?: RestoreChoices;
  /** A Pro feature: the note offers the Decay Pro tab (P4.2). */
  pro?: boolean;
};
type RestoreChoices = NonNullable<ReturnType<typeof restoreChoices>>;

const keyOf = (query: string, kind: Kind) => `${kind}\u0000${query}`;

// The search panel (P1.9, PRD F5/F6, Designs/01): Spotlight-style, keyboard-first.
// ↑↓ select · ↵ restore · ⌘↵ restore the group · ⌥↵ into a new window · ⌘P always keep
// the site · ⌘I details · Esc clears, then closes.
export function Panel() {
  const box = useRef<HTMLDivElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const [query, setQuery] = useState('');
  const [kind, setKind] = useState<Kind>('all');
  const [results, setResults] = useState<Results>({ key: '', items: [], cursor: null });
  const [total, setTotal] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState(0);
  const [note, setNote] = useState<Note | null>(null);
  // Pro features on (P4.2): ⌘↵ restores a whole group only with Pro. Unknown (`null`)
  // until read; the engine refuses Pro work on its own side too.
  const [pro, setPro] = useState<boolean | null>(null);
  useEffect(() => {
    const read = (tier: string) => setPro(tier !== 'free');
    api
      .getLicense()
      .then((l) => read(l.tier))
      .catch(() => {});
    const off = listen<{ tier: string }>('license:changed', (e) => read(e.payload.tier));
    return () => {
      off.then((f) => f()).catch(() => {});
    };
  }, []);
  const [opened, setOpened] = useState(0);
  const seq = useRef(0);
  const loadingMore = useRef(false);
  /** An action pressed before the results for the current query arrived. */
  const queued = useRef<PanelAction | null>(null);
  /** A restore waiting for its result (`items:restored`). */
  const restoring = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Insights (P2.7, the popup's rules): one search per opening, and the time from its
  // first keystroke to a restore picked from the results (sent with the restore).
  const firstKeyAt = useRef<number | null>(null);
  const searchCounted = useRef(false);
  const onQueryChange = (q: string) => {
    if (!q)
      firstKeyAt.current = null; // cleared: the next search starts over
    else if (firstKeyAt.current === null) firstKeyAt.current = performance.now();
    if (q.trim().length >= 2 && !searchCounted.current) {
      searchCounted.current = true;
      api.recordSearch().catch(() => {});
    }
  };
  /** The search time for a restore. The timer stops once a restore reopens something
   * (`items:restored`), so a retry after a failed one still carries it. */
  const takeSearchMs = () => {
    const start = firstKeyAt.current;
    return start === null ? undefined : Math.round(performance.now() - start);
  };

  const wanted = keyOf(query, kind);
  const fresh = results.key === wanted;

  const focusInput = useCallback((selectAll = true) => {
    const input = box.current?.querySelector('input');
    input?.focus();
    if (selectAll) input?.select();
    else input?.setSelectionRange(input.value.length, input.value.length);
  }, []);

  // Opened (hotkey, tray, decay://search): take the prefill, start fresh.
  useEffect(() => {
    const take = () =>
      api
        .takePending()
        .then((p) => {
          setQuery(p.prefill ?? '');
          // A new opening: a new search (a prefill isn't typed, so no timer yet).
          firstKeyAt.current = null;
          searchCounted.current = false;
          setNote(null);
          setSelected(0);
          setResults((r) => ({ ...r, key: '' })); // what's shown is stale until the search answers
          setOpened((n) => n + 1);
          focusInput();
        })
        .catch(() => {});
    take();
    const off = listen('panel:open', take);
    return () => {
      off.then((f) => f()).catch(() => {});
    };
  }, [focusInput]);

  // The result of a restore this panel started: close when it worked, say why when not.
  useEffect(() => {
    const off = listen<{ restored: number; failed: number; skipped: number }>(
      'items:restored',
      (e) => {
        if (!restoring.current) return;
        clearTimeout(restoring.current);
        restoring.current = null;
        const { restored, failed, skipped } = e.payload;
        if (restored > 0) firstKeyAt.current = null; // one sample per search
        if (restored > 0 && failed === 0) {
          setNote(null);
          api.hidePanel();
        } else if (failed > 0) {
          setNote({
            text:
              restored > 0
                ? t('panel.restoredSome', { n: restored, failed })
                : t('panel.couldntReopen', { n: failed }),
            tone: 'error',
          });
        } else if (skipped > 0) {
          setNote({ text: t('panel.already'), tone: 'info' });
        }
      },
    );
    return () => {
      off.then((f) => f()).catch(() => {});
    };
  }, []);

  // Search as you type; a slower, older answer never replaces a newer one.
  // biome-ignore lint/correctness/useExhaustiveDependencies: `opened` re-runs the search on every open
  useEffect(() => {
    const mine = ++seq.current;
    const key = keyOf(query, kind);
    const timer = setTimeout(() => {
      api
        .search({ query, kind: kind === 'all' ? undefined : kind, limit: PAGE })
        .then((out) => {
          if (mine !== seq.current) return;
          setResults({ key, items: out.items, cursor: out.nextCursor });
          setTotal(out.totalArchived);
          setError(null);
          setSelected(0);
        })
        .catch((e) => {
          if (mine !== seq.current) return;
          setResults({ key, items: [], cursor: null });
          setError(errorMessage(e));
        });
    }, DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [query, kind, opened]);

  // The next page, when the list is scrolled near its end.
  const loadMore = () => {
    if (!fresh || !results.cursor || loadingMore.current) return;
    loadingMore.current = true;
    const mine = seq.current;
    api
      .search({
        query,
        kind: kind === 'all' ? undefined : kind,
        limit: PAGE,
        cursor: results.cursor,
      })
      .then((out) => {
        if (mine !== seq.current) return;
        setResults((r) => {
          const seen = new Set(r.items.map((i) => i.id));
          return {
            ...r,
            items: [...r.items, ...out.items.filter((i) => !seen.has(i.id))],
            cursor: out.nextCursor,
          };
        });
      })
      .catch(() => {})
      .finally(() => {
        loadingMore.current = false;
      });
  };

  const locale = getLocale(); // group labels are words: made again in a new language
  const groups = useMemo(
    () => groupResults(results.items, Date.now(), locale),
    [results.items, locale],
  );
  const flat = useMemo(() => groups.flatMap((g) => g.items), [groups]);
  const groupOf = useMemo(() => {
    const m = new Map<string, ResultGroup>();
    for (const g of groups) for (const i of g.items) m.set(i.id, g);
    return m;
  }, [groups]);
  const words = useMemo(() => searchWords(query), [query]);
  const current = flat[selected];
  const now = Date.now();

  // Keep the selected row in view.
  useEffect(() => {
    if (!current) return;
    list.current
      ?.querySelector(`#opt-${CSS.escape(current.id)}`)
      ?.scrollIntoView({ block: 'nearest' });
  }, [current]);

  const startRestore = (call: Promise<unknown>) => {
    setNote({ text: t('panel.restoring'), tone: 'busy' });
    if (restoring.current) clearTimeout(restoring.current);
    restoring.current = setTimeout(() => {
      restoring.current = null;
      setNote({ text: t('panel.noAnswer'), tone: 'error' });
    }, RESTORE_WAIT_MS);
    call.catch((e) => {
      if (restoring.current) clearTimeout(restoring.current);
      restoring.current = null;
      setNote({ text: errorMessage(e), tone: 'error', choices: restoreChoices(e) ?? undefined });
    });
  };

  const act = (action: PanelAction, item = current) => {
    switch (action) {
      case 'next':
        setSelected((i) => Math.min(i + 1, Math.max(flat.length - 1, 0)));
        if (selected >= flat.length - 5) loadMore();
        return;
      case 'previous':
        setSelected((i) => Math.max(i - 1, 0));
        return;
      case 'first':
        setSelected(0);
        return;
      case 'last':
        setSelected(Math.max(flat.length - 1, 0));
        return;
    }
    if (!item) return;
    switch (action) {
      case 'restore':
        startRestore(api.restoreItems([item.id], 'original', takeSearchMs()));
        return;
      case 'restoreNewWindow':
        startRestore(api.restoreItems([item.id], 'newWindow', takeSearchMs()));
        return;
      case 'restoreGroup': {
        // Exactly what the group shows (the engine puts each window's tabs in order).
        const group = groupOf.get(item.id)?.items ?? [item];
        if (group.length > 1 && pro === false) {
          setNote({ text: t('panel.batchPro'), tone: 'info', pro: true });
          return;
        }
        startRestore(
          api.restoreItems(
            group.map((i) => i.id),
            'original',
            takeSearchMs(),
          ),
        );
        return;
      }
      case 'protect':
        api.protectItem(item.id).then(
          (rule) => setNote({ text: t('panel.keeping', { match: rule.match }), tone: 'info' }),
          (e) => setNote({ text: errorMessage(e), tone: 'error' }),
        );
        return;
      case 'details':
        api.showItem(item.id).catch((e) => setNote({ text: errorMessage(e), tone: 'error' }));
        return;
    }
  };

  // An action pressed before the current results arrived runs on them, not the old ones.
  // biome-ignore lint/correctness/useExhaustiveDependencies: runs when fresh results land
  useEffect(() => {
    if (!fresh || !queued.current) return;
    const action = queued.current;
    queued.current = null;
    act(action, flat[0]);
  }, [fresh, flat]);

  const emptyTitle = t(`panel.none.${kind}`);

  return (
    <div className="panel-frame h-screen p-4">
      <main
        ref={box}
        className="panel-card flex h-full flex-col overflow-hidden rounded-card border border-line bg-surface shadow-panel"
      >
        <h1 className="sr-only">{t('panel.title')}</h1>
        <div className="flex items-center gap-3 border-b border-line px-2 py-3 pe-4 focus-within:border-accent">
          <SearchInput
            large
            bare
            className="flex-1"
            label={t('panel.title')}
            placeholder={t('panel.placeholder')}
            value={query}
            onValueChange={(v) => {
              setQuery(v);
              setNote(null);
              onQueryChange(v);
            }}
            onEscape={() => api.hidePanel()}
            role="combobox"
            aria-expanded={flat.length > 0}
            aria-controls="results"
            aria-autocomplete="list"
            aria-activedescendant={current ? `opt-${current.id}` : undefined}
            onKeyDown={(e) => {
              const action = panelAction(
                {
                  key: e.key,
                  metaKey: e.metaKey,
                  ctrlKey: e.ctrlKey,
                  altKey: e.altKey,
                  shiftKey: e.shiftKey,
                  isComposing: e.nativeEvent.isComposing,
                },
                MAC,
              );
              if (!action) return;
              e.preventDefault();
              const moves = ['next', 'previous', 'first', 'last'].includes(action);
              if (!moves && !fresh) {
                queued.current = action; // runs on the results for what's typed now
                return;
              }
              act(action);
            }}
          />
          <fieldset
            className="flex shrink-0 items-center gap-1 border-0 p-0"
            aria-label={t('panel.show')}
          >
            {KINDS.map((k) => (
              <button
                key={k}
                type="button"
                aria-pressed={kind === k}
                onClick={() => {
                  setKind(k);
                  focusInput(false);
                }}
                className={cx(
                  'rounded-full px-3 py-1 text-xs transition-colors',
                  kind === k
                    ? 'bg-primary text-primary-ink'
                    : 'bg-surface-2 text-muted hover:text-ink',
                )}
              >
                {t(`panel.kind.${k}`)}
              </button>
            ))}
            <button
              type="button"
              title={t('panel.siteHint')}
              onClick={() => {
                if (!/\bsite:/.test(query)) setQuery((q) => `${q.trim()} site:`.trimStart());
                focusInput(false);
              }}
              className="rounded-full border border-line px-3 py-1 font-mono text-xs text-muted hover:text-ink"
            >
              {'site:' /* i18n-ignore: the search operator */}
            </button>
          </fieldset>
        </div>

        <div
          ref={list}
          id="results"
          role="listbox"
          aria-label={t('panel.results')}
          aria-busy={!fresh}
          onScroll={(e) => {
            const el = e.currentTarget;
            if (el.scrollTop + el.clientHeight > el.scrollHeight - 200) loadMore();
          }}
          className="min-h-0 flex-1 overflow-y-auto px-2 py-2"
        >
          {error ? (
            <EmptyState icon={<CircleAlert size={20} />} title={t('panel.broken')}>
              {error}
            </EmptyState>
          ) : flat.length === 0 && fresh ? (
            <EmptyState
              icon={<Archive size={20} />}
              title={query.trim() ? t('panel.nothingFound') : emptyTitle}
            >
              {query.trim() ? t('panel.tryFewer') : t('panel.emptyText')}
            </EmptyState>
          ) : (
            groups.map((g, gi) => (
              // biome-ignore lint/a11y/useSemanticElements: a group of options inside the listbox (ARIA listbox pattern)
              <div
                key={g.key}
                role="group"
                aria-label={g.label}
                className={cx(gi > 0 && 'mt-2 border-t border-line pt-2')}
              >
                {/* The visible label repeats the group's name; "Restore all" repeats ⌘↵.
                    Both are for the eye and the mouse, so they stay out of the listbox's tree. */}
                <div aria-hidden className="flex items-center justify-between px-3 py-1.5">
                  <span className="flex items-center gap-2 text-[11px] font-semibold tracking-wider text-muted uppercase">
                    {gi === 0 ? <Clock size={13} /> : <History size={13} />}
                    {g.label}
                  </span>
                  {g.items.length > 1 && g.items[0] && (
                    <button
                      type="button"
                      tabIndex={-1}
                      onClick={() => act('restoreGroup', g.items[0])}
                      className="flex items-center gap-1 rounded-full border border-line bg-surface-2 px-2.5 py-0.5 text-[11px] text-muted hover:text-ink"
                    >
                      {t('panel.restoreAll')} <span className="font-[system-ui]">{MOD}↵</span>
                    </button>
                  )}
                </div>
                {g.items.map((item) => {
                  const index = flat.indexOf(item);
                  const isSelected = index === selected;
                  const m = monogram(item);
                  return (
                    // biome-ignore lint/a11y/useFocusableInteractive: options are reached with the arrow keys from the combobox (aria-activedescendant)
                    // biome-ignore lint/a11y/useKeyWithClickEvents: the combobox handles the keys for every option
                    <div
                      key={item.id}
                      id={`opt-${item.id}`}
                      role="option"
                      aria-selected={isSelected}
                      onMouseMove={() => index !== selected && setSelected(index)}
                      onClick={() => act('restore', item)}
                      className={cx(
                        'relative flex cursor-default items-center gap-3 rounded-xl px-3 py-2.5',
                        isSelected && 'bg-accent-soft',
                      )}
                    >
                      {isSelected && (
                        <span
                          aria-hidden
                          className="absolute top-1.5 bottom-1.5 start-0 w-0.5 rounded-full bg-accent"
                        />
                      )}
                      <span
                        aria-hidden
                        className={cx(
                          'flex size-8 shrink-0 items-center justify-center rounded-lg text-sm font-semibold',
                          TINT[m.tint],
                        )}
                      >
                        {m.letter}
                      </span>
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-sm text-ink">
                          {highlight(item.title || item.url || t('panel.untitled'), words).map(
                            (part, i) =>
                              part.match ? (
                                // biome-ignore lint/suspicious/noArrayIndexKey: parts of one fixed string
                                <mark key={i} className="rounded bg-lime px-1 text-lime-ink">
                                  {part.text}
                                </mark>
                              ) : (
                                // biome-ignore lint/suspicious/noArrayIndexKey: parts of one fixed string
                                <Fragment key={i}>{part.text}</Fragment>
                              ),
                          )}
                        </span>
                        <span className="block truncate font-mono text-xs text-muted">
                          {displayUrl(item.url) || item.appName}
                        </span>
                      </span>
                      {item.reason && (
                        <Chip
                          className="border border-line bg-surface text-muted"
                          icon={<span className="block size-1.5 rounded-full bg-accent-dot" />}
                        >
                          {item.reason}
                        </Chip>
                      )}
                      {item.archivedAt !== null && (
                        <span className="w-28 shrink-0 text-end text-xs text-muted">
                          {timeLabel(item.archivedAt, now)}
                        </span>
                      )}
                    </div>
                  );
                })}
              </div>
            ))
          )}
        </div>

        <footer className="flex items-center gap-4 border-t border-line px-4 py-2.5 text-xs text-muted">
          {note ? (
            <div className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
              <p
                role={note.tone === 'error' ? 'alert' : 'status'}
                className={cx(
                  'min-w-0 flex-1 truncate',
                  note.choices && 'basis-full',
                  note.tone === 'error' && 'text-danger',
                )}
              >
                {note.text}
              </p>
              {note.pro && (
                <button
                  type="button"
                  className="shrink-0 rounded-full border border-line px-2.5 py-1 text-ink hover:bg-surface-2"
                  onClick={() => {
                    api.showWindow('settings', '/license').catch(() => {});
                  }}
                >
                  {t('settings.seePro')}
                </button>
              )}
              {note.choices?.alternatives.map((c) => (
                <button
                  key={`${c.appId}|${c.profileId}`}
                  type="button"
                  className="max-w-full truncate rounded-full border border-line px-2.5 py-1 text-ink hover:bg-surface-2"
                  onClick={() => {
                    const ids = note.choices?.itemIds ?? [];
                    startRestore(api.restoreItemsIn(ids, c.appId, c.profileId));
                  }}
                >
                  {t('restore.openIn', { browser: c.label })}
                </button>
              ))}
            </div>
          ) : (
            <ul className="flex flex-1 items-center gap-3" aria-label={t('panel.keys')}>
              <li className="flex items-center gap-1.5">
                <Kbd>↵</Kbd> {t('panel.key.restore')}
              </li>
              <li className="flex items-center gap-1.5">
                <Kbd>{`${MOD}↵`}</Kbd> {t('panel.key.group')}
              </li>
              <li className="flex items-center gap-1.5">
                <Kbd>{`${MOD}P`}</Kbd> {t('panel.key.protect')}
              </li>
              <li className="flex items-center gap-1.5">
                <Kbd>{`${MOD}I`}</Kbd> {t('panel.key.details')}
              </li>
              <li className="flex items-center gap-1.5">
                <Kbd>{'esc' /* i18n-ignore: a key cap */}</Kbd> {t('panel.key.close')}
              </li>
            </ul>
          )}
          <span className="flex shrink-0 items-center gap-1.5">
            <span aria-hidden className="size-1.5 rounded-full bg-accent-dot" />
            {t(results.cursor ? 'panel.countMore' : 'panel.count', { shown: flat.length, total })}
          </span>
        </footer>
      </main>
    </div>
  );
}
