# Decay — Progress Tracker

| | |
| --- | --- |
| **Last updated** | 5 Nov 2026 |
| **Legend (design)** | 🟨 = generated in Stitch, awaiting your approval |
| **Current phase** | Phase 0 — Extension validation (designs awaiting review) |
| **Capacity assumption** | ~15 h/week |
| **Legend** | ⬜ Not started · 🟨 In progress · ✅ Done · ⛔ Blocked · ⏭️ Skipped |

---

## Decisions log

| Date | Decision | Notes |
| --- | --- | --- |
| 2026-11-04 | Stack: **Tauri 2 + Rust core + React/TS**, native platform modules | Replaces the spec's Swift-only proposal, for macOS + Windows |
| 2026-11-04 | **Extension-first validation** before the desktop app | Kill criteria in the PRD §9 |
| 2026-11-04 | Business model: **open core + paid Pro** (one-time $19–29) | Engine + extension open source |
| 2026-11-04 | Default thresholds **12 / 36 / 60 active h**, with presets | More cautious than the spec |
| 2026-11-04 | Targets: macOS 14+ (arm64 + x64), Windows 11; Chrome/Edge/Brave/Arc | Safari/Firefox P2 |
| 2026-11-04 | Hotkey `⌥⌘D` / `Ctrl+Alt+D` | Rebindable |
| 2026-11-04 | Design: Google Stitch, calm minimal, ~~Ember accent~~ → **Sage direction** (Lably Dribbble ref + Color Hunt #FFFDF6 #FAF6E9 #DDEB9D #A0C878): Newsreader serif titles/numbers, DM Sans UI, top pill-tab nav, black pill CTAs, one deep-green #4F8A4B feature card | Supersedes the colour tokens and type in UI Design Guidelines §2–3 (update when screens are approved) |
| 2026-11-04 | Extras: launch at login, auto-update, tray badge, export/import, light/dark, plus a full professional feature set (F16–F35) | |
| — | Assumed: developer audience first; working name "Decay" | Revisit before launch |

## Open items

- [ ] Final name + trademark/domain check
- [ ] Pro price (from beta pre-order data)
- [ ] Payment provider: Lemon Squeezy vs Paddle
- [ ] Windows 10: official or best-effort
- [ ] Apple Developer account ($99/yr) and Windows signing (Azure Trusted Signing)
- [ ] Chrome Web Store developer account ($5)

---

## Workflow per sub-phase

Build → run a **code-review agent** and a **manual-QA agent** in parallel → share both reports → fix every issue (with regression tests) → only then commit and push → update this file.

## Build notes (read before implementing UI)

- **Source of truth for designs is `Designs/`** — `light/` (31), `dark/` (31), `windows/` (6); PNG + HTML per screen, final versions only. Stitch still holds older copies (API can't delete single screens); in Stitch, the "(v2)"/"(v3)" screens are current.
- **Ignore invented details** when converting designs to code: in `light/09g-settings-updates-light` the Beta helper text "memory compressor" and the sidebar "port 4829" label are not real features. Generally, drop any footer/status text, memory figures or browsers (e.g. Safari) not in the PRD.
- **Landing page numbers are samples** (78 → 34 open tabs, 2.1% regret rate, "0 bytes"). Replace with real beta data (Gate 0 metrics) before launch.
- **Stitch cleanup (manual):** delete older duplicates in the Stitch web app, keeping "(v2)"/"(v3)" versions.

## Phase −1 — Setup & design (week 0–1)

| # | Task | Status |
| --- | --- | --- |
| S1 | Product docs (PRD, Software Design, DB, API, UI, Stack, Progress) | ✅ |
| S2 | Configure the **Stitch MCP** in Claude Code (`.mcp.json` + `STITCH_API_KEY`) | ✅ |
| S3 | Create the Stitch project "Decay Desktop" and theme | ✅ project `6384390215445978880`; light + dark design systems |
| S4 | Design screen 15 (extension popup) — needed for Phase 0 | 🟨 light + dark; awaiting review |
| S5 | Design screens 1–5, 7–10, 12 (P0) light + dark | 🟨 all done in light + dark; awaiting review |
| S6 | Design screens 6, 11, 13, 14, 16 (P1) | 🟨 light + dark; awaiting review |
| S7 | Windows variants for screens 3, 5, 9 | 🟨 light + dark |
| S8 | Init git repo, monorepo scaffold (pnpm + Cargo workspaces, Turborepo, Biome, CI) | ✅ JS side: pnpm + Turborepo + Biome + GitHub Actions. Cargo workspace deferred to Phase 1 (Rust not installed yet) |

## Phase 0 — Extension validation (weeks 1–4) · **GATE**

| # | Task | Req | Status |
| --- | --- | --- | --- |
| P0.1 | WXT MV3 extension scaffold, Dexie DB, settings | — | ✅ `apps/extension`: Dexie schema v1 + default rules, typed settings + presets, 19 tests; code review + QA passed after fixes |
| P0.2 | Tab tracker + active-time clock (`chrome.idle`) | F1 | ⬜ |
| P0.3 | Policy engine (TS port: protections, rules, thresholds) | F8, F9 | ⬜ |
| P0.4 | Shadow mode + review page | F2 | ⬜ |
| P0.5 | Sweep executor (write-before-close) + undo | F3, F7 | ⬜ |
| P0.6 | Popup search (MiniSearch) + restore single/batch | F5, F6 | ⬜ |
| P0.7 | Form/`beforeunload` probe | §7.3 | ⬜ |
| P0.8 | Local metrics + "share with beta team" export | F34 | ⬜ |
| P0.9 | Publish unlisted to the Chrome Web Store; recruit 20–30 testers | — | ⬜ |
| P0.10 | 14-day measurement + 10 interviews + pre-order ask | — | ⬜ |
| **GATE 0** | Auto mode on after 14 days ≥ 30% · regret ≤ 15% · ≥ 5 pre-orders | — | ⬜ |

## Phase 1 — Desktop alpha, macOS (weeks 5–9)

| # | Task | Req | Status |
| --- | --- | --- | --- |
| P1.1 | `decay-core`: domain, lifecycle (proptest), policy, aging clock | F1, F8 | ⬜ |
| P1.2 | `decay-store`: SQLCipher, migrations V1, FTS5, backups | F5, F28 | ⬜ |
| P1.3 | `decay-bridge` + `decay-host` + native host manifests (Chrome/Edge/Brave/Arc) | F3 | ⬜ |
| P1.4 | Extension adapter mode + IndexedDB → app migration | F3 | ⬜ |
| P1.5 | `decay-engine`: scheduler, executor, crash recovery, undo | F6, F7 | ⬜ |
| P1.6 | macOS platform: idle, sleep/lock, focus observer, Accessibility permission | F1, F33 | ⬜ |
| P1.7 | Tauri shell: tray + badge, global hotkey, single instance, deep links, autostart | F16, F17, F24, F26 | ⬜ |
| P1.8 | UI: design tokens + `packages/ui` from Stitch | F19, F30 | ⬜ |
| P1.9 | UI: Search panel (NSPanel) | F5, F6 | ⬜ |
| P1.10 | UI: Onboarding | F21 | ⬜ |
| P1.11 | UI: Library, Shadow review, Sweep history | F2, F22 | ⬜ |
| P1.12 | UI: Settings + Rules | F8, F23 | ⬜ |
| P1.13 | Auto-update + signing + notarization pipeline | F18 | ⬜ |
| P1.14 | Safety test suite ("never lose work") green | NFR | ⬜ |
| **GATE 1** | 2 weeks of dogfooding, 0 data loss, perf budgets met | — | ⬜ |

## Phase 2 — Window hiding + polish (weeks 10–12)

| # | Task | Req | Status |
| --- | --- | --- | --- |
| P2.0 | **Spikes:** minimize UX; unsaved-state detection; NSPanel across Spaces | — | ⬜ |
| P2.1 | macOS window tracking via AX + CGWindowList | F1, F4 | ⬜ |
| P2.2 | Hide stale windows | F4 | ⬜ |
| P2.3 | Meeting/screen-share detection + manual hold | F12 | ⬜ |
| P2.4 | Daily summary + notifications | F10, F25 | ⬜ |
| P2.5 | Export/import (JSON, HTML bookmarks, CSV) | F20 | ⬜ |
| P2.6 | Diagnostics, logs, opt-in crash reports | F29 | ⬜ |
| P2.7 | Insights screen, i18n extraction, Help/What's New | F31, F32, F34 | ⬜ |
| P2.8 | Multi-profile / multi-browser polish | F35 | ⬜ |
| **GATE 2** | Beta with 20–30 users; regret < 5% target trend | — | ⬜ |

## Phase 3 — Windows alpha (weeks 13–14)

| # | Task | Req | Status |
| --- | --- | --- | --- |
| P3.1 | Windows platform module (windows-rs): idle, power, focus hooks, minimize | F1, F4 | ⬜ |
| P3.2 | Named pipe IPC + registry native host install | F3 | ⬜ |
| P3.3 | Tray, Mica titlebar, Windows UI variants | F16 | ⬜ |
| P3.4 | MSI/NSIS installer, Trusted Signing, updater | F18 | ⬜ |
| P3.5 | Meeting detection (CapabilityAccessManager) | F12 | ⬜ |
| **GATE 3** | Parity smoke tests pass on Windows 11 | — | ⬜ |

## Phase 4 — Pro + public launch (weeks 15–16)

| # | Task | Req | Status |
| --- | --- | --- | --- |
| P4.1 | Close allowlisted windows (Finder/Explorer folders first) | F11 | ⬜ |
| P4.2 | Licensing: Ed25519 keys, trial, activation UI, Pro gates | F27 | ⬜ |
| P4.3 | Purchase webhook → license email (serverless) | F27 | ⬜ |
| P4.4 | Landing page + docs site | — | ⬜ |
| P4.5 | Open-source the core repo | — | ⬜ |
| P4.6 | Launch: r/macapps, Product Hunt, build-in-public posts | — | ⬜ |

## Phase 5 — Post-MVP

- ⬜ F13 Safari Web Extension · ⬜ F14 Firefox · ⬜ F15 Local AI batch labels

---

## Stitch design status

Exports: `Designs/light`, `Designs/dark`, `Designs/windows` (PNG + HTML). Stitch project `6384390215445978880`.
🟨 = generated, awaiting your approval.

| # | Screen | Light | Dark | Windows (L/D) | Built |
| --- | --- | --- | --- | --- | --- |
| 0 | Overview (new) | 🟨 | 🟨 | — | ⬜ |
| 1 | Search panel | 🟨 | 🟨 | — | ⬜ |
| 2 | Tray popover (+ Windows right-click menu) | 🟨 | 🟨 | 🟨 | ⬜ |
| 3 | Onboarding (welcome, pace, connect, window access, shadow + done) | 🟨 | 🟨 | 🟨 connect step | ⬜ |
| 4 | Shadow review | 🟨 | 🟨 | — | ⬜ |
| 5 | Library | 🟨 | 🟨 | 🟨 | ⬜ |
| 6 | Live now | 🟨 | 🟨 | — | ⬜ |
| 7 | Sweep history | 🟨 | 🟨 | — | ⬜ |
| 8 | Rules + add-rule sheet | 🟨 | 🟨 | — | ⬜ |
| 9 | Settings: Aging, General, Protection, Browsers, Privacy & data, Shortcuts, Notifications, Updates, About | 🟨 | 🟨 | 🟨 Aging | ⬜ |
| 10 | Undo toast + follow-up | 🟨 | 🟨 | — | ⬜ |
| 11 | Daily summary | 🟨 | 🟨 | — | ⬜ |
| 12 | Health banners / empty states | 🟨 | 🟨 | — | ⬜ |
| 13 | Pro upgrade + license | 🟨 | 🟨 | — | ⬜ |
| 14 | Insights | 🟨 | 🟨 | — | ⬜ |
| 15 | Extension popup | 🟨 | 🟨 | — | ⬜ |
| 16 | Update / What's new | 🟨 | 🟨 | — | ⬜ |
| 17 | Landing page (website, new) | 🟨 | 🟨 | — | ⬜ |

All planned screens designed (68 total). Landing page numbers (78 → 34 tabs, 2.1% regret) are samples — replace with real beta data before launch.

## Changelog

- **2026-11-05** — P0.1 review + QA fixes: settings writes are all validated (unknown presets/modes, bad numbers, `undefined` values) and keep `preset`/`thresholds` in sync; reads return copies (shared defaults could be mutated); first-run setup runs on every service-worker start; archived items drop their tab id so reused ids find the live tab; domain rules are lowercased; DB Design §6–7 updated to match.

- **2026-11-05** — Monorepo scaffold (pnpm, Turborepo, Biome, CI) and extension P0.1: WXT + React popup placeholder, Dexie DB (items/events/sweeps/rules/settings/metrics), settings store with Gentle/Balanced/Aggressive presets. Notes: sweep items are embedded in `sweeps` records in Phase 0; indexed flags (`isShadow`) are stored as 0/1 because IndexedDB can't index booleans.

- **2026-11-04** — Fixed undo toast, states board, Live now (dark), Windows Settings; added Settings Browsers/Notifications/Updates/About and Windows tray (light + dark). 68 screens exported to `Designs/`.

- **2026-11-04** — Full design set: 29 screens × light/dark (58 total) incl. landing page and Windows variants; v1 screens superseded (delete manually in Stitch UI — API can't delete single screens).

- **2026-11-04** — Stitch connected. Visual direction changed to Sage (Lably reference). Generated 12 screens (light/dark/Windows) in project "Decay Desktop"; PNG + HTML exports in `Designs/`.

- **2026-11-04** — Spec reviewed; decisions captured; all design docs created in `Docs/`; Stitch MCP config added (awaiting API key).
