import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { DecayDB } from '../db/db';
import type { Mode } from '../db/types';
import { buildShadowReport } from '../shadow/report';
import { SWEEP_INTERVAL_MS } from '../sweep/executor';
import type { TabInfo } from '../tracker/tabs';
import { FakeBrowser, FakeCommands, makeTab } from '../tracker/test-utils';
import { type TabProber, Tracker } from '../tracker/tracker';
import {
  MAX_PROBES_PER_TICK,
  PROBE_CONCURRENCY,
  PROBE_TTL_MS,
  type ProbeRecord,
  type ProbeVerdict,
} from './probe';

const APP = 'com.google.Chrome';
const H = 3600;

/** Verdicts per tab id: a fixed value, or a queue (one per probe, last one repeats). */
class FakeProber implements TabProber {
  verdicts = new Map<number, ProbeVerdict | ProbeVerdict[]>();
  calls: number[] = [];
  stored: ProbeRecord = {};
  async probe(tab: TabInfo): Promise<ProbeVerdict> {
    const id = tab.id ?? -1;
    this.calls.push(id);
    const v = this.verdicts.get(id) ?? 'clean';
    if (!Array.isArray(v)) return v;
    return (v.length > 1 ? v.shift() : v[0]) ?? 'clean';
  }
  async load() {
    return structuredClone(this.stored);
  }
  async save(record: ProbeRecord) {
    this.stored = structuredClone(record);
  }
}

let n = 0;
let db: DecayDB;
let fake: FakeBrowser;
let commands: FakeCommands;
let prober: FakeProber;
let clock: number;
let mode: Mode;
let tracker: Tracker;
const thresholds = { stale: 12, hidden: 36, archive: 60 };

const makeTracker = (withProber = true) =>
  new Tracker({
    db,
    api: fake,
    appId: APP,
    commands,
    prober: withProber ? prober : undefined,
    getIdleCutoffS: async () => 300,
    getPolicy: async () => ({ mode, holdUntil: null, thresholds, maxItemsPerSweep: 50 }),
    now: () => clock,
    activationSettleMs: 0,
  });

beforeEach(() => {
  db = new DecayDB(`decay-probe-test-${n++}`);
  fake = new FakeBrowser();
  commands = new FakeCommands(fake);
  prober = new FakeProber();
  clock = 10_000_000;
  mode = 'automatic';
  tracker = makeTracker();
});
afterEach(() => db.delete());

const live = (tabId: number) => db.findLiveByExternalId(APP, String(tabId));

async function setup(tabs: TabInfo[], old: Record<number, number>) {
  fake.tabs = tabs;
  await tracker.tick();
  for (const [tabId, hours] of Object.entries(old)) {
    const item = await live(Number(tabId));
    await db.items.update(item?.id ?? '', { activeAgeS: hours * H });
  }
  prober.calls = [];
}

async function tickAfter(ms: number) {
  clock += ms;
  return tracker.tick();
}

const page = (id: number, over: Partial<TabInfo> = {}) =>
  makeTab({ id, url: `https://site${id}.com/`, title: `Site ${id}`, index: id, ...over });

