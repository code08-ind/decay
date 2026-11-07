import { chromium } from 'playwright';
import http from 'node:http';
import fs from 'node:fs';
const DIR = new URL('.', import.meta.url).pathname;
// Usage: node record.mjs <copy of .output/chrome-mv3 with stage.html + stage.js added>
const EXT = process.argv[2];
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const PAGES = {
  'docs.google.com': 'Q3 planning notes – Google Docs',
  'github.com': 'Pull requests · acme/web',
  'www.figma.com': 'Onboarding flow v3 – Figma',
  'linear.app': 'DEC-214 Popup search ranking – Linear',
  'stackoverflow.com': 'How do I cancel a fetch request? - Stack Overflow',
  'developer.mozilla.org': 'Using the Fetch API – MDN',
  'calendar.google.com': 'Google Calendar – Week of 9 November',
};
const STALE = ['docs.google.com', 'stackoverflow.com', 'developer.mozilla.org'];
const srv = http.createServer((req, res) => {
  const host = (req.headers.host || '').split(':')[0];
  res.setHeader('content-type', 'text/html; charset=utf-8');
  res.end(`<!doctype html><title>${PAGES[host] ?? host}</title><body style="font:20px sans-serif;padding:40px">${PAGES[host] ?? host}</body>`);
}).listen(8766);

const prof = `${DIR}prof`; fs.rmSync(prof, { recursive: true, force: true });
const vids = `${DIR}raw`; fs.rmSync(vids, { recursive: true, force: true });
const t0 = Date.now();
const ctx = await chromium.launchPersistentContext(prof, {
  headless: true, channel: 'chromium', viewport: { width: 1280, height: 800 },
  recordVideo: { dir: vids, size: { width: 1280, height: 800 } },
  args: [`--disable-extensions-except=${EXT}`, `--load-extension=${EXT}`, '--host-resolver-rules=MAP * 127.0.0.1'],
});
let sw = ctx.serviceWorkers()[0] ?? (await ctx.waitForEvent('serviceworker'));
const id = new URL(sw.url()).host;
// One window, like real Chrome: the stage is the active tab, the others are background tabs.
const stage = ctx.pages()[0];
await stage.goto(`chrome-extension://${id}/stage.html`);
const ev = (fn, arg) => stage.evaluate(fn, arg);
for (const h of Object.keys(PAGES)) await ev((url) => chrome.tabs.create({ url, active: false }), `http://${h}:8766/`);
await sleep(1500);
// Decay state: automatic mode, ticks so it knows the open tabs.
const idb = (fn, arg) => ev(fn, arg);
const put = (store, rows) => idb(({ store, rows }) => new Promise((res) => {
  const r = indexedDB.open('decay'); r.onsuccess = () => { const tx = r.result.transaction(store, 'readwrite'); for (const x of rows) tx.objectStore(store).put(x); tx.oncomplete = () => { r.result.close(); res(); }; };
}), { store, rows });
const all = (store) => idb((store) => new Promise((res) => {
  const r = indexedDB.open('decay'); r.onsuccess = () => { const q = r.result.transaction(store).objectStore(store).getAll(); q.onsuccess = () => { r.result.close(); res(q.result); }; };
}), store);
const tick = async () => { await ev(async () => { await chrome.storage.session.set({ 'decay.lastTickAt': Date.now() - 30000 }); await chrome.alarms.create('decay-tick', { when: Date.now() + 50, periodInMinutes: 0.5 }); }); await sleep(2500); };
await tick(); await tick();
await put('settings', [{ key: 'mode', value: 'automatic', updatedAt: Date.now() }, { key: 'shadowStartedAt', value: Date.now() - 9 * 864e5, updatedAt: Date.now() }]);
await stage.bringToFront();

// ---- recording starts to matter from here (first seconds are trimmed) ----
console.log('START', (Date.now() - t0) / 1000);
await ev(() => { window.ready = true; say('Decay · demo', 'Your tabs, gently put away', 'Decay archives the tabs you stopped using, by active browsing time, and brings any of them back in a second.', 'Real extension, recorded in Chromium. The strip above shows this window’s real open tabs.'); show('popup.html', 400, 590); });
await sleep(6000);
await ev(() => say('1 · Tabs go stale', 'Three tabs haven’t been used in a while', 'Google Docs, Stack Overflow and MDN have been unused for more than 60 hours of active browsing (120 h for pages Decay can’t check).', 'For this demo their age is set directly; normally it builds up over days of use.'));
const live = (await all('items')).filter((i) => i.state !== 'archived' && STALE.some((h) => i.url?.includes(h)));
await put('items', live.map((i) => ({ ...i, activeAgeS: 130 * 3600 })));
console.log('aged', live.length, live.map((i) => i.state));
await sleep(5000);
await ev(() => { say('2 · A sweep', 'Decay archives them, and keeps Undo', 'Sweeps normally run on their own at a quiet moment. Here we press “Sweep now” on the Sweeps page.', ''); show('sweeps.html', 780, 590); });
await sleep(3500);
const view = stage.frameLocator('#view');
await view.getByRole('button', { name: /Sweep now/ }).click();
await sleep(5500);

await ev(() => { say('3 · Search', 'Need one back? Alt+Shift+D', 'Type a few letters of the title or site. Enter reopens it right where it was.', ''); show('popup.html', 400, 590); });
await sleep(3000);
await view.locator('input').pressSequentially('q3 plan', { delay: 160 });
await sleep(2200);
await view.locator('input').press('Enter');
await sleep(800); await ev(() => chrome.tabs.getCurrent((t) => chrome.tabs.update(t.id, { active: true }))); await stage.setViewportSize({ width: 1280, height: 800 }); await sleep(4000);
await ev(() => { say('4 · Restore a batch', 'Or bring back the whole sweep', 'Ctrl+Enter (⌘+Enter on a Mac) reopens everything archived together, in its original order, in a new window.', ''); show('popup.html', 400, 590); });
await sleep(3000);
await view.locator('input').pressSequentially('fetch', { delay: 160 });
await sleep(2000);
await view.locator('input').press('Control+Enter');
await sleep(800); await ev(() => chrome.tabs.getCurrent((t) => chrome.tabs.update(t.id, { active: true }))); await stage.setViewportSize({ width: 1280, height: 800 }); await sleep(4500);
await ev(() => { say('Private by design', 'Everything stays in your browser', 'No account, no server, no analytics. Decay starts in shadow mode for 7 days, so nothing closes until you’ve seen what it would do.', 'Optional “Allow checks” lets Decay skip pages with unsaved typing or uploads.'); show('popup.html', 400, 590); });
await sleep(6000);

const items = await all('items');
console.log('archived now:', items.filter((i) => i.state === 'archived').length, 'restored:', items.filter((i) => (i.restoreCount ?? 0) > 0).length);
const path = await stage.video().path();
await ctx.close(); srv.close();
console.log('VIDEO', path);
