import {
  AudioLines,
  CircleAlert,
  Eye,
  Hand,
  Info,
  Leaf,
  Link2Off,
  Lock,
  PauseCircle,
  Pin,
  Search,
  ShieldCheck,
  SquarePen,
  Undo2,
  Zap,
} from 'lucide-react';
import { type KeyboardEvent, type ReactNode, useId, useMemo, useRef, useState } from 'react';
import type { Mode } from '@/lib/db/types';
import { protectTargetFor } from '@/lib/shadow/actions';
import type { KeptGroup, ReportItem, ShadowReport } from '@/lib/shadow/report';
import {
  activeHours,
  KEPT_GROUP_LABEL,
  minutesUntil,
  PROTECTED_LABEL,
  percent,
  shortUrl,
} from './format';

export interface ShadowReviewProps {
  report: ShadowReport;
  now: number;
  liveTabs: number;
  /** False until real sweeps exist (P0.5): automatic mode isn't offered yet. */
  automaticAvailable: boolean;
  faviconFor?: (url: string) => string | undefined;
  onTurnOnAutomatic: () => void;
  onBackToShadow: () => void;
  onKeepWatching: () => void;
  onProtect: (entry: ReportItem) => void;
  busyItemId?: string | null;
  modeBusy?: boolean;
  error?: string | null;
}

const PREVIEW_COUNT = 5;
type TabKey = 'archive' | 'kept';
const TABS: TabKey[] = ['archive', 'kept'];

const GROUP_ICON: Record<KeptGroup, ReactNode> = {
  pinned: <Pin size={16} aria-hidden />,
  audio: <AudioLines size={16} aria-hidden />,
  active: <Eye size={16} aria-hidden />,
  form: <SquarePen size={16} aria-hidden />,
  rules: <ShieldCheck size={16} aria-hidden />,
  hold: <PauseCircle size={16} aria-hidden />,
  other: <Link2Off size={16} aria-hidden />,
};

const MODE_LABEL: Record<Mode, string> = {
  shadow: 'Shadow mode',
  automatic: 'Automatic',
  paused: 'Paused',
};

const AVATARS = [
  'bg-avatar-1 text-avatar-1-ink',
  'bg-avatar-2 text-avatar-2-ink',
  'bg-avatar-3 text-avatar-3-ink',
  'bg-avatar-4 text-avatar-4-ink',
];

function avatarClass(key: string): string {
  let hash = 0;
  for (const ch of key) hash = (hash * 31 + ch.charCodeAt(0)) >>> 0;
  return AVATARS[hash % AVATARS.length] ?? AVATARS[0] ?? '';
}

