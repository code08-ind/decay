import { setLocale } from '@decay/ui';
import { useCallback, useEffect, useRef, useState } from 'react';
import { browser } from 'wxt/browser';
import { Popup } from '@/components/Popup';
import { appSearchResult, withAppArchive } from '@/lib/bridge/archive';
import { type AppLinkStatus, linkEnabled, readLinkStatus } from '@/lib/bridge/chrome';
import type { PopupProfile } from '@/lib/bridge/protocol';
import { db } from '@/lib/db/db';
import type { Item } from '@/lib/db/types';
import { loadPopupData, type PopupData } from '@/lib/popup/data';
import { probeAccess as loadProbeAccess, probeStore, requestProbeAccess } from '@/lib/probe/chrome';
import { RuleError, rules } from '@/lib/rules/rules';
import { ArchiveSearch } from '@/lib/search/search';
import { settings } from '@/lib/settings/settings';
import { protectRuleFor } from '@/lib/shadow/actions';
import { t } from '@/lib/strings';
import { sendDecay } from '@/lib/sweep/messages';
import type { TabInfo } from '@/lib/tracker/tabs';
import { faviconFor } from '@/lib/ui/favicon';
import { applyTheme } from '@/lib/ui/theme';

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));
const isMac = /Mac/i.test(navigator.platform || navigator.userAgent);

