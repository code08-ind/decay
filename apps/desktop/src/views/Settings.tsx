import { listen } from '@tauri-apps/api/event';
import { useEffect, useState } from 'react';
import { api, errorMessage } from '../api';
import { useRoute } from '../useStatus';

// Placeholder until the Settings screens: the shell's two settings.
export function Settings() {
  const route = useRoute();
  const [hotkey, setHotkey] = useState('');
  const [note, setNote] = useState('');
  const [licenseKey, setLicenseKey] = useState<string | null>(null);
  useEffect(() => {
    // A license key from decay://license/activate waits here to be confirmed.
    const take = () =>
      api
        .takePending()
        .then((p) => p.licenseKey && setLicenseKey(p.licenseKey))
        .catch(() => {});
    take();
    const off = listen('pending', take);
    return () => {
      off.then((f) => f());
    };
  }, []);
  return (
    <main className="page">
      <header>
        <h1>Settings</h1>
        <p className="muted">{route}</p>
      </header>
      <section className="card">
        <form
          className="row"
          onSubmit={(e) => {
            e.preventDefault();
            api
              .setHotkey(hotkey)
              .then(() => setNote(`Hotkey set to ${hotkey}`))
              .catch((err) => setNote(errorMessage(err)));
          }}
        >
          <label>
            Search hotkey{' '}
            <input
              value={hotkey}
              placeholder="Alt+Super+D"
              onChange={(e) => setHotkey(e.target.value)}
            />
          </label>
          <button type="submit">Save</button>
        </form>
        <div className="row">
          <button
            type="button"
            onClick={() =>
              api
                .setLaunchAtLogin(true)
                .then(() => setNote('Decay starts at login'))
                .catch((e) => setNote(errorMessage(e)))
            }
          >
            Launch at login
          </button>
          <button
            type="button"
            onClick={() =>
              api
                .setLaunchAtLogin(false)
                .then(() => setNote("Decay doesn't start at login"))
                .catch((e) => setNote(errorMessage(e)))
            }
          >
            Don't launch at login
          </button>
        </div>
      </section>
      {licenseKey && (
        <section className="card" aria-label="License">
          <p>
            Activate the license key ending in <strong>{licenseKey.slice(-4)}</strong>? Licensing
            arrives in a later version.
          </p>
          <button type="button" onClick={() => setLicenseKey(null)}>
            Dismiss
          </button>
        </section>
      )}
      {note && <p role="status">{note}</p>}
    </main>
  );
}