describe('form probe in ticks and sweeps (P0.7)', () => {
  it('archives a checked, clean web page at the normal threshold', async () => {
    await setup([page(1, { active: true }), page(2)], { 2: 61 });
    const r = await tickAfter(30_000);
    expect(prober.calls).toContain(2);
    expect(commands.removed).toEqual([2]);
    expect(r.sweep).toMatchObject({ archived: 1 });
    const [item] = await db.items.where('state').equals('archived').toArray();
    expect(item?.archiveReason).toMatchObject({ type: 'unused', threshold: 60 });
  });

  it('never closes a page with unsaved typing, and shows it as kept for that reason', async () => {
    prober.verdicts.set(2, 'dirty');
    await setup([page(1, { active: true }), page(2)], { 2: 500 });
    await tickAfter(30_000);
    await tickAfter(SWEEP_INTERVAL_MS);
    expect(commands.removed).toEqual([]);
    expect((await live(2))?.state).toBe('active'); // protected items show as active

    const items = await db.items.toArray();
    const report = buildShadowReport({
      now: clock,
      mode: 'shadow',
      holdUntil: null,
      thresholds,
      shadowStartedAt: null,
      rules: [],
      items,
      metrics: [],
      tabs: fake.tabs,
      lastShadowSweepAt: null,
      probes: prober.stored,
      probeAccess: 'all',
    });
    expect(report.kept.map((k) => k.item.url)).toEqual(['https://site2.com/']);
    expect(report.keptByGroup.form).toBe(1);
  });

  it('waits twice as long for a page it could not check', async () => {
    prober.verdicts.set(2, 'unknown');
    await setup([page(1, { active: true }), page(2)], { 2: 61 });
    await tickAfter(30_000);
    expect(commands.removed).toEqual([]);
    const item = await live(2);
    await db.items.update(item?.id ?? '', { activeAgeS: 121 * H });
    await tickAfter(SWEEP_INTERVAL_MS);
    expect(commands.removed).toEqual([2]);
    const [archived] = await db.items.where('state').equals('archived').toArray();
    expect(archived?.archiveReason).toMatchObject({ threshold: 120 });
  });

  it('only probes web pages that would be archived, and reuses fresh verdicts', async () => {
    mode = 'shadow';
    await setup(
      [
        page(1, { active: true }),
        page(2), // young
        page(3, { pinned: true }),
        page(4, { url: 'chrome://settings/' }),
        page(5), // candidate
      ],
      { 1: 200, 2: 5, 3: 200, 4: 200, 5: 70 },
    );
    await tickAfter(30_000);
    expect(prober.calls).toEqual([5]);
    await tickAfter(30_000);
    expect(prober.calls).toEqual([5]); // cached
    await tickAfter(PROBE_TTL_MS);
    expect(prober.calls).toEqual([5, 5]); // expired → checked again
  });

  it('checks again after the tab is used or navigates', async () => {
    mode = 'shadow';
    await setup([page(1, { active: true }), page(2)], { 2: 70 });
    await tickAfter(30_000);
    expect(prober.calls).toEqual([2]);
    // Navigated: a new URL needs a new verdict.
    const tab = fake.tabs.find((t) => t.id === 2);
    if (tab) tab.url = 'https://site2.com/other';
    await tickAfter(30_000);
    // A tick that sees a new URL tracks it as a new page (age 0): age it, then check.
    const moved = (await db.items.toArray()).find((i) => i.url === 'https://site2.com/other');
    await db.items.update(moved?.id ?? '', { activeAgeS: 70 * H });
    await tickAfter(30_000);
    expect(prober.calls).toEqual([2, 2]);
    // Used: the verdict no longer counts (typing needs the tab in front).
    await db.items.update(moved?.id ?? '', { lastUsedAt: clock + 1 });
    clock += 2;
    await tracker.tick();
    expect(prober.calls).toEqual([2, 2, 2]);
  });

  it('probes oldest tabs first, at most MAX_PROBES_PER_TICK per tick', async () => {
    mode = 'shadow';
    const count = MAX_PROBES_PER_TICK + 5;
    const tabs = [page(1, { active: true })];
    const old: Record<number, number> = {};
    for (let i = 0; i < count; i++) {
      tabs.push(page(100 + i));
      old[100 + i] = 61 + i;
    }
    await setup(tabs, old);
    await tickAfter(30_000);
    expect(prober.calls).toHaveLength(MAX_PROBES_PER_TICK);
    expect(prober.calls).toContain(100 + count - 1);
    expect(prober.calls).not.toContain(100);
    await tickAfter(30_000);
    expect(prober.calls).toHaveLength(count);
  });

  it('checks again right before closing and keeps a tab that gained typing', async () => {
    prober.verdicts.set(2, ['clean', 'dirty']);
    await setup([page(1, { active: true }), page(2)], { 2: 61 });
    const r = await tickAfter(30_000);
    expect(prober.calls).toEqual([2, 2]);
    expect(commands.removed).toEqual([]);
    expect(r.sweep).toMatchObject({ archived: 0, failed: 1 });
    const [sweep] = await db.sweeps.where('isShadow').equals(0).toArray();
    expect(sweep?.items[0]).toMatchObject({ result: 'skipped', skipReason: 'formInput' });
    expect((await live(2))?.state).not.toBe('archiving');
    // The close-time verdict is remembered: the next tick protects the tab.
    await tickAfter(SWEEP_INTERVAL_MS);
    expect(commands.removed).toEqual([]);
  });

  it('keeps a page that can no longer be checked unless it is past the 2× threshold', async () => {
    prober.verdicts.set(2, ['clean', 'unknown']);
    prober.verdicts.set(3, ['clean', 'unknown']);
    await setup([page(1, { active: true }), page(2), page(3)], { 2: 61, 3: 130 });
    await tickAfter(30_000);
    expect(commands.removed).toEqual([3]);
    const [sweep] = await db.sweeps.where('isShadow').equals(0).toArray();
    const entry = sweep?.items.find((e) => e.result === 'skipped');
    expect(entry?.skipReason).toBe('changed');
  });

  it('forgets verdicts for closed tabs and when site access changes', async () => {
    mode = 'shadow';
    await setup([page(1, { active: true }), page(2), page(3)], { 2: 70, 3: 70 });
    await tickAfter(30_000);
    expect(Object.keys(prober.stored).sort()).toEqual(['2', '3']);
    fake.tabs = fake.tabs.filter((t) => t.id !== 3);
    await tickAfter(30_000);
    expect(Object.keys(prober.stored)).toEqual(['2']);
    await tracker.resetProbes();
    expect(prober.stored).toEqual({});
    prober.calls = [];
    await tickAfter(30_000);
    expect(prober.calls).toEqual([2]);
  });

  it('loads saved verdicts after a worker restart instead of probing again', async () => {
    mode = 'shadow';
    await setup([page(1, { active: true }), page(2)], { 2: 70 });
    await tickAfter(30_000);
    expect(prober.calls).toEqual([2]);
    tracker = makeTracker();
    await tickAfter(30_000);
    expect(prober.calls).toEqual([2]);
  });

  it('limits probes in flight, also for the checks right before closing', async () => {
    const tabs = [page(1, { active: true })];
    const old: Record<number, number> = {};
    for (let i = 0; i < 20; i++) {
      tabs.push(page(10 + i));
      old[10 + i] = 61 + i;
    }
    await setup(tabs, old);
    let inFlight = 0;
    let peak = 0;
    const base = prober.probe.bind(prober);
    prober.probe = async (tab) => {
      inFlight++;
      peak = Math.max(peak, inFlight);
      await new Promise((r) => setTimeout(r, 2));
      inFlight--;
      return base(tab);
    };
    const r = await tickAfter(30_000);
    expect(r.sweep?.archived).toBe(20);
    expect(prober.calls).toHaveLength(40); // planning + pre-close
    expect(peak).toBeLessThanOrEqual(PROBE_CONCURRENCY);
  });

  it('treats a probe failure in a tick as unknown (2×), not as a crash', async () => {
    prober.probe = async () => {
      throw new Error('boom');
    };
    await setup([page(1, { active: true }), page(2)], { 2: 61 });
    const r = await tickAfter(30_000);
    expect(r.liveCount).toBe(2);
    expect(commands.removed).toEqual([]);
  });
});
