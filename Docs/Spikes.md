# Spikes (P2.0)

Findings and decisions for the risks in Software Design §10, before window tracking
(P2.1) and hiding (P2.2) are built. Probe programs are in `tools/spikes/`.

Run on macOS 27 (Darwin 27.0), 14 Nov 2026. The development terminal has no
Accessibility access, so cross-app behaviour was checked on Decay's own test window
(`ax-own-window.swift`: Accessibility answers for one's own process) and with the
public `CGWindowList` API. `ax-other-apps.swift` repeats the reads for every running
app once Accessibility is granted; run it before P2.2 ships (see "Open checks").

## 1. Hiding a window = minimising it (spike 1)

**Finding.** Setting `AXMinimized = true` on a window minimises it to the Dock, and
reading it back returns `1` (the window's `isMiniaturized` is `true`). Windows list
`AXMinimizeButton` (with `AXEnabled`) and `AXFullScreen`.

**Decision.** "Hidden" = minimised to the Dock, through Accessibility:
- Only windows with an **enabled minimise button**, `AXSubrole = AXStandardWindow`,
  not full screen and not modal / sheets / panels are hideable. Anything else is never
  hidden, so never closed (Software Design §4.3: when in doubt, keep).
- Hiding happens only in a sweep at a quiet moment (as tab archiving), at most a few
  windows at a time, so the minimise animation doesn't play out under the user's hands.
- Restore = `AXMinimized = false`, `AXRaise`, then activate the app.
- macOS's own "Minimise windows into application icon" setting is respected (that's
  where minimised windows go); Decay doesn't change it.
- Why not alternatives: hiding the whole app (`NSRunningApplication.hide`) hides every
  window of it, not just the stale one; moving windows off-screen or to another Space
  needs private APIs and confuses Mission Control.

## 2. Unsaved changes and the document (spike 2)

**Findings.**
- `AXDocument` on the window is the document's file URL
  (`file:///…/decay-spike-report.txt` for a window with `representedURL`).
- The "edited" state isn't a window attribute (`AXEdited` / `AXModified` on the window
  → `kAXErrorAttributeUnsupported`, -25205). It's **`AXEdited` on the window's close
  button** (`AXCloseButton`): `1` while `isDocumentEdited`, `0` after saving.
- These are AppKit's standard document windows. Apps that aren't document-based, and
  many non-AppKit apps (Electron, Qt, Java), don't expose `AXDocument` or the edited
  flag.

**Decision.**
- `save_state`: close button `AXEdited` = 1 → `unsaved`; = 0 **and** an `AXDocument`
  is present → `saved`; anything else (no close button, attribute missing, an error, no
  document) → `unknown`. The policy already treats `unknown` like `unsaved`: hidden at
  most, never closed (`unsaved_or_unknown_documents_are_never_closed`).
- Closing (Pro, allowlisted apps, F11) re-reads both right before closing, as tabs are
  re-probed.
- Reopening a closed window = `open -a <app> <AXDocument path>` (Software Design §4.5),
  so a window without a document is never closable at all.

## 3. Finding windows: Accessibility + `CGWindowList`

**Findings.** Without Screen Recording permission, `CGWindowListCopyWindowInfo`
returns every window (all Spaces, minimised ones too) with number, owner pid and name,
bounds, layer and on-screen flag, but **no titles** (`kCGWindowName` absent for all 26
normal windows). `CGPreflightScreenCaptureAccess()` was `false`.

**Decision.**
- Decay never asks for Screen Recording (it isn't needed and would worry users).
- Titles, documents and edited state come from Accessibility (`AXWindows` per app).
- `CGWindowList` (layer 0) is the cheap change detector and gives windows on other
  Spaces; a window Accessibility can't see (another Space, for apps that only list the
  current Space) is tracked for age but **not hideable** until it's visible again.
- AX window ↔ CG window: `_AXUIElementGetWindow` (private but long stable, used by
  window managers outside the App Store; Decay isn't sandboxed), falling back to owner
  pid + frame. A window that can't be matched is not hideable.

## 4. Search panel across Spaces and full-screen apps (spike 3)

Built in P1.9 as a non-activating `NSPanel` (tauri-nspanel) that joins all Spaces and
full-screen apps. Its behaviour can only be judged on screen (screen capture is blocked
here), so it stays on the manual checklist: open ⌥⌘D over a full-screen app and on a
second Space.

## Open checks (need a Mac with Accessibility granted)

- Run `tools/spikes/ax-other-apps.swift`: confirm `AXDocument` / close-button
  `AXEdited` in TextEdit, Preview, Pages, Xcode; note what VS Code / Slack / Figma
  report (expected: no document → never closable).
- Minimise / restore a few real windows (including one on another Space) with Decay's
  hide once P2.2 exists.
- The search panel over full-screen apps (above).
