import { describe, expect, it } from 'vitest';
import type { ProbeVerdict } from '../probe/probe';
import {
  Adapter,
  type AdapterDeps,
  type ChromeTab,
  type ChromeWindow,
  RECENT_USE_MS,
  windowKey,
} from './adapter';
import { type Envelope, envelope, kinds } from './protocol';

/** The value, or a test failure when it's missing. */
function must<T>(value: T | undefined | null): T {
  if (value === undefined || value === null) throw new Error('missing');
  return value;
}

const NOW = 1_000_000_000;
const OWN = 'chrome-extension://decay/';

const tab = (id: number, over: Partial<ChromeTab> = {}): ChromeTab => ({
  id,
  windowId: 1,
  index: id,
  url: `https://site${id}.com/`,
  title: `Tab ${id}`,
  pinned: false,
  active: false,
  incognito: false,
  lastAccessed: NOW - 3_600_000,
  ...over,
});

function setup(tabs: ChromeTab[], verdicts: Record<number, ProbeVerdict> = {}) {
  const sent: Envelope[] = [];
  const removed: number[] = [];
  const created: { windowId: number; url: string; focused?: boolean }[] = [];
  const groups: { tabIds: number[]; title: string }[] = [];
  let connected = true;
  let nextTab = 100;
  let nextWindow = 50;
  const windows: ChromeWindow[] = [
    { id: 1, focused: true, state: 'normal', type: 'normal', incognito: false },
    { id: 2, focused: false, type: 'normal', incognito: true },
    { id: 3, focused: false, type: 'popup', incognito: false },
  ];
  const deps: AdapterDeps = {
    queryTabs: async () => tabs,
    getWindows: async () => windows,
    getGroups: async () => [{ id: 7, title: 'Research', color: 'blue', windowId: 1 }],
    getTab: async (id) => tabs.find((t) => t.id === id),
    removeTab: async (id) => {
      removed.push(id);
    },
    createTab: async ({ windowId, url }) => {
      created.push({ windowId, url });
      return tab(nextTab++, { windowId, url });
    },
    createWindow: async (url, focused) => {
      const windowId = nextWindow++;
      created.push({ windowId, url, focused });
      return tab(nextTab++, { windowId, url });
    },
    groupTabs: async (tabIds, _w, g) => {
      groups.push({ tabIds, title: g.title });
    },
    focusTab: async () => undefined,
    probe: async (t) => verdicts[t.id ?? -1] ?? 'clean',
    send: (e) => {
      sent.push(e);
      return true;
    },
    isConnected: () => connected,
    session: () => 'S1',
    ownOrigin: OWN,
    now: () => NOW,
  };
  const adapter = new Adapter(deps);
  const configure = (excludedDomains: string[] = []) =>
    adapter.handle(envelope(kinds.config, { probeEnabled: true, excludedDomains }));
  return {
    adapter,
    configure,
    sent,
    removed,
    created,
    groups,
    disconnect: () => {
      connected = false;
    },
  };
}

const ackOf = (sent: Envelope[], cmd: Envelope) =>
  must(sent.find((e) => e.type === 'ack' && e.replyTo === cmd.id));

