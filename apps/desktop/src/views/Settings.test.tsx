// @vitest-environment jsdom

import { listen } from '@tauri-apps/api/event';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Rule, UpdateState } from '../api';

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('@tauri-apps/api/app', () => ({ getVersion: vi.fn(async () => '0.1.0') }));
let route = '/general';
vi.mock('../useStatus', async () => {
  const { useState } = await import('react');
  return {
    useStatus: () => ({ mode: 'shadow', browsers: [], liveTabs: 0 }),
    useRoute: () => useState(route),
  };
});

const api = {
  getSettings: vi.fn(),
  updateSettings: vi.fn(),
  setLaunchAtLogin: vi.fn(),
  setMode: vi.fn(),
  resume: vi.fn(),
  setHotkey: vi.fn(),
  takePending: vi.fn(),
  listRules: vi.fn(),
  createRule: vi.fn(),
  updateRule: vi.fn(),
  deleteRule: vi.fn(),
  testRule: vi.fn(),
  previewExclude: vi.fn(),
  detectBrowsers: vi.fn(),
  openExtensionStore: vi.fn(),
  showWindow: vi.fn(),
  getUpdateState: vi.fn(),
  checkForUpdates: vi.fn(),
  installUpdate: vi.fn(),
};
vi.mock('../api', async (orig) => ({ ...(await orig<typeof import('../api')>()), api }));
const { Settings } = await import('./Settings');

const rule = (over: Partial<Rule>): Rule => ({
  id: 'r1',
  scope: 'domain',
  match: 'mail.google.com',
  action: 'protect',
  enabled: true,
  source: 'user',
  createdAt: 1,
  hitCount: 0,
  ...over,
});

const flush = () =>
  act(async () => {
    await new Promise((r) => setTimeout(r, 0));
  });
const show = async (at: string) => {
  route = at;
  render(<Settings />);
  await flush();
  await flush();
};

beforeEach(() => {
  for (const f of Object.values(api)) f.mockReset();
  api.getSettings.mockResolvedValue({
    theme: 'system',
    hotkey: 'Alt+Super+D',
    launch_at_login: true,
    preset: 'balanced',
    thresholds: { stale: 12, hidden: 36, archive: 60 },
    retention_days: 90,
  });
  api.takePending.mockResolvedValue({ prefill: null, licenseKey: null });
  for (const f of [
    api.updateSettings,
    api.setLaunchAtLogin,
    api.setMode,
    api.setHotkey,
    api.deleteRule,
  ]) {
    f.mockResolvedValue({});
  }
  api.listRules.mockResolvedValue([
    rule({}),
    rule({ id: 'x', match: 'bank.example', action: 'exclude' }),
  ]);
  api.testRule.mockResolvedValue({ matchesLive: 2, matchesArchived: 1, sample: [] });
  api.createRule.mockResolvedValue({ rule: rule({ id: 'n' }), purged: 0 });
  api.updateRule.mockResolvedValue({ rule: rule({}), purged: 0 });
  api.previewExclude.mockResolvedValue({ count: 3 });
  api.detectBrowsers.mockResolvedValue([]);
  api.getUpdateState.mockResolvedValue(update({}));
  api.checkForUpdates.mockResolvedValue(update({ checkedAt: Date.now() }));
  api.installUpdate.mockResolvedValue(update({ phase: 'ready', installOnQuit: true }));
});

function update(over: Partial<UpdateState>): UpdateState {
  return {
    current: '0.1.0',
    phase: 'idle',
    version: null,
    notes: null,
    date: null,
    downloaded: 0,
    total: null,
    checkedAt: null,
    error: null,
    failed: null,
    installOnQuit: false,
    installNow: false,
    ...over,
  };
}
afterEach(cleanup);

