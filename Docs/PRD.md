# Decay — Product Requirements Document (PRD)

| | |
| --- | --- |
| **Version** | 1.0 (MVP) |
| **Date** | 4 Nov 2026 |
| **Owner** | Aryan Garg |
| **Status** | Approved for build — Phase 0 (extension validation) |
| **Source** | [Decay Product Spec.md](./Decay%20Product%20Spec.md) |
| **Related** | [Software Design](./Software%20Design.md) · [DB Design](./DB%20Design.md) · [API Design](./API%20Design.md) · [UI Design Guidelines](./UI%20Design%20Guidelines.md) · [Technical Stack Document](./Technical%20Stack%20Document.md) · [Progress](./Progress.md) |

---

## 1. Overview

Decay is a **cross-platform desktop app (macOS + Windows)** that lives in the menu bar / system tray. It retires stale browser tabs and app windows automatically and keeps everything **searchable and restorable**. It treats open items like a cache: whatever you stop using ages out, and anything you need comes back with one hotkey.

It runs fully locally and needs no account.

### 1.1 Decisions locked for this PRD

| Decision | Choice |
| --- | --- |
| Platforms | macOS 14 Sonoma+ (Apple Silicon and Intel), Windows 11 (best-effort Windows 10 22H2) |
| Stack | Tauri 2 + shared Rust core + React/TypeScript UI, with native platform modules (see Technical Stack Document) |
| Browsers (MVP) | Chromium family through one MV3 extension: Chrome, Edge, Brave, Arc. Safari and Firefox are P2 |
| Delivery order | **Phase 0:** extension-only validation beta → **Phase 1+:** desktop app, only if the validation gates pass |
| Business model | **Open core:** the archive engine and extension are open source (MIT/Apache-2.0); the desktop app has a paid Pro tier (one-time license, $19–29) |
| Default thresholds | **12 / 36 / 60 active hours** (Stale / Hidden / Archived), with Gentle, Balanced and Aggressive presets |
| Default hotkey | `⌥⌘D` (macOS) / `Ctrl+Alt+D` (Windows), rebindable |
| Audience | Developers and knowledge workers first |
| Design tool | Google Stitch, accessed through the Stitch MCP |

---

## 2. Problem

People keep 30+ tabs and many windows open because closing them feels like losing something. The clutter then makes it harder to find what matters. Existing tools each cover one slice: they quit whole apps (MagicQuit, Quitter), hide windows (Hocus Focus) or auto-close Chrome tabs (WhyTab). None retires **tabs and individual windows together** while keeping a **restorable, searchable archive**, and none uses a safety model based on **active work time** instead of wall-clock time.

## 3. Goals and non-goals

### Goals
1. Reduce open tabs and windows without the user managing them.
2. Make every archived item findable in seconds.
3. **Never lose user work.** If Decay cannot prove an action is safe, it does nothing.
4. Earn enough trust that users leave automatic mode on.

### Non-goals (MVP)
- Window tiling, layouts, named workspaces or project switching.
- Cloud sync, accounts or team features.
- AI summaries, page content capture, screenshots or screen recording.
- Mac App Store or Microsoft Store distribution (the sandbox blocks cross-app Accessibility access; revisit later).

## 4. Users and personas

| Persona | Description | Key need |
| --- | --- | --- |
| **Dev Dana** (primary) | Full-stack developer with 3 projects, 60–120 tabs across 4 Chrome windows, plus IDE, terminal, Finder and docs windows | Clean up without fear; find "that GitHub issue from Tuesday" |
| **Researcher Ravi** | Analyst or PM who opens 40 research tabs per task | Batch cleanup at the end of the day; restore a whole research session |
| **Cautious Casey** | Has lost work to an auto-closing tool before | Shadow mode, a clear reason for every action, instant undo |

## 5. Core scenarios (user stories)

