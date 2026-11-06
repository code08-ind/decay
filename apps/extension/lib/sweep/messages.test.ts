import { describe, expect, it } from 'vitest';
import { isDecayRequest } from './messages';

describe('isDecayRequest', () => {
  it('accepts well-formed requests', () => {
    expect(isDecayRequest({ type: 'decay.sweepNow' })).toBe(true);
    expect(isDecayRequest({ type: 'decay.undoSweep' })).toBe(true);
    expect(isDecayRequest({ type: 'decay.restore', ids: ['a'], intoWindowId: 3 })).toBe(true);
    expect(isDecayRequest({ type: 'decay.restoreBatch', itemId: 'a' })).toBe(true);
  });

  it('rejects malformed ones', () => {
    for (const bad of [
      null,
      'decay.sweepNow',
      { type: 'other' },
      { type: 'decay.undoSweep', sweepId: 5 },
      { type: 'decay.restore', ids: [] },
      { type: 'decay.restore', ids: [1] },
      { type: 'decay.restore', ids: ['a'], intoWindowId: 'x' },
      { type: 'decay.restore', ids: Array.from({ length: 501 }, () => 'a') },
      { type: 'decay.restoreBatch' },
    ]) {
      expect(isDecayRequest(bad)).toBe(false);
    }
  });
});
