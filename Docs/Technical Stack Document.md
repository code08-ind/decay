# Decay — Technical Stack Document

| | |
| --- | --- |
| **Version** | 1.0 |
| **Date** | 4 Nov 2026 |
| **Related** | [PRD](./PRD.md) · [Software Design](./Software%20Design.md) |

> **Change from the original spec:** the spec proposed native Swift (Mac only). Because Decay must ship on **macOS and Windows**, and the designs come from **Google Stitch** (which exports HTML/Tailwind), we use **Tauri 2 with a shared Rust core and a React UI**. OS-specific window control stays native through small Rust platform modules.

Pin exact versions in `Cargo.lock` and `pnpm-lock.yaml` when the project is scaffolded. The majors listed below are the targets.

---

## 1. Stack at a glance

| Layer | Choice | Why |
| --- | --- | --- |
| Desktop shell | **Tauri 2.x** | Small binaries (~10 MB), low RAM, system WebView, first-class tray, global shortcuts, updater, deep links, single-instance plugins; macOS + Windows from one codebase |
| Core language | **Rust (stable, 2021/2024 edition)** | Shared core for both OSes; memory-safe; direct FFI to Apple and Win32 APIs; reusable for the native messaging host |
| UI | **React 19 + TypeScript 5 + Vite** | Stitch exports HTML + Tailwind → straightforward conversion to React components |
| Styling | **Tailwind CSS 4** + CSS variables (design tokens) | Matches Stitch output; tokens drive light/dark themes |
| UI primitives | **Radix UI** (headless) + **cmdk** (command palette/search) | Accessible primitives; cmdk is the standard for Spotlight-style lists |
| Icons | **Lucide** | Clean, consistent, open source |
| UI state | **Zustand** (client state) + **TanStack Query** (core data via Tauri commands) | Minimal boilerplate; caching and invalidation on core events |
| Virtualized lists | **TanStack Virtual** | Library views with 50k+ items |
| Typed IPC | **tauri-specta** | Generates TS types from Rust commands and events — one source of truth |
| Storage | **SQLite 3 + FTS5** via **rusqlite** (`bundled-sqlcipher` feature) | Local, fast, full-text search; encrypted at rest |
| Migrations | **refinery** (or `rusqlite_migration`) | Versioned SQL migrations embedded in the binary |
| Secrets | **keyring** crate → macOS Keychain / Windows Credential Manager | Stores the DB encryption key and license |
| Async runtime | **tokio** | Scheduler, IPC server, timers |
| Serialization | **serde / serde_json** | Native messaging JSON, export/import |
| Logging | **tracing** + `tracing-appender` (rolling files) | Structured, redactable logs |
| Errors | **thiserror** (libs) + **anyhow** (binaries) | Idiomatic error handling |
| Browser extension | **Manifest V3**, TypeScript, **WXT** framework | One codebase for Chrome/Edge/Brave/Arc; Firefox later via WXT |
| Extension storage (Phase 0) | **IndexedDB via Dexie** | Standalone archive before the desktop app exists |
| Extension ↔ app | **Chrome native messaging** (stdio, 1 MB/message) → `decay-host` binary → local IPC | Official route; chunking for large tab lists |
| Local IPC | **interprocess** crate: Unix domain socket (macOS) / named pipe (Windows) | Per-user, no network ports |
| Monorepo | **pnpm workspaces** + **Cargo workspace** + **Turborepo** | Shared UI package for the app and extension |

## 2. Platform-specific native layer

| Concern | macOS | Windows |
| --- | --- | --- |
| Bindings | `objc2`, `objc2-app-kit`, `accessibility-sys` / `core-foundation`, `core-graphics` | `windows` crate (windows-rs) |
| Window enumeration | `CGWindowListCopyWindowInfo` (IDs, owner PID, bounds — **no titles**, to avoid needing Screen Recording) + `AXUIElement` for titles | `EnumWindows` + `GetWindowTextW`, `DwmGetWindowAttribute` (cloaked check) |
| Focus/use events | `NSWorkspace` notifications + `AXObserver` (`kAXFocusedWindowChangedNotification`) | `SetWinEventHook` (`EVENT_SYSTEM_FOREGROUND`, `EVENT_OBJECT_NAMECHANGE`) |
| Hide window | AX `kAXMinimizedAttribute = true` | `ShowWindow(SW_MINIMIZE)` |
| Close window | AX close-button press (allowlisted apps only) | `WM_CLOSE` (allowlisted only) |
| Document path | `kAXDocumentAttribute` | Per-app (Explorer via Shell `IShellWindows`) |
| Idle time | `CGEventSourceSecondsSinceLastEventType` | `GetLastInputInfo` |
| Sleep/lock | `NSWorkspace` will/did sleep, screens did sleep, session resign | `WM_POWERBROADCAST`, `WTSRegisterSessionNotification` |
| Mic/camera in use | CoreAudio `kAudioDevicePropertyDeviceIsRunningSomewhere`, CoreMediaIO | `CapabilityAccessManager\ConsentStore` registry (`LastUsedTimeStop == 0`) |
| Screen share | Known meeting apps + mic heuristic; `CGSessionCopyCurrentDictionary` sharing flag | Known meeting apps + mic heuristic |
| Permissions | Accessibility (`AXIsProcessTrustedWithOptions`) | None required (same-integrity windows) |
| As built (P1.6) | `crates/decay-platform`: direct `extern "C"` declarations for CoreGraphics / ApplicationServices (incl. `GetFrontProcess` for the active app) / CoreAudio / libproc plus `core-foundation` for CF types (instead of `objc2`: no Objective-C calls are needed yet). Focus and sleep/lock are **polled about once a second** instead of the `NSWorkspace` notifications / `AXObserver` above (those need a run loop on the main thread; the window-level observer comes with P2.1); sleep is a gap on `CLOCK_MONOTONIC`. The workspace forbids `unsafe`; this crate denies it and allows it only in `mac.rs`, each call with a `SAFETY` comment | `NullPlatform` until the Windows implementation |
| Tray | `NSStatusItem` (via Tauri tray) | Notification area (via Tauri tray) |

