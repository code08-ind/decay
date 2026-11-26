# Microsoft Edge Add-ons listing: Decay (optional)

Edge users can already install Decay from the Chrome Web Store (Edge allows it), so this listing is optional: it makes Decay findable inside Edge's own store. Written 26 Nov 2026. Dashboard: https://partner.microsoft.com/dashboard/microsoftedge (a free Microsoft Partner Center developer account; registration can take a few days).

**Do it with the one final upload** (`Docs/Progress.md`): the same package goes to Chrome, Firefox and Edge at the same time.

## Important: the app has to learn the Edge ID

Edge Add-ons gives the extension its **own ID**, different from the Chrome Web Store's `jidljehgkacgjoefbancbihmdkkpcaaa`. The desktop app only talks to extension IDs it knows, so after the Edge item is created:

1. Partner Center → Decay → **Extension overview**: copy the **Extension ID** (32 letters a–p; it's shown once the package is uploaded, before review finishes).
2. Add it to `BUILTIN_EXTENSION_IDS` in `crates/decay-host/src/config.rs` (and to its test).
3. Optionally point Edge's install button at the Edge listing: `store_url` of the Microsoft Edge entry in `apps/desktop/src-tauri/src/onboarding.rs` → `https://microsoftedge.microsoft.com/addons/detail/<edge id>`.
4. Release the app. Until that release, people who install from Edge Add-ons can't connect to the app (the extension still works on its own).

## What to upload

| What | File | Notes |
| --- | --- | --- |
| Package | `apps/extension/.output/decayextension-<version>-chrome.zip` | The **Chrome** zip, unchanged (Edge takes Chromium MV3 packages). Same version as on the Chrome Web Store. |
| Extension store logo | `Docs/Store/assets/edge-logo-300.png` | 300×300, from `apps/desktop/assets/app-icon.svg`. |
| Small promotional tile | `Docs/Store/assets/promo-small-440x280.png` | 440×280. |
| Large promotional tile (optional) | `Docs/Store/assets/promo-marquee-1400x560.png` | 1400×560. |
| Screenshots | `Docs/Store/assets/screenshot-1-search.png` … `screenshot-5-insights.png` | 1280×800, in order. They're taken in Chrome, which is fine: the extension looks the same in Edge. |
| YouTube video (optional) | — | Leave empty. |

## 1. Packages

**Extensions → Create new extension** → drag in the Chrome zip → **Continue**.

## 2. Availability

- Visibility: **Public**
- Markets: **all markets**

## 3. Properties

| Field | Value |
| --- | --- |
| Category | **Productivity** |
| Does your extension access, collect or transmit personal information? | **Yes** (it keeps tab addresses and titles on the device; the policy explains nothing leaves it) |
| Privacy policy URL | `https://www.aryangarg.com/decay/privacy-policy` |
| Website URL | `https://decay.aryangarg.com` |
| Support contact details | `support.decay@aryangarg.com` |
| Mature content | **No** |

## 4. Store listings (English)

**Description** (Edge needs at least 250 characters):

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
The free Decay app for macOS and Windows does the same for your app windows, across Edge, Chrome, Brave, Arc and Firefox. Install it and click "Connect" in the popup: the extension and the app talk directly on your computer, never over the network. The extension works fully on its own without it. https://decay.aryangarg.com

PRIVATE BY DESIGN
• Everything stays on your computer. No account, no server, no analytics, no ads.
• The extension makes no network requests of its own.
• Open source: https://github.com/code08-ind/decay

Questions or bugs: support.decay@aryangarg.com
```

**Short description** (if asked; the manifest's): `Your tabs, gently put away. Archives stale tabs; search and restore anything.`

**Search terms** (up to 7, at most 21 words in all):

```
tab manager
tab archive
too many tabs
tab search
tab suspender
productivity
restore tabs
```

Then the images from the table above.

## 5. Submit

**Notes for certification:**

```
No account or login is needed. The extension works on its own; the "Connect" button in the popup is for an optional desktop app and can be ignored.

1. Install it and open a few tabs. Decay starts in shadow mode, which closes nothing.
2. Click the toolbar icon (or press Alt+Shift+D) to open the popup.
3. To skip the 7-day shadow period: open "Shadow review" (from the popup), click "Turn on automatic mode early", then confirm.
4. Tabs become stale only after 60 hours of active browsing without being used (120 hours for pages Decay isn't allowed to check), so on a fresh install "Sweep now" on the Sweeps page has nothing to archive yet. The screenshots show the flow with aged tabs: a sweep archives the stale tabs, the popup search finds an archived tab, Enter restores it, Undo reopens the whole sweep.
5. Optional site access ("Allow checks") is used only to check pages for unsaved typing, uploads and "leave site?" prompts before archiving; typed text is never stored.
6. The optional "nativeMessaging" permission is requested only when the user clicks "Connect", to talk to the Decay desktop app on the same computer.

No remote code, no network requests of its own. The same package is published on the Chrome Web Store (jidljehgkacgjoefbancbihmdkkpcaaa). Privacy policy: https://www.aryangarg.com/decay/privacy-policy. Source: https://github.com/code08-ind/decay (apps/extension).
```

**Publish.** Edge review usually takes up to 7 business days.

## Each new version

Partner Center → Decay → **Packages** → **Replace** with the new Chrome zip (same version as the Chrome Web Store) → **Publish**.
