// Main-world bridge that exposes the dirplayer Ruffle fork to the
// isolated-world content script via `window.postMessage`. Loaded
// alongside `ruffle/dirplayer_ruffle.js` by the extension service
// worker (chrome.scripting.registerContentScripts world:'MAIN').
//
// Why a bridge: in Chrome MV3 isolated worlds `window.customElements`
// is null, so Ruffle's `customElements.define(...)` registration of
// its player element fails. Running Ruffle in the page's main world
// fixes the registration but means dirplayer (still in the isolated
// world) can no longer call Ruffle's methods directly — JS objects
// don't cross worlds. The bridge ferries method calls and property
// reads via `postMessage`.

(function () {
  'use strict';
  if (window.__dirplayerRuffleBridgeHostInstalled) return;
  window.__dirplayerRuffleBridgeHostInstalled = true;

  // Per-page-load registry of active Ruffle player instances.
  // Created via `createPlayer`, looked up by id for every subsequent
  // method/property request, and pruned via `destroyPlayer`.
  /** @type {Map<string, any>} */
  const players = new Map();
  let nextId = 1;

  // Pre-seed the dirplayer_RufflePlayer config so the Ruffle bundle
  // picks it up when it boots. Mirrors what standalone.tsx does for
  // the page-loaded polyfill build, plus an extension-specific
  // `publicPath` so chunk loads (the Ruffle WASM) resolve against the
  // chrome-extension URL instead of the host page. Ruffle's bundle
  // replaces the webpack runtime `r.p` with a function that reads
  // `config.publicPath` first — setting `__webpack_public_path__`
  // alone isn't enough.
  if (!window.dirplayer_RufflePlayer) {
    window.dirplayer_RufflePlayer = {};
  }
  window.dirplayer_RufflePlayer.config = Object.assign(
    {},
    window.dirplayer_RufflePlayer.config || {},
    {
      allowNetworking: 'all',
      // The chrome-extension URL is stamped on `<html>` by the
      // isolated-world pre-init content script (extension/src/
      // dirplayer-pre-init.ts) — DOM is shared across worlds, JS
      // globals aren't. Falls back to the legacy global for any caller
      // that still uses it. Absent on the page-loaded standalone
      // polyfill, where Ruffle's normal currentScript-based detection
      // works correctly.
      ...(function () {
        const fromAttr = document.documentElement
          && document.documentElement.getAttribute('data-dirplayer-ruffle-url');
        const fromGlobal = window.__dirplayerRufflePublicPath;
        const url = fromAttr || fromGlobal;
        return url ? { publicPath: url } : {};
      })(),
    },
  );

  // Communicate via DOM CustomEvents instead of postMessage. The Wayback
  // Machine's `wombat.js` overrides `window.postMessage` to sanitize
  // origins and silently drops our bridge traffic; CustomEvents propagate
  // through the shared document and are not intercepted. Standalone
  // (non-Wayback) pages also work fine — events are a strict superset of
  // postMessage's broadcast semantics for same-window communication.
  var REQ_EVENT = 'dirplayer-ruffle-bridge-request';
  var RES_EVENT = 'dirplayer-ruffle-bridge-response';
  var EVT_EVENT = 'dirplayer-ruffle-bridge-event';

  function respond(requestId, payload, error) {
    window.dispatchEvent(new CustomEvent(RES_EVENT, {
      detail: {
        requestId: requestId,
        result: payload,
        error: error == null ? null : (error && error.message) || String(error),
      },
    }));
  }

  function announceEvent(playerId, eventName, detail) {
    // Forward player events back to the isolated world. Detail must be
    // structured-cloneable (no DOM elements / functions). We only
    // forward the data we know dirplayer cares about today.
    window.dispatchEvent(new CustomEvent(EVT_EVENT, {
      detail: {
        playerId: playerId,
        eventName: eventName,
        detail: detail,
      },
    }));
  }

  function attachEventForwarders(playerId, player) {
    // Ruffle dispatches custom events on the player element. Forward
    // the ones dirplayer's flashPlayerManager listens for so the
    // bridge client can mirror addEventListener semantics.
    const events = ['loadedmetadata', 'play', 'pause', 'ended', 'error'];
    for (const ev of events) {
      player.addEventListener(ev, (e) => {
        announceEvent(playerId, ev, {
          // Best-effort detail snapshot — most Ruffle events have no
          // useful payload beyond firing.
          metadata: ev === 'loadedmetadata'
            ? safeClone(player.metadata)
            : undefined,
        });
      });
    }
  }

  function safeClone(value) {
    try {
      return JSON.parse(JSON.stringify(value));
    } catch (e) {
      return undefined;
    }
  }

  window.addEventListener(REQ_EVENT, async (ev) => {
    const m = ev.detail;
    if (!m) return;

    const { requestId, method, playerId, methodName, propName, args } = m;
    try {
      switch (method) {
        case 'isReady': {
          const ready = typeof window.dirplayer_RufflePlayer?.newest === 'function';
          respond(requestId, ready);
          break;
        }
        case 'createPlayer': {
          if (typeof window.dirplayer_RufflePlayer?.newest !== 'function') {
            throw new Error('Ruffle bundle has not finished loading');
          }
          const ruffle = window.dirplayer_RufflePlayer.newest();
          const player = ruffle.createPlayer();
          const id = String(nextId++);
          // Stamp the element so the isolated world can find it via
          // querySelector. Append to a temporary hidden parent so it's
          // attached to the document tree (querySelector only matches
          // attached elements) — the isolated-world client moves it
          // into its own offscreen container immediately after.
          player.dataset.dirplayerBridgeId = id;
          player.style.position = 'absolute';
          player.style.left = '-99999px';
          player.style.top = '-99999px';
          (document.body || document.documentElement).appendChild(player);
          players.set(id, player);
          attachEventForwarders(id, player);
          respond(requestId, { playerId: id });
          break;
        }
        case 'callMethod': {
          const player = players.get(playerId);
          if (!player) throw new Error('player not registered: ' + playerId);
          // Special-case `load` — Ruffle's selfhosted entry point is
          // `player.ruffle().load(config)`, not `player.load(config)`.
          // Routing through ruffle() here keeps the bridge client
          // simple (a single `callMethod('load', [config])` call).
          let result;
          if (methodName === 'load') {
            result = player.ruffle().load.apply(player.ruffle(), args || []);
          } else {
            const fn = player[methodName];
            if (typeof fn !== 'function') {
              throw new Error('player has no method ' + methodName);
            }
            result = fn.apply(player, args || []);
          }
          // Methods like `.load(...)` return a promise — await so the
          // response carries the resolved value (or the rejection).
          const resolved = result && typeof result.then === 'function'
            ? await result
            : result;
          respond(requestId, safeClone(resolved));
          break;
        }
        case 'getProp': {
          const player = players.get(playerId);
          if (!player) throw new Error('player not registered: ' + playerId);
          respond(requestId, safeClone(player[propName]));
          break;
        }
        case 'setProp': {
          const player = players.get(playerId);
          if (!player) throw new Error('player not registered: ' + playerId);
          player[propName] = m.value;
          respond(requestId, null);
          break;
        }
        case 'destroyPlayer': {
          const player = players.get(playerId);
          if (player) {
            try { player.remove(); } catch (e) { /* ignore */ }
            players.delete(playerId);
          }
          respond(requestId, null);
          break;
        }
        default:
          throw new Error('unknown bridge method: ' + method);
      }
    } catch (e) {
      respond(requestId, undefined, e);
    }
  });
})();
