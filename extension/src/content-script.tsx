import { initPolyfill, PolyfillConfig } from '../../polyfill/src/core';
// Note: the Shockwave plugin polyfill (MAIN world) and the Ruffle fork
// bundle (ISOLATED world, same as us) are registered by the service
// worker via chrome.scripting.registerContentScripts at document_start
// — see extension/src/background.ts. Going through that API bypasses
// the page's CSP `script-src` restrictions, which block inline
// scripts and `unsafe-eval` from regular content-script injection.


// Pre-seed the dirplayer_RufflePlayer config so the bundle picks it up
// when it loads. Matches what standalone.tsx does for the page-loaded
// polyfill build.
{
  const win = window as any;
  win.dirplayer_RufflePlayer = win.dirplayer_RufflePlayer || {};
  win.dirplayer_RufflePlayer.config = {
    ...(win.dirplayer_RufflePlayer.config || {}),
    allowNetworking: 'all',
  };
}

// Get asset URLs from the Chrome extension
const config: PolyfillConfig = {
  wasmUrl: chrome.runtime.getURL('vm-rust/pkg/vm_rust_bg.wasm'),
  systemFontUrl: chrome.runtime.getURL('charmap-system.png'),
};

// Initialize the polyfill with extension version for priority negotiation
const version = chrome.runtime.getManifest().version;
initPolyfill(config, version, 'extension');
