import { describe, expect, it } from 'vitest';
import { domainOf, inUseTabIds, markUsed, newItemFromTab } from './tabs';
import { makeTab } from './test-utils';

describe('domainOf', () => {
  it('returns the registrable domain, lowercase', () => {
    expect(domainOf('https://Docs.Google.com/document/d/1')).toBe('google.com');
    expect(domainOf('https://foo.bar.co.uk/x')).toBe('bar.co.uk');
    expect(domainOf('https://user.github.io/')).toBe('user.github.io'); // public suffix
  });

  it('falls back to the host for localhost and IPs', () => {
    expect(domainOf('http://localhost:3000/')).toBe('localhost');
    expect(domainOf('http://192.168.1.10/admin')).toBe('192.168.1.10');
  });

  it('ignores non-web URLs', () => {
    expect(domainOf('chrome://newtab/')).toBeUndefined();
    expect(domainOf('file:///Users/me/a.pdf')).toBeUndefined();
    expect(domainOf(undefined)).toBeUndefined();
  });
});

describe('inUseTabIds', () => {
  it('counts the active tab of visible windows and audible tabs', () => {
    const tabs = [
      makeTab({ id: 1, windowId: 1, active: true }),
      makeTab({ id: 2, windowId: 1 }),
      makeTab({ id: 3, windowId: 2, active: true }), // minimized window
      makeTab({ id: 4, windowId: 2, audible: true }),
      makeTab({ id: 5, windowId: 3, active: true, incognito: true }),
    ];
    const windows = [
      { id: 1, focused: true, incognito: false, state: 'normal' },
      { id: 2, focused: false, incognito: false, state: 'minimized' },
    ];
    expect([...inUseTabIds(tabs, windows)].sort()).toEqual(['1', '4']);
  });
});

describe('newItemFromTab / markUsed', () => {
  it('maps tab fields and group', () => {
    const groups = new Map([[7, { id: 7, title: 'Research', color: 'blue' }]]);
    const item = newItemFromTab(makeTab({ id: 9, groupId: 7, index: 3 }), 'app', groups, 100);
    expect(item).toMatchObject({
      kind: 'tab',
      appId: 'app',
      externalId: '9',
      windowExternalId: '1',
      domain: 'example.com',
      tabIndex: 3,
      groupTitle: 'Research',
      groupColor: 'blue',
      state: 'active',
      activeAgeS: 0,
      firstSeenAt: 100,
    });
  });

  it('resets age, revives stale items and samples used events', () => {
    const item = newItemFromTab(makeTab(), 'app', new Map(), 0);
    item.activeAgeS = 50_000;
    item.state = 'stale';
    expect(markUsed(item, 11 * 60_000)?.type).toBe('used');
    expect(item).toMatchObject({ activeAgeS: 0, state: 'active', lastUsedAt: 11 * 60_000 });
    expect(markUsed(item, 12 * 60_000)).toBeUndefined(); // within 10 min
  });
});
