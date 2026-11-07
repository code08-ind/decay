import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { DecayDB } from '../db/db';
import type { Item, MetricsDay, Rule } from '../db/types';
import { emptyMetrics, localDay } from '../shadow/shadow';
import { FakeBrowser, makeTab } from '../tracker/test-utils';
import { Tracker } from '../tracker/tracker';
import { buildBetaExport, getInstallId, toCsv } from './export';
import { automaticSince, buildInsights, dayRange } from './insights';
import {
  MAX_SAMPLES,
  MAX_SEARCH_TO_RESTORE_MS,
  median,
  recordSearch,
  recordSearchToRestore,
  sampleOpenCounts,
} from './metrics';

const NOW = new Date(2026, 10, 7, 15, 0).getTime();
const day = (offset: number) => dayRange(NOW, offset + 1)[0] as string;
const row = (offset: number, over: Partial<MetricsDay> = {}): MetricsDay => ({
  ...emptyMetrics(day(offset)),
  ...over,
});
const sampled = (offset: number, tabs: number, over: Partial<MetricsDay> = {}) =>
  row(offset, { openTabsP50: tabs, openTabsSamples: [tabs], activeSeconds: 7200, ...over });

let n = 0;
let db: DecayDB;
beforeEach(() => {
  db = new DecayDB(`decay-metrics-test-${n++}`);
});
afterEach(() => db.delete());

describe('median / sampling', () => {
  it('computes medians', () => {
    expect(median([])).toBe(0);
    expect(median([5])).toBe(5);
    expect(median([9, 1, 5])).toBe(5);
    expect(median([1, 2, 3, 4])).toBe(3); // 2.5 rounded
  });

  it('samples open tabs at most once per local hour, keeping medians', () => {
    const r = emptyMetrics(localDay(NOW));
    expect(sampleOpenCounts(r, NOW, { tabs: 40, windows: 2 })).toBe(true);
    expect(sampleOpenCounts(r, NOW + 10 * 60_000, { tabs: 99, windows: 9 })).toBe(false);
    expect(sampleOpenCounts(r, NOW + 3600_000, { tabs: 20, windows: 4 })).toBe(true);
    expect(sampleOpenCounts(r, NOW + 2 * 3600_000, { tabs: 30, windows: 3 })).toBe(true);
    expect(r).toMatchObject({ openTabsP50: 30, openWindowsP50: 3, sampledHours: [15, 16, 17] });
  });

  it('skips hours without open tabs (no normal window yet), so week 1 never has a 0 median', () => {
    const r = emptyMetrics(localDay(NOW));
    expect(sampleOpenCounts(r, NOW, { tabs: 0, windows: 0 })).toBe(false);
    expect(r.sampledHours).toBeUndefined();
    expect(sampleOpenCounts(r, NOW + 60_000, { tabs: 5, windows: 1 })).toBe(true); // same hour
  });

  it('drops idle search → restore times (popup left open)', async () => {
    await recordSearchToRestore(db, MAX_SEARCH_TO_RESTORE_MS + 1, NOW);
    expect(await db.metrics.get(localDay(NOW))).toBeUndefined();
    await recordSearchToRestore(db, MAX_SEARCH_TO_RESTORE_MS, NOW);
    expect((await db.metrics.get(localDay(NOW)))?.searchToRestoreSamples).toEqual([
      MAX_SEARCH_TO_RESTORE_MS,
    ]);
  });

  it('records searches and search → restore times', async () => {
    await recordSearch(db, NOW);
    await recordSearch(db, NOW);
    await recordSearchToRestore(db, 4000, NOW);
    await recordSearchToRestore(db, 2000, NOW);
    await recordSearchToRestore(db, -5, NOW); // ignored
    const r = await db.metrics.get(localDay(NOW));
    expect(r).toMatchObject({ searchCount: 2, searchToRestoreMsP50: 3000 });
    for (let i = 0; i < MAX_SAMPLES + 5; i++) await recordSearchToRestore(db, 100, NOW);
    expect((await db.metrics.get(localDay(NOW)))?.searchToRestoreSamples).toHaveLength(MAX_SAMPLES);
  });
});

