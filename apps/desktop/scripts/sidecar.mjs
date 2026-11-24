// Builds the native messaging host (`decay-host`) for the target being bundled and
// stages it at src-tauri/sidecar/, where `bundle.macOS.files` puts it next to the app's
// executable (Contents/MacOS/decay-host: the app installs the browser manifests that
// point there, Software Design §4.8). Runs as Tauri's `beforeBundleCommand`.
//
// Windows: src-tauri/windows/installer-hooks.nsh puts it next to Decay.exe.
//
// macOS also gets `decay-labeler` (F15): a Swift helper that names archived batches with
// Apple's on-device model (src-tauri/labeler/main.swift), staged and signed the same way
// (`bundle.macOS.files` puts it at Contents/MacOS/decay-labeler).
//
// macOS: the host is signed before bundling, with $APPLE_SIGNING_IDENTITY when set
// (Developer ID, hardened runtime, timestamp: needed for notarization), else ad hoc.
// Windows: signed with Azure Trusted Signing when $TRUSTED_SIGNING_ACCOUNT is set (the
// release workflow sets it with the same account the installer is signed with), else
// left unsigned.
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync, rmSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const app = join(dirname(fileURLToPath(import.meta.url)), '..');
const root = join(app, '..', '..');
const run = (cmd, args) => execFileSync(cmd, args, { cwd: root, stdio: 'inherit' });
const hostTriple = () =>
  /host: (\S+)/.exec(execFileSync('rustc', ['-vV'], { encoding: 'utf8' }))?.[1] ?? '';

const triple = process.env.TAURI_ENV_TARGET_TRIPLE || hostTriple();
const debug = process.env.TAURI_ENV_DEBUG === 'true';
const profile = debug ? 'debug' : 'release';
const windows = triple.includes('windows');
const exe = windows ? 'decay-host.exe' : 'decay-host';
const build = (target) =>
  run('cargo', ['build', '-p', 'decay-host', '--target', target, ...(debug ? [] : ['--release'])]);
// Cargo's target folder (honours CARGO_TARGET_DIR and .cargo/config).
const targetDir = JSON.parse(
  execFileSync('cargo', ['metadata', '--format-version', '1', '--no-deps'], {
    cwd: root,
    encoding: 'utf8',
    maxBuffer: 64 * 1024 * 1024,
  }),
).target_directory;
const built = (target) => join(targetDir, target, profile, exe);

const outDir = join(app, 'src-tauri', 'sidecar');
const out = join(outDir, exe);
mkdirSync(outDir, { recursive: true });

if (triple === 'universal-apple-darwin') {
  const arches = ['aarch64-apple-darwin', 'x86_64-apple-darwin'];
  for (const t of arches) build(t);
  run('lipo', ['-create', '-output', out, ...arches.map(built)]);
} else {
  build(triple);
  copyFileSync(built(triple), out);
}

if (triple.includes('apple-darwin')) {
  // The labeler: one slice per architecture (macOS 14+; the model itself needs macOS 26
  // and is checked at run time).
  const labeler = join(outDir, 'decay-labeler');
  const source = join(app, 'src-tauri', 'labeler', 'main.swift');
  const arches =
    triple === 'universal-apple-darwin'
      ? ['arm64', 'x86_64']
      : [triple.split('-')[0] === 'aarch64' ? 'arm64' : 'x86_64'];
  const slices = arches.map((arch) => {
    const slice = join(outDir, `decay-labeler-${arch}`);
    run('swiftc', [
      ...(debug ? ['-Onone'] : ['-O']),
      '-target',
      `${arch}-apple-macos14`,
      '-Xlinker',
      '-weak_framework',
      '-Xlinker',
      'FoundationModels',
      source,
      '-o',
      slice,
    ]);
    return slice;
  });
  run('lipo', ['-create', '-output', labeler, ...slices]);
  for (const slice of slices) rmSync(slice);
  const identity = process.env.APPLE_SIGNING_IDENTITY || '-';
  const real = identity !== '-';
  for (const binary of [out, labeler]) {
    run('codesign', [
      '--force',
      '--options',
      'runtime',
      ...(real ? ['--timestamp'] : []),
      '--sign',
      identity,
      binary,
    ]);
  }
}
if (windows && process.env.TRUSTED_SIGNING_ACCOUNT) {
  const env = process.env;
  run('trusted-signing-cli', [
    '-e',
    env.TRUSTED_SIGNING_ENDPOINT,
    '-a',
    env.TRUSTED_SIGNING_ACCOUNT,
    '-c',
    env.TRUSTED_SIGNING_PROFILE,
    '-d',
    'Decay',
    out,
  ]);
}
console.log(`decay-host staged for ${triple}${debug ? ' (debug)' : ''}: ${out}`);
