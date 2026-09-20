/**
 * Extension CORS proxy e2e — isolated from the movie Playwright suite.
 *
 * Why a separate project: Chrome extensions only load in a persistent Chromium
 * context with --load-extension. The movie runner uses Playwright's default
 * context and a same-origin WASM page, so chrome.runtime never exists there.
 *
 * DCRs: fast.dcr / slow.dcr are synthetic bytes for Port framing. A separate
 * spec loads the corpus shapes movie through the same SW path so WASM
 * fetch_net_task actually parses the body.
 */
import { decodeBody, expect, isolatedCorsFetch, portKinds, test } from "./fixtures";
import {
  CDN_ORIGIN,
  FAST_PATH,
  FAST_PAYLOAD,
  PORT,
  SLOW_PATH,
  SLOW_PAYLOAD,
} from "./payloads.mjs";

test.describe.configure({ mode: "serial" });

test("single-chunk cross-origin .dcr still completes through the SW relay", async ({
  page,
}) => {
  const result = await isolatedCorsFetch(page, `${CDN_ORIGIN}${FAST_PATH}`);

  expect(result.error, `isolated fetch failed: ${result.error}`).toBeUndefined();
  expect(result.ok).toBe(true);
  expect(result.status).toBe(200);
  expect(result.headers?.contentType).toBe("application/x-director");
  // Fetch forbids `Content-Length` on a synthetic Response. WASM already
  // falls back to the received byte count. Port `meta` still carries the
  // header from the SW.
  expect(decodeBody(result).equals(FAST_PAYLOAD)).toBe(true);
  expect(result.byteLength).toBe(FAST_PAYLOAD.length);
  expect(result.chunkCount).toBeGreaterThanOrEqual(1);

  const kinds = portKinds(result);
  expect(kinds[0]).toBe("meta");
  expect(kinds).toContain("chunk");
  expect(kinds[kinds.length - 1]).toBe("done");
  const meta = result.portLog?.find((e) => e.kind === "meta");
  expect(meta?.contentLength).toBe(String(FAST_PAYLOAD.length));
});

test("slow multi-chunk .dcr streams through the SW Port as meta/chunk/done", async ({
  page,
}) => {
  const result = await isolatedCorsFetch(page, `${CDN_ORIGIN}${SLOW_PATH}`);

  expect(result.error, `isolated fetch failed: ${result.error}`).toBeUndefined();
  expect(result.ok).toBe(true);
  expect(result.status).toBe(200);
  expect(result.headers?.contentType).toBe("application/x-director");
  expect(decodeBody(result).equals(SLOW_PAYLOAD)).toBe(true);
  expect(result.byteLength).toBe(SLOW_PAYLOAD.length);

  const kinds = portKinds(result);
  expect(kinds[0]).toBe("meta");
  expect(kinds.filter((k) => k === "chunk").length).toBeGreaterThanOrEqual(2);
  expect(kinds[kinds.length - 1]).toBe("done");
  const meta = result.portLog?.find((e) => e.kind === "meta");
  expect(meta?.contentLength).toBe(String(SLOW_PAYLOAD.length));

  const log = result.portLog || [];
  const metaAt = log.find((e) => e.kind === "meta")?.t ?? 0;
  const doneAt = log.find((e) => e.kind === "done")?.t ?? 0;
  // Headers-first: `meta` is posted before the last chunk. The sendMessage
  // fallback never opens this Port, so this sequence would be missing.
  expect(doneAt).toBeGreaterThan(metaAt);
});

test("loopback URLs are not relayed (page CORS still applies)", async ({ page }) => {
  const result = await isolatedCorsFetch(
    page,
    `http://127.0.0.1:${PORT}${FAST_PATH}`,
  );
  expect(result.error).toBeTruthy();
  expect(portKinds(result)).toEqual([]);
});
