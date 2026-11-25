// Chrome Web Store icon: 128×128 PNG, the 512 px app icon scaled to 96×96 and centred with
// 16 px of transparent padding (the store's icon guideline).
//   node Docs/Store/demo/store-icon.mjs Docs/Store/assets/store-icon-128.png
import { chromium } from 'playwright';
import path from 'node:path';
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';

const out = path.resolve(process.argv[2]);
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const icon = `data:image/png;base64,${fs.readFileSync(path.join(root, 'apps/desktop/src-tauri/icons/icon.png')).toString('base64')}`;
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 128, height: 128 }, deviceScaleFactor: 1 });
await page.setContent(`<style>html,body{margin:0;background:transparent}img{display:block;margin:16px;width:96px;height:96px}</style><img src="${icon}">`);
await page.locator('img').evaluate((img) => img.decode());
await page.screenshot({ path: out, omitBackground: true });
await browser.close();
console.log('icon', out);