describe('Settings', () => {
  it('general: theme, mode and launch at login', async () => {
    await show('/general');
    fireEvent.click(screen.getByRole('radio', { name: 'Dark' }));
    expect(api.updateSettings).toHaveBeenCalledWith({ theme: 'dark' });
    fireEvent.click(screen.getByRole('radio', { name: 'Automatic' }));
    expect(api.setMode).toHaveBeenCalledWith('automatic');
    fireEvent.click(screen.getByRole('switch', { name: 'Launch Decay at login' }));
    expect(api.setLaunchAtLogin).toHaveBeenCalledWith(false);
  });

  it('switches tabs with the arrow keys', async () => {
    await show('/general');
    fireEvent.keyDown(screen.getByRole('tab', { name: 'General' }), { key: 'ArrowDown' });
    await flush();
    expect(screen.getByRole('heading', { name: 'Aging' })).toBeTruthy();
    expect(screen.getByRole('tab', { name: 'Aging' }).getAttribute('aria-selected')).toBe('true');
  });

  it('aging: a pace, and custom thresholds checked before saving', async () => {
    await show('/aging');
    fireEvent.click(screen.getByRole('radio', { name: 'Gentle' }));
    expect(api.updateSettings).toHaveBeenCalledWith({ preset: 'gentle' });
    const stale = screen.getByLabelText(/Mark as stale after/);
    fireEvent.change(stale, { target: { value: '40' } });
    expect(screen.getByRole('alert').textContent).toMatch(/before/);
    expect(
      (screen.getByRole('button', { name: 'Save thresholds' }) as HTMLButtonElement).disabled,
    ).toBe(true);
    fireEvent.change(stale, { target: { value: '10' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save thresholds' }));
    expect(api.updateSettings).toHaveBeenLastCalledWith({
      thresholds: { stale: 10, hidden: 36, archive: 60 },
    });
  });

  it('protection: lists rules (not the never-store ones), tests and adds a rule', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    await show('/protection');
    expect(screen.getByText('mail.google.com')).toBeTruthy();
    expect(screen.queryByText('bank.example')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'New rule' }));
    fireEvent.change(screen.getByRole('textbox', { name: 'Matches' }), {
      target: { value: 'notion.so' },
    });
    fireEvent.click(screen.getByRole('radio', { name: 'Keep longer' }));
    await act(async () => {
      vi.advanceTimersByTime(300);
    });
    await flush();
    expect(api.testRule).toHaveBeenLastCalledWith(
      expect.objectContaining({ scope: 'domain', match: 'notion.so', action: 'customThreshold' }),
    );
    expect(screen.getByText(/2 open tabs match/)).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Add rule' }));
    await flush();
    expect(api.createRule).toHaveBeenCalledWith(
      expect.objectContaining({ thresholdHours: { stale: 24, hidden: 72, archive: 120 } }),
    );
    vi.useRealTimers();
  });

  it('protection: toggles and deletes a rule after confirming', async () => {
    await show('/protection');
    fireEvent.click(screen.getByRole('switch', { name: /rule for mail.google.com/ }));
    await flush();
    expect(api.updateRule).toHaveBeenCalledWith('r1', expect.objectContaining({ enabled: false }));
    fireEvent.click(screen.getByRole('button', { name: 'Delete rule for mail.google.com' }));
    expect(api.deleteRule).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
    await flush();
    expect(api.deleteRule).toHaveBeenCalledWith('r1');
  });

  it('protection: saving a timing rule untouched keeps its own hours', async () => {
    const own = { stale: 30, hidden: 50, archive: 120 };
    api.listRules.mockResolvedValue([
      rule({ action: 'customThreshold', thresholdHours: own, match: 'notion.so' }),
    ]);
    await show('/protection');
    fireEvent.click(screen.getByRole('button', { name: 'Edit rule for notion.so' }));
    await flush();
    fireEvent.click(screen.getByRole('button', { name: 'Save rule' }));
    await flush();
    expect(api.updateRule).toHaveBeenCalledWith(
      'r1',
      expect.objectContaining({ thresholdHours: own }),
    );
  });

  it('reloads the rules when they change in another window', async () => {
    await show('/protection');
    const calls = api.listRules.mock.calls.length;
    const handler = vi
      .mocked(listen)
      .mock.calls.filter(([name]) => name === 'rules:changed')
      .pop()?.[1] as (() => void) | undefined;
    expect(handler).toBeTruthy();
    await act(async () => handler?.());
    await flush();
    expect(api.listRules.mock.calls.length).toBeGreaterThan(calls);
  });

  it('aging: Custom is not sent while the thresholds are invalid', async () => {
    await show('/aging');
    fireEvent.change(screen.getByLabelText(/Mark as stale after/), { target: { value: '40' } });
    fireEvent.click(screen.getByRole('radio', { name: 'Custom' }));
    expect(api.updateSettings).not.toHaveBeenCalled();
  });

  it('updates: checks now, and a ready release restarts or installs on quit', async () => {
    await show('/updates');
    expect(screen.getByText('Decay 0.1.0')).toBeTruthy();
    expect(screen.getByText('Not checked yet')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Check now' }));
    await flush();
    expect(api.checkForUpdates).toHaveBeenCalled();
    expect(screen.getByText(/Up to date · Last checked today/)).toBeTruthy();
    fireEvent.click(screen.getByRole('radio', { name: 'Beta' }));
    expect(api.updateSettings).toHaveBeenCalledWith({ update_channel: 'beta' });
    fireEvent.click(screen.getByRole('switch', { name: 'Download updates automatically' }));
    expect(api.updateSettings).toHaveBeenCalledWith({ auto_update: false });
    cleanup();

    api.getUpdateState.mockResolvedValue(
      update({ phase: 'ready', version: '0.2.0', notes: 'Faster search', total: 9, downloaded: 9 }),
    );
    await show('/updates');
    expect(screen.getByRole('heading', { name: 'Decay 0.2.0 is ready' })).toBeTruthy();
    expect(screen.getByText('Faster search')).toBeTruthy();
    api.installUpdate.mockResolvedValue(
      update({ phase: 'ready', version: '0.2.0', total: 9, downloaded: 9, installOnQuit: true }),
    );
    fireEvent.click(screen.getByRole('button', { name: 'Install on quit' }));
    await flush();
    expect(api.installUpdate).toHaveBeenCalledWith('onQuit');
    expect(screen.getByText('Installs when you quit Decay')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Restart to update' }));
    expect(api.installUpdate).toHaveBeenLastCalledWith('now');
  });

  it('updates: download progress and a failed check', async () => {
    api.getUpdateState.mockResolvedValue(
      update({ phase: 'downloading', version: '0.2.0', downloaded: 5, total: 10 }),
    );
    await show('/updates');
    expect(screen.getByRole('progressbar').getAttribute('aria-valuenow')).toBe('50');
    expect((screen.getByRole('button', { name: 'Check now' }) as HTMLButtonElement).disabled).toBe(
      true,
    );
    cleanup();
    api.getUpdateState.mockResolvedValue(
      update({ phase: 'error', error: 'offline', failed: 'check' }),
    );
    await show('/updates');
    expect(screen.getByText('Couldn’t check: offline')).toBeTruthy();
    cleanup();
    // A failed download keeps the release, says what failed, and offers a retry.
    api.getUpdateState.mockResolvedValue(
      update({ phase: 'error', version: '0.2.0', error: 'bad signature', failed: 'download' }),
    );
    await show('/updates');
    expect(screen.getByRole('alert').textContent).toBe('Couldn’t download: bad signature');
    fireEvent.click(screen.getByRole('button', { name: 'Try again and restart' }));
    expect(api.installUpdate).toHaveBeenLastCalledWith('now');
    cleanup();
    // While a "Restart now" waits for its download, the button can't be pressed again.
    api.getUpdateState.mockResolvedValue(
      update({ phase: 'downloading', version: '0.2.0', installNow: true }),
    );
    await show('/updates');
    expect(
      (screen.getByRole('button', { name: /Restarting when ready/ }) as HTMLButtonElement).disabled,
    ).toBe(true);
  });

  it('privacy: retention, and a never-store site confirmed with what it deletes', async () => {
    await show('/privacy');
    fireEvent.click(screen.getByRole('radio', { name: '30 days' }));
    expect(api.updateSettings).toHaveBeenCalledWith({ retention_days: 30 });
    expect(screen.getByText('bank.example')).toBeTruthy();
    fireEvent.change(screen.getByLabelText('Never record'), {
      target: { value: 'secret.example' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Add' }));
    await flush();
    expect(api.previewExclude).toHaveBeenCalled();
    expect(api.createRule).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Delete 3 and add' }));
    await flush();
    expect(api.createRule).toHaveBeenCalledWith({
      scope: 'domain',
      match: 'secret.example',
      action: 'exclude',
    });
  });

  it('shortcuts: records a key press and uses it', async () => {
    await show('/shortcuts');
    const rec = screen.getByRole('button', { name: 'Record a new shortcut' });
    fireEvent.click(rec);
    fireEvent.keyDown(rec, { key: 'k', code: 'KeyK', ctrlKey: true, altKey: true });
    fireEvent.click(screen.getByRole('button', { name: 'Use it' }));
    await flush();
    expect(api.setHotkey).toHaveBeenCalledWith('Ctrl+Alt+K');
  });

  it('shows errors from the app', async () => {
    api.updateSettings.mockRejectedValueOnce({ code: 'INVALID_INPUT', message: 'nope' });
    await show('/general');
    fireEvent.click(screen.getByRole('radio', { name: 'Dark' }));
    await flush();
    expect(screen.getByText('nope')).toBeTruthy();
  });
});
