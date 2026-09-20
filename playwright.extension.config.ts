import { defineConfig } from "@playwright/test";
import { dirname } from "path";
import { fileURLToPath } from "url";
import { PORT } from "./tests/extension-e2e/payloads.mjs";

const __dirname = dirname(fileURLToPath(import.meta.url));

export default defineConfig({
  testDir: "./tests/extension-e2e",
  testMatch: "**/*.spec.ts",
  timeout: 60_000,
  workers: 1,
  fullyParallel: false,
  globalSetup: "./tests/extension-e2e/global-setup.ts",
  use: {
    headless: !!process.env.CI,
    video: process.env.CI ? "on" : "off",
  },
  webServer: {
    command: "node scripts/serve-extension-e2e.mjs",
    port: PORT,
    cwd: __dirname,
    reuseExistingServer: false,
    stdout: "pipe",
    stderr: "pipe",
  },
});
