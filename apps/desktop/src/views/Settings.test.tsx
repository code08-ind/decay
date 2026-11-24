// @vitest-environment jsdom

import { listen } from '@tauri-apps/api/event';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProfileInfo, Rule, UpdateState } from '../api';

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('@tauri-apps/api/app', () => ({ getVersion: vi.fn(async () => '0.1.0') }));
let route = '/general';
let profiles: ProfileInfo[] | null = null;
vi.mock('../useStatus', async () => {
  const { useState } = await import('react');
  return {
    useStatus: () => ({ mode: 'shadow', browsers: [], liveTabs: 0 }),
    useProfiles: () => profiles,
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
  renameProfile: vi.fn(),
  forgetProfile: vi.fn(),
  openExtensionStore: vi.fn(),
  showWindow: vi.fn(),
  getLicense: vi.fn(),
  activateLicense: vi.fn(),
  deactivateLicense: vi.fn(),
  startTrial: vi.fn(),
  getUpdateState: vi.fn(),
  checkForUpdates: vi.fn(),
  installUpdate: vi.fn(),
  exportArchive: vi.fn(),
  importArchive: vi.fn(),
  confirmImport: vi.fn(),
  getDiagnosticsInfo: vi.fn(),
  openLogsFolder: vi.fn(),
  exportDiagnostics: vi.fn(),
  openLink: vi.fn(),
  showWhatsNew: vi.fn(),
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
  api.getDiagnosticsInfo.mockResolvedValue({ crashReportsAvailable: false, logsDir: '~/logs' });
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
  api.getLicense.mockResolvedValue({ tier: 'free', trialAvailable: true });
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

  it('license: a key from a link is prefilled; activating, the trial and deactivating', async () => {
    api.takePending.mockResolvedValue({ prefill: null, licenseKey: 'DECAY-ABCDE-12345' });
    api.activateLicense.mockResolvedValue({ tier: 'pro', trialAvailable: true, email: 'me@x.com' });
    await show('/general');
    await flush();
    expect(screen.getByText('You’re using Decay Free.')).toBeTruthy();
    const field = screen.getByRole('textbox', { name: 'License key' }) as HTMLTextAreaElement;
    expect(field.value).toBe('DECAY-ABCDE-12345');
    fireEvent.click(screen.getByRole('button', { name: 'Activate' }));
    await flush();
    expect(api.activateLicense).toHaveBeenCalledWith('DECAY-ABCDE-12345');
    expect(screen.getByText('Decay Pro is active.')).toBeTruthy();
    expect(screen.getByText('Licensed to me@x.com.')).toBeTruthy();
    api.deactivateLicense.mockResolvedValue({ tier: 'free', trialAvailable: true });
    fireEvent.click(screen.getByRole('button', { name: 'Deactivate on this computer' }));
    await flush();
    expect(api.deactivateLicense).toHaveBeenCalled();
    api.startTrial.mockResolvedValue({ tier: 'trial', trialDaysLeft: 14, trialAvailable: false });
    fireEvent.click(screen.getByRole('button', { name: 'Start free trial' }));
    await flush();
    expect(screen.getByText('Pro trial: 14 days left.')).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Start free trial' })).toBeNull();
  });

  it('license: a refused key shows why; a Pro refusal elsewhere offers the Pro tab', async () => {
    api.activateLicense.mockRejectedValue({
      code: 'INVALID_INPUT',
      message: 'This license key isn’t valid.',
    });
    await show('/license');
    fireEvent.change(screen.getByRole('textbox', { name: 'License key' }), {
      target: { value: 'DECAY-X' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Activate' }));
    await flush();
    expect(screen.getByText('This license key isn’t valid.')).toBeTruthy();
    api.updateSettings.mockRejectedValue({ code: 'PRO_REQUIRED', message: 'needs Pro' });
    cleanup();
    await show('/general');
    fireEvent.click(screen.getByRole('radio', { name: 'Dark' }));
    await flush();
    fireEvent.click(screen.getByRole('button', { name: 'See Decay Pro' }));
    await flush();
    expect(screen.getByRole('tab', { name: 'Decay Pro' }).getAttribute('aria-selected')).toBe(
      'true',
    );
  });

  it('privacy: batch names (Pro) are switched on and off', async () => {
    await show('/privacy');
    const toggle = screen.getByRole('switch', { name: 'Name archived batches on this computer' });
    expect(toggle.getAttribute('aria-checked')).toBe('false');
    fireEvent.click(toggle);
    expect(api.updateSettings).toHaveBeenCalledWith({ batch_labels: true });
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

  it('notifications: switches and the summary time are saved', async () => {
    await show('/notifications');
    fireEvent.click(screen.getByRole('switch', { name: 'Notify after each sweep' }));
    expect(api.updateSettings).toHaveBeenCalledWith({
      notifications: {
        summary: true,
        sweep: true,
        sweepBigOnly: false,
        browser: true,
        permission: true,
      },
    });
    // A second toggle right after keeps the first (no broadcast in between).
    fireEvent.click(screen.getByRole('switch', { name: 'Tell me if a browser disconnects' }));
    expect(api.updateSettings).toHaveBeenLastCalledWith({
      notifications: {
        summary: true,
        sweep: true,
        sweepBigOnly: false,
        browser: false,
        permission: true,
      },
    });
    fireEvent.change(screen.getByLabelText('Morning summary time'), { target: { value: '07:30' } });
    expect(api.updateSettings).toHaveBeenLastCalledWith({ daily_summary_time: '07:30' });
    // Sweep notices are on now (shown at once): their "only above 10" switch is usable.
    const big = screen.getByRole('switch', { name: 'Only when more than 10 items are tidied' });
    expect((big as HTMLInputElement).disabled).toBe(false);
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

  it('privacy: exports keep focus, say they are busy, and name the file', async () => {
    let done: (v: { path: string; count: number }) => void = () => {};
    api.exportArchive.mockReturnValue(new Promise((r) => (done = r)));
    await show('/privacy');
    const csv = screen.getByRole('button', { name: 'Export CSV' });
    csv.focus();
    fireEvent.click(csv);
    await flush();
    expect(screen.getByText('Exporting…')).toBeTruthy();
    expect(csv.getAttribute('aria-disabled')).toBe('true');
    expect(document.activeElement).toBe(csv);
    fireEvent.click(screen.getByRole('button', { name: 'Export JSON' }));
    expect(api.exportArchive).toHaveBeenCalledTimes(1);
    done({ path: '/Users/me/Decay archive.csv', count: 2 });
    await flush();
    expect(screen.getByText('Exported 2 items to Decay archive.csv')).toBeTruthy();
    expect(screen.queryByText('Exporting…')).toBeNull();
  });

  it('privacy: an import whose rules delete stored items asks first', async () => {
    api.importArchive.mockResolvedValue({
      confirm: { token: 7, purge: 3, fileName: 'other.json' },
    });
    await show('/privacy');
    fireEvent.click(screen.getByRole('button', { name: 'Import…' }));
    await flush();
    expect(
      screen.getByText(/other\.json has never-record rules that cover 3 stored items/),
    ).toBeTruthy();
    api.confirmImport.mockResolvedValue(null);
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await flush();
    expect(api.confirmImport).toHaveBeenCalledWith(7, false);

    api.confirmImport.mockReset();
    api.confirmImport.mockResolvedValue({
      done: {
        fileName: 'other.json',
        itemsImported: 5,
        itemsExisting: 0,
        itemsUpdated: 0,
        itemsLive: 0,
        itemsInvalid: 0,
        itemsExpired: 0,
        itemsExcluded: 0,
        itemsPurged: 3,
        rulesImported: 1,
        rulesExisting: 0,
        rulesInvalid: 0,
        rulesTurnedOff: 0,
        settingsImported: 0,
        linksCapped: false,
      },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Import…' }));
    await flush();
    fireEvent.click(screen.getByRole('button', { name: 'Delete 3 and import' }));
    await flush();
    expect(api.confirmImport).toHaveBeenCalledWith(7, true);
    expect(
      screen.getByText('Imported 5 items, 1 rule (3 stored items deleted by its rules)'),
    ).toBeTruthy();
  });

  it('privacy: diagnostics — crash reports opt in, logs, export', async () => {
    api.getDiagnosticsInfo.mockResolvedValue({
      crashReportsAvailable: true,
      logsDir: '~/Library/Application Support/Decay/logs',
    });
    api.exportDiagnostics.mockResolvedValue('/Users/me/Decay diagnostics 2026-11-15.zip');
    api.openLogsFolder.mockResolvedValue(undefined);
    await show('/privacy');
    const crash = screen.getByRole('switch', { name: /Share anonymous crash reports/ });
    expect(crash.getAttribute('aria-checked')).toBe('false');
    fireEvent.click(crash);
    await flush();
    expect(api.updateSettings).toHaveBeenCalledWith({ crash_reports: true });
    expect(screen.getByText(/~\/Library\/Application Support\/Decay\/logs/)).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Open logs folder' }));
    fireEvent.click(screen.getByRole('button', { name: 'Export diagnostics…' }));
    await flush();
    expect(api.openLogsFolder).toHaveBeenCalled();
    expect(screen.getByText('Diagnostics saved to Decay diagnostics 2026-11-15.zip')).toBeTruthy();
  });

  it("privacy: says so when it can't tell whether crash reports are available", async () => {
    api.getDiagnosticsInfo.mockRejectedValue({ code: 'TIMEOUT', message: 'busy' });
    await show('/privacy');
    expect(
      screen.getByText("Couldn't check whether this build can send crash reports."),
    ).toBeTruthy();
    expect(screen.getByText(/Stored on this device for troubleshooting/)).toBeTruthy();
  });

  it("privacy: crash reports can't be turned on in a build without them", async () => {
    api.getDiagnosticsInfo.mockResolvedValue({ crashReportsAvailable: false, logsDir: '~/x' });
    await show('/privacy');
    const crash = screen.getByRole('switch', { name: /Share anonymous crash reports/ });
    expect((crash as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText("This build can't send crash reports.")).toBeTruthy();
  });

  it('privacy: a refused import says why', async () => {
    api.importArchive.mockRejectedValue({
      code: 'INVALID_INPUT',
      message: "This file isn't a Decay archive.",
    });
    await show('/privacy');
    fireEvent.click(screen.getByRole('button', { name: 'Import…' }));
    await flush();
    expect(screen.getByRole('alert').textContent).toBe("This file isn't a Decay archive.");
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

  it('about: version, help, changelog, feedback, shortcuts and what is new', async () => {
    api.openLink.mockResolvedValue(undefined);
    api.showWhatsNew.mockResolvedValue(undefined);
    await show('/about');
    expect(screen.getByText('Version 0.1.0')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: /send feedback/i }));
    fireEvent.click(screen.getByRole('button', { name: /^changelog/i }));
    fireEvent.click(screen.getByRole('button', { name: /help & guides/i }));
    expect(api.openLink.mock.calls).toEqual([['feedback'], ['changelog'], ['help']]);
    fireEvent.click(screen.getByRole('button', { name: /what’s new/i }));
    expect(api.showWhatsNew).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole('button', { name: /keyboard shortcuts/i }));
    expect(await screen.findByRole('button', { name: 'Record a new shortcut' })).toBeTruthy();
  });

  it('about: a link that cannot open says so', async () => {
    api.openLink.mockRejectedValue({ code: 'IO_ERROR', message: 'Couldn’t open the link' });
    await show('/about');
    fireEvent.click(screen.getByRole('button', { name: /send feedback/i }));
    await flush();
    expect(screen.getByText(/couldn’t open the link/i)).toBeTruthy();
  });

  it('browsers: names a profile, clears a name, and Escape keeps the old one', async () => {
    const base = {
      appId: 'com.google.Chrome',
      appName: 'Google Chrome',
      hasSiblings: true,
      firstSeenAt: 0,
      lastSeenAt: Date.now(),
    };
    profiles = [
      { ...base, profileId: 'a', name: null, ordinal: 1, connected: true, tabs: 3 },
      { ...base, profileId: 'b', name: 'Work', ordinal: 2, connected: false, tabs: 0 },
    ];
    api.renameProfile.mockImplementation(async (_a: string, _p: string, n: string | null) => n);
    await show('/browsers');
    expect(screen.getByText('Connected · 3 tabs')).toBeTruthy();
    expect(screen.getByText(/^Last seen /)).toBeTruthy();
    const first = screen.getByLabelText('Name for Google Chrome profile 1') as HTMLInputElement;
    expect(first.placeholder).toBe('Profile 1');
    fireEvent.change(first, { target: { value: '  Personal ' } });
    fireEvent.blur(first);
    await flush();
    const second = screen.getByLabelText('Name for Google Chrome profile 2') as HTMLInputElement;
    expect(second.value).toBe('Work');
    fireEvent.change(second, { target: { value: 'Oops' } });
    fireEvent.keyDown(second, { key: 'Escape' });
    expect(second.value).toBe('Work');
    fireEvent.change(second, { target: { value: '' } });
    fireEvent.blur(second);
    await flush();
    expect(api.renameProfile.mock.calls).toEqual([
      ['com.google.Chrome', 'a', 'Personal'],
      ['com.google.Chrome', 'b', null],
    ]);
    // Only a disconnected profile can be forgotten, after a confirm.
    expect(screen.getAllByRole('button', { name: /^Forget / })).toHaveLength(1);
    api.forgetProfile.mockResolvedValue(undefined);
    fireEvent.click(screen.getByRole('button', { name: 'Forget Google Chrome · Work' }));
    expect(api.forgetProfile).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Forget' }));
    await flush();
    expect(api.forgetProfile).toHaveBeenCalledWith('com.google.Chrome', 'b');
    profiles = null;
  });

  it('shows errors from the app', async () => {
    api.updateSettings.mockRejectedValueOnce({ code: 'INVALID_INPUT', message: 'nope' });
    await show('/general');
    fireEvent.click(screen.getByRole('radio', { name: 'Dark' }));
    await flush();
    expect(screen.getByText('nope')).toBeTruthy();
  });
});
