import { describe, expect, it } from 'vitest';
import {
  cachedVerdict,
  combineProbe,
  createLimiter,
  formSignal,
  mapLimit,
  PROBE_TTL_MS,
  PROBE_UNKNOWN_TTL_MS,
  parseProbeRecord,
} from './probe';

const forms = (over = {}) => ({ tracked: true, dirty: false, unknown: false, ...over });
const unload = (over = {}) => ({ tracked: true, handlers: 0, activated: true, ...over });
const top = <T>(result: T) => ({ frameId: 0, result });
const sub = <T>(result: T, frameId = 3) => ({ frameId, result });

describe('combineProbe', () => {
  it('is clean when every frame answered and nothing was typed', () => {
    expect(combineProbe([top(forms()), sub(forms())], [top(unload()), sub(unload())])).toBe(
      'clean',
    );
  });

  it('is dirty when any frame holds typed input', () => {
    expect(combineProbe([top(forms()), sub(forms({ dirty: true }))], [top(unload())])).toBe(
      'dirty',
    );
  });

  it('is dirty for a beforeunload handler only after user activation', () => {
    expect(combineProbe([top(forms())], [top(unload({ handlers: 1 }))])).toBe('dirty');
    // Without user activation the browser never prompts: nothing can be lost that way.
    expect(combineProbe([top(forms())], [top(unload({ handlers: 2, activated: false }))])).toBe(
      'clean',
    );
  });

  it('is unknown when the top frame did not answer or an answer is malformed', () => {
    expect(combineProbe([sub(forms())], [top(unload())])).toBe('unknown');
    expect(combineProbe([top(forms())], [])).toBe('unknown');
    expect(combineProbe([top(forms()), sub(null)], [top(unload())])).toBe('unknown');
    expect(combineProbe([top({ dirty: 'yes' })], [top(unload())])).toBe('unknown');
  });

  it('is dirty while an upload is running (closing would cancel it)', () => {
    expect(combineProbe([top(forms())], [top(unload({ uploads: 1 }))])).toBe('dirty');
    expect(combineProbe([top(forms())], [top(unload({ uploads: 'x' }))])).toBe('unknown');
  });

  it('is unknown when an embedded page comes from a site Decay may not check', () => {
    const page = top(forms({ frameOrigins: ['https://pay.example'] }));
    expect(combineProbe([page], [top(unload())])).toBe('clean'); // all sites: frames were checked
    expect(combineProbe([page], [top(unload())], () => false)).toBe('unknown');
    expect(combineProbe([page], [top(unload())], (o) => o === 'https://pay.example')).toBe('clean');
    expect(combineProbe([top(forms({ frameOrigins: [7] }))], [top(unload())])).toBe('unknown');
  });

  it('trusts a dirty answer even when other answers are missing', () => {
    expect(combineProbe([sub(forms({ dirty: true }))], [])).toBe('dirty');
  });

  it('is unknown when an unwatched page had user activation (listeners invisible)', () => {
    expect(combineProbe([top(forms())], [top(unload({ tracked: false }))])).toBe('unknown');
    expect(combineProbe([top(forms())], [top(unload({ tracked: false, activated: false }))])).toBe(
      'clean',
    );
    expect(combineProbe([top(forms())], [top(unload({ tracked: false, handlers: 1 }))])).toBe(
      'dirty',
    );
  });

  it('is unknown when a rich-text editor in an unwatched page might hold typing', () => {
    expect(combineProbe([top(forms({ tracked: false, unknown: true }))], [top(unload())])).toBe(
      'unknown',
    );
  });
});

describe('cachedVerdict', () => {
  const now = 10_000_000;
  const tab = { id: 4, url: 'https://a.com/x' };
  const rec = (over = {}) => ({
    '4': { url: 'https://a.com/x', at: now - 1000, verdict: 'clean' as const, ...over },
  });

  it('returns a fresh verdict for the same tab and URL', () => {
    expect(cachedVerdict(rec(), tab, 0, now)).toBe('clean');
  });

  it('ignores it after a navigation, a use of the tab, or expiry', () => {
    expect(cachedVerdict(rec(), { id: 4, url: 'https://a.com/y' }, 0, now)).toBeUndefined();
    expect(cachedVerdict(rec(), tab, now - 500, now)).toBeUndefined();
    expect(cachedVerdict(rec({ at: now - PROBE_TTL_MS }), tab, 0, now)).toBeUndefined();
    expect(
      cachedVerdict(rec({ at: now - PROBE_UNKNOWN_TTL_MS, verdict: 'dirty' }), tab, 0, now),
    ).toBeUndefined();
    expect(
      cachedVerdict(rec({ at: now - PROBE_UNKNOWN_TTL_MS, verdict: 'unknown' }), tab, 0, now),
    ).toBeUndefined();
    expect(cachedVerdict(rec({ at: now + 5 }), tab, 0, now)).toBeUndefined();
    expect(cachedVerdict(rec(), undefined, 0, now)).toBeUndefined();
    expect(cachedVerdict(rec(), { id: 5, url: tab.url }, 0, now)).toBeUndefined();
  });
});

describe('formSignal / parseProbeRecord / mapLimit', () => {
  it('maps verdicts to policy signals', () => {
    expect(formSignal('dirty')).toBe(true);
    expect(formSignal('clean')).toBe(false);
    expect(formSignal('unknown')).toBe('unknown');
    expect(formSignal(undefined)).toBe('unknown');
  });

  it('keeps only well-formed stored entries', () => {
    expect(
      parseProbeRecord({
        '1': { url: 'u', at: 1, verdict: 'dirty' },
        '2': { url: 'u', at: 1, verdict: 'maybe' },
        '3': null,
        '4': { url: 5, at: 1, verdict: 'clean' },
      }),
    ).toEqual({ '1': { url: 'u', at: 1, verdict: 'dirty' } });
    expect(parseProbeRecord('junk')).toEqual({});
    expect(parseProbeRecord(undefined)).toEqual({});
  });

  it('createLimiter caps tasks started from many places', async () => {
    const limit = createLimiter(2);
    let inFlight = 0;
    let peak = 0;
    const results = await Promise.all(
      [1, 2, 3, 4, 5].map((i) =>
        limit(async () => {
          inFlight++;
          peak = Math.max(peak, inFlight);
          await new Promise((r) => setTimeout(r, 1));
          inFlight--;
          return i * 10;
        }),
      ),
    );
    expect(results).toEqual([10, 20, 30, 40, 50]);
    expect(peak).toBe(2);
    await expect(limit(async () => Promise.reject(new Error('x')))).rejects.toThrow('x');
    expect(await limit(async () => 'still works')).toBe('still works');
  });

  it('runs every task with bounded concurrency', async () => {
    let inFlight = 0;
    let peak = 0;
    const done: number[] = [];
    await mapLimit([1, 2, 3, 4, 5, 6, 7], 3, async (i) => {
      inFlight++;
      peak = Math.max(peak, inFlight);
      await new Promise((r) => setTimeout(r, 1));
      done.push(i);
      inFlight--;
    });
    expect(done.sort()).toEqual([1, 2, 3, 4, 5, 6, 7]);
    expect(peak).toBe(3);
  });
});
