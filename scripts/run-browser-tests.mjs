#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import dotenv from "dotenv";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import * as esbuild from "esbuild";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(__dirname, "..");
const VM_RUST_DIR = path.join(REPO_ROOT, "vm-rust");
const ASSET_DIR = path.join(REPO_ROOT, "public");
const RUFFLE_DIR = path.join(ASSET_DIR, "ruffle");
const FLASH_MANAGER_SRC = path.join(REPO_ROOT, "src", "services", "flashPlayerManager.ts");
const RUNNER_DIR = path.join(VM_RUST_DIR, "target", "browser_runner");
const TEMPLATE_DIR = path.join(VM_RUST_DIR, "tests", "browser_templates");
const CONFIG_DIR = path.join(VM_RUST_DIR, "tests", "e2e", "configs");
const DOTENV_PATH = path.join(REPO_ROOT, ".env");
const IS_WIN = process.platform === "win32";

// Release by default. The dev profile compiles far quicker but the
// interpreter it produces is several times slower to RUN, which on a corpus
// this size costs much more wall clock than it saves -- and on a CPU-bound CI
// runner the difference is the whole job. `E2E_PROFILE=dev` opts into the fast
// compile when you are iterating on one movie.
const CARGO_PROFILE =
  (process.env.E2E_PROFILE ?? "release") === "dev" ? "dev" : "release";

const dotenvResult = dotenv.config({ path: DOTENV_PATH, quiet: true });
const loadedEnv = {
  ...(dotenvResult.parsed ?? {}),
  ...process.env,
};

