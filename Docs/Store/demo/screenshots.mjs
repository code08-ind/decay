// Store screenshots (1280×800) of the real extension with sample data. Like record.mjs it
// loads a copy of the Chrome build with stage.html / stage.js added (recording-only pages,
// never shipped), opens sample tabs, ages some of them directly in IndexedDB, runs real
// shadow and automatic sweeps, and captures each screen with a caption.
//
//   pnpm --filter @decay/extension build
//   rm -rf /tmp/decay-shots-ext && cp -R apps/extension/.output/chrome-mv3 /tmp/decay-shots-ext
//   cp Docs/Store/demo/stage.* /tmp/decay-shots-ext/
//   node Docs/Store/demo/screenshots.mjs /tmp/decay-shots-ext Docs/Store/assets
import { chromium } from 'playwright';
import https from 'node:https';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const [EXT, OUT] = process.argv.slice(2);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const PAGES = {
  'docs.google.com': 'Q3 planning notes – Google Docs',
  'github.com': 'Pull requests · acme/web',
  'www.figma.com': 'Onboarding flow v3 – Figma',
  'linear.app': 'DEC-214 Popup search ranking – Linear',
  'stackoverflow.com': 'How do I cancel a fetch request? - Stack Overflow',
  'developer.mozilla.org': 'Using the Fetch API – MDN',
  'calendar.google.com': 'Google Calendar – Week of 23 November',
  'www.notion.so': 'Reading list – Notion',
  'news.ycombinator.com': 'Show HN: A local-first tab archive | Hacker News',
  'www.booking.com': 'Lisbon apartments, 12–16 Dec – Booking.com',
  'en.wikipedia.org': 'Azulejo – Wikipedia',
  'www.youtube.com': 'Rust async, explained – YouTube',
  'www.timeout.com': 'The 30 best things to do in Lisbon – Time Out',
  'www.visitlisboa.com': 'Tram 28: Lisbon’s most famous ride – Visit Lisboa',
  'www.tripadvisor.com': 'Lisbon restaurants in Alfama – Tripadvisor',
};
const STALE = [
  'docs.google.com', 'stackoverflow.com', 'developer.mozilla.org',
  'www.booking.com', 'en.wikipedia.org', 'news.ycombinator.com',
  'www.timeout.com', 'www.visitlisboa.com', 'www.tripadvisor.com',
];
// HTTPS with a throwaway self-signed certificate: many of these sites are HSTS-preloaded,
// so Chrome would refuse plain HTTP for them.
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'decay-cert-'));
execFileSync('openssl', ['req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1', '-subj', '/CN=localhost',
  '-keyout', `${tmp}/key.pem`, '-out', `${tmp}/cert.pem`], { stdio: 'ignore' });
const srv = https.createServer({ key: fs.readFileSync(`${tmp}/key.pem`), cert: fs.readFileSync(`${tmp}/cert.pem`) }, (req, res) => {
  const host = (req.headers.host || '').split(':')[0];
  res.setHeader('content-type', 'text/html; charset=utf-8');
  res.end(`<!doctype html><title>${PAGES[host] ?? host}</title><body style="font:20px sans-serif;padding:40px">${PAGES[host] ?? host}</body>`);
}).listen(8767);
fs.rmSync(tmp, { recursive: true, force: true });

const prof = fs.mkdtempSync(path.join(os.tmpdir(), 'decay-shots-'));
fs.rmSync(prof, { recursive: true, force: true });
fs.mkdirSync(OUT, { recursive: true });
const ctx = await chromium.launchPersistentContext(prof, {
  headless: true, channel: 'chromium', viewport: { width: 1280, height: 800 }, deviceScaleFactor: 1,
  args: [`--disable-extensions-except=${EXT}`, `--load-extension=${EXT}`, '--host-resolver-rules=MAP * 127.0.0.1:8767', '--ignore-certificate-errors'],
});
const sw = ctx.serviceWorkers()[0] ?? (await ctx.waitForEvent('serviceworker'));
const id = new URL(sw.url()).host;
const stage = ctx.pages()[0];
await stage.goto(`chrome-extension://${id}/stage.html`);
const ev = (fn, arg) => stage.evaluate(fn, arg);
for (const h of Object.keys(PAGES)) await ev((url) => chrome.tabs.create({ url, active: false }), `https://${h}/`);
await sleep(1500);

const put = (store, rows) => ev(({ store, rows }) => new Promise((res) => {
  const r = indexedDB.open('decay');
  r.onsuccess = () => { const tx = r.result.transaction(store, 'readwrite'); for (const x of rows) tx.objectStore(store).put(x); tx.oncomplete = () => { r.result.close(); res(); }; };
}), { store, rows });
const all = (store) => ev((store) => new Promise((res) => {
  const r = indexedDB.open('decay');
  r.onsuccess = () => { const q = r.result.transaction(store).objectStore(store).getAll(); q.onsuccess = () => { r.result.close(); res(q.result); }; };
}), store);
const tick = async () => {
  await ev(async () => {
    await chrome.storage.session.set({ 'decay.lastTickAt': Date.now() - 30000 });
    await chrome.alarms.create('decay-tick', { when: Date.now() + 50, periodInMinutes: 0.5 });
  });
  await sleep(2500);
};
const setting = (key, value) => ({ key, value, updatedAt: Date.now() });
await tick(); await tick();