| # | Scenario | Story | Acceptance |
| --- | --- | --- | --- |
| S1 | Monday reset | As a user returning after a weekend, nothing has aged, because only active time counts | No item's `active_age` grows while the machine is asleep, locked or idle for more than 5 min |
| S2 | Quiet cleanup | After a busy day, 40 unused research tabs are archived and a morning summary says what moved | The sweep runs at a quiet moment; a summary notification is shown the next morning |
| S3 | Need it back | The user presses `⌥⌘D`, types "pricing" and restores last Tuesday's pricing page | The panel opens in < 150 ms; results appear in < 50 ms; Enter restores the tab |
| S4 | Restore a batch | The user reopens every item archived from one browser window, in the original order and tab groups | `⌘/Ctrl+Enter` restores the whole batch in one new window, with groups rebuilt |
| S5 | Bad sweep | Something still in use was archived; the user clicks Undo and adds a protection rule in one click | Undo restores the whole sweep; the "Always protect this domain/app" action creates a rule |

---

## 6. Item lifecycle

```
Active ──(12h active, unused)──► Stale ──(36h)──► Hidden* ──(60h)──► Archived ──(retention 90d)──► Purged
   ▲                               │                │                   │
   └──────── used / restored ──────┴────────────────┴───────────────────┘
* Tabs skip Hidden. Windows reach Archived (closed) only if their app is on the close allowlist (Pro, F11);
  otherwise they stop at Hidden (minimized).
```

- **Stale:** a visual hint only (a badge in the tray or panel). Nothing changes on screen.
- **Hidden:** the window is minimized to the Dock or taskbar. Never closed.
- **Archived:** a restore recipe is written to the DB **first**, then the item is closed.
- **Purged:** deleted after the retention period (90 days free; unlimited or configurable on Pro).

**Threshold presets** (active hours to Stale / Hidden / Archived):

| Preset | Stale | Hidden | Archived |
| --- | --- | --- | --- |
| Gentle | 24 | 60 | 96 |
| **Balanced (default)** | **12** | **36** | **60** |
| Aggressive | 8 | 24 | 40 |
| Custom | user | user | user |

---

## 7. Functional requirements

Priority: **P0** = MVP must-have, **P1** = MVP should-have, **P2** = post-MVP.
Tier: **F** = Free, **Pro** = paid.

### 7.1 Core engine (from the spec)

