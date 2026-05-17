/**
 * FlashPlayerManager - Bridges Ruffle (Flash player) with dirplayer-rs
 *
 * Manages Ruffle player instances for Flash cast members, reads rendered frames,
 * and sends pixel data to dirplayer's WASM rendering pipeline so Flash content
 * can be composited with Director sprites (Director sprites can layer on top).
 */ 

import { update_flash_frame, trigger_lingo_callback_on_script, dispatch_flash_event } from 'vm-rust';
import {
  isBridgeRequired,
  waitForBridge,
  bridgeCreatePlayer,
  bridgeFindElement,
  bridgeCallMethod,
  bridgeDestroyPlayer,
} from './ruffleBridgeClient';

interface FlashInstance {
  spriteNum: number;     // Director sprite number this instance belongs to
  castLib: number;       // SWF source cast member (diagnostics + cleanup)
  castMember: number;
  rufflePlayer: any; // RufflePlayerElement (direct) or stub element (bridge mode)
  bridgeId: string | null; // Set when this instance is driven by the main-world bridge
  container: HTMLDivElement;
  canvas: HTMLCanvasElement | null;
  width: number;
  height: number;
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

const origFetch = window.fetch;
window.fetch = function(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  if (typeof input === 'string') {
    try {
      const url = new URL(input, window.location.origin);
      if (applyFetchRewrite(url)) {
        input = url.toString();
      }
    } catch { /* ignore parse errors */ }
  } else if (input instanceof Request) {
    try {
      const url = new URL(input.url);
      if (applyFetchRewrite(url)) {
        const newUrl = url.toString();
        const req = input;
        return req.arrayBuffer().then(bodyBuf => {
          const newInit: RequestInit = {
            method: req.method,
            headers: req.headers,
            body: bodyBuf.byteLength > 0 ? bodyBuf : undefined,
            mode: 'cors' as RequestMode,
            credentials: 'omit' as RequestCredentials,
          };
          return origFetch.call(window, newUrl, newInit);
        });
      }
    } catch (e) { console.error('[fetch-intercept] Error:', e); }
  }
  return origFetch.call(window, input, init);
};

// Monkey-patch HTMLCanvasElement.getContext to force preserveDrawingBuffer: true
// for all WebGL contexts. This is needed so we can read pixels back from Ruffle's
// wgpu-webgl canvas after the frame is presented.
const origGetContext = HTMLCanvasElement.prototype.getContext;
(HTMLCanvasElement.prototype as any).getContext = function(type: string, attrs?: any) {
  if (type === 'webgl' || type === 'webgl2') {
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
): boolean {
  const inst = instances.get(instanceKey(spriteNum));
  if (!inst) {
    console.warn(`[Flash mouse] no instance for sprite#${spriteNum}; known=`,
      Array.from(instances.keys()));
    return false;
  }

  let canvasX = localX;
  let canvasY = localY;
  if (inst.canvas) {
    const rect = inst.canvas.getBoundingClientRect();
    if (rect.width > 0 && rect.height > 0) {
      canvasX = (localX / rect.width) * inst.canvas.width;
      canvasY = (localY / rect.height) * inst.canvas.height;
    }
  }

  const player = inst.rufflePlayer as
    | { dirplayer_dispatchPointer?: (t: string, x: number, y: number) => boolean }
    | undefined;
  if (typeof player?.dirplayer_dispatchPointer !== 'function') {
    console.warn(`[Flash mouse] sprite#${spriteNum} player has no dirplayer_dispatchPointer; ` +
      `bridgeId=${inst.bridgeId}, player keys=`, player ? Object.keys(player as object) : 'no player');
    return false;
  }
  return !!player.dirplayer_dispatchPointer(type, canvasX, canvasY);
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
    if (typeof url !== 'string' || !url.startsWith('event:')) {
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

  // Capture the about-to-die instance's current frame BEFORE destroy so
  // we can carry it across a member change. storyscramble does
  // `gotoFrame(sprite 2, 31)` in BS38 while sprite 2 holds member 1:31;
  // when the scene change swaps sprite 2 to member 1:45 (bubble) we
  // destroy the old instance and have no record of frame 31. Stash it
  // here keyed by sprite number, then check below after sibling
  // inheritance. Mimics Director's per-sprite frame property which
  // survives member swaps.
  const priorIntendedFrame: number | null = (() => {
    const prior = instances.get(key);
    if (!prior) return null;
    try {
      const f = parseInt(prior.rufflePlayer.GetVariable?.('/:_currentframe') || '0', 10);
      return f >= 1 ? f : null;
    } catch { return null; }
  })();

  // Destroy existing instance for this sprite if any.
  destroyFlashInstance(spriteNum);

  // Playhead inheritance — preserve Director's "Flash member state is
  // shared across sprites using that member" semantic the old cast-keyed
  // architecture used to give us for free. With per-sprite instances,
  // sprite 2 in storyscramble's main scene would spin up a fresh Ruffle
  // player and autoplay the bubble SWF (1:45) from frame 1→11, hiding
  // the frame-21 state sprite 17 already advanced it to. Scan live
  // siblings here for the same member and capture their current frame;
  // we'll seed the new instance to that frame in the finally block
  // below, after the SWF has loaded and AS initialisation finishes.
  let inheritedFrame: number | null = null;
  const siblings = Array.from(instances.values());
  for (const sibling of siblings) {
    if (sibling.castLib === castLib && sibling.castMember === castMember) {
      try {
        const cur = parseInt(
          sibling.rufflePlayer.GetVariable?.('/:_currentframe') || '0', 10
        );
        if (cur >= 1) {
          inheritedFrame = cur;
          break;
        }
      } catch { /* getVariable failed (bridge mode etc.) — fall through to autoplay */ }
    }
  }

  // Per-sprite intent wins over sibling inheritance: if BS38 set this
  // sprite to frame N before the member changed, the new member should
  // also land at frame N (Director's per-sprite frame attribute).
  if (priorIntendedFrame !== null) {
    inheritedFrame = priorIntendedFrame;
  }

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

  // Hidden container for Ruffle - pixels are read back and composited into dirplayer's canvas
  const container = document.createElement('div');
  container.style.position = 'absolute';
  container.style.left = '-9999px';
  container.style.top = '-9999px';
  container.style.width = `${width}px`;
  container.style.height = `${height}px`;
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
    player.style.width = `${width}px`;
    player.style.height = `${height}px`;
    container.appendChild(player);
  } else {
    // Direct mode (page-loaded polyfill, same world as Ruffle).
    const ruffle = await loadRuffle();
    player = ruffle.createPlayer();
    player.style.width = `${width}px`;
    player.style.height = `${height}px`;
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
    width,
    height,
    animFrameId: null,
    ready: false,
    pausedAtStart,
  };

  instances.set(key, instance);

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
    logLevel: 'info',
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
    renderer: 'canvas',  // Force Canvas2D so we can read pixels back
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
  if (pausedAtStart) {
    try {
      // Two-step pin: gotoAndPlay so Ruffle paints frame 1, wait one
      // browser RAF so the paint lands on the canvas, then gotoAndStop
      // to halt the MovieClip. Mirrors applyGotoAndPin's strategy
      // (Ruffle skips paint for already-stopped MovieClips).
      player.GotoFrame(1, false);
      await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
      player.GotoFrame(1, true);
    } catch (e) {
      console.warn(`[Flash] Instance ${key} pausedAtStart pin failed:`, e);
    }
  }

  // Director's Flash Asset Xtra intercepts `getURL("event: …")` and routes
  // the body into the host movie's event chain (e.g. `event: send #done`
  // fires the `done` handler). Register the handler speculatively — when
  // the Ruffle-fork patch is present, navigations whose URL starts with
  // `event:` get routed into dispatch_flash_event and the real open is
  // suppressed. Otherwise it's a safe no-op.
  registerEventUrlHandler(player, castLib, castMember);

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

    // Seed the new instance with the frame captured from a sibling
    // sprite using the same member (see scan above). Bubbles in
    // storyscramble's main scene rely on this — sprite 2 picks up
    // sprite 17's frame-21 state instead of autoplaying back to 11.
    //
    // By the time this runs (3s after instance creation), the SWF has
    // already autoplayed 1→11 and AS `stop()` has parked the MovieClip.
    // A bare `GotoFrame(N, false)` against an AS-stopped MovieClip
    // doesn't reliably seek — we have to mirror the playFlash
    // workaround: call `play()` first to flip the player-level paused
    // flag, THEN `GotoFrame(N, false)` to hit MovieClip's goto_frame
    // (which seeks AND clears the stopped flag). Frame N's own AS
    // `stop()` then re-parks the playhead at N.
    const live = instances.get(key);
    // Mark ready BEFORE the seed and queue replay so the internal
    // `live.rufflePlayer.GotoFrame(...)` calls aren't seen as targeting
    // a not-yet-ready instance (they're the seed itself). After this
    // point any Lingo goTo/play/stop calls bypass the queue.
    if (live) live.ready = true;
    if (inheritedFrame !== null && live) {
      try {
        live.rufflePlayer.play();
        live.rufflePlayer.GotoFrame(inheritedFrame, false);
      } catch (e) {
        console.warn(`[Flash] inherit seed failed for ${key}:`, e);
      }
    }

    // Replay any beginSprite-time `gotoFrame(sprite,N)` / `play(sprite)` /
    // `stop(sprite)` Lingo calls that arrived before this instance was
    // created. Runs AFTER the inheritance seed so an explicit
    // `gotoFrame(31)` from BS38 wins over an inherited frame 21.
    flushPendingGoto(spriteNum);
  }
}

/**
 * Capture frames from Ruffle's canvas and send pixel data to dirplayer WASM.
 */
function startFrameCapture(key: string): void {
  const instance = instances.get(key);
  if (!instance) return;

  function captureFrame() {
    const inst = instances.get(key);
    if (!inst || !inst.canvas) return;

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
}

/**
 * Get a Flash variable from a Ruffle instance.
 * Called from WASM via window.dirplayer_ruffleGetVariable.
 */
function translateLevel0(path: string): string {
  if (path.startsWith('_level0')) {
    return '_root' + path.substring('_level0'.length);
  }
  // Director's getVariable/setVariable/callFunction accept bare names
  // and resolve them as `_root.<name>`. Ruffle's GetVariable is strict
  // — a bare path returns Void. Rewrite bare names (no path
  // separator, no leading `_root`/`_level0`/`this`/`super`) into the
  // `/:<name>` form Ruffle handles, so e.g. JunkbotUndercover's
  // `getVariable(sprite(15), "sceneNumber")` actually reads the SWF's
  // `_root.sceneNumber`.
  const isBareName = !/[\/:.]/.test(path) && !/^_?(root|level\d+|this|super|parent)\b/i.test(path);
  if (isBareName && path.length > 0) {
    return '/:' + path;
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
  if (!instance) {
    console.warn(`ruffleGetVariable: no instance for sprite#${spriteNum}`);
    flashAccessBeforeReady = true;
    return null;
  }

  try {
    return instance.rufflePlayer.GetVariable(translateLevel0(path));
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
  if (!instance) {
    console.warn(`ruffleSetVariable: no instance for sprite#${spriteNum}`);
    return false;
  }

  try {
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
  | { kind: 'rewind' };
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

/**
 * Numeric seek that LEAVES THE PLAYHEAD RUNNING — matches Director's
 * `sprite(N).gotoFrame(frame)` method semantic (the Flash Asset Xtra's
 * `gotoFrame` calls `gotoAndPlay`, not `gotoAndStop`). Used by
 * `sprite.gotoFrame(N)` Lingo method calls; the SWF continues playing
 * from frame N afterwards. Required for mello's Fire/Marshmello where
 * each label points to an animated frame range that must keep playing.
 */
function applyGotoPlay(instance: FlashInstance, frame: number): void {
  try {
    instance.rufflePlayer.GotoFrame(frame, false);
  } catch (e) {
    console.warn(`[Flash gotoPlay] error:`, e);
  }
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
  const player = instance.rufflePlayer as unknown as {
    CallFunction?: (path: string, args: unknown[]) => unknown;
  };
  if (typeof player.CallFunction !== 'function') {
    console.warn(`[Flash gotoLabelPlay] sprite#${instance.spriteNum} player has no CallFunction`);
    return;
  }
  try {
    player.CallFunction('_root.gotoAndPlay', [label]);
  } catch (e) {
    console.warn(`[Flash gotoLabelPlay] error:`, e);
  }
}

/**
 * Numeric seek that STOPS at the target frame (gotoAndStop semantic).
 * Director's `sprite.frame = N` property setter uses this — required for
 * storyscramble's tiles which must pin at a poster frame (2/4/6) and not
 * advance through the SWF's other frames. Uses a microtask-pin to avoid
 * a render race against Ruffle's RAF; see schedulePin for the rationale.
 */
function applyGotoAndPin(instance: FlashInstance, frame: number): void {
  try {
    // Goto+play first forces Ruffle to render frame N synchronously
    // (Ruffle skips paint for already-stopped MovieClips, so a bare
    // gotoAndStop wouldn't show frame N until something else triggered
    // a repaint).
    instance.rufflePlayer.GotoFrame(frame, false);
  } catch (e) {
    console.warn(`[Flash goto] play-step error:`, e);
    return;
  }
  pinTarget.set(instance.spriteNum, frame);
  schedulePin(instance, frame);
}

/**
 * Label variant of applyGotoAndPin — gotoAndPlay followed by stop via
 * AS1 CallFunction.
 */
function applyGotoLabelAndPin(instance: FlashInstance, label: string): void {
  const player = instance.rufflePlayer as unknown as {
    CallFunction?: (path: string, args: unknown[]) => unknown;
  };
  if (typeof player.CallFunction !== 'function') {
    console.warn(`[Flash gotoLabel] sprite#${instance.spriteNum} player has no CallFunction`);
    return;
  }
  try {
    player.CallFunction('_root.gotoAndPlay', [label]);
  } catch (e) {
    console.warn(`[Flash gotoLabel] play-step error:`, e);
    return;
  }
  queueMicrotask(() => {
    try {
      player.CallFunction!('_root.stop', []);
    } catch (e) {
      console.warn(`[Flash gotoLabel] pin-step error:`, e);
    }
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
    try {
      instance.rufflePlayer.GotoFrame(frame, true);
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
          applyGotoAndPin(instance, op.frame);
          break;
        case 'gotoLabel':
          applyGotoLabelAndPin(instance, op.label);
          break;
        case 'play':
          // Queued `play` ops fire only on instances we just created —
          // at which point autoplay has already started the SWF. Re-
          // firing play() / GotoFrame(_currentframe, false) here is
          // redundant AND visually disruptive: it seeks back to the
          // queued goto target and makes the bubble grow animation
          // restart from frame 1 after autoplay already played it.
          //
          // All we need to do is cancel any pending pin from a queued
          // goto earlier in the same queue so the deferred stop
          // doesn't undo the implicit play.
          pinTarget.delete(spriteNum);
          break;
        case 'stop':
          pinTarget.delete(spriteNum);
          instance.rufflePlayer.pause();
          break;
        case 'rewind':
          pinTarget.delete(spriteNum);
          instance.rufflePlayer.GotoFrame(1, true);
          break;
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
  if (isNumeric) {
    applyGotoPlay(instance, parseInt(trimmed, 10));
  } else {
    applyGotoLabelPlay(instance, frameOrLabel);
  }
}

/**
 * `the frame of sprite N = X` property setter — seek and STOP (Director's
 * frame setter is gotoAndStop-semantic). Called from WASM via
 * window.dirplayer_ruffleGoToFrameAndStop. Used for poster-frame Flash
 * sprites like storyscramble's tiles where the playhead must not advance
 * past the chosen frame.
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
    return;
  }
  if (isNumeric) {
    applyGotoAndPin(instance, parseInt(trimmed, 10));
  } else {
    applyGotoLabelAndPin(instance, frameOrLabel);
  }
}

/**
 * Call a Flash function on a Ruffle instance.
 * Called from WASM via window.dirplayer_ruffleCallFunction.
 */
function callFunction(spriteNum: number, path: string, argsXml: string): string | null {
  const key = instanceKey(spriteNum);
  const instance = instances.get(key);
  if (!instance) {
    console.warn(`ruffleCallFunction: no instance for sprite#${spriteNum}`);
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
  try {
    instance.rufflePlayer.pause();
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
  try {
    instance.rufflePlayer.play();
    const cur = parseInt(
      instance.rufflePlayer.GetVariable?.('/:_currentframe') || '1', 10
    ) || 1;
    instance.rufflePlayer.GotoFrame(cur, false);
  } catch (e) {
    console.warn(`rufflePlay error:`, e);
  }
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
  try {
    instance.rufflePlayer.GotoFrame(1, true);
  } catch (e) {
    console.warn(`ruffleRewind error:`, e);
  }
}

/**
 * Check if a Ruffle instance is currently playing.
 */
function isPlaying(spriteNum: number): boolean {
  const key = instanceKey(spriteNum);
  const instance = instances.get(key);
  if (!instance) return false;
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
    // Use GetVariable to read _totalframes
    return parseInt(instance.rufflePlayer.GetVariable("/:_totalframes") || "0", 10);
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
    return parseInt(instance.rufflePlayer.GetVariable("/:_currentframe") || "0", 10);
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
  try {
    // callFrame in Director executes the actions on a given frame
    // Best approximation: go to that frame (which runs its scripts) and stop
    instance.rufflePlayer.GotoFrame(frame, true);
  } catch (e) {
    console.warn(`ruffleCallFrame error:`, e);
  }
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
 * Perform a hit test on a Ruffle instance.
 * Returns true if the point (in Flash coordinates) hits content.
 */
function hitTest(spriteNum: number, x: number, y: number): boolean {
  const key = instanceKey(spriteNum);
  const instance = instances.get(key);
  if (!instance) return false;
  try {
    // Use CallFunction to invoke _root.hitTest(x, y, true)
    const result = instance.rufflePlayer.CallFunction("_root.hitTest", [x, y, true]);
    return result === true || result === "true" || result === 1;
  } catch (e) {
    return false;
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
    return instance.rufflePlayer.GetVariable(path)?.toString() ?? null;
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
    instance.rufflePlayer.SetVariable(path, value);
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
      instance.rufflePlayer.SetVariable(`${target}:_visible`, "1");
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
  win.dirplayer_ruffleGetVariable = getVariable;
  win.dirplayer_flashInstances = instances;
  win.dirplayer_ruffleSetVariable = setVariable;
  win.dirplayer_ruffleCallFunction = callFunction;
  win.dirplayer_ruffleGoToFrame = goToFrame;
  win.dirplayer_ruffleGoToFrameAndStop = goToFrameAndStop;
  win.dirplayer_ruffleStop = stopFlash;
  win.dirplayer_rufflePlay = playFlash;
  win.dirplayer_ruffleRewind = rewindFlash;
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

  // Expose flash loading state for WASM frame loop to check.
  // Also returns true if scripts tried to access a Flash instance that doesn't exist yet
  // (the rendering loop will dispatch it shortly).
  win.dirplayer_isFlashLoading = () => flashLoadingCount > 0 || flashAccessBeforeReady;

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
