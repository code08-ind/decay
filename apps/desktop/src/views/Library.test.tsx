// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { EngineStatus, PanelItem, SweepRow } from '../api';

const listeners = new Map<string, (e: { payload: unknown }) => void>();
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, cb: (e: { payload: unknown }) => void) => {
    listeners.set(name, cb);
    return () => {};
  }),
}));
let status: EngineStatus | null = null;
let route = '/';
vi.mock('../useStatus', async () => {
  const { useState } = await import('react');
  return {
    useStatus: () => status,
    useRoute: () => useState(route),
  };
});

const api = {
  getDailySummary: vi.fn(),
  getSettings: vi.fn(),
  createRule: vi.fn(),
  libraryStats: vi.fn(),
  search: vi.fn(),
  getItem: vi.fn(),
  restoreItems: vi.fn(),
  protectItem: vi.fn(),
  deleteItems: vi.fn(),
  undoSweep: vi.fn(),
  runSweepNow: vi.fn(),
  listSweeps: vi.fn(),
  getSweep: vi.fn(),
  getShadowReport: vi.fn(),
  setMode: vi.fn(),
  resume: vi.fn(),
  showWindow: vi.fn(),
  getMetrics: vi.fn(),
  getBetaContext: vi.fn(),
  saveMetricsFile: vi.fn(),
  whatsNew: vi.fn(),
  whatsNewSeen: vi.fn(),
  openLink: vi.fn(),
};
vi.mock('../api', async (orig) => ({ ...(await orig<typeof import('../api')>()), api }));

const { Library } = await import('./Library');

const NOW = Date.now();
const item = (id: string, over: Partial<PanelItem> = {}): PanelItem => ({
  id,
  kind: 'tab',
  title: `Page ${id}`,
  url: `https://${id}.example/`,
  domain: `${id}.example`,
  appName: 'Google Chrome',
  sourceId: 's',
  sourceTitle: 'Research',
  archivedAt: NOW - 1000,
  reason: 'Unused 61 active h',
  batch: true,
  ...over,
});
const sweep = (id: string, over: Partial<SweepRow> = {}): SweepRow => ({
  id,
  number: 7,
  startedAt: NOW - 1000,
  trigger: 'scheduled',
  isShadow: false,
  status: 'done',
  itemCount: 2,
  tabs: 2,
  windows: 0,
  restored: 0,
  failed: 0,
  undoneAt: null,
  undoable: true,
  ...over,
});

const flush = () =>
  act(async () => {
    await new Promise((r) => setTimeout(r, 0));
  });
const show = async (at: string) => {
  route = at;
  render(<Library />);
  await flush();
  await flush();
};

beforeEach(() => {
  vi.useRealTimers();
  listeners.clear();
  for (const f of Object.values(api)) f.mockReset();
  status = {
    mode: 'shadow',
    holdUntil: null,
    staleCount: 1,
    liveTabs: 9,
    browsers: [{ browser: 'chrome', profileId: 'p', tabs: 9 }],
    sweeping: false,
    undoableSweep: 's1',
    accessibility: true,
    inMeeting: false,
  };
  api.whatsNew.mockResolvedValue({ version: '0.1.0', show: false });
  api.whatsNewSeen.mockResolvedValue(undefined);
  api.openLink.mockResolvedValue(undefined);
  api.libraryStats.mockResolvedValue({
    archivedTotal: 2,
    archivedToday: { tabs: 1, windows: 0 },
    restoredWeek: 1,
    openTabs: 9,
    mode: 'shadow',
    shadowDay: 3,
  });
  api.search.mockResolvedValue({
    items: [item('a'), item('b')],
    nextCursor: null,
    totalArchived: 2,
    tookMs: 1,
  });
  api.getItem.mockResolvedValue({
    ...item('a'),
    state: 'archived',
    pinned: false,
    groupTitle: null,
    firstSeenAt: NOW - 5000,
    lastUsedAt: NOW - 4000,
    activeHours: 61,
    sweepId: 's1',
    sweepNumber: 7,
    restoreCount: 0,
    events: [{ type: 'archived', at: NOW - 1000, label: 'Unused 61 active h' }],
  });
  for (const f of [api.restoreItems, api.undoSweep, api.runSweepNow, api.setMode]) {
    f.mockResolvedValue({});
  }
  api.protectItem.mockResolvedValue({ match: 'a.example' });
  api.deleteItems.mockResolvedValue({ deleted: 1 });
  api.listSweeps.mockResolvedValue([sweep('s1'), sweep('s0', { restored: 1 })]);
  api.getSweep.mockResolvedValue({
    ...sweep('s1'),
    items: [
      { ...item('a'), state: 'archived', archived: true },
      { ...item('b'), state: 'restored', archived: false },
    ],
  });
  api.getShadowReport.mockResolvedValue({
    mode: 'shadow',
    startedAt: NOW - 1e8,
    day: 7,
    days: 7,
    tabs: 2,
    windows: 0,
    reopened: 1,
    wouldArchive: [
      { ...item('a'), wouldArchiveAt: NOW, usedAfter: false, live: true },
      { ...item('b'), wouldArchiveAt: NOW, usedAfter: true, live: true },
    ],
    kept: [{ ...item('k'), by: 'pinned', label: 'Pinned', activeHours: 20 }],
    keptBy: [{ by: 'pinned', label: 'Pinned', count: 1 }],
  });
});
afterEach(cleanup);

