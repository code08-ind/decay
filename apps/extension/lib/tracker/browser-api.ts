import { browser } from 'wxt/browser';
import type { IdleState } from './clock';
import type { TabInfo } from './tabs';
import type { BrowserApi } from './tracker';

const LAST_TICK_KEY = 'decay.lastTickAt';

/** `BrowserApi` backed by the real extension APIs. */
export const chromeApi: BrowserApi = {
  async queryTabs() {
    return (await browser.tabs.query({})) as TabInfo[];
  },
  async getTab(tabId) {
    try {
      return (await browser.tabs.get(tabId)) as TabInfo;
    } catch {
      return undefined; // closed in the meantime
    }
  },
  async getActiveTab(windowId) {
    const [tab] = await browser.tabs.query({ windowId, active: true });
    return tab as TabInfo | undefined;
  },
  async getWindows() {
    return browser.windows.getAll();
  },
  async getGroups() {
    return browser.tabGroups ? browser.tabGroups.query({}) : [];
  },
  async queryIdle(cutoffS) {
    // chrome.idle requires at least 15 s.
    return (await browser.idle.queryState(Math.max(15, cutoffS))) as IdleState;
  },
  async getLastTickAt() {
    const got = await browser.storage.session.get(LAST_TICK_KEY);
    const value = got[LAST_TICK_KEY];
    return typeof value === 'number' ? value : null;
  },
  async setLastTickAt(at) {
    await browser.storage.session.set({ [LAST_TICK_KEY]: at });
  },
};

/** Which Chromium browser we're running in, as an app id (Docs/DB Design.md §3.1). */
export function detectAppId(): string {
  const nav = navigator as Navigator & {
    brave?: unknown;
    userAgentData?: { brands?: { brand: string }[] };
  };
  if (nav.brave) return 'com.brave.Browser';
  const brands = nav.userAgentData?.brands?.map((b) => b.brand) ?? [];
  if (brands.includes('Microsoft Edge') || /Edg\//.test(navigator.userAgent)) {
    return 'com.microsoft.Edge';
  }
  return 'com.google.Chrome';
}
