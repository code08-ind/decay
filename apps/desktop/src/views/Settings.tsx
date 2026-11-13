import {
  Banner,
  Button,
  Card,
  isThemePreference,
  LiveRegion,
  SegmentedControl,
  Shortcut,
  Switch,
  type ThemePreference,
} from '@decay/ui';
import { listen } from '@tauri-apps/api/event';
import { KeyRound } from 'lucide-react';
import { type ReactNode, useEffect, useState } from 'react';
import { api, errorMessage } from '../api';

/** Section label (Designs/09: small uppercase, not a serif heading). */
function Section(props: { title: string; children: ReactNode }) {
  return (
    <Card as="section" className="grid gap-4">
      <h2 className="text-[11px] font-semibold tracking-wider text-muted uppercase">
        {props.title}
      </h2>
      {props.children}
    </Card>
  );
}

// Settings, built from the shared components: appearance (F19), shortcut, launch at login.
// The other tabs arrive with the Settings screens. What's shown always comes from the app:
// the initial read, then `settings:changed` (sent after every change, from any window).
export function Settings() {
  const [settings, setSettings] = useState<Record<string, unknown> | null>(null);
  const [hotkey, setHotkey] = useState('');
  const [note, setNote] = useState('');
  const [error, setError] = useState('');
  const [licenseKey, setLicenseKey] = useState<string | null>(null);
  useEffect(() => {
    let changed = false;
    const offs = [
      listen<Record<string, unknown>>('settings:changed', (e) => {
        changed = true;
        setSettings(e.payload);
      }),
    ];
    api
      .getSettings()
      .then((s) => changed || setSettings(s))
      .catch((e) => setError(errorMessage(e)));
    // A license key from decay://license/activate waits here to be confirmed.
    const take = () =>
      api
        .takePending()
        .then((p) => p.licenseKey && setLicenseKey(p.licenseKey))
        .catch(() => {});
    take();
    offs.push(listen('pending', take));
    return () => {
      for (const off of offs) off.then((f) => f()).catch(() => {});
    };
  }, []);
  const fail = (e: unknown) => {
    setNote('');
    setError(errorMessage(e));
  };
  const done = (text: string) => {
    setError('');
    setNote(text);
  };
  const theme: ThemePreference = isThemePreference(settings?.theme) ? settings.theme : 'system';
  const currentHotkey = typeof settings?.hotkey === 'string' ? settings.hotkey : '';
  return (
    <main className="grid min-h-screen content-start gap-4 bg-canvas p-6">
      <h1 className="font-serif text-2xl font-normal">Settings</h1>
      {licenseKey && (
        <Banner
          tone="success"
          icon={<KeyRound size={16} />}
          action={<Button onClick={() => setLicenseKey(null)}>Dismiss</Button>}
        >
          Activate the license key ending in {licenseKey.slice(-4)}? Licensing arrives in a later
          version.
        </Banner>
      )}
      <Section title="Appearance">
        <div className="flex items-center justify-between gap-4">
          <span className="text-sm font-medium">Theme</span>
          <SegmentedControl
            label="Theme"
            value={theme}
            onChange={(t) => api.updateSettings({ theme: t }).catch(fail)}
            options={[
              { value: 'light', label: 'Light' },
              { value: 'dark', label: 'Dark' },
              { value: 'system', label: 'Match system' },
            ]}
          />
        </div>
      </Section>
      <Section title="Shortcuts">
        <div className="flex items-center justify-between gap-4 text-sm">
          <span className="font-medium">Search panel</span>
          {currentHotkey && <Shortcut accelerator={currentHotkey} />}
        </div>
        <form
          className="flex items-center gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            const wanted = hotkey.trim();
            api
              .setHotkey(wanted)
              .then(() => {
                setHotkey('');
                done(`Search panel shortcut set to ${wanted}`);
              })
              .catch(fail);
          }}
        >
          <label className="sr-only" htmlFor="hotkey">
            New shortcut for the search panel
          </label>
          <input
            id="hotkey"
            value={hotkey}
            placeholder="Type a new shortcut, e.g. Ctrl+Shift+Space"
            onChange={(e) => setHotkey(e.target.value)}
            className="h-8 flex-1 rounded-full border border-line bg-surface-2 px-3 text-xs placeholder:text-muted"
          />
          <Button type="submit" size="sm" variant="primary" disabled={!hotkey.trim()}>
            Change
          </Button>
        </form>
      </Section>
      {settings && (
        <Card as="section">
          <Switch
            label="Launch at login"
            description="Start Decay quietly when you log in."
            checked={settings.launch_at_login !== false}
            onChange={(on) =>
              api
                .setLaunchAtLogin(on)
                .then(() => done(on ? 'Decay starts at login' : "Decay doesn't start at login"))
                .catch(fail)
            }
          />
        </Card>
      )}
      <LiveRegion className="text-sm text-muted">{note}</LiveRegion>
      {error && (
        <p role="alert" className="text-sm text-danger">
          {error}
        </p>
      )}
    </main>
  );
}
