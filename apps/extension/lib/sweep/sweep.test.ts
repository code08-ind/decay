import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { DecayDB } from '../db/db';
import type { Item, Mode, Sweep } from '../db/types';
import type { TabInfo } from '../tracker/tabs';
import { FakeBrowser, makeTab } from '../tracker/test-utils';
import { Tracker } from '../tracker/tracker';
import {
  closeTabs,
  QUIET_MS,
  RESTORE_GRACE_MS,
  SWEEP_INTERVAL_MS,
  type TabCommands,
  UNDO_HISTORY,
} from './executor';
import { describeSweep, loadSweepHistory } from './history';
import { UndoError } from './restore';

const APP = 'com.google.Chrome';
const H = 3600;

/** Tab commands acting on the fake browser's tab list. */
class FakeCommands implements TabCommands {
  nextTabId = 100;
  nextWindowId = 50;
  session = 's1';
  removed: number[] = [];
  createdWindows: string[] = [];
  groups: { id: number; tabIds: number[]; windowId: number; title?: string; color?: string }[] = [];
  onRemove?: (tabId: number) => Promise<void> | void;
  failCreate = new Set<string>();
  constructor(private readonly fake: FakeBrowser) {}
  async removeTab(tabId: number) {
    await this.onRemove?.(tabId);
    this.fake.tabs = this.fake.tabs.filter((t) => t.id !== tabId);
    this.removed.push(tabId);
  }
  async createTab(opts: { windowId: number; url: string; index?: number; active?: boolean }) {
    if (this.failCreate.has(opts.url)) throw new Error('create failed');
    const tab = makeTab({
      id: this.nextTabId++,
      windowId: opts.windowId,
      url: opts.url,
      index: opts.index ?? this.fake.tabs.length,
      active: opts.active ?? false,
      title: '',
      status: 'loading',
    });
    this.fake.tabs.push(tab);
    return structuredClone(tab);
  }
  async createWindow(url: string) {
    if (this.failCreate.has(url)) throw new Error('create failed');
    const windowId = this.nextWindowId++;
    this.fake.windows.push({ id: windowId, focused: false, incognito: false, state: 'normal' });
    this.createdWindows.push(url);
    const tab = await this.createTab({ windowId, url, index: 0 });
    return { windowId, tab };
  }
  async windowExists(windowId: number) {
    return this.fake.windows.some((w) => w.id === windowId);
  }
  async groupTabs(tabIds: number[], windowId: number) {
    const id = 900 + this.groups.length;
    this.groups.push({ id, tabIds, windowId });
    return id;
  }
  async updateGroup(groupId: number, props: { title?: string; color?: string }) {
    const g = this.groups.find((x) => x.id === groupId);
    if (g) Object.assign(g, props);
  }
  sessionStartedAt = 0;
  async browserSession() {
    return { id: this.session, startedAt: this.sessionStartedAt };
  }
}

let n = 0;
let db: DecayDB;
let fake: FakeBrowser;
let commands: FakeCommands;
let clock: number;
let mode: Mode;
let holdUntil: number | null;
let maxItems: number;
let tracker: Tracker;

beforeEach(() => {
  db = new DecayDB(`decay-sweep-test-${n++}`);
  fake = new FakeBrowser();
  commands = new FakeCommands(fake);
  clock = 10_000_000;
  mode = 'automatic';
  holdUntil = null;
  maxItems = 50;
  tracker = new Tracker({
    db,
    api: fake,
    appId: APP,
    commands,
    getIdleCutoffS: async () => 300,
    getPolicy: async () => ({
      mode,
      holdUntil,
      thresholds: { stale: 12, hidden: 36, archive: 60 },
      maxItemsPerSweep: maxItems,
    }),
    now: () => clock,
    activationSettleMs: 0,
  });
});
afterEach(() => db.delete());

const live = (tabId: number) => db.findLiveByExternalId(APP, String(tabId));

/** Opens the tabs, records them, and ages the given tab ids past the archive threshold. */
async function setup(tabs: TabInfo[], old: Record<number, number>) {
  fake.tabs = tabs;
  await tracker.tick();
  for (const [tabId, hours] of Object.entries(old)) {
    const item = await live(Number(tabId));
    await db.items.update(item?.id ?? '', { activeAgeS: hours * H });
  }
}

async function tickAfter(ms: number) {
  clock += ms;
  return tracker.tick();
}

const realSweeps = () => db.sweeps.where('isShadow').equals(0).toArray();

