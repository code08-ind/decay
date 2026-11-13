import { describe, expect, it } from 'vitest';
import { DecayDB } from '../db/db';
import type { Item, Sweep } from '../db/types';
import { createSettingsStore } from '../settings/settings';
import { loadPopupData } from './data';

const NOW = new Date(2026, 10, 13, 15, 0).getTime();

const item = (id: string, over: Partial<Item> = {}): Item => ({
  id,
  kind: 'tab',
  appId: 'com.google.Chrome',
  title: `Tab ${id}`,
  url: `https://${id}.example/`,
  domain: `${id}.example`,
  pinned: false,
  state: 'archived',
  activeAgeS: 0,
  firstSeenAt: NOW - 9e6,
  lastUsedAt: NOW - 8e6,
  stateChangedAt: NOW - 1000,
  restoreCount: 0,
  ...over,
});

describe('loadPopupData', () => {
  it('lists the newest archived items, the last sweep and the current site', async () => {
    const db = new DecayDB(`popup-${Math.random()}`);
    const settings = createSettingsStore(db);
    await settings.update({ mode: 'automatic' });
    await db.items.bulkPut([
      item('a', { archivedAt: NOW - 5000, lastSweepId: 's1' }),
      item('b', { archivedAt: NOW - 1000, lastSweepId: 's1' }),
      item('c', { archivedAt: NOW - 3000 }),
      item('d', { archivedAt: NOW - 9000 }),
      item('live', { state: 'active', externalId: '4' }),
    ]);
    const sweep: Sweep = {
      id: 's1',
      startedAt: NOW - 1000,
      trigger: 'scheduled',
      isShadow: 0,
      status: 'done',
      itemCount: 3,
      failedCount: 0,
      items: [
        { itemId: 'a', action: 'archive', prevState: 'stale', result: 'ok', windowExternalId: '7' },
        { itemId: 'b', action: 'archive', prevState: 'stale', result: 'ok', windowExternalId: '7' },
      ],
    } as Sweep;
    await db.sweeps.put(sweep);
    const tabs = [
      {
        id: 4,
        windowId: 1,
        index: 0,
        url: 'https://zoom.us/j/1',
        pinned: false,
        active: true,
        incognito: false,
      },
    ];
    const data = await loadPopupData(db, await settings.getAll(), tabs, tabs[0], NOW);
    expect(data.mode).toBe('automatic');
    expect(data.openTabs).toBe(1);
    expect(data.recent.map((i) => i.id)).toEqual(['b', 'c', 'a']);
    expect(data.archived).toHaveLength(4);
    expect(data.windowOf.get('a')).toBe('7');
    expect(data.lastSweep?.sweep.id).toBe('s1');
    expect(data.lastSweep?.remaining).toBe(2);
    // zoom.us is protected by a default rule.
    expect(data.currentSite).toEqual({
      host: 'zoom.us',
      url: 'https://zoom.us/j/1',
      protected: true,
    });
  });

  it('has no current site for non-web pages and no last sweep before the first', async () => {
    const db = new DecayDB(`popup-${Math.random()}`);
    const settings = createSettingsStore(db);
    const tab = {
      id: 1,
      windowId: 1,
      index: 0,
      url: 'chrome://settings',
      pinned: false,
      active: true,
      incognito: false,
    };
    const data = await loadPopupData(db, await settings.getAll(), [tab], tab, NOW);
    expect(data.currentSite).toBeNull();
    expect(data.lastSweep).toBeNull();
    expect(data.recent).toEqual([]);
  });
});
