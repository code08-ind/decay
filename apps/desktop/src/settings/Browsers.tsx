import { Banner, Button, DialogActions, StatusDot } from '@decay/ui';
import { listen } from '@tauri-apps/api/event';
import { ExternalLink, Globe } from 'lucide-react';
import { useEffect, useState } from 'react';
import {
  api,
  type DetectedBrowser,
  errorMessage,
  type ProfileInfo,
  profileLabel,
  profileName,
} from '../api';
import { rowDetail, rowState, sortRows } from '../onboarding/model';
import { timeLabel } from '../panel/results';
import { t } from '../strings';
import { useProfiles } from '../useStatus';
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
      <Profiles fail={props.fail} />
    </>
  );
}

// Profiles (P2.8, Designs/09e "Profiles: Work, Personal"): every browser profile seen,
// named here so the Library, search and Insights can tell them apart.
function Profiles(props: { fail: (e: unknown) => void }) {
  const profiles = useProfiles();
  if (!profiles?.length) return null;
  return (
    <Group title={t('browsers.profiles')} aside={t('browsers.profiles.hint')}>
      {profiles.map((p) => (
        <ProfileRow key={`${p.appId}|${p.profileId}`} profile={p} fail={props.fail} />
      ))}
    </Group>
  );
}

function ProfileRow(props: { profile: ProfileInfo; fail: (e: unknown) => void }) {
  const p = props.profile;
  const [draft, setDraft] = useState(p.name ?? '');
  const [saving, setSaving] = useState(false);
  const [confirming, setConfirming] = useState(false);
  // The name as last stored here: `p.name` catches up only once `profiles:changed` lands.
  const [stored, setStored] = useState(p.name ?? '');
  useEffect(() => {
    setDraft(p.name ?? '');
    setStored(p.name ?? '');
  }, [p.name]);
  const save = () => {
    const name = draft.trim();
    if (name === stored) {
      setDraft(stored);
      return;
    }
    setSaving(true);
    api
      .renameProfile(p.appId, p.profileId, name || null)
      .then((saved) => {
        setDraft(saved ?? '');
        setStored(saved ?? '');
      })
      .catch((e) => {
        setDraft(stored);
        props.fail(e);
      })
      .finally(() => setSaving(false));
  };
  const fallback = profileName({ name: null, ordinal: p.ordinal }, p.hasSiblings);
  const inputId = `profile-${p.appId}-${p.profileId}`;
  return (
    <div className="flex flex-wrap items-center gap-3 py-3 first:pt-0 last:pb-0">
      <span className="min-w-0 flex-1">
        <label htmlFor={inputId} className="block text-sm font-medium">
          {p.appName}
        </label>
        <span className="block text-xs text-muted">
          {p.connected
            ? t('browsers.profile.connected', { n: p.tabs })
            : t('browsers.profile.lastSeen', { when: timeLabel(p.lastSeenAt, Date.now()) })}
        </span>
      </span>
      <input
        id={inputId}
        value={draft}
        maxLength={40}
        disabled={saving}
        placeholder={fallback ?? t('browsers.profile.placeholder')}
        aria-label={t('browsers.profile.nameOf', { app: p.appName, n: p.ordinal })}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={save}
        onKeyDown={(e) => {
          if (e.key === 'Enter') e.currentTarget.blur();
          if (e.key === 'Escape') {
            setDraft(stored);
            e.stopPropagation();
          }
        }}
        className="h-9 w-44 rounded-lg border border-line bg-surface-2 px-3 text-sm"
      />
      {p.connected ? (
        <StatusDot status="connected" />
      ) : confirming ? (
        <span className="flex basis-full flex-wrap items-center gap-2 text-xs text-muted">
          <span className="min-w-0 flex-1">
            {t('browsers.profile.forgetConfirm', { name: profileLabel(p) })}
          </span>
          <DialogActions
            action={
              <Button
                size="sm"
                loading={saving}
                onClick={() => {
                  setSaving(true);
                  api
                    .forgetProfile(p.appId, p.profileId)
                    .catch(props.fail)
                    .finally(() => {
                      setSaving(false);
                      setConfirming(false);
                    });
                }}
              >
                {t('browsers.profile.forget')}
              </Button>
            }
            cancel={
              <Button size="sm" variant="ghost" onClick={() => setConfirming(false)}>
                {t('common.cancel')}
              </Button>
            }
          />
        </span>
      ) : (
        <Button
          size="sm"
          variant="ghost"
          disabled={saving}
          aria-label={t('browsers.profile.forgetOf', { name: profileLabel(p) })}
          onClick={() => setConfirming(true)}
        >
          {t('browsers.profile.forget')}
        </Button>
      )}
    </div>
  );
}