describe('sweep executor', () => {
  it('commits the archive record before closing, then archives the tab', async () => {
    await setup(
      [
        makeTab({ id: 1, active: true }),
        makeTab({ id: 2, url: 'https://old.com/a', title: 'Old', index: 1, groupId: 7 }),
      ],
      { 2: 130 },
    );
    fake.groups = [{ id: 7, title: 'Research', color: 'blue' }];
    let seenAtClose: { item?: Item; sweep?: Sweep } = {};
    commands.onRemove = async () => {
      const [item] = await db.items.where('state').equals('archiving').toArray();
      const [sweep] = await realSweeps();
      seenAtClose = { item, sweep };
    };
    const r = await tickAfter(30_000);

    expect(commands.removed).toEqual([2]);
    expect(seenAtClose.item?.url).toBe('https://old.com/a');
    expect(seenAtClose.sweep?.status).toBe('running');
    expect(seenAtClose.sweep?.items[0]).toMatchObject({ result: 'pending', windowExternalId: '1' });

    expect(r.sweep).toMatchObject({ archived: 1, failed: 0 });
    const [item] = await db.items.where('state').equals('archived').toArray();
    expect(item).toMatchObject({ url: 'https://old.com/a', lastSweepId: r.sweep?.id });
    expect(item?.archivedAt).toBe(clock);
    expect(item?.externalId).toBeUndefined();
    expect(item?.archiveReason).toMatchObject({ type: 'unused', threshold: 120 }); // 2× until the form probe;
    const [sweep] = await realSweeps();
    expect(sweep).toMatchObject({ status: 'done', itemCount: 1, failedCount: 0 });
    expect(sweep?.items[0]?.result).toBe('ok');
    expect(await db.events.where('type').equals('archived').count()).toBe(1);
    expect((await db.metrics.toArray())[0]?.archivedCount).toBe(1);
  });

  it('runs no real sweep in shadow or paused mode, or during a hold', async () => {
    await setup([makeTab({ id: 2, url: 'https://old.com/' })], { 2: 130 });
    for (const m of ['shadow', 'paused'] as const) {
      mode = m;
      await tickAfter(SWEEP_INTERVAL_MS);
    }
    mode = 'automatic';
    holdUntil = clock + 10 * SWEEP_INTERVAL_MS;
    await tickAfter(SWEEP_INTERVAL_MS);
    expect(commands.removed).toEqual([]);
    expect(await realSweeps()).toHaveLength(0);
  });

  it('caps a sweep at maxItemsPerSweep, oldest first, and waits 15 min between sweeps', async () => {
    maxItems = 2;
    await setup(
      [1, 2, 3].map((id) => makeTab({ id, url: `https://s${id}.com/` })),
      { 1: 121, 2: 160, 3: 140 },
    );
    await tickAfter(30_000);
    expect(commands.removed.sort()).toEqual([2, 3]);
    await tickAfter(30_000);
    expect(commands.removed).toHaveLength(2);
    await tickAfter(SWEEP_INTERVAL_MS);
    expect(commands.removed).toContain(1);
  });

  it('waits for a quiet moment after the user switches tabs', async () => {
    await setup([makeTab({ id: 1, active: true }), makeTab({ id: 2, url: 'https://old.com/' })], {
      2: 130,
    });
    await tracker.tabUsed(1);
    await tickAfter(QUIET_MS - 1000);
    expect(commands.removed).toEqual([]);
    await tickAfter(2000);
    expect(commands.removed).toEqual([2]);
  });

  it('"Sweep now" ignores the interval and the quiet moment', async () => {
    await setup([makeTab({ id: 1, active: true })], {});
    await tickAfter(30_000); // nothing to do: no sweep record
    fake.tabs.push(makeTab({ id: 2, url: 'https://old.com/' }));
    await tickAfter(30_000);
    await db.items.update((await live(2))?.id ?? '', { activeAgeS: 130 * H });
    await tracker.tabUsed(1);
    const r = await tracker.sweepNow();
    expect(r.sweep?.archived).toBe(1);
    expect((await realSweeps())[0]?.trigger).toBe('manual');
  });

  it('keeps a tab that became active, pinned or audible before the close', async () => {
    await setup([makeTab({ id: 2, url: 'https://old.com/' })], { 2: 130 });
    const realGet = fake.getTab.bind(fake);
    fake.getTab = async (id) => {
      const tab = await realGet(id);
      return tab ? { ...tab, active: true } : tab;
    };
    const r = await tickAfter(30_000);
    expect(r.sweep).toMatchObject({ archived: 0, failed: 1 });
    expect(commands.removed).toEqual([]);
    const item = await live(2);
    expect(item?.state).toBe('stale');
    const [sweep] = await realSweeps();
    expect(sweep?.items[0]).toMatchObject({ result: 'skipped', skipReason: 'inUse' });
    expect(await db.events.where('type').equals('close_failed').count()).toBe(1);
  });

  it('cancels the closes when Decay is paused between planning and closing', async () => {
    await setup([makeTab({ id: 2, url: 'https://old.com/' })], { 2: 130 });
    let calls = 0;
    tracker = new Tracker({
      db,
      api: fake,
      appId: APP,
      commands,
      getIdleCutoffS: async () => 300,
      getPolicy: async () => ({
        mode: calls++ === 0 ? 'automatic' : 'paused',
        holdUntil: null,
        thresholds: { stale: 12, hidden: 36, archive: 60 },
      }),
      now: () => clock,
      activationSettleMs: 0,
    });
    await tickAfter(30_000);
    expect(commands.removed).toEqual([]);
    expect((await realSweeps())[0]?.items[0]?.skipReason).toBe('modeChanged');
    expect((await live(2))?.state).toBe('stale');
  });

  it('settles a sweep interrupted by a worker crash: open tabs revert, gone tabs archive', async () => {
    await setup(
      [makeTab({ id: 1, url: 'https://a.com/' }), makeTab({ id: 2, url: 'https://b.com/' })],
      {},
    );
    const a = await live(1);
    const b = await live(2);
    const sweep: Sweep = {
      id: 'crashed',
      startedAt: clock,
      trigger: 'scheduled',
      isShadow: 0,
      status: 'running',
      itemCount: 2,
      failedCount: 0,
      items: [a, b].map((i) => ({
        itemId: i?.id ?? '',
        action: 'archive' as const,
        prevState: 'active' as const,
        result: 'pending' as const,
      })),
    };
    await db.sweeps.add(sweep);
    for (const i of [a, b]) {
      await db.items.update(i?.id ?? '', { state: 'archiving', lastSweepId: 'crashed' });
    }
    fake.tabs = fake.tabs.filter((t) => t.id !== 2); // closed before the crash
    await tickAfter(30_000);
    expect((await db.items.get(a?.id ?? ''))?.state).toBe('active');
    expect((await db.items.get(b?.id ?? ''))?.state).toBe('archived');
    expect(await db.sweeps.get('crashed')).toMatchObject({ status: 'done', itemCount: 1 });
  });
});

