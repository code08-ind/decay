# Decay

*Your tabs, gently put away.*

Decay is a calm menu-bar app for macOS and Windows that quietly archives stale browser tabs and hides idle windows. Search and restore anything in a keystroke — local-first, nothing is ever lost.

> Status: design phase. No app code yet. See [`Docs/Progress.md`](Docs/Progress.md).

## Repository layout

| Path | Contents |
| --- | --- |
| `Docs/` | PRD, software/DB/API design, UI guidelines, tech stack, progress tracker |
| `Designs/light`, `Designs/dark`, `Designs/windows` | Final UI designs (PNG + HTML per screen), generated in Google Stitch |
| `.mcp.json` | Stitch MCP config (reads `STITCH_API_KEY` from your environment) |

## Planned stack

Tauri 2 · Rust core · React + TypeScript · Chromium browser extension (WXT, MV3).
