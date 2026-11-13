import { expect, it, describe as suite } from 'vitest';
import { describe, type EngineStatus, errorMessage } from './api';

const base: EngineStatus = {
  mode: 'automatic',
  holdUntil: null,
  staleCount: 3,
  liveTabs: 12,
  browsers: [{ browser: 'chrome', profileId: 'p', tabs: 12 }],
  sweeping: false,
  undoableSweep: null,
  accessibility: true,
};

suite('describe', () => {
  it('matches the tray line', () => {
    expect(describe(base)).toBe('12 tabs open, 3 stale');
    expect(describe({ ...base, mode: 'shadow', liveTabs: 1 })).toBe(
      '1 tab open, 3 stale · shadow mode',
    );
    expect(describe({ ...base, mode: 'paused' })).toBe('Paused');
    expect(describe({ ...base, browsers: [] })).toBe('No browser connected');
    expect(describe({ ...base, sweeping: true })).toBe('Sweeping…');
  });

  it('shows holds, and ignores one that ran out', () => {
    expect(describe({ ...base, holdUntil: 253_402_300_799_000 }, 0)).toBe('On hold');
    expect(describe({ ...base, holdUntil: 60_000 }, 0)).toMatch(/^On hold until /);
    expect(describe({ ...base, holdUntil: 1 }, 2)).toBe('12 tabs open, 3 stale');
  });
});

it('reads app errors', () => {
  expect(errorMessage({ code: 'CONFLICT', message: 'taken' })).toBe('taken');
  expect(errorMessage('plain')).toBe('plain');
});