describe('sweep executor: unknown state and races (review fixes)', () => {
  it('keeps a timed-out close `archiving` and settles it at the next tick', async () => {
    await setup([makeTab({ id: 2, url: 'https://old.com/' })], { 2: 130 });
    let finishRemove: () => void = () => {};
    commands.removeTab = (tabId) =>
      new Promise<void>((resolve) => {
        finishRemove = () => {
          fake.tabs = fake.tabs.filter((t) => t.id !== tabId);
          resolve();
        };
      });
    tracker = new Tracker({
      db,
      api: fake,
      appId: APP,
      commands,
      closeTimeoutMs: 10,
      getIdleCutoffS: async () => 300,
      getPolicy: async () => ({
        mode,
        holdUntil,
        thresholds: { stale: 12, hidden: 36, archive: 60 },
      }),
      now: () => clock,
      activationSettleMs: 0,
    });
    const r = await tickAfter(30_000);
    expect(r.sweep).toMatchObject({ archived: 0, pending: 1 });
    expect((await db.items.toArray())[0]?.state).toBe('archiving');
    expect((await realSweeps())[0]?.status).toBe('running');

    // The close completes late; the browser's onRemoved must not delete the record.
    finishRemove();
    await tracker.tabRemoved(2, false, 1);
    expect((await db.items.toArray())[0]?.state).toBe('archiving');

    await tickAfter(30_000);
    expect((await db.items.toArray())[0]?.state).toBe('archived');
    expect((await realSweeps())[0]).toMatchObject({ status: 'done', itemCount: 1 });
  });

  it('closes nothing and archives nothing when the tab state is unknown', async () => {
    await setup([makeTab({ id: 2, url: 'https://old.com/' })], { 2: 130 });
    const realGet = fake.getTab.bind(fake);
    let calls = 0;
    fake.getTab = async (id) => {
      if (calls++ === 0) throw new Error('Browser is shutting down');
      return realGet(id);
    };
    await tickAfter(30_000);
    expect(commands.removed).toEqual([]);
    expect((await live(2))?.state).toBe('stale');
    expect((await realSweeps())[0]?.items[0]?.skipReason).toBe('error');
  });

  it('drops the record when the user closed the tab before the sweep did', async () => {
    await setup([makeTab({ id: 2, url: 'https://old.com/' })], { 2: 130 });
    commands.onRemove = undefined;
    const realGet = fake.getTab.bind(fake);
    fake.getTab = async (id) => {
      fake.tabs = fake.tabs.filter((t) => t.id !== id); // user closes it right now
      return realGet(id);
    };
    const r = await tickAfter(30_000);
    expect(r.sweep).toMatchObject({ archived: 0, failed: 1 });
    expect(await db.items.count()).toBe(0);
    expect((await realSweeps())[0]?.items[0]?.skipReason).toBe('closedByUser');
  });

  it('keeps a tab the user switched to while the sweep was running, and resets its age', async () => {
    await setup([makeTab({ id: 1, active: true }), makeTab({ id: 2, url: 'https://old.com/' })], {
      2: 130,
    });
    // The user activates tab 2 and moves on, while it's `archiving` (handlers ignore it).
    const realGet = fake.getTab.bind(fake);
    let used = false;
    fake.getTab = async (id) => {
      if (id === 2 && !used) {
        used = true;
        await tracker.tabUsed(2);
      }
      return realGet(id);
    };
    const r = await tickAfter(30_000);
    expect(commands.removed).toEqual([]);
    expect(r.sweep).toMatchObject({ archived: 0, failed: 1 });
    const item = await live(2);
    expect(item).toMatchObject({ state: 'active', activeAgeS: 0 });
  });

  it('waits for session restore before settling a sweep from an earlier browser session', async () => {
    await setup([makeTab({ id: 1, url: 'https://a.com/' })], {});
    const a = await live(1);
    await db.sweeps.add({
      id: 'old-session',
      startedAt: clock,
      trigger: 'scheduled',
      isShadow: 0,
      status: 'running',
      itemCount: 1,
      failedCount: 0,
      browserSession: 's0',
      items: [{ itemId: a?.id ?? '', action: 'archive', prevState: 'active', result: 'pending' }],
    });
    await db.items.update(a?.id ?? '', { state: 'archiving', lastSweepId: 'old-session' });
    // Browser restarted: no tabs restored yet.
    commands.sessionStartedAt = clock;
    fake.tabs = [];
    await tickAfter(30_000);
    expect((await db.items.get(a?.id ?? ''))?.state).toBe('archiving');
    // Session restore brings the tab back with a new id.
    fake.tabs = [makeTab({ id: 41, url: 'https://a.com/' })];
    await tickAfter(30_000);
    expect((await db.items.get(a?.id ?? ''))?.state).toBe('archiving');
    clock += 2 * 60_000;
    await tracker.tick();
    expect((await db.items.get(a?.id ?? ''))?.state).toBe('active');
    expect(await db.items.where('state').equals('archived').count()).toBe(0);
  });
});

