import { uuidv7 } from 'uuidv7';
import { afterEach, describe, expect, it } from 'vitest';
import { DecayDB } from './db';
import type { Item } from './types';

let n = 0;
const open: DecayDB[] = [];
function freshDb() {
  const d = new DecayDB(`decay-test-${n++}`);
  open.push(d);
  return d;
}
afterEach(async () => {
  await Promise.all(open.splice(0).map((d) => d.delete()));
});

function tab(overrides: Partial<Item> = {}): Item {
  const now = Date.now();
  return {
    id: uuidv7(),
    kind: 'tab',
    appId: 'com.google.Chrome',
    title: 'Example',
    url: 'https://example.com/',
    domain: 'example.com',
    pinned: false,
    state: 'active',
    activeAgeS: 0,
    firstSeenAt: now,
    lastUsedAt: now,
    stateChangedAt: now,
    restoreCount: 0,
    ...overrides,
  };
}

describe('DecayDB', () => {
  it('seeds the default protection rules on first open', async () => {
    const d = freshDb();
    const rules = await d.rules.toArray();
    expect(rules.map((r) => r.match).sort()).toEqual([
      'accounts.google.com',
      'meet.google.com',
      'teams.microsoft.com',
      'zoom.us',
    ]);
    expect(rules.every((r) => r.action === 'protect' && r.source === 'default')).toBe(true);
  });

  it('rejects a duplicate (scope, match, action) rule', async () => {
    const d = freshDb();
    const [first] = await d.rules.toArray();
    if (!first) throw new Error('no seeded rules');
    await expect(d.rules.add({ ...first, id: uuidv7() })).rejects.toThrow();
  });

  it('queries sweep candidates by state and active age', async () => {
    const d = freshDb();
    await d.items.bulkAdd([
      tab({ activeAgeS: 10 * 3600 }),
      tab({ activeAgeS: 70 * 3600 }),
      tab({ state: 'archived', activeAgeS: 90 * 3600 }),
    ]);
    const candidates = await d.items
      .where('[state+activeAgeS]')
      .between(['active', 60 * 3600], ['active', Number.POSITIVE_INFINITY])
      .toArray();
    expect(candidates).toHaveLength(1);
    expect(candidates[0]?.activeAgeS).toBe(70 * 3600);
  });

  it('finds a live item by tab id', async () => {
    const d = freshDb();
    const item = tab({ externalId: '42' });
    await d.items.add(item);
    const found = await d.items.get({ appId: 'com.google.Chrome', externalId: '42' });
    expect(found?.id).toBe(item.id);
  });
});
