// Records the launch video (Docs/Launch/Demo video.md) from the real desktop UI with sample
// data: apps/desktop/demo/stage.html, served by the desktop app's Vite dev server.
//
//   (cd apps/desktop && npx vite --port 1420)            # the dev server
//   node Docs/Launch/demo/record-launch.mjs <out dir>    # frames + decay-launch.mp4 + 30 s cut
//
// Playwright: install it outside the repo and symlink its node_modules here (git-ignored),
// like Docs/Store/demo. Frames come from Chromium's screencast (sharper than Playwright's
// video), timed by their timestamps, then encoded by ffmpeg.
import { chromium } from 'playwright';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';

const OUT = path.resolve(process.argv[2] ?? 'launch-video');
const FRAMES = path.join(OUT, 'frames');
fs.rmSync(FRAMES, { recursive: true, force: true });
fs.mkdirSync(FRAMES, { recursive: true });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 1, colorScheme: 'light' });
page.on('pageerror', (e) => console.error('page error:', e.message));
await page.goto('http://localhost:1420/demo/stage.html');
await page.waitForFunction(() => 'scene' in window);
const scene = (name, ...args) => page.evaluate(([n, a]) => window.scene[n](...a), [name, args]);
await scene('setup');

// Screencast: every repaint, with its time.
const cdp = await page.context().newCDPSession(page);
const frames = [];
cdp.on('Page.screencastFrame', async ({ data, metadata, sessionId }) => {
  const file = path.join(FRAMES, `${String(frames.length).padStart(5, '0')}.jpg`);
  fs.writeFileSync(file, Buffer.from(data, 'base64'));
  frames.push({ file, t: metadata.timestamp, at: Date.now() / 1000 });
  await cdp.send('Page.screencastFrameAck', { sessionId }).catch(() => {});
});
await cdp.send('Page.startScreencast', { format: 'jpeg', quality: 92, maxWidth: 1920, maxHeight: 1080 });
const t0 = Date.now();
let tEnd = 0;
const marks = {};
const mark = (name) => (marks[name] = (Date.now() - t0) / 1000);
const until = async (s) => {
  const wait = t0 + s * 1000 - Date.now();
  if (wait > 0) await sleep(wait);
};

const panel = page.frameLocator('#panel');
const library = page.frameLocator('#library-frame');
const settings = page.frameLocator('#settings-frame');

// 1 · Too many tabs (0–4 s)
mark('s1');
await scene('caption', 'Too many tabs?');
await scene('scrollTabs');
await until(4);

// 2 · A sweep (4–10 s)
mark('s2');
await scene('caption', 'Decay quietly archives the ones you stopped using');
await sleep(600);
await scene('sweep');
await until(10);

// 3 · Undo (10–14 s)
mark('s3');
await scene('caption', 'Every sweep has Undo');
await scene('undo');
await until(14);
await scene('caption', null);
await scene('later');

// 4 · Search and restore one (≈16–25 s)
mark('s4');
await scene('caption', 'Need one back? ⌥⌘D, type, Return');
await scene('keys', ['⌥', '⌘', 'D']);
await sleep(500);
await scene('openPanel');
await sleep(400);
await scene('keys', null);
const input = panel.locator('input').first();
await input.pressSequentially('tram', { delay: 150 });
await sleep(900);
const restoredOne = page.evaluate(() => window.scene.waitRestore());
await input.press('Enter');
await restoredOne;
await sleep(1800);

// 5 · The whole batch (Pro)
mark('s5');
await scene('caption', 'Or bring back the whole batch (Pro)');
await scene('openPanel');
await sleep(300);
await input.pressSequentially('lisbon', { delay: 120 });
await sleep(1000);
await input.press('ArrowDown');
await sleep(500);
const restoredBatch = page.evaluate(() => window.scene.waitRestore());
await input.press('Meta+Enter');
await restoredBatch;
await sleep(1800);

