/**
 * FlashPlayerManager - Bridges Ruffle (Flash player) with dirplayer-rs
 *
 * Manages Ruffle player instances for Flash cast members, reads rendered frames,
 * and sends pixel data to dirplayer's WASM rendering pipeline so Flash content
 * can be composited with Director sprites (Director sprites can layer on top).
 */

import { update_flash_frame, trigger_lingo_callback_on_script } from 'vm-rust';

interface FlashInstance {
  castLib: number;
  castMember: number;
  rufflePlayer: any; // RufflePlayerElement
  container: HTMLDivElement;
  canvas: HTMLCanvasElement | null;
  width: number;
  height: number;
  animFrameId: number | null;
}

// Map of "castLib:castMember" -> FlashInstance
const instances = new Map<string, FlashInstance>();

// Intercept fetch to rewrite URLs based on fetchRewriteRules config.
// On the server (empty rules), no rewriting happens — webserver should handle proxying.
function getFetchRewriteRules(): Array<{pathPrefix: string, targetHost: string, targetPort: string, targetProtocol: string}> {
  const win = window as any;
  if (win.__dirplayerFlashConfig?.fetchRewriteRules) {
    return win.__dirplayerFlashConfig.fetchRewriteRules;
  }
  // Local dev fallback
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
  if (win.__dirplayerFlashConfig?.socketProxy) {
    return win.__dirplayerFlashConfig.socketProxy;
  }
  // Local dev fallback
  return [];
}

function instanceKey(castLib: number, castMember: number): string {
  return `${castLib}:${castMember}`;
}

/**
 * Load Ruffle library. Assumes ruffle is available at a known path or via CDN.
 * Returns the RufflePlayer constructor.
 */
let rufflePromise: Promise<any> | null = null;

async function loadRuffle(): Promise<any> {
  if (rufflePromise) return rufflePromise;

  rufflePromise = (async () => {
    // Try to get Ruffle from window (if loaded via script tag)
    const win = window as any;
    if (win.RufflePlayer) {
      const ruffle = win.RufflePlayer.newest();
      return ruffle;
    }

    throw new Error('Ruffle not found. Ensure ruffle.js is loaded via a script tag.');
  })();

  return rufflePromise;
}

/**
 * Create a Ruffle player instance for a Flash cast member.
 * The player renders to a hidden container; frames are read back and sent to WASM.
 */
export async function createFlashInstance(
  castLib: number,
  castMember: number,
  swfData: Uint8Array,
  width: number,
  height: number,
): Promise<void> {
  const key = instanceKey(castLib, castMember);

  // Destroy existing instance if any
  destroyFlashInstance(castLib, castMember);

  const ruffle = await loadRuffle();

  // Hidden container for Ruffle - pixels are read back and composited into dirplayer's canvas
  const container = document.createElement('div');
  container.style.position = 'absolute';
  container.style.left = '-9999px';
  container.style.top = '-9999px';
  container.style.width = `${width}px`;
  container.style.height = `${height}px`;
  container.style.overflow = 'hidden';
  document.body.appendChild(container);

  // Create and configure the Ruffle player element
  const player = ruffle.createPlayer();
  player.style.width = `${width}px`;
  player.style.height = `${height}px`;
  container.appendChild(player);

  const instance: FlashInstance = {
    castLib,
    castMember,
    rufflePlayer: player,
    container,
    canvas: null,
    width,
    height,
    animFrameId: null,
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

  // Load the SWF data into Ruffle
  // Use .ruffle().load() API as per Ruffle's selfhosted interface
  const ruffleInstance = player.ruffle();
  await ruffleInstance.load({
    data: actualSwfData,
    allowScriptAccess: true,
    openUrlMode: 'deny',
    autoplay: 'on',
    unmuteOverlay: 'hidden',
    logLevel: 'info',
    splashScreen: false,
    wmode: 'transparent',
    renderer: 'canvas',  // Force Canvas2D so we can read pixels back
    socketProxy: getSocketProxyConfig(),
  });

  // Find the internal canvas element that Ruffle renders to
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
  }, 500);
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
          update_flash_frame(inst.castLib, inst.castMember, width, height, new Uint8Array(imageData.data.buffer));
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
export function destroyFlashInstance(castLib: number, castMember: number): void {
  const key = instanceKey(castLib, castMember);
  const instance = instances.get(key);
  if (!instance) return;

  if (instance.animFrameId !== null) {
    cancelAnimationFrame(instance.animFrameId);
  }

  try {
    instance.rufflePlayer.remove();
  } catch (e) {
    // Ignore cleanup errors
  }

  instance.container.remove();
  instances.delete(key);
}