// 30 days of on-device metrics: open tabs fall from ~46 to ~18 once Decay runs.
const days = [];
for (let i = 30; i >= 1; i--) {
  const d = new Date(Date.now() - i * 864e5);
  const day = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
  const n = 30 - i; // 0…29
  const auto = n >= 7 ? 1 : 0;
  const tabs = Math.round(n < 7 ? 46 - n * 0.6 + (n % 3) : 42 - (n - 7) * 1.1 + (n % 2));
  days.push({
    day, activeSeconds: (5 + (n % 4)) * 3600, openTabsP50: tabs, openWindowsP50: n < 7 ? 4 : 3,
    archivedCount: auto ? 3 + (n % 5) : 0, restored24hCount: auto && n % 6 === 0 ? 1 : 0, undoCount: 0,
    searchCount: auto ? 2 + (n % 4) : 1, searchToRestoreMsP50: 2600 + (n % 5) * 300, autoModeOn: auto,
    shadowFlaggedCount: auto ? 0 : 4 + (n % 3), shadowRegretCount: 0,
    // The chart only counts days with hourly samples behind the medians.
    sampledHours: [9, 10, 11, 12, 13, 14, 15, 16], openTabsSamples: Array(8).fill(tabs),
    openWindowsSamples: Array(8).fill(n < 7 ? 4 : 3), searchToRestoreSamples: [2600 + (n % 5) * 300],
  });
}
await put('metrics', days);

const shoot = async (name, caption, page, w, h, after) => {
  await ev(({ caption, page, w, h }) => { say(...caption); show(page, w, h); }, { caption, page, w, h });
  await sleep(2500);
  if (after) { await after(stage.frameLocator('#view')); await sleep(1800); }
  await stage.screenshot({ path: `${OUT}/${name}.png` });
  console.log('shot', name);
};

// ---- Shadow mode: what Decay would archive ----
await put('settings', [setting('mode', 'shadow'), setting('shadowStartedAt', Date.now() - 5 * 864e5)]);
const live = (await all('items')).filter((i) => i.state !== 'archived' && STALE.some((h) => i.url?.includes(h)));
await put('items', live.map((i) => ({ ...i, activeAgeS: 130 * 3600 })));
console.log('shadow sweep', JSON.stringify(await ev(() => chrome.runtime.sendMessage({ type: 'decay.sweepNow' }))));
await sleep(3000);
await shoot('screenshot-4-review',
  ['Shadow mode first', 'See what it would do before it does anything',
    'For the first 7 active days nothing closes. Decay shows which tabs it would have archived, so you can check its judgement.', ''],
  'review.html', 780, 600);

// ---- Automatic mode: a real sweep ----
await put('settings', [setting('mode', 'automatic')]);
await tick();
await put('items', (await all('items')).filter((i) => i.state !== 'archived' && STALE.some((h) => i.url?.includes(h))).map((i) => ({ ...i, activeAgeS: 130 * 3600 })));
console.log('sweep', JSON.stringify(await ev(() => chrome.runtime.sendMessage({ type: 'decay.sweepNow' }))));
await sleep(6000);
const items = await all('items');
console.log('archived:', items.filter((i) => i.state === 'archived').length);

await shoot('screenshot-3-sweeps',
  ['Quiet sweeps', 'Stale tabs are put away, with Undo',
    'Tabs age by active browsing time, not the clock. Sweeps run at a quiet moment, and every one can be undone.', 'Never touches pinned tabs, tabs playing audio, or sites you keep.'],
  'sweeps.html', 780, 600);
await shoot('screenshot-2-popup',
  ['Alt+Shift+D', 'Your archive, one shortcut away',
    'The popup shows what Decay is doing, your last sweep with Undo, and the tabs it put away most recently.', ''],
  'popup.html', 400, 600);
await shoot('screenshot-1-search',
  ['Search and restore', 'Need a tab back? Type a few letters',
    'Search titles and sites. Enter reopens the tab right where it was; Ctrl+Enter brings back the whole batch it left with.', ''],
  'popup.html', 400, 600,
  async (view) => { await view.locator('input').first().pressSequentially('lisbon', { delay: 60 }); });
await shoot('screenshot-5-insights',
  ['Insights', 'Fewer open tabs, nothing lost',
    'See your open tabs fall, how often you needed one back, and how fast search found it. All counted on your computer.', 'Sample data shown.'],
  'insights.html', 780, 452,
  async () => {
    await stage.frameLocator('#view').locator('body').evaluate(() => {
      const h = [...document.querySelectorAll('h2, h3')].find((e) => /Open tabs/.test(e.textContent ?? ''));
      h?.closest('section, article, div')?.scrollIntoView({ block: 'start' });
      window.scrollBy(0, -20);
    });
  });

await ctx.close();
srv.close();
fs.rmSync(prof, { recursive: true, force: true });
