import { useState } from 'react';
import type { AppLinkStatus } from '@/lib/bridge/chrome';
import type { PopupProfile } from '@/lib/bridge/protocol';
import { t } from '@/lib/strings';

/**
 * The link to the Decay desktop app (P1.4), one quiet line in the popup: whether the app
 * is connected, and a button to connect (asks for the `nativeMessaging` permission) or to
 * go back to working on its own.
 */
export function AppLink(props: {
  status: AppLinkStatus | null;
  enabled: boolean;
  busy?: boolean;
  onConnect: () => void;
  onDisconnect: () => void;
  /** This browser profile as the app names it (P2.8), while connected. */
  profile?: PopupProfile | null;
  /** Names it in the app (`null`: unnamed). */
  onRename?: (name: string | null) => void;
}) {
  const { status, enabled } = props;
  const state = enabled ? (status?.state ?? 'connecting') : 'off';
  const label: Record<string, string> = {
    off: t('app.off'),
    connecting: t('app.connecting'),
    connected: t('app.connected'),
    unavailable: t('app.unavailable'),
    outdated: t('app.outdated'),
  };
  const showProfile = state === 'connected' && props.profile && props.onRename;
  return (
    <>
      <div
        className="mt-3 flex items-center justify-between gap-2 text-xs text-muted"
        data-testid="app-link"
      >
        <span>{label[state] ?? label.off}</span>
        {state === 'off' ? (
          <button
            type="button"
            onClick={props.onConnect}
            disabled={props.busy}
            className="rounded-full border border-line-strong px-3 py-1 text-ink hover:bg-surface-2 disabled:opacity-50"
          >
            {t('app.connect')}
          </button>
        ) : (
          <button
            type="button"
            onClick={props.onDisconnect}
            disabled={props.busy}
            className="text-muted underline-offset-2 hover:text-ink hover:underline disabled:opacity-50"
          >
            {t('app.disconnect')}
          </button>
        )}
      </div>
      {showProfile && props.profile && props.onRename && (
        <ProfileName profile={props.profile} busy={props.busy} onRename={props.onRename} />
      )}
    </>
  );
}

/** "This profile: Work · Rename" — the app tells profiles apart by this name (P2.8). */
function ProfileName(props: {
  profile: PopupProfile;
  busy?: boolean;
  onRename: (name: string | null) => void;
}) {
  const { profile } = props;
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(profile.name ?? '');
  const shown =
    profile.name ??
    (profile.hasSiblings
      ? t('app.profile.numbered', { n: profile.ordinal })
      : t('app.profile.unnamed'));
  const done = (save: boolean) => {
    setEditing(false);
    const name = draft.trim();
    if (save && name !== (profile.name ?? '')) props.onRename(name || null);
    else setDraft(profile.name ?? '');
  };
  return (
    <div className="mt-1 flex items-center justify-between gap-2 text-xs text-muted">
      {editing ? (
        <input
          // biome-ignore lint/a11y/noAutofocus: opened by the Rename click
          autoFocus
          value={draft}
          maxLength={40}
          aria-label={t('app.profile.label')}
          placeholder={t('app.profile.placeholder')}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={() => done(true)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') done(true);
            if (e.key === 'Escape') {
              e.preventDefault();
              e.stopPropagation();
              done(false);
            }
          }}
          className="h-7 min-w-0 flex-1 rounded-lg border border-line bg-surface-2 px-2 text-ink"
        />
      ) : (
        <span className="min-w-0 truncate">{t('app.profile', { name: shown })}</span>
      )}
      {!editing && (
        <button
          type="button"
          disabled={props.busy}
          onClick={() => {
            setDraft(profile.name ?? '');
            setEditing(true);
          }}
          className="shrink-0 text-muted underline-offset-2 hover:text-ink hover:underline disabled:opacity-50"
        >
          {t('app.profile.rename')}
        </button>
      )}
    </div>
  );
}
