import { Banner, Button, cx, LiveRegion, StatusDot, Toast } from '@decay/ui';
import { listen } from '@tauri-apps/api/event';
import {
  Bell,
  Clock,
  Globe,
  Info,
  Keyboard,
  KeyRound,
  LayoutGrid,
  Lock,
  RefreshCw,
  ShieldCheck,
} from 'lucide-react';
import { type KeyboardEvent, type ReactNode, useEffect, useRef, useState } from 'react';
import { api, errorMessage } from '../api';
import { About } from '../settings/About';
import { Aging } from '../settings/Aging';
import { Browsers } from '../settings/Browsers';
import { General } from '../settings/General';
import { TABS, type TabId, tabFor } from '../settings/model';
import { Notifications } from '../settings/Notifications';
import { Privacy } from '../settings/Privacy';
import { Protection } from '../settings/Protection';
import { type SettingsMap, useSettings } from '../settings/parts';
import { Shortcuts } from '../settings/Shortcuts';
import { Updates } from '../settings/Updates';
import { useRoute, useStatus } from '../useStatus';

const ICONS: Record<TabId, ReactNode> = {
  general: <LayoutGrid size={16} />,
  aging: <Clock size={16} />,
  protection: <ShieldCheck size={16} />,
  browsers: <Globe size={16} />,
  privacy: <Lock size={16} />,
  shortcuts: <Keyboard size={16} />,
  notifications: <Bell size={16} />,
  updates: <RefreshCw size={16} />,
  about: <Info size={16} />,
};

// The Settings window (P1.12, F23; Designs/08–09): a sidebar of tabs. What's shown
// always comes from the app: the initial read, then `settings:changed` (from any window).
export function Settings() {
  const [route, setRoute] = useRoute();
  const tab = tabFor(route);
  const status = useStatus();
  const [error, setError] = useState('');
  const [toast, setToast] = useState<{ id: number; text: string } | null>(null);
  const [licenseKey, setLicenseKey] = useState<string | null>(null);
  // Bumped when rules change here: lists reload.
  const [version, setVersion] = useState(0);
  const fail = (e: unknown) => setError(errorMessage(e));
  const notify = (text: string) => {
    setError('');
    setToast({ id: Date.now() + Math.random(), text });
  };
  const settings = useSettings(fail);
  const save = (patch: SettingsMap, done?: string) => {
    api
      .updateSettings(patch)
      .then(() => done && notify(done))
      .catch(fail);
  };
  useEffect(() => {
    // A license key from decay://license/activate waits here to be confirmed.
    const take = () =>
      api
        .takePending()
        .then((p) => p.licenseKey && setLicenseKey(p.licenseKey))
        .catch(() => {});
    take();
    const off = listen('pending', take);
    // Rules changed anywhere (⌘P "Always keep" in the panel or Library too): reload lists.
    const offRules = listen('rules:changed', () => setVersion((v) => v + 1));
    return () => {
      off.then((f) => f()).catch(() => {});
      offRules.then((f) => f()).catch(() => {});
    };
  }, []);
  const go = (id: string) => {
    setError('');
    setRoute(`/${id}`);
    try {
      history.replaceState(null, '', `#/${id}`);
    } catch {
      // not available (tests)
    }
  };
  // The sidebar is a vertical tab list: arrow keys move, Home / End jump.
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const onKey = (e: KeyboardEvent, i: number) => {
    const step = { ArrowDown: 1, ArrowUp: -1 }[e.key];
    const edge = { Home: 0, End: TABS.length - 1 }[e.key];
    if (step === undefined && edge === undefined) return;
    e.preventDefault();
    const next = edge ?? (i + (step ?? 0) + TABS.length) % TABS.length;
    const t = TABS[next];
    if (!t) return;
    go(t.id);
    refs.current[next]?.focus();
  };
  const props = { settings, save, notify, fail };
  return (
    <div className="flex h-screen bg-canvas text-ink">
      <nav aria-label="Settings" className="flex w-56 shrink-0 flex-col border-r border-line p-3">
        <div
          role="tablist"
          aria-orientation="vertical"
          aria-label="Settings"
          className="grid gap-0.5"
        >
          {TABS.map((t, i) => {
            const on = t.id === tab;
            return (
              <button
                key={t.id}
                ref={(el) => {
                  refs.current[i] = el;
                }}
                type="button"
                role="tab"
                id={`settings-tab-${t.id}`}
                aria-selected={on}
                aria-controls="settings-panel"
                tabIndex={on ? 0 : -1}
                onClick={() => go(t.id)}
                onKeyDown={(e) => onKey(e, i)}
                className={cx(
                  'flex items-center gap-2.5 rounded-lg px-3 py-2 text-left text-sm',
                  on
                    ? 'bg-lime font-medium text-lime-ink'
                    : 'text-muted hover:bg-surface-2 hover:text-ink',
                )}
              >
                <span aria-hidden>{ICONS[t.id]}</span>
                {t.label}
              </button>
            );
          })}
        </div>
        <div className="mt-auto rounded-lg border border-line px-3 py-2">
          <StatusDot
            status={status ? 'connected' : 'down'}
            label={status ? 'Engine running' : 'Engine not reachable'}
          />
        </div>
      </nav>
      <main className="min-w-0 flex-1 overflow-y-auto">
        <div
          id="settings-panel"
          role="tabpanel"
          aria-labelledby={`settings-tab-${tab}`}
          className="mx-auto grid max-w-3xl gap-4 p-6"
        >
          {licenseKey && (
            <Banner
              tone="success"
              icon={<KeyRound size={16} />}
              action={<Button onClick={() => setLicenseKey(null)}>Dismiss</Button>}
            >
              Activate the license key ending in {licenseKey.slice(-4)}? Licensing arrives in a
              later version.
            </Banner>
          )}
          {error && (
            <Banner tone="danger" action={<Button onClick={() => setError('')}>Dismiss</Button>}>
              {error}
            </Banner>
          )}
          <div>
            {tab === 'general' && <General {...props} go={go} />}
            {tab === 'aging' && <Aging {...props} />}
            {tab === 'protection' && (
              <Protection {...props} version={version} changed={() => setVersion((v) => v + 1)} />
            )}
            {tab === 'browsers' && <Browsers {...props} />}
            {tab === 'privacy' && (
              <Privacy {...props} version={version} changed={() => setVersion((v) => v + 1)} />
            )}
            {tab === 'shortcuts' && <Shortcuts {...props} />}
            {tab === 'notifications' && <Notifications {...props} />}
            {tab === 'updates' && <Updates {...props} />}
            {tab === 'about' && <About go={go} fail={fail} />}
          </div>
        </div>
      </main>
      <LiveRegion className="fixed right-6 bottom-6">
        {toast && (
          <Toast key={toast.id} onDismiss={() => setToast(null)}>
            {toast.text}
          </Toast>
        )}
      </LiveRegion>
    </div>
  );
}
