import { describe, expect, it } from 'vitest';
import type { Item } from '../db/types';
import { applyTick, DELETE_AFTER_MISSING_MS, reconcile } from './reconcile';
import { newItemFromTab } from './tabs';
import { makeTab } from './test-utils';

const groups = new Map();
function stored(overrides: Partial<Item>, tab = makeTab()): Item {
  return { ...newItemFromTab(tab, 'app', groups, 0), ...overrides };
}
const run = (live: Item[], tabs = [makeTab()], extra: { now?: number; missing?: Item[] } = {}) => {
  const now = extra.now ?? 1000;
  // `missing` items were already seen missing long enough ago to be deleted now.
  const missingSince = new Map(
    (extra.missing ?? []).map((i) => [i.id, now - DELETE_AFTER_MISSING_MS] as const),
  );
  return reconcile({ live, tabs, groups, appId: 'app', now, snapshotAt: now, missingSince });
};

describe('reconcile', () => {
  it('keeps an item when tab id and URL match', () => {
    const item = stored({ activeAgeS: 500 });
    const r = run([item], [makeTab({ title: 'Renamed' })]);
    expect(r.items).toHaveLength(1);
    expect(r.items[0]).toMatchObject({ id: item.id, activeAgeS: 500, title: 'Renamed' });
    expect(r.deleteIds).toEqual([]);
    expect(r.events).toEqual([]);
  });

  it('keeps ages across a browser restart (new tab ids, same URLs)', () => {
    const a = stored({ activeAgeS: 100, externalId: '1', url: 'https://a.com/' });
    const b = stored({ activeAgeS: 200, externalId: '2', url: 'https://b.com/' });
    const r = run(
      [a, b],
      [makeTab({ id: 1, url: 'https://b.com/' }), makeTab({ id: 2, url: 'https://a.com/' })],
    );
    expect(r.items.find((i) => i.url === 'https://a.com/')).toMatchObject({
      id: a.id,
      activeAgeS: 100,
      externalId: '2',
    });
    expect(r.items.find((i) => i.url === 'https://b.com/')).toMatchObject({
      id: b.id,
      externalId: '1',
    });
  });

  it('matches duplicate URLs one-to-one, most recently used first', () => {
    const older = stored({ externalId: '1', lastUsedAt: 10, activeAgeS: 900 });
    const newer = stored({ externalId: '2', lastUsedAt: 20, activeAgeS: 100 });
    const r = run([older, newer], [makeTab({ id: 50 })], { missing: [older] });
    expect(r.items.map((i) => i.id)).toEqual([newer.id]);
    expect(r.deleteIds).toEqual([older.id]);
  });

  it('creates new items with a seen event and deletes closed ones', () => {
    const gone = stored({ externalId: '9', url: 'https://gone.com/' });
    const r = run([gone], [makeTab({ id: 3, url: 'https://new.com/' })], { missing: [gone] });
    expect(r.deleteIds).toEqual([gone.id]);
    expect(r.items).toHaveLength(1);
    expect(r.events).toEqual([{ itemId: r.items[0]?.id, type: 'seen', at: 1000 }]);
  });

  it('waits before deleting an item whose tab is gone', () => {
    const gone = stored({ externalId: '9', url: 'https://gone.com/', firstSeenAt: 0 });
    const first = run([gone], []);
    expect(first.deleteIds).toEqual([]);
    expect(first.missingSince.get(gone.id)).toBe(1000);
    const soon = reconcile({
      live: [gone],
      tabs: [],
      groups,
      appId: 'app',
      now: 31_000,
      snapshotAt: 31_000,
      missingSince: first.missingSince,
    });
    expect(soon.deleteIds).toEqual([]);
    const later = reconcile({
      live: [gone],
      tabs: [],
      groups,
      appId: 'app',
      now: 61_000,
      snapshotAt: 61_000,
      missingSince: soon.missingSince,
    });
    expect(later.deleteIds).toEqual([gone.id]);
  });

  it('leaves tabs that handlers touched after the snapshot alone', () => {
    const navigated = stored({ externalId: '1', url: 'https://new.com/', activeAgeS: 120 });
    const r = reconcile({
      live: [navigated],
      tabs: [makeTab({ id: 1, url: 'https://old.com/' }), makeTab({ id: 2, url: 'https://x/' })],
      groups,
      appId: 'app',
      now: 1000,
      snapshotAt: 1000,
      skipTabIds: new Set(['1', '2']),
      missingSince: new Map([[navigated.id, 0]]),
    });
    expect(r.items).toEqual([]);
    expect(r.deleteIds).toEqual([]);
    expect(r.dirty.size).toBe(0);
    expect(navigated).toMatchObject({ url: 'https://new.com/', activeAgeS: 120 });
  });

  it('marks only changed items dirty', () => {
    const same = stored({});
    const r = run([same], [makeTab()]);
    expect(r.dirty.size).toBe(0);
    const renamed = run([same], [makeTab({ title: 'New title' })]);
    expect([...renamed.dirty]).toEqual([same]);
  });

  it('never deletes items being archived or newer than the snapshot', () => {
    const archiving = stored({ state: 'archiving', externalId: '5', url: 'https://x.com/' });
    const fresh = stored({ externalId: '6', url: 'https://y.com/', firstSeenAt: 1500 });
    // Even when marked missing long ago, neither is deleted, aged or tracked as missing.
    const r = run([archiving, fresh], [], { missing: [archiving, fresh] });
    expect(r.deleteIds).toEqual([]);
    expect(r.items).toEqual([]);
    expect(r.missingSince.size).toBe(0);
  });

  it('treats a reused tab id with a different URL as a different tab', () => {
    const old = stored({ externalId: '1', url: 'https://old.com/', activeAgeS: 999 });
    const r = run([old], [makeTab({ id: 1, url: 'https://other.com/' })], { missing: [old] });
    expect(r.deleteIds).toEqual([old.id]);
    expect(r.items[0]?.activeAgeS).toBe(0);
  });
});

describe('applyTick', () => {
  it('ages idle items and resets in-use ones', () => {
    const idle = stored({ externalId: '1', activeAgeS: 10 });
    const used = stored({ externalId: '2', activeAgeS: 10, state: 'stale' });
    const archiving = stored({ externalId: '3', activeAgeS: 10, state: 'archiving' });
    applyTick([idle, used, archiving], new Set(['2']), 30, 5000);
    expect(idle.activeAgeS).toBe(40);
    expect(used).toMatchObject({ activeAgeS: 0, state: 'active', lastUsedAt: 5000 });
    expect(archiving.activeAgeS).toBe(10);
  });

  it('changes nothing when there is no credit', () => {
    const item = stored({ externalId: '1', activeAgeS: 10 });
    const visible = stored({ externalId: '2', activeAgeS: 10 });
    applyTick([item, visible], new Set(['2']), 0, 5000);
    expect(item.activeAgeS).toBe(10);
    expect(visible.activeAgeS).toBe(10);
  });
});
