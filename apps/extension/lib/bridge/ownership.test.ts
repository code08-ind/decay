import { describe, expect, it } from 'vitest';
import { localEngineMayRun } from './ownership';

describe('localEngineMayRun', () => {
  it('runs as in Phase 0 when the link is off', () => {
    expect(localEngineMayRun(false, 'off', false)).toBe(true);
    expect(localEngineMayRun(false, 'connected', true)).toBe(true);
  });

  it('never runs while the app is (or may be) in charge', () => {
    expect(localEngineMayRun(true, 'connected', true)).toBe(false);
    expect(localEngineMayRun(true, 'connecting', true)).toBe(false);
    // A fresh worker: no attempt has finished yet.
    expect(localEngineMayRun(true, 'off', false)).toBe(false);
  });

  it('takes over once an attempt has failed', () => {
    expect(localEngineMayRun(true, 'unavailable', true)).toBe(true);
    expect(localEngineMayRun(true, 'outdated', true)).toBe(true);
    expect(localEngineMayRun(true, 'off', true)).toBe(true); // no permission
  });
});