describe("Decay's own pages", () => {
  it('are never tracked, so a sweep cannot close the page that started it', async () => {
    const own = 'chrome-extension://decayid/';
    tracker = new Tracker({
      db,
      api: fake,
      appId: APP,
      commands,
      ownOrigin: own,
      getIdleCutoffS: async () => 300,
      getPolicy: async () => ({
        mode,
        holdUntil,
        thresholds: { stale: 12, hidden: 36, archive: 60 },
      }),
      now: () => clock,
      activationSettleMs: 0,
    });
    fake.tabs = [makeTab({ id: 5, url: `${own}sweeps.html` })];
    await tracker.tick();
    await tracker.upsertTab(makeTab({ id: 5, url: `${own}sweeps.html` }));
    expect(await db.items.count()).toBe(0);
  });
});

describe('closeTabs', () => {
  const item = (over: Partial<Item> = {}) =>
    ({ id: 'i1', externalId: '5', url: 'https://x.com/', ...over }) as Item;

  it('treats a failed close of a tab that is gone as success', async () => {
    const c = new FakeCommands(new FakeBrowser());
    c.removeTab = async () => {
      throw new Error('No tab with id');
    };
    let calls = 0;
    const getTab = async () =>
      calls++ === 0 ? makeTab({ id: 5, url: 'https://x.com/' }) : undefined;
    const out = await closeTabs([item()], getTab, c);
    expect(out.get('i1')?.result).toBe('ok');
  });

  it('fails a close that times out while the tab is still open', async () => {
    const c = new FakeCommands(new FakeBrowser());
    c.removeTab = () => new Promise(() => {});
    const getTab = async () => makeTab({ id: 5, url: 'https://x.com/' });
    const out = await closeTabs([item()], getTab, c, { timeoutMs: 10 });
    // Unknown: the close may still complete, so recovery settles it at the next tick.
    expect(out.get('i1')).toEqual({ result: 'pending' });
  });

  it('skips a tab that navigated to another page', async () => {
    const c = new FakeCommands(new FakeBrowser());
    const getTab = async () => makeTab({ id: 5, url: 'https://elsewhere.com/' });
    const out = await closeTabs([item()], getTab, c);
    expect(out.get('i1')).toEqual({ result: 'skipped', skipReason: 'changed' });
  });
});

