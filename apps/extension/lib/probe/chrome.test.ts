import { beforeEach, describe, expect, it, vi } from 'vitest';
import { makeTab } from '../tracker/test-utils';

type Script = { id: string; world?: string; [k: string]: unknown };

const state = vi.hoisted(() => ({
  origins: [] as string[],
  registered: [] as Script[],
  calls: [] as string[],
  execute: (_opts: { world?: string }): Promise<unknown> => Promise.resolve([]),
  session: {} as Record<string, unknown>,
}));

vi.mock('wxt/browser', () => ({
  browser: {
    permissions: {
      getAll: async () => ({ permissions: [], origins: [...state.origins] }),
      // Granted if an all-sites pattern for the scheme, or the exact site pattern, is held.
      contains: async ({ origins }: { origins: string[] }) =>
        origins.every((o) => {
          const scheme = o.split('//')[0];
          return state.origins.includes(o) || state.origins.includes(`${scheme}//*/*`);
        }),
    },
    scripting: {
      getRegisteredContentScripts: async ({ ids }: { ids: string[] }) =>
        state.registered.filter((s) => ids.includes(s.id)),
      registerContentScripts: async (scripts: Script[]) => {
        state.calls.push(`register:${scripts.map((s) => s.id).join(',')}`);
        state.registered.push(...scripts);
      },
      updateContentScripts: async (scripts: Script[]) => {
        state.calls.push(`update:${scripts.map((s) => s.id).join(',')}`);
        state.registered = state.registered.map((r) => scripts.find((s) => s.id === r.id) ?? r);
      },
      unregisterContentScripts: async ({ ids }: { ids: string[] }) => {
        state.calls.push(`unregister:${ids.join(',')}`);
        state.registered = state.registered.filter((s) => !ids.includes(s.id));
      },
      executeScript: (opts: { world?: string }) => state.execute(opts),
    },
    storage: {
      session: {
        get: async (key: string) => ({ [key]: state.session[key] }),
        set: async (items: Record<string, unknown>) => Object.assign(state.session, items),
      },
    },
  },
}));

const { probeTab, probeStore, syncProbeScripts } = await import('./chrome');
const ALL = ['http://*/*', 'https://*/*'];

const clean = (world?: string) =>
  world === 'MAIN'
    ? [{ frameId: 0, result: { tracked: true, handlers: 0, activated: true } }]
    : [{ frameId: 0, result: { tracked: true, dirty: false, unknown: false } }];

beforeEach(() => {
  state.origins = [...ALL];
  state.registered = [];
  state.calls = [];
  state.session = {};
  state.execute = async ({ world }) => clean(world);
});

describe('syncProbeScripts', () => {
  it('registers both trackers once access is granted, and leaves correct ones alone', async () => {
    await syncProbeScripts();
    expect(state.calls).toEqual(['register:decay-typing,decay-unload-watch']);
    expect(state.registered.map((s) => [s.id, s.world, s.runAt, s.allFrames])).toEqual([
      ['decay-typing', 'ISOLATED', 'document_start', true],
      ['decay-unload-watch', 'MAIN', 'document_start', true],
    ]);
    state.calls = [];
    await syncProbeScripts(); // e.g. the next worker start: no unregister gap
    expect(state.calls).toEqual([]);
  });

  it('adds a missing script and updates an outdated one', async () => {
    await syncProbeScripts();
    state.registered = state.registered
      .filter((s) => s.id !== 'decay-typing')
      .map((s) => ({ ...s, runAt: 'document_idle' }));
    state.calls = [];
    await syncProbeScripts();
    expect(state.calls).toEqual(['register:decay-typing', 'update:decay-unload-watch']);
  });

  it('runs the trackers only on the sites picked under "On specific sites"', async () => {
    state.origins = ['https://github.com/*'];
    await syncProbeScripts();
    expect(
      state.registered.every((s) => JSON.stringify(s.matches) === '["https://github.com/*"]'),
    ).toBe(true);
    state.origins = [...ALL];
    state.calls = [];
    await syncProbeScripts();
    expect(state.calls).toEqual(['update:decay-typing,decay-unload-watch']);
  });

  it('unregisters the trackers when access is gone', async () => {
    await syncProbeScripts();
    state.origins = [];
    state.calls = [];
    await syncProbeScripts();
    expect(state.calls).toEqual(['unregister:decay-typing,decay-unload-watch']);
    expect(state.registered).toEqual([]);
  });
});

