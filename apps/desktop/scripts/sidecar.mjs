// Builds the native messaging host (`decay-host`) for the target being bundled and
// stages it at src-tauri/sidecar/, where `bundle.macOS.files` puts it next to the app's
// executable (Contents/MacOS/decay-host: the app installs the browser manifests that
// point there, Software Design §4.8). Runs as Tauri's `beforeBundleCommand`.
//
// macOS: the host is signed before bundling, with $APPLE_SIGNING_IDENTITY when set
// (Developer ID, hardened runtime, timestamp: needed for notarization), else ad hoc.
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync } from 'node:fs';
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
  const identity = process.env.APPLE_SIGNING_IDENTITY || '-';
  const real = identity !== '-';
  run('codesign', [
    '--force',
    '--options',
    'runtime',
    ...(real ? ['--timestamp'] : []),
    '--sign',
    identity,
    out,
  ]);
}
console.log(`decay-host staged for ${triple}${debug ? ' (debug)' : ''}: ${out}`);
