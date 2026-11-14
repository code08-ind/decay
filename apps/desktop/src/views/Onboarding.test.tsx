// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { DetectedBrowser, EngineStatus } from '../api';

const listeners = new Map<string, () => void>();
const unlisten = vi.fn();
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, cb: () => void) => {
    listeners.set(name, cb);
    return unlisten;
  }),
}));
const close = vi.fn(async () => {});
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ close }) }));
let status: EngineStatus | null = null;
vi.mock('../useStatus', () => ({ useStatus: () => status }));

const api = {
  getSettings: vi.fn(),
  detectBrowsers: vi.fn(),
  installNativeHosts: vi.fn(),
  openExtensionStore: vi.fn(),
  requestAccessibility: vi.fn(),
  completeOnboarding: vi.fn(),
  setLaunchAtLogin: vi.fn(),
  showWindow: vi.fn(),
};
vi.mock('../api', async (orig) => ({ ...(await orig<typeof import('../api')>()), api }));

// The view reads the platform once at load: test it as macOS (the Accessibility row).
Object.defineProperty(navigator, 'userAgent', { value: 'Mozilla/5.0 (Macintosh)' });
const { Onboarding } = await import('./Onboarding');

const brave: DetectedBrowser = {
  id: 'brave',
  name: 'Brave',
  installed: true,
  hostInstalled: true,
  extensionConnected: false,
  profiles: 0,
  storeUrl: '',
};

const flush = () => act(async () => {});
const click = async (name: RegExp | string) => {
  fireEvent.click(screen.getByRole('button', { name }));
  await flush();
};
/** Renders and walks to step `n` (0-based) with Continue. */
async function at(n: number) {
  render(<Onboarding />);
  await flush();
  for (let i = 0; i < n; i++) await click(/Get started|Continue/);
}

beforeEach(() => {
  vi.useRealTimers();
  status = null;
  listeners.clear();
  for (const f of Object.values(api)) f.mockReset();
  api.getSettings.mockResolvedValue({ hotkey: 'Alt+Super+D' });
  api.detectBrowsers.mockResolvedValue([brave]);
  api.installNativeHosts.mockResolvedValue({ installed: ['Brave'] });
  api.openExtensionStore.mockResolvedValue({});
  api.completeOnboarding.mockResolvedValue({});
  api.setLaunchAtLogin.mockResolvedValue({ enabled: true });
  api.showWindow.mockResolvedValue({});
});
afterEach(cleanup);