describe('probeTab', () => {
  const tab = makeTab({ id: 9, url: 'https://a.test/' });

  it('combines both checks into a verdict', async () => {
    expect(await probeTab(tab)).toBe('clean');
    state.execute = async ({ world }) =>
      world === 'MAIN'
        ? [{ frameId: 0, result: { tracked: true, handlers: 1, activated: true } }]
        : clean();
    expect(await probeTab(tab)).toBe('dirty');
  });

  it('is unknown without access, for non-web, discarded or unloaded tabs, and on errors', async () => {
    expect(await probeTab(makeTab({ id: 9, url: 'chrome://settings/' }))).toBe('unknown');
    expect(await probeTab({ ...tab, discarded: true })).toBe('unknown');
    expect(await probeTab({ ...tab, status: 'unloaded' })).toBe('unknown');
    state.execute = async () => {
      throw new Error('Cannot access contents of the page');
    };
    expect(await probeTab(tab)).toBe('unknown');
    state.origins = [];
    state.execute = async ({ world }) => clean(world);
    expect(await probeTab(tab)).toBe('unknown');
  });

  it('checks pages on picked sites only', async () => {
    state.origins = ['https://a.test/*'];
    expect(await probeTab(tab)).toBe('clean');
    expect(await probeTab(makeTab({ id: 10, url: 'https://b.test/' }))).toBe('unknown');
  });

  it('is unknown when the page does not answer in time', async () => {
    vi.useFakeTimers();
    try {
      state.execute = () => new Promise(() => undefined);
      const verdict = probeTab(tab);
      await vi.advanceTimersByTimeAsync(3_100);
      expect(await verdict).toBe('unknown');
    } finally {
      vi.useRealTimers();
    }
  });
});

describe('probeStore', () => {
  it('round-trips verdicts and drops malformed ones', async () => {
    await probeStore.save({ '3': { url: 'https://a.test/', at: 1, verdict: 'dirty' } });
    expect(await probeStore.load()).toEqual({
      '3': { url: 'https://a.test/', at: 1, verdict: 'dirty' },
    });
    state.session['decay.probe'] = { '4': { url: 1 } };
    expect(await probeStore.load()).toEqual({});
  });
});

describe('site access levels', () => {
  it('reports all / some / none, ignoring non-web patterns', async () => {
    const { probeAccess, probeOrigins } = await import('./chrome');
    state.origins = ['https://*/*', 'http://*/*', 'file:///*', 'chrome://favicon/*'];
    expect(await probeOrigins()).toEqual(['http://*/*', 'https://*/*']);
    expect(await probeAccess()).toBe('all');
    state.origins = ['*://*/*'];
    expect(await probeAccess()).toBe('all');
    state.origins = ['https://github.com/*'];
    expect(await probeAccess()).toBe('some');
    state.origins = ['file:///*'];
    expect(await probeAccess()).toBe('none');
  });

  it('checks a page on a non-default port against its site grant', async () => {
    state.origins = ['https://a.test/*'];
    expect(await probeTab(makeTab({ id: 9, url: 'https://a.test:8443/x' }))).toBe('clean');
  });

  it('with some sites only, embedded pages from other sites make the verdict unknown', async () => {
    state.origins = ['https://a.test/*'];
    state.execute = async ({ world }) =>
      world === 'MAIN'
        ? clean('MAIN')
        : [
            {
              frameId: 0,
              result: {
                tracked: true,
                dirty: false,
                unknown: false,
                frameOrigins: ['https://pay.example'],
              },
            },
          ];
    expect(await probeTab(makeTab({ id: 9, url: 'https://a.test/' }))).toBe('unknown');
    state.origins = ['https://a.test/*', 'https://pay.example/*'];
    expect(await probeTab(makeTab({ id: 9, url: 'https://a.test/' }))).toBe('clean');
    state.origins = ['http://*/*', 'https://*/*'];
    expect(await probeTab(makeTab({ id: 9, url: 'https://a.test/' }))).toBe('clean');
  });
});
