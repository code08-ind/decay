# Releasing Decay (desktop)

How a version gets from a tag to users' Macs (P1.13, PRD F18). Workflow:
`.github/workflows/release.yml`.

## How it works

- **Where:** release files live in the **public** repo `code08-ind/decay-releases`; this repo stays private. The app reads:
  - Stable: `https://github.com/code08-ind/decay-releases/releases/latest/download/latest.json`
  - Beta: `https://github.com/code08-ind/decay-releases/releases/download/beta/latest.json` (the `beta` release holds only this manifest; every beta **and** every stable release is copied there, so beta users never fall behind).
- **Two signatures:**
  1. **Update signature** (required): Ed25519 (minisign) over `Decay.app.tar.gz`. The public key is in `apps/desktop/src-tauri/tauri.conf.json` (`plugins.updater.pubkey`); the app refuses any update whose signature doesn't verify.
  2. **Code signing** (optional for now): Apple Developer ID + notarization. Without it, builds are signed ad hoc (see "Unsigned builds" below).
- **What the app does:** checks the channel's manifest 1 minute after start-up and then daily (release builds only), downloads in the background when "Download updates automatically" is on, and installs on **Restart to update** or at quit when **Install on quit** was chosen. Settings → Updates shows all of it.
- **The native host** (`decay-host`) is built by `apps/desktop/scripts/sidecar.mjs` (Tauri's `beforeBundleCommand`) and placed at `Decay.app/Contents/MacOS/decay-host`, signed with the same identity as the app.

## One-time setup

1. **Create the releases repo:** public `code08-ind/decay-releases`, with a README (it needs one commit so the `beta` tag can be created).
2. **Releases token:** a fine-grained personal access token with access to `code08-ind/decay-releases` only, permission *Contents: Read and write*. Save it as the secret `RELEASES_TOKEN` in `code08-ind/decay` (Settings → Secrets and variables → Actions).
3. **Update key:** generated on 14 Nov 2026 and kept outside the repo:
   - `~/.tauri/decay.key`: the private key. Save its contents as the secret `TAURI_SIGNING_PRIVATE_KEY`.
   - `~/.tauri/decay.key.password`: its password. Save it as `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.
   - Back both up somewhere safe (a password manager). **If the key is lost, installed copies can never be updated again**: they only accept updates signed with it.
4. **Later, with an Apple Developer account** ($99/year): create a *Developer ID Application* certificate, export it as `.p12`, and add:
   - `APPLE_CERTIFICATE`: `base64 -i cert.p12 | pbcopy`
   - `APPLE_CERTIFICATE_PASSWORD`: the export password
   - `APPLE_SIGNING_IDENTITY`: e.g. `Developer ID Application: Your Name (TEAMID)`
   - `APPLE_ID`, `APPLE_PASSWORD` (an app-specific password from account.apple.com), `APPLE_TEAM_ID`
   
   The workflow then signs with hardened runtime and notarizes automatically. Nothing else changes.
5. **Crash reports (optional):** create a Sentry project (platform: Rust) and save its DSN as the secret `DECAY_SENTRY_DSN`. It's compiled into release builds; users still have to turn crash reports on (Settings → Privacy & data → Diagnostics). Without it (or if it doesn't parse), the switch says the build can't send them. The client starts at launch either way but drops every event while the setting is off; after a user turns reports on, the first report may carry a count of events dropped before (a number only). In Sentry, also turn off IP address storage (Project → Settings → Security & Privacy → *Prevent Storing of IP Addresses*): the app sends no user data, but Sentry records the sender's IP unless told not to.

## Cutting a release

1. Set the version in `apps/desktop/src-tauri/tauri.conf.json` (and `apps/desktop/package.json`, `apps/desktop/src-tauri/Cargo.toml` to keep them in step). A beta uses a pre-release version, e.g. `0.3.0-beta.1`. If the release has something worth telling, add its **What's New** entry (exactly this version) at the top of `apps/desktop/src/help/releases.ts`: it shows once in the Library after the update. Without one, nothing is shown.
2. Commit, then tag exactly `v<version>` and push the tag: `git tag v0.2.0 && git push origin v0.2.0`.
3. The **Release** workflow builds a universal (Apple silicon + Intel) app and uploads a **draft** release to `decay-releases` with `Decay_<v>_universal.dmg`, `Decay.app.tar.gz`, its `.sig` and `latest.json`. **Replace the draft's text** ("RELEASE NOTES GO HERE") with the release notes: Publish copies it into `latest.json`, and that's what Settings → Updates shows. Publish refuses a draft whose text wasn't changed.
4. Try the draft's `.dmg` on a Mac.
5. **Publish:** Actions → Release → *Run workflow* with the tag. That writes the notes into `latest.json`, publishes the draft (a pre-release for beta tags, "Latest" for stable) and refreshes the beta manifest, unless the beta manifest already holds a newer version (e.g. a stable hotfix published after a newer beta): then it's left alone. From then on, installed copies find it within a day, or at once with *Check now*.

To withdraw a bad release: mark it as a draft again (stable users stop seeing it), and re-run *Publish* for the previous good tag with **force_beta** ticked to point the beta manifest back at it. Installed copies never downgrade; ship a fixed, higher version.

## Unsigned builds (until the Apple account exists)

- **First install:** macOS says the app "can't be opened because Apple cannot check it". Open it from Finder with right-click → Open, or System Settings → Privacy & Security → *Open Anyway*. Updates installed by the app itself aren't affected (they don't carry the download quarantine).
- **Keychain prompt after each update:** the database key is in the login Keychain, trusted for the app's code signature. An ad-hoc signature changes with every build, so after an update macOS asks whether Decay may use its Keychain item: choose *Always Allow*. Developer ID signing removes this (the signature's identity stays the same).
- **Window access (Accessibility) after each update:** macOS ties the permission to the code signature too, so an ad-hoc update most likely turns it off. Decay keeps working with tabs only; turn it back on in System Settings → Privacy & Security → Accessibility (remove the old Decay entry first if it's listed). Developer ID signing removes this too.

## Testing an update locally

Debug builds read a test manifest from `DECAY_UPDATE_URL` (ignored in release builds) and check only when `DECAY_UPDATE_CHECK=1` or on *Check now*. Build two versions with the key (`TAURI_SIGNING_PRIVATE_KEY`, `--config src-tauri/tauri.release.conf.json`), serve the newer one's `.tar.gz` and a `latest.json` over HTTPS (or allow plain HTTP with `plugins.updater.dangerousInsecureTransportProtocol` in a test-only config), and point the older one at it. Always set `DECAY_SUPPORT_DIR` to a scratch folder.
