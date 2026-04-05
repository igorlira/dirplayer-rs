import { defineConfig } from "@playwright/test";
import { dirname } from "path";
import { fileURLToPath } from "url";

const __dirname = dirname(fileURLToPath(import.meta.url));

export default defineConfig({
  testDir: "./vm-rust/tests/browser",
  timeout: 300_000,
  use: {
    headless: !!process.env.CI,
    baseURL: "http://127.0.0.1:9101",
  },
  webServer: {
    command:
      "python3 -m http.server 9101 --directory vm-rust/target/browser_runner --bind 127.0.0.1",
    port: 9101,
    cwd: __dirname,
    reuseExistingServer: true,
  },
});
