# Decay — UI Design Guidelines

| | |
| --- | --- |
| **Version** | 1.0 |
| **Date** | 4 Nov 2026 |
| **Design tool** | Google Stitch (via Stitch MCP) |
| **Related** | [PRD](./PRD.md) · [Technical Stack](./Technical%20Stack%20Document.md) |

---

## 1. Design direction

**Calm, minimal, native-feeling. Keyboard-first.** Think Raycast, Linear and macOS System Settings: quiet surfaces, dense but readable lists, one warm accent.

**Brand idea: "gentle ageing, never loss."** Items fade gracefully (opacity and warmth shift with age) instead of disappearing. The accent is a warm **ember/amber** — reassuring, not alarming. Avoid anything that reads as destruction (no trash-can imagery for archiving, no red for normal actions).

**Principles**
1. **Trust over cleverness.** Always show *why* (reason chips) and *how to undo*.
2. **Quiet by default.** No modal interruptions; toasts and the morning summary only.
3. **Instant.** The panel feels like Spotlight: zero chrome, results while typing.
4. **Native on each OS.** Shared components, platform-specific chrome (see §8).
5. **Accessible.** WCAG 2.2 AA minimum; fully keyboard operable.

## 2. Colour tokens

Define as CSS variables in `packages/ui/tokens.css`; Tailwind maps to them. **Never hard-code hex values in components.** Replace Stitch's raw colours with these tokens during conversion.

| Token | Light | Dark | Use |
| --- | --- | --- | --- |
| `--bg` | `#FAFAF9` | `#161514` | App background |
| `--surface` | `#FFFFFF` | `#1E1D1B` | Cards, panel |
| `--surface-2` | `#F4F3F1` | `#262522` | Sidebar, hover rows |
| `--surface-glass` | `rgba(255,255,255,0.72)` | `rgba(30,29,27,0.72)` | Panel with backdrop blur (vibrancy / Mica) |
| `--border` | `#E7E5E2` | `#33312D` | Dividers |
| `--text` | `#1C1917` | `#F5F5F4` | Primary text |
| `--text-muted` | `#6B6560` | `#A8A29E` | Secondary text |
| `--text-faint` | `#A29C96` | `#78716C` | Hints, timestamps |
| `--accent` | `#D9771F` (Ember 600) | `#F59E4B` (Ember 400) | Primary actions, focus, selection |
| `--accent-soft` | `#FDF1E6` | `#3A2716` | Selected rows, chips |
| `--accent-contrast` | `#FFFFFF` | `#1C1917` | Text on accent |
| `--success` | `#2F855A` | `#5FBF8A` | Restored, connected |
| `--warning` | `#B7791F` | `#E0B252` | Degraded, shadow mode |
| `--danger` | `#C53030` | `#F07272` | Erase data, errors only |
| `--info` | `#2B6CB0` | `#6CA6E8` | Info banners |
| `--focus-ring` | `0 0 0 3px color-mix(in srgb, var(--accent) 45%, transparent)` | same | Keyboard focus |

**Age scale** (stale-indicator dots and the timeline): `--age-0` fresh `#A3A3A3` → `--age-1` stale `#E0A458` → `--age-2` hidden `#D9771F` → `--age-3` archived `#8C5A2B`.

All text/background pairs must meet ≥ 4.5:1 (body) and ≥ 3:1 (large text, icons, focus rings). Support **High Contrast** (Windows) and **Increase Contrast** (macOS) by swapping to `--border: currentColor`-strength borders.

## 3. Typography

| Token | Size / line | Weight | Use |
| --- | --- | --- | --- |
| `display` | 28 / 34 | 600 | Onboarding headlines |
| `title-1` | 20 / 26 | 600 | Window titles, section headers |
| `title-2` | 16 / 22 | 600 | Card titles, group headers |
| `body` | 13 / 18 | 400 | Default (macOS-native density) |
| `body-strong` | 13 / 18 | 500 | Item titles |
| `caption` | 11 / 14 | 400 | Timestamps, URLs, chips |
| `mono` | 12 / 16 | 400 | URLs in detail view, shortcuts |
| `panel-input` | 20 / 28 | 400 | Search input in the panel |

**Font stack:** `system-ui, -apple-system, "SF Pro Text", "Segoe UI Variable Text", "Segoe UI", Inter, sans-serif`; mono `ui-monospace, "SF Mono", "Cascadia Code", Consolas, monospace`. Use the OS font on each platform (SF on macOS, Segoe UI Variable on Windows) — no web fonts are shipped. **In Stitch, design with Inter** as a stand-in.

