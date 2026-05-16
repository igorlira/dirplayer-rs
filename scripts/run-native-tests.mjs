#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(__dirname, "..");
const VM_RUST_DIR = path.join(REPO_ROOT, "vm-rust");
const IS_WIN = process.platform === "win32";

const forwardArgs = process.argv.slice(2);

const cargo = spawnSync(
  "cargo",
  ["test", "--test", "mod", "--", "e2e", "--nocapture", ...forwardArgs],
  { cwd: VM_RUST_DIR, stdio: "inherit", shell: IS_WIN },
);

// Always generate the HTML snapshot report regardless of test outcome.
spawnSync(
  "node",
  [
    path.join(__dirname, "generate-snapshot-report.mjs"),
    path.join(VM_RUST_DIR, "tests", "snapshots"),
    path.join(REPO_ROOT, "test-results", "snapshot-report"),
  ],
  { cwd: REPO_ROOT, stdio: "inherit", shell: IS_WIN },
);

process.exit(cargo.status ?? 1);
