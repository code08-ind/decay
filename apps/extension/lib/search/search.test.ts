import { describe, expect, it } from 'vitest';
import type { Item } from '../db/types';
import { ArchiveSearch, groupItems, parseDate, parseQuery, urlWords } from './search';

const NOW = new Date(2026, 10, 7, 15, 0).getTime();
const DAY = 86_400_000;

let n = 0;
function item(over: Partial<Item> & { url: string; title: string }): Item {
  const id = `i${n++}`;
  return {
    id,
    kind: 'tab',
    appId: 'com.google.Chrome',
    domain: new URL(over.url).hostname.replace(/^www\./, ''),
    pinned: false,
    state: 'archived',
    activeAgeS: 0,
    firstSeenAt: 0,
    lastUsedAt: 0,
    stateChangedAt: 0,
    restoreCount: 0,
    archivedAt: NOW - DAY,
    lastSweepId: 's1',
    ...over,
  };
}

describe('parseQuery', () => {
  it('splits inline filters from the text', () => {
    const q = parseQuery('pricing site:www.Stripe.com app:edge after:2026-11-01 before:today', NOW);
    expect(q).toEqual({
      text: 'pricing',
      site: 'stripe.com',
      appId: 'com.microsoft.Edge',
      after: new Date(2026, 10, 1).getTime(),
      before: new Date(2026, 10, 7).getTime(),
    });
  });

  it('keeps unknown or invalid operators as text', () => {
    expect(parseQuery('app:safari after:soon foo:bar', NOW)).toEqual({
      text: 'app:safari after:soon foo:bar',
    });
  });

  it('computes "yesterday" by calendar day across DST changes', () => {
    // Whatever the machine's zone, yesterday is the previous calendar date at 00:00.
    for (const ts of [
      new Date(2026, 2, 30, 0, 30).getTime(),
      new Date(2026, 10, 2, 0, 30).getTime(),
    ]) {
      const y = new Date(parseDate('yesterday', ts) ?? 0);
      const expected = new Date(ts);
      expected.setDate(expected.getDate() - 1);
      expect([y.getFullYear(), y.getMonth(), y.getDate(), y.getHours()]).toEqual([
        expected.getFullYear(),
        expected.getMonth(),
        expected.getDate(),
        0,
      ]);
    }
  });

  it('parses relative dates and rejects impossible calendar dates', () => {
    expect(parseDate('3d', NOW)).toBe(NOW - 3 * DAY);
    expect(parseDate('2w', NOW)).toBe(NOW - 14 * DAY);
    expect(parseDate('yesterday', NOW)).toBe(new Date(2026, 10, 6).getTime());
    expect(parseDate('2026-02-30', NOW)).toBeUndefined();
  });
});

describe('ArchiveSearch', () => {
  const stripe = item({
    url: 'https://stripe.com/pricing?plan=pro',
    title: 'Stripe — Compare plans',
  });
  const docs = item({
    url: 'https://docs.github.com/en/actions',
    title: 'GitHub Actions documentation',
    archivedAt: NOW - 10 * DAY,
    lastSweepId: 's2',
  });
  const linear = item({
    url: 'https://linear.app/team/cycle/42',
    title: 'Linear — Cycle 42',
    appId: 'com.brave.Browser',
    archivedAt: NOW - 2 * 3600_000,
    groupTitle: 'Planning',
  });
  const open = item({ url: 'https://open.com/', title: 'Still open', state: 'active' });
  const search = new ArchiveSearch([stripe, docs, linear, open]);

  const ids = (q: string) => search.search(q, NOW).items.map((i) => i.id);

  it('matches URL path words and prefixes', () => {
    expect(ids('pricing')).toEqual([stripe.id]);
    expect(ids('pric')).toEqual([stripe.id]);
    expect(ids('plan pro')).toEqual([stripe.id]);
  });

  it('tolerates typos in longer words', () => {
    expect(ids('documentaton')).toEqual([docs.id]);
  });

  it('tolerates swapped letters in longer words, and site: without a suffix', () => {
    expect(ids('stirpe')).toEqual([stripe.id]);
    expect(ids('site:github')).toEqual([docs.id]);
  });

  it('only searches archived items', () => {
    expect(ids('still')).toEqual([]);
    expect(search.size).toBe(3);
  });

  it('applies site:, app:, after: and before: filters', () => {
    expect(ids('site:github.com')).toEqual([docs.id]); // subdomains included
    expect(ids('app:brave')).toEqual([linear.id]);
    expect(new Set(ids('after:3d'))).toEqual(new Set([stripe.id, linear.id]));
    expect(ids('before:3d')).toEqual([docs.id]);
  });

  it('lists everything newest first without text', () => {
    expect(ids('')).toEqual([linear.id, stripe.id, docs.id]);
    expect(search.search('after:2026-01-01', NOW).items.map((i) => i.id)).toEqual([
      linear.id,
      stripe.id,
      docs.id,
    ]);
  });

  it('matches tab group names', () => {
    expect(ids('planning')).toEqual([linear.id]);
  });

  it('ranks recent items higher among equal matches', () => {
    const old = item({ url: 'https://a.com/report', title: 'Report', archivedAt: NOW - 60 * DAY });
    const fresh = item({ url: 'https://b.com/report', title: 'Report', archivedAt: NOW - DAY });
    const s = new ArchiveSearch([old, fresh]);
    expect(s.search('report', NOW).items.map((i) => i.id)).toEqual([fresh.id, old.id]);
  });

  it('stays fast at 20k items', () => {
    const many = Array.from({ length: 20_000 }, (_, i) =>
      item({ url: `https://site${i % 500}.com/page/${i}`, title: `Research note ${i}` }),
    );
    const s = new ArchiveSearch(many);
    const r = s.search('research note 1234', NOW);
    expect(r.items[0]?.title).toBe('Research note 1234');
    expect(r.tookMs).toBeLessThan(200); // budget 50 ms; slack for CI machines
  });
});

describe('groupItems', () => {
  it('groups by batch (sweep + original window), and names shared tab groups', () => {
    const a = item({ url: 'https://a.com/', title: 'A', groupTitle: 'Research' });
    const b = item({ url: 'https://b.com/', title: 'B', groupTitle: 'Research' });
    const c = item({ url: 'https://c.com/', title: 'C' });
    const windowOf = new Map([
      [a.id, '1'],
      [b.id, '1'],
      [c.id, '2'],
    ]);
    const groups = groupItems([a, c, b], NOW, windowOf);
    expect(groups.map((g) => g.items.map((i) => i.title))).toEqual([['A', 'B'], ['C']]);
    expect(groups[0]).toMatchObject({
      dayLabel: 'Yesterday',
      sourceLabel: '“Research” group · window 1',
    });
    expect(groups[1]?.sourceLabel).toMatch(/^Sweep at .* · window 2$/);
  });
});

describe('urlWords', () => {
  it('splits URLs into searchable words', () => {
    expect(urlWords('https://www.stripe.com/pricing?plan=pro#top')).toBe(
      'www stripe com pricing plan pro top',
    );
  });
});
