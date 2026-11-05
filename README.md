# Decay

*Your tabs, gently put away.*

Decay is a calm menu-bar app for macOS and Windows that quietly archives stale browser tabs and hides idle windows. Search and restore anything in a keystroke — local-first, nothing is ever lost.

> Status: Phase 0 (browser extension validation) has started. See [`Docs/Progress.md`](Docs/Progress.md).

## Repository layout

| Path | Contents |
| --- | --- |
| `Docs/` | PRD, software/DB/API design, UI guidelines, tech stack, progress tracker |
| `Designs/light`, `Designs/dark`, `Designs/windows` | Final UI designs (PNG + HTML per screen), generated in Google Stitch |
| `apps/extension` | WXT MV3 browser extension (Phase 0 standalone archive) |
| `packages/config` | Shared TypeScript config |
| `.mcp.json` | Stitch MCP config (reads `STITCH_API_KEY` from your environment) |

## Planned stack

Tauri 2 · Rust core · React + TypeScript · Chromium browser extension (WXT, MV3).

## Development

Requires Node 22 and pnpm 11.

```sh
pnpm install
pnpm --filter @decay/extension dev   # opens Chrome with the extension loaded
pnpm test                            # Vitest
pnpm lint                            # Biome
pnpm typecheck
pnpm build                           # → apps/extension/.output/chrome-mv3
```
