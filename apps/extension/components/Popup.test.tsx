// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Item, Sweep } from '@/lib/db/types';
import type { PopupData } from '@/lib/popup/data';
import { ArchiveSearch } from '@/lib/search/search';
import { Popup, type PopupProps } from './Popup';

afterEach(cleanup);

const NOW = new Date(2026, 10, 7, 15, 0).getTime();

const archived = (id: string, title: string, url: string): Item =>
  ({
    id,
    kind: 'tab',
    appId: 'com.google.Chrome',
    title,
    url,
    domain: new URL(url).hostname,
    pinned: false,
    state: 'archived',
    archivedAt: NOW - 3600_000,
    lastSweepId: 's1',
    activeAgeS: 0,
    firstSeenAt: 0,
    lastUsedAt: 0,
    stateChangedAt: 0,
    restoreCount: 0,
  }) as Item;

const items = [
  archived('a', 'Stripe pricing', 'https://stripe.com/pricing'),
  archived('b', 'Stripe docs', 'https://stripe.com/docs'),
];

function data(over: Partial<PopupData> = {}): PopupData {
  return {
    mode: 'automatic',
    holdUntil: null,
    probeAccess: 'all',
    shadow: { dayNumber: 3, targetDays: 7, ready: false, wouldArchive: 4 },
    openTabs: 41,
    archivedToday: 2,
    lastSweep: {
      sweep: { id: 's1', status: 'done', itemCount: 2, startedAt: NOW - 3600_000 } as Sweep,
      remaining: 2,
    },
    recent: items,
    archived: items,
    windowOf: new Map(),
    currentSite: { host: 'github.com', url: 'https://github.com/', protected: false },
    ...over,
  };
}

function renderPopup(over: Partial<PopupProps> = {}) {
  const props: PopupProps = {
    data: data(),
    getSearch: () => new ArchiveSearch(items),
    now: NOW,
    isMac: true,
    onRestore: vi.fn(),
    onRestoreBatch: vi.fn(),
    onUndoLastSweep: vi.fn(),
    onOpenReview: vi.fn(),
    onOpenSweeps: vi.fn(),
    onProtectSite: vi.fn(),
    onClose: vi.fn(),
    ...over,
  };
  render(<Popup {...props} />);
  return { props, input: screen.getByRole('combobox') };
}

describe('Popup', () => {
  it('asks for site access only while pages cannot be checked for typing', () => {
    const onAllowProbe = vi.fn();
    renderPopup({ onAllowProbe, data: data({ probeAccess: 'none' }) });
    expect(screen.getByText("Can't check for unsaved work")).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Allow checks' }));
    expect(onAllowProbe).toHaveBeenCalledOnce();
    cleanup();
    cleanup();
    renderPopup({ onAllowProbe, data: data({ probeAccess: 'some' }) });
    expect(screen.getByText('Checking only the sites you picked')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Allow all sites' })).toBeTruthy();
    cleanup();
    renderPopup({ onAllowProbe, data: data({ probeAccess: 'all' }) });
    expect(screen.queryByRole('button', { name: 'Allow checks' })).toBeNull();
  });

  it('shows status, the last sweep with Undo, and recent tabs', () => {
    const { props } = renderPopup();
    expect(screen.getByText('41')).toBeTruthy();
    expect(screen.getByText('archived today')).toBeTruthy();
    expect(screen.getByRole('heading', { name: 'Archived 2 tabs' })).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Undo' }));
    expect(props.onUndoLastSweep).toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Restore Stripe docs' }));
    expect(props.onRestore).toHaveBeenCalledWith(items[1]);
    fireEvent.click(screen.getByRole('button', { name: 'Never archive this site' }));
    expect(props.onProtectSite).toHaveBeenCalled();
  });

  it('shows shadow mode with the review link', () => {
    const { props } = renderPopup({ data: data({ mode: 'shadow', lastSweep: null }) });
    expect(screen.getByText('Shadow mode · day 3')).toBeTruthy();
    expect(screen.getByText('would archive now')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: /Review what Decay would do/ }));
    expect(props.onOpenReview).toHaveBeenCalled();
  });

  it('searches as you type; arrows move, Enter restores, Cmd+Enter restores the batch', () => {
    const { props, input } = renderPopup();
    fireEvent.change(input, { target: { value: 'stripe' } });
    const options = screen.getAllByRole('option');
    expect(options).toHaveLength(2);
    expect(options[0]?.getAttribute('aria-selected')).toBe('true');
    fireEvent.keyDown(input, { key: 'ArrowDown' });
    expect(screen.getAllByRole('option')[1]?.getAttribute('aria-selected')).toBe('true');
    const second = screen.getAllByRole('option')[1]?.textContent ?? '';
    fireEvent.keyDown(input, { key: 'Enter' });
    const restored = vi.mocked(props.onRestore).mock.calls[0]?.[0];
    expect(second).toContain(restored?.title);
    fireEvent.keyDown(input, { key: 'Enter', metaKey: true });
    expect(props.onRestoreBatch).toHaveBeenCalled();
  });

  it('Esc clears the query first, then closes', () => {
    const { props, input } = renderPopup();
    fireEvent.change(input, { target: { value: 'nothing-matches-this' } });
    expect(screen.getByText(/No archived tabs match/)).toBeTruthy();
    fireEvent.keyDown(input, { key: 'Escape' });
    expect((input as HTMLInputElement).value).toBe('');
    expect(props.onClose).not.toHaveBeenCalled();
    fireEvent.keyDown(input, { key: 'Escape' });
    expect(props.onClose).toHaveBeenCalled();
  });

  it('waits for two letters, and only reports expanded when results exist', () => {
    const getSearch = vi.fn(() => new ArchiveSearch(items));
    const { input } = renderPopup({ getSearch });
    fireEvent.change(input, { target: { value: 's' } });
    expect(screen.getByText(/Keep typing/)).toBeTruthy();
    expect(getSearch).not.toHaveBeenCalled(); // index not built yet
    expect(input.getAttribute('aria-expanded')).toBe('false');
    fireEvent.change(input, { target: { value: 'st' } });
    expect(input.getAttribute('aria-expanded')).toBe('true');
    expect(input.getAttribute('aria-controls')).toBeTruthy();
    fireEvent.change(input, { target: { value: 'zzzz' } });
    expect(input.getAttribute('aria-expanded')).toBe('false');
    expect(input.getAttribute('aria-controls')).toBeNull();
  });
});
