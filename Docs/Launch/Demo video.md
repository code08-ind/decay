# Launch demo video (45–60 s)

**Recorded automatically (28 Nov 2026):** `assets/decay-launch.mp4` (48 s, 1920×1080, 30 fps) and `assets/decay-launch-30s.mp4` (20 s cut: shots 1–2, 4, end card), from the **real desktop UI** with sample data: `apps/desktop/demo/` (recording-only; Tauri's `mockIPC` answers the UI's commands; never shipped) on a stage drawn in HTML (Mac menu bar, a Chrome-like window, the notification). "Sample data" shows in a corner. To re-record after UI changes: `cd apps/desktop && npx vite --port 1420`, then `node Docs/Launch/demo/record-launch.mjs <out dir>` (Playwright symlinked into `Docs/Launch/demo/node_modules`, like `Docs/Store/demo`; the first run after an edit can fail while Vite reloads: run it again). The plan below is the hand-recorded alternative.

For Product Hunt (first gallery item), the website and the launch posts. It shows the **desktop app**, so it's recorded by hand on a Mac; the store video (`Docs/Store/assets/decay-demo.mp4`, the extension alone, automated) stays as it is.

**Format:** 1920×1080 (or 2560×1440, scaled down), 30 fps, no voice-over: short on-screen captions instead (most people watch muted). MP4 (H.264), under 50 MB. For Product Hunt, upload it to YouTube (unlisted is fine) and paste the link.

## Before recording

- A Mac with **v0.1.2**, the extension connected, **automatic mode** on, and a few days of real use, so the Library has real archived items. Or use a separate browser profile with harmless tabs (docs, recipes, travel, GitHub, Figma, a YouTube video): **nothing private on screen** (no email, bank, personal chats, company names).
- Light appearance, a plain wallpaper, the Dock set to auto-hide (System Settings → Desktop & Dock), other notifications off (Focus → Do Not Disturb, with Decay allowed so its sweep notification still shows).
- Chrome with 15–25 tabs open in one window, so "too many tabs" is visible.
- Recording: **⌘ Shift 5** → Record selected portion (or the whole screen) → Options: show mouse clicks. Or CleanShot X / Screen Studio if you have them (smooth zooms look better).
- Do each shot as its own clip; joining them is easier than one perfect take. iMovie or CapCut is enough to cut and add captions.

## Shots

| # | Time | On screen | Caption |
| --- | --- | --- | --- |
| 1 | 0–4 s | Chrome with 20+ cramped tabs; scroll the tab strip a little. | **Too many tabs?** |
| 2 | 4–10 s | The Decay notification: "Archived N tabs" with **Undo**. Use **Run sweep now** in Decay's menu-bar menu to make one happen on cue. The tab strip gets shorter. | **Decay quietly archives the ones you stopped using** |
| 3 | 10–14 s | Click **Undo** on the notification: the tabs come back. | **Every sweep has Undo** |
| 4 | 14–24 s | Press **⌥⌘D** (the search panel), type 3–4 letters of an archived tab, press **Return**: the tab reopens in its window. Hold a second on the restored tab. | **Need one back? ⌥⌘D, type, Return** |
| 5 | 24–30 s | In the panel, select a result and press **⌘ Return**: the whole batch comes back in order (Pro). | **Or bring back the whole batch** |
| 6 | 30–38 s | The Library: today's archived items with the **Reason** column (why each one was archived); hover one. Then Insights: the open-tabs chart going down. | **It ages tabs by active time, not the clock** |
| 7 | 38–44 s | Settings → Rules: an "Always keep" site; then a pinned tab and a tab playing audio staying put during a sweep (optional). | **Never touches pinned tabs, audio, or sites you keep** |
| 8 | 44–50 s | Settings → Privacy ("Everything stays on this device, encrypted. No account, no tracking."), quickly. | **Local only. No account. No analytics.** |
| 9 | 50–56 s | End card: the leaf icon, "Decay", `decay.aryangarg.com`. | **Free for macOS and Windows · Pro $24 one-time** |

**Shorter cut (30 s) for X / Bluesky:** shots 1, 2, 4, 9.

## Captions

Same font as the site (serif headline, e.g. "Source Serif" or "Fraunces"), dark text on the cream background (#F7F3E8) or white text with a soft shadow, bottom-centre, about 2 s per caption, no more than eight words.

## Checklist before publishing

- [ ] Nothing private in any frame (tab titles, notifications, the menu bar).
- [ ] Every caption is true of v0.1.2 (Pro features labelled Pro).
- [ ] Under 60 s; readable muted; first frame interesting (it's the thumbnail).
- [ ] Saved as `Docs/Launch/assets/decay-launch.mp4` (git-ignored if large) and uploaded to YouTube for Product Hunt.
