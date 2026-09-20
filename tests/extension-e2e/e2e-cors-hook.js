/**
 * Isolated-world hook. Shares the extension's world with the production
 * content script, so `fetch` is the flashPlayerManager CORS relay — not the
 * page's fetch. The page talks to this script with window.postMessage.
 *
 * We wrap `chrome.runtime.connect` so tests can see Port `meta`/`chunk`/`done`
 * even when Chromium delivers the HTTP body in one burst.
 */
const CHANNEL = "dirplayer-e2e-cors";
const CORS_FETCH_PORT = "dirplayer-cors-fetch";

function isNativeFetch() {
  return /\[native code\]/.test(Function.prototype.toString.call(window.fetch));
}

function waitForPatchedFetch(timeoutMs) {
  const start = Date.now();
  return new Promise((resolve, reject) => {
    function tick() {
      if (!isNativeFetch()) {
        resolve();
        return;
      }
      if (Date.now() - start >= timeoutMs) {
        reject(new Error("timed out waiting for extension fetch patch"));
        return;
      }
      setTimeout(tick, 50);
    }
    tick();
  });
}

function installPortSpy() {
  const runtime = globalThis.chrome && globalThis.chrome.runtime;
  if (!runtime || typeof runtime.connect !== "function") {
    throw new Error("chrome.runtime.connect unavailable");
  }
  if (runtime.connect.__dirplayerE2eWrapped) return;
  const orig = runtime.connect.bind(runtime);
  function wrapped(info) {
    const port = orig(info);
    if (info && info.name === CORS_FETCH_PORT) {
      port.onMessage.addListener((msg) => {
        const log = (wrapped.__log = wrapped.__log || []);
        log.push({
          kind: msg && msg.kind,
          t: performance.now(),
          dataBytes: msg && msg.dataBase64 ? atob(msg.dataBase64).length : 0,
          contentLength: msg && msg.contentLength ? String(msg.contentLength) : "",
        });
      });
    }
    return port;
  }
  wrapped.__dirplayerE2eWrapped = true;
  wrapped.__log = [];
  runtime.connect = wrapped;
}

try {
  installPortSpy();
} catch (err) {
  document.documentElement.setAttribute("data-dirplayer-e2e-hook", "error");
  document.documentElement.setAttribute(
    "data-dirplayer-e2e-hook-error",
    String(err && err.message ? err.message : err),
  );
}

waitForPatchedFetch(20000)
  .then(() => {
    if (document.documentElement.getAttribute("data-dirplayer-e2e-hook") !== "error") {
      document.documentElement.setAttribute("data-dirplayer-e2e-hook", "ready");
    }
  })
  .catch((err) => {
    document.documentElement.setAttribute("data-dirplayer-e2e-hook", "error");
    document.documentElement.setAttribute(
      "data-dirplayer-e2e-hook-error",
      String(err && err.message ? err.message : err),
    );
  });

window.addEventListener("message", (event) => {
  if (event.source !== window) return;
  const msg = event.data;
  if (!msg || msg.type !== CHANNEL) return;
  if (msg.kind === "fetch") {
    void runFetch(msg);
    return;
  }
  if (msg.kind === "port-log") {
    window.postMessage(
      { type: CHANNEL, kind: "result", id: msg.id, portLog: portLogSnapshot(true) },
      "*",
    );
    return;
  }
  if (msg.kind === "port-peek") {
    window.postMessage(
      { type: CHANNEL, kind: "result", id: msg.id, portLog: portLogSnapshot(false) },
      "*",
    );
  }
});

function portLogSnapshot(clear) {
  const runtime = globalThis.chrome && globalThis.chrome.runtime;
  const connect = runtime && runtime.connect;
  const log = (connect && connect.__log) || [];
  const copy = log.slice();
  if (clear && connect) connect.__log = [];
  return copy;
}

async function runFetch(msg) {
  const id = msg.id;
  try {
    const t0 = performance.now();
    const res = await fetch(msg.url);
    const metaAt = performance.now();
    const reader = res.body ? res.body.getReader() : null;
    const chunks = [];
    let firstChunkAt = null;
    if (reader) {
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        if (value && value.byteLength > 0) {
          if (firstChunkAt == null) firstChunkAt = performance.now();
          chunks.push(value);
        }
      }
    }
    const doneAt = performance.now();
    let total = 0;
    for (const c of chunks) total += c.byteLength;
    const bytes = new Uint8Array(total);
    let offset = 0;
    for (const c of chunks) {
      bytes.set(c, offset);
      offset += c.byteLength;
    }
    let bin = "";
    const STEP = 0x8000;
    for (let i = 0; i < bytes.length; i += STEP) {
      bin += String.fromCharCode.apply(null, bytes.subarray(i, i + STEP));
    }
    window.postMessage(
      {
        type: CHANNEL,
        kind: "result",
        id,
        ok: res.ok,
        status: res.status,
        headers: {
          contentType: res.headers.get("content-type") || "",
          contentLength: res.headers.get("content-length") || "",
        },
        chunkCount: chunks.length,
        byteLength: bytes.length,
        bodyBase64: btoa(bin),
        portLog: portLogSnapshot(true),
        timing: {
          metaMs: metaAt - t0,
          firstChunkMs: firstChunkAt == null ? null : firstChunkAt - t0,
          doneMs: doneAt - t0,
        },
      },
      "*",
    );
  } catch (err) {
    window.postMessage(
      {
        type: CHANNEL,
        kind: "result",
        id,
        error: String(err && err.message ? err.message : err),
        portLog: portLogSnapshot(true),
      },
      "*",
    );
  }
}