// Through the Windows shell an argument with a space splits in two, so a
// checkout under "C:\Users\Some Name\..." handed wasm-bindgen half a path.
function quoted(args) {
  if (!IS_WIN) return args;
  return args.map((a) => (/\s/.test(a) && !/^"/.test(a) ? `"${a}"` : a));
}

function run(cmd, args, opts = {}) {
  const res = spawnSync(cmd, quoted(args), {
    stdio: "inherit",
    shell: IS_WIN,
    ...opts,
  });
  if (res.status !== 0) {
    process.exit(res.status ?? 1);
  }
  return res;
}

// Separate our own flags from args forwarded to playwright.
const cliArgs = process.argv.slice(2);
const forwardArgs = [];
let updateSnapshots = loadedEnv.SNAPSHOT_UPDATE === "1";
let keepOpen = false;
for (const arg of cliArgs) {
  if (arg === "--update" || arg === "-u") {
    updateSnapshots = true;
  } else if (arg === "--debug") {
    keepOpen = true;
  } else {
    forwardArgs.push(arg);
  }
}

// 1. Build browser tests
console.log(`Building browser tests (${CARGO_PROFILE} profile)...`);
run("cargo", [
  "build",
  "--test",
  "mod",
  "--target",
  "wasm32-unknown-unknown",
  ...(CARGO_PROFILE === "release" ? ["--release"] : []),
], { cwd: VM_RUST_DIR });

// 2. Find the built wasm artifact (newest mod-*.wasm)
const depsDir = path.join(
  VM_RUST_DIR,
  "target",
  "wasm32-unknown-unknown",
  CARGO_PROFILE === "release" ? "release" : "debug",
  "deps",
);
const wasmCandidates = fs
  .readdirSync(depsDir)
  .filter((f) => f.startsWith("mod-") && f.endsWith(".wasm"))
  .map((f) => ({
    name: f,
    mtime: fs.statSync(path.join(depsDir, f)).mtimeMs,
  }))
  .sort((a, b) => b.mtime - a.mtime);
if (wasmCandidates.length === 0) {
  console.error("No mod-*.wasm artifact found in", depsDir);
  process.exit(1);
}
const wasmFile = path.join(depsDir, wasmCandidates[0].name);

// 3. Regenerate the runner directory and JS glue.
fs.rmSync(RUNNER_DIR, { recursive: true, force: true });
fs.mkdirSync(RUNNER_DIR, { recursive: true });
run("wasm-bindgen", [wasmFile, "--out-dir", RUNNER_DIR, "--target", "web"]);

// 4. Identify the generated JS filename (exclude *_bg.js).
const jsBasename = fs
  .readdirSync(RUNNER_DIR)
  .find(
    (f) => f.startsWith("mod-") && f.endsWith(".js") && !f.includes("_bg"),
  );
if (!jsBasename) {
  console.error("wasm-bindgen did not produce a mod-*.js file in", RUNNER_DIR);
  process.exit(1);
}

// 4-detach. Append a `__detach()` export to the wasm-bindgen glue.
//
//   Panic recovery re-instantiates the module by importing the glue under a
//   fresh `?gen=N` URL, which gives a distinct module record with its own
//   `wasm` binding. That isolation is deliberate: the glue's
//   FinalizationRegistry callbacks close over `wasm` and call
//   `wasm.__wbg_*_free(ptr)`, so if the OLD record were re-pointed at the NEW
//   instance a late GC would free a stale pointer in the live heap.
//
//   The cost of a separate record is that it pins `instance.exports` — and
//   therefore the whole dead `WebAssembly.Memory`, which for a bitmap-heavy
//   movie is hundreds of MB — for the life of the realm, since module records
//   are never evicted. `__detach()` fixes that: it drops the exports and the
//   cached memory views, leaving behind a Proxy whose every property is a
//   no-op function, so a finalizer that fires afterwards does nothing instead
//   of throwing, and the real memory becomes collectable.
{
  const NL = String.fromCharCode(10);
  const gluePath = path.join(RUNNER_DIR, jsBasename);
  const glue = fs.readFileSync(gluePath, "utf8");
  // Drift-proof: take the cache names from the glue's own declarations rather
  // than hardcoding the list wasm-bindgen happens to emit today.
  const cacheNames = [
    ...glue.matchAll(/^let (cached[A-Za-z0-9_$]*Memory0)/gm),
  ].map((m) => m[1]);
  const detach = [
    "",
    "// ---- appended by scripts/run-browser-tests.mjs (e2e panic recovery) ----",
    "// Release this module record's wasm instance so its memory can be",
    "// collected. The stand-in Proxy keeps late FinalizationRegistry callbacks",
    "// (which call `wasm.__wbg_*_free`) harmless instead of throwing.",
    "export function __detach() {",
    "  wasm = new Proxy({}, { get: () => () => undefined });",
    "  wasmModule = undefined;",
    ...cacheNames.map((n) => `  ${n} = null;`),
    "}",
    "",
  ].join(NL);
  fs.writeFileSync(gluePath, glue + detach);
}

// 4a. Generate `vm-rust-live.js` — a forwarding shim the importmap points at
//     instead of the wasm-bindgen glue directly.
//
//     The e2e runner re-instantiates the wasm module after a Rust panic (a wasm
//     trap leaves the instance unusable, see the panic-recovery block in the
//     template). Re-instantiating means a NEW module record with a NEW set of
//     exports, so anything holding a static `import ... from 'vm-rust'` would
//     keep calling into the dead instance forever. `flashPlayerManager.ts` does
//     exactly that.
//
//     So `vm-rust` resolves to this shim, whose exports are thin forwarders to
//     whichever instance is current. The template calls `__setLive(wasm)` after
//     every (re-)instantiation. Generated from the glue's own export list so it
//     can never drift from it.
{
  const NL = String.fromCharCode(10);
  const glue = fs.readFileSync(path.join(RUNNER_DIR, jsBasename), "utf8");
  const fnNames = [
    ...glue.matchAll(/^export function ([A-Za-z_$][A-Za-z0-9_$]*)/gm),
  ].map((m) => m[1]);
  const classNames = [
    ...glue.matchAll(/^export class ([A-Za-z_$][A-Za-z0-9_$]*)/gm),
  ].map((m) => m[1]);
  const shim = [
    "// GENERATED by scripts/run-browser-tests.mjs -- do not edit.",
    "// Forwards the `vm-rust` module specifier to the CURRENT wasm instance,",
    "// so a post-panic re-instantiation is picked up by static importers.",
    "let live = null;",
    "export function __setLive(mod) { live = mod; }",
    "function __live() {",
    "  if (!live) throw new Error('vm-rust shim: no live instance (call __setLive first)');",
    "  return live;",
    "}",
    ...fnNames.map(
      (n) => `export function ${n}(...a) { return __live().${n}(...a); }`,
    ),
    // Classes can't be forwarded per-call, so expose each as a Proxy that
    // resolves against the live instance at construct/get time.
    ...classNames.map(
      (n) =>
        `export const ${n} = new Proxy(function () {}, { construct: (_t, a) => Reflect.construct(__live().${n}, a), get: (_t, k) => __live().${n}[k] });`,
    ),
    "",
  ].join(NL);
  fs.writeFileSync(path.join(RUNNER_DIR, "vm-rust-live.js"), shim);
}

// 5. Scan TOML test configs for ${VAR_NAME} references and collect values
//    from the current process environment.
const envVars = new Set();
if (fs.existsSync(CONFIG_DIR)) {
  const envVarRe = /\$\{([A-Z0-9_]+)/g;
  for (const entry of fs.readdirSync(CONFIG_DIR)) {
    if (!entry.endsWith(".toml")) continue;
    const contents = fs.readFileSync(path.join(CONFIG_DIR, entry), "utf8");
    let m;
    while ((m = envVarRe.exec(contents))) envVars.add(m[1]);
  }
}
const testEnv = {};
for (const name of envVars) {
  const value = loadedEnv[name];
  if (value !== undefined && value !== "") testEnv[name] = value;
}
const testEnvJson = JSON.stringify(testEnv);

// 5a. Build the test -> tags map for `E2E_TAGS`.
//
// Tags describe the SUBSYSTEMS a movie exercises (`[test] tags` in its TOML —
// 2d / flash / 3d / havok / physx / groove3d, the Type column of
// docs/github_wiki/Tested-Movies.md). They let a run be narrowed to the port
// being worked on: `E2E_TAGS=havok` replays only the Havok titles instead of
// the whole suite, which is both slow and, played end to end, heavy enough to
// exhaust the browser before it finishes.
//
// Resolved here rather than in Rust because it costs nothing at build time and
// needs no cross-language plumbing: each e2e source file covers exactly one
// movie and `include_str!`s that movie's config, so the tests a file declares
// inherit the tags of the config(s) it includes.
const E2E_DIR = path.join(VM_RUST_DIR, "tests", "e2e");

function tomlTags(configFile) {
  const full = path.join(CONFIG_DIR, configFile);
  if (!fs.existsSync(full)) return [];
  const contents = fs.readFileSync(full, "utf8");
  // `tags = ["3d", "havok"]` — single-line array, which is how the configs
  // are written. Deliberately not a TOML parse: this is one well-known key.
  const m = /^\s*tags\s*=\s*\[([^\]]*)\]/m.exec(contents);
  if (!m) return [];
  return [...m[1].matchAll(/["']([^"']+)["']/g)].map((t) => t[1].toLowerCase());
}

// `[test] shard_group = "..."` — an AFFINITY key, not a filter. Tests whose
// configs declare the same group are pinned to the same shard, so they run one
// after another instead of side by side.
//
// It exists for the movies that hold a LIVE MULTIUSER SESSION: Coke Studios and
// Habbo each log a single account into a real server, so two of their tests in
// different shards fight over the same user. Measured with E2E_SHARDS=3:
// `test_cokestudios_jukebox` sat waiting on `oIsoScene` for its full 180 s
// while another shard held the login, then failed.
function tomlShardGroup(configFile) {
  const full = path.join(CONFIG_DIR, configFile);
  if (!fs.existsSync(full)) return null;
  const contents = fs.readFileSync(full, "utf8");
  const m = /^\s*shard_group\s*=\s*["']([^"']+)["']/m.exec(contents);
  return m ? m[1].toLowerCase() : null;
}

function collectTestTags(dir, out) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name !== "configs") collectTestTags(full, out);
      continue;
    }
    if (!entry.name.endsWith(".rs")) continue;
    const src = fs.readFileSync(full, "utf8");
    const tags = new Set();
    let shardGroup = null;
    for (const m of src.matchAll(/include_str!\("\.\.\/configs\/([^"]+)"\)/g)) {
      for (const t of tomlTags(m[1])) tags.add(t);
      shardGroup = shardGroup || tomlShardGroup(m[1]);
    }
    if (shardGroup) {
      for (const m of src.matchAll(
        /^[ 	]*(?:browser_e2e_test|hybrid_e2e_test|native_e2e_test)!\(\s*(test_\w+)/gm,
      )) {
        shardGroups[m[1]] = shardGroup;
      }
    }
    if (tags.size === 0) continue;
    // A commented-out test is still a `//`-prefixed macro call; skip those so a
    // disabled test can't be resurrected by a tag filter.
    for (const m of src.matchAll(
      /^[ \t]*(?:browser_e2e_test|hybrid_e2e_test|native_e2e_test)!\(\s*(test_\w+)/gm,
    )) {
      out[m[1]] = [...tags];
    }
  }
}

