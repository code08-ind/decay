import { describe, expect, it } from 'vitest';
import type { PanelItem } from '../api';
import {
  dayLabel,
  displayHost,
  displayUrl,
  groupResults,
  highlight,
  monogram,
  panelAction,
  searchWords,
  timeLabel,
} from './results';

// Local-time anchors (tests run in the machine's zone; labels are local by design).
const at = (y: number, mo: number, d: number, h = 12, mi = 0) =>
  new Date(y, mo - 1, d, h, mi).getTime();
const NOW = at(2026, 11, 13, 15, 0); // a Friday

function item(id: string, over: Partial<PanelItem> = {}): PanelItem {
  return {
    id,
    kind: 'tab',
    title: `Item ${id}`,
    url: `https://${id}.example/page`,
    domain: `${id}.example`,
    appName: 'Google Chrome',
    sourceId: 's1',
    sourceTitle: 'Research',
    archivedAt: NOW - 3_600_000,
    reason: 'Unused 61 active h',
    batch: true,
    ...over,
  };
}

describe('labels', () => {
  it('names days the way people say them', () => {
    expect(dayLabel(at(2026, 11, 13, 0, 1), NOW, 'en-US')).toBe('Today');
    expect(dayLabel(at(2026, 11, 12, 23, 59), NOW, 'en-US')).toBe('Yesterday');
    expect(dayLabel(at(2026, 11, 10), NOW, 'en-US')).toBe('Last Tuesday');
    expect(dayLabel(at(2026, 10, 28), NOW, 'en-US')).toBe('Oct 28');
    expect(dayLabel(at(2025, 10, 28), NOW, 'en-US')).toBe('Oct 28, 2025');
  });

  it('shows the time today, the date otherwise', () => {
    expect(timeLabel(at(2026, 11, 13, 14, 14), NOW, 'en-US')).toBe('Today, 2:14 PM');
    expect(timeLabel(at(2026, 10, 28), NOW, 'en-US')).toBe('Wed, Oct 28');
  });

  it('shows the URL as host / path', () => {
    expect(displayUrl('https://stripe.com/pricing/compare?x=1#y')).toBe(
      'stripe.com / pricing/compare',
    );
    expect(displayUrl('https://vercel.com/')).toBe('vercel.com');
    expect(displayUrl(null)).toBe('');
    expect(displayUrl('not a url')).toBe('not a url');
    // International addresses read as people write them.
    expect(displayUrl('https://xn--strae-oqa.example/%C3%B1and%C3%BA')).toBe(
      'straße.example / ñandú',
    );
    expect(displayHost('xn--bcher-kva.example')).toBe('bücher.example');
    expect(displayHost('xn--99999999999999.example')).toBe('xn--99999999999999.example');
    expect(displayUrl('https://a.example/%E0%A4%A')).toBe('a.example / %E0%A4%A');
  });
});

describe('groupResults', () => {
  it('groups by day and window, in ranking order', () => {
    const older = at(2026, 11, 10);
    const groups = groupResults(
      [
        item('a'),
        item('b', { archivedAt: older, sourceId: 's2', sourceTitle: null, appName: 'Arc' }),
        item('c'),
        item('d', { sourceId: null, sourceTitle: null, batch: false }),
      ],
      NOW,
      'en-US',
    );
    expect(groups.map((g) => g.label)).toEqual([
      'Today · Google Chrome — Research',
      'Last Tuesday · Arc',
      'Today · Google Chrome',
    ]);
    expect(groups[0]?.items.map((i) => i.id)).toEqual(['a', 'c']);
    // "Restore all" only where there is a batch of more than one.
    expect(groups.map((g) => g.batch)).toEqual([true, false, false]);
  });

  it('a named batch (F15) shows its name instead of its window', () => {
    const groups = groupResults(
      [item('a', { batchLabel: 'Lisbon trip' }), item('b')],
      NOW,
      'en-US',
    );
    expect(groups.map((g) => g.label)).toEqual(['Today · Lisbon trip']);
  });
});

describe('highlighting', () => {
  it('drops operators from the words', () => {
    expect(searchWords('Pricing site:stripe.com  "plans" before:2026-11-01')).toEqual([
      'pricing',
      'plans',
    ]);
  });

  it('marks word-prefix matches, any case', () => {
    expect(highlight('Stripe pricing — Compare plans', ['pric', 'plan'])).toEqual([
      { text: 'Stripe ', match: false },
      { text: 'pricing', match: true },
      { text: ' — Compare ', match: false },
      { text: 'plans', match: true },
    ]);
    // Not inside a word; special characters are literal.
    expect(highlight('Repricing', ['pric'])).toEqual([{ text: 'Repricing', match: false }]);
    expect(highlight('axb', ['a.b'])).toEqual([{ text: 'axb', match: false }]);
    expect(highlight('a.b (c)', ['(c'])).toEqual([
      { text: 'a.b ', match: false },
      { text: '(c', match: true },
      { text: ')', match: false },
    ]);
    expect(highlight('Café menu', ['café'])[0]).toEqual({ text: 'Café', match: true });
    // Accents don't matter, like in the search.
    expect(highlight('Ñandú facts', ['nandu'])[0]).toEqual({ text: 'Ñandú', match: true });
    expect(highlight('Cafe', ['café'])[0]).toEqual({ text: 'Cafe', match: true });
    expect(highlight('Anything', [])).toEqual([{ text: 'Anything', match: false }]);
  });
});

it('monograms are stable per site', () => {
  const a = monogram({ domain: 'stripe.com', title: 'x' });
  expect(a).toEqual(monogram({ domain: 'stripe.com', title: 'other' }));
  expect(a.letter).toBe('S');
  expect(monogram({ domain: null, title: '  — notes' }).letter).toBe('N');
  expect(monogram({ domain: null, title: '' }).letter).toBe('?');
});

describe('panelAction', () => {
  const key = (
    k: string,
    mods: Partial<Record<'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey', boolean>> = {},
  ) => ({
    key: k,
    metaKey: false,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    ...mods,
  });

  it('maps the guideline keys on macOS', () => {
    expect(panelAction(key('ArrowDown'), true)).toBe('next');
    expect(panelAction(key('Enter'), true)).toBe('restore');
    expect(panelAction(key('Enter', { metaKey: true }), true)).toBe('restoreGroup');
    expect(panelAction(key('Enter', { altKey: true }), true)).toBe('restoreNewWindow');
    expect(panelAction(key('p', { metaKey: true }), true)).toBe('protect');
    expect(panelAction(key('i', { metaKey: true }), true)).toBe('details');
    // Plain letters are typing, not actions.
    expect(panelAction(key('p'), true)).toBeNull();
    expect(panelAction(key('p', { ctrlKey: true }), true)).toBeNull();
  });

  it('leaves keys to an input method while it composes', () => {
    expect(panelAction({ ...key('Enter'), isComposing: true }, true)).toBeNull();
    expect(panelAction(key('Process'), true)).toBeNull();
    expect(panelAction({ ...key('ArrowDown'), isComposing: true }, false)).toBeNull();
  });

  it('uses Ctrl on Windows', () => {
    expect(panelAction(key('Enter', { ctrlKey: true }), false)).toBe('restoreGroup');
    expect(panelAction(key('P', { ctrlKey: true }), false)).toBe('protect');
    expect(panelAction(key('i', { metaKey: true }), false)).toBeNull();
  });
});
