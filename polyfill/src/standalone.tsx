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
 * Load Ruffle by injecting a <script> tag for ruffle/ruffle.js
 * relative to the polyfill script's location.
 * Skips if RufflePlayer is already available on window.
 */
function loadRuffle(): Promise<void> {
  if (window.RufflePlayer?.newest) {
    return Promise.resolve();
  }

  // Check for a custom ruffle URL on the script tag: data-ruffle-url="..."
  const customUrl = polyfillScript?.getAttribute('data-ruffle-url');
  const ruffleUrl = customUrl || (getPolyfillBaseUrl() + 'ruffle/ruffle.js');

  // Set up RufflePlayer config before the script loads
  window.RufflePlayer = window.RufflePlayer || {};
  window.RufflePlayer.config = {
    ...(window.RufflePlayer.config || {}),
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
  const config = {
    wasmUrl: getEmbeddedWasmUrl(),
    systemFontUrl: getEmbeddedFontUrl(),
  };

  // Register with version for priority negotiation (deferred init handled inside initPolyfill)
  initPolyfill(config, DIRPLAYER_VERSION, 'polyfill');
}

function init() {
  // Always ensure Ruffle is loaded before initializing
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
