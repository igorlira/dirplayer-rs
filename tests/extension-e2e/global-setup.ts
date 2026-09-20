import { logE2e, prepareExtensionE2e } from "../../scripts/prepare-extension-e2e.mjs";

export default function globalSetup() {
  logE2e("Playwright globalSetup: build the unpacked overlay Chromium will load");
  prepareExtensionE2e();
}
