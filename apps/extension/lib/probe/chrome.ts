import { browser } from 'wxt/browser';
import type { TabInfo } from '../tracker/tabs';
import { inspectForms, inspectUnload } from './inspect';
import {
  combineProbe,
  isWebUrl,
  PROBE_TIMEOUT_MS,
  type ProbeRecord,
  type ProbeVerdict,
  parseProbeRecord,
} from './probe';

/**
 * Chrome side of the form probe (P0.7): site access, the typing / `beforeunload` trackers,
 * and on-demand probing with `scripting.executeScript`.
 */

/** Site access the probe needs (optional, asked for at runtime). Web pages only. */
export const PROBE_ORIGINS = ['http://*/*', 'https://*/*'];

const SCRIPTS = [
  { id: 'decay-typing', js: ['typing.js'], world: 'ISOLATED' as const },
  { id: 'decay-unload-watch', js: ['unload-watch.js'], world: 'MAIN' as const },
];

const PROBE_KEY = 'decay.probe';

export async function hasProbeAccess(): Promise<boolean> {
  try {
    return await browser.permissions.contains({ origins: PROBE_ORIGINS });
  } catch {
    return false;
  }
}

/** Must be called from a click handler (a user gesture). */
export async function requestProbeAccess(): Promise<boolean> {
  return browser.permissions.request({ origins: PROBE_ORIGINS });
}

let syncChain: Promise<unknown> = Promise.resolve();

const wanted = () =>
  SCRIPTS.map((s) => ({
    ...s,
    matches: PROBE_ORIGINS,
    allFrames: true,
    matchOriginAsFallback: true,
    runAt: 'document_start' as const,
    persistAcrossSessions: true,
  }));

type Registered = Browser.scripting.RegisteredContentScript;

const sameScript = (have: Registered, want: ReturnType<typeof wanted>[number]) =>
  JSON.stringify(have.js) === JSON.stringify(want.js) &&
  JSON.stringify(have.matches) === JSON.stringify(want.matches) &&
  have.allFrames === want.allFrames &&
  have.runAt === want.runAt &&
  (have.world ?? 'ISOLATED') === want.world &&
  have.matchOriginAsFallback === want.matchOriginAsFallback &&
  have.persistAcrossSessions !== false;

/**
 * Keeps the trackers registered while site access is granted, and unregisters them when it
 * isn't. Never unregisters a correct registration (pages loading in that gap would go
 * unwatched): only missing scripts are added and outdated ones updated. Serialized (worker
 * start and `permissions.onAdded` can overlap). Already-open pages aren't injected: a tracker
 * that starts late could miss earlier typing, so those pages are probed without one.
 */
export function syncProbeScripts(): Promise<void> {
  const run = async () => {
    const granted = await hasProbeAccess();
    const existing = await browser.scripting.getRegisteredContentScripts({
      ids: SCRIPTS.map((s) => s.id),
    });
    if (!granted) {
      if (existing.length) {
        await browser.scripting.unregisterContentScripts({ ids: existing.map((s) => s.id) });
      }
      return;
    }
    const byId = new Map(existing.map((s) => [s.id, s]));
    const want = wanted();
    const missing = want.filter((s) => !byId.has(s.id));
    const outdated = want.filter((s) => {
      const have = byId.get(s.id);
      return have !== undefined && !sameScript(have, s);
    });
    if (missing.length) await browser.scripting.registerContentScripts(missing);
    if (outdated.length) await browser.scripting.updateContentScripts(outdated);
  };
  const next = syncChain.then(run);
  syncChain = next.catch(() => undefined);
  return next;
}

const TIMEOUT = Symbol('timeout');
const timeout = (ms: number) =>
  new Promise<typeof TIMEOUT>((resolve) => setTimeout(() => resolve(TIMEOUT), ms));

/**
 * Probes one tab. Never throws: anything that goes wrong is `unknown`.
 * Discarded / not-yet-loaded tabs (session restore) can't be checked, yet Chrome may bring
 * back a half-filled form when they load, so they're `unknown` (2×) until they load.
 */
export async function probeTab(tab: TabInfo): Promise<ProbeVerdict> {
  if (tab.id === undefined || !isWebUrl(tab.url || tab.pendingUrl)) return 'unknown';
  if (tab.discarded || tab.status === 'unloaded') return 'unknown';
  if (!(await hasProbeAccess())) return 'unknown';
  const target = { tabId: tab.id, allFrames: true };
  try {
    const both = Promise.all([
      browser.scripting.executeScript({ target, func: inspectForms }),
      browser.scripting.executeScript({ target, func: inspectUnload, world: 'MAIN' }),
    ]);
    const result = await Promise.race([both, timeout(PROBE_TIMEOUT_MS)]);
    if (result === TIMEOUT) return 'unknown';
    const [forms, unload] = result;
    return combineProbe(forms, unload);
  } catch {
    return 'unknown';
  }
}

/** Cached verdicts, shared with Decay's pages through `storage.session`. */
export const probeStore = {
  async load(): Promise<ProbeRecord> {
    try {
      const got = await browser.storage.session.get(PROBE_KEY);
      return parseProbeRecord(got[PROBE_KEY]);
    } catch {
      return {};
    }
  },
  async save(record: ProbeRecord): Promise<void> {
    await browser.storage.session.set({ [PROBE_KEY]: record });
  },
};
