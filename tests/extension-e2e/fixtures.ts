import { chromium, test as base, expect, type BrowserContext, type Page } from "@playwright/test";
import {
  EXTENSION_E2E_DIR,
  logE2e,
} from "../../scripts/prepare-extension-e2e.mjs";
import {
  HOST_RESOLVER_RULES,
  PLAYER_ORIGIN,
} from "./payloads.mjs";

export type PortLogEntry = {
  kind?: string;
  t: number;
  dataBytes: number;
  contentLength: string;
};

export type CorsFetchResult = {
  ok?: boolean;
  status?: number;
  headers?: { contentType: string; contentLength: string };
  chunkCount?: number;
  byteLength?: number;
  bodyBase64?: string;
  portLog?: PortLogEntry[];
  timing?: { metaMs: number; firstChunkMs: number | null; doneMs: number };
  error?: string;
};

export function portKinds(result: CorsFetchResult): string[] {
  return (result.portLog || []).map((e) => e.kind || "").filter(Boolean);
}

export const test = base.extend<
  {
    context: BrowserContext;
    page: Page;
  },
  {
    extensionContext: BrowserContext;
    extensionId: string;
  }
>({
  extensionContext: [async ({}, use) => {
    logE2e(`Launching Chromium with --load-extension=${EXTENSION_E2E_DIR}`);
    const launchStarted = Date.now();
    const context = await chromium.launchPersistentContext("", {
      channel: "chromium",
      headless: !!process.env.CI,
      ignoreDefaultArgs: ["--disable-extensions"],
      args: [
        `--disable-extensions-except=${EXTENSION_E2E_DIR}`,
        `--load-extension=${EXTENSION_E2E_DIR}`,
        `--host-resolver-rules=${HOST_RESOLVER_RULES}`,
      ],
    });
    logE2e(`Chromium launched (${Date.now() - launchStarted}ms)`);
    await use(context);
    await context.close();
  }, { scope: "worker" }],
  extensionId: [async ({ extensionContext }, use) => {
    let [serviceWorker] = extensionContext.serviceWorkers();
    if (!serviceWorker) {
      serviceWorker = await extensionContext.waitForEvent("serviceworker");
    }
    await use(new URL(serviceWorker.url()).hostname);
  }, { scope: "worker" }],
  // Alias the built-in test-scoped context to the shared extension window.
  context: async ({ extensionContext }, use) => {
    await use(extensionContext);
  },
  page: async ({ extensionContext }, use) => {
    const page = extensionContext.pages()[0] ?? (await extensionContext.newPage());
    if ((await page.locator("html").getAttribute("data-dirplayer-e2e-hook")) !== "ready") {
      await page.goto(PLAYER_ORIGIN + "/", { waitUntil: "domcontentloaded" });
      await expect(page.locator("html")).toHaveAttribute(
        "data-dirplayer-e2e-hook",
        "ready",
        { timeout: 20_000 },
      );
    }
    await use(page);
  },
});

export { expect };

export async function isolatedCorsFetch(
  page: Page,
  url: string,
): Promise<CorsFetchResult> {
  return isolatedHookCall(page, { kind: "fetch", url });
}

export async function isolatedPortLog(page: Page): Promise<PortLogEntry[]> {
  const result = await isolatedHookCall(page, { kind: "port-log" });
  return result.portLog || [];
}

export async function isolatedPortPeek(page: Page): Promise<PortLogEntry[]> {
  const result = await isolatedHookCall(page, { kind: "port-peek" });
  return result.portLog || [];
}

async function isolatedHookCall(
  page: Page,
  extra: { kind: string; url?: string },
): Promise<CorsFetchResult> {
  return page.evaluate(async ({ kind, url }) => {
    const CHANNEL = "dirplayer-e2e-cors";
    const id = `${Date.now()}-${Math.random().toString(36).slice(2)}`;
    return await new Promise((resolve, reject) => {
      const timer = window.setTimeout(() => {
        window.removeEventListener("message", onMessage);
        reject(new Error("timed out waiting for isolated-world hook"));
      }, 30_000);

      function onMessage(event: MessageEvent) {
        if (event.source !== window) return;
        const msg = event.data as CorsFetchResult & {
          type?: string;
          kind?: string;
          id?: string;
        };
        if (!msg || msg.type !== CHANNEL || msg.kind !== "result" || msg.id !== id) {
          return;
        }
        window.removeEventListener("message", onMessage);
        window.clearTimeout(timer);
        resolve(msg);
      }

      window.addEventListener("message", onMessage);
      window.postMessage({ type: CHANNEL, kind, id, url }, "*");
    });
  }, extra);
}

export async function gotoPlayerPage(page: Page, path: string) {
  await page.goto(`${PLAYER_ORIGIN}${path}`, { waitUntil: "domcontentloaded" });
  await expect(page.locator("html")).toHaveAttribute(
    "data-dirplayer-e2e-hook",
    "ready",
    { timeout: 20_000 },
  );
}

export async function waitForMovieLoad(page: Page) {
  const error = page.getByText("Movie failed to load");
  await expect
    .poll(
      async () => {
        if ((await error.count()) > 0) {
          const message = (await error.textContent())?.trim() || "unknown error";
          throw new Error(`movie failed to load: ${message}`);
        }
        const log = await isolatedPortPeek(page);
        return portKinds({ portLog: log });
      },
      { timeout: 45_000, intervals: [200, 500, 1000] },
    )
    .toContain("done");
  await expect(page.locator("canvas[data-dp-stage]")).toBeVisible();
}

export function decodeBody(result: CorsFetchResult): Buffer {
  if (!result.bodyBase64) return Buffer.alloc(0);
  return Buffer.from(result.bodyBase64, "base64");
}
