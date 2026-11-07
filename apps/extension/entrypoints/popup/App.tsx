import { useCallback, useEffect, useRef, useState } from 'react';
import { browser } from 'wxt/browser';
import { Popup } from '@/components/Popup';
import { db } from '@/lib/db/db';
import type { Item } from '@/lib/db/types';
import { loadPopupData, type PopupData } from '@/lib/popup/data';
import { probeAccess as loadProbeAccess, probeStore, requestProbeAccess } from '@/lib/probe/chrome';
import { RuleError, rules } from '@/lib/rules/rules';
import { ArchiveSearch } from '@/lib/search/search';
import { settings } from '@/lib/settings/settings';
import { protectRuleFor } from '@/lib/shadow/actions';
import { sendDecay } from '@/lib/sweep/messages';
import type { TabInfo } from '@/lib/tracker/tabs';
import { faviconFor } from '@/lib/ui/favicon';
import { applyTheme } from '@/lib/ui/theme';

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));
const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`;
const isMac = /Mac/i.test(navigator.platform || navigator.userAgent);

/** Extension popup (Designs/15): status, search across the archive, restore, undo. */
export function App() {
  const [data, setData] = useState<PopupData | null>(null);
  const [now, setNow] = useState(Date.now());
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

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
      const t = Date.now();
      setData(
        await loadPopupData(
          db,
          all,
          tabs.filter((tab) => !tab.incognito),
          current,
          t,
          { probes, probeAccess },
        ),
      );
      setNow(t);
    } catch (e) {
      setError(`Couldn't load Decay: ${message(e)}`);
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

  const open = (path: '/review.html' | '/sweeps.html') => {
    void browser.tabs.create({ url: browser.runtime.getURL(path) });
    window.close();
  };

  // Enter: reopen as the active tab of this window, then close the popup. Private windows
  // aren't tracked, so from one the tab opens in its original (or a new) normal window.
  const onRestore = (item: Item) =>
    act(async () => {
      const win = await browser.windows.getCurrent();
      const intoWindowId = win.incognito ? undefined : win.id;
      const r = await sendDecay({ type: 'decay.restore', ids: [item.id], intoWindowId });
      if (r.restored) {
        window.close();
        return null;
      }
      if (r.skipped) return 'That tab is already open again.';
      throw new Error("Couldn't reopen that tab. Try again, or open it from Sweeps.");
    });

  const onRestoreBatch = (item: Item) =>
    act(async () => {
      const r = await sendDecay({ type: 'decay.restoreBatch', itemId: item.id });
      const restored = r.restored ?? 0;
      if (!restored && r.skipped && !r.failed) return 'That batch is already open again.';
      return r.failed
        ? `Restored ${restored} of ${restored + r.failed} tabs.`
        : `Restored ${plural(restored, 'tab')} in a new window, in their original order.`;
    });

  const onUndoLastSweep = () =>
    act(async () => {
      const sweepId = data?.lastSweep?.sweep.id;
      const r = await sendDecay({ type: 'decay.undoSweep', sweepId });
      return `Restored ${plural(r.restored ?? 0, 'tab')}.`;
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
      return `Decay will never archive ${site.host}.`;
    });

  // `permissions.request` needs the click's user gesture, so it isn't wrapped in `act`
  // (which awaits state updates first). Chrome may close the popup while asking; the
  // background picks up the grant either way.
  const onAllowProbe = () => {
    if (busy) return;
    setBusy(true); // one permission request at a time (double clicks)
    requestProbeAccess()
      .then((granted) => {
        if (granted) setStatus('Thanks. Decay now checks pages for unsaved work.');
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
        {error ?? 'Loading…'}
      </p>
    );
  }
  return (
    <Popup
      data={data}
      getSearch={getSearch}
      now={now}
      isMac={isMac}
      faviconFor={(url) => faviconFor(url)}
      onRestore={(item) => void onRestore(item)}
      onRestoreBatch={(item) => void onRestoreBatch(item)}
      onUndoLastSweep={() => void onUndoLastSweep()}
      onOpenReview={() => open('/review.html')}
      onOpenSweeps={() => open('/sweeps.html')}
      onProtectSite={() => void onProtectSite()}
      onAllowProbe={onAllowProbe}
      onClose={() => window.close()}
      busy={busy}
      status={status}
      error={error}
    />
  );
}
