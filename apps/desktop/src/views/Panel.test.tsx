// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
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
  hidePanel: vi.fn(),
  protectItem: vi.fn(),
  showItem: vi.fn(),
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

    fireEvent.keyDown(input, { key: 'Enter' });
    const first = api.restoreItems.mock.calls[0];
    expect(first?.[2]).toEqual(expect.any(Number));
    await restored({ restored: 0, failed: 1, skipped: 0 }); // the browser was closed

    fireEvent.keyDown(input, { key: 'Enter' });
    const retry = api.restoreItems.mock.calls[1];
    expect(retry?.[2]).toEqual(expect.any(Number)); // still timed
    await restored({ restored: 1, failed: 0, skipped: 0 });

    fireEvent.keyDown(input, { key: 'Enter' });
    expect(api.restoreItems.mock.calls[2]?.[2]).toBeUndefined(); // one sample per search
    expect(api.recordSearch).toHaveBeenCalledTimes(1);
  });
});
