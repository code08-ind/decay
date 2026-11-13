// Generates the cross-language fixtures for the extension ⇄ app link from the extension's
// real code, so the Rust tests read exactly what the extension sends:
//   crates/decay-bridge/tests/fixtures/ext_chunks.json  — a message chunked by
//     `splitEnvelope` (emoji, quotes, control characters), plus the original
//   crates/decay-store/tests/fixtures/import_archive.json — `buildImportArchive` over a
//     seeded IndexedDB archive (fake-indexeddb)
//   crates/decay-engine/tests/fixtures/adapter_acks.json — the acks the real `Adapter`
//     sends for `probe`, `tabs.close` and `tabs.restore` (the engine settles sweeps and
//     restores from them)
// CI regenerates them and fails on any diff. Re-run after changing lib/bridge or the
// archive types:
//   pnpm --filter @decay/extension bridge-fixtures
import 'fake-indexeddb/auto';
import { writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { Adapter, type AdapterDeps, type ChromeTab } from '../lib/bridge/adapter';
import { splitEnvelope } from '../lib/bridge/chunk';
import { buildImportArchive } from '../lib/bridge/migrate';
import { type Envelope, envelope, kinds } from '../lib/bridge/protocol';
import { DecayDB } from '../lib/db/db';
import type { Item, Rule } from '../lib/db/types';
import { createSettingsStore } from '../lib/settings/settings';

const out = (path: string, value: unknown) =>
  writeFileSync(
    fileURLToPath(new URL(`../../../${path}`, import.meta.url)),
    `${JSON.stringify(value)}\n`,
  );

// Deterministic ids and times (the fixtures must not change between runs).
let counter = 0;
const id = (prefix: string) => `${prefix}-${String(counter++).padStart(4, '0')}`;

// 1. Chunks: a large snapshot-like message whose titles mix emoji (surrogate pairs), CJK,
// quotes, backslashes and control characters — everything that grows when escaped again.
const titles = Array.from(
  { length: 400 },
  (_, i) => `${i} 😀 éß 漢字 "quoted" \\path\\ \u0001\t tab ${'x'.repeat(i % 50)}`,
);
const original: Envelope = {
  v: 1,
  id: 'fixture-original',
  type: 'snapshot',
  payload: {
    windows: [
      {
        id: 1,
        focused: true,
        state: 'normal',
        incognito: false,
        tabs: titles.map((title, i) => ({
          tabId: i,
          windowId: 1,
          index: i,
          url: `https://e.com/${i}`,
          title,
          pinned: false,
          audible: false,
          discarded: false,
          groupId: -1,
          active: false,
          status: 'complete',
        })),
      },
    ],
    groups: [],
  },
};
// Small pieces (the Rust reader doesn't care how big they are) keep the fixture small.
const chunks = splitEnvelope(original, 16 * 1024, 16 * 1024).map((c, i) => ({
  ...c,
  id: `chunk-${i}`,
  chunk: c.chunk ? { ...c.chunk, groupId: 'fixture-group' } : undefined,
}));
out('crates/decay-bridge/tests/fixtures/ext_chunks.json', { original, chunks });

// 2. Import archive from a seeded IndexedDB.
const database = new DecayDB('decay-fixture');
const t0 = 1_793_750_400_000;
const item = (over: Partial<Item>): Item => ({
  id: id('item'),
  kind: 'tab',
  appId: 'com.google.Chrome',
  title: 'Untitled',
  pinned: false,
  state: 'archived',
  activeAgeS: 220_000,
  firstSeenAt: t0 - 9e6,
  lastUsedAt: t0 - 8e6,
  stateChangedAt: t0 - 1000,
  archivedAt: t0 - 1000,
  archiveReason: { type: 'unused', activeHours: 61, threshold: 60 },
  restoreCount: 0,
  ...over,
});
const archived = item({
  title: 'Acme pricing 😀',
  url: 'https://acme.com/pricing?plan=pro',
  domain: 'acme.com',
  faviconUrl: 'https://acme.com/favicon.ico',
  groupTitle: 'Research',
  groupColor: 'blue',
  tabIndex: 3,
  lastSweepId: 'sweep-1',
  windowExternalId: '12',
});
const items: Item[] = [
  archived,
  item({
    appId: 'com.microsoft.Edge',
    title: 'Edge docs',
    url: 'https://learn.microsoft.com/x',
    domain: 'microsoft.com',
    archiveReason: { type: 'customRule', ruleId: 'rule-x', activeHours: 9 },
  }),
  item({
    appId: 'com.brave.Browser',
    title: 'Brave thing',
    url: 'https://brave.com/',
    domain: 'brave.com',
    state: 'restored',
    restoredFromId: archived.id,
    restoreCount: 1,
    graceUntil: t0 + 1000,
  }),
  item({
    title: 'Still open',
    url: 'https://open.com/',
    state: 'active',
    externalId: '55',
    archivedAt: undefined,
    archiveReason: undefined,
  }),
  item({
    title: 'Stale tab',
    url: 'https://stale.com/',
    state: 'stale',
    externalId: '56',
    archivedAt: undefined,
  }),
];
await database.items.bulkPut(items);
const rule = (over: Partial<Rule>): Rule => ({
  id: id('rule'),
  scope: 'domain',
  match: 'figma.com',
  action: 'protect',
  enabled: true,
  source: 'user',
  createdAt: t0,
  hitCount: 4,
  ...over,
});
// The database seeds its default rules on open; replace them with deterministic ones.
await database.rules.clear();
await database.rules.bulkPut([
  rule({}),
  rule({
    scope: 'urlGlob',
    match: 'https://*.notion.so/*',
    action: 'customThreshold',
    thresholdHours: { stale: 2, hidden: 4, archive: 8 },
    source: 'quick_action',
  }),
  rule({ scope: 'titleRegex', match: '^Inbox', action: 'exclude', enabled: false }),
  rule({ match: 'zoom.us', source: 'default', hitCount: 0 }),
]);
await createSettingsStore(database).update({
  mode: 'automatic',
  retentionDays: 30,
  idleCutoffS: 600,
  theme: 'dark',
});
await database.settings.put({ key: 'shadowStartedAt', value: t0 - 30 * 86_400_000, updatedAt: t0 });
const archive = await buildImportArchive(database);
// Sorted for a stable fixture.
archive.items.sort((a, b) => String((a as Item).id).localeCompare(String((b as Item).id)));
archive.rules.sort((a, b) => String((a as Rule).id).localeCompare(String((b as Rule).id)));
out('crates/decay-store/tests/fixtures/import_archive.json', archive);
console.log(
  `wrote ${chunks.length} chunks and an archive of ${archive.items.length} items, ${archive.rules.length} rules`,
);

// 3. Adapter acks: one of each outcome the engine has to read.
const ackNow = 1_793_750_400_000;
const chromeTab = (id: number, over: Partial<ChromeTab> = {}): ChromeTab => ({
  id,
  windowId: 1,
  index: id,
  url: `https://site${id}.example/`,
  title: `Tab ${id}`,
  pinned: false,
  active: false,
  incognito: false,
  lastAccessed: ackNow - 3_600_000,
  ...over,
});
const openTabs = [
  chromeTab(1),
  chromeTab(2, { active: true }),
  chromeTab(3),
  chromeTab(4),
  chromeTab(6, { pinned: true }),
  chromeTab(7, { lastAccessed: ackNow - 1000 }),
];
const verdicts: Record<number, 'clean' | 'dirty' | 'unknown'> = { 3: 'dirty', 4: 'unknown' };
const acks: Envelope[] = [];
let nextTabId = 100;
const deps: AdapterDeps = {
  queryTabs: async () => openTabs,
  getWindows: async () => [
    { id: 1, focused: true, state: 'normal', type: 'normal', incognito: false },
  ],
  getGroups: async () => [],
  getTab: async (tabId) => openTabs.find((t) => t.id === tabId),
  removeTab: async () => undefined,
  createTab: async ({ windowId, url }) => chromeTab(nextTabId++, { windowId, url }),
  createWindow: async (url) => chromeTab(nextTabId++, { windowId: 9, url }),
  groupTabs: async () => undefined,
  focusTab: async () => undefined,
  probe: async (t) => verdicts[t.id ?? -1] ?? 'clean',
  send: (e) => {
    acks.push(e);
    return true;
  },
  isConnected: () => true,
  session: () => 'S1',
  ownOrigin: 'chrome-extension://decay/',
  now: () => ackNow,
};
const adapter = new Adapter(deps);
const ackFor = async (type: string, payload: unknown) => {
  acks.length = 0;
  await adapter.handle(envelope(type, payload));
  return acks[0]?.payload;
};
const adapterAcks = {
  probe: await ackFor(kinds.probe, { tabIds: [1, 3, 4, 99] }),
  close: await ackFor(kinds.tabsClose, {
    sweepId: 'sweep-1',
    tabIds: [1, 2, 3, 4, 5, 6, 7],
    urls: { '1': 'https://site1.example/', '3': 'https://site3.example/' },
  }),
  closeChanged: await ackFor(kinds.tabsClose, {
    sweepId: 'sweep-2',
    tabIds: [1],
    urls: { '1': 'https://elsewhere.example/' },
  }),
  restore: await ackFor(kinds.tabsRestore, {
    items: [
      { itemId: 'item-a', url: 'https://a.example/', windowKey: 'S1:1', index: 2, active: true },
      { itemId: 'item-b', url: 'javascript:alert(1)' },
    ],
  }),
  unknownCommand: await ackFor('tabs.teleport', {}),
};
out('crates/decay-engine/tests/fixtures/adapter_acks.json', adapterAcks);
