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

const dotenvResult = dotenv.config({ path: DOTENV_PATH, quiet: true });
const loadedEnv = {
  ...(dotenvResult.parsed ?? {}),
  ...process.env,
};

function run(cmd, args, opts = {}) {
  const res = spawnSync(cmd, args, {
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
for (const arg of cliArgs) {
  if (arg === "--update" || arg === "-u") {
    updateSnapshots = true;
  } else {
    forwardArgs.push(arg);
  }
}

// 1. Build browser tests
console.log("Building browser tests...");
run("cargo", [
  "build",
  "--test",
  "mod",
  "--target",
  "wasm32-unknown-unknown",
  "--release",
], { cwd: VM_RUST_DIR });

// 2. Find the built wasm artifact (newest mod-*.wasm)
const depsDir = path.join(
  VM_RUST_DIR,
  "target",
  "wasm32-unknown-unknown",
  "release",
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

// 6. Copy the JS API stub and render the HTML template.
fs.copyFileSync(
  path.join(TEMPLATE_DIR, "dirplayer-js-api.js"),
  path.join(RUNNER_DIR, "dirplayer-js-api.js"),
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
  .replaceAll("$TEST_ENV_JSON", testEnvJson);
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
const pw = spawnSync("npx", ["playwright", "test", ...forwardArgs], {
  cwd: REPO_ROOT,
  stdio: "inherit",
  shell: IS_WIN,
  env: playwrightEnv,
});

// 9. Always generate the HTML snapshot report regardless of test outcome.
spawnSync(
  "node",
  [
    path.join(__dirname, "generate-snapshot-report.mjs"),
    path.join(VM_RUST_DIR, "tests", "snapshots"),
    path.join(REPO_ROOT, "test-results", "snapshot-report"),
  ],
  { cwd: REPO_ROOT, stdio: "inherit", shell: IS_WIN },
);

process.exit(pw.status ?? 1);
