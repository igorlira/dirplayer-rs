/**
 * FlashPlayerManager - Bridges Ruffle (Flash player) with dirplayer-rs
 *
 * Manages Ruffle player instances for Flash cast members, reads rendered frames,
 * and sends pixel data to dirplayer's WASM rendering pipeline so Flash content
 * can be composited with Director sprites (Director sprites can layer on top).
 */ 

import { update_flash_frame, trigger_lingo_callback_on_script, dispatch_flash_event, dispatch_flash_lingo, local_connection_send } from 'vm-rust';
import {
  isBridgeRequired,
  waitForBridge,
  bridgeCreatePlayer,
  bridgeFindElement,
  bridgeCallMethod,
  bridgeDestroyPlayer,
  bridgeGetVariableSync,
  bridgeSetVariableSync,
  bridgeCallFunctionSync,
  bridgeCallMethodSync,
  bridgeOnEvent,
  bridgeRegisterCallbackForwarders,
} from './ruffleBridgeClient';

interface FlashInstance {
  spriteNum: number;     // Director sprite number this instance belongs to
  castLib: number;       // SWF source cast member (diagnostics + cleanup)
  castMember: number;
  rufflePlayer: any; // RufflePlayerElement (direct) or stub element (bridge mode)
  bridgeId: string | null; // Set when this instance is driven by the main-world bridge
  container: HTMLDivElement;
  canvas: HTMLCanvasElement | null;
  width: number;   // current render (canvas) width in px; tracked by setFlashSize
  height: number;
  nativeW: number; // SWF native stage size (detail floor for resizes); 0 if unknown
  nativeH: number;
  animFrameId: number | null;
  /// Becomes true only after the SWF has loaded AND the 3s AS init wait
  /// has elapsed AND the inheritance/queue replay has finished. Lingo
  /// calls (goTo / play / stop / rewind) that arrive before this is
  /// true are queued instead of going to the half-initialised player.
  ready: boolean;
  /// Mirrors the Director Flash member property of the same name.
  /// When true we pass `autoplay: 'off'` to Ruffle's loadConfig (so
  /// the SWF stays parked at frame 1 instead of running), AND the
  /// queue flush re-fires any queued `play` op — because nothing is
  /// playing automatically and Lingo's `play(sprite)` is required to
  /// actually start the SWF. When false, autoplay covers it and the
  /// queued `play` op is a redundant restart we skip.
  pausedAtStart: boolean;
  /// Director-intent "is this sprite's Flash movie stopped?" flag. We stop a
  /// Flash sprite by halting its root TIMELINE (not by suspending the whole
  /// Ruffle player — that would kill the render loop and strand later
  /// GotoFrame calls on a stale canvas). Because the player keeps running,
  /// its own `isPlaying` no longer reflects the movie's stopped state, so we
  /// track it here for `sprite.playing`. Set true by stop/rewind/frame-setter,
  /// false by play/gotoFrame.
  stopped?: boolean;
}

// Per-sprite Flash instance map. Each Flash sprite gets its own Ruffle
// player so multiple sprites that share a single Flash cast member can
// display different frames simultaneously (e.g. storyscramble's 3 story
// tiles all use cast 2:1 but show poster frames 2/4/6).
const instances = new Map<string, FlashInstance>();

// Track pending Flash instance creations so the WASM frame loop can wait for them
let flashLoadingCount = 0;

// Track when getVariable/callFunction is called on a non-existent instance.
// This signals the frame loop to wait — the rendering loop will dispatch
// the Flash member creation shortly after.
let flashAccessBeforeReady = false;

// Intercept fetch to rewrite URLs based on fetchRewriteRules config.
// On the server (empty rules), no rewriting happens — webserver should handle proxying.
function getFetchRewriteRules(): Array<{pathPrefix: string, targetHost: string, targetPort: string, targetProtocol: string}> {
  const win = window as any;
  if (win.__dirplayerFlashConfig?.fetchRewriteRules) {
    return win.__dirplayerFlashConfig.fetchRewriteRules;
  }
  // Production: host page provides rewrite rules via
  // `__dirplayerFlashConfig.fetchRewriteRules` (set by `configureFlash`).
  // Without any, fetch goes direct to the original URL.
  return [];
}

function applyFetchRewrite(url: URL): boolean {
  const rules = getFetchRewriteRules();
  for (const rule of rules) {
    if (url.pathname.startsWith(rule.pathPrefix)) {
      url.hostname = rule.targetHost;
      url.port = rule.targetPort;
      url.protocol = rule.targetProtocol;
      return true;
    }
  }
  return false;
}

// Generic CORS proxy for "loader mode" (debugging a Shockwave game from its live
// site). When `__dirplayerFlashConfig.corsProxy` is set to a base like
// "http://127.0.0.1:3099/cors?url=", any CROSS-ORIGIN http(s) fetch is rewritten
// to `<base><encoded url>` so the dev CORS proxy (cors-proxy.cjs) can fetch it
// server-side and re-serve it with CORS. Opt-in: with no corsProxy configured
// this returns null and fetch behaves exactly as before. Same-origin requests
// (the dev app's own assets/xtras) and already-proxied URLs are left untouched.
function getCorsProxyBase(): string | null {
  const base = (window as any).__dirplayerFlashConfig?.corsProxy;
  return typeof base === 'string' && base ? base : null;
}

function maybeCorsProxy(urlStr: string): string | null {
  const base = getCorsProxyBase();
  if (!base) return null;
  let u: URL;
  try { u = new URL(urlStr, window.location.origin); } catch { return null; }
  if (u.protocol !== 'http:' && u.protocol !== 'https:') return null;
  if (u.origin === window.location.origin) return null; // same-origin: leave alone
  if (urlStr.startsWith(base)) return null;              // already proxied
  try { if (new URL(base, window.location.origin).host === u.host) return null; } catch { /* */ }
  return base + encodeURIComponent(u.toString());
}

// Mixed Content upgrade: a Director movie served over HTTPS (Neopets' DGS loader
// at https://www.neopets.com/games/dgs/play_shockwave.phtml) still fetches its
// game data over a hardcoded http:// URL
// (http://www.neopets.com/games/dgs/dgs_get_game_data.phtml). Chrome blocks
// active mixed content outright, so the fetch never completes and the loader
// loops retrying. The resource is served over https on the same host, so upgrade
// the scheme. Never touch localhost/127.0.0.1 — the dev proxies are deliberately
// http and are matched by applyFetchRewrite/maybeCorsProxy by hostname regardless
// of scheme. Returns true if the URL was changed. No-op when the page itself is
// http (no mixed-content restriction) or the request is already https.
function upgradeInsecureUrl(url: URL): boolean {
  if (
    window.location.protocol === 'https:' &&
    url.protocol === 'http:' &&
    url.hostname !== 'localhost' &&
    url.hostname !== '127.0.0.1'
  ) {
    url.protocol = 'https:';
    return true;
  }
  return false;
}

// Cross-origin fetch proxy (MV3 extension). A content-script `fetch()` is
// page-privileged, so a cross-origin request (Neopets' game SWF lives on
// swf.neopets.com while the loader page is www.neopets.com) is CORS-blocked even
// though the extension has host_permissions. The service worker CAN read those
// responses, so we relay cross-origin requests through it. Standalone/electron
// (no chrome.runtime) and same-origin / localhost-proxy requests stay direct.
// `chrome` isn't in the app's TS lib; access it structurally.
const extChrome = (globalThis as unknown as {
  chrome?: { runtime?: { id?: string; sendMessage?: (msg: unknown) => Promise<unknown> } };
}).chrome;

function isExtensionContext(): boolean {
  return !!extChrome?.runtime?.id;
}

function shouldProxyCrossOrigin(url: URL): boolean {
  if (!isExtensionContext()) return false;
  try {
    if (url.origin === window.location.origin) return false; // same-origin: no CORS
  } catch {
    return false;
  }
  if (url.hostname === 'localhost' || url.hostname === '127.0.0.1') return false; // dev proxy handles its own CORS
  return url.protocol === 'http:' || url.protocol === 'https:';
}

function bytesToBase64(bytes: Uint8Array): string {
  let bin = '';
  const CHUNK = 0x8000;
  for (let i = 0; i < bytes.length; i += CHUNK) {
    bin += String.fromCharCode.apply(null, bytes.subarray(i, i + CHUNK) as unknown as number[]);
  }
  return btoa(bin);
}

function base64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

function bodyInitToBytes(body: BodyInit): Uint8Array | undefined {
  if (typeof body === 'string') return new TextEncoder().encode(body);
  if (body instanceof Uint8Array) return body;
  if (body instanceof ArrayBuffer) return new Uint8Array(body);
  if (ArrayBuffer.isView(body)) return new Uint8Array(body.buffer, body.byteOffset, body.byteLength);
  return undefined; // Blob / FormData / URLSearchParams — not used by our asset fetches
}

function headersToObject(h?: HeadersInit): Record<string, string> | undefined {
  if (!h) return undefined;
  const out: Record<string, string> = {};
  if (h instanceof Headers) h.forEach((v, k) => { out[k] = v; });
  else if (Array.isArray(h)) h.forEach(([k, v]) => { out[k] = v; });
  else Object.assign(out, h);
  return out;
}

interface BgFetchResult {
  ok?: boolean;
  status?: number;
  statusText?: string;
  contentType?: string;
  bodyBase64?: string;
  error?: string;
}

async function corsFetchRawViaBackground(
  url: string,
  method: string,
  headers?: Record<string, string>,
  bodyBytes?: Uint8Array,
): Promise<BgFetchResult> {
  const resp = await extChrome!.runtime!.sendMessage!({
    type: 'dirplayer-cors-fetch',
    url,
    method,
    headers,
    body: bodyBytes && bodyBytes.byteLength ? bytesToBase64(bodyBytes) : undefined,
  }) as BgFetchResult | undefined;
  return resp || { error: 'no response from background' };
}

async function corsFetchViaBackground(
  url: string,
  method: string,
  headers?: Record<string, string>,
  bodyBytes?: Uint8Array,
): Promise<Response> {
  const resp = await corsFetchRawViaBackground(url, method, headers, bodyBytes);
  if (resp.error) throw new Error('cors-fetch: ' + resp.error);
  const bytes = base64ToBytes(resp.bodyBase64 || '');
  const status = resp.status && resp.status >= 200 ? resp.status : 200;
  return new Response(bytes as unknown as BodyInit, {
    status,
    statusText: resp.statusText || '',
    headers: resp.contentType ? { 'content-type': resp.contentType } : undefined,
  });
}

