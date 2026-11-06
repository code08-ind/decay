// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Item, Sweep } from '@/lib/db/types';
import { describeSweep } from '@/lib/sweep/history';
import { SweepHistory } from './SweepHistory';

afterEach(cleanup);

const NOW = new Date(2026, 10, 7, 15, 0).getTime();

function entry(id: string, number: number, state: Item['state'] = 'archived') {
  const item = {
    id: `${id}-i`,
    kind: 'tab',
    appId: 'app',
    title: `Page ${id}`,
    url: `https://${id}.com/`,
    domain: `${id}.com`,
    pinned: false,
    state,
    lastSweepId: id,
    archiveReason: { type: 'unused', activeHours: 130, threshold: 120 },
    activeAgeS: 0,
    firstSeenAt: 0,
    lastUsedAt: 0,
    stateChangedAt: 0,
    restoreCount: 0,
  } as Item;
  const sweep: Sweep = {
    id,
    startedAt: NOW - number * 60_000,
    trigger: 'scheduled',
    isShadow: 0,
    status: 'done',
    itemCount: 1,
    failedCount: 0,
    items: [{ itemId: item.id, action: 'archive', prevState: 'stale', result: 'ok' }],
  };
  return { sweep, item };
}

function renderHistory(onUndo = vi.fn()) {
  const a = entry('a', 2);
  const b = entry('b', 1);
  const items = new Map([a, b].map((x) => [x.item.id, x.item]));
  const sweeps = [b.sweep, a.sweep];
  const entries = sweeps.map((s, i) => describeSweep(s, 2 - i, items, sweeps));
  render(
    <SweepHistory
      history={{ mode: 'automatic', entries }}
      now={NOW}
      onUndo={onUndo}
      onSweepNow={vi.fn()}
      onRestore={vi.fn()}
      onOpenReview={vi.fn()}
    />,
  );
  return onUndo;
}

describe('SweepHistory', () => {
  it('lists sweeps with Undo and shows the newest one in the detail panel', () => {
    const onUndo = renderHistory();
    expect(screen.getByRole('complementary', { name: 'Sweep #2' })).toBeTruthy();
    expect(screen.getByText('Unused 130 active h')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Undo sweep #1' }));
    expect(onUndo).toHaveBeenCalledWith('a');
  });

  it('keeps the detail panel closed after closing it', () => {
    renderHistory();
    fireEvent.click(screen.getByRole('button', { name: 'Close details' }));
    expect(screen.queryByRole('complementary')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: /Sweep #1/ }));
    expect(screen.getByRole('complementary', { name: 'Sweep #1' })).toBeTruthy();
  });

  it('shows items of a running sweep as closing, not kept open', () => {
    const a = entry('a', 1);
    a.sweep.status = 'running';
    const first = a.sweep.items[0];
    if (first) first.result = 'pending';
    a.item.state = 'archiving';
    const e = describeSweep(a.sweep, 1, new Map([[a.item.id, a.item]]), [a.sweep]);
    render(
      <SweepHistory
        history={{ mode: 'automatic', entries: [e] }}
        now={NOW}
        onUndo={vi.fn()}
        onSweepNow={vi.fn()}
        onRestore={vi.fn()}
        onOpenReview={vi.fn()}
      />,
    );
    expect(screen.getByText('Closing…')).toBeTruthy();
    expect(screen.queryByText(/kept open/i)).toBeNull();
    expect(screen.getByText('Archiving…')).toBeTruthy();
  });

  it('restores the selected tabs, and filters sweeps by date', () => {
    const onRestore = vi.fn();
    const a = entry('a', 1);
    const old = entry('old', 60 * 24 * 40); // 40 days ago
    const items = new Map([a, old].map((x) => [x.item.id, x.item]));
    const sweeps = [a.sweep, old.sweep];
    render(
      <SweepHistory
        history={{
          mode: 'automatic',
          entries: sweeps.map((sw, i) => describeSweep(sw, 2 - i, items, sweeps)),
        }}
        now={NOW}
        onUndo={vi.fn()}
        onSweepNow={vi.fn()}
        onOpenReview={vi.fn()}
        onRestore={onRestore}
      />,
    );
    expect(screen.queryByText('Sweep #1')).toBeNull(); // older than 30 days
    fireEvent.change(screen.getByLabelText('Show sweeps from'), { target: { value: 'all' } });
    expect(screen.getByText('Sweep #1')).toBeTruthy();
    const restoreSelected = screen.getByRole('button', { name: /Restore selected/ });
    expect((restoreSelected as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole('checkbox', { name: 'Select Page a' }));
    fireEvent.click(screen.getByRole('button', { name: 'Restore selected (1)' }));
    expect(onRestore).toHaveBeenCalledWith(['a-i']);
  });
});
