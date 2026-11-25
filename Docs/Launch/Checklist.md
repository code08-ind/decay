# Launch checklist

## Must be true before any public post

- [ ] **macOS signing and notarization** (Apple Developer account, Docs/Release.md step 4). **Deferred by the user (24 Nov 2026) to a future step.** Until then every new Mac user sees "Apple cannot check it for malicious software" and must open it through System Settings → Privacy & Security → Open Anyway (right-click → Open no longer works on macOS 15+): say so in the download page and the posts, and expect fewer Mac installs; launching Windows-first is an option.
- [ ] **Windows signing** (Trusted Signing or a certificate, Docs/Release.md step 6), or launch macOS-first and say Windows is coming. Unsigned, SmartScreen warns on every download.
- [ ] **The extension is public** on the Chrome Web Store (listed, not unlisted), and on Edge Add-ons if Edge users are targeted. Its built-in store ids go into `crates/decay-host/src/config.rs` (`BUILTIN_EXTENSION_IDS`), or the desktop app can't talk to it: release the app after that change.
- [ ] **Selling works end to end** (Docs/Release.md "Selling Decay Pro"): a test-mode purchase emails a key that activates. Price decided; Buy button and price in the app.
- [ ] **A real release** published through the Release workflow (stable channel), with release notes, and tried on a clean Mac and a clean Windows PC: install, onboarding, extension connects, a sweep, Undo, restore, update to a newer build.
- [ ] **The website** (P4.4) with downloads, the privacy policy, docs, and a contact address. `HELP_URL` in `apps/desktop/src-tauri/src/help.rs` points at its docs.
- [ ] **Hands-on checks** from Progress: Finder / Explorer folder windows close and reopen (Pro), a real call on Windows holds sweeps.
- [ ] **Numbers** in the posts are real beta numbers (or the sentences using them are dropped).
- [x] **Privacy policy covers the desktop app** (25 Nov 2026): the desktop app, buying Pro (Paddle, the Cloudflare license service, Resend) and the website are in the published policy (aryangarg site PR #45; `Docs/Store/Privacy Policy.md`). Before turning crash reports on in a release (`DECAY_SENTRY_DSN`), set Sentry's *Prevent Storing of IP Addresses*: the policy says so.
- [ ] **Support ready:** ~~a reply-to address on the license email you actually read~~ (done 25 Nov 2026: support.decay@aryangarg.com, forwarded by Cloudflare Email Routing), a pinned "known issues" note in `decay-releases`, the feedback link in the app works.
- [ ] **Assets:** 3–5 screenshots (light and dark, macOS and Windows), a 30–60 s demo video, a 1270×760 Product Hunt thumbnail and gallery images. Reuse `Docs/Store/assets/` where they still match the app.

## Launch day (suggested order)

1. Morning (US Pacific, Tuesday–Thursday): Product Hunt goes live at 00:01 PT. Post the maker comment at once.
2. A few hours later: the r/macapps post (check the subreddit's current self-promotion rules and flair first).
3. The build-in-public thread, linking both.
4. Stay around all day: answer every comment within the hour, write down every bug report in the issue tracker, ship a fix release if something is broken (installed copies update within a day).
5. Next day: a short thank-you follow-up with what you changed because of the feedback.

## After launch

- Watch crash reports (Sentry, opt-in only), the license email inbox and `decay-releases` issues daily for the first week.
- Collect the questions people asked: they're the docs site's FAQ.
