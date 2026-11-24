// Every package declares the license LICENSE.md gives its folder (run in CI, P4.5).
// Open core: MIT OR Apache-2.0. The desktop app's shell, Pro licensing and the purchase
// webhook: FSL-1.1-ALv2. A new package must be added to LICENSE.md's table and here.
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

const OPEN = 'MIT OR Apache-2.0';
const FSL = 'FSL-1.1-ALv2';
const EXPECTED = {
  'crates/decay-core': OPEN,
  'crates/decay-store': OPEN,
  'crates/decay-bridge': OPEN,
  'crates/decay-host': OPEN,
  'crates/decay-platform': OPEN,
  'crates/decay-engine': OPEN,
  'crates/decay-license': FSL,
  'apps/desktop/src-tauri': FSL,
  'apps/desktop': FSL,
  'apps/extension': OPEN,
  'packages/ui': OPEN,
  'packages/config': OPEN,
  'services/license-webhook': FSL,
};

const root = join(import.meta.dirname, '..');
const workspaceLicense = /\[workspace\.package\][^[]*?\nlicense\s*=\s*"([^"]+)"/s.exec(
  readFileSync(join(root, 'Cargo.toml'), 'utf8'),
)?.[1];

function declared(dir) {
  const cargo = join(root, dir, 'Cargo.toml');
  const pkg = join(root, dir, 'package.json');
  if (existsSync(cargo)) {
    const text = readFileSync(cargo, 'utf8');
    if (/^license\.workspace\s*=\s*true/m.test(text)) return workspaceLicense;
    return /^license\s*=\s*"([^"]+)"/m.exec(text)?.[1];
  }
  if (existsSync(pkg)) return JSON.parse(readFileSync(pkg, 'utf8')).license;
  return undefined;
}

// Every package folder that exists must be listed (nothing slips through unlicensed).
const found = ['crates', 'apps', 'packages', 'services'].flatMap((top) =>
  existsSync(join(root, top))
    ? readdirSync(join(root, top), { withFileTypes: true })
        .filter((d) => d.isDirectory())
        .map((d) => `${top}/${d.name}`)
        .filter(
          (d) =>
            existsSync(join(root, d, 'Cargo.toml')) || existsSync(join(root, d, 'package.json')),
        )
    : [],
);
const problems = [];
for (const dir of found) {
  if (!(dir in EXPECTED)) problems.push(`${dir}: not in tools/check-licenses.mjs / LICENSE.md`);
}
for (const [dir, want] of Object.entries(EXPECTED)) {
  const have = declared(dir);
  if (have !== want)
    problems.push(`${dir}: declares ${have ?? 'nothing'}, LICENSE.md says ${want}`);
  if (want === FSL && !dir.endsWith('src-tauri') && !existsSync(join(root, dir, 'LICENSE.md'))) {
    problems.push(`${dir}: FSL code needs its LICENSE.md`);
  }
}
for (const f of ['LICENSE.md', 'LICENSE-MIT', 'LICENSE-APACHE']) {
  if (!existsSync(join(root, f))) problems.push(`${f} is missing`);
}
if (problems.length > 0) {
  console.error(problems.join('\n'));
  process.exit(1);
}
console.log(`licenses: ${Object.keys(EXPECTED).length} packages as declared in LICENSE.md`);
