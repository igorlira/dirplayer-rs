#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
export const REPO_ROOT = path.resolve(__dirname, "..");
export const DIST_EXTENSION = path.join(REPO_ROOT, "dist-extension");
export const EXTENSION_E2E_DIR = path.join(REPO_ROOT, ".tmp", "extension-e2e");
const HOOK_SRC = path.join(REPO_ROOT, "tests", "extension-e2e", "e2e-cors-hook.js");
const HOOK_NAME = "e2e-cors-hook.js";

/** Progress line so a slow disk or first Chromium launch does not look hung. */
export function logE2e(message) {
  const now = new Date();
  const hh = String(now.getHours()).padStart(2, "0");
  const mm = String(now.getMinutes()).padStart(2, "0");
  const ss = String(now.getSeconds()).padStart(2, "0");
  console.log(`[extension-e2e ${hh}:${mm}:${ss}] ${message}`);
}

function skipEntry(name) {
  // Vite copies public/ test movies into dist-extension; the CORS e2e server
  // serves the corpus .dcr itself. Walking those trees dominates overlay time.
  return name === "manifest.json"
    || name === ".DS_Store"
    || name === HOOK_NAME
    || name.startsWith("dcr_");
}

/**
 * Clone `src` into `dest` with hardlinks for files (`cp -al`). Chromium's
 * --load-extension does not inject content scripts from a tree that contains
 * symlinks (the e2e hook never ran). Hardlinks are regular files to Chrome,
 * cheap, and work on NTFS without admin. Fall back to copy on EXDEV.
 */
function hardlinkTree(src, dest) {
  const stat = fs.lstatSync(src);
  if (stat.isDirectory()) {
    fs.mkdirSync(dest, { recursive: true });
    for (const name of fs.readdirSync(src)) {
      if (name === ".DS_Store") continue;
      hardlinkTree(path.join(src, name), path.join(dest, name));
    }
    return;
  }
  try {
    fs.linkSync(src, dest);
  } catch {
    fs.copyFileSync(src, dest);
  }
}

export function prepareExtensionE2e() {
  if (!fs.existsSync(path.join(DIST_EXTENSION, "manifest.json"))) {
    throw new Error(
      `Missing ${DIST_EXTENSION}. Build it first with: npm run build-extension`,
    );
  }
  if (!fs.existsSync(HOOK_SRC)) {
    throw new Error(`Missing CORS e2e hook at ${HOOK_SRC}`);
  }

  logE2e(
    `Hard-linking ${DIST_EXTENSION} → ${EXTENSION_E2E_DIR} (skipping dcr_* movies already served from public/)`,
  );
  const overlayStarted = Date.now();
  fs.mkdirSync(path.dirname(EXTENSION_E2E_DIR), { recursive: true });
  fs.rmSync(EXTENSION_E2E_DIR, { recursive: true, force: true });
  fs.mkdirSync(EXTENSION_E2E_DIR, { recursive: true });

  for (const name of fs.readdirSync(DIST_EXTENSION)) {
    if (skipEntry(name)) continue;
    hardlinkTree(path.join(DIST_EXTENSION, name), path.join(EXTENSION_E2E_DIR, name));
  }

  fs.copyFileSync(HOOK_SRC, path.join(EXTENSION_E2E_DIR, HOOK_NAME));

  const manifest = JSON.parse(
    fs.readFileSync(path.join(DIST_EXTENSION, "manifest.json"), "utf8"),
  );
  manifest.content_scripts = manifest.content_scripts || [];
  manifest.content_scripts.unshift({
    matches: ["http://player.dirplayer.test/*"],
    js: [HOOK_NAME],
    run_at: "document_start",
    all_frames: false,
  });
  fs.writeFileSync(
    path.join(EXTENSION_E2E_DIR, "manifest.json"),
    JSON.stringify(manifest, null, 2),
  );
  logE2e(`Overlay ready (${Date.now() - overlayStarted}ms)`);
  return EXTENSION_E2E_DIR;
}

if (import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  prepareExtensionE2e();
}