/** Extension popup (Designs/15): status, search across the archive, restore, undo. */
export function App() {
  const [data, setData] = useState<PopupData | null>(null);
  const [now, setNow] = useState(Date.now());
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [appLink, setAppLink] = useState<{ status: AppLinkStatus | null; enabled: boolean }>({
    status: null,
    enabled: false,
  });
  /** The app is connected and answered: search, restore and Undo go through it (P1.5). */
  const [viaApp, setViaApp] = useState(false);
  const [appProfile, setAppProfile] = useState<PopupProfile | null>(null);

  const load = useCallback(async () => {
    try {
      const [all, tabs, [current], probes, probeAccess] = await Promise.all([
        settings.getAll(),
        browser.tabs.query({}) as Promise<TabInfo[]>,
        browser.tabs.query({ active: true, currentWindow: true }) as Promise<TabInfo[]>,
        probeStore.load(),
        loadProbeAccess(),
      ]);
      applyTheme(all.theme);
      setLocale(all.locale);
      const [linkStatus, enabled] = await Promise.all([readLinkStatus(), linkEnabled()]);
      setAppLink({ status: linkStatus, enabled });
      const t = Date.now();
      let popup = await loadPopupData(
        db,
        all,
        tabs.filter((tab) => !tab.incognito),
        current,
        t,
        { probes, probeAccess },
      );
      // While the app is connected its archive is the real one (it archives tabs itself).
      let app = false;
      if (linkStatus?.state === 'connected') {
        try {
          const r = await sendDecay({ type: 'decay.search', query: '', limit: 50 });
          if (r.search) {
            popup = withAppArchive(popup, r.search);
            app = true;
            setAppProfile(r.search.profile ?? null);
          }
        } catch {
          // The link just dropped: the local copy is shown instead.
        }
      }
      setViaApp(app);
      setData(popup);
      setNow(t);
    } catch (e) {
      setError(t('load.decay', { error: message(e) }));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  // The index is built on the first keystroke, not on open: most opens never search.
  const indexCache = useRef<{ source: Item[]; index: ArchiveSearch } | null>(null);
  const getSearch = useCallback(() => {
    const source = data?.archived ?? [];
    if (indexCache.current?.source !== source) {
      indexCache.current = { source, index: new ArchiveSearch(source, data?.windowOf) };
    }
    return indexCache.current.index;
  }, [data?.archived, data?.windowOf]);

  const searchApp = useCallback(async (query: string) => {
    const r = await sendDecay({ type: 'decay.search', query, limit: 50 });
    if (!r.search) throw new Error(t('popup.noAnswer'));
    return appSearchResult(r.search, Date.now());
  }, []);

  // Metrics (P0.8): one search per popup opening; time from the first keystroke to a
  // restore picked from the results.
  const firstKeyAt = useRef<number | null>(null);
  const searchCounted = useRef(false);
  const onQueryChange = (q: string) => {
    if (!q)
      firstKeyAt.current = null; // cleared: the next search starts over
    else if (firstKeyAt.current === null) firstKeyAt.current = performance.now();
    if (q.trim().length >= 2 && !searchCounted.current) {
      searchCounted.current = true;
      void sendDecay({ type: 'decay.searched' }).catch(() => undefined);
    }
  };
  /**
   * Search → restore time for a restore picked from the results, sent with the restore
   * request (the worker records it if the restore succeeds). One sample per search: the
   * timer stops here even if the restore fails, so a later retry can't add idle time.
   */
  const takeSearchMs = (fromSearch?: boolean) => {
    const start = firstKeyAt.current;
    firstKeyAt.current = null;
    return fromSearch && start !== null ? Math.round(performance.now() - start) : undefined;
  };

  const act = async (task: () => Promise<string | null>) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    setStatus(null);
    try {
      setStatus(await task());
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
      await load();
    }
  };

  const open = (path: '/review.html' | '/sweeps.html' | '/insights.html') => {
    void browser.tabs.create({ url: browser.runtime.getURL(path) });
    window.close();
  };

  // Enter: reopen as the active tab of this window, then close the popup. Private windows
  // aren't tracked, so from one the tab opens in its original (or a new) normal window.
  const onRestore = (item: Item, opts?: { fromSearch?: boolean }) =>
    act(async () => {
      const searchMs = takeSearchMs(opts?.fromSearch);
      const win = await browser.windows.getCurrent();
      const intoWindowId = win.incognito ? undefined : win.id;
      const r = await sendDecay({ type: 'decay.restore', ids: [item.id], intoWindowId, searchMs });
      if (r.restored) {
        window.close();
        return null;
      }
      if (r.skipped) return t('popup.tabOpen');
      throw new Error(t('popup.reopenFailed'));
    });

  const onRestoreBatch = (item: Item, opts?: { fromSearch?: boolean }) =>
    act(async () => {
      const searchMs = takeSearchMs(opts?.fromSearch);
      const r = await sendDecay({ type: 'decay.restoreBatch', itemId: item.id, searchMs });
      const restored = r.restored ?? 0;
      if (!restored && r.skipped && !r.failed) return t('popup.batchOpen');
      return r.failed
        ? t('popup.restoredSome', { n: restored, total: restored + r.failed })
        : t('popup.restoredBatch', { n: restored });
    });

  // The app link (P1.4): the permission must be asked for from this click.
  const onConnectApp = () =>
    act(async () => {
      const granted = await browser.permissions.request({ permissions: ['nativeMessaging'] });
      if (!granted) return null;
      await sendDecay({ type: 'decay.app.connect' });
      return null;
    });
  const onRenameProfile = (name: string | null) =>
    act(async () => {
      const r = await sendDecay({ type: 'decay.profile.rename', name });
      return r.name ? t('app.profile.renamed', { name: r.name }) : t('app.profile.cleared');
    });
  const onDisconnectApp = () =>
    act(async () => {
      await sendDecay({ type: 'decay.app.disconnect' });
      return t('popup.ownAgain');
    });
  // The link's state changes in the background (connecting → connected): follow it.
  useEffect(() => {
    const changed = (changes: Record<string, unknown>, area: string) => {
      if (area === 'session' && 'decay.appLink' in changes) void load();
    };
    browser.storage.onChanged.addListener(changed);
    return () => browser.storage.onChanged.removeListener(changed);
  }, [load]);

  const onUndoLastSweep = () =>
    act(async () => {
      const sweepId = data?.lastSweep?.sweep.id;
      const r = await sendDecay({ type: 'decay.undoSweep', sweepId });
      return t('popup.restored', { n: r.restored ?? 0 });
    });

  const onProtectSite = () =>
    act(async () => {
      const site = data?.currentSite;
      const input = site ? protectRuleFor({ url: site.url }) : null;
      if (!site || !input) return null;
      try {
        await rules.create(input);
      } catch (e) {
        if (!(e instanceof RuleError && e.code === 'DUPLICATE_RULE')) throw e;
      }
      return t('popup.neverArchiveSite', { host: site.host });
    });

  // `permissions.request` needs the click's user gesture, so it isn't wrapped in `act`
  // (which awaits state updates first). Chrome may close the popup while asking; the
  // background picks up the grant either way.
  const onAllowProbe = () => {
    if (busy) return;
    setBusy(true); // one permission request at a time (double clicks)
    requestProbeAccess()
      .then((granted) => {
        if (granted) setStatus(t('popup.probeThanks'));
      })
      .catch((e: unknown) => setError(message(e)))
      .finally(() => {
        setBusy(false);
        void load();
      });
  };

  if (!data) {
    return (
      <p className="w-[380px] p-4 text-sm text-muted" aria-busy={!error}>
        {error ?? t('common.loading')}
      </p>
    );
  }
  return (
    <Popup
      data={data}
      getSearch={getSearch}
      searchApp={viaApp ? searchApp : undefined}
      now={now}
      isMac={isMac}
      faviconFor={(url) => faviconFor(url)}
      onRestore={(item, opts) => void onRestore(item, opts)}
      onRestoreBatch={(item, opts) => void onRestoreBatch(item, opts)}
      onQueryChange={onQueryChange}
      onUndoLastSweep={() => void onUndoLastSweep()}
      onOpenReview={() => open('/review.html')}
      onOpenSweeps={() => open('/sweeps.html')}
      onOpenInsights={() => open('/insights.html')}
      onProtectSite={() => void onProtectSite()}
      appLink={appLink}
      onConnectApp={onConnectApp}
      onDisconnectApp={onDisconnectApp}
      appProfile={viaApp ? appProfile : null}
      onRenameProfile={(name) => void onRenameProfile(name)}
      onAllowProbe={onAllowProbe}
      onClose={() => window.close()}
      busy={busy}
      status={status}
      error={error}
    />
  );
}