const origFetch = window.fetch;
// Count outstanding browser fetches so dirplayer's Rust frame loop can lengthen
// its per-frame yield while any request is in flight — including Ruffle-side
// requests (LoadVars/URLLoader/XML) that dirplayer's own net_manager never sees.
// The DGS loader spins a tight high-tempo frame loop (puppetTempo(999)) waiting
// on `preloaderTranslationSuccess`, which is only set once the preloader's
// `LoadVars` POST to gettranslationxml.phtml (1-3 s) completes; without a yield
// the loop starves that fetch's completion callback and the login links never
// resolve. Tracked as a window global (read from Rust via Reflect).
let pendingNetCount = 0;
function trackFetch(p: Promise<Response>): Promise<Response> {
  pendingNetCount += 1;
  (window as any).__dirplayerPendingNetCount = pendingNetCount;
  return p.finally(() => {
    pendingNetCount -= 1;
    (window as any).__dirplayerPendingNetCount = pendingNetCount;
  });
}
window.fetch = function(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  if (typeof input === 'string') {
    try {
      const url = new URL(input, window.location.origin);
      const upgraded = upgradeInsecureUrl(url);
      if (applyFetchRewrite(url)) {
        input = url.toString();
      } else {
        const proxied = maybeCorsProxy(url.toString());
        if (proxied) input = proxied;
        else if (upgraded) input = url.toString();
      }
      // Cross-origin in the MV3 extension: relay through the service worker,
      // which (unlike this page-privileged content script) can read the
      // response. swf.neopets.com game SWFs hit this from www.neopets.com.
      const finalUrl = new URL(input as string, window.location.origin);
      if (shouldProxyCrossOrigin(finalUrl)) {
        const method = (init?.method || 'GET').toUpperCase();
        const bodyBytes = init?.body ? bodyInitToBytes(init.body) : undefined;
        return trackFetch(corsFetchViaBackground(
          finalUrl.toString(), method, headersToObject(init?.headers), bodyBytes,
        ));
      }
    } catch { /* ignore parse errors */ }
  } else if (input instanceof Request) {
    try {
      const url = new URL(input.url);
      const upgraded = upgradeInsecureUrl(url);
      const rewritten = applyFetchRewrite(url)
        ? url.toString()
        : (maybeCorsProxy(url.toString()) || (upgraded ? url.toString() : null));
      const finalUrl = rewritten || url.toString();
      const crossOrigin = shouldProxyCrossOrigin(new URL(finalUrl, window.location.origin));
      if (rewritten || crossOrigin) {
        const req = input;
        return trackFetch(req.arrayBuffer().then(bodyBuf => {
          if (crossOrigin) {
            const bodyBytes = bodyBuf.byteLength > 0 ? new Uint8Array(bodyBuf) : undefined;
            return corsFetchViaBackground(
              finalUrl, req.method, headersToObject(req.headers), bodyBytes,
            );
          }
          const newInit: RequestInit = {
            method: req.method,
            headers: req.headers,
            body: bodyBuf.byteLength > 0 ? bodyBuf : undefined,
            mode: 'cors' as RequestMode,
            credentials: 'omit' as RequestCredentials,
          };
          return origFetch.call(window, finalUrl, newInit);
        }));
      }
    } catch (e) { console.error('[fetch-intercept] Error:', e); }
  }
  return trackFetch(origFetch.call(window, input, init));
};

// Monkey-patch HTMLCanvasElement.getContext to force preserveDrawingBuffer: true
// for WebGL contexts. This is needed so we can read pixels back from Ruffle's
// wgpu-webgl canvas after the frame is presented.
//
// EXCEPTION: dirplayer's own stage canvas is marked with `data-dp-stage` (set in
// Rust before it requests its WebGL2 context). preserveDrawingBuffer keeps a full
// extra drawing buffer alive — pointless for the stage (we never read it back via
// this path) and it raises GPU memory, which makes the browser more likely to
// drop the stage context when the tab is backgrounded. So skip the stage canvas.
const origGetContext = HTMLCanvasElement.prototype.getContext;
(HTMLCanvasElement.prototype as any).getContext = function(type: string, attrs?: any) {
  const isStage = (this as HTMLElement)?.getAttribute?.('data-dp-stage') === '1';
  if ((type === 'webgl' || type === 'webgl2') && !isStage) {
    attrs = { ...(attrs || {}), preserveDrawingBuffer: true };
  }
  return origGetContext.call(this, type, attrs);
};

function getSocketProxyConfig(): Array<{host: string, port: number, proxyUrl: string}> {
  const win = window as any;
  return (win.__dirplayerFlashConfig?.socketProxy as
    | Array<{ host: string; port: number; proxyUrl: string }>
    | undefined) ?? [];
}

// Install the global socket-URL resolver immediately so the WASM-side
// Multiuser Xtra (which calls `window.dirplayerResolveSocketUrl`) finds
// it even when the host page doesn't call `configureFlashManager`.
// Re-evaluates `getSocketProxyConfig` on every call so dev defaults +
// runtime-supplied entries both stay live.
(window as any).dirplayerResolveSocketUrl = (host: string, port: number): string => {
  const proxies = getSocketProxyConfig();
  for (const entry of proxies) {
    if (entry.host.toLowerCase() === host.toLowerCase() && entry.port === port) {
      return entry.proxyUrl;
    }
  }
  return "";
};

// Each Flash sprite has its own Ruffle instance — keyed by the Director
// sprite number. Sprite numbers are unique within a movie so castLib /
// castMember don't need to be part of the key.
function instanceKey(spriteNum: number): string {
  return `${spriteNum}`;
}

// Publish the count of live Ruffle instances so dirplayer's Rust frame loop can
// floor its per-frame yield while any Flash sprite is on stage. The offscreen
// Ruffle instances self-tick via requestAnimationFrame; a tight high-tempo
// Director loop (DGS puppetTempo(999) guest-gate poll) otherwise hogs the main
// thread and starves those RAF ticks, so a text field whose htmlText was just
// updated (the preloader login links) never re-renders. Only affects movies
// running faster than ~60fps — see the yield logic in player/mod.rs.
function syncActiveFlashCount(): void {
  (window as any).__dirplayerActiveFlashCount = instances.size;
}

/**
 * Forward a Director sprite mouse event into a Ruffle player so the SWF's
 * own AS1 button handlers (`on (press)` / `on (release)`) actually run.
 *
 * Ruffle's canvas lives in a hidden container at `left: -9999px`, so real
 * browser clicks never reach it — meaning AS1 button handlers (which fire
 * on pointer events the Flash player receives) never run. Director still
 * sees the click on its stage and resolves the Flash sprite; we then
 * synthesise PointerEvents on the Ruffle canvas so the SWF processes the
 * click as if it had been delivered natively.
 *
 * Coordinates are sprite-local (origin at the sprite's top-left in stage
 * coords). They're rebased to canvas client coords here so Ruffle's input
 * code computes the correct Flash-stage position.
 */
function dispatchMouseEvent(
  spriteNum: number,
  type: 'down' | 'up' | 'move',
  localX: number,
  localY: number,
  spriteW?: number,
  spriteH?: number,
): boolean {
  const inst = instances.get(instanceKey(spriteNum));
  if (!inst) {
    // No Flash instance for this sprite (e.g. a click on a non-Flash sprite, or
    // before the SWF instance is created) — nothing to forward.
    return false;
  }

  // `localX/localY` are in the sprite's DISPLAY space (0..spriteW). The SWF renders
  // at its own native size (`canvas.width/height`), which the sprite may scale — so
  // map sprite-space → canvas internal space by the sprite dimensions (NOT the canvas
  // DOM rect, which equals the native size and would be a no-op). A 626x468 SWF shown
  // in a 600x320 sprite: local 288 → 288/600*626 ≈ 300.
  let canvasX = localX;
  let canvasY = localY;
  if (inst.canvas && spriteW && spriteH && spriteW > 0 && spriteH > 0) {
    canvasX = (localX / spriteW) * inst.canvas.width;
    canvasY = (localY / spriteH) * inst.canvas.height;
  } else if (inst.canvas) {
    const rect = inst.canvas.getBoundingClientRect();
    if (rect.width > 0 && rect.height > 0) {
      canvasX = (localX / rect.width) * inst.canvas.width;
      canvasY = (localY / rect.height) * inst.canvas.height;
    }
  }

  // Bridge mode (MV3 extension): dirplayer_dispatchPointer lives on the
  // main-world player element, invisible on the isolated-world stub. Forward it
  // through the bridge — fire-and-forget (a click's "handled" return isn't
  // consumed synchronously). Without this, Director-side clicks never reach the
  // offscreen Ruffle preloader, so DGS's "Play" button never flips `playGame`.
  if (inst.bridgeId) {
    void bridgeCallMethod(inst.bridgeId, 'dirplayer_dispatchPointer', [type, canvasX, canvasY]);
    return true;
  }

  const player = inst.rufflePlayer as
    | { dirplayer_dispatchPointer?: (t: string, x: number, y: number) => boolean }
    | undefined;
  if (typeof player?.dirplayer_dispatchPointer !== 'function') {
    return false;
  }
  const handled = !!player.dirplayer_dispatchPointer(type, canvasX, canvasY);
  return handled;
}

/**
 * Forward a Flash `event: …` URL body into Director's event chain.
 *
 * Director's Flash Asset Xtra intercepts `getURL("event: …")` (e.g. the
 * storyscramble bubble's `getURL("event: send #done", "")` at frame 11
 * and frame 21) and routes the body into the host movie's event chain.
 * Returns true if the body was understood and dispatched.
 *
 * Also exposed as `window.dirplayer_dispatchFlashEvent` so the chain can
 * be hand-fired from DevTools while debugging.
 */
export function dispatchFlashEvent(castLib: number, castMember: number, body: string): boolean {
  try {
    return dispatch_flash_event(castLib, castMember, body);
  } catch (e) {
    console.warn('[Flash] dispatchFlashEvent error:', e);
    return false;
  }
}

/**
 * Run a Flash `getURL("lingo: …")` navigation body as a Lingo command.
 *
 * Director's Flash Asset Xtra interprets a `getURL` whose URL uses the
 * `lingo:` scheme by evaluating the remainder as a Lingo command in the
 * movie's global handler context (like `do "…"`). Pengapop's titleScreen
 * SWF drives every button this way: Play → `lingo:startGameTimed`, hover
 * SFX → `lingo:bdPlaySound(#generalSound,"s_mouseOver")`, etc. The body is
 * everything after the `lingo:` prefix.
 */
export function dispatchFlashLingo(body: string): boolean {
  try {
    return dispatch_flash_lingo(body);
  } catch (e) {
    console.warn('[Flash] dispatchFlashLingo error:', e);
    return false;
  }
}

/**
 * Register an open-URL handler on a Ruffle player so `getURL("event: …")`
 * is routed into Director instead of being denied. Requires the dirplayer
 * Ruffle fork's `dirplayer_addOpenUrlHandler` patch; until it lands the
 * call is a no-op and navigations stay denied via `openUrlMode: 'deny'`.
 */
function registerEventUrlHandler(player: any, castLib: number, castMember: number): void {
  if (typeof player?.dirplayer_addOpenUrlHandler !== 'function') {
    console.warn(
      `[Flash] ${castLib}:${castMember}: dirplayer_addOpenUrlHandler missing on Ruffle player — ` +
      `event:URLs from the SWF will be denied. ` +
      `Use window.dirplayer_dispatchFlashEvent(${castLib}, ${castMember}, "send #done") to fire dispatch manually.`
    );
    return;
  }
  player.dirplayer_addOpenUrlHandler((url: string, _target: string): boolean => {
    if (typeof url !== 'string') {
      return false;
    }
    // Director's Flash Asset `lingo:` scheme — run the URL body as a Lingo
    // command (`do "…"` semantics). Pengapop's titleScreen buttons use this
    // for everything (Play → `lingo:startGameTimed`, hover SFX →
    // `lingo:bdPlaySound(...)`). Swallow the navigation either way.
    if (url.startsWith('lingo:')) {
      const body = url.slice('lingo:'.length).trim();
      const handled = dispatchFlashLingo(body);
      if (!handled) {
        console.warn(
          `[Flash] ${castLib}:${castMember}: empty/failed lingo URL body: ${JSON.stringify(body)}`
        );
      }
      return true;
    }
    if (!url.startsWith('event:')) {
      return false; // not ours — let Ruffle's openUrlMode decide
    }
    const body = url.slice('event:'.length).trim();
    const handled = dispatchFlashEvent(castLib, castMember, body);
    if (!handled) {
      console.warn(
        `[Flash] ${castLib}:${castMember}: unrecognised event URL body: ${JSON.stringify(body)}`
      );
    }
    // Always swallow the navigation: even an unrecognised event:URL must
    // not open a popup. Director silently ignores malformed event: bodies.
    return true;
  });
}

