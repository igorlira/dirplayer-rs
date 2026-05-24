import { initPolyfill, PolyfillConfig } from '../../polyfill/src/core';
import { loadDefaultXtraRegistry, setXtraHostBase } from 'dirplayer-js-api';
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

// Tell the xtra plugin loader where the extension's resources live.
// Registry entries prefixed with "~/" then resolve against the
// extension's root URL — so the same `~/bobba_xtra.wasm` entry in
// xtra-registry.json works for dev (resolves against document.baseURI)
// and the extension (resolves against chrome-extension://<id>/).
//
// We keep the wasms + xtra-registry.json at the extension root rather
// than under xtras/ so vite's default publicDir copy from public/
// lands them at the right path with no build-config gymnastics — and
// the dev / extension / electron file layouts stay identical.
setXtraHostBase(chrome.runtime.getURL(''));
// Fire-and-forget: kicks off the JSON fetch in parallel with polyfill
// init. Movies that load after the JSON arrives pick up its entries;
// the convention fallback covers anything not pinned in the JSON.
loadDefaultXtraRegistry();

// Initialize the polyfill with extension version for priority negotiation
const version = chrome.runtime.getManifest().version;
initPolyfill(config, version, 'extension');
