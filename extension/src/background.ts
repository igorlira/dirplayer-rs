// Service worker — registers a `world: "MAIN"` content script that
// installs the fake `Shockwave for Director` plugin into the page's
// `navigator.plugins` at document_start. Going through
// chrome.scripting.registerContentScripts (instead of declaring the
// content script in the manifest) bypasses CSP restrictions: pages
// with strict `script-src` directives block the manifest-declared
// inline / extension-URL scripts but cannot block the scripting API.
//
// The polyfill source is a plain JS file in `public/` (copied as-is
// to the extension root by Vite). Modeled after Ruffle's
// `web/packages/extension/src/background.ts` strategy.

const POLYFILL_SCRIPT_ID = 'dirplayer-shockwave-plugin-polyfill';
const POLYFILL_SCRIPT_FILE = 'dirplayer-shockwave-polyfill.js';
const PREINIT_SCRIPT_ID = 'dirplayer-pre-init';
const RUFFLE_SCRIPT_ID = 'dirplayer-ruffle-bundle';
const RUFFLE_SCRIPT_FILE = 'ruffle/dirplayer_ruffle.js';

async function getRegistered(): Promise<Set<string>> {
  try {
    const existing = await chrome.scripting.getRegisteredContentScripts({
      ids: [POLYFILL_SCRIPT_ID, PREINIT_SCRIPT_ID, RUFFLE_SCRIPT_ID],
    });
    return new Set(existing.map((s) => s.id));
  } catch {
    return new Set();
  }
}

async function ensureRegistered(): Promise<void> {
  if (!chrome.scripting) {
    console.warn('[DirPlayer] chrome.scripting API not available');
    return;
  }
  const registered = await getRegistered();
  const scripts: chrome.scripting.RegisteredContentScript[] = [];

  // Pre-init (ISOLATED world) — runs FIRST, stamps the chrome-extension
  // URL on `<html data-dirplayer-ruffle-url="...">` so the main-world
  // Ruffle bundle (registered below) can pick it up as
  // `__webpack_public_path__`. Registering it via the same scripting
  // API call as Ruffle (and listing it first) gives us a much more
  // reliable ordering than a separate manifest content_scripts entry,
  // which the previous attempt used and saw racing the dynamic
  // registration.
  if (!registered.has(PREINIT_SCRIPT_ID)) {
    scripts.push({
      id: PREINIT_SCRIPT_ID,
      js: ['dirplayer-pre-init.js'],
      matches: ['<all_urls>'],
      runAt: 'document_start',
      allFrames: true,
      world: 'ISOLATED',
      persistAcrossSessions: true,
    });
  }

  // Plugin polyfill — injected into the page's MAIN world so detection
  // scripts find the fake `Shockwave for Director` entry. Going through
  // chrome.scripting bypasses the page's CSP `script-src` restrictions.
  if (
    !registered.has(POLYFILL_SCRIPT_ID) &&
    chrome.scripting.ExecutionWorld &&
    chrome.scripting.ExecutionWorld.MAIN
  ) {
    scripts.push({
      id: POLYFILL_SCRIPT_ID,
      js: [POLYFILL_SCRIPT_FILE],
      matches: ['<all_urls>'],
      runAt: 'document_start',
      allFrames: true,
      world: 'MAIN',
      persistAcrossSessions: true,
    });
  }

  // Ruffle fork — injected into the MAIN world. The previous attempt
  // registered Ruffle in the isolated world (so dirplayer in the same
  // world could call `.newest()` directly), but Chrome MV3 isolated
  // worlds expose a null `customElements`, which breaks Ruffle's
  // `customElements.define(...)` registration of its player element.
  // Main world has a working CustomElementRegistry; the isolated-world
  // dirplayer talks to Ruffle there via a postMessage bridge planted
  // alongside Ruffle (extension/src/main-world-ruffle-bridge.js, copied
  // through public/).
  if (!registered.has(RUFFLE_SCRIPT_ID) && chrome.scripting.ExecutionWorld?.MAIN) {
    scripts.push({
      id: RUFFLE_SCRIPT_ID,
      js: [RUFFLE_SCRIPT_FILE, 'dirplayer-ruffle-bridge-host.js'],
      matches: ['<all_urls>'],
      runAt: 'document_start',
      allFrames: true,
      world: 'MAIN',
      persistAcrossSessions: true,
    });
  }

  if (scripts.length > 0) {
    try {
      await chrome.scripting.registerContentScripts(scripts);
    } catch (e) {
      console.warn('[DirPlayer] failed to register content scripts:', e);
    }
  }
}

