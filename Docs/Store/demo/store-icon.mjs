// Decay's icons for the browser extension, drawn from the app icon's SVG
// (apps/desktop/assets/app-icon.svg) so every icon shows the same leaf, tip to the top left:
// - the extension's own icons (apps/extension/public/icon/{16,32,48,128}.png), full size;
// - the Chrome Web Store icon: 128×128 with 96×96 artwork in 16 px of transparent padding
//   (the store's icon guideline).
//   node Docs/Store/demo/store-icon.mjs
import { chromium } from 'playwright';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const svg = fs.readFileSync(path.join(root, 'apps/desktop/assets/app-icon.svg'), 'utf8');
const sized = (n) => svg.replace(/width="1024" height="1024"/, `width="${n}" height="${n}"`);
const jobs = [
  ...[16, 32, 48, 128].map((n) => ({ out: `apps/extension/public/icon/${n}.png`, size: n, art: n })),
  { out: 'Docs/Store/assets/store-icon-128.png', size: 128, art: 96 },
];
const browser = await chromium.launch();
for (const { out, size, art } of jobs) {
  const page = await browser.newPage({ viewport: { width: size, height: size }, deviceScaleFactor: 1 });
  const pad = (size - art) / 2;
  await page.setContent(`<style>html,body{margin:0;background:transparent}svg{display:block;margin:${pad}px}</style>${sized(art)}`);
  await page.screenshot({ path: path.join(root, out), omitBackground: true });
  await page.close();
  console.log('icon', out);
}
await browser.close();
