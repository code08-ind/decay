# Chrome Web Store listing — Decay (public)

Everything to enter in the Chrome Web Store Developer Dashboard (https://chrome.google.com/webstore/devconsole), tab by tab, for the **public** listing. Updated 25 Nov 2026 for extension 0.1.0 as built from the current code (the desktop app link, the Insights page, translated strings). Edge Add-ons takes the same package and text (its own dashboard: https://partner.microsoft.com/dashboard/microsoftedge).

## What to upload

| What | File | Notes |
| --- | --- | --- |
| Package | `apps/extension/.output/decayextension-0.1.0-chrome.zip` | `pnpm --filter @decay/extension zip`. 569 KB, no source maps, no remote code. If a 0.1.0 was ever uploaded to this item before (the unlisted beta), bump `version` in `apps/extension/package.json` first: the store refuses a version it has seen. |
| Store icon | `Docs/Store/assets/store-icon-128.png` | 128×128: 96×96 artwork with 16 px transparent padding, per the store's icon guideline, drawn from `apps/desktop/assets/app-icon.svg` like the extension's own icons, leaf tip to the top left (`Docs/Store/demo/store-icon.mjs`) |
| Screenshots, in this order | `Docs/Store/assets/screenshot-1-search.png` … `screenshot-5-insights.png` | 1280×800, the real extension with sample data (`Docs/Store/demo/screenshots.mjs`) |
| Small promo tile (required) | `Docs/Store/assets/promo-small-440x280.png` | 440×280 |
| Marquee promo tile (optional) | `Docs/Store/assets/promo-marquee-1400x560.png` | 1400×560; only used if the store features Decay |
| Promo video (optional) | — | `assets/decay-demo.mp4` shows the 7 Nov UI; leave the field empty, or re-record (`Docs/Store/demo/README.md`) and upload to YouTube first |

Regenerate the images after UI changes:

```sh
pnpm --filter @decay/extension build
rm -rf /tmp/decay-shots-ext && cp -R apps/extension/.output/chrome-mv3 /tmp/decay-shots-ext
cp Docs/Store/demo/stage.* /tmp/decay-shots-ext/
node Docs/Store/demo/screenshots.mjs /tmp/decay-shots-ext Docs/Store/assets
node Docs/Store/demo/promo.mjs /tmp/decay-shots-ext Docs/Store/assets
```

(Playwright: `Docs/Store/demo/README.md`. The scripts need nothing else; the sample pages are served locally.)

---

## Store listing tab

**Name** (from the manifest): `Decay`

**Summary** (from the manifest, ≤ 132 characters):

> Your tabs, gently put away. Archives stale tabs; search and restore anything.

**Description:**

```
Decay retires the tabs you've stopped using, and brings any of them back in a second.

People keep 30, 50, 100 tabs open because closing one feels like losing it. Decay treats your tabs like a cache: what you stop using quietly ages out into a searchable archive, and Alt+Shift+D finds and reopens it exactly where it was.

HOW IT WORKS
• Ages tabs by active browsing time, not the clock. A weekend away doesn't make Monday's tabs "old".
• Starts in shadow mode: for the first 7 active days nothing is closed. Decay shows you what it would have archived, so you can check its judgement first.
• Then, in automatic mode, stale tabs are archived in small, quiet sweeps, with Undo after each one.
• Never touches pinned tabs, tabs playing audio, or sites you've marked "Never archive".
• With your permission, checks a page for unsaved typing, an upload in progress or a "leave site?" prompt before archiving it. Pages it can't check get twice as long.

SEARCH AND RESTORE
• Alt+Shift+D opens the search popup. Type a few letters of a title or site, press Enter, and the tab is back.
• Ctrl+Enter (⌘+Enter on a Mac) restores the whole batch a tab was archived with, in its original order and tab groups.
• The Sweeps page lists every sweep; the last 20 can be undone.

INSIGHTS
• See your open tabs fall over time, how often you needed an archived tab back, and how fast search found it. Counted on your computer.

WORKS WITH THE DECAY DESKTOP APP (OPTIONAL)
The free Decay app for macOS and Windows does the same for your app windows, across Chrome, Edge, Brave, Arc and Firefox. Install it and click "Connect" in the popup: the extension and the app talk directly on your computer, never over the network. The extension works fully on its own without it. https://decay.aryangarg.com

PRIVATE BY DESIGN
• Everything stays on your computer. No account, no server, no analytics, no ads.
• The extension makes no network requests of its own.
• Open source: https://github.com/code08-ind/decay

Questions or bugs: support.decay@aryangarg.com
```

**Category:** Productivity → **Workflow & Planning** (or Tools).

**Language:** English.

**Additional fields:**

| Field | Value |
| --- | --- |
| Official URL | `https://decay.aryangarg.com` if the domain shows in the list (it must be verified in Google Search Console with the same Google account; `aryangarg.com` already has a Search Console TXT record). Otherwise "None". |
| Homepage URL | `https://decay.aryangarg.com` |
| Support URL | `https://decay.aryangarg.com/docs/browsers` |
| Mature content | No |

---

## Privacy practices tab

**Single purpose description:**

> Decay archives browser tabs that have gone unused for a while, and lets the user search and restore them. All of its features (aging tabs by active browsing time, archiving, the optional unsaved-work check, search, restore, undo, on-device statistics, and the optional link to the Decay desktop app on the same computer) serve that one purpose.

**Permission justifications** (one box each):

| Permission | Justification |
| --- | --- |
| `tabs` | Decay reads each open tab's URL, title and activity to measure how long it has gone unused, closes stale tabs after saving them to the on-device archive, and reopens them on restore. |
| `tabGroups` | Archived tabs remember their tab group (name and colour), so a restore puts them back in the same group. |
| `storage` | Stores the tab archive, the user's rules and settings, and daily usage counts locally (IndexedDB and chrome.storage). Nothing is synced or sent anywhere. |
| `alarms` | Runs the aging clock every 30 seconds in the MV3 service worker, which can't keep timers alive on its own. |
| `idle` | Counts only time when the user is actively at the computer, so tabs don't "age" overnight or while the user is away. |
| `scripting` | Only with the optional site access below: registers two small content scripts (watchers for typing, "leave site?" prompts and uploads in progress) and, just before archiving a stale tab, runs a one-off check in that tab that returns yes / no / unknown. |
| `favicon` | Shows site icons in the popup and pages from Chrome's local favicon cache, so Decay never downloads icons. |
| `notifications` | Shows "Archived N tabs" with Undo / View buttons after a sweep, and an "Always keep <site> open?" suggestion when a tab is restored soon after being archived. |
| `nativeMessaging` (optional) | Requested only when the user clicks "Connect" to link the Decay desktop app installed on the same computer. The extension then exchanges tab information and its archive with that app through the native messaging host the app installs. Nothing is sent over the network. |
| Host permissions (`http://*/*`, `https://*/*`, optional) | Requested at runtime only when the user clicks "Allow checks", and can be limited to specific sites. Used solely to check pages for unsaved form input, a "leave site?" handler or a file upload in progress before archiving. Typed text and upload contents are never read into storage; only a yes / no / unknown answer with the tab's address and check time is kept in session storage, until the tab closes or the browser quits (re-checked after 10–30 minutes). |

**Are you using remote code?** No, I am not using remote code. (All JavaScript ships in the package; no `eval`, no remotely hosted scripts, no WebAssembly from the network.)

**Data usage — what user data do you collect or use?**

Decay keeps tab URLs, titles and usage times on the user's device and never transmits them to a server. The conservative answer is to declare what it handles, and the privacy policy explains that it stays on the device. Tick:

- ☑ **Web history** (URLs and titles of tabs)
- ☑ **User activity** (when tabs were used)
- ☐ everything else (personally identifiable information, health, financial, authentication, personal communications, location, website content: Decay never reads page content)

Then certify all three:

- ☑ I do not sell or transfer user data to third parties, outside of the approved use cases
- ☑ I do not use or transfer user data for purposes that are unrelated to my item's single purpose
- ☑ I do not use or transfer user data to determine creditworthiness or for lending purposes

**Privacy policy URL:** `https://www.aryangarg.com/decay/privacy-policy`

---

## Distribution tab

- Payments: **Free of charge** (the extension has no paid features; the optional desktop app's Pro tier is sold on the website, not in the extension)
- Visibility: **Public**
- Distribution: **All regions**

---

## Test instructions tab (for the reviewer)

**Username / password:** none (leave empty).

**Additional instructions:**

```
No account or login is needed. The extension works on its own; the "Connect" button in the popup is for an optional desktop app and can be ignored.

1. Install it and open a few tabs. Decay starts in shadow mode, which closes nothing.
2. Click the toolbar icon (or press Alt+Shift+D) to open the popup.
3. To skip the 7-day shadow period: open "Shadow review" (from the popup), click "Turn on automatic mode early", then confirm.
4. Tabs become stale only after 60 hours of active browsing without being used (120 hours for pages Decay isn't allowed to check), so on a fresh install "Sweep now" on the Sweeps page has nothing to archive yet. The screenshots show the flow with aged tabs: a sweep archives the stale tabs, the popup search finds an archived tab, Enter restores it, Undo reopens the whole sweep.
5. Optional site access ("Allow checks") is used only to check pages for unsaved typing, uploads and "leave site?" prompts before archiving; typed text is never stored.

No remote code, no network requests of its own. Privacy policy: https://www.aryangarg.com/decay/privacy-policy. Source: https://github.com/code08-ind/decay (apps/extension).
```

---

## After the first upload

1. Item ID: **`jidljehgkacgjoefbancbihmdkkpcaaa`** (added to `BUILTIN_EXTENSION_IDS` on 25 Nov 2026). The dashboard shows the item's **ID** (32 letters) as soon as the package is uploaded, before review. Send it to Claude / put it in `crates/decay-host/src/config.rs` → `BUILTIN_EXTENSION_IDS`, so the released desktop app accepts the store build, and release the app (v0.1.1). Do the same for the Edge Add-ons ID if Edge is listed.
2. After approval: replace the store search link in `apps/desktop/src-tauri/src/onboarding.rs` (`STORE_URL`) and the website's docs with the listing URL (`https://chromewebstore.google.com/detail/<id>`).
3. Each new version: bump `apps/extension/package.json`, rebuild the zip, **Package → Upload new package**, and submit for review.

## Review tips (avoid the common rejections)

- Keep the description factual: no keyword lists, no other brands' names beyond the browsers it supports, no ratings or "best" claims.
- Every permission must be used by this exact package: the table above matches the manifest of 0.1.0. If a permission is added or removed later, update its justification in the same submission.
- Screenshots must show the real extension: they do (sample tab data, real UI). "Sample data shown" is on the Insights image.
- The privacy policy must cover what the extension does with tab data: it does (the extension section, plus native messaging).
- Optional host permissions (`http://*/*`, `https://*/*`) usually trigger a longer, manual review (a few days). That's normal.
