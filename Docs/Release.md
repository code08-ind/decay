# Releasing Decay (desktop)

How a version gets from a tag to users' Macs and Windows PCs (P1.13, P3.4, PRD F18).
Workflow: `.github/workflows/release.yml`.

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

6. **Windows signing (optional, Azure Trusted Signing):** without it the installer is unsigned and SmartScreen shows "Windows protected your PC" (More info → Run anyway) until the download builds reputation. Trusted Signing needs an Azure subscription and a validated identity (Microsoft has limited individual developers to some countries: check eligibility first; an OV/EV certificate is the alternative). Once you have an account and a certificate profile:
   - an app registration (service principal) with the *Trusted Signing Certificate Profile Signer* role on the account; secrets `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`, `AZURE_TENANT_ID`;
   - repository **variables** (not secrets) `TRUSTED_SIGNING_ENDPOINT` (e.g. `https://eus.codesigning.azure.net`), `TRUSTED_SIGNING_ACCOUNT`, `TRUSTED_SIGNING_PROFILE`.

   The Windows job then signs `decay-host.exe`, `Decay.exe`, the installer and the uninstaller. Nothing else changes.

## Licenses (Decay Pro, P4.2)

- **Signing key:** generated on 24 Nov 2026 (replaced the same day: the first one was printed into a development session's log before any key was issued) at `~/.decay/license-signing.key` (32 private bytes as hex, mode 0600); copies are in the login Keychain ("Decay license signing key") and on the T Shield drive beside the repository (`Decay App Secrets/`, never inside it: the repository is public). Its public half is compiled into the app (`crates/decay-license/src/public_key.in`). **Back the file up** (a password manager): without it no new key can be issued that existing installations accept, and replacing it means a new public key in a new release, after which old keys stop working.
- **Issuing by hand** (support, testers): `cargo run -p decay-license --bin decay-license-tool -- issue ~/.decay/license-signing.key someone@example.com [license-id]`; `… -- verify <key>` checks one. P4.3's purchase webhook issues keys the same way (`decay_license::issue`).
- **Activation link:** `decay://license/activate?key=<key>` opens Settings → Decay Pro with the key filled in (nothing is activated without a click).

## Website (P4.4)

https://decay.aryangarg.com is `apps/site`, a static Astro site. One-time setup (your accounts):

1. **Vercel:** New Project → import `code08-ind/decay` → **Root Directory** `apps/site` (Vercel reads `apps/site/vercel.json`: install from the repo root with pnpm, build `pnpm run build`, output `dist`, clean URLs). Deploy.
2. **Domain:** in the project's Settings → Domains add `decay.aryangarg.com`; Vercel shows the record to create.
3. **Cloudflare DNS** (aryangarg.com's zone): add a `CNAME` named `decay` with the target Vercel gives (usually `cname.vercel-dns.com`), **DNS only** (grey cloud), so Vercel can issue the certificate.

Every push to `main` then redeploys it. Local preview: `pnpm --filter @decay/site dev`.

**As set up (24 Nov 2026):** Vercel project `decay-site` (team *code08ind's projects*), linked to `code08-ind/decay`, root `apps/site`, Node 22, production branch `main`; `decay.aryangarg.com` attached (verified). DNS: `CNAME decay → 6c74f33a4eafd734.vercel-dns-017.com`, DNS only. The `*.vercel.app` addresses are behind Vercel Authentication (the project default); the custom domain is public. Note `decay-site.vercel.app` is someone else's site: this project's is `decay-site-green.vercel.app`.

## Selling Decay Pro (P4.3)

Decay Pro ($24, one-time) is sold through **Paddle** (merchant of record: payment, tax, VAT, refunds). Lemon Squeezy, the first choice, closed to new stores in 2026, and Stripe Managed Payments doesn't accept India-based sellers. Keys are issued by `services/license-webhook`, a Cloudflare Worker that Paddle notifies on every completed transaction. One-time setup:

1. **Paddle (sandbox first):** sign up at sandbox-vendors.paddle.com (test) and vendors.paddle.com (live; Paddle reviews the seller and the website: decay.aryangarg.com has pricing, [Terms](https://decay.aryangarg.com/terms) naming Paddle as merchant of record, a [Refund Policy](https://decay.aryangarg.com/refunds) and the privacy page). Create the product **Decay Pro** with a one-time price of **$24 USD**; note its **price id** (`pri_…`). Create a **hosted checkout** (Checkout → Hosted checkouts) for that price: its link is the Buy button. Create an **API key** with permission to read customers.
2. **Resend:** the sending domain `aryangarg.com` (aryangarg.com's contact form may already use it), and an API key with sending access only.
3. **Worker:** in `services/license-webhook/wrangler.toml` set `PADDLE_PRICE_IDS` (the price id) and `PADDLE_ENV` (`sandbox` while testing, then `production`); `EMAIL_FROM` is `Decay <licenses.decay@aryangarg.com>`. Then, logged in with `npx wrangler login`, from that folder:
   - `npx wrangler secret put LICENSE_SIGNING_KEY < ~/.decay/license-signing.key` (read from the file: the key is never typed or shown)
   - `npx wrangler secret put PADDLE_API_KEY`
   - `npx wrangler secret put RESEND_API_KEY`
   - `npx wrangler deploy` → note the Worker's URL.
4. **Paddle notifications:** Developer tools → Notifications → New destination: URL `https://<worker URL>/webhooks/purchase`, event **transaction.completed**. Copy the destination's **secret key** and `npx wrangler secret put PADDLE_WEBHOOK_SECRET`.
5. **Test:** buy through the sandbox checkout with Paddle's test card and your own address; activate the emailed key in Decay (Settings → Decay Pro). Then repeat steps 1, 3 and 4 against the live account with `PADDLE_ENV = "production"`.

**As set up (24 Nov 2026, sandbox):** price `pri_01m878svs4bgc3z7wq69kpp85z` (in `wrangler.toml` and `apps/site/src/consts.ts`), client-side token in `apps/site/src/consts.ts` (public by design), default payment link `https://decay.aryangarg.com/buy`. The checkout opens on the website's `/buy` page (Paddle.js overlay; Paddle's own links arrive with `?_ptxn=`), success page `/thanks`; the app's Settings → Decay Pro has **Buy Decay Pro — $24** (opens `/buy`). For live sales switch `PADDLE.environment`, the token and the price id in `consts.ts`, and `PADDLE_ENV` / `PADDLE_PRICE_IDS` in `wrangler.toml`.

**Live since 28 Nov 2026** (account approved 28 Nov; first live purchase and activation verified the same day). Set up 26 Nov 2026: price `pri_01m8bsze9kntjnct8hcx8jdgk8` ($24 one-time), client-side token `live_cdb7e26dd1d269e4a743fbeafdd`. Both are already switched on branch `paddle-live` (6e2a4c7). On approval day: live API key → `npx wrangler secret put PADDLE_API_KEY`; Events/Notifications → new destination `https://decay-license-webhook.decay-license-webhook.workers.dev/webhooks/purchase` for `transaction.completed`, its secret → `npx wrangler secret put PADDLE_WEBHOOK_SECRET` (both in a real terminal); merge `paddle-live`, `npx wrangler deploy` from `services/license-webhook`; approve the domain; one real purchase → activate → refund.

Notes: a transaction always produces the same key (`pd-<txn id>`; a retried notification resends it at most once, through Resend's idempotency key). A refund doesn't revoke the key: offline keys can't be. To resend a key by hand, `decay-license-tool issue` (above) with the buyer's address and `pd-<txn id>`.

## Windows

- **Installer:** one x64 NSIS installer, `Decay_<v>_x64-setup.exe`, **per user** (`%LOCALAPPDATA%\Decay`, no administrator prompt; matches the per-user data, pipe and native-host registry entries). No MSI for now: it installs per machine, which needs an administrator, and the updater would then need one too. WebView2 is fetched by a small bootstrapper when missing (Windows 11 and up-to-date Windows 10 have it).
- **The native host:** `decay-host.exe` sits next to `Decay.exe`, added by `apps/desktop/src-tauri/windows/installer-hooks.nsh` from what `scripts/sidecar.mjs` staged. The installer ends a running `decay-host.exe` first (a browser keeps one per connected extension; the extension reconnects). Uninstalling removes the host's registry entries for Chrome, Chromium, Edge and Brave, and Windows' login item; ticking *Delete the application data* also removes `%APPDATA%\Decay` (the database key stays in Credential Manager as `com.decay.app` and is useless without it). An update keeps all of these.
- **Updates:** the same `latest.json` lists `windows-x86_64` (the installer and its `.sig`). *Restart to update* stops the engine, runs the installer with a progress bar only (passive) and the installer starts the new version. *Install on quit* runs it at quit without starting Decay again.
- **Checking an installer:** Actions → *Windows installer check* → Run workflow builds an unsigned installer and checks install, the host, reinstall over a running host and uninstall on a clean machine (`tools/windows-installer-check.ps1`; results in the run's artifact).

## Cutting a release

1. Set the version in `apps/desktop/src-tauri/tauri.conf.json` (and `apps/desktop/package.json`, `apps/desktop/src-tauri/Cargo.toml` to keep them in step). A beta uses a pre-release version, e.g. `0.3.0-beta.1`. If the release has something worth telling, add its **What's New** entry (exactly this version) at the top of `apps/desktop/src/help/releases.ts`: it shows once in the Library after the update. Without one, nothing is shown.
2. Commit, then tag exactly `v<version>` and push the tag: `git tag v0.2.0 && git push origin v0.2.0`.
3. The **Release** workflow builds a universal (Apple silicon + Intel) app and uploads a **draft** release to `decay-releases` with `Decay_<v>_universal.dmg`, `Decay.app.tar.gz`, its `.sig` and `latest.json`; then the Windows job adds `Decay_<v>_x64-setup.exe`, its `.sig` and the Windows entry in `latest.json`. **Replace the draft's text** ("RELEASE NOTES GO HERE") with the release notes: Publish copies it into `latest.json`, and that's what Settings → Updates shows. Publish refuses a draft whose text wasn't changed.
4. Try the draft's `.dmg` on a Mac and its `-setup.exe` on a Windows PC.
5. **Publish:** Actions → Release → *Run workflow* with the tag. That writes the notes into `latest.json`, publishes the draft (a pre-release for beta tags, "Latest" for stable) and refreshes the beta manifest, unless the beta manifest already holds a newer version (e.g. a stable hotfix published after a newer beta): then it's left alone. From then on, installed copies find it within a day, or at once with *Check now*.

To withdraw a bad release: mark it as a draft again (stable users stop seeing it), and re-run *Publish* for the previous good tag with **force_beta** ticked to point the beta manifest back at it. Installed copies never downgrade; ship a fixed, higher version.

## Unsigned builds (until the Apple account exists)

- **First install:** macOS says the app "can't be opened because Apple cannot check it". On macOS 15 and later the only way is System Settings → Privacy & Security → *Open Anyway* (right-click → Open no longer bypasses it; it still works on macOS 14). Updates installed by the app itself aren't affected (they don't carry the download quarantine).
- **Keychain prompt after each update:** the database key is in the login Keychain, trusted for the app's code signature. An ad-hoc signature changes with every build, so after an update macOS asks whether Decay may use its Keychain item: choose *Always Allow*. Developer ID signing removes this (the signature's identity stays the same).
- **Window access (Accessibility) after each update:** macOS ties the permission to the code signature too, so an ad-hoc update most likely turns it off. Decay keeps working with tabs only; turn it back on in System Settings → Privacy & Security → Accessibility (remove the old Decay entry first if it's listed). Developer ID signing removes this too.

## Testing an update locally

Debug builds read a test manifest from `DECAY_UPDATE_URL` (ignored in release builds) and check only when `DECAY_UPDATE_CHECK=1` or on *Check now*. Build two versions with the key (`TAURI_SIGNING_PRIVATE_KEY`, `--config src-tauri/tauri.release.conf.json`), serve the newer one's `.tar.gz` and a `latest.json` over HTTPS (or allow plain HTTP with `plugins.updater.dangerousInsecureTransportProtocol` in a test-only config), and point the older one at it. Always set `DECAY_SUPPORT_DIR` to a scratch folder.