## 3. Tauri plugins

| Plugin | Purpose |
| --- | --- |
| `tauri-plugin-global-shortcut` | `⌥⌘D` / `Ctrl+Alt+D` panel hotkey |
| `tauri-plugin-autostart` | Launch at login |
| `tauri-plugin-updater` | Signed auto-updates (Stable/Beta channels) |
| `tauri-plugin-single-instance` | Focus the running instance; forward deep links |
| `tauri-plugin-deep-link` | `decay://` scheme |
| `tauri-plugin-notification` | Daily summary and alerts |
| `tauri-plugin-window-state` | Remember Library/Settings window size and position |
| `tauri-plugin-dialog` / `-fs` | Export/import file pickers (scoped) |
| `tauri-plugin-opener` | Open URLs and help links |
| `tauri-plugin-log` | Bridges frontend logs into `tracing` — **as built (P2.6):** not used; a `log_ui_error` command does this for errors |
| `tauri-nspanel` (community, macOS) | Non-activating floating panel for Spotlight-style search |

## 4. Tooling and quality

| Area | Tool |
| --- | --- |
| Rust lint/format | `clippy` (deny warnings in CI), `rustfmt` |
| TS lint/format | **Biome** (lint + format) |
| Rust tests | `cargo nextest`, `insta` (snapshots), `proptest` (lifecycle state machine) |
| TS tests | **Vitest** + Testing Library |
| E2E (app) | **WebdriverIO + tauri-driver** (Windows), Playwright against the dev server for UI flows |
| E2E (extension) | **Playwright** with a persistent Chromium context loading the unpacked extension |
| Visual regression | Playwright screenshots vs Stitch reference exports |
| Accessibility | `axe-core` in Vitest/Playwright |
| CI | **GitHub Actions**: matrix `macos-latest` (arm64), `windows-latest` (macos-14/13 images are retired; x64 macOS builds cross-compile) |
| Release | `tauri-action` → GitHub Releases; `latest.json` updater manifest |
| Commit hygiene | Conventional Commits, `lefthook` pre-commit |

## 5. Distribution and signing

| | macOS | Windows |
| --- | --- | --- |
| Format | Universal `.dmg` (+ `.app.tar.gz` for the updater) | `.msi` (WiX) + NSIS `.exe` |
| Signing | Apple **Developer ID Application** + **notarization** (`notarytool`) + hardened runtime | **Azure Trusted Signing** (or EV cert) — avoids SmartScreen warnings |
| Updater signature | Tauri Ed25519 updater key (separate from code signing) | Same |
| Native host install | Manifest in `~/Library/Application Support/Google/Chrome/NativeMessagingHosts/` (+ Edge, Brave, Arc paths) | Registry `HKCU\Software\Google\Chrome\NativeMessagingHosts\com.decay.host` (+ Edge, Brave) |
| Extension | Chrome Web Store (works for Brave/Arc) + Microsoft Edge Add-ons | Same |

## 6. Services (minimal, no user accounts)

| Service | Choice | Notes |
| --- | --- | --- |
| Updates | GitHub Releases (static `latest.json`) | No server |
| License sales | Lemon Squeezy or Paddle (merchant of record) | Webhook → serverless function signs an Ed25519 license key and emails it |
| License verification | **Offline** in the app (public key embedded) | No phone-home |
| Crash reports (opt-in) | Sentry (Rust + JS SDKs) with PII scrubbing | Off by default. **As built (P2.6):** the Rust SDK only (panics, scrubbed, DSN compiled in from `DECAY_SENTRY_DSN`); the windows' JavaScript errors go to the log through `log_ui_error` instead of a JS SDK |
| Landing/docs | Astro static site on Cloudflare Pages | — |

## 7. Design tooling

| Tool | Use |
| --- | --- |
| **Google Stitch** | Generate and iterate screens from the prompts in [UI Design Guidelines](./UI%20Design%20Guidelines.md) |
| **Stitch MCP** (`https://stitch.googleapis.com/mcp`) | Lets Claude Code list projects, fetch screens and code, and compare the build against designs |
| Conversion | Stitch HTML/Tailwind → React components in `packages/ui`, with tokens replacing raw colours |

## 8. Why not the alternatives

| Option | Rejected because |
| --- | --- |
| Native Swift only | No Windows; Stitch output can't be reused directly |
| Electron | 150 MB+ RAM — a poor fit for an app about reducing clutter |
| Rust core + SwiftUI + WinUI | Best fidelity but two UI codebases for a solo builder at ~15 h/week |
| Flutter | Weak fit for Accessibility/Win32 interop; no Stitch path |
