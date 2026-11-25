// Chrome Web Store promo tiles: small 440×280 (required) and marquee 1400×560 (optional,
// used if the store features Decay). Brand colours and fonts from packages/ui; the marquee
// shows the real popup screenshot made by screenshots.mjs.
//
//   node Docs/Store/demo/promo.mjs <chrome-mv3 build dir> Docs/Store/assets
import { chromium } from 'playwright';
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const [EXT, OUT] = process.argv.slice(2);
const ROOT = path.resolve(path.dirname(new URL(import.meta.url).pathname.replace(/%20/g, ' ')), '../../..');
const font = (re) => pathToFileURL(path.join(EXT, 'assets', fs.readdirSync(path.join(EXT, 'assets')).find((f) => re.test(f)))).href;
const icon = pathToFileURL(path.join(ROOT, 'apps/desktop/src-tauri/icons/icon.png')).href;
const popup = pathToFileURL(path.resolve(OUT, 'screenshot-1-search.png')).href;

const css = `
@font-face{font-family:Newsreader;src:url(${font(/newsreader-latin-400/)})}
@font-face{font-family:'DM Sans';src:url(${font(/dm-sans-latin-400/)})}
@font-face{font-family:'DM Sans';font-weight:500;src:url(${font(/dm-sans-latin-500/)})}
*{box-sizing:border-box;margin:0}
body{font-family:'DM Sans',sans-serif;background:#f7f3e8;color:#1d2219;overflow:hidden}
.brand{display:flex;align-items:center;gap:14px;font-family:Newsreader,serif}
.brand img{border-radius:22%}
h1{font-family:Newsreader,serif;font-weight:400;letter-spacing:-.01em;line-height:1.05}
.chip{display:inline-flex;align-items:center;gap:8px;border-radius:999px;background:#ddeb9d;color:#2f4614;font-weight:500}
`;

const small = `<!doctype html><meta charset="utf-8"><style>${css}
body{width:440px;height:280px;background:#3d6f3a;color:#fff;padding:34px 36px;display:flex;flex-direction:column;justify-content:space-between}
.brand{font-size:30px}.brand img{background:#f7f3e8;padding:6px;border-radius:12px}h1{font-size:40px;color:#fff}p{font-size:15px;color:#eef6e6}
</style>
<div class="brand"><img src="${icon}" width="46" height="46">Decay</div>
<div><h1>Your tabs,<br>gently put away.</h1></div>
<p>Archives stale tabs · search &amp; restore anything</p>`;

const marquee = `<!doctype html><meta charset="utf-8"><style>${css}
body{width:1400px;height:560px;display:grid;grid-template-columns:640px 1fr;gap:40px;padding:0 0 0 88px}
.text{display:flex;flex-direction:column;justify-content:center;gap:26px}
.brand{font-size:34px}h1{font-size:70px}p{font-size:22px;line-height:1.45;color:#5d6253;max-width:540px}
.chip{font-size:16px;padding:8px 16px;align-self:flex-start}
.shot{position:relative;background:#3d6f3a;border-radius:48px 0 0 48px;margin:44px 0;overflow:hidden}
.popup{position:absolute;left:150px;top:46px;width:400px;height:420px;border-radius:18px;overflow:hidden;box-shadow:0 24px 60px rgba(20,30,15,.35)}
.popup img{position:absolute;left:-650px;top:-88px;width:1280px}
</style>
<div class="text">
  <div class="brand"><img src="${icon}" width="52" height="52">Decay</div>
  <h1>Your tabs,<br>gently put away.</h1>
  <p>Decay archives the tabs you stopped using and brings any of them back in a second. Everything stays on your computer.</p>
  <span class="chip">Alt+Shift+D · search and restore</span>
</div>
<div class="shot"><div class="popup"><img src="${popup}"></div></div>`;

const browser = await chromium.launch();
for (const [name, html, w, h] of [
  ['promo-small-440x280', small, 440, 280],
  ['promo-marquee-1400x560', marquee, 1400, 560],
]) {
  const page = await browser.newPage({ viewport: { width: w, height: h }, deviceScaleFactor: 1 });
  const file = path.join(OUT, `.${name}.html`);
  fs.writeFileSync(file, html);
  await page.goto(pathToFileURL(file).href);
  await page.waitForTimeout(600);
  await page.screenshot({ path: path.join(OUT, `${name}.png`) });
  fs.rmSync(file);
  console.log('tile', name);
}
await browser.close();
