# Licenses

Decay is **open core**: everything that reads, stores or moves your tabs and windows is
open source; the desktop app's shell and Decay Pro's licensing are source-available.

| Path | License |
| --- | --- |
| `crates/decay-core`, `crates/decay-store`, `crates/decay-bridge`, `crates/decay-host`, `crates/decay-platform`, `crates/decay-engine` | [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option |
| `apps/extension` (the browser extension), `apps/site` (the website), `packages/ui`, `packages/config` | [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option |
| `apps/desktop` (the desktop app), `crates/decay-license`, `services/license-webhook` | [Functional Source License 1.1, Apache-2.0 future license](apps/desktop/LICENSE.md) (`FSL-1.1-ALv2`) |
| Everything else (docs, designs, tools, workflows) | [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option |

**What FSL means here:** you may read, build, change and run the desktop app for any
purpose except offering it (or something built from it) as a competing commercial product
or service. Each version becomes Apache-2.0 two years after it's released. Decay Pro is a
paid feature of the app: building the app yourself is allowed, selling it is not.

The names "Decay" and the Decay logo aren't licensed under any of these: please don't use
them for your own builds or products.

Each package declares its license (`Cargo.toml` `license`, `package.json` `license`);
`tools/check-licenses.mjs` (run in CI) keeps them in line with this table.
