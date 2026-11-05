import Dexie, { type EntityTable } from 'dexie';
import { uuidv7 } from 'uuidv7';
import type { Item, ItemEvent, MetricsDay, Rule, SettingRecord, Sweep } from './types';

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
    // `[appId+externalId]` finds the live record for a tab id.
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
  }
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
