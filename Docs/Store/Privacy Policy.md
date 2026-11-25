# Decay: Privacy Policy

> Published at https://www.aryangarg.com/decay/privacy-policy (source: `lib/content/decay-privacy.ts` in the aryangarg site repo). Keep both in sync.

**Effective:** 25 November 2026 · **Publisher:** Aryan Garg

This policy covers Decay: the desktop app for macOS and Windows, its browser extension (Chrome, Edge, Brave and Firefox), Decay Pro, and the website decay.aryangarg.com.

## The short version

Decay keeps what it records about your tabs and windows on your computer. It has no account, no analytics and no advertising. The browser extension makes no network requests of its own. The desktop app checks GitHub for updates once a day, and sends crash reports only if you turn them on. If you buy Decay Pro, Paddle handles the payment and your license key is emailed to you; the key is then checked on your computer and never sent anywhere.

## What Decay keeps on your device, and why

To decide which tabs have gone stale, and to let you search and restore them later, the browser extension keeps the following in its own storage (IndexedDB and the browser’s extension storage) on your device. The desktop app’s data is described further down.

| What | Why |
| --- | --- |
| Open and archived tabs: address (URL), page title, site, favicon address, tab group name and colour, pinned state, window and position | To show, search and reopen archived tabs exactly where they were. |
| Timing: when a tab was opened and last used, a log of when you used each tab (at most one entry per 10 minutes), and how long it has been unused while you were actively browsing | Decay ages tabs by active browsing time, not wall-clock time. |
| History of what Decay did: sweeps, archives, restores and undos | Undo, the Sweeps page and the regret check (tabs you reopened within a day). |
| Your rules (sites you never want archived, for example) and settings | So Decay behaves the way you asked. |
| Daily counts: open-tab and window counts sampled once an hour, number of archives, restores, undos and searches, search-to-restore time, active minutes | The Insights page and the optional beta metrics file. |

Nothing is deleted automatically yet: the archive and its history stay on your device until you remove Decay, and uninstalling deletes all of it. Decay doesn’t run in Incognito windows and records nothing from them.

## Checking pages for unsaved work (optional)

If you allow it, Decay checks a page for unsaved work before archiving it, so it never closes a half-written form or an upload. With that optional site access, small scripts on each page note which form fields you typed into, whether the page asks before closing (a “leave site?” prompt), and whether a file upload is in progress, and just before archiving a stale tab Decay runs a one-off check in it. It never records what you typed, and it counts uploads without reading their contents or destinations. Only the answer (yes / no / can’t tell) is kept, together with the page’s address and the time of the check, in session memory that is cleared when the tab is closed or the browser quits; the answer is re-checked after 10–30 minutes. You can allow all sites, pick sites, or turn the access off at any time on your browser’s extensions page (chrome://extensions, or Firefox’s Add-ons Manager); without it, Decay simply waits longer before archiving pages it can’t check.

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
| nativeMessaging (optional) | Only if you install the Decay desktop app and click “Connect”: the extension then sends your open tabs, its archive, rules and settings to the Decay app on the same computer, which takes over archiving. Nothing leaves your computer. |

## The Decay desktop app

The desktop app (macOS and Windows) archives your browsers’ tabs and your apps’ windows, and keeps everything on your computer.

**What it stores, and where.** The addresses, titles and sites of your tabs, the titles of app windows (and, for files and Finder / Explorer folders, their location on disk), when you used each one, and your rules and settings. It never reads the content of pages or documents. Everything is in one database on your computer, encrypted with a key kept in the macOS Keychain or Windows Credential Manager. Daily backups (the last 7) stay next to it.

**What it sends over the network.** Only this:

- Update checks: once a day (and when you click Check now), the app downloads a small file listing the latest version from GitHub (github.com/code08-ind/decay-releases), and the update itself when you choose to install it. The request carries nothing about you or your tabs; GitHub sees your IP address, as with any download. Updates are only installed if they’re signed by Decay.
- Crash reports, only if you turn them on (Settings → Privacy & data, off by default): when the app crashes, a report goes to Sentry, a crash-reporting service. Before it’s sent, Decay removes the computer’s name, any user information, file paths and addresses, and keeps only the error and where it happened in Decay’s code. Decay’s Sentry project is set not to store IP addresses.
- Nothing else: no account, no analytics, no advertising.

**Window access (Accessibility on macOS).** Only to see which app windows are open, minimise the ones you stopped using, and close Finder / Explorer folder windows (Pro). Decay doesn’t read what’s inside windows.

**Microphone and camera.** Decay checks whether some app is using the microphone or a camera, so it doesn’t tidy up during a call. It never records or reads audio or video.

**Batch names (Pro, off by default).** If you turn on Name archived batches on this computer, Decay names each group of archived tabs. On Macs with Apple Intelligence it uses Apple’s on-device model, which runs on your Mac (Decay gives it the titles and sites of that group only); elsewhere it uses the words the titles share. The names are stored with your archive and nothing is sent anywhere. Turning the setting off deletes every name.

**Diagnostics you share yourself.** Export diagnostics saves a file of logs and settings without addresses, titles or sites; nothing is sent unless you send that file yourself. Send feedback opens a GitHub issue form in your browser with Decay’s version and your operating system filled in.

**Deleting your data.** Uninstalling Decay keeps your archive unless you choose to delete it (Windows: Delete the application data in the uninstaller; macOS: delete ~/Library/Application Support/Decay). An “Erase all data” button is planned.

## Buying Decay Pro

Decay Pro is sold by Paddle.com, our reseller and merchant of record. Paddle collects your payment details, name, email address and country to process the order, invoices and tax, under its own privacy policy (paddle.com/legal/privacy). We never see your card details.

When an order completes, Paddle notifies Decay’s license service, a small program running on Cloudflare Workers. It asks Paddle for your name and email address, creates your license key (the key contains your email address, so it can be shown in the app), and emails it to you through Resend, an email delivery service. The service has no database and keeps nothing once the email is sent; Paddle keeps its record of your order, and Resend keeps a delivery log of the email.

Decay checks the key on your computer, without contacting any server. If you email support, I receive what you send and use it only to answer you.

## The website

decay.aryangarg.com is hosted on Vercel, which handles the requests like any web host. The site uses no cookies, analytics or trackers. The Buy page loads Paddle’s checkout, which Paddle runs under its own privacy policy. The activation page reads your license key from the link in your email inside your browser; the key isn’t sent to the website.

## Third parties

The extension uses no third-party services, SDKs, analytics, fonts or CDNs; the open-source libraries bundled inside it run locally and send nothing anywhere. The desktop app talks to GitHub (updates) and, only if you turn crash reports on, Sentry. Buying Pro involves Paddle (payment), Cloudflare (the license service) and Resend (the license email). The website is hosted on Vercel. None of them receives your tabs, windows or archive.

## Children

Decay is a productivity tool and isn’t directed at children under 13.

## Changes

If this policy changes, the updated version will be posted at this address with a new effective date. Any change that started sending data off your device would be stated here, and in the app or extension, before it shipped.

## Contact

Questions about this policy, your purchase or your data: support.decay@aryangarg.com.