/**
 * Register an fscommand handler on a Ruffle player so a SWF's
 * `fscommand("handler", "args")` reaches Director's Lingo, matching the Flash
 * Asset Xtra's Flash→Director bridge. This is the classic channel Director
 * Flash movies use to call back into Lingo (distinct from `getURL("event:…")`).
 *
 * Neopets' DGS include movie (`objMain`) signals init readiness this way:
 * `fscommand("FlashLoaderLoaded")` → Director's movie handler
 * `on FlashLoaderLoaded` → `gMainObject.flashLoaderIsReady()`. Without this,
 * the fscommand is dropped and the DGS loader stalls at load_state 6.
 *
 * The reference (Director 11.5 Scripting Dictionary) doesn't document the
 * Xtra's fscommand→handler mapping, so this mirrors the observed contract:
 * the command name is the handler; any args string is appended so
 * dispatch_flash_event tokenises trailing args.
 */
function registerFSCommandHandler(player: any, castLib: number, castMember: number): void {
  // Prefer the fork's namespaced `dirplayer_addFSCommandHandler` (binds only to
  // our player, never a stock Ruffle sharing the page); fall back to the stock
  // `addFSCommandHandler` if an older bundle is loaded.
  const reg = player?.dirplayer_addFSCommandHandler || player?.addFSCommandHandler;
  if (typeof reg !== 'function') {
    console.warn(
      `[Flash] ${castLib}:${castMember}: dirplayer_addFSCommandHandler missing on Ruffle player — ` +
      `fscommand() calls from the SWF will be dropped.`
    );
    return;
  }
  reg.call(player, (command: string, args: string): void => {
    if (typeof command !== 'string' || !command.trim()) return;
    const body = (typeof args === 'string' && args.trim())
      ? `${command.trim()} ${args.trim()}`
      : command.trim();
    const handled = dispatchFlashEvent(castLib, castMember, body);
    if (!handled) {
      console.warn(
        `[Flash] ${castLib}:${castMember}: unhandled fscommand: ${JSON.stringify(command)} ${JSON.stringify(args)}`
      );
    }
  });
}

/**
 * Bridge-mode counterpart to registerEventUrlHandler + registerFSCommandHandler.
 * The Flash→Director callbacks (getURL("event:"/"lingo:") and fscommand) are
 * functions, which can't cross the isolated↔main world boundary. So the
 * main-world host registers its OWN handlers on the player — it decides
 * synchronously whether to claim an open-URL (for our lingo:/event: schemes) and
 * forwards the body back here via the bridge event channel, where the actual
 * Lingo dispatch runs. Neopets' DGS include movie fires `fscommand("FlashLoader
 * Loaded")` this way; without it the loader stalls at load_state 6.
 */
function registerBridgeCallbacks(bridgeId: string, castLib: number, castMember: number): void {
  bridgeOnEvent(bridgeId, (name, detail) => {
    const d = detail as { url?: string; target?: string; command?: string; args?: string } | undefined;
    if (!d) return;
    if (name === 'openUrl') {
      const url = d.url;
      if (typeof url !== 'string') return;
      if (url.startsWith('lingo:')) {
        dispatchFlashLingo(url.slice('lingo:'.length).trim());
      } else if (url.startsWith('event:')) {
        dispatchFlashEvent(castLib, castMember, url.slice('event:'.length).trim());
      }
    } else if (name === 'fsCommand') {
      const command = d.command;
      const args = d.args;
      if (typeof command !== 'string' || !command.trim()) return;
      const body = (typeof args === 'string' && args.trim())
        ? `${command.trim()} ${args.trim()}`
        : command.trim();
      dispatchFlashEvent(castLib, castMember, body);
    }
  });
  void bridgeRegisterCallbackForwarders(bridgeId);
}

/**
 * Load Ruffle library. Assumes ruffle is available at a known path or via CDN.
 * Returns the RufflePlayer constructor.
 */
let rufflePromise: Promise<any> | null = null;

async function loadRuffle(): Promise<any> {
  if (rufflePromise) return rufflePromise;

  rufflePromise = (async () => {
    // Try to get our forked Ruffle from window (loaded via script tag).
    // The selfhosted bundle installs itself under window.dirplayer_RufflePlayer
    // so we don't collide with stock Ruffle if another copy is on the page
    // (e.g. via a browser extension or another script).
    const win = window as any;
    // Test `.newest` directly — `win.dirplayer_RufflePlayer` may already
    // exist as a config-only stub planted by `installFlashShims()` before
    // the actual Ruffle bundle has loaded. The stub is truthy but has
    // no `.newest()` method, so a plain truthy check would fall through
    // to a TypeError when we try to invoke it. Treat the stub as
    // "Ruffle not yet loaded" and surface the clearer error message.
    if (typeof win.dirplayer_RufflePlayer?.newest === 'function') {
      const ruffle = win.dirplayer_RufflePlayer.newest();
      return ruffle;
    }

    throw new Error('dirplayer Ruffle fork not found. Ensure dirplayer_ruffle.js is loaded via a script tag.');
  })();

  return rufflePromise;
}

/**
 * Whether the host page has explicitly disabled Flash via
 * `__dirplayerFlashConfig.disableFlash` (set by the polyfill's
 * `data-disable-flash` attribute, or directly by the host page).
 *
 * When true, `createFlashInstance` skips Ruffle entirely. Any
 * subsequent calls into the `window.dirplayer_ruffle*` shims are still safe —
 * they early-return because no instances exist in the per-(castLib,
 * castMember) map.
 */
function isFlashDisabled(): boolean {
  const win = window as any;
  return !!win.__dirplayerFlashConfig?.disableFlash;
}

/**
 * Parse the SWF header's stage size (width, height) in pixels from the movie
 * bytes. Only uncompressed FWS is handled (returns null for CWS/ZWS). Used to
 * pick a high-resolution render canvas so a Flash sprite that is scaled UP on
 * stage (bogey_nights' boogyflash/spitflash splashes GROW as they absorb
 * others; the bogeyman arm swaps member dims) stays sharp: Ruffle renders the
 * vector at high res and dirplayer DOWNSCALES to the sprite rect, instead of
 * upscaling a tiny creation-time capture (which pixelates).
 */
function parseSwfStageSize(data: Uint8Array): { w: number; h: number } | null {
  if (data.length < 9 || data[0] !== 0x46 || data[1] !== 0x57 || data[2] !== 0x53) {
    return null; // not "FWS"
  }
  const nbits = data[8] >> 3;
  const readBits = (bitPos: number, n: number): number => {
    let v = 0;
    for (let i = 0; i < n; i++) {
      const byteIdx = (bitPos + i) >> 3;
      const bitIdx = 7 - ((bitPos + i) & 7);
      if ((data[byteIdx] >> bitIdx) & 1) v |= 1 << (n - 1 - i);
    }
    return v;
  };
  let p = 8 * 8 + 5; // byte 8, past the 5-bit nbits field
  const xMin = readBits(p, nbits); p += nbits;
  const xMax = readBits(p, nbits); p += nbits;
  const yMin = readBits(p, nbits); p += nbits;
  const yMax = readBits(p, nbits);
  const w = Math.round((xMax - xMin) / 20);
  const h = Math.round((yMax - yMin) / 20);
  if (w <= 0 || h <= 0) return null;
  return { w, h };
}

/**
 * Resize a live Ruffle instance to match the sprite's current on-stage size so
 * the vector re-renders sharp at the new scale (bogey_nights' splashes grow,
 * the bogeyman arm swaps member dims). Keeps the render ~1:1 with what dirplayer
 * draws — no blur from up-render + downscale, and no pixelation from upscaling a
 * stale small capture. Never shrinks below the SWF's native size (detail floor)
 * and no-ops for tiny changes / off-screen 3D-texture instances.
 */
function setFlashSize(spriteNum: number, w: number, h: number): void {
  if (spriteNum < 0) return; // off-screen 3D texture: fixed size
  const inst = instances.get(instanceKey(spriteNum));
  if (!inst) return;
  let tw = Math.max(1, Math.round(w));
  let th = Math.max(1, Math.round(h));
  if (inst.nativeW && inst.nativeH) {
    tw = Math.max(tw, inst.nativeW);
    th = Math.max(th, inst.nativeH);
  }
  // Skip sub-2px churn so a slowly-growing splash doesn't reflow Ruffle every
  // single frame.
  if (Math.abs(tw - inst.width) < 2 && Math.abs(th - inst.height) < 2) return;
  inst.width = tw;
  inst.height = th;
  try {
    inst.container.style.width = `${tw}px`;
    inst.container.style.height = `${th}px`;
    inst.rufflePlayer.style.width = `${tw}px`;
    inst.rufflePlayer.style.height = `${th}px`;
  } catch (e) {
    /* bridge mode / detached element */
  }
}

/**
 * Create a Ruffle player instance for a specific Flash sprite.
 * Each sprite gets its own player so multiple sprites that share a single
 * Flash cast member can display different frames simultaneously.
 */