export function ShadowReview(props: ShadowReviewProps) {
  const { report } = props;
  const [tab, setTab] = useState<TabKey>('archive');
  const [query, setQuery] = useState('');
  const [showAll, setShowAll] = useState(false);
  const baseId = useId();
  const tabRefs = useRef<Record<TabKey, HTMLButtonElement | null>>({ archive: null, kept: null });

  const list = tab === 'archive' ? report.wouldArchive : report.kept;
  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return list;
    return list.filter(
      ({ item }) =>
        item.title.toLowerCase().includes(q) || (item.url ?? '').toLowerCase().includes(q),
    );
  }, [list, query]);
  const visible = showAll ? filtered : filtered.slice(0, PREVIEW_COUNT);
  const nextMin = minutesUntil(report.nextSimulationAt, props.now);

  const select = (key: TabKey) => {
    setTab(key);
    setShowAll(false);
  };
  // WAI-ARIA tabs: arrow keys move between tabs (automatic activation).
  const onTabKeyDown = (e: KeyboardEvent<HTMLButtonElement>) => {
    const i = TABS.indexOf(tab);
    let next: TabKey | undefined;
    if (e.key === 'ArrowRight') next = TABS[(i + 1) % TABS.length];
    else if (e.key === 'ArrowLeft') next = TABS[(i - 1 + TABS.length) % TABS.length];
    else if (e.key === 'Home') next = TABS[0];
    else if (e.key === 'End') next = TABS[TABS.length - 1];
    if (!next) return;
    e.preventDefault();
    select(next);
    tabRefs.current[next]?.focus();
  };
  const tabId = (key: TabKey) => `${baseId}-tab-${key}`;
  const panelId = `${baseId}-panel`;

  return (
    <div className="min-h-screen bg-canvas text-ink">
      <header className="flex items-center justify-between border-b border-line px-4 py-3 sm:px-10">
        <div className="flex items-center gap-2">
          <Leaf size={20} className="text-accent" aria-hidden />
          <span className="font-serif text-xl">Decay</span>
        </div>
        <span className="inline-flex items-center gap-2 rounded-full border border-line bg-surface px-3 py-1 text-sm">
          <span className="size-2 rounded-full bg-accent-dot" aria-hidden />
          {MODE_LABEL[report.mode]}
        </span>
      </header>

      <main className="mx-auto max-w-6xl px-4 py-8 sm:px-10">
        <h1 className="font-serif text-4xl tracking-tight sm:text-5xl">Shadow review</h1>
        <p className="mt-2 text-muted">
          {report.mode === 'shadow'
            ? "Here's what Decay would have done. Nothing has been closed."
            : 'What Decay would have archived during shadow mode.'}
        </p>

        <Hero {...props} />

        {props.error && (
          <p
            role="alert"
            className="mt-4 flex items-center gap-2 rounded-tile bg-surface-2 px-4 py-3 text-sm text-danger"
          >
            <CircleAlert size={16} aria-hidden /> {props.error}
          </p>
        )}

        <div className="mt-6 grid grid-cols-[minmax(0,1fr)] gap-6 lg:grid-cols-[minmax(0,1fr)_340px]">
          <section
            aria-label="Simulated decisions"
            className="min-w-0 rounded-card border border-line bg-surface p-4 shadow-[var(--shadow-card)] sm:p-6"
          >
            <div className="flex flex-col gap-3 border-b border-line pb-4 sm:flex-row sm:items-center sm:justify-between">
              <div
                role="tablist"
                aria-label="Decisions"
                className="inline-flex self-start rounded-full bg-surface-2 p-1"
              >
                {TABS.map((key) => (
                  <button
                    key={key}
                    ref={(el) => {
                      tabRefs.current[key] = el;
                    }}
                    id={tabId(key)}
                    type="button"
                    role="tab"
                    aria-selected={tab === key}
                    aria-controls={panelId}
                    tabIndex={tab === key ? 0 : -1}
                    onClick={() => select(key)}
                    onKeyDown={onTabKeyDown}
                    className={`rounded-full px-4 py-1.5 text-sm ${
                      tab === key ? 'bg-primary text-primary-ink' : 'text-muted hover:text-ink'
                    }`}
                  >
                    {key === 'archive'
                      ? `Would archive (${report.wouldArchive.length})`
                      : `Kept (${report.kept.length})`}
                  </button>
                ))}
              </div>
              <label className="relative block sm:w-72">
                <span className="sr-only">Filter tabs</span>
                <Search
                  size={16}
                  className="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-faint"
                  aria-hidden
                />
                <input
                  type="search"
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  placeholder="Filter simulated tabs…"
                  className="w-full rounded-full border border-line bg-canvas py-2 pr-4 pl-9 text-sm placeholder:text-faint"
                />
              </label>
            </div>

            <div id={panelId} role="tabpanel" aria-labelledby={tabId(tab)}>
              {visible.length === 0 ? (
                <EmptyList tab={tab} filtered={query.trim().length > 0} />
              ) : (
                <ul className="divide-y divide-line">
                  {visible.map((entry) => (
                    <Row
                      key={entry.item.id}
                      entry={entry}
                      favicon={
                        entry.item.url && entry.item.faviconUrl
                          ? props.faviconFor?.(entry.item.url)
                          : undefined
                      }
                      onProtect={props.onProtect}
                      busy={props.busyItemId === entry.item.id}
                    />
                  ))}
                </ul>
              )}

              {filtered.length > 0 && (
                <div className="flex items-center justify-between pt-4 text-sm text-muted">
                  <span>
                    Showing {visible.length} of {filtered.length}
                  </span>
                  {filtered.length > PREVIEW_COUNT && (
                    <button
                      type="button"
                      onClick={() => setShowAll((v) => !v)}
                      className="text-accent-text underline underline-offset-4"
                    >
                      {showAll ? 'Show fewer' : `View all ${filtered.length}`}
                    </button>
                  )}
                </div>
              )}
            </div>
          </section>

          <aside className="min-w-0 rounded-card border border-line bg-surface p-6 shadow-[var(--shadow-card)]">
            <h2 className="font-serif text-2xl">Why things were kept</h2>
            <p className="mt-2 text-sm text-muted">
              Decay leaves a tab alone when one of your safety signals matches, even when it's old
              enough to archive.
            </p>
            <ul className="mt-5 space-y-3">
              {(Object.keys(KEPT_GROUP_LABEL) as KeptGroup[])
                .filter((g) => g !== 'hold' || report.keptByGroup.hold > 0)
                .filter((g) => g !== 'other' || report.keptByGroup.other > 0)
                .map((group) => (
                  <li
                    key={group}
                    className="flex items-center gap-3 rounded-tile border border-line bg-surface-2 px-3 py-3"
                  >
                    <span className="grid size-8 place-items-center rounded-full bg-surface-3 text-muted">
                      {GROUP_ICON[group]}
                    </span>
                    <span className="flex-1 text-sm">{KEPT_GROUP_LABEL[group]}</span>
                    <span className="grid min-w-7 place-items-center rounded-full border border-line-strong px-2 text-sm tabular-nums">
                      {report.keptByGroup[group]}
                    </span>
                  </li>
                ))}
            </ul>
            <p className="mt-5 flex gap-3 rounded-tile bg-surface-2 p-4 text-sm text-muted">
              <Info size={18} className="mt-0.5 shrink-0 text-accent" aria-hidden />
              {asideNote(report.mode, report.targetDays)}
            </p>
            <p className="mt-5 flex items-center gap-2 border-t border-line pt-4 text-sm text-muted">
              <Lock size={14} aria-hidden /> Local processing only
            </p>
          </aside>
        </div>
      </main>

      <footer className="border-t border-line px-4 py-3 text-sm text-muted sm:px-10">
        <div className="mx-auto flex max-w-6xl flex-wrap justify-between gap-2">
          <span className="inline-flex items-center gap-2">
            <span className="size-2 rounded-full bg-accent-dot" aria-hidden />
            {props.liveTabs} open tabs tracked
          </span>
          {nextMin !== null && (
            <span className="text-accent-text">
              {nextMin === 0
                ? 'Next simulation at the next check'
                : `Next simulation in ${nextMin}m`}
            </span>
          )}
        </div>
      </footer>
    </div>
  );
}

