import {
  Archive,
  ArrowDown,
  ArrowUp,
  Check,
  Clock,
  Download,
  History,
  Lock,
  Minus,
  PanelTop,
  Share2,
  Sparkles,
  X,
} from 'lucide-react';
import { type ReactNode, useEffect, useId, useRef, useState } from 'react';
import { type Goal, type Insights, RANGES, type RangeDays } from './insights';
import type { Mode } from './types';

function percent(rate: number | null): string {
  if (rate === null) return '—';
  if (rate > 0 && rate < 0.01) return '<1%';
  return `${Math.round(rate * 100)}%`;
}

/** What differs between the extension's page and the app's (where it's computed). */
export interface InsightsCopy {
  intro: string;
  privacy: string;
  shareWhat: string;
}

export const BROWSER_COPY: InsightsCopy = {
  intro:
    'How your browsing has settled since you started using Decay. Computed in this browser only.',
  privacy: 'Computed in this browser. Nothing leaves it unless you share.',
  shareWhat: 'and the extension and browser version',
};

export interface InsightsProps {
  insights: Insights;
  mode: Mode;
  onRangeChange: (range: RangeDays) => void;
  onExportCsv: () => void;
  /** Opens the share preview (the JSON that would be downloaded). */
  sharePreview: string | null;
  onShare: () => void;
  onShareDownload: () => void;
  onShareCancel: () => void;
  error?: string | null;
  /** Above the page (the extension's page header); none in the app. */
  header?: ReactNode;
  /** Defaults to the extension's wording. */
  copy?: InsightsCopy;
  /** Inside another page (the app's Library): no full-page background or margins. */
  embedded?: boolean;
}

const fmt = (n: number) => n.toLocaleString();
const shortDay = (day: string) =>
  new Date(`${day}T12:00:00`).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });

/** Insights (Designs/14, adapted to what the extension measures): on-device metrics. */
export function InsightsView(props: InsightsProps) {
  const { insights: data } = props;
  const mostTidied = data.topArchived[0];
  const regret = data.regretRate ?? data.shadowRegretRate;
  const copy = props.copy ?? BROWSER_COPY;
  return (
    <div className={props.embedded ? 'text-ink' : 'min-h-screen bg-canvas text-ink'}>
      {/* Inert while the share dialog is open: focus stays in the dialog. */}
      <div inert={props.sharePreview !== null}>
        {props.header}

        <main className={props.embedded ? '' : 'mx-auto max-w-6xl px-4 py-8 sm:px-10'}>
          <h1 className="font-serif text-4xl tracking-tight sm:text-5xl">Insights</h1>
          <p className="mt-2 text-muted">{copy.intro}</p>

          {/* Filters scope everything below them: one row, above the content. */}
          <div className="mt-6 flex flex-wrap items-center gap-3">
            <fieldset className="inline-flex rounded-full bg-surface-2 p-1">
              <legend className="sr-only">Time range</legend>
              {RANGES.map((r) => (
                <label
                  key={r}
                  className={`cursor-pointer rounded-full px-4 py-1.5 text-sm has-[:focus-visible]:outline-2 has-[:focus-visible]:outline-accent ${
                    data.range === r ? 'bg-primary text-primary-ink' : 'text-muted hover:text-ink'
                  }`}
                >
                  <input
                    type="radio"
                    name="range"
                    value={r}
                    checked={data.range === r}
                    onChange={() => props.onRangeChange(r)}
                    className="sr-only"
                  />
                  Last {r} days
                </label>
              ))}
            </fieldset>
            <div className="flex gap-2 sm:ml-auto">
              <button
                type="button"
                onClick={props.onExportCsv}
                className="inline-flex items-center gap-2 rounded-full border border-line-strong bg-cta px-4 py-1.5 text-sm font-medium text-cta-ink hover:brightness-95"
              >
                <Download size={16} aria-hidden /> Export CSV (all days)
              </button>
              <button
                type="button"
                onClick={props.onShare}
                className="inline-flex items-center gap-2 rounded-full bg-primary px-4 py-1.5 text-sm font-medium text-primary-ink hover:opacity-90"
              >
                <Share2 size={16} aria-hidden /> Share with beta team
              </button>
            </div>
          </div>

          {props.error && (
            <p
              role="alert"
              className="mt-4 rounded-tile bg-surface-2 px-4 py-3 text-sm text-danger"
            >
              {props.error}
            </p>
          )}

          <section aria-label="Summary" className="mt-6 grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
            <Tile label="Median open tabs" icon={<PanelTop size={18} aria-hidden />}>
              <Value>{data.openTabs === null ? '—' : fmt(data.openTabs)}</Value>
              {data.openTabsChange !== null && data.firstWeekOpenTabs !== null ? (
                <p className="mt-2 flex items-center gap-2 text-sm text-muted">
                  <span className="inline-flex items-center gap-1 rounded-full bg-chip px-2 py-0.5 text-chip-ink">
                    {data.openTabsChange < 0 ? (
                      <ArrowDown size={12} aria-hidden />
                    ) : data.openTabsChange > 0 ? (
                      <ArrowUp size={12} aria-hidden />
                    ) : null}
                    {percent(Math.abs(data.openTabsChange))}
                    <span className="sr-only">
                      {data.openTabsChange < 0
                        ? 'fewer'
                        : data.openTabsChange > 0
                          ? 'more'
                          : 'no change'}
                    </span>
                  </span>
                  last 7 days vs {fmt(data.firstWeekOpenTabs)} in week 1
                </p>
              ) : (
                <Sub>
                  {data.openTabs === null
                    ? 'Counted every hour while you browse'
                    : 'Compared with week 1 after 14 days'}
                </Sub>
              )}
            </Tile>
            <Tile label="Regret rate" icon={<History size={18} aria-hidden />}>
              <Value>{regret === null ? '—' : percent(regret)}</Value>
              <Sub>
                {data.regretRate !== null
                  ? `${fmt(data.restored24h)} restored within a day`
                  : data.shadowRegretRate !== null
                    ? 'Shadow mode: used again within a day'
                    : 'Nothing archived yet'}
              </Sub>
            </Tile>
            <Tile label="Items archived" icon={<Archive size={18} aria-hidden />}>
              <Value>{fmt(data.archived)}</Value>
              <Sub>
                {data.undos
                  ? `${fmt(data.undos)} sweep${data.undos === 1 ? '' : 's'} undone`
                  : `In the last ${data.range} days`}
              </Sub>
            </Tile>
            <Tile label="Most tidied site" icon={<Sparkles size={18} aria-hidden />}>
              <p className="mt-3 truncate text-2xl font-medium" title={mostTidied?.domain}>
                {mostTidied?.domain ?? '—'}
              </p>
              <Sub>
                {mostTidied
                  ? `${fmt(mostTidied.count)} tab${mostTidied.count === 1 ? '' : 's'} quietly archived`
                  : 'Nothing archived yet'}
              </Sub>
            </Tile>
          </section>

          {/* Wide screens: chart and site lists on the left, goals beside both (no gap
              under the goals card). Narrow: chart, goals, lists. */}
          <div className="mt-6 grid grid-cols-[minmax(0,1fr)] gap-6 lg:grid-cols-[minmax(0,1fr)_360px]">
            {/* Keyed by range: the roving focus index must fit the new number of days. */}
            <OpenTabsChart key={data.range} insights={data} />
            <Goals goals={data.goals} privacy={copy.privacy} />
            <div className="grid gap-6 sm:grid-cols-2 lg:col-start-1 lg:row-start-2">
              <DomainList
                title="Archived most"
                subtitle={`Sites Decay put away in the last ${data.range} days`}
                rows={data.topArchived}
                unit="tab"
                empty="Nothing archived in this range."
              />
              <DomainList
                title="Restored most often"
                subtitle="All time, including Undo — consider “Never archive this site”"
                rows={data.restoredMost}
                unit="restore"
                empty="You haven't restored anything yet."
              />
            </div>
          </div>
        </main>
      </div>

      {props.sharePreview !== null && (
        <ShareDialog
          json={props.sharePreview}
          onDownload={props.onShareDownload}
          onCancel={props.onShareCancel}
          shareWhat={copy.shareWhat}
        />
      )}
    </div>
  );
}

function Tile(props: { label: string; icon: ReactNode; children: ReactNode }) {
  return (
    <div className="rounded-card border border-line bg-surface p-5 shadow-[var(--shadow-card)]">
      <div className="flex items-center justify-between text-muted">
        <h2 className="text-xs font-medium tracking-widest uppercase">{props.label}</h2>
        {props.icon}
      </div>
      {props.children}
    </div>
  );
}