Respect the OS text size (Dynamic Type isn't available in WebViews; scale the root `font-size` from the OS setting where it can be read).

## 4. Spacing, layout, shape and elevation

- **Grid:** 4 px base. Spacing scale: `2, 4, 8, 12, 16, 20, 24, 32, 40, 48`.
- **Radius:** `sm 6` (chips, inputs), `md 8` (buttons, rows), `lg 12` (cards, panel), `xl 16` (onboarding cards). On Windows 11 use `8` for windows (OS-rounded).
- **Elevation:** `shadow-1` rows on hover: none (colour only); `shadow-2` popovers: `0 4px 16px rgba(0,0,0,.08)`; `shadow-3` panel: `0 24px 64px rgba(0,0,0,.24)` plus a 1 px `--border` hairline.
- **Row height:** 36 px compact (default), 44 px comfortable (setting).
- **Window minimums:** Library 860×560; Settings 720×520 (fixed width); Panel 680 wide × up to 480 tall; Tray popover 340×420.

## 5. Iconography and imagery

- **Lucide** icons, 16 px in lists, 20 px in toolbars, 1.5 px stroke. Use favicons and app icons for items (rounded 4 px; fallback is a monogram tile).
- Key icons: archive → `archive` (not trash), hide → `eye-off`, restore → `rotate-ccw`, protect → `shield-check`, hold → `pause-circle`, shadow mode → `ghost`, sweep → `wind`, undo → `undo-2`.
- **Menu bar / tray icon:** a monochrome template glyph (an hourglass leaf: a leaf shape whose lower half is filled). macOS: template image 18×18 pt @1x/@2x. Windows: 16/20/24/32 px ICO with light and dark variants. The badge shows the stale count (hidden when 0; "99+").
- Illustrations (onboarding and empty states only): line-art, single accent colour, generous whitespace.

## 6. Motion

| Motion | Duration | Easing |
| --- | --- | --- |
| Panel appear | 120 ms (scale 0.98→1, opacity 0→1) | `cubic-bezier(.2,.8,.2,1)` |
| Panel dismiss | 80 ms | ease-in |
| Row selection | 0 ms (instant) | — |
| Toast in/out | 180 / 120 ms (slide 8 px) | standard |
| Item "ageing" fade on archive | 240 ms opacity to 0.4 then collapse | standard |
| Drawer | 200 ms slide | standard |

`prefers-reduced-motion` → all transitions become ≤ 1 ms opacity changes. Never animate list reordering while the user is typing.

## 7. Components

| Component | Spec |
| --- | --- |
| **Button** | Variants: `primary` (accent fill), `secondary` (surface-2 fill), `ghost`, `danger` (only for erase/delete). Heights 28 (sm) / 32 (md) / 40 (lg, onboarding). Always show the shortcut hint in a tooltip |
| **Search input** | Large (panel) and compact (library). Leading search icon, trailing filter chips, `Esc` clears then closes |
| **Result row** | `[favicon/app icon] Title (body-strong, 1 line ellipsis) · domain or path (caption, muted) · reason chip · relative time (right-aligned, faint)`. Selected row: `--accent-soft` background with a 2 px left accent bar. Hover reveals actions: Restore, Protect, Delete |
| **Group header** | Sticky; `Today · Chrome — "Research" window · 14 items`; a "Restore all ⌘↵" button on hover (Pro badge if free) |
| **Reason chip** | Caption, `--surface-2` background, icon + text: "Unused 61 active h", "Protected: playing audio", "Rule: github.com" |
| **Kbd** | Shortcut keycap: mono 11 px, 1 px border, radius 4, padding 1×5 |
| **Toast** | Bottom-centre of the Library or a native notification when no window is open. "Archived 14 tabs · **Undo** (⌘Z) · View". 30 s lifetime, pauses on hover |
| **Banner** | Inline, full-width, at the top of the Library/Settings: info, warning (shadow mode, degraded), danger (permission revoked). Has a fix-it button |
| **Segmented control** | Presets Gentle/Balanced/Aggressive/Custom |
| **Slider + number field** | Threshold editing (hours), with a live "≈ 3 working days" helper text |
| **Toggle / Checkbox / Radio** | Platform-tinted with the accent |
| **Sidebar** | 220 px, sections with icons; collapsible; counts right-aligned |
| **Detail drawer** | 360 px right drawer: large favicon, title, URL (mono, copy button), source window, group, timeline of events, restore-fidelity note ("Form input and scroll position aren't saved") |
| **Empty state** | Illustration + one line + one action. E.g. "Nothing archived yet. Decay is watching quietly in shadow mode." |
| **Pro badge** | Small accent-outline pill "PRO"; clicking opens the upgrade sheet |
| **Status dot** | 8 px: connected (success), degraded (warning), down (danger) |

**Component states (all interactive):** default, hover, pressed, focus-visible, disabled, loading, selected. The focus ring is always visible on keyboard focus.

## 8. Platform conventions

| | macOS | Windows |
| --- | --- | --- |
| Window chrome | Overlay/transparent titlebar with traffic lights at the top-left (inset 12 px); sidebar runs under the titlebar | Custom titlebar with caption buttons at the top-right (min/max/close, 46×32), **Mica** background |
| Panel | Floating `NSPanel` with vibrancy, centred at 1/3 from the top of the active screen | Acrylic floating window, same position |
| Tray | Menu bar extra (template icon) → native menu + "Open popover" | Notification-area icon → left click: popover; right click: native context menu |
| Shortcuts | `⌘` based; show glyphs `⌘ ⌥ ⇧ ⌃` | `Ctrl`/`Alt` text labels |
| Settings | "Settings…" `⌘,` in the app menu | "Settings" in the tray menu, `Ctrl+,` |
| Buttons order | Primary on the right | Primary on the left of Cancel (Windows convention) — handled by a `<DialogActions>` component |
| Notifications | `UNUserNotification` style | Toast notifications with action buttons |
| Fonts | SF Pro | Segoe UI Variable |

## 9. Keyboard map (defaults)

| Action | macOS | Windows |
| --- | --- | --- |
| Open search panel (global) | `⌥⌘D` | `Ctrl+Alt+D` |
| Restore selected | `↵` | `Enter` |
| Restore batch | `⌘↵` | `Ctrl+Enter` |
| Open in new window | `⌥↵` | `Alt+Enter` |
| Protect (always keep) | `⌘P` | `Ctrl+P` |
| Delete from archive | `⌘⌫` | `Ctrl+Del` |
| Undo last sweep | `⌘Z` (in app) | `Ctrl+Z` |
| Show detail | `⌘I` / `→` | `Ctrl+I` / `→` |
| Filter by app / site | `⌘1…9` / type `site:` | `Ctrl+1…9` |
| Library | `⌘L` (in panel) | `Ctrl+L` |
| Settings | `⌘,` | `Ctrl+,` |
| Shortcut cheat sheet | `⌘/` | `Ctrl+/` |
| Close panel | `Esc` | `Esc` |

## 10. Voice and copy

- Plain, calm, specific. "Archived 14 tabs you hadn't used in 3 working days." Not "Decayed!" or "Cleaned up your mess."
- Say **archived**, **hidden**, **restored**, **protected**. Avoid "deleted" and "killed" (except for real deletion).
- Always state restore limits honestly: "Decay saves the address, not the page. Unsaved form input won't come back."
- Time is shown as **active hours** with a human equivalent: "60 active hours (≈ 1.5 work weeks)".
- Sentence case everywhere. No exclamation marks except in onboarding success.

## 11. Accessibility checklist

- [ ] Every interactive element is reachable by Tab/arrow keys, with a visible focus ring.
- [ ] Lists use `role="listbox"`/`option` with `aria-activedescendant` (panel) or grid semantics (Library).
- [ ] Reason chips and status dots have text equivalents (never colour alone).
- [ ] Contrast meets AA in both themes and with High Contrast enabled.
- [ ] Toasts are announced via `aria-live="polite"`; errors via `assertive`.
- [ ] Tested with VoiceOver (macOS) and Narrator (Windows).
- [ ] Hit targets ≥ 24×24 px (WCAG 2.2 target size).

---

## 12. Screen inventory (to design in Stitch)

| # | Screen | Window | Priority |
| --- | --- | --- | --- |
| 1 | **Search panel** — empty, typing with grouped results, no results, filter chips, batch-restore hover | Panel | P0 |
| 2 | **Tray popover** — status (mode, clock state, stale count), Hold menu, quick search, last sweep with Undo | Tray | P0 |
| 3 | **Onboarding** (6 steps): Welcome · Choose preset · Connect browser (detection states) · Window access (Accessibility / tabs-only) · Shadow mode explainer · All set | Onboarding | P0 |
| 4 | **Shadow review** — "Here's what Decay would have archived": summary stats, list with reasons, protect-from-here, "Turn on automatic mode" CTA | Library | P0 |
| 5 | **Library** — sidebar (Today, This week, All, By source, By app, Live now, Protected, Sweeps), list grouped by day/source, detail drawer | Library | P0 |
| 6 | **Live now** — current tabs/windows with an age bar and protection reason (makes the engine transparent) | Library | P1 |
| 7 | **Sweep history** + sweep detail with Undo | Library | P0 |
| 8 | **Rules** — list, add-rule sheet (scope, match, action, test preview) | Settings | P0 |
| 9 | **Settings** — General, Aging, Protection, Browsers, Privacy & Data, Shortcuts, Notifications, License, Updates, Advanced, About | Settings | P0 |
| 10 | **Undo toast** + "Always protect this" follow-up | Overlay | P0 |
| 11 | **Daily summary** — notification + summary view | Library | P1 |
| 12 | **Health banners/empty states** — extension disconnected, Accessibility revoked, paused for meeting, nothing archived yet | Various | P0 |
| 13 | **Pro upgrade sheet** + license activation | Sheet | P1 |
| 14 | **Insights** — local metrics (regret rate, open-tab trend) | Library | P1 |
| 15 | **Extension popup** (Phase 0 standalone) — mini search, status, shadow review link | Extension | P0 (Phase 0) |
| 16 | **Update available / What's new** sheet | Sheet | P1 |

Design every screen in **light and dark**, at **macOS** chrome first; then produce Windows variants for 3, 5 and 9 (titlebar and button-order differences).

## 13. Stitch workflow

1. **Create a Stitch project** "Decay Desktop". Set the theme from §2–4 (Ember accent `#D9771F`, Inter, radius 8–12, light + dark).
2. **Generate screens** with the base prompt below plus each screen's prompt. Keep one screen per Stitch screen and name them `NN-screen-name-light|dark` (e.g. `01-search-panel-dark`).
3. **Iterate in Stitch** until approved; mark approved screens in [Progress.md](./Progress.md).
4. **Build:** Claude Code uses the **Stitch MCP** to list projects/screens, pull each screen's HTML/Tailwind and screenshot, convert it to React components in `packages/ui` (tokens replace raw values; Radix/cmdk replace raw markup), and visually diff the running app against the Stitch screenshot.
5. **Source of truth:** Stitch owns layout and visuals; this document owns tokens, behaviour, accessibility and copy. If they conflict, update one so they agree — never silently diverge.

**Base prompt (prefix every screen prompt):**
> Desktop app UI for "Decay", a macOS/Windows menu-bar utility that automatically archives stale browser tabs and windows and lets users search and restore them. Calm, minimal, native macOS feel similar to Raycast and Linear. Warm ember accent #D9771F, neutral warm greys, Inter font at 13 px body, 4 px spacing grid, 8–12 px radii, subtle hairline borders, no heavy shadows. Keyboard-first: show shortcut keycaps. Use Lucide-style line icons. Design for desktop at {width}×{height}, {light|dark} mode.

**Example screen prompts:**
- **01 Search panel:** "A floating Spotlight-style search panel 680 px wide with a frosted-glass background. Large search input with the query 'pricing'. Results grouped by day and source window, e.g. 'Last Tuesday · Chrome — Research window'. Each row: favicon, page title, domain in grey, a small reason chip like 'Unused 61 active h', and the relative time on the right. The first row is selected with a soft amber highlight and a left accent bar. Footer hint bar with keycaps: ↵ Restore, ⌘↵ Restore batch, ⌘P Protect, Esc Close."
- **05 Library:** "Main window 1100×720 with a transparent macOS titlebar and sidebar: Today, This week, All, By source, By app, Live now, Protected, Sweeps, with counts. Main list of archived tabs grouped by day with sticky headers and a 'Restore all' button. Right detail drawer showing a large favicon, title, monospace URL with a copy button, source window, tab group chip, and a vertical timeline (Opened, Last used, Stale, Archived). A top banner in amber: 'Shadow mode · 3 days left — nothing is being closed yet. Review'."
- **03 Onboarding — Connect browser:** "Onboarding step 3 of 6, centred card 640×560. Title 'Connect your browser'. A list of detected browsers (Chrome, Arc, Brave, Edge) with their icons and status: 'Connected ✓' in green, 'Install extension' button, 'Not installed' in grey. Footer: Back, Continue (primary amber), and a progress dot indicator."

## Implementation notes (from the build)

- Tokens live in `packages/ui/tokens.css` (Sage direction) and are mapped to Tailwind 4 utilities in each app; components never use raw hex.
- Light-theme feature card green is **#3D6F3A** (design used #4F8A4B) so white/cream text on it meets WCAG AA (≥ 4.5:1).
- Chrome extension pages default `body` to 75% font size; app CSS resets it to `1rem`.
- Fonts are bundled (Latin woff2 only); favicons come from Chrome's local `_favicon` cache — pages make no network requests.