describe('Onboarding', () => {
  it('walks the steps, choosing a pace by keyboard, and finishes with the choices', async () => {
    await at(1);
    expect(screen.getByRole('heading', { name: 'How tidy do you like it?' })).toBe(
      document.activeElement,
    );
    const balanced = screen.getByRole('radio', { name: /Balanced/ });
    expect(balanced.getAttribute('aria-checked')).toBe('true');
    fireEvent.keyDown(balanced, { key: 'ArrowRight' });
    expect(screen.getByRole('radio', { name: /Aggressive/ }).getAttribute('aria-checked')).toBe(
      'true',
    );
    await click('Continue'); // connect
    await click('Continue'); // windows
    fireEvent.click(screen.getByRole('radio', { name: /Tabs only/ }));
    await click('Continue'); // shadow
    await click(/Start shadow mode/);
    expect(api.completeOnboarding).toHaveBeenCalledWith({
      preset: 'aggressive',
      windowManagement: 'off',
      launchAtLogin: true,
    });
    expect(screen.getByRole('heading', { name: "You're all set" })).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Back' })).toBeNull();
  });

  it('keeps the user on the shadow step when finishing fails, and retries', async () => {
    api.completeOnboarding.mockRejectedValueOnce({
      code: 'DB_ERROR',
      message: 'database is locked',
    });
    await at(4);
    await click(/Start shadow mode/);
    expect(screen.getByText('database is locked')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Back' })).toBeTruthy();
    await click(/Start shadow mode/);
    expect(api.completeOnboarding).toHaveBeenCalledTimes(2);
    expect(screen.getByRole('heading', { name: "You're all set" })).toBeTruthy();
  });

  it('connect step: installs the host, lists browsers, opens the store, and cleans up', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    await at(2);
    expect(api.installNativeHosts).toHaveBeenCalled();
    await click('Install extension in Brave');
    expect(api.openExtensionStore).toHaveBeenCalledWith('brave');
    expect(screen.getByText(/Add Decay in Brave/)).toBeTruthy();
    // The extension connects: the next poll shows it.
    api.detectBrowsers.mockResolvedValue([{ ...brave, extensionConnected: true, profiles: 1 }]);
    await act(async () => {
      vi.advanceTimersByTime(2100);
    });
    expect(screen.getByText('Extension connected')).toBeTruthy();
    // adapter:health re-checks at once.
    const before = api.detectBrowsers.mock.calls.length;
    await act(async () => listeners.get('adapter:health')?.());
    expect(api.detectBrowsers.mock.calls.length).toBe(before + 1);
    await click('Back');
    const after = api.detectBrowsers.mock.calls.length;
    await act(async () => {
      vi.advanceTimersByTime(5000);
    });
    expect(api.detectBrowsers.mock.calls.length).toBe(after);
    expect(unlisten).toHaveBeenCalled();
  });

  it('connect step: explains a missing connector and recovers from a failed look', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    api.installNativeHosts.mockRejectedValue({ code: 'NOT_FOUND', message: 'internal' });
    api.detectBrowsers.mockRejectedValueOnce({ code: 'INTERNAL', message: 'no home folder' });
    await at(2);
    expect(screen.getByText(/doesn't include its browser connector/)).toBeTruthy();
    expect(screen.queryByText(/internal/)).toBeNull();
    expect(screen.getByText(/couldn't look for browsers/)).toBeTruthy();
    expect(screen.queryByText(/Looking for browsers/)).toBeNull();
    await act(async () => {
      vi.advanceTimersByTime(2100);
    });
    expect(screen.queryByText(/couldn't look for browsers/)).toBeNull();
    expect(screen.getByText('Brave')).toBeTruthy();
  });

  it('window step: asks for Accessibility, then shows it allowed', async () => {
    api.requestAccessibility.mockResolvedValue({ requested: true, opened: 'prompt' });
    status = { accessibility: false } as EngineStatus;
    await at(3);
    await click(/Open System Settings/);
    expect(api.requestAccessibility).toHaveBeenCalled();
    expect(screen.getAllByText(/macOS is asking/).length).toBeGreaterThan(0);
    // Arrow keys on the button don't change the choice.
    fireEvent.keyDown(screen.getByRole('button', { name: /Open System Settings/ }), {
      key: 'ArrowDown',
    });
    expect(
      screen.getByRole('radio', { name: /Tabs and windows/ }).getAttribute('aria-checked'),
    ).toBe('true');
    cleanup();
    status = { accessibility: true } as EngineStatus;
    await at(3);
    expect(screen.getAllByText(/Allowed/).length).toBeGreaterThan(0);
  });

  it('done step: the login switch rolls back on failure, and Open Decay opens the Library', async () => {
    await at(4);
    await click(/Start shadow mode/);
    api.setLaunchAtLogin.mockRejectedValueOnce({ code: 'PERMISSION_DENIED', message: 'nope' });
    const toggle = screen.getByRole('switch', { name: 'Launch Decay at login' });
    fireEvent.click(toggle);
    await flush();
    expect(api.setLaunchAtLogin).toHaveBeenCalledWith(false);
    expect(toggle.getAttribute('aria-checked')).toBe('true');
    expect(screen.getByText('nope')).toBeTruthy();
    await click('Open Decay');
    expect(api.showWindow).toHaveBeenCalledWith('library');
    expect(close).toHaveBeenCalled();
  });
});
