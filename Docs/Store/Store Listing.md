# Chrome Web Store listing — Decay (beta, unlisted)

Everything to paste into the Chrome Web Store Developer Dashboard for P0.9. Visibility: **Unlisted** (only people with the link can install).

Package: `pnpm --filter @decay/extension zip` → `apps/extension/.output/decayextension-<version>-chrome.zip` (538 KB for 0.1.0; no source maps, no remote code).

---

## Store listing tab

**Name** (from the manifest): `Decay`

**Summary** (manifest `description`, ≤ 132 chars):

> Your tabs, gently put away. Archives stale tabs; search and restore anything.

**Category:** Productivity → Tools (or "Workflow & Planning").

**Language:** English.

**Detailed description:**

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
• The Sweeps page lists your recent sweeps (the last 20 can be undone), with Restore selected.

PRIVATE BY DESIGN
• Everything stays on your device. No account, no server, no analytics, no network requests.
• The Insights page shows how your tab count and "regret rate" (tabs you reopened within a day) change over time.
• Beta testers can save an anonymised metrics file (counts only, no addresses, titles or sites) and send it themselves. You see the whole file before it's saved.

This is a beta. Decay is in a 3–4 week test with a small group of testers. Feedback: gargaryan82000@gmail.com
Privacy policy: https://www.aryangarg.com/decay/privacy-policy
```

**Graphic assets** (`Docs/Store/assets/`):

| Asset | Size | File |
| --- | --- | --- |
| Store icon | 128×128 PNG | `apps/extension/public/icon/128.png` |
| Screenshots (1–5, in this order) | 1280×800 PNG | `screenshot-1-search.png`, `screenshot-2-popup.png`, `screenshot-3-sweeps.png`, `screenshot-4-insights.png`, `screenshot-5-review.png` (real UI with sample data) |
| Small promo tile | 440×280 PNG | `promo-small-440x280.png` |
| Demo video (48 s) | MP4, 1280×712 | `decay-demo.mp4`. Upload to YouTube as **Unlisted** and paste the link into the listing's "Video" field and the reviewer instructions below |

**Additional fields:**
- Official URL: none (or https://www.aryangarg.com once a Decay page exists)
- Homepage URL: https://www.aryangarg.com/decay/privacy-policy (until a landing page exists)
- Support URL / email: gargaryan82000@gmail.com
- Mature content: No

---

## Privacy practices tab

**Single purpose description:**

> Decay archives browser tabs that have gone unused for a while, and lets the user search and restore them. All of its features (aging, archiving, the unsaved-work check, search, restore, undo, and on-device usage statistics) serve that one purpose.

**Permission justifications:**

| Permission | Justification |
| --- | --- |
| `tabs` | Decay reads each open tab's URL, title and activity to measure how long it has gone unused, closes stale tabs after saving them to the on-device archive, and reopens them on restore. |
| `tabGroups` | Archived tabs remember their tab group (name and colour) so a restore puts them back in the same group. |
| `storage` | Stores the tab archive, the user's rules and settings, and daily usage counts locally (IndexedDB / chrome.storage). Nothing is synced or sent anywhere. |
| `alarms` | Runs the aging clock every 30 seconds in the MV3 service worker, which can't keep timers alive on its own. |
| `idle` | Counts only time when the user is actively at the computer, so tabs don't "age" overnight or while the user is away. |
| `scripting` | Only with the optional site access below: registers two small content scripts (watchers for typing, "leave site?" prompts and uploads in progress) and, just before archiving a stale tab, runs a one-off check in that tab that returns yes / no / unknown. |
| `favicon` | Shows site icons in the popup and pages from Chrome's local favicon cache, so Decay never downloads icons. |
| `notifications` | Shows "Archived N tabs" with Undo / View buttons after a sweep, and an "Always keep <site> open?" suggestion when a tab is restored soon after being archived. |
| `nativeMessaging` (**optional**, from the version with the desktop app link) | Requested only when the user clicks "Connect" to link the Decay desktop app. It lets the extension exchange tab information and its archive with that app on the same computer, through the native messaging host the app installs. Nothing is sent over the network. |
| Host permissions (`http://*/*`, `https://*/*`, **optional**) | Requested at runtime only when the user clicks "Allow checks", and can be limited to specific sites. Used solely to check pages for unsaved form input, `beforeunload` handlers or a file upload in progress before archiving. Typed text and upload contents are never read into storage; only a yes / no / unknown verdict with the tab's address and check time is kept in session storage, until the tab closes or the browser quits (re-checked after 10–30 minutes). |

**Remote code:** No, I am not using remote code. (All JavaScript ships in the package; no `eval`, no remotely loaded scripts.)

**Data usage** — what to tick:
- Tick **Web history** (tab URLs and titles) and **User activity** (when tabs were used). Decay only keeps this on the device, but the conservative answer is to declare what it handles; the privacy policy explains that nothing leaves the device. Consider also ticking **Website content** (page titles and whether a form holds typing).
- Certify all three:
  - ☑ I do not sell or transfer user data to third parties, outside of the approved use cases
  - ☑ I do not use or transfer user data for purposes that are unrelated to my item's single purpose
  - ☑ I do not use or transfer user data to determine creditworthiness or for lending purposes

**Privacy policy URL:** https://www.aryangarg.com/decay/privacy-policy

---

## Distribution tab

- Visibility: **Unlisted**
- Regions: all
- Pricing: free

## Test instructions (for the reviewer)

```
No account or login is needed.
1. Install and open a few tabs. Decay starts in shadow mode, which closes nothing.
2. Press Alt+Shift+D (or click the toolbar icon) to open the popup.
3. To skip the 7-day shadow period: in the popup click "Review what Decay would do" → "Turn on automatic mode early" → "Turn on".
4. Open the Sweeps page (the clock icon labelled "Sweep history" in the popup header). "Sweep now" archives tabs that are already stale. Note: tabs become stale only after 60 hours of active browsing without being used (120 hours for pages Decay can't check), so on a fresh install Sweep now archives nothing. This 48-second video shows the full flow in the real extension (Sweep now archives three tabs, search + Enter restores one, Ctrl+Enter restores the rest): <YouTube link>
5. Search for an archived tab in the popup and press Enter to restore it.
Optional site access ("Allow checks") is only used to check pages for unsaved typing, uploads and "leave site?" prompts before archiving.
```
