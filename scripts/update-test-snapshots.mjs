#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const VM_RUST_DIR = path.resolve(__dirname, "..", "vm-rust");
const IS_WIN = process.platform === "win32";

const extraArgs = process.argv.slice(2);
const res = spawnSync(
  "cargo",
  ["test", "--test", "mod", "e2e", "--", ...extraArgs],
  {
    cwd: VM_RUST_DIR,
    stdio: "inherit",
    shell: IS_WIN,
    env: { ...process.env, SNAPSHOT_UPDATE: "1" },
  },
);
if (res.status === 0) {
  console.log("Snapshots updated in vm-rust/tests/snapshots/reference/");
}
process.exit(res.status ?? 1);
