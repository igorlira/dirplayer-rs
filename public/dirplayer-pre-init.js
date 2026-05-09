// Tiny isolated-world content script that runs BEFORE the Ruffle main-
// world bundle. Stamps the chrome-extension URL on `<html>` so the
// main-world Ruffle bundle can pick it up via a DOM-attribute lookup.
// DOM is shared across worlds, but JS globals aren't — and the
// previous webNavigation/executeScript approach was racey: the
// publicPath setter sometimes fired AFTER Ruffle had already parsed,
// leaving chunks loading from the page URL.
//
// Static plain-JS file (in public/) so the service worker's
// chrome.scripting.registerContentScripts call can reference it by a
// stable path; TypeScript files in `extension/src/` are processed by
// the Vite build into hashed `assets/...` filenames that wouldn't
// match the static path string.
//
// Read by the bundled `ruffle/web/packages/selfhosted/js/dirplayer-
// runtime-public-path.js` webpack entry and by the bridge host's
// config.publicPath fallback.

(function () {
  try {
    var url = chrome.runtime.getURL('ruffle/');
    document.documentElement.setAttribute('data-dirplayer-ruffle-url', url);
  } catch (e) {
    console.warn('[DirPlayer] failed to stamp ruffle URL:', e);
  }
})();