// 6 · Library and Insights
mark('s6');
await scene('caption', 'It ages tabs by active time, not the clock');
await scene('library', true);
let p = await page.evaluate(() => window.scene.libraryPoint(862, 457));
await scene('moveTo', p.x, p.y, 900);
await sleep(1600);
p = await page.evaluate(() => window.scene.libraryPoint(720, 31));
await scene('moveTo', p.x, p.y, 800);
await scene('click');
await library.getByText('Insights', { exact: true }).first().evaluate((el) => el.click());
await sleep(2800);

// 7 · Rules
mark('s7');
await scene('library', false);
await scene('caption', 'Never touches pinned tabs, audio, or sites you keep');
await scene('settings', true);
p = await page.evaluate(() => window.scene.settingsPoint(420, 404));
await scene('moveTo', p.x, p.y, 900);
await sleep(2600);

// 8 · Privacy
mark('s8');
await scene('caption', 'Local only. No account. No analytics.');
p = await page.evaluate(() => window.scene.settingsPoint(95, 182));
await scene('moveTo', p.x, p.y, 700);
await scene('click');
await settings.getByText('Privacy & data', { exact: true }).first().evaluate((el) => el.click());
await sleep(3200);

// 9 · End card
mark('s9');
await scene('end');
await sleep(4800);
mark('end');
tEnd = Date.now() / 1000;

await cdp.send('Page.stopScreencast');
await browser.close();

// Frames → video: each frame lasts until the next one.
const list = [];
for (let i = 0; i < frames.length; i++) {
  // The last frame (the still end card) lasts until the recording ended.
  const next = frames[i + 1]?.t ?? frames[i].t + Math.max(tEnd - frames[i].at, 0.5);
  list.push(`file '${frames[i].file}'`, `duration ${Math.max(next - frames[i].t, 0.001).toFixed(4)}`);
}
list.push(`file '${frames.at(-1).file}'`);
// ffmpeg shows that repeated last entry for the whole last duration again: cap the length.
const total = frames.at(-1).t - frames[0].t + Math.max(tEnd - frames.at(-1).at, 0.5);
fs.writeFileSync(path.join(OUT, 'frames.txt'), list.join('\n'));
const full = path.join(OUT, 'decay-launch.mp4');
const enc = ['-c:v', 'libx264', '-preset', 'slow', '-crf', '18', '-pix_fmt', 'yuv420p', '-movflags', '+faststart'];
execFileSync('ffmpeg', ['-loglevel', 'error', '-y', '-f', 'concat', '-safe', '0', '-i', path.join(OUT, 'frames.txt'), '-vf', 'fps=30', '-t', total.toFixed(3), ...enc, full]);

// Marks are wall-clock; the video runs on the frames' own timestamps: map one to the other.
const t0Wall = t0 / 1000;
const videoTime = (markS) => {
  const wall = t0Wall + markS;
  let f = frames[0];
  for (const g of frames) if (g.at <= wall) f = g;
  return Math.max(f.t - frames[0].t + (wall - f.at), 0);
};
for (const k of Object.keys(marks)) marks[k] = Number(videoTime(marks[k]).toFixed(3));

// 30 s cut for X / Bluesky: shots 1, 2, 4 and the end card.
const part = (from, to, name) => {
  const f = path.join(OUT, name);
  execFileSync('ffmpeg', ['-loglevel', 'error', '-y', '-ss', String(from), '-to', String(to), '-i', full, ...enc, f]);
  return f;
};
const parts = [part(marks.s1, marks.s3, 'p1.mp4'), part(marks.s4, marks.s5, 'p2.mp4'), part(marks.s9, marks.end, 'p3.mp4')];
fs.writeFileSync(path.join(OUT, 'cut.txt'), parts.map((f) => `file '${f}'`).join('\n'));
execFileSync('ffmpeg', ['-loglevel', 'error', '-y', '-f', 'concat', '-safe', '0', '-i', path.join(OUT, 'cut.txt'), ...enc, path.join(OUT, 'decay-launch-30s.mp4')]);
for (const f of parts) fs.rmSync(f);
fs.writeFileSync(path.join(OUT, 'marks.json'), JSON.stringify(marks, null, 2));
console.log('frames', frames.length, 'marks', marks);
