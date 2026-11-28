// Recording-only harness for the launch video (Docs/Launch/demo/record-launch.mjs), never
// shipped: the app's build only uses ../index.html. It answers the real UI's commands
// with sample data (Tauri's mockIPC), then loads the real UI (../src/main.tsx).
//
//   demo/index.html?view=library#/          the Library (any route after #)
//   demo/index.html?view=panel              the search panel
//   demo/index.html?view=settings#/privacy  Settings
//
// Restores and sweeps are reported to the parent page (the stage) with postMessage.
import { emit } from '@tauri-apps/api/event';
import { mockIPC, mockWindows } from '@tauri-apps/api/mocks';

const view = new URLSearchParams(location.search).get('view') ?? 'library';
mockWindows(view);
// In the stage the panel's frame is exactly the window: its shadow would be cut at the
// frame's edge, so the stage draws it outside instead.
if (view === 'panel') {
  document.head.insertAdjacentHTML(
    'beforeend',
    '<style>.panel-card{box-shadow:none!important}</style>',
  );
}

const MIN = 60_000;
const HOUR = 60 * MIN;
const NOW = Date.now();
const todayAt = (h: number, m: number) => {
  const d = new Date(NOW);
  d.setHours(h, m, 0, 0);
  return d.getTime();
};
const SWEEP_AT = todayAt(9, 41);

type Kind = 'tab' | 'window';
interface Sample {
  id: string;
  kind: Kind;
  title: string;
  domain: string | null;
  appName: string;
  archivedAt: number;
  reason: string;
  batch?: string;
}

// Archived today by sweep #12 (a batch), plus earlier ones.
const ITEMS: Sample[] = [
  ['tram28', 'Tram 28: Lisbon’s most famous ride – Visit Lisboa', 'visitlisboa.com', 'lisbon'],
  ['alfama', 'Lisbon restaurants in Alfama – Tripadvisor', 'tripadvisor.com', 'lisbon'],
  ['apartments', 'Lisbon apartments, 12–16 Dec – Booking.com', 'booking.com', 'lisbon'],
  ['things', 'The 30 best things to do in Lisbon – Time Out', 'timeout.com', 'lisbon'],
  ['azulejo', 'Azulejo – Wikipedia', 'en.wikipedia.org', 'lisbon'],
  ['q3', 'Q3 planning notes – Google Docs', 'docs.google.com', undefined],
  ['fetch', 'How do I cancel a fetch request? – Stack Overflow', 'stackoverflow.com', undefined],
  ['mdn', 'Using the Fetch API – MDN', 'developer.mozilla.org', undefined],
  ['hn', 'Show HN: A local-first tab archive | Hacker News', 'news.ycombinator.com', undefined],
].map(([id, title, domain, batch], i) => ({
  id: `00000000-0000-4000-8000-00000000000${i}`.slice(0, 36),
  kind: 'tab' as Kind,
  title: title as string,
  domain: domain as string,
  appName: 'Google Chrome',
  archivedAt: SWEEP_AT,
  reason: `Unused ${[61, 74, 66, 90, 58, 130, 72, 64, 96][i]} active h`,
  batch: (batch as string | undefined) ?? (id as string),
}));
ITEMS.push(
  {
    id: '00000000-0000-4000-8000-0000000000a1',
    kind: 'window',
    title: 'Invoices 2026 — Finder',
    domain: null,
    appName: 'Finder',
    archivedAt: SWEEP_AT - 26 * HOUR,
    reason: 'Unused 48 active h',
  },
  {
    id: '00000000-0000-4000-8000-0000000000a2',
    kind: 'tab',
    title: 'Sourdough starter, day by day – King Arthur Baking',
    domain: 'kingarthurbaking.com',
    appName: 'Google Chrome',
    archivedAt: SWEEP_AT - 27 * HOUR,
    reason: 'Unused 80 active h',
  },
  {
    id: '00000000-0000-4000-8000-0000000000a3',
    kind: 'tab',
    title: 'Pull request #418: cache logic – GitHub',
    domain: 'github.com',
    appName: 'Google Chrome',
    archivedAt: SWEEP_AT - 50 * HOUR,
    reason: 'Unused 62 active h',
  },
);