| ID | Requirement | Pri | Tier | Acceptance criteria |
| --- | --- | --- | --- | --- |
| F1 | Track last use of tabs and windows, counted in **active work time** | P0 | F | The clock ticks only while awake, unlocked and user-active (input within 5 min, or audio playing). Focus, typing, audible audio or a visible tab counts as use and resets age to 0 |
| F2 | **Shadow mode** for the first 7 days (calendar days with ≥ 1 h of activity) | P0 | F | Sweeps run as simulations; a "Would have archived" review screen lists items and reasons; nothing changes |
| F3 | Archive stale tabs in Chromium browsers | P0 | F | Stores URL, title, favicon, browser profile, window, index, tab group (name and color), pinned flag, timestamps; then closes the tab |
| F4 | Hide stale windows without closing them | P0 | F | macOS: minimize via Accessibility; Windows: `SW_MINIMIZE`. Restore un-minimizes the window and focuses it |
| F5 | Hotkey search across the archive by title, URL, app, domain and date | P0 | F | FTS5 prefix plus fuzzy search; filters `app:`, `site:`, `before:`, `after:`, `in:window`; < 50 ms at 50k items |
| F6 | Restore one item or a whole batch | P0 | F (single) / Pro (batch) | Enter restores one item; `⌘/Ctrl+Enter` restores the batch with original order and groups |
| F7 | Undo the last sweep | P0 | F | A toast with Undo appears for 30 s after a sweep; Sweep History allows undoing any of the last 20 sweeps |
| F8 | Protection rules for apps, domains, URL patterns, and pinned/playing items | P0 | F | Rule scopes: app, domain, URL glob, window title regex; actions: protect, never archive, archive sooner (custom threshold) |
| F9 | Exclude private/incognito windows and chosen domains from the archive | P0 | F | Incognito is never recorded (the extension doesn't request incognito access). Excluded domains are never stored, even in events |
| F10 | Daily summary of what moved | P1 | F | Local notification at a user-chosen time (default 9:00) plus a summary view |
| F11 | Close windows of allowlisted apps with reliable restore | P1 | **Pro** | Built-in allowlist: Finder/Explorer folder windows, Preview, TextEdit (saved docs only). Unsaved or unknown state → never closed |
| F12 | Pause during screen sharing, meetings or manual hold | P1 | F | Detects mic/camera in use or screen capture active; manual hold for 1 h / until tomorrow / indefinitely |
| F13 | Safari support | P2 | F | Separate Safari Web Extension in the macOS app bundle |
| F14 | Firefox support | P2 | F | WebExtension port with native messaging |
| F15 | Optional local AI labels for archive batches | P2 | Pro | On-device model only; off by default |

### 7.2 Professional desktop app features (added for the MVP)

| ID | Requirement | Pri | Tier | Acceptance criteria |
| --- | --- | --- | --- | --- |
| F16 | Menu bar (macOS) / system tray (Windows) icon with **stale-count badge** | P0 | F | The icon shows the count of stale items; the menu has Search, Hold 1 h, Pause/Resume, Run sweep now, Undo last sweep, Open Library, Settings, Quit |
| F17 | **Launch at login** | P0 | F | On by default after onboarding; toggle in Settings → General |
| F18 | **Auto-update** with signed updates | P0 | F | Checks daily; Stable/Beta channel; "Install on quit" or "Restart now"; release notes shown |
| F19 | **Light / Dark / System theme** | P0 | F | Follows the OS by default; manual override; respects accent colour where possible |
| F20 | **Export / import archive** | P1 | F | Export as JSON (full fidelity), HTML bookmarks (Netscape format) or CSV; import from JSON or bookmarks HTML |
| F21 | **Onboarding wizard** | P0 | F | Welcome → choose preset → install extension (with detection) → grant Accessibility (or tabs-only) → shadow mode explained → done |
| F22 | **Main window ("Library")** | P0 | F | Sidebar (Today, This week, All, By source, By app, Protected, Sweeps), list or grid, detail drawer |
| F23 | **Settings window** | P0 | F | Tabs: General, Aging, Protection, Browsers, Privacy & Data, Shortcuts, Notifications, License, Updates, Advanced, About |
| F24 | **Keyboard-first UX** | P0 | F | Every action reachable by keyboard; shortcut cheat sheet (`⌘/` or `Ctrl+/`); rebindable global hotkey with conflict detection |
| F25 | **Native notifications** | P1 | F | Daily summary, sweep done (optional), extension disconnected, permission revoked; respects Do Not Disturb / Focus Assist |
| F26 | **Single instance and deep links** | P0 | F | A second launch focuses the running instance; `decay://` scheme (`decay://search?q=`, `decay://restore/<id>`, `decay://license/activate?key=`) |
| F27 | **Licensing (Pro)** | P1 | Pro | Offline Ed25519-signed license key; activate, deactivate and view status; no account; 14-day Pro trial |
| F28 | **Data safety: encryption, backup and erase** | P0 | F | DB encrypted at rest (key in Keychain or Windows Credential Manager); automatic daily backup (7 kept); "Erase all data" in one step with confirmation |
| F29 | **Diagnostics** | P1 | F | Local rotating logs; "Export diagnostics" zip (redacted, no URLs by default); opt-in crash reports |
| F30 | **Accessibility (a11y)** | P0 | F | Full VoiceOver/Narrator labels, focus rings, WCAG 2.2 AA contrast, Reduce Motion and High Contrast support |
| F31 | **Localization-ready** | P1 | F | All strings externalized (i18n keys); English at launch; RTL-safe layouts |
| F32 | **Help and feedback** | P1 | F | Help menu → docs site, keyboard shortcuts, "Send feedback" (prefilled email or GitHub issue), What's New after updates |
| F33 | **Permission health** | P0 | F | Detects revoked Accessibility access or a disconnected extension; shows a banner with a fix-it button; affected sweeps stop safely |
| F34 | **Local metrics dashboard** | P1 | F | Shows on-device success metrics (regret rate, open-tab trend, items archived); "Share with beta team" exports an anonymised JSON |
| F35 | **Multi-profile and multi-browser** | P1 | F | Items are tagged by browser and profile; restore reopens in the same browser and profile |

### 7.3 Always protected (non-configurable)

- Pinned tabs and the **active tab in each window**.
- Tabs that are audible or have an upload, download, WebRTC call or `beforeunload` handler.
- Tabs with typed form input (where it can be detected; otherwise a 2× threshold applies).
- Windows with unsaved or **unknown** state.
- Everything during screen share, meeting or manual hold.
- Apps and domains on the user's protected list.

### 7.4 Free vs Pro

| Capability | Free | Pro |
| --- | --- | --- |
| Tab archiving, search, single restore, undo | ✅ | ✅ |
| Window hiding | ✅ | ✅ |
| Window closing (allowlisted apps) | — | ✅ |
| Batch restore | — | ✅ |
| Retention | 90 days | Unlimited or custom |
| Custom threshold rules per app/domain | 3 rules | Unlimited |
| Export / import | ✅ | ✅ |
| Local AI labels (P2) | — | ✅ |

---

## 8. Non-functional requirements

| Category | Requirement |
| --- | --- |
| **Safety** | 0 data-loss incidents. The archive record is committed **before** any close. Unknown state means no action. Undo is always available for the last 20 sweeps |
| **Performance** | Idle CPU < 1% (averaged over 1 min); RAM < 120 MB for the app plus WebView; search panel visible < 150 ms after the hotkey; search < 50 ms at 50k items; extension background service worker < 30 MB |
| **Startup** | Cold start to tray-ready < 1.5 s; no UI window at login |
| **Reliability** | Crash-free sessions > 99.5%; DB in WAL mode; corrupted DB → restore from latest backup automatically |
| **Privacy** | No telemetry by default; no page content or screenshots; incognito never recorded; 90-day default retention; one-step erase |
| **Security** | Code-signed and notarized (macOS Developer ID; Windows Authenticode, EV or Azure Trusted Signing); native messaging host restricted to the published extension ID; local IPC restricted to the current user; strict CSP in the WebView |
| **Accessibility** | WCAG 2.2 AA; fully keyboard-operable; screen-reader tested on VoiceOver and Narrator |
| **Compatibility** | macOS 14+, Windows 11 (Windows 10 22H2 best-effort); Chrome, Edge, Brave and Arc at current stable − 2 |
| **Size** | Installer < 20 MB (macOS universal DMG < 30 MB) |

## 9. Success metrics (on-device; shared by beta users only by choice)

| Metric | Target |
| --- | --- |
| Data-loss incidents | 0 |
| Regret rate (archived items restored within 24 h) | < 5% |
| Automatic mode still on after 14 days | > 60% of activated users |
| Median open tabs after 14 days vs week 1 | ≥ 50% lower |
| Search to restore time | < 5 s (median) |

**Phase 0 kill criteria:** stop or rethink if < 30% keep automatic mode on after 14 days, regret stays > 15%, or < 5 of 20–30 testers would pre-order.

## 10. Release plan

| Phase | Scope | Gate |
| --- | --- | --- |
| **0 — Validation (3–4 wks)** | Standalone MV3 extension: tracking, shadow mode, tab archiving, search popup, undo, rules. Data in IndexedDB | Kill criteria above |
| **1 — Desktop alpha (macOS)** | Tauri app, Rust core, native messaging bridge, search panel, Library, Settings, onboarding, tray; F1–F9, F16–F19, F21–F24, F26, F28, F30, F33 | Internal dogfooding for 2 weeks with 0 data loss |
| **2 — Windows hiding + polish** | macOS window tracking and hiding (F4), F10, F12, F20, F25, F29, F31, F32, F34, F35 | Beta with 20–30 users |
| **3 — Windows alpha** | Windows platform module, tray, installer, auto-update | Parity smoke tests pass |
| **4 — Pro and public launch** | F11, F27, batch restore, licensing, landing page, Product Hunt / r/macapps | Launch checklist |
| **5 — Post-MVP** | F13 Safari, F14 Firefox, F15 AI labels | — |

## 11. Risks

See spec §Risks. Additions caused by the stack choice:

| Risk | Mitigation |
| --- | --- |
| WebView (WKWebView/WebView2) differences between OSes | Shared design tokens; visual regression tests on both; avoid bleeding-edge CSS |
| Windows has no global "unsaved" signal | Windows closing stays off on Windows until a per-app allowlist is proven; hide-only |
| Panel must feel native and instant | Pre-warmed hidden panel window; Rust-side search; no network |

## 12. Open questions (remaining)

1. Final product name and trademark/domain check (working name "Decay").
2. Pro price point within $19–29, to be set from beta pre-order data.
3. Payment provider for license sales (Lemon Squeezy vs Paddle). Both act as merchant of record.
4. Whether Windows 10 is officially supported or best-effort only.
