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
  ShieldCheck,
  Sparkles,
  X,
} from 'lucide-react';
import { type ReactNode, useEffect, useId, useRef, useState } from 'react';
import { DialogActions } from '../DialogActions';
import { formatDate, formatNumber, logicalArrow } from '../i18n';
import { t } from '../strings';
import { type Goal, type Insights, percent, RANGES, type RangeDays } from './insights';
import type { Mode } from './types';

/** What differs between the extension's page and the app's (where it's computed). */
export interface InsightsCopy {
  intro: string;
  privacy: string;
  /** Who made the shared file (the share dialog says what else is in it). */
  client: 'browser' | 'app';
}

/** The extension's wording (in the current locale). */
export const browserCopy = (): InsightsCopy => ({
  intro: t('insights.intro.browser'),
  privacy: t('insights.privacy.browser'),
  client: 'browser',
});

/** The desktop app's wording (in the current locale). */
export const appCopy = (): InsightsCopy => ({
  intro: t('insights.intro.app'),
  privacy: t('insights.privacy.app'),
  client: 'app',
});

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
  /** "Always keep" on the restored-most rows: makes a protect rule for the site. Rejects
   * on failure (the page shows the error); without it, the rows have no button. */
  onProtectSite?: (domain: string) => Promise<void>;
  /** Sites already always kept (an enabled protect rule for the domain). */
  protectedSites?: ReadonlySet<string>;
  /** What was archived by browser profile or app (the app only; Designs/14 "Top
   * sources"). Without it, no card. */
  topSources?: readonly { id: string; label: string; tabs: number; windows: number }[];
  /** Everything archived in the range (shares are of this; default: the rows' sum). */
  topSourcesTotal?: number;
}

const fmt = (n: number) => formatNumber(n);
const shortDay = (day: string) =>
  formatDate(new Date(`${day}T12:00:00`), { month: 'short', day: 'numeric' });

