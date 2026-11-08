import { describe, expect, it } from 'vitest';
import type { Item, Rule } from '../db/types';
import { newItemFromTab } from '../tracker/tabs';
import { makeTab } from '../tracker/test-utils';
import { protectRuleFor } from './actions';
import { buildShadowReport, type ReportInput } from './report';
import { emptyMetrics, SHADOW_SWEEP_INTERVAL_MS } from './shadow';

const H = 3600;
let n = 1;
const item = (ageH: number, over: Partial<Item> = {}): Item => ({
  ...newItemFromTab(makeTab({ id: n++, url: `https://site${n}.com/` }), 'app', new Map(), 0),
  activeAgeS: ageH * H,
  ...over,
});
/** Items' tabs are open unless the test passes `tabs` itself. */
const openTabs = (items: readonly Item[]) =>
  items.map((i) => makeTab({ id: Number(i.externalId), url: i.url }));
const base = (over: Partial<ReportInput> = {}): ReportInput => ({
  now: 10_000_000,
  mode: 'shadow',
  holdUntil: null,
  // Without probe verdicts, web pages archive at 2× (PRD §7.3): 60 h here.
  thresholds: { stale: 12, hidden: 20, archive: 30 },
  shadowStartedAt: null,
  rules: [],
  items: [],
  metrics: [],
  lastShadowSweepAt: null,
  ...over,
  tabs: over.tabs ?? openTabs(over.items ?? []),
});

describe('buildShadowReport', () => {
  it('skips records whose tab is no longer open (QA P0: after a browser restart)', () => {
    const open = item(70);
    const gone = item(80);
    const r = buildShadowReport(base({ items: [open, gone], tabs: openTabs([open]) }));
    expect(r.wouldArchive.map((x) => x.item.id)).toEqual([open.id]);
  });

  it('splits open tabs into would-archive and kept, with reason groups', () => {
    const old = item(70);
    const older = item(90);
    const pinned = item(80, { pinned: true });
    const young = item(5);
    const archived = item(100, { state: 'archived' });
    const ruled = item(65, { url: 'https://keep.me/', domain: 'keep.me' });
    const rule: Rule = {
      id: 'r1',
      scope: 'domain',
      match: 'keep.me',
      action: 'protect',
      enabled: true,
      source: 'user',
      createdAt: 0,
      hitCount: 0,
    };
    const r = buildShadowReport(
      base({ items: [old, older, pinned, young, archived, ruled], rules: [rule] }),
    );
    expect(r.wouldArchive.map((e) => e.item.id)).toEqual([older.id, old.id]);
    expect(r.kept.map((e) => e.item.id)).toEqual([pinned.id, ruled.id]);
    expect(r.keptByGroup).toMatchObject({ pinned: 1, rules: 1, audio: 0 });
  });

  it('uses live tab signals for active and audible tabs', () => {
    const a = item(70);
    const r = buildShadowReport(
      base({ items: [a], tabs: [makeTab({ id: Number(a.externalId), audible: true })] }),
    );
    expect(r.kept[0]?.decision).toMatchObject({ action: 'protect', reason: { by: 'audible' } });
    expect(r.keptByGroup.audio).toBe(1);
  });

  it('counts today once it has an hour of activity, and caps the day number', () => {
    const today = new Date(10_000_000);
    const day = `${today.getFullYear()}-${String(today.getMonth() + 1).padStart(2, '0')}-${String(today.getDate()).padStart(2, '0')}`;
    const r = buildShadowReport(base({ metrics: [{ ...emptyMetrics(day), activeSeconds: 3600 }] }));
    expect(r).toMatchObject({ activeDays: 1, todayCounted: true, dayNumber: 1 });
    expect(buildShadowReport(base()).dayNumber).toBe(1); // first day, nothing yet
  });

  it('lists top domains among would-archive tabs', () => {
    const a = item(70, { domain: 'a.com' });
    const b = item(70, { domain: 'a.com' });
    const c = item(70, { domain: 'b.com' });
    expect(buildShadowReport(base({ items: [a, b, c] })).topDomains).toEqual([
      { domain: 'a.com', count: 2 },
      { domain: 'b.com', count: 1 },
    ]);
  });

  it('puts empty new tabs last', () => {
    const blank = item(200, { url: 'chrome://newtab/', domain: undefined });
    const page = item(70);
    const r = buildShadowReport(base({ items: [blank, page] }));
    expect(r.wouldArchive.map((e) => e.item.id)).toEqual([page.id, blank.id]);
  });

  it('computes days, totals and regret for the shadow period only', () => {
    const start = new Date(2026, 10, 2, 9).getTime();
    const m = (day: string, active: number, flagged: number, regret: number) => ({
      ...emptyMetrics(day),
      activeSeconds: active,
      shadowFlaggedCount: flagged,
      shadowRegretCount: regret,
    });
    const r = buildShadowReport(
      base({
        shadowStartedAt: start,
        metrics: [
          m('2026-11-01', 9000, 50, 50), // before shadow mode started
          m('2026-11-02', 4000, 10, 1),
          m('2026-11-03', 100, 5, 0),
          m('2026-11-04', 7200, 25, 1),
        ],
      }),
    );
    expect(r).toMatchObject({ activeDays: 2, ready: false, flaggedTotal: 40, regretTotal: 2 });
    // Today (not in the metrics) hasn't reached 1 h yet: it is day 3 in progress.
    expect(r).toMatchObject({ todayCounted: false, dayNumber: 3 });
    expect(r.regretRate).toBeCloseTo(0.05);
  });

  it('is ready after 7 active days and schedules the next simulation', () => {
    const days = Array.from({ length: 7 }, (_, i) => ({
      ...emptyMetrics(`2026-11-0${i + 1}`),
      activeSeconds: 3600,
    }));
    const r = buildShadowReport(base({ metrics: days, lastShadowSweepAt: 10_000_000 - 60_000 }));
    expect(r.ready).toBe(true);
    expect(r.regretRate).toBeNull();
    expect(r.nextSimulationAt).toBe(10_000_000 - 60_000 + SHADOW_SWEEP_INTERVAL_MS);
    expect(buildShadowReport(base({ mode: 'automatic' })).nextSimulationAt).toBeNull();
  });
});

describe('protectRuleFor', () => {
  it('protects the host (not the whole registrable domain), or the exact URL', () => {
    expect(protectRuleFor({ url: 'https://www.stripe.com/x' })).toEqual({
      scope: 'domain',
      match: 'stripe.com',
      action: 'protect',
    });
    expect(protectRuleFor({ url: 'https://docs.google.com/d/1' })).toMatchObject({
      match: 'docs.google.com',
    });
    const long = `chrome-extension://abc/${'x'.repeat(3000)}`;
    expect(protectRuleFor({ url: long })?.match.length).toBeLessThanOrEqual(2001);
    expect(protectRuleFor({ url: 'chrome://settings/' })).toMatchObject({
      scope: 'urlGlob',
      match: 'chrome://settings/',
    });
    expect(protectRuleFor({})).toBeNull();
  });
});
