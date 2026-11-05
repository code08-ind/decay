import { uuidv7 } from 'uuidv7';
import { afterEach, describe, expect, it } from 'vitest';
import { DecayDB } from '../db/db';
import type { Item } from '../db/types';
import { createRulesStore, normalizeDomain, RuleError, validateRule } from './rules';

let n = 0;
const open: DecayDB[] = [];
function fresh() {
  const d = new DecayDB(`decay-rules-test-${n++}`);
  open.push(d);
  return { d, store: createRulesStore(d) };
}
afterEach(async () => {
  await Promise.all(open.splice(0).map((d) => d.delete()));
});

const item = (url: string, state: Item['state'] = 'active'): Item => ({
  id: uuidv7(),
  kind: 'tab',
  appId: 'com.google.Chrome',
  title: 't',
  url,
  pinned: false,
  state,
  activeAgeS: 0,
  firstSeenAt: 0,
  lastUsedAt: 0,
  stateChangedAt: 0,
  restoreCount: 0,
});

describe('validateRule', () => {
  it('normalizes domains, including pasted URLs', () => {
    expect(normalizeDomain(' *.GitHub.com ')).toBe('github.com');
    expect(normalizeDomain('https://docs.github.com/en/x')).toBe('docs.github.com');
    expect(normalizeDomain('bücher.de')).toBe('xn--bcher-kva.de');
    expect(normalizeDomain('localhost')).toBe('localhost');
    expect(normalizeDomain('192.168.1.10')).toBe('192.168.1.10');
    expect(normalizeDomain('user.github.io')).toBe('user.github.io');
    for (const bad of ['not a domain!', 'com', 'co.uk', 'github.io', 'https://a@b.com/', '']) {
      expect(() => normalizeDomain(bad), bad).toThrow(RuleError);
    }
  });

  it('rejects invalid combinations', () => {
    const bad = [
      { scope: 'domain', match: '', action: 'protect' },
      { scope: 'nope', match: 'a.com', action: 'protect' },
      { scope: 'titleRegex', match: '(unclosed', action: 'protect' },
      { scope: 'urlGlob', match: '**', action: 'protect' },
      { scope: 'titleRegex', match: '(a+)+$', action: 'protect' },
      { scope: 'urlGlob', match: '*://*', action: 'exclude' },
      { scope: 'urlGlob', match: 'http*', action: 'exclude' },
      { scope: 'titleRegex', match: '.*', action: 'exclude' },
      { scope: 'titleRegex', match: 'a', action: 'exclude' },
      { scope: 'domain', match: 'com', action: 'exclude' },
      { scope: 'global', match: '*', action: 'exclude' },
      { scope: 'domain', match: 'a.com', action: 'closeAllowed' },
      { scope: 'domain', match: 'a.com', action: 'customThreshold' },
      {
        scope: 'domain',
        match: 'a.com',
        action: 'customThreshold',
        thresholdHours: { stale: 5, hidden: 5, archive: 9 },
      },
    ];
    for (const input of bad) {
      // biome-ignore lint/suspicious/noExplicitAny: testing untyped input
      expect(() => validateRule(input as any), JSON.stringify(input)).toThrow(RuleError);
    }
  });

  it('drops thresholds from non-custom rules', () => {
    const r = validateRule({
      scope: 'domain',
      match: 'a.com',
      action: 'protect',
      thresholdHours: { stale: 1, hidden: 2, archive: 3 },
    });
    expect(r.thresholdHours).toBeUndefined();
    expect(validateRule({ scope: 'global', match: 'anything', action: 'protect' }).match).toBe('*');
  });
});

describe('rules store', () => {
  it('creates, updates and removes rules', async () => {
    const { store } = fresh();
    const { rule: r } = await store.create({
      scope: 'domain',
      match: 'GitHub.com',
      action: 'protect',
    });
    expect(r).toMatchObject({ match: 'github.com', enabled: true, source: 'user', hitCount: 0 });
    const { rule: updated } = await store.update(r.id, { enabled: false });
    expect(updated.enabled).toBe(false);
    await store.remove(r.id);
    expect((await store.list()).some((x) => x.id === r.id)).toBe(false);
  });

  it('reports duplicates clearly', async () => {
    const { store } = fresh();
    await store.create({ scope: 'domain', match: 'a.com', action: 'protect' });
    await expect(
      store.create({ scope: 'domain', match: 'https://A.com/', action: 'protect' }),
    ).rejects.toMatchObject({ code: 'DUPLICATE_RULE' });
  });

  it('purges stored items and events when a site is excluded (F9)', async () => {
    const { d, store } = fresh();
    const bank = item('https://my.bank.com/acct', 'archived');
    const other = item('https://news.com/');
    await d.items.bulkAdd([bank, other]);
    await d.events.bulkAdd([
      { itemId: bank.id, type: 'seen', at: 1 },
      { itemId: other.id, type: 'seen', at: 1 },
    ]);
    const input = { scope: 'domain', match: 'bank.com', action: 'exclude' } as const;
    expect(await store.previewExclude(input)).toBe(1);
    expect((await store.create(input)).purged).toBe(1);
    expect((await d.items.toArray()).map((i) => i.id)).toEqual([other.id]);
    expect(await d.events.where('itemId').equals(bank.id).count()).toBe(0);
  });

  it('rolls back a failed update', async () => {
    const { store } = fresh();
    const { rule: r } = await store.create({ scope: 'domain', match: 'a.com', action: 'protect' });
    await expect(store.update(r.id, { scope: 'titleRegex', match: '(' })).rejects.toThrow();
    expect((await store.list()).find((x) => x.id === r.id)?.scope).toBe('domain');
    await expect(store.update('missing', {})).rejects.toMatchObject({ code: 'NOT_FOUND' });
  });

  it('purges when an exclude rule is enabled or a rule becomes exclude', async () => {
    const { d, store } = fresh();
    await d.items.bulkAdd([item('https://a.example.org/'), item('https://b.example.net/')]);
    const { rule: a } = await store.create({
      scope: 'domain',
      match: 'example.org',
      action: 'exclude',
      enabled: false,
    });
    expect(await d.items.count()).toBe(2);
    expect((await store.update(a.id, { enabled: true })).purged).toBe(1);
    const { rule: b } = await store.create({
      scope: 'domain',
      match: 'example.net',
      action: 'protect',
    });
    expect((await store.update(b.id, { action: 'exclude' })).purged).toBe(1);
    expect(await d.items.count()).toBe(0);
  });

  it('can always disable a legacy rule that no longer validates', async () => {
    const { d, store } = fresh();
    await d.rules.add({
      id: 'legacy',
      scope: 'domain',
      match: 'a.com',
      action: 'customThreshold',
      thresholdHours: { stale: 12, hidden: 12, archive: 60 },
      enabled: true,
      source: 'user',
      createdAt: 0,
      hitCount: 0,
    });
    expect((await store.update('legacy', { enabled: false })).rule.enabled).toBe(false);
    await expect(store.update('legacy', { enabled: true })).rejects.toMatchObject({
      code: 'INVALID_RULE',
    });
  });
});
