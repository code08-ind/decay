import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { DecayDB } from '../db/db';
import type { IdleState } from './clock';
import type { GroupInfo, TabInfo, WindowInfo } from './tabs';
import { makeTab } from './test-utils';
import { type BrowserApi, Tracker } from './tracker';

class FakeBrowser implements BrowserApi {
  tabs: TabInfo[] = [];
  windows: WindowInfo[] = [{ id: 1, focused: true, incognito: false, state: 'normal' }];
  groups: GroupInfo[] = [];
  idle: IdleState = 'active';
  lastTickAt: number | null = null;
  async queryTabs() {
    return structuredClone(this.tabs);
  }
  async getTab(id: number) {
    return structuredClone(this.tabs.find((t) => t.id === id));
  }
  async getActiveTab(windowId: number) {
    return structuredClone(this.tabs.find((t) => t.windowId === windowId && t.active));
  }
  async getWindows() {
    return this.windows;
  }
  async getGroups() {
    return this.groups;
  }
  async queryIdle() {
    return this.idle;
  }
  async getLastTickAt() {
    return this.lastTickAt;
  }
  async setLastTickAt(at: number) {
    this.lastTickAt = at;
  }
}

let n = 0;
let db: DecayDB;
let fake: FakeBrowser;
let clock: number;
let tracker: Tracker;

beforeEach(() => {
  db = new DecayDB(`decay-tracker-test-${n++}`);
  fake = new FakeBrowser();
  clock = 1_000_000;
  tracker = new Tracker({
    db,
    api: fake,
    appId: 'com.google.Chrome',
    getIdleCutoffS: async () => 300,
    now: () => clock,
    activationSettleMs: 0,
  });
});
afterEach(() => db.delete());

const ageOf = async (tabId: number) =>
  (await db.findLiveByExternalId('com.google.Chrome', String(tabId)))?.activeAgeS;

async function tickAfter(ms: number) {
  clock += ms;
  return tracker.tick();
}