export async function createFlashInstance(
  spriteNum: number,
  castLib: number,
  castMember: number,
  swfData: Uint8Array,
  width: number,
  height: number,
  pausedAtStart: boolean = false,
  assertedFrame: number = -1,
): Promise<void> {
  const key = instanceKey(spriteNum);

  // Skip when Flash is explicitly disabled by the host. The Lingo
  // bridge functions all early-return on missing instance, so the
  // movie won't error — Flash sprites just stay invisible / inert.
  if (isFlashDisabled()) {
    console.log(
      `[Flash] disableFlash is set; skipping Ruffle instance for ${key} ` +
      `(Lingo Flash calls will safely no-op).`
    );
    return;
  }

  // Destroy existing instance for this sprite if any.
  destroyFlashInstance(spriteNum);

  // Per-sprite frame intent is now owned by the Rust sprite
  // (`flash_asserted_frame`) and threaded in as `assertedFrame`, which we pin
  // below. The old cross-sprite "sibling frame inheritance" is gone: it seeded
  // a new instance from ANOTHER sprite sharing the member, which for
  // shared-member sets (StoryScramble's 3 story tiles) gave them all the SAME
  // frame — clobbering each tile's unique poster.

  flashLoadingCount++;
  console.log(`[Flash] Instance ${key} creation started (pending: ${flashLoadingCount})`);

  try {

  // Bridge mode: extension content scripts run in an isolated world
  // where `customElements` is null, so Ruffle (which registers a
  // custom element in `createPlayer`) cannot be invoked here directly.
  // The service worker registers Ruffle in the page's main world; we
  // talk to it via window.postMessage. See ruffleBridgeClient.ts and
  // public/dirplayer-ruffle-bridge-host.js.
  const bridgeMode = isBridgeRequired();
  let player: any;
  let bridgeId: string | null = null;

  // Render resolution = the sprite's current on-stage size, so the captured
  // frame is ~1:1 with what dirplayer draws (crisp; no bilinear softening from
  // an up-render + downscale roundtrip). As the sprite is scaled on stage
  // (bogey_nights' splashes GROW, the arm swaps dims), `setFlashSize` resizes
  // the player so Ruffle re-renders the vector sharp at the new size — matching
  // Director's vector rendering at any scale. Never render below the SWF's
  // native size, so a sprite briefly created tiny (splash at ~10px) still has
  // detail until the first resize. Off-screen 3D-texture members (negative
  // sprite number) keep their exact requested size.
  let renderW = Math.max(1, Math.round(width));
  let renderH = Math.max(1, Math.round(height));
  const native = spriteNum >= 0 ? parseSwfStageSize(swfData) : null;
  if (native) {
    renderW = Math.max(renderW, native.w);
    renderH = Math.max(renderH, native.h);
  }

  // Hidden container for Ruffle - pixels are read back and composited into dirplayer's canvas
  const container = document.createElement('div');
  container.style.position = 'absolute';
  container.style.left = '-9999px';
  container.style.top = '-9999px';
  container.style.width = `${renderW}px`;
  container.style.height = `${renderH}px`;
  container.style.overflow = 'hidden';
  document.body.appendChild(container);

  if (bridgeMode) {
    if (!(await waitForBridge())) {
      throw new Error('main-world Ruffle bridge did not become ready');
    }
    bridgeId = await bridgeCreatePlayer();
    const elem = bridgeFindElement(bridgeId);
    if (!elem) throw new Error('bridge created player but DOM element not found: ' + bridgeId);
    player = elem;
    player.style.width = `${renderW}px`;
    player.style.height = `${renderH}px`;
    container.appendChild(player);
  } else {
    // Direct mode (page-loaded polyfill, same world as Ruffle).
    const ruffle = await loadRuffle();
    player = ruffle.createPlayer();
    player.style.width = `${renderW}px`;
    player.style.height = `${renderH}px`;
    container.appendChild(player);
  }

  const instance: FlashInstance = {
    spriteNum,
    castLib,
    castMember,
    rufflePlayer: player,
    bridgeId,
    container,
    canvas: null,
    width: renderW,
    height: renderH,
    nativeW: native ? native.w : 0,
    nativeH: native ? native.h : 0,
    animFrameId: null,
    ready: false,
    pausedAtStart,
  };

  instances.set(key, instance);
  syncActiveFlashCount();

  // Copy data out of WASM memory immediately — the underlying ArrayBuffer
  // can be detached/invalidated when WASM memory grows
  const dataCopy = new Uint8Array(swfData);

  // Log first bytes for debugging
  const firstBytes = Array.from(dataCopy.slice(0, 50));
  console.log(`Flash data first 50 bytes for ${castLib}:${castMember}: [${firstBytes.join(', ')}]`);
  console.log(`Flash data as string: "${String.fromCharCode.apply(null, Array.from(dataCopy.slice(0, 20)))}"`);

  // Search for SWF signature (FWS/CWS/ZWS) in the data — Director may prepend headers
  let swfOffset = -1;
  for (let i = 0; i < dataCopy.length - 3; i++) {
    if ((dataCopy[i] === 70 || dataCopy[i] === 67 || dataCopy[i] === 90) && // F, C, or Z
        dataCopy[i + 1] === 87 && // W
        dataCopy[i + 2] === 83) { // S
      swfOffset = i;
      console.log(`Found SWF signature at offset ${i}: ${String.fromCharCode(dataCopy[i])}WS`);
      break;
    }
  }

  if (swfOffset < 0) {
    console.error(`No SWF signature (FWS/CWS/ZWS) found in Flash member ${castLib}:${castMember} data (${dataCopy.length} bytes)`);
    return;
  }
  const actualSwfData = dataCopy.slice(swfOffset);

  // Load the SWF data into Ruffle. Direct mode goes through
  // `player.ruffle().load(...)` (the selfhosted API entry point);
  // bridge mode delegates to the main-world host because chained
  // method calls like `.ruffle().load(...)` can't cross worlds.
  const loadConfig = {
    data: actualSwfData,
    allowScriptAccess: true,
    openUrlMode: 'deny',
    // Always autoplay. Tried gating this on `pausedAtStart` (so the
    // SWF would load paused at frame 1 like Director's docs suggest),
    // but Ruffle's `autoplay: 'off'` leaves the player in a preroll
    // state where `_currentframe` reads 0 and `GotoFrame(N, …)` has
    // no effect until something kicks the SWF — which broke tile
    // poster-frame pinning AND the bubble's frame-31 seed. The
    // microtask pin in `applyGotoAndPin` already keeps tiles from
    // cycling visibly, so we don't need to suppress autoplay to fix
    // that. We still track `pausedAtStart` on the instance for the
    // queue-flush `play` heuristic below.
    autoplay: 'on',
    unmuteOverlay: 'hidden',
    // Ruffle logs to console at this level. `info` (the old hardcoded value)
    // emits every AS `trace()` and unsupported-feature notice — for a busy SWF
    // that's console output every frame, and the browser RETAINS each console
    // entry (plus its arguments), so RAM climbs steadily over a long run. Honour
    // the host's configured level (`__dirplayerFlashConfig.logLevel`) and default
    // to `error` so the frame-capture loop doesn't flood the console.
    logLevel: ((window as any).__dirplayerFlashConfig?.logLevel as string) ?? 'error',
    splashScreen: false,
    // Transparent stage so SWFs that layer over other Director sprites
    // (mello's fire/marshmello/stick) composite correctly. The downside:
    // SWFs that bake their stage color in (e.g. storyscramble's chat
    // bubble's white interior) lose that fill — Director would normally
    // overlay member.bgColor + handle ink modes (especially ink 36
    // "Background Transparent") to manage transparency. Our renderer
    // takes over those responsibilities on the dirplayer side; this
    // wmode just makes the Ruffle canvas itself transparent-where-empty.
    wmode: 'transparent',
    // Force the Canvas2D backend. Ruffle's config key is `preferredRenderer`
    // (see load-options.ts RenderBackend); the old `renderer: 'canvas'` was an
    // UNKNOWN key that Ruffle silently ignored, so every instance defaulted to
    // the wgpu-webgl backend and allocated its own WebGL context. Games with
    // many simultaneous Flash sprites (bogey_nights' boogyflash/spitflash
    // "superstar" splashes) then blew past the browser's ~16 WebGL-context cap
    // ("Too many active WebGL contexts"). Canvas2D uses no WebGL context and
    // makes the getImageData frame-capture readback cheaper — which is the
    // whole reason we wanted the canvas backend in the first place.
    preferredRenderer: 'canvas',
    socketProxy: getSocketProxyConfig(),
  };
  if (bridgeId) {
    // The host's `callMethod` convention: methodName='load' resolves
    // via `player.ruffle().load(args)` on the main-world side.
    await bridgeCallMethod(bridgeId, 'load', [loadConfig]);
  } else {
    const ruffleInstance = player.ruffle();
    await ruffleInstance.load(loadConfig);
  }

  // Honour Director's `pausedAtStart` Flash member property.
  // Ruffle's own `autoplay: 'off'` leaves the SWF in a preroll state
  // where `_currentframe=0` and `GotoFrame` is a no-op, so we keep
  // autoplay on for SWF initialisation — then immediately pin the
  // MovieClip at frame 1 (rendered + stopped) so it doesn't visibly
  // cycle during the 3-second AS-init wait below. Subsequent Lingo
  // `mySprite.play()` (via playFlash) unsticks it normally.
  // Pin the initial frame BEFORE autoplay runs and before frame-capture starts,
  // so the very first captured frame is already correct. A Lingo-asserted frame
  // (`sprite.frame = N`, threaded from Rust as `assertedFrame` — it lives on the
  // SPRITE so it's correct even though this JS instance was just (re)created and
  // never saw the `frame =` op) takes PRECEDENCE over `pausedAtStart`'s frame-1:
  // StoryScramble's 3 story tiles share cast 2:1 but each must show its own
  // poster; pinning them all to frame 1 (pausedAtStart) shows the SAME picture.
  const initialPin = assertedFrame >= 0 ? assertedFrame : (pausedAtStart ? 1 : -1);
  if (initialPin >= 0) {
    try {
      // Two-step: gotoAndPlay so Ruffle paints the frame (it skips paint for an
      // already-stopped MovieClip), wait one RAF so the paint lands, then
      // gotoAndStop to halt there.
      playerExec(instance, 'GotoFrame', [initialPin, false]);
      await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
      playerExec(instance, 'GotoFrame', [initialPin, true]);
      instance.stopped = true;
    } catch (e) {
      console.warn(`[Flash] Instance ${key} initial-frame pin (${initialPin}) failed:`, e);
    }
  }

  // Director's Flash Asset Xtra intercepts `getURL("event: …")` and routes
  // the body into the host movie's event chain (e.g. `event: send #done`
  // fires the `done` handler). Register the handler speculatively — when
  // the Ruffle-fork patch is present, navigations whose URL starts with
  // `event:` get routed into dispatch_flash_event and the real open is
  // suppressed. Otherwise it's a safe no-op.
  // The other Flash→Director channel: `fscommand("handler", "args")`. DGS's
  // include movie (objMain) uses this to fire `on FlashLoaderLoaded`.
  if (bridgeId) {
    // Bridge mode (MV3 extension): the player + its handlers live in the main
    // world, and callback functions can't cross worlds — so the host registers
    // its own forwarders and posts each event/fscommand back here. Handles both
    // the event:/lingo: URL channel and fscommand in one call.
    registerBridgeCallbacks(bridgeId, castLib, castMember);
  } else {
    registerEventUrlHandler(player, castLib, castMember);
    registerFSCommandHandler(player, castLib, castMember);
  }

  // Find the internal canvas element that Ruffle renders to
  await new Promise<void>((resolve) => {
    setTimeout(() => {
      const shadow = player.shadowRoot;
      if (shadow) {
        const canvas = shadow.querySelector('canvas');
        if (canvas) instance.canvas = canvas;
      }
      if (!instance.canvas) {
        const canvas = player.querySelector('canvas');
        if (canvas) instance.canvas = canvas;
      }
      if (instance.canvas) {
        startFrameCapture(key);
      }
      resolve();
    }, 500);
  });

  // Give the SWF time to run its ActionScript initialization (ExternalInterface callbacks etc.)
  console.log(`[Flash] Instance ${key} loaded, waiting for SWF ActionScript to initialize...`);
  await new Promise(resolve => setTimeout(resolve, 3000));

  } finally {
    flashLoadingCount--;
    flashAccessBeforeReady = false;
    console.log(`[Flash] Instance ${key} fully ready (pending: ${flashLoadingCount})`);

    const live = instances.get(key);
    // Mark ready BEFORE the queue replay so the internal
    // `live.rufflePlayer.GotoFrame(...)` calls aren't seen as targeting
    // a not-yet-ready instance. After this point any Lingo goTo/play/stop
    // calls bypass the queue.
    if (live) live.ready = true;

    // Replay any beginSprite-time `gotoFrame(sprite,N)` / `play(sprite)` /
    // `stop(sprite)` Lingo calls that arrived before this instance was created.
    flushPendingGoto(spriteNum);

    // Finally, re-assert the sprite's authoritative frame (from Rust). The
    // early pin above set it before autoplay, but the 3s AS-init window +
    // flushPendingGoto may have moved the playhead; re-pinning here guarantees
    // the poster survives to `ready` (StoryScramble tiles). Skipped if a queued
    // `play`/`gotoFrame` already resumed the sprite (the flush's stopped flag
    // reflects that).
    if (assertedFrame >= 0 && live && live.stopped) {
      try {
        playerExec(live, 'GotoFrame', [assertedFrame, false]);
        await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
        playerExec(live, 'GotoFrame', [assertedFrame, true]);
      } catch (e) {
        console.warn(`[Flash] asserted-frame re-pin failed for ${key}:`, e);
      }
    }
  }
}

/**
 * Capture frames from Ruffle's canvas and send pixel data to dirplayer WASM.
 */