// Cross-origin fetch proxy for the isolated-world content script. In MV3 a
// content-script `fetch()` is page-privileged (subject to the page's CORS), so
// `host_permissions` does NOT let it read cross-origin responses — but the
// service worker (with host_permissions <all_urls>) can. Neopets' DGS loads its
// game SWF (ml_maraqua.swf) and other assets from cross-origin neopets hosts;
// flashPlayerManager's fetch shim routes those here. Bytes are base64-framed
// because chrome.runtime messaging JSON-serializes payloads.
interface CorsFetchRequest {
  type: 'dirplayer-cors-fetch';
  url: string;
  method?: string;
  headers?: Record<string, string>;
  body?: string; // base64
}

chrome.runtime.onMessage.addListener((msg: CorsFetchRequest, _sender, sendResponse) => {
  if (!msg || msg.type !== 'dirplayer-cors-fetch') return; // not ours — let others handle
  (async () => {
    try {
      const body = msg.body
        ? Uint8Array.from(atob(msg.body), (c) => c.charCodeAt(0))
        : undefined;
      const res = await fetch(msg.url, {
        method: msg.method || 'GET',
        headers: msg.headers,
        body,
        credentials: 'omit',
      });
      const buf = new Uint8Array(await res.arrayBuffer());
      let bin = '';
      const CHUNK = 0x8000;
      for (let i = 0; i < buf.length; i += CHUNK) {
        bin += String.fromCharCode.apply(
          null,
          buf.subarray(i, i + CHUNK) as unknown as number[],
        );
      }
      sendResponse({
        ok: res.ok,
        status: res.status,
        statusText: res.statusText,
        contentType: res.headers.get('content-type') || '',
        bodyBase64: btoa(bin),
      });
    } catch (e) {
      sendResponse({ ok: false, status: 0, error: String((e as Error)?.message || e) });
    }
  })();
  return true; // keep the message channel open for the async sendResponse
});

// CORS relaxation for Ruffle's MAIN-world SWF loads. Ruffle runs in the page's
// main world (no extension privileges), so its cross-origin asset fetches
// (Neopets' swf.neopets.com dgs_include_v2.swf / game SWFs) are CORS-blocked.
// Rather than monkey-patching the page's window.fetch (which reads as page
// tampering to store reviewers), add the CORS response header declaratively via
// declarativeNetRequest — an inspectable, Google-endorsed API. The rule is
// scoped to the media/plugin/fetch resource types an emulator actually loads,
// not arbitrary API traffic. Content-script (isolated-world) fetches keep going
// through the background message proxy above; this rule is what unblocks the
// main-world Ruffle player. `Access-Control-Allow-Origin: *` is safe here
// because these are un-credentialed asset GETs.
const CORS_DNR_RULE_ID = 2001;

async function ensureCorsRules(): Promise<void> {
  const dnr = (chrome as unknown as { declarativeNetRequest?: {
    updateDynamicRules?: (opts: unknown) => Promise<void>;
  } }).declarativeNetRequest;
  if (!dnr?.updateDynamicRules) return;
  try {
    await dnr.updateDynamicRules({
      removeRuleIds: [CORS_DNR_RULE_ID],
      addRules: [{
        id: CORS_DNR_RULE_ID,
        priority: 1,
        action: {
          type: 'modifyHeaders',
          responseHeaders: [
            { header: 'Access-Control-Allow-Origin', operation: 'set', value: '*' },
          ],
        },
        condition: {
          urlFilter: '*',
          resourceTypes: ['media', 'object', 'xmlhttprequest', 'other', 'sub_frame'],
        },
      }],
    });
  } catch (e) {
    console.warn('[DirPlayer] failed to install CORS DNR rule:', e);
  }
}

chrome.runtime.onInstalled.addListener(() => { void ensureRegistered(); void ensureCorsRules(); });
chrome.runtime.onStartup.addListener(() => { void ensureRegistered(); void ensureCorsRules(); });
// Also register on service-worker activation in case the listeners above
// missed (e.g. extension was loaded mid-session via reload).
void ensureRegistered();
void ensureCorsRules();

// Note: the chrome-extension URL needed by Ruffle's chunk loader gets
// stamped onto `<html data-dirplayer-ruffle-url="...">` by an isolated-
// world content script (extension/src/dirplayer-pre-init.ts) at
// document_start. Both Ruffle's webpack entry and the bridge host
// read that attribute. Going through a DOM attribute (rather than
// chrome.scripting.executeScript via webNavigation) eliminates the
// race where the publicPath setter sometimes fired AFTER Ruffle had
// already parsed and chunks loaded relative to the page URL.
