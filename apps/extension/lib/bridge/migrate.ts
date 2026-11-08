// Archive sync to the app (Software Design §4.7 "Mode switch", DB Design §7): the
// extension's archive moves into the app with `import.archive`, and stays in step after
// that. While the app isn't running the extension keeps working on its own (archiving,
// restores, rules), so on every connection — and every minute while connected — it sends
// what changed since the last batch the app confirmed:
// - items archived or restored since then (by `stateChangedAt`), in batches of
//   `SYNC_BATCH_ITEMS`, each acked, so even a huge archive fits the message limits;
// - rules and settings when they changed (the app keeps settings the user changed there).
// The app's import is idempotent and keeps the newest state per item, so a batch cut short
// is simply sent again. The local IndexedDB copy is kept as the standalone fallback.

import type { DecayDB } from '../db/db';
import type { Item } from '../db/types';
import { createSettingsStore } from '../settings/settings';
import type { Bridge } from './connection';
import { type Envelope, envelope, type ImportArchive, kinds } from './protocol';

/** Sync progress (`chrome.storage.local`). */
export const SYNC_KEY = 'decay.archiveSync';
export const SYNC_BATCH_ITEMS = 2_000;
/** The app can take a while for a big batch. */
export const IMPORT_TIMEOUT_MS = 120_000;

export interface SyncRecord {
  /** The newest `stateChangedAt` the app has confirmed. */
  through: number;
  /** Fingerprint of the rules and settings last confirmed. */
  config: string;
  /** When the first sync completed (the archive moved into the app). */
  firstAt: number;
  /** When the last sync completed. */
  at: number;
}

export interface KeyValueStore {
  get(key: string): Promise<unknown>;
  set(key: string, value: unknown): Promise<void>;
}

export type SyncResult =
  | { ok: true; record: SyncRecord; sent: number }
  | { ok: false; error: string };

/** An item as sent: large inline (`data:`) favicons are dropped. */
function forApp(item: Item): Item {
  if (!item.faviconUrl?.startsWith('data:')) return item;
  const { faviconUrl: _drop, ...rest } = item;
  return rest as Item;
}

/** Archived / restored items changed after `since`, oldest change first. */
export async function changedItems(database: DecayDB, since: number): Promise<Item[]> {
  const items = await database.items.where('state').anyOf(['archived', 'restored']).toArray();
  return items
    .filter((i) => i.stateChangedAt > since)
    .sort((a, b) => a.stateChangedAt - b.stateChangedAt);
}

/** The whole archive as one `import.archive` payload (tests, fixtures). */
export async function buildImportArchive(database: DecayDB): Promise<ImportArchive> {
  const [items, rules, settings] = await Promise.all([
    changedItems(database, Number.NEGATIVE_INFINITY),
    database.rules.toArray(),
    createSettingsStore(database).getAll(),
  ]);
  return {
    items: items.map(forApp),
    rules,
    settings: settings as unknown as Record<string, unknown>,
  };
}

/** A cheap fingerprint (FNV-1a) of the rules and settings. */
function fingerprint(value: unknown): string {
  const text = JSON.stringify(value);
  let h = 0x811c9dc5;
  for (let i = 0; i < text.length; i++) {
    h ^= text.charCodeAt(i);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return `${text.length}:${h.toString(16)}`;
}

/** Sends what changed since the app's last confirmation (see the module docs). */
export async function syncArchive(
  bridge: Bridge,
  database: DecayDB,
  storage: KeyValueStore,
  now: () => number,
  batchItems = SYNC_BATCH_ITEMS,
): Promise<SyncResult> {
  if (bridge.state !== 'connected') return { ok: false, error: 'not connected' };
  const saved = (await storage.get(SYNC_KEY)) as SyncRecord | undefined;
  const [items, rules, settings] = await Promise.all([
    changedItems(database, saved?.through ?? Number.NEGATIVE_INFINITY),
    database.rules.toArray(),
    createSettingsStore(database).getAll(),
  ]);
  const config = fingerprint({ rules, settings });
  if (saved && items.length === 0 && saved.config === config) {
    return { ok: true, record: saved, sent: 0 };
  }
  let record: SyncRecord = saved ?? {
    through: Number.NEGATIVE_INFINITY,
    config: '',
    firstAt: 0,
    at: 0,
  };
  const batches = Math.max(1, Math.ceil(items.length / batchItems));
  for (let b = 0; b < batches; b++) {
    const batch = items.slice(b * batchItems, (b + 1) * batchItems);
    const withConfig = b === 0 && record.config !== config;
    const payload: ImportArchive = {
      items: batch.map(forApp),
      rules: withConfig ? rules : [],
      settings: withConfig ? (settings as unknown as Record<string, unknown>) : {},
    };
    let reply: Envelope;
    try {
      reply = await bridge.request(envelope(kinds.importArchive, payload), IMPORT_TIMEOUT_MS);
    } catch (e) {
      return { ok: false, error: e instanceof Error ? e.message : String(e) };
    }
    const ack = reply.payload as { ok?: boolean; error?: { message?: string } } | null;
    if (reply.type !== kinds.ack || ack?.ok !== true) {
      return { ok: false, error: ack?.error?.message ?? 'the app refused the archive' };
    }
    const at = now();
    record = {
      through: batch.length
        ? Math.max(record.through, ...batch.map((i) => i.stateChangedAt))
        : record.through,
      config: withConfig ? config : record.config,
      firstAt: record.firstAt || at,
      at,
    };
    await storage.set(SYNC_KEY, record);
  }
  return { ok: true, record, sent: items.length };
}
