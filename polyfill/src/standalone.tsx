import { initPolyfill } from './core';
import { getEmbeddedWasmUrl, getEmbeddedFontUrl } from './embedded-loader';

declare const DIRPLAYER_VERSION: string;

// Lightweight config interface — must NOT import from flashPlayerManager.ts
// to avoid pulling in its side effects (fetch intercept, WebGL patch)
// which would collide with the main app's copy.
interface FlashConfig {
  socketProxy?: Array<{host: string, port: number, proxyUrl: string}>;
  fetchRewriteRules?: Array<{pathPrefix: string, targetHost: string, targetPort: string, targetProtocol: string}>;
  renderer?: string;
  logLevel?: string;
  /** When true, skip Ruffle entirely. Lingo Flash calls become no-ops. */
  disableFlash?: boolean;
}

function configureFlash(partial: FlashConfig): void {
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

declare global {
  interface Window {
    DirPlayer: {
      init: () => void;
      configureFlash: (config: FlashConfig) => void;
    };
    RufflePlayer: any;
  }
}

/**
 * Resolve the base URL of the polyfill script itself,
 * so we can load sibling assets (like ruffle/) relative to it.
 */
const polyfillScript = document.currentScript as HTMLScriptElement | null;
function getPolyfillBaseUrl(): string {
  if (polyfillScript?.src) {
    return polyfillScript.src.substring(0, polyfillScript.src.lastIndexOf('/') + 1);
  }
  return '';
}

/**
 * Load Ruffle by injecting a <script> tag for ruffle/dirplayer_ruffle.js
 * relative to the polyfill script's location. The loader file is renamed
 * (from upstream's `ruffle.js`) so the fork doesn't collide with stock
 * Ruffle if both bundles are on the same page (e.g. via a browser
 * extension that ships its own ruffle.js).
 *
 * Skips if dirplayer's Ruffle fork is already on window. The fork
 * namespaces its global as `dirplayer_RufflePlayer` so it doesn't collide
 * with stock Ruffle.
 */
function loadRuffle(): Promise<void> {
  const win = window as any;
  if (win.dirplayer_RufflePlayer?.newest) {
    return Promise.resolve();
  }

  // Check for a custom ruffle URL on the script tag: data-ruffle-url="..."
  const customUrl = polyfillScript?.getAttribute('data-ruffle-url');
  const ruffleUrl = customUrl || (getPolyfillBaseUrl() + 'ruffle/dirplayer_ruffle.js');

  // Set up dirplayer_RufflePlayer config before the script loads
  win.dirplayer_RufflePlayer = win.dirplayer_RufflePlayer || {};
  win.dirplayer_RufflePlayer.config = {
    ...(win.dirplayer_RufflePlayer.config || {}),
    allowNetworking: 'all',
  };

  return new Promise<void>((resolve, reject) => {
    const script = document.createElement('script');
    script.src = ruffleUrl;
    script.onload = () => resolve();
    script.onerror = () => {
      console.warn(`[DirPlayer] Failed to load Ruffle from ${ruffleUrl} — Flash content will not work`);
      resolve(); // Don't block dirplayer init if Ruffle fails
    };
    document.head.appendChild(script);
  });
}

function initCore() {
  const requireClick = polyfillScript?.hasAttribute('data-require-click') ?? false;
  const config = {
    wasmUrl: getEmbeddedWasmUrl(),
    systemFontUrl: getEmbeddedFontUrl(),
    requireClickToPlay: requireClick,
  };

  // Register with version for priority negotiation (deferred init handled inside initPolyfill)
  initPolyfill(config, DIRPLAYER_VERSION, 'polyfill');
}

function init() {
  // `data-disable-flash` on the polyfill <script> tag completely skips
  // Ruffle. The flag is also written to __dirplayerFlashConfig so
  // flashPlayerManager.createFlashInstance() short-circuits cleanly
  // instead of throwing "Ruffle not found" for every Flash member.
  const disableFlash = polyfillScript?.hasAttribute('data-disable-flash') ?? false;
  if (disableFlash) {
    const win = window as any;
    win.__dirplayerFlashConfig = {
      ...(win.__dirplayerFlashConfig || {}),
      disableFlash: true,
    };
    console.log('[DirPlayer] data-disable-flash set — skipping Ruffle. Lingo Flash calls will no-op.');
    initCore();
    return;
  }

  // Default: ensure Ruffle is loaded before initializing
  loadRuffle().then(initCore);
}

// Expose the API globally
window.DirPlayer = {
  init,
  configureFlash,
};

// Auto-initialize unless data-manual-init is present on the script tag
if (!polyfillScript?.hasAttribute('data-manual-init')) {
  init();
}
