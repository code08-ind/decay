// The app link backed by the real extension APIs: adapter operations, the bridge's
// native port and timers, and the link status the popup shows.

import { browser } from 'wxt/browser';
import { probeTab } from '../probe/chrome';
import type { AdapterDeps, ChromeTab } from './adapter';
import type { BridgeState } from './connection';
import type { BrowserKind } from './protocol';

/** The link's status, in `storage.session` for the popup. */
export const APP_LINK_STATUS_KEY = 'decay.appLink';
/** Whether the user turned the link on (`storage.local`). */
export const APP_LINK_ENABLED_KEY = 'decay.appLinkEnabled';
/** This browser profile's id for the app (`storage.local`). */
export const PROFILE_ID_KEY = 'decay.profileId';

export interface AppLinkStatus {
  state: BridgeState;
  detail?: string;
  at: number;
  /** When the archive was first moved into the app. */
  migratedAt?: number;
  /** The extension version this status is about (an `outdated` link waits for a new one). */
  version?: string;
}

let profile: Promise<string> | null = null;

/** This profile's id for the app; created once (concurrent first calls share it). */
export function profileId(): Promise<string> {
  profile ??= (async () => {
    const got = await browser.storage.local.get(PROFILE_ID_KEY);
    const existing = got[PROFILE_ID_KEY];
    if (typeof existing === 'string' && existing) return existing;
    const id = crypto.randomUUID();
    await browser.storage.local.set({ [PROFILE_ID_KEY]: id });
    return id;
  })().catch((e) => {
    profile = null;
    throw e;
  });
  return profile;
}

const SESSION_KEY = 'decay.appLinkSession';
let session: string | null = null;

/**
 * This browser session's id (`storage.session` lives as long as the browser): window ids
 * are reused after a restart, so `tabs.restore` window keys carry it.
 */
export async function browserSession(): Promise<string> {
  if (session) return session;
  const got = await browser.storage.session.get(SESSION_KEY);
  const existing = got[SESSION_KEY];
  session = typeof existing === 'string' && existing ? existing : crypto.randomUUID();
  if (session !== existing) await browser.storage.session.set({ [SESSION_KEY]: session });
  return session;
}

/** The session id once known (after `browserSession()` resolved), else ''. */
export const currentSession = () => session ?? '';

export function browserKind(appId: string): BrowserKind {
  if (appId === 'com.microsoft.Edge') return 'edge';
  if (appId === 'com.brave.Browser') return 'brave';
  if (appId === 'org.mozilla.firefox') return 'firefox';
  return 'chrome'; // Arc and Chromium report themselves as Chrome
}

export async function linkEnabled(): Promise<boolean> {
  const got = await browser.storage.local.get(APP_LINK_ENABLED_KEY);
  return got[APP_LINK_ENABLED_KEY] === true;
}

export async function setLinkEnabled(on: boolean): Promise<void> {
  await browser.storage.local.set({ [APP_LINK_ENABLED_KEY]: on });
}

export async function readLinkStatus(): Promise<AppLinkStatus | null> {
  const got = await browser.storage.session.get(APP_LINK_STATUS_KEY);
  return (got[APP_LINK_STATUS_KEY] as AppLinkStatus | undefined) ?? null;
}

export async function writeLinkStatus(status: AppLinkStatus): Promise<void> {
  await browser.storage.session.set({ [APP_LINK_STATUS_KEY]: status });
}

export function adapterDeps(
  send: AdapterDeps['send'],
  isConnected: AdapterDeps['isConnected'],
): AdapterDeps {
  return {
    isConnected,
    session: currentSession,
    ownOrigin: browser.runtime.getURL('/'),
    now: Date.now,
    queryTabs: async () => (await browser.tabs.query({})) as ChromeTab[],
    getWindows: async () => browser.windows.getAll(),
    // Throws "No current window" while the browser has no window open: no groups then.
    getGroups: async () => (browser.tabGroups ? browser.tabGroups.query({}).catch(() => []) : []),
    getTab: async (tabId) => {
      try {
        return (await browser.tabs.get(tabId)) as ChromeTab;
      } catch {
        return undefined;
      }
    },
    removeTab: async (tabId) => {
      await browser.tabs.remove(tabId);
    },
    createTab: async ({ windowId, url, index, active }) =>
      (await browser.tabs.create({ windowId, url, index, active })) as ChromeTab,
    createWindow: async (url, focused) => {
      const win = await browser.windows.create({ url, focused });
      const tab = win?.tabs?.[0];
      if (!tab) throw new Error('Window not created');
      return tab as ChromeTab;
    },
    groupTabs: async (tabIds, windowId, group) => {
      if (!browser.tabGroups || tabIds.length === 0) return;
      const groupId = await browser.tabs.group({
        tabIds: tabIds as [number, ...number[]],
        createProperties: { windowId },
      });
      await browser.tabGroups.update(groupId, {
        title: group.title,
        color: group.color as Parameters<typeof browser.tabGroups.update>[1]['color'],
      });
    },
    focusTab: async (tab) => {
      if (tab.id === undefined) return;
      await browser.tabs.update(tab.id, { active: true });
      await browser.windows.update(tab.windowId, { focused: true });
    },
    probe: (tab) => probeTab(tab),
    send,
  };
}
