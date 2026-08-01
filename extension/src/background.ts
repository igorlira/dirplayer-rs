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
      // The fetch bridge rides along in the same MAIN-world registration: it
      // lets the isolated-world player borrow the page's `fetch`, which archive
      // front-ends replace to serve content under its original URL.
      js: [POLYFILL_SCRIPT_FILE, 'dirplayer-fetch-bridge.js'],
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
  void _sender;
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
// declarativeNetRequest — an inspectable, Google-endorsed API. Content-script
// (isolated-world) fetches keep going through the background message proxy
// above; this rule is what unblocks the main-world Ruffle player.
//
// CRITICAL: the rule is scoped to tabs that have actually mounted a DirPlayer
// player. It used to be installed globally over `urlFilter: '*'`, which
// rewrote `Access-Control-Allow-Origin` on every matching response in every
// tab. That breaks any site whose own requests are credentialed: the CORS spec
// forbids the wildcard when a request's credentials mode is `include`, so
// overwriting a site's correct `Access-Control-Allow-Origin: <origin>` with `*`
// turns a working response into a blocked one. YouTube was the reported
// casualty — its `videoplayback` fetches are credentialed, so videos loaded
// forever until the extension was disabled:
//
//   The value of the 'Access-Control-Allow-Origin' header in the response must
//   not be the wildcard '*' when the request's credentials mode is 'include'.
//
// Tab scoping means a page with no Shockwave content is never touched.
const CORS_DNR_RULE_ID = 2001;

/** Tabs with at least one mounted player. */
const corsTabs = new Set<number>();

interface DnrApi {
  updateDynamicRules?: (opts: unknown) => Promise<void>;
  updateSessionRules?: (opts: unknown) => Promise<void>;
}
function getDnr(): DnrApi | undefined {
  return (chrome as unknown as { declarativeNetRequest?: DnrApi }).declarativeNetRequest;
}

/**
 * Rebuild the session rule from `corsTabs`. Session rules (unlike dynamic ones)
 * are torn down with the browser session and support `condition.tabIds`.
 */
async function syncCorsRule(): Promise<void> {
  const dnr = getDnr();
  if (!dnr?.updateSessionRules) return;
  const tabIds = [...corsTabs];

  const action = {
    type: 'modifyHeaders',
    responseHeaders: [
      { header: 'Access-Control-Allow-Origin', operation: 'set', value: '*' },
    ],
  };
  const baseCondition = {
    urlFilter: '*',
    resourceTypes: ['media', 'object', 'xmlhttprequest', 'other', 'sub_frame'],
    tabIds,
  };

  // Only touch responses that DON'T already carry an Access-Control-Allow-Origin
  // header. A request's credentials mode is invisible to us — declarativeNetRequest
  // has no condition for it, and MV3 removed blocking webRequest — but skipping
  // responses that already answered the CORS question gets the same protection
  // where it matters: the failure mode is us OVERWRITING a server's correct
  // `Access-Control-Allow-Origin: <origin>` with `*`, which the spec forbids for
  // a credentialed request. YouTube's videoplayback responses carry that header,
  // so this alone would have prevented the breakage.
  //
  // Responses with no such header are ones the page could not read anyway, so
  // setting the wildcard there can only help (a credentialed request to such a
  // server was already blocked, header or not).
  //
  // `excludedResponseHeaders` needs Chrome 128+; older Chrome and Firefox reject
  // the rule outright, so fall back to the tab-scoped rule alone.
  const strictRule = {
    id: CORS_DNR_RULE_ID,
    priority: 1,
    action,
    condition: {
      ...baseCondition,
      excludedResponseHeaders: [{ header: 'access-control-allow-origin' }],
    },
  };
  const fallbackRule = { id: CORS_DNR_RULE_ID, priority: 1, action, condition: baseCondition };

  if (tabIds.length === 0) {
    try {
      await dnr.updateSessionRules({ removeRuleIds: [CORS_DNR_RULE_ID], addRules: [] });
    } catch (e) {
      console.warn('[DirPlayer] failed to clear CORS DNR rule:', e);
    }
    return;
  }

  try {
    await dnr.updateSessionRules({ removeRuleIds: [CORS_DNR_RULE_ID], addRules: [strictRule] });
    return;
  } catch {
    /* excludedResponseHeaders unsupported — fall through */
  }
  try {
    await dnr.updateSessionRules({ removeRuleIds: [CORS_DNR_RULE_ID], addRules: [fallbackRule] });
  } catch (e) {
    console.warn('[DirPlayer] failed to sync CORS DNR rule:', e);
  }
}

/**
 * Drop the pre-existing GLOBAL dynamic rule. Dynamic rules persist across
 * extension updates, so without this an upgraded install would keep rewriting
 * headers browser-wide even though nothing adds that rule any more.
 */
async function removeLegacyGlobalCorsRule(): Promise<void> {
  const dnr = getDnr();
  if (!dnr?.updateDynamicRules) return;
  try {
    await dnr.updateDynamicRules({ removeRuleIds: [CORS_DNR_RULE_ID], addRules: [] });
  } catch (e) {
    console.warn('[DirPlayer] failed to remove legacy CORS DNR rule:', e);
  }
}

// A player mounted in this tab — from here on its cross-origin asset loads need
// the CORS header. Sent by the content script on first mount (see
// polyfill/src/core.tsx renderPlayer).
chrome.runtime.onMessage.addListener((msg: { type?: string }, sender, sendResponse) => {
  if (!msg || msg.type !== 'dirplayer-player-mounted') return;
  const tabId = sender.tab?.id;
  if (typeof tabId === 'number' && !corsTabs.has(tabId)) {
    corsTabs.add(tabId);
    void syncCorsRule();
  }
  sendResponse({ ok: true });
  return true;
});

function forgetTab(tabId: number): void {
  if (corsTabs.delete(tabId)) void syncCorsRule();
}
chrome.tabs?.onRemoved?.addListener((tabId) => forgetTab(tabId));
// A tab navigating away is no longer running a player until it says so again;
// the content script re-activates per document.
chrome.tabs?.onUpdated?.addListener((tabId, changeInfo) => {
  if (changeInfo.status === 'loading') forgetTab(tabId);
});

chrome.runtime.onInstalled.addListener(() => {
  void ensureRegistered();
  void removeLegacyGlobalCorsRule();
});
chrome.runtime.onStartup.addListener(() => {
  void ensureRegistered();
  void removeLegacyGlobalCorsRule();
});
// Also register on service-worker activation in case the listeners above
// missed (e.g. extension was loaded mid-session via reload).
void ensureRegistered();
void removeLegacyGlobalCorsRule();

// Note: the chrome-extension URL needed by Ruffle's chunk loader gets
// stamped onto `<html data-dirplayer-ruffle-url="...">` by an isolated-
// world content script (extension/src/dirplayer-pre-init.ts) at
// document_start. Both Ruffle's webpack entry and the bridge host
// read that attribute. Going through a DOM attribute (rather than
// chrome.scripting.executeScript via webNavigation) eliminates the
// race where the publicPath setter sometimes fired AFTER Ruffle had
// already parsed and chunks loaded relative to the page URL.
