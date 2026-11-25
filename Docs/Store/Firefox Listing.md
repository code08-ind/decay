# Firefox Add-ons (addons.mozilla.org) listing — Decay

Everything to paste into AMO's Developer Hub (https://addons.mozilla.org/developers/addon/submit/distribution). Release Firefox installs only add-ons Mozilla has signed, and listing on AMO is how they get signed.

## Files

From the repo root:

```
pnpm --filter @decay/extension zip:firefox
git archive --format=zip --prefix=decay-source/ --add-file=<BUILD-FIREFOX.md> \
  -o apps/extension/.output/decay-firefox-source-<version>.zip HEAD -- . \
  ':!Docs/Store/demo' ':!Docs/Store/assets' ':!Designs'
```

- **Add-on:** `apps/extension/.output/decayextension-<version>-firefox.zip` (569 KB for 0.1.0).
- **Source code:** `apps/extension/.output/decay-firefox-source-<version>.zip` (1.7 MB). AMO asks for it because the code is bundled and minified. WXT's own `-sources.zip` isn't enough: it leaves out the workspace packages (`packages/ui`, `packages/config`). This archive is the whole repo at that commit plus `BUILD-FIREFOX.md` (below), and rebuilding from it gives a byte-identical add-on (checked 25 Nov 2026).
- `web-ext lint`: 0 errors, 2 warnings (`innerHTML` in a bundled library chunk). Reviewers see these routinely.

`BUILD-FIREFOX.md` (put in the archive's root):

```
# Building Decay's Firefox add-on from source

This archive is the Decay monorepo at the commit the add-on was built from
(public: https://github.com/code08-ind/decay). The add-on is `apps/extension`;
it uses two workspace packages (`packages/ui`, `packages/config`).

Requirements: Node.js 22.20.0 (see `.nvmrc`) and pnpm 11.1.2
(`corepack enable` picks it up from `package.json`'s `packageManager`).
Works on macOS, Linux and Windows.

    pnpm install --frozen-lockfile
    pnpm --filter @decay/extension zip:firefox

The add-on is then `apps/extension/.output/decayextension-<version>-firefox.zip`
(unpacked: `apps/extension/.output/firefox-mv3/`). Build tool: WXT (Vite);
the code is TypeScript + React, bundled and minified, not obfuscated.
```

## Submission steps

1. **Where:** "On this site" (listed on AMO; signed and public after review).
2. **Upload** the add-on zip. Compatible with: Firefox for desktop only (not Android).
3. **Source code:** "Yes", upload the source archive.
4. **Describe:** the fields below.
5. **Review notes:** the "Notes for reviewers" below.

The add-on's id is fixed (`decay@decay-app`, in the manifest): never change it, or Firefox treats it as a different add-on and the desktop app's native messaging host (which allows only this id) stops working.

## Describe the add-on

**Name:** Decay

**Add-on URL:** `decay` (→ addons.mozilla.org/firefox/addon/decay), or `decay-tabs` if taken.

**Summary** (≤ 250 characters):

> Your tabs, gently put away. Decay archives the tabs you've stopped using into a searchable archive, and brings any of them back with one shortcut. Everything stays on your computer.

**Description:**

```
Decay retires the tabs you've stopped using, and brings any of them back in a second.

People keep 30, 50, 100 tabs open because closing one feels like losing it. Decay treats your tabs like a cache: what you stop using quietly ages out into a searchable archive, and Alt+Shift+D finds and reopens it exactly where it was.

HOW IT WORKS
• Ages tabs by active browsing time, not the clock. A weekend away doesn't make Monday's tabs "old".
• Starts in shadow mode: for the first 7 active days nothing is closed. Decay shows you what it would have archived, so you can check its judgement first.
• Then, in automatic mode, stale tabs are archived in small, quiet sweeps, with an Undo button after each one.
• Never touches pinned tabs, tabs playing audio, or sites you've marked "Never archive".
• With your permission, checks a page for unsaved typing, an upload in progress or a "leave site?" prompt before archiving it. Pages it can't check get twice as long.

SEARCH AND RESTORE
• Alt+Shift+D opens the search popup. Type a few letters of a title or site, press Enter, and the tab is back.
• Ctrl/Cmd+Enter restores the whole batch a tab was archived with, in its original order.
• The Sweeps page lists your recent sweeps (the last 20 can be undone).

WORKS WITH THE DECAY DESKTOP APP (OPTIONAL)
The free Decay app for macOS and Windows does the same for your app windows, across all your browsers. Install it and click "Connect" in the extension: the two talk directly on your computer, never over the network. https://decay.aryangarg.com

PRIVATE BY DESIGN
• Everything stays on your computer. No account, no server, no analytics, no network requests.
• The Insights page shows how your tab count and "regret rate" (tabs you reopened within a day) change over time.
```

**Categories:** Tabs; Productivity. (AMO allows up to two for Firefox.)

**Tags:** tabs, tab manager, productivity, archive, search.

**Support email:** support.decay@aryangarg.com

**Support website:** https://decay.aryangarg.com/docs/browsers

**Homepage:** https://decay.aryangarg.com

**License:** the extension is MIT or Apache-2.0 (open core, `LICENSE.md`). AMO takes one: pick **MIT License**.

**Privacy policy:** "Yes", paste the text of https://www.aryangarg.com/decay/privacy-policy (AMO wants the text itself, not a link; `Docs/Store/Privacy Policy.md` is the same text in Markdown).

**Data collection** (asked by AMO and declared in the manifest as `data_collection_permissions: { required: ["none"] }`): none. Decay transmits nothing off the device; the desktop app link is local native messaging.

**Icon:** `apps/extension/public/icon/128.png`. **Screenshots:** `Docs/Store/assets/screenshot-1-search.png` … `screenshot-5-review.png` (1280×800; captured in Chrome, which is fine for AMO).

## Notes for reviewers

```
No account or login is needed. Source: the uploaded archive (BUILD-FIREFOX.md has the two build commands; the rebuild is byte-identical). Public repo: https://github.com/code08-ind/decay

To try it:
1. Install and open a few tabs. Decay starts in shadow mode, which closes nothing; the popup (toolbar button or Alt+Shift+D) shows what it would archive.
2. To see a sweep immediately: Settings (from the popup) → set the mode to Automatic and pick the shortest preset, wait a few minutes without using some tabs, or use "Sweep now". Archived tabs appear in the popup's search and on the Sweeps page, with Undo.

Permissions:
- tabs, storage, alarms, idle, notifications: aging tabs by active time, archiving and restoring them, the Undo notification. Data stays in IndexedDB / extension storage on the device.
- scripting + optional host permissions (http/https): only after the user clicks "Allow checks"; used to check a page for unsaved typing, a "leave site?" handler or an upload in progress before archiving it. Typed text is never read into storage.
- nativeMessaging (optional): only when the user clicks "Connect" to link the Decay desktop app on the same computer.

No remote code, no eval, no network requests. The two web-ext lint warnings (innerHTML) are inside a bundled library chunk, not Decay's own code.
```

## After approval

- Note the listing URL (https://addons.mozilla.org/firefox/addon/<slug>/) and put it in the desktop app's onboarding (Firefox currently opens an AMO search) and on the site's browsers docs; release the app with that change.
- Each new version: bump `apps/extension/package.json`, rebuild both zips, upload under "Upload new version".