describe('hourly sampling in the tick', () => {
  it('samples open tabs and normal windows only while the browser is in use', async () => {
    let clock = NOW;
    const fake = new FakeBrowser();
    fake.windows = [
      { id: 1, focused: true, incognito: false, state: 'normal' },
      { id: 2, focused: false, incognito: false, state: 'normal', type: 'popup' },
    ];
    fake.tabs = [makeTab({ id: 1, active: true }), makeTab({ id: 2, url: 'https://b.test/' })];
    const tracker = new Tracker({
      db,
      api: fake,
      appId: 'com.google.Chrome',
      getIdleCutoffS: async () => 300,
      getPolicy: async () => ({
        mode: 'shadow',
        holdUntil: null,
        thresholds: { stale: 12, hidden: 36, archive: 60 },
      }),
      now: () => clock,
      activationSettleMs: 0,
    });
    await tracker.tick(); // first tick: no credit yet
    expect((await db.metrics.get(localDay(NOW)))?.openTabsSamples).toBeUndefined();
    clock += 30_000;
    await tracker.tick();
    expect(await db.metrics.get(localDay(NOW))).toMatchObject({
      openTabsSamples: [2],
      openWindowsSamples: [1],
      openTabsP50: 2,
    });
    fake.idle = 'idle';
    clock += 3600_000;
    await tracker.tick(); // idle: no credit, no sample
    expect((await db.metrics.get(localDay(NOW)))?.openTabsSamples).toEqual([2]);
  });
});

describe('buildInsights', () => {
  const item = (domain: string, over: Partial<Item> = {}) =>
    ({ id: `${domain}-${n++}`, domain, restoreCount: 0, ...over }) as Item;

  it('summarises the range: open tabs vs first week, regret, archives, sites', () => {
    const metrics = [
      ...[40, 39, 38, 37, 36, 35, 34].map((t, i) => sampled(30 + i, t)), // first week, outside 7d
      sampled(2, 20, { archivedCount: 10, restored24hCount: 1, autoModeOn: 1 }),
      sampled(1, 18, { archivedCount: 10, searchToRestoreSamples: [3000, 1000, 2000] }),
      row(0, { searchCount: 4, undoCount: 1 }),
    ];
    const items = [
      item('a.com', { archivedAt: NOW - 3600_000 }),
      item('a.com', { archivedAt: NOW - 3600_000, restoreCount: 2 }),
      item('b.com', { archivedAt: NOW - 86_400_000 }),
      item('old.com', { archivedAt: NOW - 40 * 86_400_000 }),
    ];
    const r = buildInsights({ now: NOW, range: 7, mode: 'automatic', metrics, items });
    expect(r.days).toHaveLength(7);
    expect(r.days.at(-1)?.day).toBe(localDay(NOW));
    expect(r.days.at(-1)?.openTabs).toBeNull(); // today: no samples
    expect(r.openTabs).toBe(19);
    expect(r.firstWeekOpenTabs).toBe(37);
    expect(r.openTabsChange).toBeCloseTo((19 - 37) / 37);
    expect(r).toMatchObject({ archived: 20, restored24h: 1, regretRate: 0.05, searches: 4 });
    expect(r.searchToRestoreMs).toBe(2000);
    expect(r.topArchived).toEqual([
      { domain: 'a.com', count: 2 },
      { domain: 'b.com', count: 1 },
    ]);
    expect(r.restoredMost).toEqual([{ domain: 'a.com', count: 2 }]);
    const goal = Object.fromEntries(r.goals.map((g) => [g.id, g]));
    expect(goal.regret).toMatchObject({ value: '5%', status: 'notYet' });
    expect(goal.openTabs?.status).toBe('notYet'); // 49% fewer, target 50%
    expect(goal.search).toMatchObject({ value: '2.0 s', status: 'met' });
    // An active day with automatic mode off (day 1) broke the run.
    expect(goal.automatic).toMatchObject({ value: 'Off', status: 'notYet' });
  });

  it('caps regret at 100% when restores outnumber archives in the range', () => {
    const r = buildInsights({
      now: NOW,
      range: 7,
      mode: 'automatic',
      metrics: [sampled(1, 10, { archivedCount: 1, restored24hCount: 3 })],
      items: [],
    });
    expect(r.regretRate).toBe(1);
    expect(r.goals[0]?.value).toBe('100%');
  });

  it('compares open tabs with week 1 only after 14 days', () => {
    const metrics = [sampled(10, 40), sampled(1, 10)];
    const early = buildInsights({ now: NOW, range: 30, mode: 'shadow', metrics, items: [] });
    expect(early.firstWeekOpenTabs).toBe(40);
    expect(early.openTabsChange).toBeNull();
    const later = buildInsights({
      now: NOW,
      range: 30,
      mode: 'shadow',
      metrics: [sampled(14, 40), sampled(1, 10)],
      items: [],
    });
    expect(later.openTabsChange).toBeCloseTo(-0.75);
  });

  it('finds the current unbroken run of automatic mode', () => {
    const on = (o: number) => row(o, { autoModeOn: 1, activeSeconds: 7200 });
    // Inactive days don't break the run.
    expect(automaticSince([on(5), row(4), on(3), on(0)], 'automatic')).toBe(day(5));
    // An active day with automatic off (paused / shadow) does.
    expect(automaticSince([on(5), row(4, { activeSeconds: 600 }), on(3), on(0)], 'automatic')).toBe(
      day(3),
    );
    // Mode not automatic now: no run.
    expect(automaticSince([on(1), on(0)], 'paused')).toBeNull();
    // A trailing inactive day keeps it.
    expect(automaticSince([on(2), row(1), row(0)], 'automatic')).toBe(day(2));
  });

  it('falls back to shadow regret, and reports no data honestly', () => {
    const empty = buildInsights({ now: NOW, range: 30, mode: 'shadow', metrics: [], items: [] });
    expect(empty.openTabs).toBeNull();
    expect(
      empty.goals.filter((g) => g.id !== 'automatic').every((g) => g.status === 'noData'),
    ).toBe(true);
    const shadow = buildInsights({
      now: NOW,
      range: 14,
      mode: 'shadow',
      metrics: [row(1, { shadowFlaggedCount: 50, shadowRegretCount: 1 })],
      items: [],
    });
    expect(shadow.regretRate).toBeNull();
    expect(shadow.shadowRegretRate).toBe(0.02);
    expect(shadow.goals[0]).toMatchObject({ label: 'Regret (shadow, 14 days)', status: 'met' });
  });
});

