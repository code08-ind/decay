// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { EngineStatus, SweepRow } from '../api';

const listeners = new Map<string, () => void>();
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, cb: () => void) => {
    listeners.set(name, cb);
    return () => {};
  }),
}));
const hide = vi.fn(async () => {});
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ hide }) }));
vi.mock('@tauri-apps/api/app', () => ({ getVersion: vi.fn(async () => '1.4.0') }));
let status: EngineStatus | null = null;
vi.mock('../useStatus', () => ({ useStatus: () => status }));

const api = {
  listSweeps: vi.fn(),
  getSettings: vi.fn(async () => ({ hotkey: 'Ctrl+Alt+D' })),
  undoSweep: vi.fn(async () => ({})),
  hold: vi.fn(async () => ({ holdUntil: null })),
  releaseHold: vi.fn(async () => ({})),
  resume: vi.fn(async () => ({ mode: 'automatic' })),
  showWindow: vi.fn(async () => ({})),
  openSearch: vi.fn(async () => ({})),
  quit: vi.fn(async () => ({})),
};
vi.mock('../api', async (orig) => ({ ...(await orig<typeof import('../api')>()), api }));
const { Popover } = await import('./Popover');

const STATUS: EngineStatus = {
  mode: 'automatic',
  holdUntil: null,
  staleCount: 7,
  liveTabs: 41,
  browsers: [{ browser: 'chrome', profileId: 'p1', tabs: 41 }],
  sweeping: false,
  undoableSweep: 's1',
  accessibility: true,
  inMeeting: false,
} as EngineStatus;

const SWEEP: SweepRow = {
  id: 's1',
  number: 409,
  startedAt: Date.UTC(2026, 10, 4, 14, 14),
  trigger: 'scheduled',
  isShadow: false,
  status: 'done',
  itemCount: 14,
  tabs: 11,
  windows: 3,
  restored: 0,
  failed: 0,
  undoneAt: null,
  undoable: true,
};

const flush = () => act(async () => {});
const click = async (name: RegExp | string) => {
  fireEvent.click(screen.getByRole('button', { name }));
  await flush();
};

beforeEach(() => {
  status = { ...STATUS };
  listeners.clear();
  for (const f of Object.values(api)) f.mockClear();
  hide.mockClear();
  api.listSweeps.mockResolvedValue([SWEEP]);
});
afterEach(cleanup);

describe('Popover', () => {
  it('shows the counts, the mode, the version and the last sweep', async () => {
    render(<Popover />);
    await flush();
    expect(screen.getByText('41')).toBeTruthy();
    expect(screen.getByText('7')).toBeTruthy();
    expect(screen.getByText('Automatic')).toBeTruthy();
    expect(screen.getByText('v1.4.0')).toBeTruthy();
    expect(screen.getByText('Archived 14 items')).toBeTruthy();
    expect(screen.getByText('11 tabs · 3 windows')).toBeTruthy();
    expect(api.listSweeps).toHaveBeenCalledWith({ limit: 1, includeShadow: true });
  });

  it('undoes and opens the last sweep', async () => {
    render(<Popover />);
    await flush();
    await click('Undo');
    expect(api.undoSweep).toHaveBeenCalledWith('s1');
    await click('View');
    expect(api.showWindow).toHaveBeenCalledWith('library', '/sweeps/s1');
  });

  it('reloads the sweep when one completes', async () => {
    render(<Popover />);
    await flush();
    api.listSweeps.mockResolvedValue([{ ...SWEEP, id: 's2', itemCount: 3 }]);
    await act(async () => listeners.get('sweep:completed')?.());
    expect(screen.getByText('Archived 3 items')).toBeTruthy();
  });

  it('a shadow sweep says what it would do and has no Undo', async () => {
    status = { ...STATUS, mode: 'shadow', undoableSweep: null };
    api.listSweeps.mockResolvedValue([{ ...SWEEP, isShadow: true, undoable: false }]);
    render(<Popover />);
    await flush();
    expect(screen.getByText('Would archive 14 items')).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Undo' })).toBeNull();
  });

  it('says when there is no sweep yet', async () => {
    api.listSweeps.mockResolvedValue([]);
    render(<Popover />);
    await flush();
    expect(screen.getByText(/No sweeps yet/)).toBeTruthy();
  });

  it('holds sweeps, and resumes a hold or a pause', async () => {
    const { unmount } = render(<Popover />);
    await flush();
    await click('1 hour');
    expect(api.hold).toHaveBeenCalledWith('1h');
    await click('Until tomorrow');
    expect(api.hold).toHaveBeenCalledWith('untilTomorrow');
    await click('Until I resume');
    expect(api.hold).toHaveBeenCalledWith('indefinite');
    unmount();

    status = { ...STATUS, holdUntil: Date.now() + 3_600_000 };
    const held = render(<Popover />);
    await flush();
    await click('Resume');
    expect(api.releaseHold).toHaveBeenCalled();
    held.unmount();

    status = { ...STATUS, mode: 'paused' };
    render(<Popover />);
    await flush();
    await click('Resume');
    expect(api.resume).toHaveBeenCalled();
  });

  it('opens the search panel, the windows, and quits', async () => {
    render(<Popover />);
    await flush();
    await click(/Search archive/);
    expect(api.openSearch).toHaveBeenCalled();
    await click('Open Decay');
    expect(api.showWindow).toHaveBeenCalledWith('library');
    await click('Shadow review');
    expect(api.showWindow).toHaveBeenCalledWith('library', '/shadow-report');
    fireEvent.click(screen.getAllByRole('button', { name: 'Settings' })[0] as HTMLElement);
    expect(api.showWindow).toHaveBeenCalledWith('settings');
    await click('Quit');
    expect(api.quit).toHaveBeenCalled();
  });

  it('focuses the search box and hides on Esc', async () => {
    render(<Popover />);
    await flush();
    expect(document.activeElement?.textContent).toContain('Search archive');
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(hide).toHaveBeenCalled();
  });
});