/**
 * Get a Flash variable from a Ruffle instance.
 * Called from WASM via window.ruffleGetVariable.
 */
function translateLevel0(path: string): string {
  if (path.startsWith('_level0')) {
    return '_root' + path.substring('_level0'.length);
  }
  return path;
}

function getVariable(castLib: number, castMember: number, path: string): string | null {
  const key = instanceKey(castLib, castMember);
  const instance = instances.get(key);
  if (!instance) {
    console.warn(`ruffleGetVariable: no instance for ${key}`);
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
 * Called from WASM via window.ruffleSetVariable.
 */
function setVariable(castLib: number, castMember: number, path: string, value: string): boolean {
  const key = instanceKey(castLib, castMember);
  const instance = instances.get(key);
  if (!instance) {
    console.warn(`ruffleSetVariable: no instance for ${key}`);
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
 * Go to a specific frame on a Ruffle instance.
 * Called from WASM via window.ruffleGoToFrame.
 * frame is 1-based (Director convention).
 */
function goToFrame(castLib: number, castMember: number, frame: number): void {
  const key = instanceKey(castLib, castMember);
  const instance = instances.get(key);
  if (!instance) {
    console.warn(`ruffleGoToFrame: no instance for ${key}`);
    return;
  }

  try {
    instance.rufflePlayer.GotoFrame(frame, false);
  } catch (e) {
    console.warn(`ruffleGoToFrame error:`, e);
  }
}

/**
 * Call a Flash function on a Ruffle instance.
 * Called from WASM via window.ruffleCallFunction.
 */
function callFunction(castLib: number, castMember: number, path: string, argsXml: string): string | null {
  const key = instanceKey(castLib, castMember);
  const instance = instances.get(key);
  if (!instance) {
    console.warn(`ruffleCallFunction: no instance for ${key}`);
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
function stopFlash(castLib: number, castMember: number): void {
  const key = instanceKey(castLib, castMember);
  const instance = instances.get(key);
  if (!instance) return;
  try {
    instance.rufflePlayer.pause();
  } catch (e) {
    console.warn(`ruffleStop error:`, e);
  }
}

/**
 * Start/resume playback of a Ruffle instance.
 */
function playFlash(castLib: number, castMember: number): void {
  const key = instanceKey(castLib, castMember);
  const instance = instances.get(key);
  if (!instance) return;
  try {
    instance.rufflePlayer.play();
  } catch (e) {
    console.warn(`rufflePlay error:`, e);
  }
}

/**
 * Rewind a Ruffle instance to frame 1 and stop.
 */
function rewindFlash(castLib: number, castMember: number): void {
  const key = instanceKey(castLib, castMember);
  const instance = instances.get(key);
  if (!instance) return;
  try {
    instance.rufflePlayer.GotoFrame(1, true);
  } catch (e) {
    console.warn(`ruffleRewind error:`, e);
  }
}

/**
 * Check if a Ruffle instance is currently playing.
 */
function isPlaying(castLib: number, castMember: number): boolean {
  const key = instanceKey(castLib, castMember);
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
function getFrameCount(castLib: number, castMember: number): number {
  const key = instanceKey(castLib, castMember);
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
function getCurrentFrame(castLib: number, castMember: number): number {
  const key = instanceKey(castLib, castMember);
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
function callFrame(castLib: number, castMember: number, frame: number): void {
  const key = instanceKey(castLib, castMember);
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
function findLabel(castLib: number, castMember: number, label: string): number {
  const key = instanceKey(castLib, castMember);
  const instance = instances.get(key);
  if (!instance) return -1;
  try {
    // Use TellTarget + GetVariable to find label frame
    // Flash's /:_framesloaded and label lookup via GetVariable
    const result = instance.rufflePlayer.GetVariable("/:_currentframe");
    // Try navigating to the label using SetVariable approach
    // Unfortunately there's no direct label lookup in Flash Player API
    // Best effort: use CallFunction if the SWF has a findLabel method
    return -1;
  } catch (e) {
    return -1;
  }
}

/**
 * Perform a hit test on a Ruffle instance.
 * Returns true if the point (in Flash coordinates) hits content.
 */
function hitTest(castLib: number, castMember: number, x: number, y: number): boolean {
  const key = instanceKey(castLib, castMember);
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
function getFlashProperty(castLib: number, castMember: number, target: string, propNum: number): string | null {
  const key = instanceKey(castLib, castMember);
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
function setFlashProperty(castLib: number, castMember: number, target: string, propNum: number, value: string): void {
  const key = instanceKey(castLib, castMember);
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
function tellTarget(castLib: number, castMember: number, target: string, action: string): void {
  const key = instanceKey(castLib, castMember);
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

  // Call Ruffle's WASM export to register the callback in LINGO_CALLBACKS
  const win = window as any;
  if (win.ruffleRegisterLingoCallback) {
    win.ruffleRegisterLingoCallback(
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
    console.warn('ruffleRegisterLingoCallback not available on window (Ruffle not loaded yet?)');
  }
}

/**
 * Register global JS functions that the WASM module calls into.
 */
export function initFlashBridge(): void {
  const win = window as any;
  win.ruffleGetVariable = getVariable;
  win._flashInstances = instances;
  win.ruffleSetVariable = setVariable;
  win.ruffleCallFunction = callFunction;
  win.ruffleGoToFrame = goToFrame;
  win.ruffleStop = stopFlash;
  win.rufflePlay = playFlash;
  win.ruffleRewind = rewindFlash;
  win.ruffleIsPlaying = isPlaying;
  win.ruffleGetFrameCount = getFrameCount;
  win.ruffleGetCurrentFrame = getCurrentFrame;
  win.ruffleCallFrame = callFrame;
  win.ruffleFindLabel = findLabel;
  win.ruffleHitTest = hitTest;
  win.ruffleGetFlashProperty = getFlashProperty;
  win.ruffleSetFlashProperty = setFlashProperty;
  win.ruffleTellTarget = tellTarget;
  win.ruffleRegisterLingoCallback_dirplayer = registerLingoCallback;

  // Expose dirplayer's WASM exports as window.wasmModule so that Ruffle's
  // wasm_bindgen extern (js_namespace = wasmModule) can resolve
  // trigger_lingo_callback_on_script back to dirplayer's WASM export.
  // Expose as global function for Ruffle's wasm_bindgen extern
  // Ruffle sends args as a JSON array of base64-encoded JSON values.
  // Decode them to native JS values before passing to WASM.
  win.triggerLingoCallbackOnScript = (castLib: number, castMember: number, handlerName: string, argsJson: string, flashCastLib: number, flashCastMember: number) => {
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

  // Ensure RufflePlayer config is set up before any instances are created
  win.RufflePlayer = win.RufflePlayer || {};
  win.RufflePlayer.config = {
    ...(win.RufflePlayer.config || {}),
    allowNetworking: 'all',
  };
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
}

/**
 * Apply external configuration. Called from polyfill's configureFlash().
 * Stores config on window so getSocketProxyConfig() and other code can read it.
 */
export function configureFlashManager(partial: Partial<FlashManagerConfig>): void {
  const win = window as any;
  const existing = win.__dirplayerFlashConfig || {};
  win.__dirplayerFlashConfig = { ...existing, ...partial };

  // Set up the global socket URL resolver for the Multiuser Xtra (WASM side)
  if (partial.socketProxy && partial.socketProxy.length > 0) {
    win.dirplayerResolveSocketUrl = (host: string, port: number): string => {
      for (const entry of partial.socketProxy!) {
        if (entry.host === host && entry.port === port) {
          return entry.proxyUrl;
        }
      }
      return '';
    };
  }
}