describe('undo', () => {
  async function sweepOf(tabs: TabInfo[]) {
    const old = Object.fromEntries(tabs.map((t) => [t.id, 130]));
    await setup([makeTab({ id: 1, active: true, url: 'https://keep.com/' }), ...tabs], old);
    const r = await tickAfter(30_000);
    return r.sweep?.id ?? '';
  }

  it('reopens tabs in their window and position, with grace protection', async () => {
    const sweepId = await sweepOf([
      makeTab({ id: 2, url: 'https://a.com/', title: 'A', index: 1 }),
      makeTab({ id: 3, url: 'https://b.com/', title: 'B', index: 2 }),
    ]);
    clock += 60_000;
    const r = await tracker.undoSweep();
    expect(r).toMatchObject({ restored: 2, failed: 0 });
    expect(commands.createdWindows).toEqual([]);
    const reopened = fake.tabs.filter((t) => (t.id ?? 0) >= 100);
    expect(reopened.map((t) => [t.url, t.windowId, t.index, t.active])).toEqual([
      ['https://a.com/', 1, 1, false],
      ['https://b.com/', 1, 2, false],
    ]);

    const records = await db.items.where('lastSweepId').equals(sweepId).toArray();
    expect(records.map((i) => i.state)).toEqual(['restored', 'restored']);
    const restoredLive = await live(reopened[0]?.id ?? 0);
    expect(restoredLive).toMatchObject({
      state: 'active',
      title: 'A',
      restoredFromId: records.find((i) => i.url === 'https://a.com/')?.id,
      graceUntil: clock + RESTORE_GRACE_MS,
    });
    expect(await db.sweeps.get(sweepId)).toMatchObject({ status: 'undone', undoneAt: clock });
    expect(await db.events.where('type').equals('undo').count()).toBe(2);
    const metrics = (await db.metrics.toArray())[0];
    expect(metrics).toMatchObject({ undoCount: 1, restored24hCount: 2 });

    // The next ticks keep one live item per tab and don't archive them again (grace).
    await tickAfter(SWEEP_INTERVAL_MS);
    expect(commands.removed).toEqual([2, 3]);
    const liveItems = await db.items.where('state').anyOf(['active', 'stale']).toArray();
    expect(liveItems.filter((i) => i.url === 'https://a.com/')).toHaveLength(1);
  });

  it('does not duplicate the live item when the tab-created handler ran first', async () => {
    await sweepOf([makeTab({ id: 2, url: 'https://a.com/' })]);
    const createTab = commands.createTab.bind(commands);
    commands.createTab = async (opts) => {
      const tab = await createTab(opts);
      await tracker.upsertTab(tab); // tabs.onCreated
      return tab;
    };
    await tracker.undoSweep();
    const items = await db.items.where('state').anyOf(['active', 'stale']).toArray();
    expect(items.filter((i) => i.url === 'https://a.com/')).toHaveLength(1);
    expect(items.find((i) => i.url === 'https://a.com/')?.restoredFromId).toBeDefined();
  });

  it('opens one new window per closed window and rebuilds tab groups', async () => {
    fake.windows.push({ id: 2, focused: false, incognito: false, state: 'normal' });
    fake.groups = [{ id: 7, title: 'Research', color: 'blue' }];
    await sweepOf([
      makeTab({ id: 2, url: 'https://a.com/', windowId: 2, index: 0, groupId: 7 }),
      makeTab({ id: 3, url: 'https://b.com/', windowId: 2, index: 1, groupId: 7 }),
      makeTab({ id: 4, url: 'https://c.com/', windowId: 1, index: 1 }),
    ]);
    fake.windows = fake.windows.filter((w) => w.id !== 2); // window 2 was closed
    await tracker.undoSweep();
    expect(commands.createdWindows).toEqual(['https://a.com/']);
    const b = fake.tabs.find((t) => t.url === 'https://b.com/');
    const a = fake.tabs.find((t) => t.url === 'https://a.com/');
    expect(b?.windowId).toBe(a?.windowId);
    expect(fake.tabs.find((t) => t.url === 'https://c.com/')?.windowId).toBe(1);
    expect(commands.groups).toEqual([
      { id: 900, tabIds: [a?.id, b?.id], windowId: a?.windowId, title: 'Research', color: 'blue' },
    ]);
  });

  it('uses a new window after a browser restart, even if the old window id exists', async () => {
    await sweepOf([makeTab({ id: 2, url: 'https://a.com/' })]);
    commands.session = 's2';
    await tracker.undoSweep();
    expect(commands.createdWindows).toEqual(['https://a.com/']);
  });

  it('does not reopen tabs twice after an interrupted undo', async () => {
    const sweepId = await sweepOf([
      makeTab({ id: 2, url: 'https://a.com/' }),
      makeTab({ id: 3, url: 'https://b.com/' }),
    ]);
    // A previous attempt opened a.com, then the worker died before recording it.
    await db.sweeps.update(sweepId, { undoStartedAt: clock });
    await commands.createTab({ windowId: 1, url: 'https://a.com/' });
    const r = await tracker.undoSweep(sweepId);
    expect(r).toMatchObject({ restored: 2, failed: 0 });
    expect(fake.tabs.filter((t) => t.url === 'https://a.com/')).toHaveLength(1);
    expect(fake.tabs.filter((t) => t.url === 'https://b.com/')).toHaveLength(1);
    expect(await db.sweeps.get(sweepId)).toMatchObject({ status: 'undone' });
    expect((await db.sweeps.get(sweepId))?.undoStartedAt).toBeUndefined();
  });

  it('keeps the sweep undoable when some tabs fail to reopen', async () => {
    const sweepId = await sweepOf([
      makeTab({ id: 2, url: 'https://a.com/' }),
      makeTab({ id: 3, url: 'https://b.com/' }),
    ]);
    commands.failCreate.add('https://b.com/');
    expect(await tracker.undoSweep(sweepId)).toMatchObject({ restored: 1, failed: 1 });
    expect((await db.sweeps.get(sweepId))?.status).toBe('done');
    commands.failCreate.clear();
    expect(await tracker.undoSweep(sweepId)).toMatchObject({ restored: 1, failed: 0 });
    expect((await db.sweeps.get(sweepId))?.status).toBe('undone');
    expect(fake.tabs.filter((t) => t.url === 'https://a.com/')).toHaveLength(1);
  });

  it('refuses to undo twice, and beyond the last 20 sweeps', async () => {
    const sweepId = await sweepOf([makeTab({ id: 2, url: 'https://a.com/' })]);
    await tracker.undoSweep(sweepId);
    await expect(tracker.undoSweep(sweepId)).rejects.toMatchObject({ code: 'NOT_UNDOABLE' });
    await expect(tracker.undoSweep()).rejects.toBeInstanceOf(UndoError);

    const old = (await db.sweeps.get(sweepId)) as Sweep;
    await db.sweeps.update(sweepId, { status: 'done', undoneAt: undefined });
    await db.sweeps.bulkAdd(
      Array.from({ length: UNDO_HISTORY }, (_, i) => ({
        ...old,
        id: `later-${i}`,
        startedAt: old.startedAt + i + 1,
        items: [
          {
            itemId: `x${i}`,
            action: 'archive' as const,
            prevState: 'stale' as const,
            result: 'ok' as const,
          },
        ],
      })),
    );
    await expect(tracker.undoSweep(sweepId)).rejects.toMatchObject({ code: 'NOT_UNDOABLE' });
  });
});