function startFrameCapture(key: string): void {
  const instance = instances.get(key);
  if (!instance) return;

  // Off-screen Flash members used as 3D textures use a NEGATIVE synthetic
  // sprite number (dispatched by the Rust newTexture path). These are STATIC
  // textures, so capturing every frame would run one expensive getImageData
  // GPU→CPU readback per texture per frame — with ~10 of them (frog01's
  // environment + wheels) that tanks the frame rate. Instead, capture a few
  // throttled frames to let the SWF paint, then STOP the readback loop and
  // pause the player. The last captured frame stays in the GPU texture.
  const isOffscreenTexture = instance.spriteNum < 0;
  let rafCount = 0;
  let textureCaptures = 0;
  const TEX_CAPTURE_INTERVAL = 10; // every Nth RAF
  const TEX_CAPTURE_LIMIT = 10;    // ~10 captures (~1.7s) then stop

  function captureFrame() {
    const inst = instances.get(key);
    if (!inst || !inst.canvas) return;

    rafCount++;
    // On-stage sprites capture every frame (they animate). Off-screen
    // textures only capture on the throttled interval.
    const doCapture = !isOffscreenTexture || (rafCount % TEX_CAPTURE_INTERVAL === 0);

    if (doCapture) {
      try {
        const canvas = inst.canvas;
        const width = canvas.width;
        const height = canvas.height;

        if (width > 0 && height > 0) {
          const offscreen = document.createElement('canvas');
          offscreen.width = width;
          offscreen.height = height;
          const offCtx = offscreen.getContext('2d');
          if (offCtx) {
            offCtx.drawImage(canvas, 0, 0);
            const imageData = offCtx.getImageData(0, 0, width, height);
            update_flash_frame(inst.spriteNum, width, height, new Uint8Array(imageData.data.buffer));
          }
        }
      } catch (e) {
        // Silently ignore frame capture errors
      }

      if (isOffscreenTexture && ++textureCaptures >= TEX_CAPTURE_LIMIT) {
        // Static texture fully captured — stop the readback loop and pause
        // the player so it stops consuming CPU every frame.
        inst.animFrameId = null;
        try { inst.rufflePlayer.pause?.(); } catch { /* bridge mode / no pause */ }
        return;
      }
    }

    inst.animFrameId = requestAnimationFrame(captureFrame);
  }

  instance.animFrameId = requestAnimationFrame(captureFrame);
}

/**
 * Destroy a Flash instance and clean up resources.
 */
export function destroyFlashInstance(spriteNum: number): void {
  const key = instanceKey(spriteNum);
  const instance = instances.get(key);
  if (!instance) return;

  if (instance.animFrameId !== null) {
    cancelAnimationFrame(instance.animFrameId);
  }

  try {
    // The DOM element lives in the page (shared between worlds), so
    // `.remove()` is safe to call from either side. Bridge mode also
    // tells the main-world host to drop its registry entry so the
    // RufflePlayer object can be GCed.
    instance.rufflePlayer.remove();
    if (instance.bridgeId) {
      void bridgeDestroyPlayer(instance.bridgeId);
    }
  } catch (e) {
    // Ignore cleanup errors
  }

  instance.container.remove();
  instances.delete(key);
  syncActiveFlashCount();
}

/**
 * Get a Flash variable from a Ruffle instance.
 * Called from WASM via window.dirplayer_ruffleGetVariable.
 */
function translateLevel0(path: string): string {
  if (path.startsWith('_level0')) {
    return '_root' + path.substring('_level0'.length);
  }
  return path;
}

// Bridge mode (extension content scripts) can't make synchronous calls
// across the world boundary, so the Lingo Flash getter/setter handlers
// below can't reach the main-world Ruffle player. In bridge mode each
// `instance.rufflePlayer` is a plain DOM element (no `GetVariable` /
// `SetVariable` / etc.); calls fall into the existing try/catch and
// safely return defaults. SWF playback (the path that goes through
// `createFlashInstance` / `bridgeCallMethod`) still works; only
// Lingo-driven Flash interactivity is degraded.

function getVariable(spriteNum: number, path: string): string | null {
  const key = instanceKey(spriteNum);
  const instance = instances.get(key);
  if (!instance || !instance.ready) {
    // Not ready: either the SWF instance hasn't been created yet, or it has
    // loaded but its ActionScript hasn't finished initializing (so the objects
    // a caller wants — e.g. Coke Studios' `_level0.oLoginServlet` — don't exist
    // yet). `getVariable` must gate on `.ready` like setVariable/callFunction do;
    // reading a not-fully-initialized SWF hands back undefined/garbage.
    // We can't hand a value back to an already-returned synchronous Lingo
    // call, but this isn't lost data for the common case — the Rust
    // getVariable(sprite, path, 0) handler falls back to a lazy
    // FlashObjectRef that re-resolves the sprite when the handle is actually
    // used (by which point the instance is ready). Setting
    // flashAccessBeforeReady makes the frame loop wait for the pending
    // instance before running more scripts. Log at debug to avoid spamming
    // the console for a benign, self-healing condition.
    console.debug(`ruffleGetVariable: no instance yet for sprite#${spriteNum} (path=${path}); deferring via lazy handle`);
    flashAccessBeforeReady = true;
    return null;
  }

  try {
    // Bridge mode (MV3 extension): the real player lives in the main world, so
    // its GetVariable method isn't visible on the isolated-world element — route
    // the read through the synchronous bridge instead.
    if (instance.bridgeId) {
      return bridgeGetVariableSync(instance.bridgeId, translateLevel0(path));
    }
    const val = instance.rufflePlayer.GetVariable(translateLevel0(path));
    return val;
  } catch (e) {
    console.warn(`ruffleGetVariable error:`, e);
    return null;
  }
}

/**
 * Set a Flash variable on a Ruffle instance.
 * Called from WASM via window.dirplayer_ruffleSetVariable.
 */
function setVariable(spriteNum: number, path: string, value: string): boolean {
  const key = instanceKey(spriteNum);
  const instance = instances.get(key);
  // The instance may not exist / be ready yet: a script can push values into
  // the SWF (e.g. spectral-wizard's loader writing `playerScore`) before the
  // renderer has lazily created the Flash instance and finished AS init.
  // Queue the write and replay it on ready (in frame order with goto/play/etc)
  // instead of dropping it. Same pre-instance pattern as gotoFrame/play/stop.
  if (!instance || !instance.ready) {
    queueOp(spriteNum, { kind: 'setVariable', path, value });
    return true; // optimistic — the write will land once the instance is ready
  }

  try {
    // Bridge mode: route through the synchronous bridge (see getVariable).
    if (instance.bridgeId) {
      return bridgeSetVariableSync(instance.bridgeId, translateLevel0(path), value);
    }
    return instance.rufflePlayer.SetVariable(translateLevel0(path), value);
  } catch (e) {
    console.warn(`ruffleSetVariable error:`, e);
    return false;
  }
}

/**
 * Pre-instance method-call queue.
 *
 * A sprite's `beginSprite` script can call `gotoFrame(sprite, N)`,
 * `play(sprite)`, `stop(sprite)` etc. BEFORE the renderer triggers
 * the lazy Flash member load — so the very first calls would otherwise
 * no-op (no instance to act on). storyscramble hits this on sprite 2's
 * BehaviorScript 38, whose beginSprite runs `gotoFrame(31); stop()`
 * to park the help-bubble at the "Read the words…" frame; without
 * queueing, those calls vanish and the bubble sits at frame 11.
 *
 * We queue them in order and replay against the live instance once
 * `createFlashInstance` has loaded the SWF and finished AS init
 * (after the inheritance seed — so explicit `gotoFrame(N)` from
 * beginSprite wins over an inherited sibling frame).
 */
type PendingOp =
  | { kind: 'goto'; frame: number }
  | { kind: 'gotoLabel'; label: string }
  | { kind: 'play' }
  | { kind: 'stop' }
  | { kind: 'rewind' }
  | { kind: 'setVariable'; path: string; value: string }
  | { kind: 'callFunction'; path: string; argsXml: string };
const pendingOps = new Map<number, PendingOp[]>();

/**
 * Pending goto-and-pin target per sprite. Director's `the frame of
 * sprite N = X` is implicitly gotoAndStop, but Ruffle's `GotoFrame(X,
 * true)` (goto+stop) skips paint when the MovieClip is stopped — so the
 * canvas stays blank. Work around it in two steps:
 *   1. `GotoFrame(X, false)` (goto+play) — forces a paint of frame X
 *   2. On the next browser RAF, `GotoFrame(X, true)` — stops the
 *      MovieClip after the paint has landed
 *
 * The map exists so a subsequent `play(sprite N)` (BS69's
 * `mySprite.frame=N; mySprite.play()` shrink pattern) can cancel the
 * deferred stop before it fires — otherwise the shrink animation gets
 * pinned at its first frame and never advances to frame 21.
 */
const pinTarget = new Map<number, number>();

// Fire-and-forget Ruffle player method. In the MV3 extension the player lives in
// the main world, so its methods are invisible on the isolated-world stub —
// route through the bridge; direct otherwise. Used for playback control
// (GotoFrame / play / pause / CallFunction-as-command).
function playerExec(instance: FlashInstance, method: string, args: unknown[] = []): void {
  if (instance.bridgeId) {
    void bridgeCallMethod(instance.bridgeId, method, args);
    return;
  }
  const p = instance.rufflePlayer as Record<string, ((...a: unknown[]) => unknown) | undefined> | undefined;
  const fn = p?.[method];
  if (typeof fn === 'function') {
    try { fn.apply(p, args); } catch (e) { console.warn(`[Flash ${method}] error:`, e); }
  }
}

// Synchronous GetVariable that also works in bridge mode (frameCount /
// currentFrame / findLabel / getFlashProperty need the value immediately).
function playerGetVar(instance: FlashInstance, path: string): string | null {
  if (instance.bridgeId) return bridgeGetVariableSync(instance.bridgeId, path);
  const v = (instance.rufflePlayer as { GetVariable?: (p: string) => string | null } | undefined)?.GetVariable?.(path);
  return v ?? null;
}

/**
 * Numeric seek that LEAVES THE PLAYHEAD RUNNING — matches Director's
 * `sprite(N).gotoFrame(frame)` method semantic (the Flash Asset Xtra's
 * `gotoFrame` calls `gotoAndPlay`, not `gotoAndStop`). Used by
 * `sprite.gotoFrame(N)` Lingo method calls; the SWF continues playing
 * from frame N afterwards. Required for mello's Fire/Marshmello where
 * each label points to an animated frame range that must keep playing.
 */
function applyGotoPlay(instance: FlashInstance, frame: number): void {
  playerExec(instance, 'GotoFrame', [frame, false]);
  // Cancel any pending pin from a previous gotoAndStop on this sprite —
  // play-mode wins over a stale pin target.
  pinTarget.delete(instance.spriteNum);
}

/**
 * Label variant of applyGotoPlay. Ruffle's `GotoFrame(n, …)` only takes
 * numeric frame indices; label resolution lives in AS1, so we call
 * `_root.gotoAndPlay(label)` via CallFunction.
 */
function applyGotoLabelPlay(instance: FlashInstance, label: string): void {
  playerExec(instance, 'CallFunction', ['_root.gotoAndPlay', [label]]);
}

/**
 * Numeric seek that STOPS at the target frame (gotoAndStop semantic).
 * Director's `sprite.frame = N` property setter uses this — required for
 * storyscramble's tiles which must pin at a poster frame (2/4/6) and not
 * advance through the SWF's other frames. Uses a microtask-pin to avoid
 * a render race against Ruffle's RAF; see schedulePin for the rationale.
 */
function applyGotoAndPin(instance: FlashInstance, frame: number): void {
  // Goto+play first forces Ruffle to render frame N synchronously (Ruffle skips
  // paint for already-stopped MovieClips, so a bare gotoAndStop wouldn't show
  // frame N until something else triggered a repaint).
  playerExec(instance, 'GotoFrame', [frame, false]);
  pinTarget.set(instance.spriteNum, frame);
  schedulePin(instance, frame);
}

/**
 * Label variant of applyGotoAndPin — gotoAndPlay followed by stop via
 * AS1 CallFunction.
 */
