// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { PanelItem } from '../api';

const listeners = new Map<string, (e: { payload: unknown }) => void>();
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, cb: (e: { payload: unknown }) => void) => {
    listeners.set(name, cb);
    return () => {};
  }),
}));

const api = {
  takePending: vi.fn(),
  search: vi.fn(),
  recordSearch: vi.fn(),
  restoreItems: vi.fn(),
  restoreItemsIn: vi.fn(),
  hidePanel: vi.fn(),
  protectItem: vi.fn(),
  showItem: vi.fn(),
  getLicense: vi.fn(),
  showWindow: vi.fn(),
};
vi.mock('../api', async (orig) => ({ ...(await orig<typeof import('../api')>()), api }));

const { Panel } = await import('./Panel');

const item: PanelItem = {
  id: 'a',
  kind: 'tab',
  title: 'Rust book',
  url: 'https://doc.rust-lang.org/book/',
  domain: 'doc.rust-lang.org',
  appName: 'Google Chrome',
  sourceId: 's',
  sourceTitle: null,
  archivedAt: Date.now() - 1000,
  reason: 'Unused 61 active h',
  batch: false,
};

beforeEach(() => {
  Element.prototype.scrollIntoView = vi.fn();
  api.takePending.mockResolvedValue({ prefill: null });
  api.search.mockResolvedValue({ items: [item], nextCursor: null, totalArchived: 1 });
  api.recordSearch.mockResolvedValue(undefined);
  api.restoreItems.mockResolvedValue({ started: true });
  api.hidePanel.mockResolvedValue(undefined);
  api.getLicense.mockResolvedValue({ tier: 'free', trialAvailable: true });
  api.showWindow.mockResolvedValue(undefined);
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  listeners.clear();
});

const restored = async (payload: { restored: number; failed: number; skipped: number }) => {
  await act(async () => listeners.get('items:restored')?.({ payload }));
};

describe('Panel search metrics', () => {
  it('keeps the search time for a retry after a failed restore, once per search', async () => {
    render(<Panel />);
    const input = await screen.findByRole('combobox');
    fireEvent.change(input, { target: { value: 'rust' } });
    await screen.findByRole('option');
    expect(api.recordSearch).toHaveBeenCalledTimes(1);

    // Enter before the results are current is queued: wait for each restore.
    fireEvent.keyDown(input, { key: 'Enter' });
    await waitFor(() => expect(api.restoreItems).toHaveBeenCalledTimes(1));
    const first = api.restoreItems.mock.calls[0];
    expect(first?.[2]).toEqual(expect.any(Number));
    await restored({ restored: 0, failed: 1, skipped: 0 }); // the browser was closed

    fireEvent.keyDown(input, { key: 'Enter' });
    await waitFor(() => expect(api.restoreItems).toHaveBeenCalledTimes(2));
    const retry = api.restoreItems.mock.calls[1];
    expect(retry?.[2]).toEqual(expect.any(Number)); // still timed
    await restored({ restored: 1, failed: 0, skipped: 0 });

    fireEvent.keyDown(input, { key: 'Enter' });
    await waitFor(() => expect(api.restoreItems).toHaveBeenCalledTimes(3));
    expect(api.restoreItems.mock.calls[2]?.[2]).toBeUndefined(); // one sample per search
    expect(api.recordSearch).toHaveBeenCalledTimes(1);
  });
});

describe('Panel restore elsewhere', () => {
  it('names the missing profile and opens it in another one only when asked', async () => {
    api.restoreItems.mockRejectedValue({
      code: 'ADAPTER_UNAVAILABLE',
      message: 'Open Google Chrome · Work to restore this',
      details: {
        itemIds: ['a'],
        home: { appId: 'c', profileId: 'w', label: 'Google Chrome · Work' },
        alternatives: [{ appId: 'arc', profileId: 'p', label: 'Arc' }],
      },
    });
    api.restoreItemsIn.mockResolvedValue({ started: true });
    render(<Panel />);
    const input = await screen.findByRole('combobox');
    await screen.findByRole('option');
    fireEvent.keyDown(input, { key: 'Enter' });
    expect((await screen.findByRole('alert')).textContent).toBe(
      'Open Google Chrome · Work to restore this',
    );
    expect(api.restoreItemsIn).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Open in Arc instead' }));
    expect(api.restoreItemsIn).toHaveBeenCalledWith(['a'], 'arc', 'p');
  });
});

describe('Panel Pro', () => {
  const two = [item, { ...item, id: 'b', title: 'Cargo book' }];

  it('⌘↵ on a group needs Pro, and offers the Decay Pro tab', async () => {
    api.search.mockResolvedValue({ items: two, nextCursor: null, totalArchived: 2 });
    render(<Panel />);
    const input = await screen.findByRole('combobox');
    await screen.findAllByRole('option');
    fireEvent.keyDown(input, { key: 'Enter', metaKey: true, ctrlKey: true });
    expect(
      (await screen.findByText(/Restoring a whole group at once is part of Decay Pro/)).textContent,
    ).toBeTruthy();
    expect(api.restoreItems).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'See Decay Pro' }));
    expect(api.showWindow).toHaveBeenCalledWith('settings', '/license');
  });

  it('with Pro (or a trial) ⌘↵ restores the whole group', async () => {
    api.getLicense.mockResolvedValue({ tier: 'trial', trialDaysLeft: 3, trialAvailable: false });
    api.search.mockResolvedValue({ items: two, nextCursor: null, totalArchived: 2 });
    render(<Panel />);
    const input = await screen.findByRole('combobox');
    await screen.findAllByRole('option');
    fireEvent.keyDown(input, { key: 'Enter', metaKey: true, ctrlKey: true });
    expect(api.restoreItems.mock.calls[0]?.slice(0, 2)).toEqual([['a', 'b'], 'original']);
  });
});
