# r/macapps post

Check the subreddit's rules on the day (self-promotion, required flair, developer disclosure). Post as yourself, say you made it, and stay to answer.

**Title:**

```
I made Decay: it quietly archives the tabs (and windows) you've stopped using, and brings any of them back with one shortcut
```

**Body:**

```
Hi r/macapps! I'm the developer. I kept 60+ tabs open "just in case", because closing one felt like losing it. So I built Decay.

What it does
• Watches which tabs and windows you actually use, by active time (a weekend away doesn't make Monday's tabs "old").
• Tabs you've stopped using are archived in small, quiet sweeps, with Undo after each one. Idle app windows are minimised out of the way.
• ⌥⌘D opens a search panel over everything you've ever archived: type a few letters, press Return, and the tab is back in its window.
• Starts in shadow mode: for the first week nothing is closed, and it shows you what it would have archived, so you can judge it first.
• Never touches pinned tabs, tabs playing audio, pages where you're typing, or sites you mark "always keep". Sweeps wait while you're in a call or a meeting.

Private by design
• Everything stays on your Mac, in an encrypted database. No account, no analytics.
• The only thing it fetches is a daily update check. Crash reports are off unless you turn them on.
• The core (the engine, the storage, the browser extension) is open source: https://github.com/code08-ind/decay. The app itself is source-available.

Details
• macOS 14+ (Windows 10/11 too). Works with Chrome, Edge, Brave, Arc and Firefox through a small extension.
• Free: tab archiving, search, restore, Undo, window hiding, 90 days of archive.
• Decay Pro ($24, one-time, 14-day free trial, no account): closes stale Finder folder windows (they reopen from the archive), restores whole batches at once, keeps the archive longer than 90 days, unlimited custom timing rules, and names each archived batch on your Mac (Apple's on-device model where available).

Download: https://decay.aryangarg.com/download  ·  Chrome extension: https://chromewebstore.google.com/detail/jidljehgkacgjoefbancbihmdkkpcaaa  ·  Site: https://decay.aryangarg.com

Heads-up: Decay isn't notarized by Apple yet, so the first launch needs System Settings → Privacy & Security → Open Anyway (once).

I'd love feedback, especially on how it decides a tab is stale. Happy to answer anything.
```

## Likely questions, and honest answers

- **"Why not just bookmark / use OneTab / Workona?"** They save tabs when you tell them to. Decay decides on its own, slowly, by how you actually use each tab, and you can search everything it ever archived. Shadow mode lets you check it before it touches anything.
- **"Does it read my pages?"** No content is stored: the address, the title, the site and when you used it. With your permission it checks a page for unsaved typing before archiving it; nothing of what you typed is kept.
- **"Why does it need Accessibility access?"** Only to see and minimise app windows. Without it, Decay works with tabs only.
- **"Will it close a window with unsaved work?"** No. Windows are only minimised; Pro closes Finder folder windows (which have nothing to save) and reopens them on restore.
- **"Is it signed / notarized?"** Not yet: I'm a solo developer and haven't paid for the Apple Developer Program yet. The first launch needs System Settings → Privacy & Security → Open Anyway (once), and after an update macOS may ask for your login password so Decay can open its encrypted database (click Always Allow). The source of the core is public, and the builds are made by GitHub Actions from the public tags. *(Change this to "yes" once the Apple Developer account exists.)*
- **"Firefox?"** Yes: the add-on is on Firefox Add-ons. *(Until AMO approves it: "it's in review at Firefox Add-ons; Chrome, Edge, Brave and Arc work today".)*
- **"Subscription?"** No. Free forever for the basics; Pro is a one-time $24.