const panelItem = (s: Sample) => ({
  id: s.id,
  kind: s.kind,
  title: s.title,
  url: s.domain ? `https://${s.domain}/` : null,
  domain: s.domain,
  appName: s.appName,
  sourceId: s.kind === 'tab' ? 'w1' : null,
  sourceTitle: null,
  profile: null,
  archivedAt: s.archivedAt,
  reason: s.reason,
  batch: ITEMS.filter((o) => o.batch && o.batch === s.batch).length > 1,
});

const words = (s: string) => s.toLowerCase();
function search(input: { query?: string; kind?: Kind; limit?: number }) {
  const q = words(input.query ?? '').trim();
  const items = ITEMS.filter((s) => !input.kind || s.kind === input.kind)
    .filter((s) => !q || words(`${s.title} ${s.domain ?? ''}`).includes(q))
    .sort((a, b) => b.archivedAt - a.archivedAt)
    .slice(0, input.limit ?? 50)
    .map(panelItem);
  return { items, nextCursor: null, totalArchived: 214, tookMs: 4 };
}

// Insights: 30 days, open tabs falling once automatic mode started (day 8).
const days = Array.from({ length: 30 }, (_, i) => {
  const d = new Date(NOW - (29 - i) * 24 * HOUR);
  const day = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
  const auto = i >= 7;
  const tabs = auto ? Math.round(58 - (i - 7) * 1.7 + ((i * 7) % 5)) : 56 + ((i * 3) % 6);
  const open = Math.max(tabs, 18);
  const hours = Array.from({ length: 9 }, (_, h) => 9 + h);
  const ms = auto ? 2400 + ((i * 131) % 900) : 0;
  return {
    day,
    activeSeconds: 6 * 3600 + ((i * 977) % 7200),
    openTabsP50: open,
    openWindowsP50: auto ? 4 : 7,
    archivedCount: auto ? 6 + ((i * 5) % 9) : 0,
    restored24hCount: auto && i % 5 === 0 ? 1 : 0,
    undoCount: 0,
    searchCount: auto ? 2 + (i % 4) : 0,
    searchToRestoreMsP50: ms,
    autoModeOn: (auto ? 1 : 0) as 0 | 1,
    shadowFlaggedCount: auto ? 0 : 5 + (i % 4),
    shadowRegretCount: 0,
    sampledHours: hours,
    openTabsSamples: hours.map((_, h) => open + ((h * 3) % 5) - 2),
    openWindowsSamples: hours.map(() => (auto ? 4 : 7)),
    searchToRestoreSamples: auto ? [ms - 300, ms, ms + 250] : [],
  };
});

const rules = [
  ['domain', 'mail.google.com', 'protect'],
  ['domain', 'figma.com', 'protect'],
  ['domain', 'calendar.google.com', 'protect'],
  ['domain', 'youtube.com', 'customThreshold'],
].map(([scope, match, action], i) => ({
  id: `00000000-0000-4000-8000-0000000001${i}0`,
  scope,
  match,
  action,
  thresholdHours: action === 'customThreshold' ? { stale: 24, hidden: 60, archive: 96 } : undefined,
  enabled: true,
  source: 'user',
  createdAt: NOW - (20 - i) * 24 * HOUR,
  hitCount: [42, 17, 9, 5][i],
}));

const settings: Record<string, unknown> = {
  mode: 'automatic',
  preset: 'balanced',
  window_management: 'hide',
  theme: 'light',
  locale: 'en',
  hotkey: 'Alt+Command+D',
  launch_at_login: true,
  crash_reports: false,
  notifications: true,
  name_batches: true,
};

const status = {
  mode: 'automatic',
  holdUntil: null,
  staleCount: 3,
  liveTabs: 23,
  browsers: [{ browser: 'chrome', profileId: 'p1', tabs: 23 }],
  sweeping: false,
  undoableSweep: 's12',
  accessibility: true,
  inMeeting: false,
};

const tell = (msg: Record<string, unknown>) =>
  window.parent?.postMessage({ decay: true, ...msg }, '*');
const restored = (n: number) =>
  setTimeout(() => void emit('items:restored', { restored: n, failed: 0, skipped: 0 }), 150);