function asideNote(mode: Mode, days: number): string {
  if (mode === 'shadow') {
    return `Shadow mode keeps running until day ${days}. Nothing is closed until you turn on automatic mode.`;
  }
  if (mode === 'paused')
    return 'Decay is paused. Nothing is simulated or archived until you resume.';
  return 'Automatic mode is on. Archived tabs can always be restored.';
}

function Hero(props: ShadowReviewProps) {
  const { report } = props;
  const [confirming, setConfirming] = useState(false);
  const regret = percent(report.regretRate);
  const flagged = report.flaggedTotal;
  const tabs = `${flagged} ${flagged === 1 ? 'tab' : 'tabs'}`;

  let title: string;
  let summary: string;
  if (report.mode === 'automatic') {
    title = 'Automatic mode is on';
    summary = `During shadow mode, Decay would have archived ${tabs}.`;
  } else if (report.mode === 'paused') {
    title = 'Decay is paused';
    summary = `Nothing is simulated while paused. So far Decay would have archived ${tabs}.`;
  } else {
    title = report.ready ? "You're ready for automatic mode" : 'Decay is watching quietly';
    summary = `Day ${report.dayNumber} of ${report.targetDays} · Decay would have archived ${tabs}.`;
  }
  if (flagged > 0) summary += ` You went back to ${report.regretTotal} of them (${regret}).`;

  return (
    <section
      aria-label="Shadow mode summary"
      className="mt-8 rounded-card bg-feature p-6 text-feature-ink sm:p-8"
    >
      <div className="flex flex-col gap-6 lg:flex-row lg:items-start lg:justify-between">
        <div className="min-w-0 flex-1">
          <h2 className="font-serif text-3xl">{title}</h2>
          <p className="mt-2 text-feature-sub">{summary}</p>
          <div className="mt-6 grid gap-3 sm:grid-cols-3">
            <Stat value={tabs} label="simulated archive" />
            <Stat
              value={`${Math.min(report.activeDays, report.targetDays)} of ${report.targetDays}`}
              label="active days (≥ 1 h)"
            />
            <Stat value={`${regret} regret`} label="used again within 24 h" />
          </div>
        </div>
        <div className="flex flex-col gap-3 lg:w-64">
          {report.mode !== 'shadow' ? (
            <button
              type="button"
              onClick={props.onBackToShadow}
              disabled={props.modeBusy}
              className="inline-flex items-center justify-center gap-2 rounded-full bg-cta px-5 py-3 font-medium text-cta-ink disabled:opacity-60"
            >
              <Undo2 size={16} aria-hidden /> Back to shadow mode
            </button>
          ) : !props.automaticAvailable ? (
            <p className="rounded-tile border border-feature-tile-line px-5 py-3 text-sm">
              <Eye size={16} className="mb-1 inline" aria-hidden /> Watching · nothing is closed.
              Automatic mode arrives in the next update.
            </p>
          ) : confirming ? (
            <div className="rounded-tile border border-feature-tile-line p-4 text-sm">
              <p>
                {report.ready
                  ? 'Decay will start archiving stale tabs. You can undo every sweep.'
                  : `This ends shadow mode before day ${report.targetDays}. Decay will start archiving stale tabs.`}
              </p>
              <div className="mt-3 flex gap-2">
                <button
                  type="button"
                  onClick={props.onTurnOnAutomatic}
                  disabled={props.modeBusy}
                  className="rounded-full bg-cta px-4 py-2 font-medium text-cta-ink disabled:opacity-60"
                >
                  Turn on
                </button>
                <button
                  type="button"
                  onClick={() => setConfirming(false)}
                  className="rounded-full border border-feature-tile-line px-4 py-2"
                >
                  Cancel
                </button>
              </div>
            </div>
          ) : report.ready ? (
            <>
              <button
                type="button"
                onClick={() => setConfirming(true)}
                className="inline-flex items-center justify-center gap-2 rounded-full bg-cta px-5 py-3 font-medium text-cta-ink"
              >
                <Zap size={16} aria-hidden /> Turn on automatic mode
              </button>
              <button
                type="button"
                onClick={props.onKeepWatching}
                className="inline-flex items-center justify-center gap-2 rounded-full border border-feature-tile-line px-5 py-3 font-medium"
              >
                <Eye size={16} aria-hidden /> Keep watching
              </button>
            </>
          ) : (
            <>
              <p className="inline-flex items-center justify-center gap-2 rounded-full border border-feature-tile-line px-5 py-3 text-sm">
                <Eye size={16} aria-hidden /> Watching · nothing is closed
              </p>
              <button
                type="button"
                onClick={() => setConfirming(true)}
                className="text-sm text-feature-sub underline underline-offset-4"
              >
                Turn on automatic mode early
              </button>
            </>
          )}
        </div>
      </div>
    </section>
  );
}

