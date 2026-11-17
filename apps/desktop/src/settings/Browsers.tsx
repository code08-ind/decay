import { Banner, Button, StatusDot } from '@decay/ui';
import { listen } from '@tauri-apps/api/event';
import { ExternalLink, Globe } from 'lucide-react';
import { useEffect, useState } from 'react';
import { api, type DetectedBrowser, errorMessage } from '../api';
import { rowDetail, rowState, sortRows } from '../onboarding/model';
import { t } from '../strings';
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
    const timer = setInterval(refresh, 5000);
    return () => {
      live = false;
      clearInterval(timer);
      off.then((f) => f()).catch(() => {});
    };
  }, []);
  return (
    <>
      <TabHeader title={t('settings.tab.browsers')}>{t('browsers.subtitle')}</TabHeader>
      {error && <Banner tone="danger">{error}</Banner>}
      <Group title={t('browsers.chromium')}>
        {rows === null && !error && <p className="text-sm text-muted">{t('browsers.looking')}</p>}
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
                    icon={<ExternalLink className="rtl:-scale-x-100" size={12} aria-hidden />}
                    aria-label={t('browsers.installIn', { name: b.name })}
                    onClick={() => api.openExtensionStore(b.id).catch(props.fail)}
                  >
                    {t('browsers.install')}
                  </Button>
                )}
                {state === 'missing' && (
                  <span className="text-xs text-muted">{t('browsers.notFound')}</span>
                )}
              </div>
            );
          })}
      </Group>
    </>
  );
}