const Value = ({ children }: { children: ReactNode }) => (
  <p className="mt-2 font-serif text-5xl tracking-tight">{children}</p>
);
const Sub = ({ children }: { children: ReactNode }) => (
  <p className="mt-2 text-sm text-muted">{children}</p>
);

/** Clean axis ticks: 0, a whole-number middle, a round top at or above the max. */
function niceMax(max: number): number {
  if (max <= 10) return Math.max(2, Math.ceil(max / 2) * 2);
  const step = 10 ** Math.floor(Math.log10(max));
  for (const m of [1, 2, 5, 10]) if (m * step >= max) return m * step;
  return 10 * step;
}

function OpenTabsChart({ insights }: { insights: Insights }) {
  const [hover, setHover] = useState<number | null>(null);
  // One tab stop for the whole chart; arrow keys move between days (roving tabindex).
  const [focusIdx, setFocusIdx] = useState(insights.days.length - 1);
  const barRefs = useRef<(HTMLButtonElement | null)[]>([]);
  const moveFocus = (i: number) => {
    const next = Math.max(0, Math.min(insights.days.length - 1, i));
    setFocusIdx(next);
    barRefs.current[next]?.focus();
  };
  const values = insights.days.map((d) => d.openTabs ?? 0);
  const baseline = insights.firstWeekOpenTabs;
  const top = niceMax(Math.max(1, ...values, baseline ?? 0));
  const hasData = insights.days.some((d) => d.openTabs !== null);
  const pctOf = (v: number) => `${(v / top) * 100}%`;
  const hovered = hover === null ? null : insights.days[hover];
  const labelIdx = new Set([
    0,
    Math.floor((insights.days.length - 1) / 2),
    insights.days.length - 1,
  ]);
  return (
    <section
      aria-labelledby="open-tabs-title"
      className="min-w-0 rounded-card border border-line bg-surface p-5 shadow-[var(--shadow-card)] sm:p-6"
    >
      <div className="flex flex-wrap items-baseline justify-between gap-2">
        <div>
          <h2 id="open-tabs-title" className="font-serif text-2xl">
            Open tabs, last {insights.range} days
          </h2>
          <p className="text-sm text-muted">Median open tabs per day, counted hourly</p>
        </div>
        {baseline !== null && (
          <p className="flex items-center gap-2 text-sm text-muted">
            <span className="inline-block w-4 border-t border-faint" aria-hidden /> First week:{' '}
            {fmt(baseline)}
          </p>
        )}
      </div>

      {hasData ? (
        <div className="mt-12 grid grid-cols-[auto_minmax(0,1fr)] gap-x-2">
          {/* Y axis ticks */}
          <div className="relative h-56 font-mono text-xs text-muted" aria-hidden>
            {[top, top / 2, 0].map((t) => (
              <span
                key={t}
                className="absolute right-0 -translate-y-1/2"
                style={{ bottom: pctOf(t) }}
              >
                {fmt(Math.round(t))}
              </span>
            ))}
          </div>
          <div className="relative h-56">
            {[top, top / 2].map((t) => (
              <div
                key={t}
                className="absolute inset-x-0 border-t border-line"
                style={{ bottom: pctOf(t) }}
                aria-hidden
              />
            ))}
            {baseline !== null && (
              <div
                className="absolute inset-x-0 border-t border-dashed border-faint"
                style={{ bottom: pctOf(baseline) }}
                aria-hidden
              >
                <span className="absolute right-0 -translate-y-full rounded-full bg-surface px-1.5 text-xs text-muted">
                  {fmt(baseline)} tabs
                </span>
              </div>
            )}
            <ul
              aria-label="Open tabs per day (arrow keys move between days)"
              className="absolute inset-0 flex items-end gap-[2px] border-b border-line-strong"
            >
              {insights.days.map((d, i) => (
                <li key={d.day} className="flex h-full min-w-0 flex-1 justify-center">
                  <button
                    ref={(el) => {
                      barRefs.current[i] = el;
                    }}
                    type="button"
                    tabIndex={i === focusIdx ? 0 : -1}
                    onKeyDown={(e) => {
                      const to =
                        e.key === 'ArrowLeft'
                          ? i - 1
                          : e.key === 'ArrowRight'
                            ? i + 1
                            : e.key === 'Home'
                              ? 0
                              : e.key === 'End'
                                ? insights.days.length - 1
                                : null;
                      if (to === null) return;
                      e.preventDefault();
                      moveFocus(to);
                    }}
                    className="group flex h-full w-full items-end justify-center rounded-sm focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-accent"
                    aria-label={`${shortDay(d.day)}: ${d.openTabs === null ? 'no data' : `${d.openTabs} open tabs`}`}
                    onPointerEnter={() => setHover(i)}
                    onPointerLeave={() => setHover((h) => (h === i ? null : h))}
                    onFocus={() => {
                      setHover(i);
                      setFocusIdx(i);
                    }}
                    onBlur={() => setHover((h) => (h === i ? null : h))}
                  >
                    {d.openTabs !== null && (
                      <span
                        className={`block w-full max-w-6 rounded-t bg-chart ${hover === i ? 'brightness-110' : ''}`}
                        style={{ height: pctOf(d.openTabs) }}
                      />
                    )}
                  </button>
                </li>
              ))}
            </ul>
            {hovered && hover !== null && (
              <div
                aria-hidden="true"
                className="pointer-events-none absolute -top-2 z-10 -translate-x-1/2 -translate-y-full rounded-full bg-primary px-3 py-1 text-xs whitespace-nowrap text-primary-ink"
                style={{
                  left: `clamp(48px, ${((hover + 0.5) / insights.days.length) * 100}%, calc(100% - 48px))`,
                }}
              >
                <strong>
                  {hovered.openTabs === null ? 'No data' : `${hovered.openTabs} tabs`}
                </strong>{' '}
                · {shortDay(hovered.day)}
              </div>
            )}
          </div>
          <div />
          <div className="mt-2 flex text-xs text-muted" aria-hidden>
            {insights.days.map((d, i) => (
              <span key={d.day} className="min-w-0 flex-1 text-center whitespace-nowrap">
                {labelIdx.has(i) ? shortDay(d.day) : ''}
              </span>
            ))}
          </div>
        </div>
      ) : (
        <p className="mt-6 rounded-tile bg-surface-2 px-4 py-10 text-center text-sm text-muted">
          No samples yet. Decay counts your open tabs once an hour while you browse.
        </p>
      )}

      {hasData && (
        <details className="mt-4 text-sm">
          <summary className="cursor-pointer text-muted hover:text-ink">Show as table</summary>
          <table className="mt-2 w-full text-left">
            <thead className="text-muted">
              <tr>
                <th className="py-1 font-medium">Day</th>
                <th className="py-1 text-right font-medium">Open tabs (median)</th>
              </tr>
            </thead>
            <tbody>
              {insights.days.map((d) => (
                <tr key={d.day} className="border-t border-line">
                  <td className="py-1">{shortDay(d.day)}</td>
                  <td className="py-1 text-right font-mono">{d.openTabs ?? '—'}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </details>
      )}
    </section>
  );
}

const STATUS: Record<Goal['status'], { label: string; icon: ReactNode }> = {
  met: { label: 'On target', icon: <Check size={14} aria-hidden /> },
  notYet: { label: 'Not yet', icon: <Clock size={14} aria-hidden /> },
  noData: { label: 'No data yet', icon: <Minus size={14} aria-hidden /> },
};

function Goals({ goals, privacy }: { goals: Goal[]; privacy: string }) {
  return (
    <section
      aria-labelledby="goals-title"
      className="flex flex-col rounded-card bg-feature p-6 text-feature-ink lg:col-start-2 lg:row-span-2 lg:row-start-1"
    >
      <span className="self-start rounded-full bg-lime px-3 py-1 text-xs font-medium tracking-widest text-lime-ink uppercase">
        Beta goals
      </span>
      <h2 id="goals-title" className="mt-4 font-serif text-3xl">
        How Decay is doing
      </h2>
      <p className="mt-1 text-sm text-feature-sub">The targets the beta is measured against.</p>
      <ul className="mt-5 space-y-3">
        {goals.map((g) => (
          <li
            key={g.id}
            className="rounded-tile border border-feature-tile-line bg-feature-tile px-4 py-3"
          >
            <div className="flex items-center justify-between gap-2">
              <span className="text-xs tracking-widest text-feature-sub uppercase">{g.label}</span>
              <span className="inline-flex shrink-0 items-center gap-1 text-xs whitespace-nowrap">
                {STATUS[g.status].icon}
                {STATUS[g.status].label}
              </span>
            </div>
            <p className="mt-1 font-serif text-2xl">{g.value}</p>
            <p className="text-sm text-feature-sub">Target: {g.target}</p>
          </li>
        ))}
      </ul>
      <p className="mt-auto flex items-center gap-2 pt-5 text-xs text-feature-sub">
        <Lock size={14} aria-hidden /> {privacy}
      </p>
    </section>
  );
}

function DomainList(props: {
  title: string;
  subtitle: string;
  rows: { domain: string; count: number }[];
  unit: string;
  empty: string;
}) {
  const max = Math.max(1, ...props.rows.map((r) => r.count));
  return (
    <section className="rounded-card border border-line bg-surface p-5 shadow-[var(--shadow-card)] sm:p-6">
      <h2 className="font-serif text-2xl">{props.title}</h2>
      <p className="text-sm text-muted">{props.subtitle}</p>
      {props.rows.length === 0 ? (
        <p className="mt-4 text-sm text-muted">{props.empty}</p>
      ) : (
        <ul className="mt-4 space-y-3">
          {props.rows.map((r) => (
            <li key={r.domain}>
              <div className="flex items-baseline justify-between gap-3 text-sm">
                <span className="truncate">{r.domain}</span>
                <span className="shrink-0 font-mono text-muted">
                  {fmt(r.count)} {props.unit}
                  {r.count === 1 ? '' : 's'}
                </span>
              </div>
              <div className="mt-1 h-1.5 rounded-full bg-surface-2" aria-hidden>
                <div
                  className="h-full rounded-full bg-chart"
                  style={{ width: `${(r.count / max) * 100}%` }}
                />
              </div>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function ShareDialog(props: {
  json: string;
  onDownload: () => void;
  onCancel: () => void;
  shareWhat: string;
}) {
  const titleId = useId();
  const downloadRef = useRef<HTMLButtonElement>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const onCancel = useRef(props.onCancel);
  onCancel.current = props.onCancel;
  // Once on open: focus Save; on close, focus goes back to whatever opened the dialog.
  useEffect(() => {
    const opener = document.activeElement as HTMLElement | null;
    downloadRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onCancel.current();
      if (e.key !== 'Tab' || !dialogRef.current) return;
      // Keep Tab inside the dialog (the page behind it is inert).
      const focusable = dialogRef.current.querySelectorAll<HTMLElement>('button');
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      const active = document.activeElement;
      if (e.shiftKey && (active === first || !dialogRef.current.contains(active))) {
        e.preventDefault();
        last?.focus();
      } else if (!e.shiftKey && (active === last || !dialogRef.current.contains(active))) {
        e.preventDefault();
        first?.focus();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('keydown', onKey);
      opener?.focus?.();
    };
  }, []);
  return (
    <div className="fixed inset-0 z-20 grid place-items-center bg-black/40 p-4">
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className="flex max-h-[90vh] w-full max-w-2xl flex-col rounded-card bg-surface p-6 shadow-xl"
      >
        <div className="flex items-start justify-between gap-4">
          <div>
            <h2 id={titleId} className="font-serif text-2xl">
              Share with the beta team
            </h2>
            <p className="mt-1 text-sm text-muted">
              This is the whole file. It has counts and medians only — no addresses, page titles,
              sites or rule patterns — plus the dates you browsed and for how many minutes, your
              Decay settings (mode, thresholds, number of rules), {props.shareWhat}. The install id
              is random, made on this device; it links your exports to each other so the team can
              count testers, but not to you. Save the file, then send it to the team yourself.
            </p>
          </div>
          <button
            type="button"
            onClick={props.onCancel}
            className="grid size-8 shrink-0 place-items-center rounded-full text-muted hover:bg-surface-2"
          >
            <X size={16} aria-hidden />
            <span className="sr-only">Close</span>
          </button>
        </div>
        {/* Chrome makes scrollable boxes keyboard-focusable on its own. */}
        <pre className="mt-4 min-h-0 flex-1 overflow-auto rounded-tile bg-surface-2 p-4 font-mono text-xs">
          {props.json}
        </pre>
        <div className="mt-4 flex justify-end gap-2">
          <button
            type="button"
            onClick={props.onCancel}
            className="rounded-full border border-line-strong bg-cta px-4 py-1.5 text-sm font-medium text-cta-ink hover:brightness-95"
          >
            Cancel
          </button>
          <button
            ref={downloadRef}
            type="button"
            onClick={props.onDownload}
            className="inline-flex items-center gap-2 rounded-full bg-primary px-4 py-1.5 text-sm font-medium text-primary-ink hover:opacity-90"
          >
            <Download size={16} aria-hidden /> Save file
          </button>
        </div>
      </div>
    </div>
  );
}