function Stat({ value, label }: { value: string; label: string }) {
  return (
    <div className="rounded-tile border border-feature-tile-line bg-feature-tile px-5 py-4">
      <div className="font-serif text-3xl">{value}</div>
      <div className="mt-1 text-sm text-feature-sub">{label}</div>
    </div>
  );
}

function Row(props: {
  entry: ReportItem;
  favicon?: string;
  onProtect: (entry: ReportItem) => void;
  busy: boolean;
}) {
  const { item, decision } = props.entry;
  const title = item.title || shortUrl(item.url) || 'Untitled';
  const key = (item.domain ?? title).replace(/^www\./, '');
  const initial = key.charAt(0).toUpperCase() || '·';
  const target = protectTargetFor(item);
  const ruleReason = decision.action === 'archive' && decision.reason.type === 'customRule';
  return (
    <li className="flex items-start gap-3 py-4 sm:items-center">
      <span
        className={`grid size-10 shrink-0 place-items-center overflow-hidden rounded-xl font-medium ${
          props.favicon ? 'border border-line bg-surface-2' : avatarClass(key)
        }`}
      >
        {props.favicon ? (
          <img src={props.favicon} alt="" className="size-5" />
        ) : (
          <span aria-hidden>{initial}</span>
        )}
      </span>
      <div className="flex min-w-0 flex-1 flex-col gap-2 sm:flex-row sm:items-center sm:gap-3">
        <div className="min-w-0 flex-1">
          <p className="truncate">{title}</p>
          <p className="mt-1 flex min-w-0 flex-wrap items-center gap-2 text-sm">
            <span className="max-w-full truncate font-mono text-muted">{shortUrl(item.url)}</span>
            <span className="inline-flex items-center gap-1.5 rounded-full bg-chip px-2 py-0.5 text-xs text-chip-ink">
              <span className="size-1.5 rounded-full bg-accent-dot" aria-hidden />
              Unused {activeHours(item.activeAgeS)} active h{ruleReason ? ' · your rule' : ''}
            </span>
          </p>
        </div>
        {decision.action === 'protect' ? (
          <span className="inline-flex items-center gap-2 self-start rounded-full bg-lime px-3 py-1.5 text-sm text-lime-ink sm:self-auto">
            <ShieldCheck size={14} aria-hidden /> Protected: {PROTECTED_LABEL[decision.reason.by]}
          </span>
        ) : (
          <button
            type="button"
            disabled={props.busy}
            onClick={() => props.onProtect(props.entry)}
            className="inline-flex items-center gap-2 self-start rounded-full border border-line-strong px-3 py-1.5 text-sm hover:bg-surface-2 disabled:opacity-50 sm:self-auto"
          >
            <Hand size={14} aria-hidden />
            {props.busy ? 'Protecting…' : `Always keep ${target.kind}`}
            <span className="sr-only">: {target.label}</span>
          </button>
        )}
      </div>
    </li>
  );
}

function EmptyList({ tab, filtered }: { tab: TabKey; filtered: boolean }) {
  const text = filtered
    ? 'No tabs match this filter.'
    : tab === 'archive'
      ? 'Nothing would be archived right now. Decay is watching quietly.'
      : 'No old tabs needed protecting.';
  return <p className="py-12 text-center text-muted">{text}</p>;
}
