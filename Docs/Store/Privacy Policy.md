# Decay: Privacy Policy

> Published at https://www.aryangarg.com/decay/privacy-policy (source: `lib/content/decay-privacy.ts` in the aryangarg site repo). Keep both in sync.

**Effective:** 7 November 2026 · **Publisher:** Aryan Garg

This policy covers Decay (beta), the browser extension that archives stale tabs.

## The short version

Decay runs entirely in your browser. It has no account, no server, no analytics and no advertising, and it makes no network requests of its own. What it records about your tabs stays on your device. Nothing is sent anywhere unless you choose to save a metrics file and send it to me yourself.

## What Decay keeps on your device, and why

To decide which tabs have gone stale, and to let you search and restore them later, Decay keeps the following in the extension’s own storage (IndexedDB and chrome.storage) on your device:

| What | Why |
| --- | --- |
| Open and archived tabs: address (URL), page title, site, favicon address, tab group name and colour, pinned state, window and position | To show, search and reopen archived tabs exactly where they were. |
| Timing: when a tab was opened and last used, a log of when you used each tab (at most one entry per 10 minutes), and how long it has been unused while you were actively browsing | Decay ages tabs by active browsing time, not wall-clock time. |
| History of what Decay did: sweeps, archives, restores and undos | Undo, the Sweeps page and the regret check (tabs you reopened within a day). |
| Your rules (sites you never want archived, for example) and settings | So Decay behaves the way you asked. |
| Daily counts: open-tab and window counts sampled once an hour, number of archives, restores, undos and searches, search-to-restore time, active minutes | The Insights page and the optional beta metrics file. |

Nothing is deleted automatically yet: the archive and its history stay on your device until you remove Decay, and uninstalling deletes all of it. Decay doesn’t run in Incognito windows and records nothing from them.

## Checking pages for unsaved work (optional)

If you allow it, Decay checks a page for unsaved work before archiving it, so it never closes a half-written form or an upload. With that optional site access, small scripts on each page note which form fields you typed into, whether the page asks before closing (a “leave site?” prompt), and whether a file upload is in progress, and just before archiving a stale tab Decay runs a one-off check in it. It never records what you typed, and it counts uploads without reading their contents or destinations. Only the answer (yes / no / can’t tell) is kept, together with the page’s address and the time of the check, in session memory that is cleared when the tab is closed or the browser quits; the answer is re-checked after 10–30 minutes. You can allow all sites, pick sites, or turn the access off at any time on chrome://extensions; without it, Decay simply waits longer before archiving pages it can’t check.

## The beta metrics file

Insights has a “Share with beta team” button. It shows you the complete file first, and only saves it to your computer when you click Save; you decide whether to send it. The file holds counts and medians only: the dates you browsed and for how many minutes, open-tab and window counts, numbers of archives, restores, undos and searches, search-to-restore times, shadow-mode counts, your Decay settings (mode, preset, thresholds, whether page checks are allowed on all, some or no sites, and the number of rules of each kind), how many days shadow and automatic mode have run, the extension version and browser name (Chrome, Edge or Brave), and a random install id created on your device so exports from the same tester can be matched. It contains no addresses, page titles, site names or rule contents. “Export CSV” saves the same daily counts to your computer.

## Beta feedback

If you join the beta and send feedback through the beta Google Form or by email, or attach the metrics file, I receive what you send. Google processes form responses and email under its own privacy policy. I use beta feedback and metrics files only to evaluate and improve Decay, never sell or share them, and delete them when the beta ends or sooner if you ask.

## Browser permissions

| Permission | Used for |
| --- | --- |
| tabs, tabGroups | See open tabs and their groups so stale ones can be archived and reopened in place. |
| storage | Keep the archive, rules, settings and counts on your device. |
| alarms, idle | Run the aging clock every 30 seconds and count only time when you’re actively browsing. |
| scripting + optional site access | The optional unsaved-work check described above: watchers for typing, “leave site?” prompts and uploads, plus a one-off check of a tab just before it is archived. |
| favicon | Show site icons from Chrome’s own cache, without downloading anything. |
| notifications | The “Undo” message after a sweep and the “always keep this site?” follow-up. |
| nativeMessaging (optional) | Only if you install the Decay desktop app and click “Connect”: the extension then sends your open tabs, its archive, rules and settings to the Decay app on the same computer, which takes over archiving. Nothing leaves your device; the app keeps its data in an encrypted database. |

## Third parties

None in the extension: no third-party services, SDKs, analytics, fonts or CDNs. Open-source libraries bundled inside it run locally and send nothing anywhere.

## Children

Decay is a productivity tool and isn’t directed at children under 13.

## Changes

If this policy changes, the updated version will be posted at this address with a new effective date. Any change that started sending data off your device would be stated here, and in the extension, before it shipped.

## Contact

Questions about this policy or your beta data: gargaryan82000@gmail.com.
