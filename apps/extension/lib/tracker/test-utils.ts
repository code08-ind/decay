import type { IdleState } from './clock';
import type { GroupInfo, TabInfo, WindowInfo } from './tabs';
import type { BrowserApi } from './tracker';

export function makeTab(overrides: Partial<TabInfo> = {}): TabInfo {
  return {
    id: 1,
    windowId: 1,
    index: 0,
    url: 'https://example.com/',
    title: 'Example',
    pinned: false,
    audible: false,
    active: false,
    incognito: false,
    ...overrides,
  };
}

/** In-memory browser for tracker tests. */
export class FakeBrowser implements BrowserApi {
  tabs: TabInfo[] = [];
  windows: WindowInfo[] = [{ id: 1, focused: true, incognito: false, state: 'normal' }];
  groups: GroupInfo[] = [];
  idle: IdleState = 'active';
  lastTickAt: number | null = null;
  async queryTabs() {
    return structuredClone(this.tabs);
  }
  async getTab(id: number) {
    return structuredClone(this.tabs.find((t) => t.id === id));
  }
  async getActiveTab(windowId: number) {
    return structuredClone(this.tabs.find((t) => t.windowId === windowId && t.active));
  }
  async getWindows() {
    return this.windows;
  }
  async getGroups() {
    return this.groups;
  }
  async queryIdle() {
    return this.idle;
  }
  async getLastTickAt() {
    return this.lastTickAt;
  }
  async setLastTickAt(at: number) {
    this.lastTickAt = at;
  }
}