describe('Library', () => {
  it('lists the archive with stats, filters by period and searches', async () => {
    await show('/');
    expect(screen.getByRole('heading', { name: 'Library' })).toBeTruthy();
    expect(screen.getByText('2 tabs and windows archived')).toBeTruthy();
    expect(screen.getByText(/Shadow mode · 5 days left/)).toBeTruthy();
    expect(screen.getByText('Page a')).toBeTruthy();
    fireEvent.click(screen.getByRole('radio', { name: 'Today' }));
    await flush();
    expect(api.search).toHaveBeenLastCalledWith(
      expect.objectContaining({ after: expect.any(Number), kind: undefined }),
    );
    fireEvent.click(screen.getByRole('radio', { name: 'Windows' }));
    await flush();
    expect(api.search).toHaveBeenLastCalledWith(expect.objectContaining({ kind: 'window' }));
  });

  it('opens an item, restores it, keeps its site and deletes it after confirming', async () => {
    await show('/');
    fireEvent.click(screen.getByRole('button', { name: /Page a/ }));
    await flush();
    await flush();
    expect(api.getItem).toHaveBeenCalledWith('a');
    expect(screen.getByRole('heading', { name: 'Page a' })).toBeTruthy();
    expect(screen.getByText('#7')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Restore' }));
    await flush();
    expect(api.restoreItems).toHaveBeenCalledWith(['a']);
    fireEvent.click(screen.getByRole('button', { name: 'Always keep site' }));
    await flush();
    expect(api.protectItem).toHaveBeenCalledWith('a');
    fireEvent.click(screen.getByRole('button', { name: 'Delete from the archive' }));
    expect(api.deleteItems).not.toHaveBeenCalled();
    const searches = api.search.mock.calls.length;
    fireEvent.click(screen.getByRole('button', { name: 'Delete for good' }));
    await flush();
    await flush();
    expect(api.deleteItems).toHaveBeenCalledWith(['a']);
    // The list and the stats reload, and the drawer closes.
    expect(api.search.mock.calls.length).toBeGreaterThan(searches);
    expect(screen.getByText('Deleted from the archive')).toBeTruthy();
    expect(screen.queryByRole('heading', { name: 'Page a' })).toBeNull();
  });

  it('the delete confirm can be cancelled, and Escape closes the drawer', async () => {
    await show('/item/a');
    fireEvent.click(screen.getByRole('button', { name: 'Delete from the archive' }));
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByRole('button', { name: 'Delete for good' })).toBeNull();
    fireEvent.keyDown(window, { key: 'Escape' });
    await flush();
    expect(screen.queryByRole('heading', { name: 'Page a' })).toBeNull();
    expect(api.deleteItems).not.toHaveBeenCalled();
  });

  it('a restore link asks first', async () => {
    await show('/item/a?confirm=restore');
    expect(screen.getByText(/A link asked to restore this tab/)).toBeTruthy();
    expect(api.restoreItems).not.toHaveBeenCalled();
  });

  it('reloads after a sweep and offers Undo in the toast', async () => {
    await show('/');
    const before = api.search.mock.calls.length;
    await act(async () =>
      listeners.get('sweep:completed')?.({ payload: { archived: 3, isShadow: false } }),
    );
    await flush();
    expect(api.search.mock.calls.length).toBeGreaterThan(before);
    expect(screen.getByText('Archived 3 items')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Undo' }));
    await flush();
    expect(api.undoSweep).toHaveBeenCalled();
  });

  it('sweeps: lists by day, undoes one, and restores selected items', async () => {
    await show('/sweeps/s1?confirm=undo');
    expect(screen.getByRole('heading', { name: 'Sweeps' })).toBeTruthy();
    expect(screen.getByText(/A link asked to undo this sweep/)).toBeTruthy();
    expect(api.undoSweep).not.toHaveBeenCalled();
    // In shadow mode the shadow sweeps show too.
    expect(api.listSweeps).toHaveBeenLastCalledWith({ limit: 100, includeShadow: true });
    expect(screen.getByRole('button', { name: 'Undo rest sweep #7' })).toBeTruthy();
    const drawer = screen.getByRole('heading', { name: 'Archived 2 tabs' }).closest('section');
    expect(drawer).toBeTruthy();
    fireEvent.click(screen.getByRole('checkbox', { name: 'Select Page a' }));
    expect(screen.queryByRole('checkbox', { name: 'Select Page b' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: /Restore selected/ }));
    await flush();
    expect(api.restoreItems).toHaveBeenCalledWith(['a']);
    fireEvent.click(screen.getByRole('button', { name: 'Undo whole sweep' }));
    await flush();
    expect(api.undoSweep).toHaveBeenCalledWith('s1');
    fireEvent.click(screen.getByRole('switch', { name: 'Show shadow sweeps' }));
    await flush();
    expect(api.listSweeps).toHaveBeenLastCalledWith({ limit: 100, includeShadow: false });
  });

  it('yesterday: what moved, hints that make rules, and a restore', async () => {
    api.getSettings.mockResolvedValue({ thresholds: { stale: 12, hidden: 36, archive: 60 } });
    api.createRule.mockResolvedValue({});
    api.getDailySummary.mockResolvedValue({
      archivedTabs: 14,
      hiddenWindows: 3,
      restored: 1,
      hints: [
        { kind: 'alwaysKeep', domain: 'stripe.com', count: 3 },
        { kind: 'keepLonger', domain: 'github.com', count: 6 },
      ],
      items: [
        {
          id: 'y1',
          kind: 'tab',
          title: 'Stripe pricing',
          url: 'https://stripe.com/pricing',
          domain: 'stripe.com',
        },
      ],
    });
    await show('/summary');
    expect(screen.getByRole('heading', { name: 'Yesterday' })).toBeTruthy();
    expect(screen.getByText('14')).toBeTruthy();
    const [from, to] = api.getDailySummary.mock.calls[0] as [number, number];
    expect(to - from).toBeGreaterThanOrEqual(23 * 3_600_000);
    fireEvent.click(screen.getByRole('button', { name: 'Always keep stripe.com' }));
    await flush();
    expect(api.createRule).toHaveBeenCalledWith({
      scope: 'domain',
      match: 'stripe.com',
      action: 'protect',
    });
    fireEvent.click(screen.getByRole('button', { name: 'Keep github.com longer' }));
    await flush();
    expect(api.createRule).toHaveBeenLastCalledWith(
      expect.objectContaining({
        match: 'github.com',
        action: 'customThreshold',
        thresholdHours: { stale: 24, hidden: 72, archive: 120 },
      }),
    );
    fireEvent.click(screen.getByRole('button', { name: 'Restore Stripe pricing' }));
    await flush();
    expect(api.restoreItems).toHaveBeenCalledWith(['y1']);
  });

  it('yesterday: an empty day, an error, and no "keep longer" at the limit', async () => {
    api.getSettings.mockResolvedValue({ thresholds: { stale: 200, hidden: 400, archive: 720 } });
    api.getDailySummary.mockResolvedValue({
      archivedTabs: 0,
      hiddenWindows: 0,
      restored: 0,
      hints: [{ kind: 'keepLonger', domain: 'github.com', count: 6 }],
      items: [],
    });
    await show('/summary');
    expect(screen.getByText('Nothing from yesterday is in the archive.')).toBeTruthy();
    expect(screen.queryByRole('button', { name: /longer/ })).toBeNull();
    cleanup();
    api.getDailySummary.mockRejectedValue({ code: 'DB_ERROR', message: 'database is busy' });
    await show('/summary');
    expect(screen.getByText('database is busy')).toBeTruthy();
  });

  it('shadow review: protects a site and turns on automatic mode', async () => {
    await show('/shadow-report');
    expect(screen.getByText('Day 7 of 7 in shadow mode')).toBeTruthy();
    expect(screen.getByText('Used again')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Protect a.example' }));
    await flush();
    expect(api.protectItem).toHaveBeenCalledWith('a');
    expect(screen.getByText('Always kept')).toBeTruthy();
    fireEvent.click(screen.getByRole('tab', { name: /Protected/ }));
    const panel = screen.getByRole('tabpanel', { name: /Protected/ });
    expect(within(panel).getByText('Protected: Pinned')).toBeTruthy();
    fireEvent.change(screen.getByRole('searchbox', { name: 'Filter' }), {
      target: { value: 'zzz' },
    });
    expect(within(panel).getByText('Nothing protected right now')).toBeTruthy();
    // Before the week is over it asks once more.
    fireEvent.click(screen.getByRole('button', { name: 'Turn on automatic mode' }));
    await flush();
    expect(api.setMode).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Yes, turn it on now' }));
    await flush();
    expect(api.setMode).toHaveBeenCalledWith('automatic');
    expect(screen.getByRole('heading', { name: 'Library' })).toBeTruthy();
  });

  it('insights: tiles from the app metrics, and the share file shown before saving', async () => {
    const today = new Date(NOW);
    const pad = (n: number) => String(n).padStart(2, '0');
    const day = `${today.getFullYear()}-${pad(today.getMonth() + 1)}-${pad(today.getDate())}`;
    api.getMetrics.mockResolvedValue({
      mode: 'automatic',
      days: [
        {
          day,
          activeSeconds: 7200,
          openTabsP50: 42,
          openWindowsP50: 3,
          archivedCount: 20,
          restored24hCount: 1,
          undoCount: 0,
          searchCount: 3,
          searchToRestoreMsP50: 2500,
          autoModeOn: 1,
          shadowFlaggedCount: 0,
          shadowRegretCount: 0,
          sampledHours: [9, 10],
          openTabsSamples: [40, 44],
          openWindowsSamples: [3, 3],
          searchToRestoreSamples: [2500],
        },
      ],
      topArchived: [{ domain: 'news.example', count: 12 }],
      restoredMost: [{ domain: 'docs.example', count: 4 }],
    });
    api.getBetaContext.mockResolvedValue({
      installId: 'abc123',
      appVersion: '0.1.0',
      platform: 'macos',
      browsers: ['chrome'],
      mode: 'automatic',
      preset: 'balanced',
      thresholds: { stale: 12, hidden: 36, archive: 60 },
      shadowStartedAt: NOW - 20 * 86_400_000,
      windowManagement: 'hide',
      rules: { protect: 2, exclude: 1, customThreshold: 0, closeAllowed: 0 },
    });
    api.saveMetricsFile.mockResolvedValue('/Users/me/decay-beta-metrics.json');
    await show('/insights');
    expect(screen.getByRole('heading', { name: 'Insights' })).toBeTruthy();
    expect(screen.getAllByText('42').length).toBeGreaterThan(0);
    expect(screen.getAllByText('5%').length).toBeGreaterThan(0); // 1 of 20 restored within a day
    expect(screen.getAllByText('news.example').length).toBeGreaterThan(0);
    expect(screen.getByText(/Nothing leaves it unless you share/).textContent).toContain(
      'this computer',
    );
    fireEvent.click(screen.getByRole('button', { name: /Share with beta team/ }));
    await flush();
    const dialog = screen.getByRole('dialog');
    const json = within(dialog).getByText(/decay-beta-metrics/).textContent ?? '';
    const file = JSON.parse(json);
    expect([file.version, file.client, file.installId]).toEqual([2, 'desktop', 'abc123']);
    expect(file.summary.archived).toBe(20);
    expect(json).not.toMatch(/news\.example|docs\.example/);
    fireEvent.click(within(dialog).getByRole('button', { name: /Save/ }));
    await flush();
    expect(api.saveMetricsFile).toHaveBeenCalledWith('beta', json);
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('switches screens from the tab bar and shows errors', async () => {
    api.runSweepNow.mockRejectedValueOnce({ code: 'CONFLICT', message: 'already sweeping' });
    await show('/overview');
    fireEvent.click(screen.getAllByRole('button', { name: 'Sweep now' })[0] as HTMLElement);
    await flush();
    expect(screen.getByText('already sweeping')).toBeTruthy();
    fireEvent.click(screen.getByRole('tab', { name: 'Sweeps' }));
    await flush();
    expect(screen.getByRole('heading', { name: 'Sweeps' })).toBeTruthy();
  });
});

describe("What's New", () => {
  afterEach(() => {
    route = '/';
  });

  it('shows after an update: Later puts it off for this run, Got it marks it seen', async () => {
    api.whatsNew.mockResolvedValue({ version: '0.1.0', show: true });
    render(<Library />);
    const dialog = await screen.findByRole('dialog', { name: /what’s new in decay/i });
    expect(within(dialog).getByText('Updated to 0.1.0')).toBeTruthy();
    expect(document.activeElement).toBe(within(dialog).getByRole('button', { name: 'Got it' }));
    fireEvent.click(within(dialog).getByRole('button', { name: 'Later' }));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(api.whatsNewSeen.mock.calls).toEqual([[true]]);
    cleanup();
    render(<Library />); // the app still says show (a new run): shown again
    const again = await screen.findByRole('dialog');
    fireEvent.click(within(again).getByRole('button', { name: 'Got it' }));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(api.whatsNewSeen.mock.calls).toEqual([[true], []]);
  });

  it('marks a version without notes seen, without showing anything', async () => {
    api.whatsNew.mockResolvedValue({ version: '9.9.9', show: true });
    render(<Library />);
    await vi.waitFor(() => expect(api.whatsNewSeen).toHaveBeenCalledTimes(1));
    expect(api.whatsNewSeen).toHaveBeenCalledWith();
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('opens from the Help menu over the current screen', async () => {
    route = '/sweeps';
    api.listSweeps.mockResolvedValue([]);
    render(<Library />);
    await screen.findByRole('tab', { name: 'Sweeps', selected: true });
    await act(async () => listeners.get('whats-new:open')?.({ payload: {} }));
    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByText(/major feature/i)).toBeTruthy();
    expect(screen.getByRole('tab', { name: 'Sweeps', selected: true })).toBeTruthy();
  });

  it('opens with the window (`?whats-new`); newest notes are labelled when this version has none', async () => {
    route = '/?whats-new';
    api.whatsNew.mockResolvedValue({ version: '9.9.9', show: false });
    render(<Library />);
    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByText('Latest notes · 0.1.0')).toBeTruthy();
  });

  it('keeps Escape and Tab to itself, and says when the changelog cannot open', async () => {
    route = '/?whats-new';
    api.openLink.mockRejectedValue({ code: 'IO_ERROR', message: 'Couldn’t open the link' });
    const behind = vi.fn();
    window.addEventListener('keydown', behind);
    render(<Library />);
    const dialog = await screen.findByRole('dialog');
    const buttons = within(dialog).getAllByRole('button');
    buttons.at(-1)?.focus();
    fireEvent.keyDown(window, { key: 'Tab' });
    expect(document.activeElement).toBe(buttons[0]);
    fireEvent.click(within(dialog).getByRole('button', { name: /full changelog/i }));
    expect(await within(dialog).findByRole('alert')).toBeTruthy();
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(behind).not.toHaveBeenCalled();
    window.removeEventListener('keydown', behind);
  });
});
