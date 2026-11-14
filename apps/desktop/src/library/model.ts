// The Library window without the UI (P1.11): routes, calendar boundaries and the words
// the screens use.
import type { SweepRow } from '../api';
import { dayLabel } from '../panel/results';

export type View = 'library' | 'sweeps' | 'shadow' | 'summary' | 'overview';

export interface Route {
  view: View;
  /** `/item/{id}`: the detail drawer. */
  itemId?: string;
  /** `/sweeps/{id}`: a sweep's details. */
  sweepId?: string;
  /** `?confirm=restore` / `?confirm=undo` from a deep link: ask before acting. */
  confirm?: 'restore' | 'undo';
}

const ID = /^[A-Za-z0-9-]{1,64}$/;

/** Parses a Library route (deep links, tray, ⌘I). Unknown routes show the Library. */
export function parseRoute(route: string): Route {
  const [path = '/', query = ''] = route.split('?');
  const parts = path.split('/').filter(Boolean);
  const confirmParam = new URLSearchParams(query).get('confirm');
  const id = parts[1] && ID.test(parts[1]) ? parts[1] : undefined;
  switch (parts[0]) {
    case 'item':
      return id
        ? {
            view: 'library',
            itemId: id,
            confirm: confirmParam === 'restore' ? 'restore' : undefined,
          }
        : { view: 'library' };
    case 'sweeps':
      return {
        view: 'sweeps',
        sweepId: id,
        confirm: id && confirmParam === 'undo' ? 'undo' : undefined,
      };
    case 'shadow-report':
      return { view: 'shadow' };
    case 'overview':
      return { view: 'overview' };
    case 'summary':
      return { view: 'summary' };
    default:
      return { view: 'library' };
  }
}

/** The route of a view (and an open item / sweep). */
export function routeFor(r: Route): string {
  switch (r.view) {
    case 'sweeps':
      return r.sweepId ? `/sweeps/${r.sweepId}` : '/sweeps';
    case 'shadow':
      return '/shadow-report';
    case 'overview':
      return '/overview';
    case 'summary':
      return '/summary';
    default:
      return r.itemId ? `/item/${r.itemId}` : '/';
  }
}

/** Local midnight of yesterday and today: yesterday's `[start, end)`. */
export function yesterday(now: number): [number, number] {
  const end = startOfDay(now);
  const d = new Date(end);
  d.setDate(d.getDate() - 1);
  return [d.getTime(), end];
}

/** Local midnight today. */
export function startOfDay(now: number): number {
  const d = new Date(now);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

/** Local midnight on the first day of this week (`firstDay`: 0 Sunday … 1 Monday). */
export function startOfWeek(now: number, firstDay = 1): number {
  const d = new Date(startOfDay(now));
  const back = (d.getDay() - firstDay + 7) % 7;
  d.setDate(d.getDate() - back);
  return d.getTime();
}

/** The first day of the week for a locale (Intl's week info where the engine has it). */
export function firstDayOfWeek(locale?: string): number {
  try {
    const loc = new Intl.Locale(locale ?? navigator.language) as Intl.Locale & {
      getWeekInfo?: () => { firstDay: number };
      weekInfo?: { firstDay: number };
    };
    const info = loc.getWeekInfo?.() ?? loc.weekInfo;
    // Intl counts 1 = Monday … 7 = Sunday.
    if (info) return info.firstDay % 7;
  } catch {
    // fall through
  }
  return 1;
}

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

/** "Archived 11 tabs, 3 windows" / "Would archive 4 tabs". */
export function sweepHeadline(
  s: Pick<SweepRow, 'tabs' | 'windows' | 'isShadow'> & { itemCount?: number },
): string {
  const parts = [s.tabs && plural(s.tabs, 'tab'), s.windows && plural(s.windows, 'window')].filter(
    Boolean,
  );
  // Some were deleted from the archive since: the sweep's own total says how many.
  const known = s.tabs + s.windows;
  const what =
    s.itemCount !== undefined && s.itemCount > known
      ? plural(s.itemCount, 'item')
      : parts.length
        ? parts.join(', ')
        : 'nothing';
  return `${s.isShadow ? 'Would archive' : 'Archived'} ${what}`;
}

/** What became of a sweep's item that isn't archived from it any more. */
export function entryState(state: string): string {
  if (state === 'archiving') return 'Closing…';
  if (state === 'archived') return 'Archived again later';
  return 'Restored';
}

/** How a sweep was started, in words. */
export function triggerText(t: SweepRow['trigger']): string {
  return t === 'manual' ? 'manual' : t === 'wake' ? 'after wake' : 'automatic';
}

/** What a sweep row offers: Undo, Undo rest (some restored), or its final state. */
export function sweepAction(s: SweepRow): { label: string; undo: boolean } {
  if (s.isShadow) return { label: 'Simulated', undo: false };
  if (s.status === 'undone' || s.undoneAt !== null) return { label: 'Undone', undo: false };
  if (s.status === 'failed') return { label: 'Failed', undo: false };
  if (!s.undoable)
    return { label: s.restored > 0 ? 'All restored' : 'Nothing to undo', undo: false };
  return { label: s.restored > 0 ? 'Undo rest' : 'Undo', undo: true };
}

/** Sweeps grouped by local day, newest first (they arrive newest first). */
export function groupByDay<T extends { startedAt: number }>(
  rows: T[],
  now: number,
  locale?: string,
): { label: string; rows: T[] }[] {
  const out: { label: string; rows: T[] }[] = [];
  for (const r of rows) {
    const label = dayLabel(r.startedAt, now, locale);
    const last = out[out.length - 1];
    if (last && last.label === label) last.rows.push(r);
    else out.push({ label, rows: [r] });
  }
  return out;
}

/** A whole percentage ("3 %"), or a dash when there's nothing to divide by. */
export function percent(part: number, whole: number): string {
  if (whole <= 0) return '–';
  return `${Math.round((part / whole) * 100)} %`;
}

/** Words for timeline event types. */
export function eventText(type: string): string {
  const words: Record<string, string> = {
    seen: 'Opened',
    used: 'Used',
    stale: 'Turned stale',
    hidden: 'Hidden',
    archived: 'Archived',
    close_failed: "Couldn't close",
    restored: 'Restored',
    undo: 'Undone',
    protected: 'Protected',
    purged: 'Removed',
    shadow_would_archive: 'Would have been archived',
  };
  return words[type] ?? type;
}

/** Shadow mode's remaining days, in words. */
export function shadowDaysLeft(day: number, days = 7): string {
  // Day 7 is the last day of the week: it's over from day 8.
  const left = days - day + 1;
  if (left <= 0) return 'the shadow week is over';
  if (left === 1) return 'last day';
  return `${plural(left, 'day')} left`;
}

/** The shadow week is over: time to decide about automatic mode. */
export function shadowWeekOver(day: number, days = 7): boolean {
  return day > days;
}
