// Chrome Web Store icon: 128×128 PNG with 96×96 artwork centred in 16 px of transparent
// padding (the store's icon guideline). Drawn from the app icon's SVG
// (apps/desktop/assets/app-icon.svg) with the leaf turned +45° instead of −45°, so it points
// the same way as the extension's toolbar icon (apps/extension/public/icon/*.png).
//   node Docs/Store/demo/store-icon.mjs Docs/Store/assets/store-icon-128.png
import { chromium } from 'playwright';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const out = path.resolve(process.argv[2]);
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const svg = fs
  .readFileSync(path.join(root, 'apps/desktop/assets/app-icon.svg'), 'utf8')
  .replace('rotate(-45 512 512)', 'rotate(45 512 512)')
  .replace(/width="1024" height="1024"/, 'width="96" height="96"');
if (!svg.includes('rotate(45 512 512)')) throw new Error('app-icon.svg changed: check the leaf rotation');
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 128, height: 128 }, deviceScaleFactor: 1 });
await page.setContent(`<style>html,body{margin:0;background:transparent}svg{display:block;margin:16px}</style>${svg}`);
await page.screenshot({ path: out, omitBackground: true });
await browser.close();
console.log('icon', out);