describe('sweep history', () => {
  it('numbers sweeps and labels partial and full restores', async () => {
    await setup(
      [makeTab({ id: 2, url: 'https://a.com/' }), makeTab({ id: 3, url: 'https://b.com/' })],
      { 2: 130, 3: 130 },
    );
    const r = await tickAfter(30_000);
    let history = await loadSweepHistory(db, 'automatic');
    expect(history.entries[0]).toMatchObject({
      number: 1,
      archived: 2,
      status: 'archived',
      undoable: true,
    });

    // One item restored on its own (P0.6 single restore) → partial.
    const [first] = await db.items
      .where('lastSweepId')
      .equals(r.sweep?.id ?? '')
      .toArray();
    await db.items.update(first?.id ?? '', { state: 'restored' });
    history = await loadSweepHistory(db, 'automatic');
    expect(history.entries[0]).toMatchObject({ status: 'partial', restored: 1, remaining: 1 });

    await tracker.undoSweep();
    history = await loadSweepHistory(db, 'automatic');
    expect(history.entries[0]).toMatchObject({ status: 'undone', undoable: false });
  });

  it('labels a sweep where nothing could be closed', () => {
    const sweep = {
      id: 's',
      startedAt: 0,
      trigger: 'scheduled',
      isShadow: 0,
      status: 'done',
      itemCount: 0,
      failedCount: 1,
      items: [{ itemId: 'x', action: 'archive', prevState: 'stale', result: 'skipped' }],
    } satisfies Sweep;
    expect(describeSweep(sweep, 3, new Map(), [sweep])).toMatchObject({
      status: 'nothing',
      undoable: false,
      number: 3,
    });
  });
});

