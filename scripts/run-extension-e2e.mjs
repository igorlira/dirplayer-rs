#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { DIST_EXTENSION, logE2e } from "./prepare-extension-e2e.mjs";
import { PORT } from "../tests/extension-e2e/payloads.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(__dirname, "..");
const IS_WIN = process.platform === "win32";
const runStarted = Date.now();

logE2e("Checking for a built extension at dist-extension/");
if (!fs.existsSync(path.join(DIST_EXTENSION, "manifest.json"))) {
  console.error("Missing dist-extension. Build it first with: npm run build-extension");
  process.exit(1);
}

logE2e("Starting Playwright. The next pauses are expected:");
logE2e("  1. npx resolving @playwright/test (slow the first time or on a cold disk)");
logE2e("  2. Hard-link overlay into .tmp/extension-e2e (skipping corpus movies)");
logE2e(`  3. Fixture HTTP server on 127.0.0.1:${PORT} (player.dirplayer.test / cdn.dirplayer.test)`);
logE2e("  4. Chromium + MV3 service worker via --load-extension (usually the longest wait)");

const forwardArgs = process.argv.slice(2);
const pw = spawnSync(
  "npx",
  ["playwright", "test", "-c", "playwright.extension.config.ts", ...forwardArgs],
  {
    cwd: REPO_ROOT,
    stdio: "inherit",
    shell: IS_WIN,
    env: process.env,
  },
);
logE2e(
  `Playwright exited with status ${pw.status ?? 1} (${((Date.now() - runStarted) / 1000).toFixed(1)}s)`,
);
process.exit(pw.status ?? 1);
