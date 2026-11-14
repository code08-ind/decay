import { Banner, Button, StatusDot } from '@decay/ui';
import { listen } from '@tauri-apps/api/event';
import { ExternalLink, Globe } from 'lucide-react';
import { useEffect, useState } from 'react';
import { api, type DetectedBrowser, errorMessage } from '../api';
import { rowDetail, rowState, sortRows } from '../onboarding/model';
import { Group, TabHeader, type TabProps } from './parts';

// Browsers (Designs/09e): which browsers are found and connected, and installing the
// extension in one (the same detection as onboarding).
export function Browsers(props: TabProps) {
  const [rows, setRows] = useState<DetectedBrowser[] | null>(null);
  const [error, setError] = useState('');
  useEffect(() => {
    let live = true;
    const refresh = () =>
      api
        .detectBrowsers()
        .then((r) => {
          if (!live) return;
          setRows(r);
          setError('');
        })
        .catch((e) => live && setError(errorMessage(e)));
    refresh();
    const off = listen('adapter:health', refresh);
    const t = setInterval(refresh, 5000);
    return () => {
      live = false;
      clearInterval(t);
      off.then((f) => f()).catch(() => {});
    };
  }, []);
  return (
    <>
      <TabHeader title="Browsers">
        Decay sees tab activity through a small extension in each browser.
      </TabHeader>
      {error && <Banner tone="danger">{error}</Banner>}
      <Group title="Chromium browsers">
        {rows === null && !error && <p className="text-sm text-muted">Looking for browsers…</p>}
        {rows &&
          sortRows(rows).map((b) => {
            const state = rowState(b);
            return (
              <div key={b.id} className="flex items-center gap-3 py-3 first:pt-0 last:pb-0">
                <Globe size={16} aria-hidden className="text-muted" />
                <span className="min-w-0 flex-1">
                  <span className="block text-sm font-medium">{b.name}</span>
                  <span className="block text-xs text-muted">{rowDetail(b)}</span>
                </span>
                {state === 'connected' && <StatusDot status="connected" />}
                {state === 'install' && (
                  <Button
                    size="sm"
                    icon={<ExternalLink size={12} aria-hidden />}
                    aria-label={`Install extension in ${b.name}`}
                    onClick={() => api.openExtensionStore(b.id).catch(props.fail)}
                  >
                    Install extension
                  </Button>
                )}
                {state === 'missing' && <span className="text-xs text-muted">Not found</span>}
              </div>
            );
          })}
      </Group>
    </>
  );
}