mockIPC(
  (cmd, args) => {
    const a = (args ?? {}) as Record<string, any>;
    switch (cmd) {
      case 'get_status':
        return status;
      case 'get_settings':
      case 'update_settings':
        return settings;
      case 'system_language':
        return 'en';
      case 'get_license':
        return { tier: 'pro', trialAvailable: false, email: 'you@example.com' };
      case 'ui_take_pending':
        return { prefill: null, licenseKey: null };
      case 'whats_new':
        return { version: '0.1.5', show: false };
      case 'list_profiles':
        return [
          {
            appId: 'com.google.Chrome',
            appName: 'Google Chrome',
            profileId: 'p1',
            name: null,
            ordinal: 1,
            hasSiblings: false,
            connected: true,
            tabs: 23,
            firstSeenAt: NOW - 30 * 24 * HOUR,
            lastSeenAt: NOW,
          },
        ];
      case 'library_stats':
        return {
          archivedTotal: 214,
          archivedToday: { tabs: 9, windows: 1 },
          restoredWeek: 11,
          openTabs: 23,
          mode: 'automatic',
          shadowDay: null,
        };
      case 'search':
        return search(a.input ?? {});
      case 'record_search':
      case 'log_ui_error':
      case 'ui_show_window':
      case 'ui_show_item':
      case 'whats_new_seen':
      case 'open_link':
        return null;
      case 'ui_hide_panel':
        tell({ type: 'panel-hidden' });
        return null;
      case 'restore_items': {
        const ids: string[] = a.ids ?? [];
        tell({
          type: 'restore',
          titles: ITEMS.filter((s) => ids.includes(s.id)).map((s) => s.title),
        });
        restored(ids.length);
        return { restored: ids.length };
      }
      case 'restore_batch': {
        const first = ITEMS.find((s) => s.id === a.itemId);
        const batch = ITEMS.filter((s) => s.batch && s.batch === first?.batch);
        tell({ type: 'restore', batch: true, titles: batch.map((s) => s.title) });
        restored(batch.length);
        return { restored: batch.length };
      }
      case 'get_metrics':
        return {
          mode: 'automatic',
          days,
          topArchived: [
            { domain: 'stackoverflow.com', count: 38 },
            { domain: 'docs.google.com', count: 27 },
            { domain: 'github.com', count: 22 },
            { domain: 'booking.com', count: 12 },
          ],
          restoredMost: [
            { domain: 'docs.google.com', count: 6 },
            { domain: 'github.com', count: 4 },
          ],
          topSources: [
            {
              appId: 'com.google.Chrome',
              appName: 'Google Chrome',
              profile: null,
              tabs: 201,
              windows: 0,
            },
            { appId: 'com.apple.finder', appName: 'Finder', profile: null, tabs: 0, windows: 13 },
          ],
          archivedTotal: 214,
        };
      case 'list_sweeps':
        return [
          {
            id: 's12',
            number: 12,
            startedAt: SWEEP_AT,
            trigger: 'scheduled',
            isShadow: false,
            status: 'done',
            itemCount: 9,
            tabs: 9,
            windows: 0,
            restored: 0,
            failed: 0,
            undoneAt: null,
            undoable: true,
          },
        ];
      case 'list_rules':
        return rules;
      case 'get_diagnostics_info':
        return { crashReportsAvailable: true, logsDir: '~/Library/Application Support/Decay/logs' };
      case 'get_update_state':
        return {
          current: '0.1.5',
          phase: 'idle',
          version: null,
          notes: null,
          date: null,
          downloaded: 0,
          total: null,
          checkedAt: NOW - 2 * HOUR,
          error: null,
          failed: null,
          installOnQuit: false,
          installNow: false,
        };
      case 'detect_browsers':
        return [];
      case 'get_daily_summary':
        return {
          archivedTabs: 9,
          hiddenWindows: 1,
          restored: 2,
          hints: [],
          items: ITEMS.slice(0, 5).map(panelItem),
        };
      default:
        console.warn('[demo] unmocked command', cmd, args);
        return null;
    }
  },
  { shouldMockEvents: true },
);

// The stage opens the panel: a new opening, like the hotkey.
window.addEventListener('message', (e) => {
  if (e.data?.decay && e.data.type === 'panel-open') void emit('panel:open', {});
});

await import('../src/main.tsx');
