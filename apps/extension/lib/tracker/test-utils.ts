import type { TabInfo } from './tabs';

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
