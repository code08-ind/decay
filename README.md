# Decay

*Your tabs, gently put away.*

Decay is a calm menu-bar app for macOS and Windows that quietly archives stale browser tabs and hides idle windows. Search and restore anything in a keystroke — local-first, nothing is ever lost.

> Status: Phase 4 (Pro and public launch). See [`Docs/Progress.md`](Docs/Progress.md).

## Open core: what's open and why

Everything that reads, stores or moves your tabs and windows is open source (MIT or
Apache-2.0): the Rust core and engine, the storage, the native messaging host, the
platform layer and the browser extension. You can check exactly what Decay does with
your data.

The desktop app around it, Decay Pro's licensing and the purchase webhook are
**source-available** under the [Functional Source License](apps/desktop/LICENSE.md): read
it, build it, change it for yourself; just don't sell it or offer it as a competing
product. Each version becomes Apache-2.0 two years after release. Buying Decay Pro is how
the project is paid for.

Details per folder: [`LICENSE.md`](LICENSE.md).

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
