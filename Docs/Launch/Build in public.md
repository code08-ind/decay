# Build in public

A launch-day thread (X / Bluesky; for LinkedIn, join the posts into one). Attach the demo video to the first post and a screenshot to each of the others.

```
1/ I've shipped Decay: an app that quietly archives the browser tabs and windows you've stopped using, and brings any of them back with one shortcut.

macOS + Windows, Chrome / Edge / Brave / Arc. Free, with a one-time Pro.
[WEBSITE]
```

```
2/ Why: I had 60+ tabs open, always. Not because I needed them. Closing a tab felt like losing it.

So instead of asking me to close tabs, Decay treats them like a cache: unused ones age out into an archive you can search in a second.
```

```
3/ It ages tabs by *active* time, not the clock. A weekend off doesn't make Monday's tabs stale.

And it never touches what you're using: pinned tabs, audio, pages you're typing in, or anything while you're in a call.
```

```
4/ Trust was the hard part. So it starts in shadow mode: a week where it only shows you what it would have archived.

After that, sweeps are small and quiet, and each one has Undo.
```

```
5/ Private by design: everything stays on your computer in an encrypted database. No account, no analytics.

The core is open source, so you can check what it does with your tabs: [repo link]
```

```
6/ Built with Rust + Tauri, a Chromium extension, and a lot of tests (the engine runs every "never lose work" scenario on every commit, on macOS and Windows).

Try it, and tell me what it got wrong: [DOWNLOAD]
```

## Follow-ups (one every few days after launch)

- What I changed in the first week because of your feedback (with the release notes).
- [BETA NUMBERS] What beta testers' tab counts did.
- How Decay decides a tab is "stale" (active time, idle cut-off, the thresholds) with a diagram.
- Why the core is open source and the app source-available (the FSL, in plain words).
- How offline license keys work (Ed25519, no account), and why there's no device limit.
