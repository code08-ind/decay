import { Banner, Button, ProBadge } from '@decay/ui';
import { listen } from '@tauri-apps/api/event';
import { Check, KeyRound, Lock } from 'lucide-react';
import { useEffect, useState } from 'react';
import { api, errorMessage, type License as LicenseState } from '../api';
import { type Key, t } from '../strings';
import { TabHeader } from './parts';

/** What Pro adds (PRD §7.4), in the order the upgrade sheet lists it (Designs/13). */
export const PRO_FEATURES: Key[] = [
  'license.feature.close',
  'license.feature.batch',
  'license.feature.retention',
  'license.feature.rules',
];

/** One line about the license. */
export function licenseLine(l: LicenseState): string {
  if (l.tier === 'pro') return t('license.status.pro');
  if (l.tier === 'trial') return t('license.status.trial', { days: l.trialDaysLeft ?? 0 });
  return l.trialAvailable ? t('license.status.free') : t('license.status.trialOver');
}

// Decay Pro (P4.2, F27; Designs/13 as a Settings tab): what Pro adds, the license's state,
// activating a key (prefilled from a decay://license/activate link), the 14-day trial
// and deactivating. Every change comes back as `license:changed`.
export function License(props: {
  /** A key from a link, waiting to be confirmed. */
  pendingKey: string | null;
  keyTaken: () => void;
  notify: (text: string) => void;
}) {
  const { pendingKey, keyTaken, notify } = props;
  const [license, setLicense] = useState<LicenseState | null>(null);
  const [key, setKey] = useState('');
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    api
      .getLicense()
      .then(setLicense)
      .catch((e) => setError(errorMessage(e)));
    const off = listen<LicenseState>('license:changed', (e) => setLicense(e.payload));
    return () => {
      off.then((f) => f()).catch(() => {});
    };
  }, []);
  useEffect(() => {
    if (pendingKey) {
      setKey(pendingKey);
      keyTaken();
      notify(t('settings.license', { last4: pendingKey.slice(-4) }));
    }
  }, [pendingKey, keyTaken, notify]);
  const act = (p: Promise<LicenseState>, done: string) => {
    setError('');
    setBusy(true);
    p.then((l) => {
      setLicense(l);
      setKey('');
      notify(done);
    })
      .catch((e) => setError(errorMessage(e)))
      .finally(() => setBusy(false));
  };
  const pro = license?.tier === 'pro';
  return (
    <>
      <TabHeader title={t('settings.tab.license')}>{t('license.subtitle')}</TabHeader>
      <div className="grid gap-4 md:grid-cols-2">
        <section
          aria-labelledby="pro-h"
          className="rounded-card bg-feature p-5 text-feature-ink shadow-sm"
        >
          <ProBadge />
          <h2 id="pro-h" className="mt-2 font-serif text-2xl">
            {t('license.pro')}
          </h2>
          <p className="text-sm">{t('license.oneTime')}</p>
          <ul className="mt-4 grid gap-2.5">
            {PRO_FEATURES.map((k) => (
              <li key={k} className="flex gap-2 text-sm">
                <Check size={16} aria-hidden className="mt-0.5 shrink-0" />
                {t(k)}
              </li>
            ))}
          </ul>
          <p className="mt-4 border-t border-current/20 pt-3 text-xs">{t('license.trialNote')}</p>
        </section>
        <section aria-labelledby="license-h" className="grid content-start gap-3">
          <h2 id="license-h" className="sr-only">
            {t('license.keyLabel')}
          </h2>
          {license && (
            <p aria-live="polite" className="text-sm">
              <span className="font-medium">{licenseLine(license)}</span>
              {pro && license.email && (
                <span className="block text-muted">
                  {t('license.status.proFor', { email: license.email })}
                </span>
              )}
            </p>
          )}
          {error && <Banner tone="danger">{error}</Banner>}
          {!pro && (
            <form
              className="grid gap-2"
              onSubmit={(e) => {
                e.preventDefault();
                if (key.trim()) act(api.activateLicense(key.trim()), t('license.activated'));
              }}
            >
              <label className="grid gap-1.5">
                <span className="text-sm">{t('license.keyLabel')}</span>
                <textarea
                  value={key}
                  onChange={(e) => setKey(e.target.value)}
                  rows={3}
                  spellCheck={false}
                  autoComplete="off"
                  placeholder={t('license.keyPlaceholder')}
                  aria-describedby="key-hint"
                  className="resize-none rounded-lg border border-line bg-surface-2 px-3 py-2 font-mono text-xs break-all"
                />
              </label>
              <p id="key-hint" className="text-xs text-muted">
                {t('license.keyHint')}
              </p>
              <Button
                type="submit"
                variant="primary"
                icon={<KeyRound size={14} aria-hidden />}
                loading={busy}
                disabled={busy || !key.trim()}
              >
                {t('license.activate')}
              </Button>
              {license?.trialAvailable && (
                <Button
                  disabled={busy}
                  onClick={() => act(api.startTrial(), t('license.trialStarted'))}
                >
                  {t('license.startTrial')}
                </Button>
              )}
            </form>
          )}
          {pro && (
            <div className="grid gap-2">
              <Button
                disabled={busy}
                onClick={() => act(api.deactivateLicense(), t('license.deactivated'))}
              >
                {t('license.deactivate')}
              </Button>
              <p className="text-xs text-muted">{t('license.deactivateHint')}</p>
            </div>
          )}
          {license && license.tier !== 'free' && (
            <p className="text-xs text-muted">{t('license.afterPro')}</p>
          )}
          <p className="flex items-start gap-1.5 border-t border-line pt-3 text-xs text-muted">
            <Lock size={12} aria-hidden className="mt-0.5 shrink-0" />
            {t('license.local')}
          </p>
        </section>
      </div>
    </>
  );
}
