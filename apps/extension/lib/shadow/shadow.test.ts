import { describe, expect, it } from 'vitest';
import type { Item } from '../db/types';
import type { Decision } from '../policy/policy';
import { newItemFromTab } from '../tracker/tabs';
import { makeTab } from '../tracker/test-utils';
import { countActiveDays, emptyMetrics, localDay, planShadowSweep } from './shadow';

const item = (age: number, over: Partial<Item> = {}): Item => ({
  ...newItemFromTab(makeTab({ id: Math.floor(Math.random() * 1e9) }), 'app', new Map(), 0),
  activeAgeS: age,
  ...over,
});
const archive: Decision = {
  action: 'archive',
  reason: { type: 'unused', activeHours: 61, threshold: 60 },
};
const keep: Decision = {
  action: 'keep',
  activeHours: 1,
  threshold: { stale: 12, hidden: 36, archive: 60 },
};

describe('shadow', () => {
  it('formats local days', () => {
    expect(localDay(new Date(2026, 10, 5, 23, 59).getTime())).toBe('2026-11-05');
  });

  it('counts calendar days with ≥ 1 h of activity since the start', () => {
    const start = new Date(2026, 10, 2, 9).getTime();
    const m = (day: string, s: number) => ({ ...emptyMetrics(day), activeSeconds: s });
    const metrics = [
      m('2026-11-01', 9000),
      m('2026-11-02', 3600),
      m('2026-11-03', 3599),
      m('2026-11-04', 5000),
    ];
    expect(countActiveDays(metrics, start)).toBe(2);
    expect(countActiveDays(metrics, null)).toBe(3);
  });

  it('flags the oldest unflagged archive candidates, capped', () => {
    const a = item(100);
    const b = item(300);
    const c = item(200);
    const flaggedBefore = item(500, { shadowFlaggedAt: 1 });
    const notDue = item(900);
    const decisions = new Map<string, Decision>([
      [a.id, archive],
      [b.id, archive],
      [c.id, archive],
      [flaggedBefore.id, archive],
      [notDue.id, keep],
    ]);
    const plan = planShadowSweep([a, b, c, flaggedBefore, notDue], decisions, 1000, 2);
    expect(plan?.flagged.map((i) => i.id)).toEqual([b.id, c.id]);
    expect(plan?.sweep).toMatchObject({
      isShadow: 1,
      status: 'done',
      itemCount: 2,
      trigger: 'scheduled',
    });
    expect(
      plan?.events.every((e) => e.type === 'shadow_would_archive' && e.sweepId === plan.sweep.id),
    ).toBe(true);
    expect(b.shadowFlaggedAt).toBe(1000);
    expect(a.shadowFlaggedAt).toBeUndefined();
    expect(planShadowSweep([notDue], decisions, 1000, 50)).toBeNull();
  });
});