describe('beta export', () => {
  const rules = [
    { id: 'r1', scope: 'domain', match: 'secret-bank.com', action: 'protect', enabled: true },
    { id: 'r2', scope: 'domain', match: 'private.example', action: 'exclude', enabled: true },
    { id: 'r3', scope: 'domain', match: 'off.example', action: 'protect', enabled: false },
  ] as Rule[];
  const input = () => ({
    now: NOW,
    installId: 'abc',
    extensionVersion: '0.1.0',
    browser: 'chrome' as const,
    mode: 'automatic' as const,
    preset: 'balanced',
    thresholds: { stale: 12, hidden: 36, archive: 60 },
    probeAccess: 'some' as const,
    rules,
    shadowStartedAt: NOW - 10 * 86_400_000,
    metrics: [
      sampled(1, 30, { archivedCount: 8, restored24hCount: 1, searchToRestoreSamples: [1500] }),
      row(0, { activeSeconds: 600 }),
    ],
  });

  it('has counts only: no sites, rule patterns or URLs', () => {
    const data = buildBetaExport(input());
    const text = JSON.stringify(data);
    for (const secret of ['secret-bank', 'private.example', 'off.example', 'http']) {
      expect(text).not.toContain(secret);
    }
    expect(data.setup.rules).toEqual({
      protect: 1,
      exclude: 1,
      customThreshold: 0,
      closeAllowed: 0,
    });
    expect(data.setup.shadowStartedDaysAgo).toBe(10);
    expect(data.summary).toMatchObject({
      days: 2,
      activeDays: 1,
      archived: 8,
      regretRate: 0.125,
      searchToRestoreMsP50: 1500,
      openTabsP50: 30,
    });
    expect(data.days[1]).toMatchObject({
      day: localDay(NOW),
      activeMinutes: 10,
      openTabsP50: null,
    });
  });

  it('reports automatic days the same way as Insights', () => {
    const metrics = [row(5, { autoModeOn: 1, activeSeconds: 7200 }), row(0, { autoModeOn: 1 })];
    const data = buildBetaExport({ ...input(), metrics });
    const page = buildInsights({ now: NOW, range: 30, mode: 'automatic', metrics, items: [] });
    expect(data.setup.automaticForDays).toBe(5);
    expect(page.goals.find((g) => g.id === 'automatic')?.value).toBe('On for 5 days');
    expect(data.summary).not.toHaveProperty('autoModeDays');
  });

  it('exports the same daily rows as CSV', () => {
    const text = toCsv(buildBetaExport(input()));
    expect(text.endsWith('\n')).toBe(true);
    const csv = text.trimEnd().split('\n');
    expect(csv[0]).toMatch(/^day,activeMinutes,active,openTabsP50/);
    expect(csv).toHaveLength(3);
    expect(csv[2]).toContain(`${localDay(NOW)},10,false,,`);
  });

  it('keeps one random install id', async () => {
    const id = await getInstallId(db);
    expect(id).toMatch(/^[0-9a-f-]{36}$/);
    expect(await getInstallId(db)).toBe(id);
  });
});