describe('restore (P0.6)', () => {
  async function archive(tabs: TabInfo[]) {
    const old = Object.fromEntries(tabs.map((t) => [t.id, 130]));
    await setup([makeTab({ id: 1, active: true, url: 'https://keep.com/' }), ...tabs], old);
    const r = await tickAfter(30_000);
    return r.sweep?.id ?? '';
  }
  const archivedByUrl = async (url: string) =>
    (await db.items.where('state').equals('archived').toArray()).find((i) => i.url === url);

  it('reopens one item as the active tab of the given window, with grace', async () => {
    await archive([makeTab({ id: 2, url: 'https://a.com/', title: 'A' })]);
    const record = await archivedByUrl('https://a.com/');
    fake.windows.push({ id: 9, focused: true, incognito: false, state: 'normal' });
    clock += 5 * 60_000;
    const r = await tracker.restore([record?.id ?? ''], { intoWindowId: 9 });
    expect(r).toMatchObject({ restored: 1, failed: 0 });
    expect(r.regretted.map((i) => i.id)).toEqual([record?.id]);
    const tab = fake.tabs.find((t) => t.url === 'https://a.com/');
    expect(tab).toMatchObject({ windowId: 9, active: true });
    expect((await db.items.get(record?.id ?? ''))?.state).toBe('restored');
    expect(await live(tab?.id ?? 0)).toMatchObject({
      restoredFromId: record?.id,
      graceUntil: clock + RESTORE_GRACE_MS,
    });
    expect(await db.events.where('type').equals('restored').count()).toBe(1);
    expect((await db.metrics.toArray())[0]).toMatchObject({ restored24hCount: 1, undoCount: 0 });
  });

  it('ignores items that are not archived', async () => {
    await archive([makeTab({ id: 2, url: 'https://a.com/' })]);
    const record = await archivedByUrl('https://a.com/');
    await tracker.restore([record?.id ?? ''], { intoWindowId: 1 });
    const again = await tracker.restore([record?.id ?? '', 'missing'], { intoWindowId: 1 });
    expect(again).toMatchObject({ restored: 0, failed: 1, skipped: 1 });
    expect(fake.tabs.filter((t) => t.url === 'https://a.com/')).toHaveLength(1);
  });

  it('restores a repeated id once (no duplicate tabs or metrics)', async () => {
    await archive([makeTab({ id: 2, url: 'https://a.com/' })]);
    const record = await archivedByUrl('https://a.com/');
    const id = record?.id ?? '';
    const r = await tracker.restore([id, id, id], { intoWindowId: 1 });
    expect(r).toMatchObject({ restored: 1, failed: 0 });
    expect(fake.tabs.filter((t) => t.url === 'https://a.com/')).toHaveLength(1);
    expect((await db.items.get(id))?.restoreCount).toBe(1);
    expect((await db.metrics.toArray())[0]?.restored24hCount).toBe(1);
  });

  it('does not track a tab restored into a private window', async () => {
    await archive([makeTab({ id: 2, url: 'https://a.com/' })]);
    const record = await archivedByUrl('https://a.com/');
    const createTab = commands.createTab.bind(commands);
    commands.createTab = async (o) => ({ ...(await createTab(o)), incognito: true });
    const r = await tracker.restore([record?.id ?? ''], { intoWindowId: 1 });
    expect(r.restored).toBe(1);
    expect((await db.items.get(record?.id ?? ''))?.state).toBe('restored');
    const liveA = (await db.items.where('state').anyOf(['active', 'stale']).toArray()).filter(
      (i) => i.url === 'https://a.com/',
    );
    expect(liveA).toHaveLength(0);
  });

  it('links an already-open tab after an interrupted restore instead of opening another', async () => {
    await archive([makeTab({ id: 2, url: 'https://a.com/' })]);
    const record = await archivedByUrl('https://a.com/');
    // The worker died after opening the tab, before recording the restore.
    await db.items.update(record?.id ?? '', { restoreStartedAt: clock });
    await commands.createTab({ windowId: 1, url: 'https://a.com/' });
    const r = await tracker.restore([record?.id ?? ''], { intoWindowId: 1 });
    expect(r.restored).toBe(1);
    expect(fake.tabs.filter((t) => t.url === 'https://a.com/')).toHaveLength(1);
    expect((await db.items.get(record?.id ?? ''))?.restoreStartedAt).toBeUndefined();
  });

  it('clears the in-progress mark when a tab fails to open, so a retry opens it', async () => {
    await archive([makeTab({ id: 2, url: 'https://a.com/' })]);
    const record = await archivedByUrl('https://a.com/');
    commands.failCreate.add('https://a.com/');
    expect(await tracker.restore([record?.id ?? ''], { intoWindowId: 1 })).toMatchObject({
      restored: 0,
      failed: 1,
    });
    expect((await db.items.get(record?.id ?? ''))?.restoreStartedAt).toBeUndefined();
    commands.failCreate.clear();
    expect((await tracker.restore([record?.id ?? ''], { intoWindowId: 1 })).restored).toBe(1);
  });

  it('restores a batch: the items archived together from the same window, in order', async () => {
    fake.windows.push({ id: 2, focused: false, incognito: false, state: 'normal' });
    await archive([
      makeTab({ id: 2, url: 'https://a.com/', windowId: 2, index: 0 }),
      makeTab({ id: 3, url: 'https://b.com/', windowId: 2, index: 1 }),
      makeTab({ id: 4, url: 'https://c.com/', windowId: 1, index: 1 }),
    ]);
    const b = await archivedByUrl('https://b.com/');
    fake.windows = fake.windows.filter((w) => w.id !== 2);
    const r = await tracker.restoreBatch(b?.id ?? '');
    expect(r.restored).toBe(2);
    // One new window for the batch (S4), even though window 1 still exists.
    expect(commands.createdWindows).toEqual(['https://a.com/']);
    const a2 = fake.tabs.find((t) => t.url === 'https://a.com/');
    expect(fake.tabs.find((t) => t.url === 'https://b.com/')?.windowId).toBe(a2?.windowId);
    expect(await archivedByUrl('https://c.com/')).toBeDefined(); // other window: not part of it
  });

  it('undo skips a sweep whose tabs were all restored by hand', async () => {
    const first = await archive([makeTab({ id: 2, url: 'https://a.com/' })]);
    fake.tabs.push(makeTab({ id: 5, url: 'https://z.com/' }));
    await tickAfter(30_000);
    await db.items.update((await live(5))?.id ?? '', { activeAgeS: 130 * H });
    const second = (await tickAfter(SWEEP_INTERVAL_MS)).sweep?.id;
    expect(second).toBeDefined();
    const z = await archivedByUrl('https://z.com/');
    await tracker.restore([z?.id ?? ''], { intoWindowId: 1 });
    const r = await tracker.undoSweep();
    expect(r.restored).toBe(1);
    expect((await db.sweeps.get(first))?.status).toBe('undone');
    await expect(tracker.undoSweep(second)).rejects.toThrow('already open again');
  });
});

describe('Sweep now in shadow mode (dry run)', () => {
  it('simulates right away, ignoring the 15-min interval, and closes nothing', async () => {
    mode = 'shadow';
    await setup([makeTab({ id: 2, url: 'https://a.com/' })], {});
    await tickAfter(30_000); // a shadow sweep with nothing to flag
    await db.items.update((await live(2))?.id ?? '', { activeAgeS: 130 * H });
    const r = await tracker.sweepNow();
    expect(r.shadowFlagged).toBe(1);
    expect(commands.removed).toEqual([]);
    expect(await realSweeps()).toHaveLength(0);
  });
});
