import { defineConfig } from "@playwright/test";
import { dirname } from "path";
import { fileURLToPath } from "url";

const __dirname = dirname(fileURLToPath(import.meta.url));

// One worker per shard (see E2E_SHARDS in vm-rust/tests/browser/e2e.spec.ts).
// Each worker drives its own browser page, so the shards genuinely run side by
// side rather than taking turns.
const SHARDS = Math.max(1, Number(process.env.E2E_SHARDS ?? 1) || 1);

export default defineConfig({
  testDir: "./vm-rust/tests/browser",
  timeout: 5_400_000,
  fullyParallel: SHARDS > 1,
  workers: SHARDS,
  use: {
    headless: !!process.env.CI,
    baseURL: "http://127.0.0.1:9101",
    video: process.env.CI ? "on" : "off",
  },
  webServer: {
    command: "node scripts/serve-browser-runner.mjs",
    port: 9101,
    cwd: __dirname,
    reuseExistingServer: true,
  },
});
