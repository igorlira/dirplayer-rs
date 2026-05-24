// Build every external Xtra plugin under `xtras/` and copy the resulting
// `.wasm` files into `public/` so the host registry can serve them.
//
// Each xtra under `xtras/<name>/` is expected to be a separate Cargo
// crate with `crate-type = ["cdylib"]` and a path dep on `../../xtra-sdk`.
// We run `cargo build --target wasm32-unknown-unknown --release` in each
// directory, then copy every `.wasm` from its release target into
// `public/`. The host's xtra registry (`public/xtra-registry.json` plus
// the snake_case URL convention) then routes Lingo lookups to those
// wasms at runtime.
//
// Skip rules:
//   - Directories without a Cargo.toml are ignored (allows scratch
//     folders, READMEs, etc.).
//   - An empty `xtras/` (or no submodule init) is non-fatal: prints a
//     hint and exits 0 so `npm run build-all` keeps working before the
//     submodules are fetched.
//
// Usage:
//   node scripts/build-xtras.mjs            # build all
//   node scripts/build-xtras.mjs bobba-xtra # build just the named one

import {
  readdirSync,
  statSync,
  existsSync,
  copyFileSync,
  mkdirSync,
} from 'node:fs';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';

const XTRAS_DIR = 'xtras';
const PUBLIC_DIR = 'public';
const TARGET_TRIPLE = 'wasm32-unknown-unknown';
const PROFILE_DIR = 'release';

const onlyName = process.argv[2] || null;

function listXtras() {
  if (!existsSync(XTRAS_DIR)) return [];
  return readdirSync(XTRAS_DIR)
    .filter((name) => {
      const dir = join(XTRAS_DIR, name);
      try {
        return (
          statSync(dir).isDirectory() &&
          existsSync(join(dir, 'Cargo.toml'))
        );
      } catch {
        return false;
      }
    })
    .map((name) => ({ name, dir: join(XTRAS_DIR, name) }));
}

function buildOne(xtra) {
  console.log(`[build-xtras] === ${xtra.name} ===`);
  const r = spawnSync(
    'cargo',
    ['build', '--target', TARGET_TRIPLE, '--release'],
    { cwd: xtra.dir, stdio: 'inherit', shell: process.platform === 'win32' }
  );
  if (r.status !== 0) {
    console.error(`[build-xtras] ${xtra.name}: cargo build failed (exit ${r.status})`);
    process.exit(r.status || 1);
  }
}

function copyWasm(xtra) {
  const targetDir = join(xtra.dir, 'target', TARGET_TRIPLE, PROFILE_DIR);
  if (!existsSync(targetDir)) {
    console.error(`[build-xtras] ${xtra.name}: no target dir at ${targetDir}`);
    process.exit(1);
  }
  const wasms = readdirSync(targetDir).filter(
    (f) => f.endsWith('.wasm') && !f.startsWith('.')
  );
  if (wasms.length === 0) {
    console.error(`[build-xtras] ${xtra.name}: no .wasm produced under ${targetDir}`);
    process.exit(1);
  }
  if (!existsSync(PUBLIC_DIR)) mkdirSync(PUBLIC_DIR, { recursive: true });
  for (const wasm of wasms) {
    const src = join(targetDir, wasm);
    const dst = join(PUBLIC_DIR, wasm);
    copyFileSync(src, dst);
    console.log(`[build-xtras] ${xtra.name}: ${wasm} → ${dst}`);
  }
}

const all = listXtras();
if (all.length === 0) {
  console.log(
    `[build-xtras] no xtras under ${XTRAS_DIR}/. If you expect plugins ` +
      `here, run \`git submodule update --init --recursive\` first.`
  );
  process.exit(0);
}

const targets = onlyName ? all.filter((x) => x.name === onlyName) : all;
if (targets.length === 0) {
  console.error(`[build-xtras] no xtra matching '${onlyName}'`);
  console.error(`[build-xtras] available: ${all.map((x) => x.name).join(', ')}`);
  process.exit(1);
}

for (const xtra of targets) {
  buildOne(xtra);
  copyWasm(xtra);
}

console.log(`[build-xtras] done (${targets.length} xtra${targets.length === 1 ? '' : 's'})`);
