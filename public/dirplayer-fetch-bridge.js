// MAIN-world fetch bridge.
//
// The extension's player runs in the ISOLATED world, so it gets the browser's
// own `fetch` — never the page's. Archive front-ends serve their content by
// REPLACING `window.fetch` and rewriting requests for the original host
// (ooooooooo.ooo serves `http://candystand.com/shock/csmgmain.dcr` out of its
// own store that way). Those URLs only resolve through the page's fetch; the
// isolated world's native fetch sends a real request to a host that is long
// gone, and the movie fails to load.
//
// This script runs in the page's world at document_start and performs fetches
// on the isolated world's behalf, so the page's rewrite applies exactly as it
// does for a page-hosted polyfill. It deliberately calls `window.fetch` at
// request time rather than capturing it here — the page installs its rewrite
// after document_start, and capturing early would pin the native one.
(function () {
  if (window.__dirplayerFetchBridgeInstalled) return;
  window.__dirplayerFetchBridgeInstalled = true;

  var TAG = 'dirplayer-fetch-bridge';

  // Announce ourselves on the DOM, which both worlds share. Module state does
  // not cross the boundary, so without this the isolated world could only
  // discover the bridge by messaging it and waiting for a reply that never
  // comes on browsers where MAIN-world registration isn't available (Firefox) —
  // paying a timeout on the first request of every movie.
  try {
    document.documentElement.setAttribute('data-dirplayer-fetch-bridge', '1');
  } catch (e) {
    /* document.documentElement always exists at document_start; ignore */
  }

  // Is the page's fetch still the browser's own? Checked per request, because
  // a page can install its rewrite at any point before the movie loads.
  function isPageFetchPatched() {
    try {
      return !/\[native code\]/.test(Function.prototype.toString.call(window.fetch));
    } catch (e) {
      return false;
    }
  }

  window.addEventListener('message', function (event) {
    // Same-window messages only — this is a world bridge, not a frame bridge.
    if (event.source !== window) return;
    var msg = event.data;
    if (!msg || msg.__dirplayer !== TAG) return;

    if (msg.kind === 'probe') {
      window.postMessage(
        { __dirplayer: TAG, kind: 'probe-result', id: msg.id, patched: isPageFetchPatched() },
        '*',
      );
      return;
    }

    if (msg.kind !== 'request') return;

    var reply = function (payload) {
      payload.__dirplayer = TAG;
      payload.kind = 'response';
      payload.id = msg.id;
      window.postMessage(payload, '*');
    };

    try {
      var init = { method: msg.method || 'GET' };
      if (msg.headers) init.headers = msg.headers;
      if (msg.body != null) init.body = msg.body;

      window
        .fetch(msg.url, init)
        .then(function (res) {
          return res.arrayBuffer().then(function (buf) {
            reply({
              ok: res.ok,
              status: res.status,
              statusText: res.statusText || '',
              contentType: (res.headers && res.headers.get('content-type')) || '',
              buffer: buf,
            });
          });
        })
        .catch(function (e) {
          reply({ ok: false, status: 0, error: String((e && e.message) || e) });
        });
    } catch (e) {
      reply({ ok: false, status: 0, error: String((e && e.message) || e) });
    }
  });
})();
