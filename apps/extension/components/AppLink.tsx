import type { AppLinkStatus } from '@/lib/bridge/chrome';

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
}) {
  const { status, enabled } = props;
  const state = enabled ? (status?.state ?? 'connecting') : 'off';
  const label: Record<string, string> = {
    off: 'Have the Decay desktop app?',
    connecting: 'Connecting to the Decay app…',
    connected: 'Connected to the Decay app. It manages your tabs now.',
    unavailable:
      'The Decay app isn’t running (or isn’t installed). Decay works on its own until it is.',
    outdated: 'Update this extension to connect to the Decay app.',
  };
  return (
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
          Connect
        </button>
      ) : (
        <button
          type="button"
          onClick={props.onDisconnect}
          disabled={props.busy}
          className="text-muted underline-offset-2 hover:text-ink hover:underline disabled:opacity-50"
        >
          Disconnect
        </button>
      )}
    </div>
  );
}