describe('Tracker', () => {
  it('ages background tabs only while the user is active', async () => {
    fake.tabs = [makeTab({ id: 1, active: true }), makeTab({ id: 2, url: 'https://b.com/' })];
    expect((await tracker.tick()).creditS).toBe(0); // first tick: no credit
    await tickAfter(30_000);
    await tickAfter(30_000);
    expect(await ageOf(1)).toBe(0);
    expect(await ageOf(2)).toBe(60);

    fake.idle = 'idle';
    await tickAfter(30_000);
    fake.idle = 'locked';
    await tickAfter(30_000);
    expect(await ageOf(2)).toBe(60);

    fake.idle = 'active';
    await tickAfter(8 * 3600_000); // overnight sleep: no back-credit
    expect(await ageOf(2)).toBe(60);
    await tickAfter(30_000);
    expect(await ageOf(2)).toBe(90);
  });

  it('does not double-credit overlapping ticks', async () => {
    fake.tabs = [makeTab({ id: 2 })];
    await tracker.tick();
    clock += 30_000;
    await Promise.all([tracker.tick(), tracker.tick()]);
    expect(await ageOf(2)).toBe(30);
  });

  it('resets age when a tab is activated or its window focused', async () => {
    fake.tabs = [makeTab({ id: 1, active: true }), makeTab({ id: 2, url: 'https://b.com/' })];
    await tracker.tick();
    await tickAfter(30_000);
    expect(await ageOf(2)).toBe(30);
    fake.tabs = [makeTab({ id: 1 }), makeTab({ id: 2, url: 'https://b.com/', active: true })];
    await tracker.tabUsed(2);
    expect(await ageOf(2)).toBe(0);

    fake.windows.push({ id: 2, focused: false, incognito: false, state: 'normal' });
    fake.tabs.push(makeTab({ id: 3, windowId: 2, active: true, url: 'https://c.com/' }));
    await tickAfter(30_000);
    const third = await db.findLiveByExternalId('com.google.Chrome', '3');
    await db.items.update(third?.id ?? '', { activeAgeS: 500 });
    expect(await ageOf(3)).toBe(500);
    await tracker.windowFocused(2);
    expect(await ageOf(3)).toBe(0);
    await tracker.windowFocused(-1); // browser lost focus: no-op
  });

  it('forgets tabs the user closes, together with their events', async () => {
    fake.tabs = [makeTab({ id: 1 }), makeTab({ id: 2, url: 'https://b.com/' })];
    await tracker.tick();
    expect(await db.events.count()).toBe(2);
    await tracker.tabRemoved(1, false);
    expect(await ageOf(1)).toBeUndefined();
    expect(await db.events.count()).toBe(1);
  });

  it("deletes a closed window's tabs only after the grace period", async () => {
    fake.tabs = [makeTab({ id: 1 }), makeTab({ id: 2, url: 'https://b.com/' })];
    await tracker.tick();
    await tracker.tabRemoved(2, true);
    fake.tabs = [makeTab({ id: 1 })];
    await tickAfter(30_000);
    expect(await db.items.count()).toBe(2); // first seen missing
    await tickAfter(30_000);
    expect(await db.items.count()).toBe(2); // 30 s missing
    await tickAfter(30_000);
    expect(await db.items.count()).toBe(1); // 60 s missing → deleted
    expect(await db.events.count()).toBe(1);
  });

  it('a reopened window reclaims its items and ages', async () => {
    fake.tabs = [
      makeTab({ id: 1, url: 'https://x.com/' }),
      makeTab({ id: 2, url: 'https://y.com/' }),
    ];
    await tracker.tick();
    await tickAfter(30_000);
    await tickAfter(30_000);
    const before = await db.items.toArray();
    await tracker.tabRemoved(1, true);
    await tracker.tabRemoved(2, true);
    // "Reopen closed window" before the next tick: new tab ids, onCreated first.
    fake.tabs = [
      makeTab({ id: 11, url: 'https://x.com/' }),
      makeTab({ id: 12, url: 'https://y.com/' }),
    ];
    for (const tab of fake.tabs) await tracker.upsertTab(tab);
    await tickAfter(30_000);
    const after = await db.items.toArray();
    expect(after).toHaveLength(2);
    expect(after.map((i) => i.id).sort()).toEqual(before.map((i) => i.id).sort());
    expect(after.every((i) => i.activeAgeS === 90)).toBe(true);
  });

  it('a tick does not undo a navigation that happened after its snapshot', async () => {
    fake.tabs = [makeTab({ id: 1, url: 'https://old.com/' })];
    await tracker.tick();
    await tickAfter(30_000);
    // Simulate: the tick snapshots tabs, then the tab navigates before the tick writes.
    const realQuery = fake.queryTabs.bind(fake);
    fake.queryTabs = async () => {
      const snapshot = await realQuery();
      fake.tabs = [makeTab({ id: 1, url: 'https://new.com/' })];
      await tracker.upsertTab(makeTab({ id: 1, url: 'https://new.com/' }));
      return snapshot;
    };
    await tickAfter(30_000);
    fake.queryTabs = realQuery;
    let items = await db.items.toArray();
    expect(items).toHaveLength(1);
    expect(items[0]).toMatchObject({ url: 'https://new.com/', activeAgeS: 30 });
    await tickAfter(30_000);
    items = await db.items.toArray();
    expect(items).toHaveLength(1);
    expect(items[0]).toMatchObject({ url: 'https://new.com/', activeAgeS: 60 });
  });

  it('a tick does not resurrect a tab closed after its snapshot', async () => {
    fake.tabs = [makeTab({ id: 1 })];
    await tracker.tick();
    const realQuery = fake.queryTabs.bind(fake);
    fake.queryTabs = async () => {
      const snapshot = await realQuery();
      fake.tabs = [];
      await tracker.tabRemoved(1, false);
      return snapshot;
    };
    await tickAfter(30_000);
    fake.queryTabs = realQuery;
    expect(await db.items.count()).toBe(0);
    expect(await db.events.count()).toBe(0);
  });

  it('samples used events every 10 minutes of repeated use', async () => {
    fake.tabs = [makeTab({ id: 1, active: true })];
    await tracker.tick();
    for (let i = 0; i < 10; i++) {
      clock += 9 * 60_000;
      await tracker.tabUsed(1); // tab 1 is the active tab
    }
    // 90 min of use → one event at the start and then at most one per 10 min.
    const used = await db.events.where({ type: 'used' }).count();
    expect(used).toBeGreaterThanOrEqual(5);
    expect(used).toBeLessThanOrEqual(10);
  });

  it('ignores activations that do not last (window restore flicker)', async () => {
    fake.tabs = [makeTab({ id: 1, active: true }), makeTab({ id: 2, url: 'https://b.com/' })];
    await tracker.tick();
    await tickAfter(30_000);
    await tracker.tabUsed(2); // onActivated fired, but tab 2 is no longer active
    expect(await ageOf(2)).toBe(30);
  });

  it('does not revive a tab whose create handler finishes after its removal', async () => {
    const tab = makeTab({ id: 8, url: 'https://fast.com/' });
    fake.tabs = [];
    await tracker.tabRemoved(8, false);
    await tracker.upsertTab(tab); // late onCreated
    expect(await db.items.count()).toBe(0);
  });

  it('skips writes when nothing changed', async () => {
    fake.tabs = [makeTab({ id: 1 }), makeTab({ id: 2, url: 'https://b.com/' })];
    await tracker.tick();
    fake.idle = 'idle';
    const r = await tickAfter(30_000);
    expect(r).toMatchObject({ creditS: 0, written: 0 });
  });

  it('keeps ages and records when the browser restarts', async () => {
    fake.tabs = [makeTab({ id: 1, url: 'https://a.com/' })];
    await tracker.tick();
    await tickAfter(30_000);
    const before = await db.items.toArray();
    // Restart: new tab ids, session storage cleared.
    fake.tabs = [makeTab({ id: 77, url: 'https://a.com/' })];
    fake.lastTickAt = null;
    await tickAfter(60_000);
    const after = await db.items.toArray();
    expect(after).toHaveLength(1);
    expect(after[0]).toMatchObject({ id: before[0]?.id, externalId: '77', activeAgeS: 30 });
  });

  it('adds new tabs on create and updates them in place', async () => {
    const tab = makeTab({ id: 4, url: 'https://new.com/' });
    fake.tabs = [tab];
    await tracker.upsertTab(tab);
    await tracker.upsertTab({ ...tab, title: 'Renamed', pinned: true });
    const items = await db.items.toArray();
    expect(items).toHaveLength(1);
    expect(items[0]).toMatchObject({ title: 'Renamed', pinned: true, domain: 'new.com' });
    expect(await db.events.where({ type: 'seen' }).count()).toBe(1);
    await tracker.tick();
    expect(await db.items.count()).toBe(1);
  });

  it('ignores incognito tabs', async () => {
    const tab = makeTab({ id: 5, incognito: true });
    fake.tabs = [tab];
    await tracker.upsertTab(tab);
    await tracker.tick();
    expect(await db.items.count()).toBe(0);
  });

  it('leaves items being archived alone', async () => {
    fake.tabs = [makeTab({ id: 1 })];
    await tracker.tick();
    const [item] = await db.items.toArray();
    await db.items.update(item?.id ?? '', { state: 'archiving' });
    await tracker.tabRemoved(1, false);
    fake.tabs = [];
    for (let i = 0; i < 4; i++) await tickAfter(30_000);
    expect(await db.items.count()).toBe(1);
  });
});
