import { describe, expect, it } from 'vitest';
import { computeCredit, MAX_TICK_GAP_MS, TICK_MS } from './clock';

describe('computeCredit', () => {
  const base = { elapsedMs: TICK_MS, idleState: 'active' as const, anyAudible: false };

  it('credits the elapsed time while the user is active', () => {
    expect(computeCredit(base)).toBe(30);
    expect(computeCredit({ ...base, elapsedMs: 41_400 })).toBe(41);
  });

  it('credits nothing while idle, unless audio is playing', () => {
    expect(computeCredit({ ...base, idleState: 'idle' })).toBe(0);
    expect(computeCredit({ ...base, idleState: 'idle', anyAudible: true })).toBe(30);
  });

  it('credits nothing while locked, even with audio', () => {
    expect(computeCredit({ ...base, idleState: 'locked', anyAudible: true })).toBe(0);
  });

  it('gives no back-credit after sleep or a closed browser (Monday reset)', () => {
    expect(computeCredit({ ...base, elapsedMs: MAX_TICK_GAP_MS + 1 })).toBe(0);
    expect(computeCredit({ ...base, elapsedMs: 64 * 3600_000 })).toBe(0);
  });

  it('credits nothing on the first tick, or when the clock went backwards', () => {
    expect(computeCredit({ ...base, elapsedMs: null })).toBe(0);
    expect(computeCredit({ ...base, elapsedMs: -5000 })).toBe(0);
    expect(computeCredit({ ...base, elapsedMs: Number.NaN })).toBe(0);
  });
});
