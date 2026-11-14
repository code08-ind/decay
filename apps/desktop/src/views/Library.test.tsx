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
