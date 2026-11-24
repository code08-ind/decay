# Decay: Privacy Policy, desktop app section (DRAFT)

> **Draft, not published.** The published policy (`Privacy Policy.md`, on the personal site) covers only the browser extension. Before the desktop app is released publicly, merge this section into it (both copies: the site's `lib/content/decay-privacy.ts` and `Privacy Policy.md`) with a new effective date. Every statement below matches the app's code as of 24 Nov 2026; re-check them if the app changes. One depends on a setting, not the code: before publishing, confirm Sentry's *Prevent Storing of IP Addresses* is on (Docs/Release.md, step 5).

## The Decay desktop app

The desktop app (macOS and Windows) does the archiving the extension does, for your browsers' tabs and your apps' windows, and keeps everything on your computer.

**What it stores, and where.** The addresses, titles and sites of your tabs, the titles of app windows (and, for files and Finder / Explorer folders, their location on disk), when you used each one, and your rules and settings. It never reads the content of pages or documents. Everything is in one database on your computer, encrypted with a key kept in the macOS Keychain or Windows Credential Manager. Daily backups (the last 7) stay next to it.

**What it sends over the network.**

- **Update checks:** once a day (and when you click *Check now*), the app downloads a small file listing the latest version from GitHub (`github.com/code08-ind/decay-releases`), and the update itself when you choose to install it. The request carries nothing about you or your tabs; GitHub sees your IP address, as with any download. Updates are only installed if they're signed by Decay.
- **Crash reports, only if you turn them on** (Settings → Privacy & data, off by default): when the app crashes, a report goes to Sentry, a crash-reporting service. Before it's sent, Decay removes the computer's name, any user information, file paths and addresses, and keeps only the error and where it happened in Decay's code. Decay's Sentry project is set not to store IP addresses.
- **Nothing else.** No account, no analytics, no advertising.

**Decay Pro.** Your license key is checked on your computer; it's never sent anywhere. If you buy Pro, the purchase is handled by Paddle (the merchant of record) under its own privacy policy, and your license key is emailed to you; Decay itself receives your email address only as part of the key you paste in.

**Batch names (Pro, off by default).** If you turn on *Name archived batches on this computer*, Decay names each group of archived tabs. On Macs with Apple Intelligence it uses Apple's on-device model, which runs on your Mac (Decay sends it the titles and sites of that group only); elsewhere it uses the words the titles share. The names are stored with your archive and nothing is sent anywhere. Turning the setting off deletes every name.

**Window access (Accessibility on macOS).** Only to see which app windows are open, minimise the ones you stopped using, and close Finder / Explorer folder windows (Pro). Decay doesn't read what's inside windows.

**Microphone and camera.** Decay checks whether some app is using the microphone or a camera, so it doesn't tidy up during a call. It never records or reads audio or video.

**Diagnostics you share yourself.** *Export diagnostics* saves a file of logs and settings without addresses, titles or sites; nothing is sent unless you send that file yourself. *Send feedback* opens a GitHub issue form in your browser with Decay's version and your operating system filled in.

**Deleting your data.** Uninstalling Decay keeps your archive unless you choose to delete it (Windows: *Delete the application data* in the uninstaller; macOS: delete `~/Library/Application Support/Decay`). An "Erase all data" button is planned.
