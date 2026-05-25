// Copy external Xtra SDK artifacts from public/ into a build target so
// the resulting `dist-<host>/` is self-contained and deployable as-is.
//
// What gets copied:
//   - public/*.wasm           → <destDir>/*.wasm           (plugin builds)
//   - public/xtra-registry.json → <destDir>/xtra-registry.json (default registry)
//
// Used by postbuild-polyfill (vite.config.polyfill.js sets
// `publicDir: false`, so the polyfill build doesn't auto-copy public/*
// the way the extension build does — this script fills the gap).
//
// Behavior when there's nothing to copy:
//   - Missing public/*.wasm   → print an info line, keep going
//   - Missing xtra-registry.json → ditto
//   - Both missing            → exit 0 (no-op)
//
// Skipping is intentional: this script ships ahead of the SDK landing
// in main, and a clean upstream build (no xtras built) should still
// succeed. It also keeps `npm run build-polyfill` from breaking when
// you build the polyfill without first running `npm run build-xtras`.

import { existsSync, mkdirSync, readdirSync, copyFileSync, statSync } from 'fs';
import { join } from 'path';

const SRC = 'public';
const destArg = process.argv[2];
if (!destArg) {
  console.error(
    'copy-xtras.mjs: missing destination argument (e.g. dist-polyfill, dist-extension)'
  );
  process.exit(1);
}
const DEST = destArg;

if (!existsSync(SRC)) {
  console.warn(`copy-xtras.mjs: ${SRC}/ does not exist — nothing to copy.`);
  process.exit(0);
}

mkdirSync(DEST, { recursive: true });

// 1. Plugin wasms.
const wasms = readdirSync(SRC).filter(
  (f) =>
    f.endsWith('.wasm') &&
    !f.startsWith('.') &&
    statSync(join(SRC, f)).isFile()
);
if (wasms.length === 0) {
  console.log(
    `copy-xtras.mjs: no *.wasm in ${SRC}/ — run \`npm run build-xtras\` ` +
      `first if you want plugin xtras shipped with ${DEST}/.`
  );
} else {
  for (const wasm of wasms) {
    const src = join(SRC, wasm);
    const dst = join(DEST, wasm);
    copyFileSync(src, dst);
    console.log(`copy-xtras.mjs: ${wasm} → ${dst}`);
  }
}

// 2. Default registry JSON.
const REGISTRY = 'xtra-registry.json';
const registrySrc = join(SRC, REGISTRY);
if (existsSync(registrySrc)) {
  const registryDst = join(DEST, REGISTRY);
  copyFileSync(registrySrc, registryDst);
  console.log(`copy-xtras.mjs: ${REGISTRY} → ${registryDst}`);
} else {
  console.log(
    `copy-xtras.mjs: ${registrySrc} not present — ${DEST}/ will rely on ` +
      `the snake_case convention fallback (~/<name>.wasm) only.`
  );
}
