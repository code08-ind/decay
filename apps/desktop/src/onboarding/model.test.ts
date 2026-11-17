import { describe, expect, it } from 'vitest';
import type { DetectedBrowser } from '../api';
import {
  back,
  hostProblem,
  next,
  PRESETS,
  rowDetail,
  rowState,
  STEPS,
  sortRows,
  stepTitle,
} from './model';

const browser = (over: Partial<DetectedBrowser>): DetectedBrowser => ({
  id: 'chrome',
  name: 'Google Chrome',
  installed: true,
  hostInstalled: true,
  extensionConnected: false,
  profiles: 0,
  storeUrl: 'https://chromewebstore.google.com/search/Decay',
  ...over,
});

describe('onboarding model', () => {
  it('has the six steps of the design', () => {
    expect(STEPS.map((s) => stepTitle(s.id))).toEqual([
      'Welcome',
      'Choose pace',
      'Connect browsers',
      'Window access',
      'Shadow mode',
      'All set',
    ]);
  });

  it('presets match the PRD and are ordered', () => {
    expect(PRESETS.map((p) => [p.id, p.hours])).toEqual([
      ['gentle', { stale: 24, hidden: 60, archive: 96 }],
      ['balanced', { stale: 12, hidden: 36, archive: 60 }],
      ['aggressive', { stale: 8, hidden: 24, archive: 40 }],
    ]);
    for (const p of PRESETS) {
      expect(p.hours.stale).toBeLessThan(p.hours.hidden);
      expect(p.hours.hidden).toBeLessThan(p.hours.archive);
    }
  });

  it('browser rows show connection, install or not found', () => {
    expect(rowState(browser({ extensionConnected: true, profiles: 1 }))).toBe('connected');
    expect(rowDetail(browser({ extensionConnected: true, profiles: 1 }))).toBe(
      'Extension connected',
    );
    expect(rowDetail(browser({ extensionConnected: true, profiles: 3 }))).toBe(
      '3 profiles connected',
    );
    expect(rowState(browser({}))).toBe('install');
    expect(rowDetail(browser({ hostInstalled: false }))).toBe('Detected');
    expect(rowState(browser({ installed: false, hostInstalled: false }))).toBe('missing');
    expect(rowDetail(browser({ installed: false }))).toBe('Not installed');
  });

  it('sorts connected first, keeping the order otherwise', () => {
    const rows = [
      browser({ id: 'edge', installed: false }),
      browser({ id: 'brave' }),
      browser({ id: 'arc', extensionConnected: true, profiles: 1 }),
      browser({ id: 'chrome' }),
    ];
    expect(sortRows(rows).map((r) => r.id)).toEqual(['arc', 'brave', 'chrome', 'edge']);
  });

  it('explains connector problems without internals', () => {
    expect(hostProblem({ code: 'NOT_FOUND', message: 'x' })).toMatch(/doesn't include/);
    const failed = hostProblem({ code: 'PERMISSION_DENIED', message: 'no extension is allowed' });
    expect(failed).toMatch(/couldn't set up/);
    expect(failed).not.toMatch(/extension is allowed/);
    expect(hostProblem('boom')).toMatch(/couldn't set up/);
  });

  it('steps forward and back, but not back after completing', () => {
    expect(next(0)).toBe(1);
    expect(next(5)).toBe(5);
    expect(back(0, false)).toBe(0);
    expect(back(3, false)).toBe(2);
    expect(back(5, true)).toBe(5);
  });
});