function applyGotoLabelAndPin(instance: FlashInstance, label: string): void {
  playerExec(instance, 'CallFunction', ['_root.gotoAndPlay', [label]]);
  queueMicrotask(() => {
    playerExec(instance, 'CallFunction', ['_root.stop', []]);
    // Resume the player so the seeked label frame paints — see schedulePin
    // for the full rationale (a movie pinning the sprite via per-frame
    // stop()/hold keeps the player suspended, so the goto never repaints).
    playerExec(instance, 'play', []);
  });
}

/**
 * Shared microtask-pin used by the numeric goto path.
 */
function schedulePin(instance: FlashInstance, frame: number): void {
  // Pin via microtask, NOT requestAnimationFrame: browser RAFs fire in
  // registration order, and Ruffle's render-loop RAF (registered first
  // on player init) runs before ours. If Ruffle's per-frame
  // accumulator has crossed the SWF tick interval by then, it advances
  // the MovieClip from N to N+1, paints N+1, captureFrame snapshots
  // N+1, and *then* our pin runs — too late to prevent the wrong
  // frame being displayed (and Ruffle won't repaint after the stop
  // because it skips paint on stopped MovieClips). A microtask runs
  // after the current synchronous Lingo dispatch but BEFORE the next
  // browser frame, so Ruffle gets no chance to tick in between. The
  // tile poster-frame race during rapid hover (`mySprite.frame =
  // tileFrame ± 1`) goes away.
  queueMicrotask(() => {
    // Only pin if no one (play/stop/rewind, or a later goto to a
    // different frame) cleared / overwrote our target in the meantime.
    if (pinTarget.get(instance.spriteNum) !== frame) return;
    pinTarget.delete(instance.spriteNum);
    if (instance.bridgeId) {
      // Bridge mode: fire-and-forget goto+play through the bridge.
      playerExec(instance, 'GotoFrame', [frame, true]);
      playerExec(instance, 'play', []);
      return;
    }
    try {
      instance.rufflePlayer.GotoFrame(frame, true);
      // Resume the PLAYER (not the clip) so the seeked frame actually paints
      // to the canvas. Ruffle's `goto_frame` only rebuilds the display list;
      // the paint happens on a player tick, which is skipped while the player
      // is SUSPENDED. A movie that pins a Flash sprite by calling
      // `stop(sprite N)` / `hold` every frame keeps the player suspended, so
      // without this the model advances to `frame` but dirplayer keeps
      // capturing the STALE previous frame (bogey_nights' intro: clicking
      // Instructions sets `sprite(1).frame = 113` but the menu stayed on
      // screen). The clip itself is stopped by the gotoAndStop above, so
      // resuming the player renders `frame` once and it stays put; the
      // movie's next per-frame stop() re-suspends. For sprites that were
      // never paused (poster-frame tiles) the player is already playing and
      // this is a harmless no-op. Runs in the microtask (after the current
      // synchronous Lingo dispatch) so a same-frame stop() can't re-suspend
      // before the paint lands.
      instance.rufflePlayer.play?.();
    } catch (e) {
      console.warn(`[Flash goto] pin-step error:`, e);
    }
  });
}

function queueOp(spriteNum: number, op: PendingOp): void {
  const list = pendingOps.get(spriteNum) ?? [];
  list.push(op);
  pendingOps.set(spriteNum, list);
}

function flushPendingGoto(spriteNum: number): void {
  const ops = pendingOps.get(spriteNum);
  if (!ops || ops.length === 0) return;
  pendingOps.delete(spriteNum);
  const instance = instances.get(instanceKey(spriteNum));
  if (!instance) return;
  for (const op of ops) {
    try {
      switch (op.kind) {
        case 'goto':
          applyFrameSetting(instance, spriteNum, String(op.frame), true);
          break;
        case 'gotoLabel':
          applyFrameSetting(instance, spriteNum, op.label, false);
          break;
        case 'play':
          // Cancel any deferred gotoAndStop pin from a queued goto earlier in
          // this queue, THEN actually resume playback from the current frame.
          // A preceding queued `stop`/gotoAndStop (very common: the bogeyman's
          // `#pickit` does `sprite(16).frame = 1` then `play(sprite 16)` on a
          // just-swapped straw/longarm instance) leaves the clip PARKED — with a
          // no-op here it stays at frame 1, so `#rollit`'s frame poll never
          // advances, the grab never completes, and the sprite is stuck showing
          // "straw". Playing from `_currentframe` (not a hardcoded 1) means an
          // already-autoplayed clip isn't restarted.
          pinTarget.delete(spriteNum);
          instance.stopped = false;
          {
            playerExec(instance, 'play');
            const cur = parseInt(playerGetVar(instance, '/:_currentframe') || '1', 10) || 1;
            playerExec(instance, 'GotoFrame', [cur, false]);
          }
          break;
        case 'stop':
          // Mirror the live stopFlash: halt the root TIMELINE, don't suspend
          // the whole player. A queued `stop` replaying `pause()` here was the
          // bug behind bogey_nights' end screen — "flash bhv" beginSprite runs
          // `sprite(3).frame = 116` then `sprite(3).stop()`; when sprite 3's
          // instance was (re)loading, both ops queued, and the queued stop
          // paused the player so the frame-116 seek never painted (stale frame
          // on screen even though `sprite(3).frame` read 116). Keeping the
          // player alive lets the seeked frame render.
          pinTarget.delete(spriteNum);
          instance.stopped = true;
          playerExec(instance, 'CallFunction', ['_root.stop', []]);
          break;
        case 'rewind':
          pinTarget.delete(spriteNum);
          instance.stopped = true;
          playerExec(instance, 'GotoFrame', [1, true]);
          break;
        case 'setVariable':
          playerExec(instance, 'SetVariable', [translateLevel0(op.path), op.value]);
          break;
        case 'callFunction': {
          // Replay a pre-ready callFunction. Decode args exactly like the
          // live callFunction path (null → undefined, `__ruffle_path:` →
          // AS object handle) so the replayed call matches what Lingo asked
          // for. The return value is discarded — the original Lingo call
          // already returned VOID; only the side effect is reproduced.
          const rawArgs: any[] = op.argsXml ? JSON.parse(op.argsXml) : [];
          const args: any[] = rawArgs.map(arg => {
            if (arg === null) return undefined;
            if (typeof arg === 'string' && arg.startsWith('__ruffle_path:')) return { __ruffle_path: arg.substring('__ruffle_path:'.length) };
            return arg;
          });
          playerExec(instance, 'CallFunction', [translateLevel0(op.path), args]);
          break;
        }
      }
    } catch (e) {
      console.warn(`[Flash flush] sprite#${spriteNum} op ${op.kind} error:`, e);
    }
  }
}

/**
 * `sprite(N).gotoFrame(frameOrLabel)` method call — seek and KEEP
 * PLAYING (Director's gotoFrame is gotoAndPlay-semantic). Called from
 * WASM via window.dirplayer_ruffleGoToFrame.
 *
 * The numeric-detection regex requires the entire trimmed string to be
 * an optional sign plus digits — odd labels that happen to start with a
 * digit (`"3frame"`) stay on the label path instead of getting
 * parseInt'd to `3`.
 *
 * For the `sprite.frame = N` property setter (gotoAndStop semantic) use
 * goToFrameAndStop below.
 */
function goToFrame(spriteNum: number, frameOrLabel: string): void {
  const key = instanceKey(spriteNum);
  const instance = instances.get(key);
  const trimmed = frameOrLabel.trim();
  const isNumeric = /^-?\d+$/.test(trimmed);

  if (!instance || !instance.ready) {
    if (isNumeric) {
      queueOp(spriteNum, { kind: 'goto', frame: parseInt(trimmed, 10) });
    } else {
      queueOp(spriteNum, { kind: 'gotoLabel', label: frameOrLabel });
    }
    return;
  }
  instance.stopped = false; // gotoFrame is gotoAndPlay-semantic → keep playing
  if (isNumeric) {
    applyGotoPlay(instance, parseInt(trimmed, 10));
  } else {
    applyGotoLabelPlay(instance, frameOrLabel);
  }
}

/**
 * `the frame of sprite N = X` property setter. Director's Flash `frame` setter
 * is a seek that PRESERVES the member's play intent (`pausedAtStart`):
 *
 *  - A static member (`pausedAtStart = true`, e.g. storyscramble's poster
 *    tiles) pins at the chosen frame and must not advance — gotoAndStop.
 *  - An animated member (`pausedAtStart = false`, the default) seeks and KEEPS
 *    PLAYING; the SWF's own `stop()` actions decide where it rests. Dora
 *    Soccer's title menu is such a member: on a replay it sets `frame = 233`
 *    (a plain frame with no stop) and relies on the timeline playing forward to
 *    the `stop()` at frame 239 — the difficulty-select screen where the buttons
 *    arm. Pinning it at 233 left the four mode buttons dead after finishing a
 *    level.
 *
 * Called from WASM via window.dirplayer_ruffleGoToFrameAndStop.
 */
function goToFrameAndStop(spriteNum: number, frameOrLabel: string): void {
  const key = instanceKey(spriteNum);
  const instance = instances.get(key);
  const trimmed = frameOrLabel.trim();
  const isNumeric = /^-?\d+$/.test(trimmed);

  if (!instance || !instance.ready) {
    if (isNumeric) {
      queueOp(spriteNum, { kind: 'goto', frame: parseInt(trimmed, 10) });
    } else {
      queueOp(spriteNum, { kind: 'gotoLabel', label: frameOrLabel });
    }
    // Make the WASM frame loop WAIT for the instance before running more
    // scripts, exactly like getVariable/callFunction/setVariable. Otherwise the
    // movie keeps running while the Flash sprite is still loading and the queued
    // frame set is applied out of order on ready — Dora Soccer's replay sets
    // `sprite(1).frame = 233` on the just-recreated title menu instance and the
    // difficulty screen ended up on the wrong frame. Blocking lets the seek land
    // in sequence.
    flashAccessBeforeReady = true;
    return;
  }
  applyFrameSetting(instance, spriteNum, frameOrLabel, isNumeric);
}

/**
 * Shared body of the `sprite.frame = X` setter — used both live
 * (goToFrameAndStop) and when replaying a queued `goto`/`gotoLabel` that arrived
 * before the Flash instance was ready (flushPendingGoto). A STATIC member
 * (pausedAtStart=true) pins at the frame; an ANIMATED member (pausedAtStart
 * =false) seeks and keeps playing so the SWF's own stop() actions decide where
 * it rests. The queued path MUST route through here too — Dora Soccer's replay
 * re-enters the menu frame, which recreates the title Flash instance, so
 * `sprite(1).frame = 233` is queued and only applied on ready; pinning it there
 * left the menu stuck at 233 instead of playing forward to the stop() at 239.
 */
function applyFrameSetting(
  instance: FlashInstance,
  spriteNum: number,
  frameOrLabel: string,
  isNumeric: boolean,
): void {
  const trimmed = frameOrLabel.trim();
  if (instance.pausedAtStart) {
    instance.stopped = true; // static member → pin at the frame
    if (isNumeric) {
      applyGotoAndPin(instance, parseInt(trimmed, 10));
    } else {
      applyGotoLabelAndPin(instance, frameOrLabel);
    }
  } else {
    // Animated member → seek and keep playing. Resume like playFlash():
    // Ruffle's root MovieClip may be sitting on an AS `stop()` (Dora's menu
    // rests at the previous stop() before the game seeks it), and a bare
    // GotoFrame(N,false) re-seats the playhead but the player-level paused flag
    // can leave it frozen — so flip play() first, then GotoFrame to clear the
    // MovieClip's own stopped flag and advance from N.
    instance.stopped = false;
    pinTarget.delete(spriteNum);
    playerExec(instance, 'play');
    if (isNumeric) {
      applyGotoPlay(instance, parseInt(trimmed, 10));
    } else {
      applyGotoLabelPlay(instance, frameOrLabel);
    }
  }
}