/** Insights (Designs/14, adapted to what the extension measures): on-device metrics. */
export function InsightsView(props: InsightsProps) {
  const { insights: data } = props;
  const mostTidied = data.topArchived[0];
  const regret = data.regretRate ?? data.shadowRegretRate;
  const copy = props.copy ?? browserCopy();
  return (
    <div className={props.embedded ? 'text-ink' : 'min-h-screen bg-canvas text-ink'}>
      {/* Inert while the share dialog is open: focus stays in the dialog. */}
      <div inert={props.sharePreview !== null}>
        {props.header}

        <main className={props.embedded ? '' : 'mx-auto max-w-6xl px-4 py-8 sm:px-10'}>
          <h1 className="font-serif text-4xl tracking-tight sm:text-5xl">{t('insights.title')}</h1>
          <p className="mt-2 text-muted">{copy.intro}</p>

          {/* Filters scope everything below them: one row, above the content. */}
          <div className="mt-6 flex flex-wrap items-center gap-3">
            <fieldset className="inline-flex rounded-full bg-surface-2 p-1">
              <legend className="sr-only">{t('insights.range')}</legend>
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
                  {t('insights.lastDays', { n: r })}
                </label>
              ))}
            </fieldset>
            <div className="flex gap-2 sm:ms-auto">
              <button
                type="button"
                onClick={props.onExportCsv}
                className="inline-flex items-center gap-2 rounded-full border border-line-strong bg-cta px-4 py-1.5 text-sm font-medium text-cta-ink hover:brightness-95"
              >
                <Download size={16} aria-hidden /> {t('insights.exportCsv')}
              </button>
              <button
                type="button"
                onClick={props.onShare}
                className="inline-flex items-center gap-2 rounded-full bg-primary px-4 py-1.5 text-sm font-medium text-primary-ink hover:opacity-90"
              >
                <Share2 size={16} aria-hidden /> {t('insights.share')}
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

          <section
            aria-label={t('insights.summary')}
            className="mt-6 grid gap-4 sm:grid-cols-2 lg:grid-cols-4"
          >
            <Tile label={t('insights.medianOpenTabs')} icon={<PanelTop size={18} aria-hidden />}>
              <Value>{data.openTabs === null ? t('insights.none') : fmt(data.openTabs)}</Value>
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
                        ? t('insights.change.fewer')
                        : data.openTabsChange > 0
                          ? t('insights.change.more')
                          : t('insights.change.none')}
                    </span>
                  </span>
                  {t('insights.vsWeek1', { n: data.firstWeekOpenTabs })}
                </p>
              ) : (
                <Sub>
                  {data.openTabs === null
                    ? t('insights.countedHourly')
                    : t('insights.comparedAfter14')}
                </Sub>
              )}
            </Tile>
            <Tile label={t('insights.regretRate')} icon={<History size={18} aria-hidden />}>
              <Value>{percent(regret)}</Value>
              <Sub>
                {data.regretRate !== null
                  ? t('insights.restoredWithinDay', { n: data.restored24h })
                  : data.shadowRegretRate !== null
                    ? t('insights.shadowRegret')
                    : t('insights.nothingArchived')}
              </Sub>
            </Tile>
            <Tile label={t('insights.itemsArchived')} icon={<Archive size={18} aria-hidden />}>
              <Value>{fmt(data.archived)}</Value>
              <Sub>
                {data.undos
                  ? t('insights.sweepsUndone', { n: data.undos })
                  : t('insights.inLastDays', { n: data.range })}
              </Sub>
            </Tile>
            <Tile label={t('insights.mostTidied')} icon={<Sparkles size={18} aria-hidden />}>
              <p className="mt-3 truncate text-2xl font-medium" title={mostTidied?.domain}>
                {mostTidied?.domain ?? t('insights.none')}
              </p>
              <Sub>
                {mostTidied
                  ? t('insights.quietlyArchived', { n: mostTidied.count })
                  : t('insights.nothingArchived')}
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
                title={t('insights.archivedMost')}
                subtitle={t('insights.archivedMost.sub', { n: data.range })}
                rows={data.topArchived}
                count={(n) => t('insights.tabs', { n })}
                empty={t('insights.archivedMost.empty')}
              />
              <DomainList
                title={t('insights.restoredMost')}
                subtitle={t('insights.restoredMost.sub')}
                rows={data.restoredMost}
                count={(n) => t('insights.restores', { n })}
                empty={t('insights.restoredMost.empty')}
                onProtect={props.onProtectSite}
                protectedSites={props.protectedSites}
              />
              {props.topSources && (
                <TopSources
                  rows={props.topSources}
                  total={props.topSourcesTotal}
                  range={data.range}
                />
              )}
            </div>
          </div>
        </main>
      </div>

      {props.sharePreview !== null && (
        <ShareDialog
          json={props.sharePreview}
          onDownload={props.onShareDownload}
          onCancel={props.onShareCancel}
          client={copy.client}
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
            {t('insights.chart.title', { n: insights.range })}
          </h2>
          <p className="text-sm text-muted">{t('insights.chart.sub')}</p>
        </div>
        {baseline !== null && (
          <p className="flex items-center gap-2 text-sm text-muted">
            <span className="inline-block w-4 border-t border-faint" aria-hidden />{' '}
            {t('insights.chart.firstWeek', { n: baseline })}
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
                className="absolute end-0 -translate-y-1/2"
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
                <span className="absolute end-0 -translate-y-full rounded-full bg-surface px-1.5 text-xs text-muted">
                  {t('insights.tabs', { n: baseline })}
                </span>
              </div>
            )}
            <ul
              aria-label={t('insights.chart.days')}
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
                      const key = logicalArrow(e.key);
                      const to =
                        key === 'ArrowLeft'
                          ? i - 1
                          : key === 'ArrowRight'
                            ? i + 1
                            : key === 'Home'
                              ? 0
                              : key === 'End'
                                ? insights.days.length - 1
                                : null;
                      if (to === null) return;
                      e.preventDefault();
                      moveFocus(to);
                    }}
                    className="group flex h-full w-full items-end justify-center rounded-sm focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-accent"
                    aria-label={t('insights.chart.bar', {
                      day: shortDay(d.day),
                      data: d.openTabs === null ? 'none' : 'some',
                      n: d.openTabs ?? 0,
                    })}
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
                className="pointer-events-none absolute -top-2 z-10 -translate-x-1/2 -translate-y-full rtl:translate-x-1/2 rounded-full bg-primary px-3 py-1 text-xs whitespace-nowrap text-primary-ink"
                style={{
                  insetInlineStart: `clamp(48px, ${((hover + 0.5) / insights.days.length) * 100}%, calc(100% - 48px))`,
                }}
              >
                <strong>
                  {hovered.openTabs === null
                    ? t('insights.chart.tip.none')
                    : t('insights.tabs', { n: hovered.openTabs })}
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
          {t('insights.chart.empty')}
        </p>
      )}

      {hasData && (
        <details className="mt-4 text-sm">
          <summary className="cursor-pointer text-muted hover:text-ink">
            {t('insights.chart.table')}
          </summary>
          <table className="mt-2 w-full text-start">
            <thead className="text-muted">
              <tr>
                <th className="py-1 font-medium">{t('insights.chart.day')}</th>
                <th className="py-1 text-end font-medium">{t('insights.chart.median')}</th>
              </tr>
            </thead>
            <tbody>
              {insights.days.map((d) => (
                <tr key={d.day} className="border-t border-line">
                  <td className="py-1">{shortDay(d.day)}</td>
                  <td className="py-1 text-end font-mono">
                    {d.openTabs === null ? t('insights.none') : fmt(d.openTabs)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </details>
      )}
    </section>
  );
}

const STATUS: Record<Goal['status'], { label: () => string; icon: ReactNode }> = {
  met: { label: () => t('insights.status.met'), icon: <Check size={14} aria-hidden /> },
  notYet: { label: () => t('insights.status.notYet'), icon: <Clock size={14} aria-hidden /> },
  noData: { label: () => t('insights.status.noData'), icon: <Minus size={14} aria-hidden /> },
};

function Goals({ goals, privacy }: { goals: Goal[]; privacy: string }) {
  return (
    <section
      aria-labelledby="goals-title"
      className="flex flex-col rounded-card bg-feature p-6 text-feature-ink lg:col-start-2 lg:row-span-2 lg:row-start-1"
    >
      <span className="self-start rounded-full bg-lime px-3 py-1 text-xs font-medium tracking-widest text-lime-ink uppercase">
        {t('insights.goals.badge')}
      </span>
      <h2 id="goals-title" className="mt-4 font-serif text-3xl">
        {t('insights.goals.title')}
      </h2>
      <p className="mt-1 text-sm text-feature-sub">{t('insights.goals.sub')}</p>
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
                {STATUS[g.status].label()}
              </span>
            </div>
            <p className="mt-1 font-serif text-2xl">{g.value}</p>
            <p className="text-sm text-feature-sub">
              {t('insights.goals.target', { target: g.target })}
            </p>
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
  /** "3 tabs", in the locale's words. */
  count: (n: number) => string;
  empty: string;
  /** Adds an "Always keep" button per row. */
  onProtect?: (domain: string) => Promise<void>;
  protectedSites?: ReadonlySet<string>;
}) {
  const max = Math.max(1, ...props.rows.map((r) => r.count));
  const [busy, setBusy] = useState<ReadonlySet<string>>(new Set());
  // Kept here until the page's `protectedSites` catch up; once they change (a rule added,
  // turned off or deleted anywhere), they alone say which sites are kept.
  const [kept, setKept] = useState<ReadonlySet<string>>(new Set());
  // biome-ignore lint/correctness/useExhaustiveDependencies: reset when the page's list changes
  useEffect(() => setKept(new Set()), [props.protectedSites]);
  // Clicks before a re-render see the same `busy`: this one is current at once.
  const inFlight = useRef(new Set<string>());
  const { onProtect } = props;
  const protect = (domain: string) => {
    if (!onProtect || inFlight.current.has(domain)) return;
    inFlight.current.add(domain);
    setBusy((b) => new Set(b).add(domain));
    onProtect(domain)
      .then(() => setKept((k) => new Set(k).add(domain)))
      .catch(() => {
        // The page shows the error; the button stays for another try.
      })
      .finally(() => {
        inFlight.current.delete(domain);
        setBusy((b) => {
          const next = new Set(b);
          next.delete(domain);
          return next;
        });
      });
  };
  return (
    <section className="rounded-card border border-line bg-surface p-5 shadow-[var(--shadow-card)] sm:p-6">
      <h2 className="font-serif text-2xl">{props.title}</h2>
      <p className="text-sm text-muted">{props.subtitle}</p>
      {props.rows.length === 0 ? (
        <p className="mt-4 text-sm text-muted">{props.empty}</p>
      ) : (
        <ul className="mt-4 space-y-3">
          {props.rows.map((r) => {
            const isKept = kept.has(r.domain) || (props.protectedSites?.has(r.domain) ?? false);
            const isBusy = busy.has(r.domain);
            return (
              <li key={r.domain}>
                <div className="flex items-center justify-between gap-3 text-sm">
                  <span className="min-w-0 truncate">{r.domain}</span>
                  <span className="ms-auto shrink-0 font-mono text-muted">
                    {props.count(r.count)}
                  </span>
                  {onProtect &&
                    (isKept ? (
                      <span className="inline-flex shrink-0 items-center gap-1 text-xs text-muted">
                        <ShieldCheck size={14} aria-hidden /> {t('insights.protected')}
                      </span>
                    ) : (
                      <button
                        type="button"
                        onClick={() => protect(r.domain)}
                        disabled={isBusy}
                        aria-busy={isBusy}
                        aria-label={t('insights.protectSite', { site: r.domain })}
                        className="inline-flex shrink-0 items-center gap-1 rounded-full border border-line-strong px-2.5 py-0.5 text-xs font-medium hover:bg-surface-2 focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-60"
                      >
                        <ShieldCheck size={14} aria-hidden />
                        {isBusy ? t('insights.protecting') : t('insights.protect')}
                      </button>
                    ))}
                </div>
                <div className="mt-1 h-1.5 rounded-full bg-surface-2" aria-hidden>
                  <div
                    className="h-full rounded-full bg-chart"
                    style={{ width: `${(r.count / max) * 100}%` }}
                  />
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}

/** Where archived tabs and windows came from: each browser profile or app, its share. */
function TopSources(props: {
  rows: readonly { id: string; label: string; tabs: number; windows: number }[];
  total?: number;
  range: number;
}) {
  const shown = props.rows.reduce((n, r) => n + r.tabs + r.windows, 0);
  const total = Math.max(props.total ?? 0, shown);
  const share = (r: { tabs: number; windows: number }) =>
    total ? Math.round(((r.tabs + r.windows) / total) * 100) : 0;
  const what = (r: { tabs: number; windows: number }) =>
    r.windows === 0
      ? t('insights.tabs', { n: r.tabs })
      : r.tabs === 0
        ? t('insights.windows', { n: r.windows })
        : t('insights.tabsAndWindows', { tabs: r.tabs, windows: r.windows });
  return (
    <section className="rounded-card border border-line bg-surface p-5 shadow-[var(--shadow-card)] sm:col-span-2 sm:p-6">
      <h2 className="font-serif text-2xl">{t('insights.topSources')}</h2>
      <p className="text-sm text-muted">{t('insights.topSources.sub', { n: props.range })}</p>
      {props.rows.length === 0 ? (
        <p className="mt-4 text-sm text-muted">{t('insights.archivedMost.empty')}</p>
      ) : (
        <ul className="mt-4 space-y-3">
          {props.rows.map((r) => (
            <li key={r.id}>
              <div className="flex items-center justify-between gap-3 text-sm">
                <span className="min-w-0 truncate">{r.label}</span>
                <span className="ms-auto shrink-0 font-mono text-muted">
                  {t('insights.sourceShare', { what: what(r), pct: share(r) })}
                </span>
              </div>
              <div className="mt-1 h-1.5 rounded-full bg-surface-2" aria-hidden>
                <div className="h-full rounded-full bg-chart" style={{ width: `${share(r)}%` }} />
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
  client: InsightsCopy['client'];
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
              {t('insights.share.title')}
            </h2>
            <p className="mt-1 text-sm text-muted">
              {t('insights.share.body', { client: props.client })}
            </p>
          </div>
          <button
            type="button"
            onClick={props.onCancel}
            className="grid size-8 shrink-0 place-items-center rounded-full text-muted hover:bg-surface-2"
          >
            <X size={16} aria-hidden />
            <span className="sr-only">{t('insights.share.close')}</span>
          </button>
        </div>
        {/* Chrome makes scrollable boxes keyboard-focusable on its own. */}
        <pre className="mt-4 min-h-0 flex-1 overflow-auto rounded-tile bg-surface-2 p-4 font-mono text-xs">
          {props.json}
        </pre>
        <DialogActions
          className="mt-4 justify-end"
          action={
            <button
              ref={downloadRef}
              type="button"
              onClick={props.onDownload}
              className="inline-flex items-center gap-2 rounded-full bg-primary px-4 py-1.5 text-sm font-medium text-primary-ink hover:opacity-90"
            >
              <Download size={16} aria-hidden /> {t('insights.share.save')}
            </button>
          }
          cancel={
            <button
              type="button"
              onClick={props.onCancel}
              className="rounded-full border border-line-strong bg-cta px-4 py-1.5 text-sm font-medium text-cta-ink hover:brightness-95"
            >
              {t('insights.share.cancel')}
            </button>
          }
        />
      </div>
    </div>
  );
}
