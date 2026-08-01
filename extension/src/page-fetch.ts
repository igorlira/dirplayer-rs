// Route the isolated-world player's fetches through the page's own `fetch`
// when the page has replaced it.
//
// Archive front-ends serve emulated content under its ORIGINAL address by
// installing a `fetch` rewrite in the page (ooooooooo.ooo answers
// `http://candystand.com/shock/csmgmain.dcr` out of its own store this way).
// A page-hosted polyfill picks that up for free — it runs in the page's world.
// The extension does not: content scripts get their own globals, so the wasm's
// `window.fetch` is the browser's, the request goes out to a host that no
// longer serves the file, and the movie fails to load.
//
// Rather than reimplementing any particular site's URL scheme, we call the
// page's fetch through a MAIN-world bridge (public/dirplayer-fetch-bridge.js).
// Whatever the page does to the request, we inherit — including future changes
// to it.
//
// Only pages that actually replaced `fetch` are routed. Everything else keeps
// the native path, which matters: a content script's fetch is subject to the
// page's CORS, but the extension relaxes that for player tabs via
// declarativeNetRequest, and the page's fetch has no such help. For the same
// reason a bridged request that fails falls back to the native one.

const TAG = 'dirplayer-fetch-bridge';
const PROBE_TIMEOUT_MS = 1000;
const REQUEST_TIMEOUT_MS = 60000;

let nextId = 1;
let patchedProbe: Promise<boolean> | null = null;

interface BridgeResponse {
  ok: boolean;
  status: number;
  statusText?: string;
  contentType?: string;
  buffer?: ArrayBuffer;
  error?: string;
}

/** Await one message from the bridge matching `id`, or resolve null on timeout. */
function awaitReply<T>(id: number, kind: string, timeoutMs: number): Promise<T | null> {
  return new Promise((resolve) => {
    const timer = window.setTimeout(() => {
      window.removeEventListener('message', onMessage);
      resolve(null);
    }, timeoutMs);

    function onMessage(event: MessageEvent) {
      if (event.source !== window) return;
      const msg = event.data;
      if (!msg || msg.__dirplayer !== TAG || msg.kind !== kind || msg.id !== id) return;
      clearTimeout(timer);
      window.removeEventListener('message', onMessage);
      resolve(msg as T);
    }

    window.addEventListener('message', onMessage);
  });
}

/**
 * Has the page replaced `fetch`? Probed lazily on first use — a page installs
 * its rewrite after document_start, so probing at startup would always say no —
 * and cached, since a page that rewrites fetch does it once.
 */
function isPageFetchPatched(): Promise<boolean> {
  if (patchedProbe) return patchedProbe;
  patchedProbe = (async () => {
    // The bridge marks the shared DOM when it installs. Without the marker
    // there is nobody to answer — Firefox's manifest registers no MAIN-world
    // script — so skip the probe rather than waiting out its timeout on the
    // first request of every movie.
    if (!document.documentElement.hasAttribute('data-dirplayer-fetch-bridge')) {
      return false;
    }
    const id = nextId++;
    window.postMessage({ __dirplayer: TAG, kind: 'probe', id }, '*');
    const reply = await awaitReply<{ patched: boolean }>(id, 'probe-result', PROBE_TIMEOUT_MS);
    // No bridge (not injected, or a page that blocked it) — stay native.
    const patched = reply?.patched === true;
    if (patched) {
      console.log('[DirPlayer] Page installs a fetch rewrite — routing player requests through it');
    }
    return patched;
  })();
  return patchedProbe;
}

async function fetchViaPage(
  url: string,
  method: string,
  headers: Record<string, string> | undefined,
  body: string | undefined,
): Promise<Response | null> {
  const id = nextId++;
  window.postMessage({ __dirplayer: TAG, kind: 'request', id, url, method, headers, body }, '*');
  const reply = await awaitReply<BridgeResponse>(id, 'response', REQUEST_TIMEOUT_MS);
  if (!reply || reply.error || !reply.buffer) return null;
  return new Response(reply.buffer, {
    status: reply.status,
    statusText: reply.statusText || '',
    headers: reply.contentType ? { 'content-type': reply.contentType } : undefined,
  });
}

export function installPageFetchBridge() {
  const nativeFetch = window.fetch.bind(window);

  window.fetch = async function (input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
    let url: string;
    let method = init?.method || 'GET';
    let body: string | undefined;

    if (typeof input === 'string') {
      url = input;
    } else if (input instanceof URL) {
      url = input.href;
    } else {
      // vm-rust builds a Request object and passes it as the sole argument.
      url = input.url;
      method = input.method || method;
      if (input.method && input.method !== 'GET' && input.method !== 'HEAD') {
        try {
          body = await input.clone().text();
        } catch {
          body = undefined;
        }
      }
    }

    // Extension-local resources (wasm, fonts, the xtra registry) must never
    // leave the isolated world — the page has no access to chrome-extension://
    // and rewriting them would be meaningless.
    if (!/^https?:/i.test(url)) {
      return nativeFetch(input as RequestInfo, init);
    }

    if (await isPageFetchPatched()) {
      if (typeof body === 'undefined' && init?.body != null && typeof init.body === 'string') {
        body = init.body;
      }
      const viaPage = await fetchViaPage(
        url,
        method,
        init?.headers as Record<string, string> | undefined,
        body,
      );
      if (viaPage) return viaPage;
      console.warn(`[DirPlayer] Page fetch failed for ${url} — retrying natively`);
    }

    return nativeFetch(input as RequestInfo, init);
  };
}
