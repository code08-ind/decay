import { describe, expect, it } from 'vitest';
import type { SweepRow } from '../api';
import {
  byBatchName,
  entryState,
  eventText,
  firstDayOfWeek,
  groupByDay,
  parseRoute,
  percent,
  routeFor,
  shadowDaysLeft,
  shadowWeekOver,
  startOfDay,
  startOfWeek,
  sweepAction,
  sweepHeadline,
  triggerText,
} from './model';

const ID = '0b7f3a52-6c1e-4f7d-9a0e-2d4c8b1e9f00';

const sweep = (over: Partial<SweepRow>): SweepRow => ({
  id: 's',
  number: 1,
  startedAt: 0,
  trigger: 'scheduled',
  isShadow: false,
  status: 'done',
  itemCount: 3,
  tabs: 3,
  windows: 0,
  restored: 0,
  failed: 0,
  undoneAt: null,
  undoable: true,
  ...over,
});

describe('routes', () => {
  it('parses the routes deep links and the tray use', () => {
    expect(parseRoute('/')).toEqual({ view: 'library' });
    expect(parseRoute(`/item/${ID}`)).toEqual({ view: 'library', itemId: ID, confirm: undefined });
    expect(parseRoute(`/item/${ID}?confirm=restore`).confirm).toBe('restore');
    expect(parseRoute(`/sweeps/${ID}?confirm=undo`)).toEqual({
      view: 'sweeps',
      sweepId: ID,
      confirm: 'undo',
    });
    expect(parseRoute('/sweeps')).toEqual({
      view: 'sweeps',
      sweepId: undefined,
      confirm: undefined,
    });
    expect(parseRoute('/shadow-report')).toEqual({ view: 'shadow' });
    expect(parseRoute('/overview')).toEqual({ view: 'overview' });
  });

  it('ignores malformed ids and unknown routes', () => {
    expect(parseRoute('/item/<script>')).toEqual({ view: 'library' });
    expect(parseRoute('/item/a b?confirm=restore')).toEqual({ view: 'library' });
    expect(parseRoute('/nowhere')).toEqual({ view: 'library' });
    expect(parseRoute('/sweeps?confirm=undo').confirm).toBeUndefined();
  });

  it('round-trips', () => {
    for (const r of [
      '/',
      `/item/${ID}`,
      '/sweeps',
      `/sweeps/${ID}`,
      '/shadow-report',
      '/overview',
    ]) {
      expect(routeFor(parseRoute(r))).toBe(r);
    }
  });
});

describe('calendar', () => {
  it('finds local midnight and the start of the week', () => {
    const wed = new Date(2026, 10, 11, 15, 30).getTime(); // Wed 11 Nov 2026
    expect(startOfDay(wed)).toBe(new Date(2026, 10, 11).getTime());
    expect(startOfWeek(wed, 1)).toBe(new Date(2026, 10, 9).getTime()); // Monday
    expect(startOfWeek(wed, 0)).toBe(new Date(2026, 10, 8).getTime()); // Sunday
    const sun = new Date(2026, 10, 15, 9).getTime();
    expect(startOfWeek(sun, 1)).toBe(new Date(2026, 10, 9).getTime());
  });

  it('reads the first day of the week from the locale', () => {
    expect([0, 1, 6]).toContain(firstDayOfWeek('en-US'));
    expect(firstDayOfWeek('not a locale!')).toBe(1);
  });
});

describe('words', () => {
  it('describes sweeps', () => {
    expect(sweepHeadline({ tabs: 11, windows: 3, isShadow: false })).toBe(
      'Archived 11 tabs, 3 windows',
    );
    expect(sweepHeadline({ tabs: 1, windows: 0, isShadow: true })).toBe('Would archive 1 tab');
    expect(sweepHeadline({ tabs: 0, windows: 0, isShadow: false })).toBe('Archived nothing');
    expect(sweepHeadline({ tabs: 1, windows: 0, isShadow: false, itemCount: 3 })).toBe(
      'Archived 3 items',
    );
    expect(sweepHeadline({ tabs: 2, windows: 1, isShadow: false, itemCount: 3 })).toBe(
      'Archived 2 tabs, 1 window',
    );
    expect(triggerText('manual')).toBe('manual');
    expect(triggerText('wake')).toBe('after wake');
    expect(triggerText('scheduled')).toBe('automatic');
  });

  it('offers Undo only when it would bring something back', () => {
    expect(sweepAction(sweep({}))).toEqual({ label: 'Undo', undo: true });
    expect(sweepAction(sweep({ restored: 2 }))).toEqual({ label: 'Undo rest', undo: true });
    expect(sweepAction(sweep({ undoable: false, restored: 3 })).undo).toBe(false);
    expect(sweepAction(sweep({ undoable: false })).label).toBe('Nothing to undo');
    expect(sweepAction(sweep({ status: 'undone', undoable: false })).label).toBe('Undone');
    expect(sweepAction(sweep({ undoneAt: 5 })).label).toBe('Undone');
    expect(sweepAction(sweep({ status: 'failed' })).label).toBe('Failed');
    expect(sweepAction(sweep({ isShadow: true, undoable: false })).label).toBe('Simulated');
  });

  it('groups by day, keeping order', () => {
    const now = new Date(2026, 10, 11, 18).getTime();
    const rows = [
      { startedAt: new Date(2026, 10, 11, 14).getTime(), id: 'a' },
      { startedAt: new Date(2026, 10, 11, 9).getTime(), id: 'b' },
      { startedAt: new Date(2026, 10, 10, 16).getTime(), id: 'c' },
    ];
    const groups = groupByDay(rows, now, 'en-US');
    expect(groups.map((g) => g.rows.map((r) => r.id))).toEqual([['a', 'b'], ['c']]);
    expect(groups[0]?.label).toBe('Today');
    expect(groups[1]?.label).toBe('Yesterday');
  });

  it('formats percentages, events and the shadow week', () => {
    expect(percent(2, 63)).toBe('3 %');
    expect(percent(0, 0)).toBe('–');
    expect(eventText('shadow_would_archive')).toBe('Would have been archived');
    expect(eventText('mystery')).toBe('mystery');
    expect(shadowDaysLeft(6)).toBe('2 days left');
    expect(shadowDaysLeft(7)).toBe('last day');
    expect(shadowDaysLeft(1)).toBe('7 days left');
    expect(shadowDaysLeft(8)).toBe('the shadow week is over');
    expect(shadowWeekOver(7)).toBe(false);
    expect(shadowWeekOver(8)).toBe(true);
    expect(entryState('archiving')).toBe('Closing…');
    expect(entryState('archived')).toBe('Archived again later');
    expect(entryState('active')).toBe('Restored');
  });
});

describe('batch names', () => {
  it("groups a sweep's items under their names, in order", () => {
    const items = [
      { id: 'a', batchLabel: 'Lisbon trip' },
      { id: 'b' },
      { id: 'c', batchLabel: 'Rust async' },
      { id: 'd', batchLabel: 'Lisbon trip' },
    ];
    expect(byBatchName(items).map((g) => [g.label, g.items.map((i) => i.id)])).toEqual([
      ['Lisbon trip', ['a', 'd']],
      [null, ['b']],
      ['Rust async', ['c']],
    ]);
    expect(byBatchName([])).toEqual([]);
  });
});