/**
 * Call a Flash function on a Ruffle instance.
 * Called from WASM via window.dirplayer_ruffleCallFunction.
 */
// Returns the RAW Ruffle value (boolean/number/string/object/null), not just a
// string — the WASM `ruffle_call_function` extern receives it as a JsValue and
// type-branches (bool → Int(1/0), object → FlashObjectRef, …). Stringifying here
// broke DGS's `includeIsLoaded() = 1` (AS true → "true" ≠ 1).
function callFunction(spriteNum: number, path: string, argsXml: string): unknown {
  const key = instanceKey(spriteNum);
  const instance = instances.get(key);
  // Instance not created / AS-init not finished yet: a beginSprite (or a
  // puppet/mid-frame) script can call into the SWF before the renderer has
  // lazily created the Ruffle player. A synchronous Lingo call can't be made
  // to block for the ~3s AS-init, and it can't be handed a return value after
  // it has already returned — but the *side effect* of the call can still be
  // replayed. Queue it (in frame order with goto/play/setVariable) and fire it
  // once the instance is ready; return null now, matching Director's own
  // "flash not ready → VOID" result for the immediate call. See setVariable
  // for the same pre-instance pattern.
  if (!instance || !instance.ready) {
    queueOp(spriteNum, { kind: 'callFunction', path, argsXml });
    flashAccessBeforeReady = true;
    return null;
  }

  try {
    // Parse JSON array of args from Rust
    const rawArgs: any[] = argsXml ? JSON.parse(argsXml) : [];
    const args: any[] = rawArgs.map(arg => {
      if (arg === null) return undefined;
      if (typeof arg === 'string' && arg.startsWith('__ruffle_path:')) return { __ruffle_path: arg.substring('__ruffle_path:'.length) };
      return arg;
    });
    // Bridge mode: route through the synchronous bridge (see getVariable).
    if (instance.bridgeId) {
      return bridgeCallFunctionSync(instance.bridgeId, translateLevel0(path), args);
    }
    return instance.rufflePlayer.CallFunction(translateLevel0(path), args);
  } catch (e) {
    console.warn(`ruffleCallFunction error:`, e);
    return null;
  }
}

/**
 * Stop playback of a Ruffle instance (stays on current frame).
 */
function stopFlash(spriteNum: number): void {
  const key = instanceKey(spriteNum);
  const instance = instances.get(key);
  if (!instance || !instance.ready) {
    queueOp(spriteNum, { kind: 'stop' });
    return;
  }
  pinTarget.delete(spriteNum);
  instance.stopped = true;
  try {
    // Stop the root TIMELINE, not the whole player. `player.pause()` suspends
    // the entire Ruffle player, which halts its render loop — so any later
    // GotoFrame updates the SWF model but never repaints, and dirplayer
    // captures a stale canvas. bogey_nights' intro calls `stop(sprite 1)`
    // every frame; with pause() the menu stayed frozen on screen even after
    // `sprite(1).frame = 113` moved the model to the Instructions frame
    // (confirmed: overriding ruffleStop to a no-op made Instructions appear).
    // Halting only the root MovieClip freezes the timeline in place while
    // leaving the player alive to paint frame changes. `sprite.playing` reads
    // `instance.stopped` since the player itself keeps running.
    // Bridge mode routes CallFunction through the main-world player.
    playerExec(instance, 'CallFunction', ['_root.stop', []]);
  } catch (e) {
    console.warn(`ruffleStop error:`, e);
  }
}

/**
 * Start/resume playback of a Ruffle instance.
 *
 * `mySprite.play()` must resume the SWF even after AS has called `stop()`
 * inside the movie (e.g. storyscramble's bubble at frame 11). Ruffle's
 * `Player::play()` only flips the player-level paused flag and doesn't
 * clear the MovieClip's own stopped state set by AS; the playhead stays
 * frozen and frame-21's `getURL("event: send #done")` never fires.
 * Routing through `GotoFrame(currentFrame, false)` hits MovieClip's
 * `goto_frame`, which both re-seats the playhead and clears that flag.
 */
function playFlash(spriteNum: number): void {
  const key = instanceKey(spriteNum);
  const instance = instances.get(key);
  if (!instance || !instance.ready) {
    queueOp(spriteNum, { kind: 'play' });
    return;
  }
  // Cancel any in-flight pin from a `mySprite.frame = N` call earlier
  // in this same Lingo dispatch — otherwise the scheduled RAF stop
  // would undo our play() a frame later (BS69 shrink animation).
  pinTarget.delete(spriteNum);
  instance.stopped = false;
  playerExec(instance, 'play');
  const cur = parseInt(playerGetVar(instance, '/:_currentframe') || '1', 10) || 1;
  playerExec(instance, 'GotoFrame', [cur, false]);
}

/**
 * Rewind a Ruffle instance to frame 1 and stop.
 */
function rewindFlash(spriteNum: number): void {
  const key = instanceKey(spriteNum);
  const instance = instances.get(key);
  if (!instance || !instance.ready) {
    queueOp(spriteNum, { kind: 'rewind' });
    return;
  }
  pinTarget.delete(spriteNum);
  instance.stopped = true;
  playerExec(instance, 'GotoFrame', [1, true]);
}

/**
 * Check if a Ruffle instance is currently playing.
 */
function isPlaying(spriteNum: number): boolean {
  const key = instanceKey(spriteNum);
  const instance = instances.get(key);
  if (!instance) return false;
  // The player's render loop stays alive even when a sprite is "stopped" (we
  // halt the root timeline, not the player — see stopFlash), so the
  // player-level isPlaying is no longer a reliable proxy for the movie's
  // stopped state. Honour the tracked intent first.
  if (instance.stopped) return false;
  // Bridge mode: the isPlaying property isn't on the isolated-world stub; past
  // the `stopped` gate the intent is "playing".
  if (instance.bridgeId) return true;
  try {
    return instance.rufflePlayer.isPlaying ?? false;
  } catch (e) {
    return false;
  }
}

/**
 * Get the total frame count of a Ruffle instance.
 */
function getFrameCount(spriteNum: number): number {
  const key = instanceKey(spriteNum);
  const instance = instances.get(key);
  if (!instance) return 0;
  try {
    return parseInt(playerGetVar(instance, "/:_totalframes") || "0", 10);
  } catch (e) {
    return 0;
  }
}

/**
 * Get the current frame of a Ruffle instance (1-based).
 */
function getCurrentFrame(spriteNum: number): number {
  const key = instanceKey(spriteNum);
  const instance = instances.get(key);
  if (!instance) return 0;
  try {
    return parseInt(playerGetVar(instance, "/:_currentframe") || "0", 10);
  } catch (e) {
    return 0;
  }
}

/**
 * Call scripts on a specific frame without navigating to it.
 * In Director, callFrame() executes the frame's scripts.
 * We implement this as goToFrame + immediate return (best effort).
 */
function callFrame(spriteNum: number, frame: number): void {
  const key = instanceKey(spriteNum);
  const instance = instances.get(key);
  if (!instance) return;
  // callFrame in Director executes the actions on a given frame.
  // Best approximation: go to that frame (which runs its scripts) and stop.
  playerExec(instance, 'GotoFrame', [frame, true]);
}

/**
 * Find a frame label and return its frame number (1-based), or -1 if not found.
 */
function findLabel(spriteNum: number, _label: string): number {
  const key = instanceKey(spriteNum);
  const instance = instances.get(key);
  if (!instance) return -1;
  // No direct label lookup in the legacy Flash Player JS API; if the SWF
  // exposes a `findLabel` AS function we could call it, otherwise return
  // -1 and let the script fall through.
  return -1;
}

/**
 * Classify what's under a sprite-local point, mirroring Director's Flash
 * `sprite.hitTest(point)` return values — and the signal behind
 * `sprite.mouseOverButton`. Coordinates are sprite-local pixels (origin at the
 * sprite's top-left), the same space `dispatchMouseEvent` takes; we rebase them
 * to canvas pixels and hand them to the fork's `dirplayer_hitTest`, which
 * injects a synthetic MouseMove to refresh hover state then reads the resolved
 * cursor + a stage shape pick.
 *
 * Returns: 0 = #background, 1 = #normal, 2 = #button, 3 = #editText
 * (0 when no instance / the fork method is unavailable).
 */
function hitTest(spriteNum: number, localX: number, localY: number): number {
  const inst = instances.get(instanceKey(spriteNum));
  if (!inst) return 0;

  let canvasX = localX;
  let canvasY = localY;
  if (inst.canvas) {
    const rect = inst.canvas.getBoundingClientRect();
    if (rect.width > 0 && rect.height > 0) {
      canvasX = (localX / rect.width) * inst.canvas.width;
      canvasY = (localY / rect.height) * inst.canvas.height;
    }
  }

  // Bridge mode: dirplayer_hitTest lives on the main-world player; call it
  // synchronously (the classification 0–3 must return to Lingo inline).
  if (inst.bridgeId) {
    const r = bridgeCallMethodSync(inst.bridgeId, 'dirplayer_hitTest', [canvasX, canvasY]);
    return (typeof r === 'number' ? r : 0) | 0;
  }
  const player = inst.rufflePlayer as
    | { dirplayer_hitTest?: (x: number, y: number) => number }
    | undefined;
  if (typeof player?.dirplayer_hitTest !== 'function') {
    return 0;
  }
  try {
    return player.dirplayer_hitTest(canvasX, canvasY) | 0;
  } catch (e) {
    return 0;
  }
}

/**
 * Get a Flash property by property number (matching Director's getFlashProperty).
 * Property numbers follow the original Flash Player property indices.
 */
function getFlashProperty(spriteNum: number, target: string, propNum: number): string | null {
  const key = instanceKey(spriteNum);
  const instance = instances.get(key);
  if (!instance) return null;

  // Flash property number to variable name mapping
  const propMap: Record<number, string> = {
    0: '_x', 1: '_y', 2: '_xscale', 3: '_yscale',
    4: '_currentframe', 5: '_totalframes', 6: '_alpha', 7: '_visible',
    8: '_width', 9: '_height', 10: '_rotation', 11: '_target',
    12: '_framesloaded', 13: '_name', 14: '_droptarget', 15: '_url',
    16: '_highquality', 17: '_focusrect', 18: '_soundbuftime', 19: '_quality',
    20: '_xmouse', 21: '_ymouse',
  };

  const propName = propMap[propNum];
  if (!propName) return null;

  try {
    const path = target ? `${target}:${propName}` : `/:${propName}`;
    return playerGetVar(instance, path)?.toString() ?? null;
  } catch (e) {
    return null;
  }
}

/**
 * Set a Flash property by property number.
 */
function setFlashProperty(spriteNum: number, target: string, propNum: number, value: string): void {
  const key = instanceKey(spriteNum);
  const instance = instances.get(key);
  if (!instance) return;

  const propMap: Record<number, string> = {
    0: '_x', 1: '_y', 2: '_xscale', 3: '_yscale',
    6: '_alpha', 7: '_visible', 10: '_rotation', 13: '_name',
    16: '_highquality', 18: '_soundbuftime',
  };

  const propName = propMap[propNum];
  if (!propName) return;

  try {
    const path = target ? `${target}:${propName}` : `/:${propName}`;
    playerExec(instance, 'SetVariable', [path, value]);
  } catch (e) {
    console.warn(`ruffleSetFlashProperty error:`, e);
  }
}

/**
 * Execute a tellTarget command on a Ruffle instance.
 * In Flash, tellTarget changes the target timeline for subsequent actions.
 */
