import { describe, expect, it } from 'vitest';
import type { PopupData } from '../popup/data';
import { appLastSweep, appSearchResult, fromApp, withAppArchive } from './archive';
import type { ArchivedItem, ArchiveSearchResult } from './protocol';

const NOW = new Date(2026, 10, 13, 15, 0).getTime();

const item = (id: string, over: Partial<ArchivedItem> = {}): ArchivedItem => ({
  id,
  kind: 'tab',
  appId: 'com.google.Chrome',
  title: `Tab ${id}`,
  url: `https://${id}.example/`,
  domain: `${id}.example`,
  pinned: false,
  state: 'archived',
  activeAgeS: 220_000,
  firstSeenAt: NOW - 9e6,
  lastUsedAt: NOW - 8e6,
  stateChangedAt: NOW - 1000,
  archivedAt: NOW - 1000,
  archiveReason: { type: 'unused', activeHours: 61, threshold: 60 },
  lastSweepId: 'sw1',
  windowKey: 'S1:1',
  restoreCount: 0,
  ...over,
});

describe('the app archive in the popup', () => {
  it('converts items without inventing fields', () => {
    const converted = fromApp(item('a', { groupTitle: 'Work', groupColor: 'blue', tabIndex: 3 }));
    expect(converted).toMatchObject({
      id: 'a',
      url: 'https://a.example/',
      groupTitle: 'Work',
      tabIndex: 3,
      lastSweepId: 'sw1',
      archiveReason: { type: 'unused', activeHours: 61, threshold: 60 },
    });
    expect('windowKey' in converted).toBe(false);
    expect('faviconUrl' in fromApp(item('b'))).toBe(false);
  });

  it('groups results by sweep and original window, in rank order', () => {
    const result: ArchiveSearchResult = {
      items: [item('a'), item('b', { windowKey: 'S1:2' }), item('c')],
      more: false,
    };
    const found = appSearchResult(result, NOW);
    expect(found.groups.map((g) => g.items.map((i) => i.id))).toEqual([['a', 'c'], ['b']]);
    expect(found.items.map((i) => i.id)).toEqual(['a', 'c', 'b']);
    expect(found.groups[0]?.sourceLabel).toMatch(/window 1/);
    expect(found.total).toBe(3);
  });

  it('offers Undo only for a finished app sweep', () => {
    expect(appLastSweep(undefined)).toBeNull();
    const done = appLastSweep({
      id: 'sw1',
      startedAt: NOW,
      finishedAt: NOW + 5,
      itemCount: 4,
      remaining: 3,
    });
    expect(done?.sweep.status).toBe('done');
    expect(done?.remaining).toBe(3);
    const running = appLastSweep({ id: 'sw2', startedAt: NOW, itemCount: 0, remaining: 2 });
    expect(running?.sweep.status).toBe('running');
  });

  it("replaces the local archive with the app's", () => {
    const local = {
      recent: [],
      archived: [],
      windowOf: new Map(),
      lastSweep: null,
    } as unknown as PopupData;
    const merged = withAppArchive(local, {
      items: ['a', 'b', 'c', 'd'].map((id) => item(id)),
      more: true,
      lastSweep: { id: 'sw1', startedAt: NOW, finishedAt: NOW, itemCount: 4, remaining: 4 },
      mode: 'automatic',
    });
    expect(merged.mode).toBe('automatic');
    expect(merged.recent.map((i) => i.id)).toEqual(['a', 'b', 'c']);
    expect(merged.archived).toHaveLength(4);
    expect(merged.windowOf.get('d')).toBe('S1:1');
    expect(merged.lastSweep?.sweep.id).toBe('sw1');
  });
});
