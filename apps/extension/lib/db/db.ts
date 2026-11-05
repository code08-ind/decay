import Dexie, { type EntityTable } from 'dexie';
import { uuidv7 } from 'uuidv7';
import type { Item, ItemEvent, ItemState, MetricsDay, Rule, SettingRecord, Sweep } from './types';

/** States in which an item is a currently open tab/window (mirrors SQL idx_items_live_ext). */
export const LIVE_STATES: readonly ItemState[] = ['active', 'stale', 'hidden', 'archiving'];
const isLive = (state: ItemState | undefined) => state !== undefined && LIVE_STATES.includes(state);

export class DecayDB extends Dexie {
  items!: EntityTable<Item, 'id'>;
  events!: EntityTable<ItemEvent, 'id'>;
  sweeps!: EntityTable<Sweep, 'id'>;
  rules!: EntityTable<Rule, 'id'>;
  settings!: EntityTable<SettingRecord, 'key'>;
  metrics!: EntityTable<MetricsDay, 'day'>;

  constructor(name = 'decay') {
    super(name);
    // Docs/DB Design.md §7. `lastSweepId` mirrors items.last_sweep_id.
    // `[appId+externalId]` finds the live record for a tab id. Browsers reuse tab ids
    // after a restart, so `externalId` is cleared when an item leaves a live state
    // (hooks below) and lookups go through `findLiveByExternalId`.
    this.version(1).stores({
      items: 'id, state, domain, archivedAt, lastSweepId, [state+activeAgeS], [appId+externalId]',
      events: '++id, itemId, type, at',
      sweeps: 'id, startedAt, isShadow',
      rules: 'id, scope, match, &[scope+match+action]',
      settings: 'key',
      metrics: 'day',
    });
    this.on('populate', (tx) => {
      tx.table('rules').bulkAdd(defaultRules(Date.now()));
    });

    this.items.hook('creating', (_key, item) => {
      if (!isLive(item.state)) delete item.externalId;
    });
    this.items.hook('updating', (mods: Partial<Item>) => {
      if ('state' in mods && !isLive(mods.state)) return { externalId: undefined };
      return undefined;
    });

    // Domains are case-insensitive: store them lowercase so the unique index catches duplicates.
    this.rules.hook('creating', (_key, rule) => {
      rule.match = normalizeRuleMatch(rule.scope, rule.match);
    });
    this.rules.hook('updating', (mods: Partial<Rule>, _key, rule) => {
      if (!('match' in mods) && !('scope' in mods)) return undefined;
      const scope = mods.scope ?? rule.scope;
      const match = mods.match ?? rule.match;
      return { match: normalizeRuleMatch(scope, match) };
    });
  }

  /** The open item for a browser tab id, ignoring archived records that reused the id. */
  async findLiveByExternalId(appId: string, externalId: string): Promise<Item | undefined> {
    return this.items
      .where('[appId+externalId]')
      .equals([appId, externalId])
      .filter((item) => isLive(item.state))
      .first();
  }
}

export function normalizeRuleMatch(scope: Rule['scope'], match: string): string {
  return scope === 'domain' ? match.trim().toLowerCase() : match.trim();
}

/** Default protections seeded on first run (Docs/DB Design.md §6). */
export function defaultRules(now: number): Rule[] {
  const protect = (scope: Rule['scope'], match: string): Rule => ({
    id: uuidv7(),
    scope,
    match,
    action: 'protect',
    enabled: true,
    source: 'default',
    createdAt: now,
    hitCount: 0,
  });
  return [
    protect('domain', 'accounts.google.com'),
    protect('domain', 'meet.google.com'),
    protect('domain', 'teams.microsoft.com'),
    protect('domain', 'zoom.us'), // domain rules match the host and its subdomains
  ];
}

export const db = new DecayDB();
