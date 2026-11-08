import 'fake-indexeddb/auto';
import { describe, expect, it } from 'vitest';
import { DecayDB } from '../db/db';
import type { Item } from '../db/types';
import type { Bridge } from './connection';
import {
  buildImportArchive,
  type KeyValueStore,
  SYNC_BATCH_ITEMS,
  SYNC_KEY,
  type SyncRecord,
  syncArchive,
} from './migrate';
import type { Envelope, ImportArchive } from './protocol';

let n = 0;
const item = (
  id: string,
  state: Item['state'],
  stateChangedAt = 1,
  over: Partial<Item> = {},
): Item => ({
  id,
  kind: 'tab',
  appId: 'com.google.Chrome',
  title: id,
  url: `https://${id}.com/`,
  pinned: false,
  state,
  activeAgeS: 1,
  firstSeenAt: 1,
  lastUsedAt: 1,
  stateChangedAt,
  restoreCount: 0,
  ...over,
});

function memory(): KeyValueStore & { data: Map<string, unknown> } {
  const data = new Map<string, unknown>();
  return { data, get: async (k) => data.get(k), set: async (k, v) => void data.set(k, v) };
}

async function seeded(items: Item[]) {
  const database = new DecayDB(`decay-sync-test-${n++}`);
  await database.items.bulkPut(items);
  return database;
}

/** A bridge whose app answers each `import.archive` with `answer`. */
function fakeBridge(
  answer: (payload: ImportArchive) => Envelope['payload'] | Error,
  state = 'connected',
) {
  const sent: ImportArchive[] = [];
  const bridge = {
    state,
    request: async (env: Envelope) => {
      const payload = env.payload as ImportArchive;
      sent.push(payload);
      const r = answer(payload);
      if (r instanceof Error) throw r;
      return { v: 1, id: 'r', type: 'ack', replyTo: env.id, payload: r };
    },
  } as unknown as Bridge;
  return { bridge, sent };
}

describe('archive sync', () => {
  it('sends archived and restored items once, then only what changed', async () => {
    const database = await seeded([
      item('a', 'archived', 10, { faviconUrl: 'data:image/png;base64,AAAA' }),
      item('r', 'restored', 20),
      item('live', 'active', 30),
    ]);
    const storage = memory();
    const { bridge, sent } = fakeBridge(() => ({ ok: true }));
    const first = await syncArchive(bridge, database, storage, () => 42);
    expect(first).toMatchObject({ ok: true, sent: 2 });
    expect(sent[0]?.items.map((i) => (i as Item).id)).toEqual(['a', 'r']);
    expect((sent[0]?.items[0] as Item | undefined)?.faviconUrl).toBeUndefined(); // data: icons dropped
    expect(sent[0]?.items[0]).toBeDefined();
    expect(sent[0]?.rules.length).toBeGreaterThan(0);
    expect(sent[0]?.settings).toMatchObject({ mode: 'shadow' });

    // Nothing changed: nothing sent.
    await syncArchive(bridge, database, storage, () => 43);
    expect(sent.length).toBe(1);

    // Archived later in standalone mode, and restored again: only those, without config.
    await database.items.bulkPut([item('b', 'archived', 50), item('a', 'restored', 60)]);
    const again = await syncArchive(bridge, database, storage, () => 44);
    expect(again).toMatchObject({ ok: true, sent: 2 });
    expect(sent[1]?.items.map((i) => (i as Item).id)).toEqual(['b', 'a']);
    expect(sent[1]?.rules).toEqual([]);
    const record = storage.data.get(SYNC_KEY) as SyncRecord;
    expect(record).toMatchObject({ through: 60, firstAt: 42, at: 44 });

    // A rule change goes out on its own.
    await database.rules.put({
      id: 'new',
      scope: 'domain',
      match: 'x.com',
      action: 'protect',
      enabled: true,
      source: 'user',
      createdAt: 1,
      hitCount: 0,
    });
    await syncArchive(bridge, database, storage, () => 45);
    expect(sent[2]?.items).toEqual([]);
    expect(sent[2]?.rules.some((r) => (r as { id: string }).id === 'new')).toBe(true);
  });

  it('sends big archives in acked batches and resumes after a failure', async () => {
    const batch = 3;
    const items = Array.from({ length: batch * 2 + 2 }, (_, i) => item(`i${i}`, 'archived', i + 1));
    const database = await seeded(items);
    const storage = memory();
    let calls = 0;
    const flaky = fakeBridge(() => (++calls === 2 ? new Error('timed out') : { ok: true }));
    const failed = await syncArchive(flaky.bridge, database, storage, () => 1, batch);
    expect(failed).toEqual({ ok: false, error: 'timed out' });
    expect((storage.data.get(SYNC_KEY) as SyncRecord).through).toBe(batch);

    const { bridge, sent } = fakeBridge(() => ({ ok: true }));
    const done = await syncArchive(bridge, database, storage, () => 2, batch);
    expect(done).toMatchObject({ ok: true, sent: batch + 2 });
    expect(sent.map((p) => p.items.length)).toEqual([batch, 2]);
    expect(SYNC_BATCH_ITEMS).toBeGreaterThan(batch);
  });

  it('records nothing when the app refuses or is gone', async () => {
    const database = await seeded([item('a', 'archived')]);
    const storage = memory();
    const refused = fakeBridge(() => ({ ok: false, error: { message: 'disk full' } }));
    expect(await syncArchive(refused.bridge, database, storage, () => 1)).toEqual({
      ok: false,
      error: 'disk full',
    });
    const offline = fakeBridge(() => ({ ok: true }), 'unavailable');
    expect(await syncArchive(offline.bridge, database, storage, () => 1)).toMatchObject({
      ok: false,
    });
    expect(offline.sent.length).toBe(0);
    expect(storage.data.has(SYNC_KEY)).toBe(false);
  });

  it('builds the whole archive for the fixtures', async () => {
    const database = await seeded([item('a', 'archived'), item('live', 'active')]);
    const archive = await buildImportArchive(database);
    expect(archive.items.map((i) => (i as Item).id)).toEqual(['a']);
  });
});
