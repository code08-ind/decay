# Decay: Product Spec

Nov 4, 2026 · @Aryan Garg

## Summary

Decay is a Mac menu bar app that retires stale tabs and windows automatically and keeps everything searchable and restorable. It treats open items like a cache: what you stop using ages out, and what you need comes back with one hotkey.

The first release targets people who keep 30 or more tabs and many windows open because closing them feels risky. It runs locally and needs no account.

This is a first draft. Thresholds, prices, targets and timings are proposals to test, not measured facts. The decisions that need your input are listed in the final section.

## Problem and evidence

People keep tabs and windows open because closing them feels like losing something. The clutter then makes it harder to find what matters. One [r/macapps user](https://rl.klein.ruhr/r/macapps/comments/1uknpm8/megathread_the_app_pile_july_2026/ox5q53d/?context=3) described the real problem as remembering which tab, doc or message mattered, and why it was open.

Existing tools each solve one slice. I found none that retires tabs and individual windows together and keeps them restorable. That is a limited search result, not proof that no such product exists.

| Product | What it does | Where Decay differs |
| --- | --- | --- |
| [MagicQuit](https://www.producthunt.com/p/magicquit/magicquit) | Quits whole apps unused for 12 hours (customizable) | Works per window and tab, and keeps a restorable archive |
| [Quitter](https://maketecheasier.com/automatically-quit-hide-apps-mac) | Quits or hides chosen apps after inactivity | Same difference, plus no per-app setup needed |
| [Hocus Focus](https://maketecheasier.com/hide-inactive-windows-mac/) | Hides inactive windows on a timer | Adds archive, search and restore |
| [WhyTab](https://chromewebstore.google.com/detail/okaaefnoijehafkdedhpipdakhdjahdd) | Flags tabs unvisited for 3 days and can auto-close them after 7 (Chrome only; pinned and active tabs are spared) | Covers windows as well as tabs |

A second gap is the safety model. A timer alone is too blunt, because a weekend away would make everything look stale. Decay counts active work time and uses protection rules.

## Goals, non-goals and success metrics

The product wins if users trust it enough to leave automatic mode on. Trust matters more than the number of items archived.

**Goals**

- Reduce open tabs and windows without the user managing them.
- Make every archived item findable in seconds.
- Never lose user work.

**Non-goals for the first release**

- Window tiling or layouts.
- Named workspaces or project switching.
- Cloud sync, accounts or team features.
- AI summaries or screen recording.

**Proposed success metrics** (targets to confirm in beta)

| Metric | Proposed target | Why it matters |
| --- | --- | --- |
| Data-loss incidents | 0 | One incident can end trust in the product |
| Regret rate: archived items restored within 24 hours | Under 5% | Shows the rules are retiring the right things |
| Automatic mode still on after 14 days | Over 60% of activated users | The clearest measure of trust |
| Median open tabs after 14 days | At least 50% lower than in the first week | Confirms the core benefit |
| Search to restore time | Under 5 seconds | Confirms the archive is usable |

These metrics stay on the device. Beta users would share them by choice.

## Users and core scenarios

The primary user is a developer or knowledge worker juggling several projects on one Mac. Developers come first because they will accept the permissions and give sharp feedback.

1. **Monday reset.** The user returns after a weekend. Nothing has aged, because only active work time counts.
2. **Quiet cleanup.** After a busy day, 40 research tabs go unused. Decay archives them, and a morning note says what moved.
3. **Need it back.** The user remembers a pricing page from last Tuesday. They press the hotkey, type "pricing" and restore it.
4. **Restore a batch.** The user reopens all items archived from one browser window, in their original order.
5. **Bad sweep.** Decay archives something still in use. The user presses Undo and adds a protection rule in one click.

## Product behaviour

Decay moves items through visible stages, so nothing disappears without warning. Tabs skip the Hidden stage, and windows are archived only if their app is allowlisted.

&#91;embedded content: item lifecycle · 5 states, thresholds are starting guesses\]

A stale item can return at any point. Windows that cannot be restored safely stop at Hidden.

**How aging works**

- The clock runs only while the Mac is awake and the user is active.
- Focus, typing, audio or a visible browser tab counts as use.
- Sweeps run in batches at quiet moments, never in the middle of active typing.

**What is always protected**

- Pinned tabs and the active tab in each window.
- Tabs playing audio, or with an upload, download or call running.
- Windows with unsaved or unknown state.
- Apps and domains on the user's protected list.
- Everything during a screen share, meeting or manual hold.

**Archive panel**

A global hotkey opens a search panel over any app. Results are grouped by day and source window, and each shows why it was archived. Enter restores one item, and a modifier key restores its whole batch.

**First-run flow**

1. Install the app and the browser extension.
2. Grant Accessibility access, or continue with tabs only.
3. Run shadow mode for 7 days.
4. Review what would have been archived.
5. Turn on automatic mode.

## MVP requirements

The MVP ships tab archiving first and treats window closing as a limited, opt-in feature. Tabs are safe to restore, while windows are not.

| ID | Requirement | Priority |
| --- | --- | --- |
| F1 | Track last use of tabs and windows, counted in active work time | P0 |
| F2 | Shadow mode for the first 7 days: show what would be archived and change nothing | P0 |
| F3 | Archive stale tabs in Chromium browsers, storing URL, title, window, group and timestamps | P0 |
| F4 | Hide stale windows without closing them | P0 |
| F5 | Hotkey search across the archive by title, URL, app and date | P0 |
| F6 | Restore one item or a whole batch | P0 |
| F7 | Undo the last sweep | P0 |
| F8 | Protection rules for apps, domains, and pinned or playing items | P0 |
| F9 | Exclude private windows and chosen domains from the archive | P0 |
| F10 | Daily summary of what moved | P1 |
| F11 | Close windows in an allowlist of apps with reliable restore | P1 |
| F12 | Pause during screen sharing, meetings or a manual hold | P1 |
| F13 | Safari support | P2 |
| F14 | Firefox support | P2 |
| F15 | Optional local AI labels for archive batches | P2 |

## Technical design

Build a native Swift app with a local core and two thin adapters. The core owns all decisions, so a new browser or OS needs a new adapter and not a new product.

&#91;embedded content: system architecture · local core with two adapters\]

The sweep executor writes the archive record before it closes anything. A failed close then leaves a harmless record, while the reverse order could lose an item.

**Proposed stack**

| Layer | Choice | Reason |
| --- | --- | --- |
| App and panel | Swift, SwiftUI and AppKit | Native feel and direct API access |
| Storage | SQLite with full-text search | Fast, local and simple to back up |
| Browser | Manifest V3 extension | Reuses one codebase across Chromium browsers |
| Bridge | Chrome native messaging | Official route between extension and app |
| Windows | Accessibility API plus Core Graphics | Needed to observe and control other apps |

**Data model**

| Table | Key fields |
| --- | --- |
| items | id, kind, app, title, url or path, source window, state, first seen, last used, active age |
| events | item id, type, timestamp |
| sweeps | id, started at, reason, item count, undone at |
| rules | scope, match, action, enabled |

**Platform constraints**

- Window titles from Core Graphics [need Screen Recording permission](https://developer.apple.com/forums/thread/126860). Reading titles through Accessibility should avoid asking for that, which keeps the permission request smaller.
- Chrome starts the native host as a separate process over stdin and stdout, and [a single host message is limited to 1 MB](https://developer.chrome.com/docs/extensions/mv3/nativeMessaging?authuser=22). Large tab lists need chunking.
- Safari [routes native messages through the containing app's extension](https://developer.apple.com/documentation/safariservices/messaging-between-the-app-and-javascript-in-a-safari-web-extension.md), so it needs a separate integration. That is why Safari is P2.
- macOS hides whole apps rather than single windows, as far as I recall. Per-window hiding would therefore mean minimizing to the Dock, which needs a prototype to check whether it feels acceptable.
- Detecting unsaved state will vary by app. This is the riskiest technical unknown, so test it before committing to window closing.

**Restore support by item type**

| Type | Restored from | Close allowed |
| --- | --- | --- |
| Browser tab | URL, window and group | Yes |
| Finder window | Folder path | Yes |
| Document window | File path, where the app exposes it | Allowlist only |
| IDE or terminal window | Project or folder, if detectable | Not in MVP |
| Chat, design and other cloud apps | App only | No, hide only |

## Safety, privacy and edge cases

Decay must fail by doing nothing. If it cannot prove an action is safe, the item stays open.

**Restore limits.** An archive stores a recipe for reopening an item, not its live memory. Form input, scroll position, in-page state and unsaved text may be lost, and the product must say so plainly.

**Edge cases**

| Case | Behaviour |
| --- | --- |
| Tab has an active upload, download, call or audio | Protected |
| Tab has typed form input | Protected where the extension can detect it; tabs it cannot inspect wait for a longer threshold |
| Window has unsaved changes | Never closed |
| Unsaved state is unknown | Treated as unsaved |
| App exposes no reopenable document | Hidden only |
| Screen sharing or a meeting is active | No sweeps run |
| Browser extension is disconnected | Tab sweeps stop |
| Machine sleeps or goes idle | The aging clock pauses |
| Sign-in page or expiring link | Restored by URL; it may need a new sign-in |
| Archived page later changes or disappears | Decay shows the stored title and URL, with no page copy |

**Privacy**

- The archive is local, with no account and no telemetry by default.
- A browsing history is sensitive, so the database should be encrypted at rest.
- Private windows are never recorded.
- Default retention is 90 days, and users can erase all data in one step.
- The MVP stores no page content or screenshots.

## Distribution, pricing and Windows path

Ship outside the Mac App Store first. [Settle's maker](https://www.producthunt.com/products/settle-5) says the App Store sandbox blocks the cross-app Accessibility access window tools need. I expect the same limit here, so test it before planning any store release.

**Distribution:** a Developer ID signed and notarized download, plus the Chrome Web Store extension. Launch on r/macapps, Product Hunt and your own build-in-public blog posts.

**Pricing hypothesis:** free for tabs, with a one-time Pro license around $19 to $29 for windows, unlimited retention and batch restore. For comparison, [Workspaces is listed at $19.99](https://onmymenubar-app.pages.dev/workspace-plus/) and [Stealthly at $12.99](https://apps.apple.com/us/app/stealthly/id6744299820). Willingness to pay is untested.

**Windows path:** treat this as a second product build, not a port. The policy rules, archive schema and browser extension can carry over, while window tracking and control need new native code. Chromium [native messaging also works on Windows](https://developer.chrome.com/docs/extensions/mv3/nativeMessaging?authuser=4), so tab archiving is the natural first Windows release. I have not researched Windows competitors yet.

## Risks, kill criteria and validation

The biggest risk is that people say they want this but will not let software close their things. Test that before building the native app.

| Risk | Why it matters | Mitigation |
| --- | --- | --- |
| Users do not trust automation | Automatic mode is the whole product | Shadow mode, undo and clear reasons for each action |
| Low willingness to pay | Similar tools are free or cheap | Measure paid intent in the beta |
| Browsers absorb tab archiving | [Chrome and Safari are adding AI tab features](https://www.superchargebrowser.com/library/ai-tab-organizer-vs-tab-manager-chrome.md) | Differentiate on cross-app coverage and search |
| App behaviour varies | Accessibility support differs by app | Allowlist apps and hide by default |
| Permission friction | Accessibility access can scare users | Explain why, offer a tabs-only mode, and consider open-sourcing the core |
| OS updates break window control | Accessibility behaviour may change | Test each macOS beta and fail safe |

**Validation before the native build**

- [ ] Ship a Chrome extension with shadow mode and tab archiving
- [ ] Recruit 20 to 30 heavy tab users from r/macapps and developer communities
- [ ] Measure how many enable automatic mode and keep it on for 14 days
- [ ] Measure regret rate
- [ ] Interview 10 users about windows, and ask for a paid pre-order

**Proposed kill criteria:** stop or rethink if under 30% keep automatic mode on after 14 days, if regret stays above 15%, or if fewer than 5 of the 20 to 30 testers would pre-order.

## Milestones

The plan takes about 16 weeks at an assumed 15 hours a week, and it can stop at any gate. These are rough estimates, not commitments.

&#91;embedded content: roadmap · 16 weeks, equal bands, not to scale\]

The early gates limit the risk, because a failed test costs 3 weeks instead of 16.

## Open decisions

These choices are yours. Each has my recommendation so you can accept or overrule it quickly.

- [ ] **Audience:** developers first, or general users? I recommend developers.
- [ ] **Business model:** paid app, open source, or open core? I recommend open source for the archive engine and a paid app, because users are granting broad access.
- [ ] **Validation first:** will you run the 3-week extension test before native work? I strongly recommend yes.
- [ ] **Stack:** native Swift only, or a shared Rust core for Windows later? I recommend Swift until validation passes.
- [ ] **Minimum macOS:** which versions will you support? I suggest macOS 26 and 27, on Apple Silicon only.
- [ ] **Default aggressiveness:** the 8, 24 and 40 active-hour thresholds are guesses. Should the first beta be more cautious?
- [ ] **Capacity:** how many hours a week can you give this? The milestone estimates assume about 15.
- [ ] **Name:** Decay is a working name and may sound negative. Check trademarks and domains before launch.

## Sources

Pages opened or reviewed on 4 November 2026. Other links in this doc come from search results and should be checked before you rely on them publicly.

- [MagicQuit on Product Hunt](https://www.producthunt.com/p/magicquit/magicquit)
- [WhyTab on the Chrome Web Store](https://chromewebstore.google.com/detail/okaaefnoijehafkdedhpipdakhdjahdd)
- [Chrome native messaging documentation](https://developer.chrome.com/docs/extensions/mv3/nativeMessaging?authuser=22)
- [Apple: messaging in a Safari web extension](https://developer.apple.com/documentation/safariservices/messaging-between-the-app-and-javascript-in-a-safari-web-extension.md)
- [Apple Developer Forums: window names and Screen Recording](https://developer.apple.com/forums/thread/126860)