function tellTarget(spriteNum: number, target: string, action: string): void {
  const key = instanceKey(spriteNum);
  const instance = instances.get(key);
  if (!instance) return;
  try {
    // tellTarget + action: best effort via SetVariable/CallFunction
    if (action === "play") {
      playerExec(instance, 'SetVariable', [`${target}:_visible`, "1"]);
      // Can't directly play a sub-timeline from JS, use CallFunction
    } else if (action === "stop") {
      // Similar limitation
    }
  } catch (e) {
    console.warn(`ruffleTellTarget error:`, e);
  }
}

/**
 * Callback registry for Lingo callbacks.
 * Maps "movieClipPath:methodName" -> { castLib, castMember, lingoHandler }
 */
interface LingoCallbackEntry {
  castLib: number;
  castMember: number;
  lingoHandler: string;
}

const callbackRegistry = new Map<string, LingoCallbackEntry>();

/**
 * Register a Lingo callback. Called from dirplayer WASM (via setCallback handler).
 * This registers the callback in Ruffle's WASM (LINGO_CALLBACKS) so that when
 * AVM1 calls the matching method, Ruffle fires trigger_lingo_callback_on_script
 * back to dirplayer.
 */
function registerLingoCallback(
  movieClipPath: string,
  methodName: string,
  lingoCastLib: number,
  lingoCastMember: number,
  lingoHandler: string,
  flashCastLib: number,
  flashCastMember: number,
): void {
  const key = `${movieClipPath}:${methodName}`;
  callbackRegistry.set(key, {
    castLib: lingoCastLib,
    castMember: lingoCastMember,
    lingoHandler,
  });

  // Call our forked Ruffle's WASM export to register the callback in
  // LINGO_CALLBACKS. The export is exposed under the dirplayer_ prefix to
  // avoid colliding with stock Ruffle if it's also on the page.
  const win = window as any;
  if (win.dirplayer_ruffleRegisterLingoCallback) {
    win.dirplayer_ruffleRegisterLingoCallback(
      movieClipPath,
      methodName,
      lingoCastLib,
      lingoCastMember,
      lingoHandler,
      flashCastLib,
      flashCastMember,
    );
    console.log(`Registered Lingo callback: ${key} -> #${lingoHandler} (lingo=${lingoCastLib}:${lingoCastMember}, flash=${flashCastLib}:${flashCastMember})`);
  } else {
    console.warn('dirplayer_ruffleRegisterLingoCallback not available on window (Ruffle not loaded yet?)');
  }
}

/**
 * Register global JS functions that the WASM module calls into.
 *
 * Every name uses the `dirplayer_` prefix so we don't collide with stock
 * Ruffle if it's already loaded on the page (e.g. via a browser extension
 * or another script tag). The matching #[wasm_bindgen(js_name = ...)]
 * imports in the Rust side use the same prefixed names.
 */
export function initFlashBridge(): void {
  const win = window as any;
  // Flash LocalConnection.send bridge (Neopets DGS score/protocol). The Ruffle
  // fork calls dirplayer_localConnectionSend(connName, method, argsJson); route
  // it into the WASM export that dispatches the Lingo setCallback handler.
  // Direct in dev (fork + this run in one world); in the MV3 extension the fork
  // is main-world, so the bridge host re-fires it here as a `dirplayer-lc-send`
  // DOM event (below).
  win.dirplayer_localConnectionSend = (name: string, method: string, argsJson: string): boolean => {
    try { return !!local_connection_send(name, method, argsJson); } catch { return false; }
  };
  if (isExtensionContext()) {
    window.addEventListener('dirplayer-lc-send', (ev) => {
      const d = (ev as CustomEvent).detail as
        { name?: string; method?: string; argsJson?: string } | undefined;
      if (!d || typeof d.name !== 'string') return;
      try { local_connection_send(d.name, d.method || '', d.argsJson || '[]'); } catch { /* ignore */ }
    });
  }
  win.dirplayer_ruffleGetVariable = getVariable;
  win.dirplayer_flashInstances = instances;
  win.dirplayer_ruffleSetVariable = setVariable;
  win.dirplayer_ruffleCallFunction = callFunction;
  win.dirplayer_ruffleGoToFrame = goToFrame;
  win.dirplayer_ruffleGoToFrameAndStop = goToFrameAndStop;
  win.dirplayer_ruffleStop = stopFlash;
  win.dirplayer_rufflePlay = playFlash;
  win.dirplayer_ruffleRewind = rewindFlash;
  win.dirplayer_ruffleSetSize = setFlashSize;
  win.dirplayer_ruffleIsPlaying = isPlaying;
  win.dirplayer_ruffleGetFrameCount = getFrameCount;
  win.dirplayer_ruffleGetCurrentFrame = getCurrentFrame;
  win.dirplayer_ruffleCallFrame = callFrame;
  win.dirplayer_ruffleFindLabel = findLabel;
  win.dirplayer_ruffleHitTest = hitTest;
  win.dirplayer_ruffleGetFlashProperty = getFlashProperty;
  win.dirplayer_ruffleSetFlashProperty = setFlashProperty;
  win.dirplayer_ruffleTellTarget = tellTarget;
  win.dirplayer_ruffleRegisterLingoCallback_dirplayer = registerLingoCallback;

  // Expose dirplayer's WASM exports as window.wasmModule so that Ruffle's
  // wasm_bindgen extern (js_namespace = wasmModule) can resolve
  // trigger_lingo_callback_on_script back to dirplayer's WASM export.
  // Expose as global function for Ruffle's wasm_bindgen extern
  // Ruffle sends args as a JSON array of base64-encoded JSON values.
  // Decode them to native JS values before passing to WASM.
  win.dirplayer_triggerLingoCallbackOnScript = (castLib: number, castMember: number, handlerName: string, argsJson: string, flashCastLib: number, flashCastMember: number) => {
    try {
      const b64Args: string[] = JSON.parse(argsJson);
      const decodedArgs = b64Args.map((b64: string) => {
        try {
          const json = atob(b64);
          return JSON.parse(json);
        } catch {
          return b64; // fallback: pass as-is
        }
      });
      return trigger_lingo_callback_on_script(castLib, castMember, handlerName, JSON.stringify(decodedArgs), flashCastLib, flashCastMember);
    } catch (e) {
      console.error('[triggerLingoCallback] decode error:', e);
      return trigger_lingo_callback_on_script(castLib, castMember, handlerName, argsJson, flashCastLib, flashCastMember);
    }
  };

  // Expose flash loading state for the WASM frame loop to check. The loop
  // BLOCKS (up to 15s) while this is true, so it must only be true when the
  // game genuinely can't proceed without a Flash instance — i.e. when a script
  // actually tried to read a not-yet-ready Flash sprite (getVariable /
  // callFunction / setVariable set `flashAccessBeforeReady`).
  //
  // It must NOT block merely because some instance is still loading
  // (`flashLoadingCount > 0`). Games that spawn many transient, display-only
  // Flash sprites (bogey_nights turns every "superstar" splash into a
  // boogyflash/spitflash SWF) would otherwise stall the ENTIRE game — including
  // player input — for ~3s per spawn, perpetually, since there's always a load
  // in flight. Those sprites are never scripted, so blocking for them is pure
  // lost time; they just pop in when their background load finishes.
  win.dirplayer_isFlashLoading = () => flashAccessBeforeReady;

  // Per-sprite readiness, used by the WASM side to BLOCK an individual Flash
  // interop call (getVariable / setVariable / callFunction / setCallback) until
  // that sprite's Ruffle instance has loaded AND finished AS init — so a
  // one-shot Lingo init (Coke Studios' SF gateway reading _level0.oLoginServlet
  // etc.) gets live objects instead of null. Unlike dirplayer_isFlashLoading
  // this is scoped to one sprite, so it never stalls unrelated display-only
  // Flash sprites. Returns true (proceed) when the instance is ready OR when no
  // instance exists and nothing is loading (so the caller can't hang forever on
  // a sprite that will never get an instance).
  win.dirplayer_isFlashInstanceReady = (spriteNum: number): boolean => {
    const inst = instances.get(instanceKey(spriteNum));
    // Only "ready" once the instance exists AND has finished AS init. A missing
    // instance is NOT ready: the sprite's SWF is (or is about to be) loading, so
    // the WASM-side wait must keep polling until it lands — otherwise a very
    // early one-shot call (Coke Studios' SESSION_createSession, made before the
    // instance dispatches) gets deferred and returns null, killing the session.
    // The WASM wait is bounded, so a sprite that never gets an instance can't
    // hang forever.
    return !!(inst && inst.ready);
  };

  // Hand-fired test entry for Flash `event: …` dispatch — lets you prove
  // the WASM dispatch chain end-to-end from DevTools without waiting for
  // a real SWF `getURL("event: …")` call. Example:
  //   dirplayer_dispatchFlashEvent(1, 45, "send #done")
  win.dirplayer_dispatchFlashEvent = dispatchFlashEvent;

  // Mouse forwarding: dirplayer's WASM-side mouseDown/mouseUp handlers
  // call this when the click lands on a Flash sprite, so the SWF's own
  // AS1 button handlers actually fire (Ruffle's canvas is hidden
  // offscreen and never receives real browser clicks). See
  // dispatchMouseEvent above.
  win.dirplayer_ruffleDispatchMouse = dispatchMouseEvent;

  // Ensure dirplayer_RufflePlayer config is set up before any instances are
  // created. Skip when the host disabled Flash — they may already have stock
  // Ruffle on the page via an extension or another script tag, and we
  // shouldn't be mutating its config when we aren't going to use it ourselves.
  if (!isFlashDisabled()) {
    win.dirplayer_RufflePlayer = win.dirplayer_RufflePlayer || {};
    win.dirplayer_RufflePlayer.config = {
      ...(win.dirplayer_RufflePlayer.config || {}),
      allowNetworking: 'all',
    };
  }
}

/**
 * Destroy all Flash instances.
 */
export function destroyAllFlashInstances(): void {
  instances.forEach((instance) => {
    if (instance.animFrameId !== null) {
      cancelAnimationFrame(instance.animFrameId);
    }
    try {
      instance.rufflePlayer.remove();
      if (instance.bridgeId) {
        void bridgeDestroyPlayer(instance.bridgeId);
      }
    } catch (e) {
      // Ignore
    }
    instance.container.remove();
  });
  instances.clear();
  syncActiveFlashCount();
}

/**
 * Configuration interface for the flash manager.
 * Used by the polyfill to pass page-level config.
 */
export interface FlashManagerConfig {
  socketProxy: Array<{host: string, port: number, proxyUrl: string}>;
  fetchRewriteRules: Array<{pathPrefix: string, targetHost: string, targetPort: number, targetProtocol: string}>;
  renderer: string;
  logLevel: string;
  /**
   * When true, dirplayer-rs skips creating Ruffle instances for Flash
   * cast members. All `window.dirplayer_ruffle*` Lingo bridge calls become
   * safe no-ops (they early-return on missing instance). Use on pages
   * that don't actually rely on Flash content — keeps Ruffle out of
   * the bundle and silences "Ruffle not found" errors.
   */
  disableFlash: boolean;
}

/**
 * Apply external configuration. Called from polyfill's configureFlash().
 * Stores config on window so getSocketProxyConfig() and other code can read it.
 */
export function configureFlashManager(partial: Partial<FlashManagerConfig>): void {
  const win = window as any;
  const existing = win.__dirplayerFlashConfig || {};
  win.__dirplayerFlashConfig = { ...existing, ...partial };
  // Note: `dirplayerResolveSocketUrl` is installed unconditionally near
  // the top of this file and reads from `getSocketProxyConfig()` on every
  // call, so it picks up `partial.socketProxy` automatically without
  // needing to be re-bound here.
}