const testTags = {};
const shardGroups = {};
if (fs.existsSync(E2E_DIR)) collectTestTags(E2E_DIR, testTags);
const testTagsJson = JSON.stringify(testTags);
const shardGroupsJson = JSON.stringify(shardGroups);

const tagFilter = (loadedEnv.E2E_TAGS || "").replace(/"/g, "");
if (tagFilter) {
  const wanted = tagFilter
    .split(",")
    .map((t) => t.trim().toLowerCase())
    .filter(Boolean);
  const matching = Object.entries(testTags).filter(([, tags]) =>
    tags.some((t) => wanted.includes(t)),
  );
  console.log(
    `E2E_TAGS=${tagFilter}: ${matching.length} of ${Object.keys(testTags).length} tagged tests match`,
  );
  if (matching.length === 0) {
    const known = [...new Set(Object.values(testTags).flat())].sort();
    console.warn(`  no test carries any of those tags. Known tags: ${known.join(", ")}`);
  }
}

// 6. Copy the JS API stub and render the HTML template.
fs.copyFileSync(
  path.join(TEMPLATE_DIR, "dirplayer-js-api.js"),
  path.join(RUNNER_DIR, "dirplayer-js-api.js"),
);

// 6.1. Also copy the REAL dirplayer-js-api bridge alongside the stub so
// the stub can re-export plugin-loading functions from it. The stub
// keeps no-op UI callbacks (onMovieLoaded etc.) but delegates xtra
// dispatch (loadExternalXtra, createExternalXtraInstance, etc.) to the
// production implementation — so tests that load SDK plugins exercise
// the real wire path end-to-end. `setVmModule(wasm)` in the HTML
// template glues vm-rust resolution to the bridge.
fs.copyFileSync(
  path.join(REPO_ROOT, "dirplayer-js-api", "index.js"),
  path.join(RUNNER_DIR, "dirplayer-js-api-real.js"),
);

// 6a. Bundle flashPlayerManager.ts for Ruffle integration.
//     The `vm-rust` import is externalized and resolved through the
//     importmap to the test's wasm-bindgen module.
await esbuild.build({
  entryPoints: [FLASH_MANAGER_SRC],
  bundle: true,
  format: "esm",
  target: "es2020",
  external: ["vm-rust"],
  outfile: path.join(RUNNER_DIR, "flashPlayerManager.bundle.js"),
  logLevel: "info",
});

// 6b. Copy the Ruffle runtime into the runner so ruffle.js can load
//     its wasm chunk from a sibling path.
if (fs.existsSync(RUFFLE_DIR)) {
  fs.cpSync(RUFFLE_DIR, path.join(RUNNER_DIR, "ruffle"), { recursive: true });
} else {
  console.warn(`Ruffle directory not found at ${RUFFLE_DIR}; Flash members won't render in tests.`);
}

const template = fs.readFileSync(
  path.join(TEMPLATE_DIR, "index.template.html"),
  "utf8",
);
const html = template
  .replaceAll("$WASM_JS_FILE", jsBasename)
  // The .wasm binary next to the glue. The template compiles it ONCE into a
  // `WebAssembly.Module` and re-instantiates from that on panic recovery, so a
  // restart costs an instantiation (ms) rather than a recompile.
  .replaceAll("$WASM_BG_FILE", jsBasename.replace(/\.js$/, "_bg.wasm"))
  .replaceAll("$TEST_ENV_JSON", testEnvJson)
  .replaceAll("$DEBUG_MODE", keepOpen ? "true" : "false")
  // Optional substring filter: `E2E_FILTER=lore npm run e2e-test-browser`
  // runs only test_* functions whose name contains the string.
  .replaceAll("$TEST_FILTER", (loadedEnv.E2E_FILTER || "").replace(/"/g, ""))
  // Subsystem filter: `E2E_TAGS=3d,havok npm run e2e-test-browser`. ANDed with
  // E2E_FILTER, so the two compose. $TEST_TAGS_JSON must be substituted FIRST:
  // $TEST_TAGS is a prefix of it, so the shorter token would eat the longer
  // one's placeholder and leave `window.__testTagMap = havok_JSON`.
  .replaceAll("$SHARD_GROUPS_JSON", shardGroupsJson)
  .replaceAll("$TEST_TAGS_JSON", testTagsJson)
  .replaceAll("$TEST_TAGS", tagFilter)
  // `E2E_INTERP_STATS=1` turns on the interpreter opcode/escape counters for
  // the run and writes test-results/interp-stats.txt. OFF by default: the
  // counters add two atomic RMWs per interpreted opcode, which is harmless for
  // counting but perturbs timing, and some snapshots are timing-sensitive.
  .replaceAll("$INTERP_STATS", loadedEnv.E2E_INTERP_STATS === "1" ? "true" : "false");
fs.writeFileSync(path.join(RUNNER_DIR, "index.html"), html);

// 7. Link the asset directory into the runner. Use a junction on Windows so
//    we don't need admin privileges; symlink elsewhere.
const assetsLink = path.join(RUNNER_DIR, "assets");
try {
  fs.rmSync(assetsLink, { recursive: true, force: true });
} catch {
  // ignore
}
try {
  fs.symlinkSync(ASSET_DIR, assetsLink, IS_WIN ? "junction" : "dir");
} catch (e) {
  console.error(
    `Failed to link assets (${ASSET_DIR} -> ${assetsLink}): ${e.message}`,
  );
  process.exit(1);
}

console.log(`Generated test runner in ${RUNNER_DIR}`);

// 8. Run Playwright. SNAPSHOT_UPDATE propagates via process.env.
console.log("Running Playwright tests...");
const playwrightEnv = { ...process.env };
if (updateSnapshots) playwrightEnv.SNAPSHOT_UPDATE = "1";
if (keepOpen) playwrightEnv.E2E_KEEP_OPEN = "1";
// `E2E_PROXY` configures the same-origin reverse proxy in
// `scripts/serve-browser-runner.mjs` (see the comment there for why a movie's
// live backend needs one). Playwright starts that server, so the value has to
// reach it through this env -- and it is read from `loadedEnv`, not
// `process.env`, so a movie's backend can be declared once in `.env` beside the
// credentials it goes with rather than exported by hand on every run.
if (loadedEnv.E2E_PROXY) playwrightEnv.E2E_PROXY = loadedEnv.E2E_PROXY;

const pw = spawnSync("npx", ["playwright", "test", ...forwardArgs], {
  cwd: REPO_ROOT,
  stdio: "inherit",
  shell: IS_WIN,
  env: playwrightEnv,
});

// 9. Always generate the HTML snapshot report regardless of test outcome.
spawnSync(
  "node",
  quoted([
    path.join(__dirname, "generate-snapshot-report.mjs"),
    path.join(VM_RUST_DIR, "tests", "snapshots"),
    path.join(REPO_ROOT, "test-results", "snapshot-report"),
  ]),
  { cwd: REPO_ROOT, stdio: "inherit", shell: IS_WIN },
);

process.exit(pw.status ?? 1);
