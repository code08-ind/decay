import type { TabCommands } from '../sweep/executor';
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

/** Tab commands acting on the fake browser's tab list. */
export class FakeCommands implements TabCommands {
  nextTabId = 100;
  nextWindowId = 50;
  session = 's1';
  removed: number[] = [];
  createdWindows: string[] = [];
  groups: { id: number; tabIds: number[]; windowId: number; title?: string; color?: string }[] = [];
  onRemove?: (tabId: number) => Promise<void> | void;
  failCreate = new Set<string>();
  constructor(private readonly fake: FakeBrowser) {}
  async removeTab(tabId: number) {
    await this.onRemove?.(tabId);
    this.fake.tabs = this.fake.tabs.filter((t) => t.id !== tabId);
    this.removed.push(tabId);
  }
  async createTab(opts: { windowId: number; url: string; index?: number; active?: boolean }) {
    if (this.failCreate.has(opts.url)) throw new Error('create failed');
    const tab = makeTab({
      id: this.nextTabId++,
      windowId: opts.windowId,
      url: opts.url,
      index: opts.index ?? this.fake.tabs.length,
      active: opts.active ?? false,
      title: '',
      status: 'loading',
    });
    this.fake.tabs.push(tab);
    return structuredClone(tab);
  }
  async createWindow(url: string) {
    if (this.failCreate.has(url)) throw new Error('create failed');
    const windowId = this.nextWindowId++;
    this.fake.windows.push({ id: windowId, focused: false, incognito: false, state: 'normal' });
    this.createdWindows.push(url);
    const tab = await this.createTab({ windowId, url, index: 0 });
    return { windowId, tab };
  }
  async windowExists(windowId: number) {
    return this.fake.windows.some((w) => w.id === windowId);
  }
  async groupTabs(tabIds: number[], windowId: number) {
    const id = 900 + this.groups.length;
    this.groups.push({ id, tabIds, windowId });
    return id;
  }
  async updateGroup(groupId: number, props: { title?: string; color?: string }) {
    const g = this.groups.find((x) => x.id === groupId);
    if (g) Object.assign(g, props);
  }
  sessionStartedAt = 0;
  async browserSession() {
    return { id: this.session, startedAt: this.sessionStartedAt };
  }
}