describe('Adapter', () => {
  it('reports nothing before the app’s config, then a snapshot without excluded sites', async () => {
    const t = setup([
      tab(1),
      tab(2, { windowId: 2, incognito: true }),
      tab(3, { url: 'https://bank.com/x' }),
      tab(4, { url: `${OWN}sweeps.html` }),
    ]);
    t.adapter.tabCreated(tab(9));
    expect(t.sent).toEqual([]);
    await t.configure(['bank.com']);
    const snap = must(t.sent.find((e) => e.type === 'snapshot')).payload as {
      windows: { id: number; tabs: { tabId: number }[] }[];
      groups: unknown[];
    };
    expect(snap.windows.map((w) => w.id)).toEqual([1]);
    expect(must(snap.windows[0]).tabs.map((x) => x.tabId)).toEqual([1]);
    expect(snap.groups).toEqual([{ id: 7, title: 'Research', color: 'blue', windowId: 1 }]);
  });

  it('reports a tab moving onto or off an excluded site once', async () => {
    const t = setup([tab(1)]);
    await t.configure(['bank.com']);
    t.sent.length = 0;
    t.adapter.tabUpdated(tab(1, { url: 'https://www.bank.com/' }), {
      url: 'https://www.bank.com/',
    });
    t.adapter.tabUpdated(tab(1, { url: 'https://www.bank.com/', title: 'Bank' }), {
      title: 'Bank',
    });
    t.adapter.tabUpdated(tab(1, { url: 'https://news.com/' }), { url: 'https://news.com/' });
    t.adapter.tabUpdated(tab(1, { url: 'https://news.com/', title: 'News' }), { title: 'News' });
    expect(t.sent.map((e) => e.type)).toEqual(['tab.removed', 'tab.created', 'tab.updated']);
    expect(t.sent[1]?.payload).toMatchObject({ tabId: 1, url: 'https://news.com/' });
  });

  it('closes only tabs that are still safe to close', async () => {
    const tabs = [
      tab(1),
      tab(2, { active: true }),
      tab(3, { pinned: true }),
      tab(4, { audible: true }),
      tab(5),
      tab(6, { url: 'https://bank.com/' }),
      tab(7, { url: 'https://moved.com/' }),
      tab(8, { lastAccessed: NOW - RECENT_USE_MS / 2 }),
      tab(10),
    ];
    const t = setup(tabs, { 5: 'dirty', 10: 'unknown' });
    await t.configure(['bank.com']);
    const cmd = envelope(kinds.tabsClose, {
      sweepId: 's1',
      tabIds: [1, 2, 3, 4, 5, 6, 7, 8, 10, 99],
      urls: { '7': 'https://site7.com/' },
    });
    await t.adapter.handle(cmd);
    expect(t.removed).toEqual([1, 10]);
    expect(ackOf(t.sent, cmd).payload).toMatchObject({
      ok: false,
      error: { code: 'UNSAFE' },
      result: {
        closed: [1, 10, 99], // 99 was already gone: that counts as closed
        unchecked: [10],
        skipped: [
          { tabId: 2, reason: 'active' },
          { tabId: 3, reason: 'pinned' },
          { tabId: 4, reason: 'audible' },
          { tabId: 5, reason: 'formInput' },
          { tabId: 6, reason: 'excluded' },
          { tabId: 7, reason: 'changed' },
          { tabId: 8, reason: 'inUse' },
        ],
      },
    });
    // Its removal event says Decay closed it; a user-closed tab says the user did.
    t.adapter.tabRemoved(1);
    t.adapter.tabRemoved(3);
    const removedEvents = t.sent.filter((e) => e.type === 'tab.removed').map((e) => e.payload);
    expect(removedEvents).toEqual([
      { tabId: 1, byUser: false },
      { tabId: 3, byUser: true },
    ]);
  });

  it('stops closing when the link drops', async () => {
    const t = setup([tab(1), tab(2)]);
    await t.configure();
    const cmd = envelope(kinds.tabsClose, { sweepId: 's', tabIds: [1, 2] });
    t.disconnect();
    await t.adapter.handle(cmd);
    expect(t.removed).toEqual([]);
  });

  it('restores into the original window only within this session, and regroups', async () => {
    const t = setup([tab(1)]);
    await t.configure();
    const cmd = envelope(kinds.tabsRestore, {
      items: [
        {
          itemId: 'a',
          url: 'https://a.com/',
          windowKey: windowKey('S1', 1),
          group: { title: 'Research', color: 'blue' },
        },
        {
          itemId: 'b',
          url: 'https://b.com/',
          windowKey: windowKey('S1', 1),
          group: { title: 'Research', color: 'blue' },
        },
        { itemId: 'c', url: 'https://c.com/', windowKey: windowKey('OLD', 1), active: true },
        { itemId: 'd', url: 'https://d.com/', windowKey: windowKey('S1', 3) }, // a popup window
        { itemId: 'e', url: 'javascript:alert(1)' },
        { itemId: 'f', url: ' data:text/html,x' },
        { itemId: 'g', url: `${OWN}x.html` },
      ],
    });
    await t.adapter.handle(cmd);
    expect(t.created).toEqual([
      { windowId: 1, url: 'https://a.com/' },
      { windowId: 1, url: 'https://b.com/' },
      { windowId: 50, url: 'https://c.com/', focused: true },
      { windowId: 51, url: 'https://d.com/', focused: false },
    ]);
    expect(t.groups).toEqual([{ tabIds: [100, 101], title: 'Research' }]);
    expect(ackOf(t.sent, cmd).payload).toMatchObject({
      ok: false,
      result: { failed: ['e', 'f', 'g'] },
    });
  });

  it('answers probes in the ack, acks focus, resync and unknown commands', async () => {
    const t = setup([tab(1), tab(2)], { 1: 'dirty', 2: 'unknown' });
    await t.configure();
    const probe = envelope(kinds.probe, { tabIds: [1, 2, 3] });
    await t.adapter.handle(probe);
    expect(ackOf(t.sent, probe).payload).toEqual({
      ok: true,
      result: {
        results: [
          { tabId: 1, dirtyForm: true, beforeUnload: false, uploading: false, unknown: false },
          { tabId: 2, dirtyForm: false, beforeUnload: false, uploading: false, unknown: true },
        ],
        missing: [3],
      },
    });
    expect(t.sent.some((e) => e.type === 'probe.result')).toBe(false);

    const focus = envelope(kinds.tabsFocus, { tabId: 404 });
    await t.adapter.handle(focus);
    expect(ackOf(t.sent, focus).payload).toMatchObject({ ok: false, error: { code: 'NOT_FOUND' } });

    const before = t.sent.filter((e) => e.type === 'snapshot').length;
    const resync = envelope(kinds.resync, {});
    await t.adapter.handle(resync);
    expect(t.sent.filter((e) => e.type === 'snapshot').length).toBe(before + 1);

    const unknown = envelope('something.new', {});
    await t.adapter.handle(unknown);
    expect(ackOf(t.sent, unknown).payload).toMatchObject({
      ok: false,
      error: { code: 'BAD_MESSAGE' },
    });
  });
});
