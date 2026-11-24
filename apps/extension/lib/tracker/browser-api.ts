import { browser } from 'wxt/browser';
import type { TabCommands } from '../sweep/executor';
import type { IdleState } from './clock';
import type { TabInfo } from './tabs';
import type { BrowserApi } from './tracker';

const LAST_TICK_KEY = 'decay.lastTickAt';
const SESSION_KEY = 'decay.browserSession';

/** `BrowserApi` backed by the real extension APIs. */
export const chromeApi: BrowserApi = {
  async queryTabs() {
    return (await browser.tabs.query({})) as TabInfo[];
  },
  async getTab(tabId) {
    try {
      return (await browser.tabs.get(tabId)) as TabInfo;
    } catch (e) {
      // Only "No tab with id" means the tab is gone; anything else is unknown state,
      // which the sweep executor must not mistake for a closed tab.
      if (/no tab with id/i.test(e instanceof Error ? e.message : String(e))) return undefined;
      throw e;
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
    // Throws "No current window" while the browser has no window open: no groups then.
    return browser.tabGroups ? browser.tabGroups.query({}).catch(() => []) : [];
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

/** `TabCommands` backed by the real extension APIs (sweeps and undo, P0.5). */
export const chromeCommands: TabCommands = {
  async removeTab(tabId) {
    await browser.tabs.remove(tabId);
  },
  async createTab({ windowId, url, index, active }) {
    return (await browser.tabs.create({ windowId, url, index, active })) as TabInfo;
  },
  async createWindow(url) {
    const win = await browser.windows.create({ url, focused: false });
    const tab = win?.tabs?.[0];
    if (win?.id === undefined || !tab) throw new Error('Window not created');
    return { windowId: win.id, tab: tab as TabInfo };
  },
  async windowExists(windowId) {
    try {
      const win = await browser.windows.get(windowId);
      return win.type === 'normal' && !win.incognito;
    } catch {
      return false;
    }
  },
  async groupTabs(tabIds, windowId) {
    return browser.tabs.group({
      tabIds: tabIds as [number, ...number[]],
      createProperties: { windowId },
    });
  },
  async updateGroup(groupId, { title, color }) {
    if (!browser.tabGroups) return;
    await browser.tabGroups.update(groupId, {
      title,
      color: color as Browser.tabGroups.Color | undefined,
    });
  },
  async browserSession() {
    // `storage.session` is cleared when the browser quits, so this changes per session.
    const got = await browser.storage.session.get(SESSION_KEY);
    const existing = got[SESSION_KEY] as { id?: unknown; startedAt?: unknown } | undefined;
    if (typeof existing?.id === 'string' && typeof existing.startedAt === 'number') {
      return { id: existing.id, startedAt: existing.startedAt };
    }
    const session = { id: crypto.randomUUID(), startedAt: Date.now() };
    await browser.storage.session.set({ [SESSION_KEY]: session });
    return session;
  },
};

/** Which browser we're running in, as an app id (Docs/DB Design.md §3.1). */
export function detectAppId(): string {
  if (import.meta.env.FIREFOX) return 'org.mozilla.firefox';
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
