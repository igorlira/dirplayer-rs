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

  // dirplayer LocalConnection.send bridge (main world → isolated). The Ruffle
  // fork runs HERE (main world) and calls window.dirplayer_localConnectionSend
  // when a SWF does LocalConnection.send; dirplayer's WASM export that dispatches
  // the Lingo setCallback handler lives in the ISOLATED world. Re-fire it there
  // as a DOM event (shared document, wombat-safe). Fire-and-forget — the fork
  // ignores the return. dirplayer_-namespaced so it never collides with stock.
  window.dirplayer_localConnectionSend = function (name, method, argsJson) {
    try {
      window.dispatchEvent(new CustomEvent('dirplayer-lc-send', {
        detail: { name: name, method: method, argsJson: argsJson },
      }));
    } catch (e) { /* ignore */ }
    return false;
  };

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

  // Shared, dirplayer-namespaced node for the SYNCHRONOUS request/response path
  // (GetVariable / SetVariable / CallFunction). Lingo consumes these inline, and
  // the async Promise transport below can't answer synchronously — so the sync
  // handler runs DURING the isolated world's dispatchEvent and writes the result
  // here before dispatch returns. Plain DOM text crosses worlds, is untouched by
  // wombat, and is invisible to stock Ruffle.
  var SYNC_NODE_ID = '__dirplayer_ruffle_sync_channel';

  function writeSyncResult(syncId, result, error) {
    var el = document.getElementById(SYNC_NODE_ID);
    if (!el) return; // client creates it before dispatch; nothing to write to otherwise
    el.textContent = JSON.stringify({
      syncId: syncId,
      result: error == null ? result : undefined,
      error: error == null ? null : ((error && error.message) || String(error)),
    });
  }

  // Synchronous handler for the inline Lingo reads. MUST stay non-async and
  // fully synchronous so the result node is populated before the isolated
  // world's dispatchEvent returns. Ruffle's GetVariable/SetVariable/CallFunction
  // all return synchronously, so this is safe (unlike `load`, which is async and
  // stays on the Promise transport below).
  window.addEventListener(REQ_EVENT, function (ev) {
    var m = ev.detail;
    if (!m || !m.sync) return;
    var syncId = m.syncId;
    try {
      var player = players.get(m.playerId);
      if (!player) throw new Error('player not registered: ' + m.playerId);
      var args = m.args || [];
      var result;
      switch (m.method) {
        case 'getVariableSync':
          result = player.GetVariable(args[0]);
          break;
        case 'setVariableSync':
          result = player.SetVariable(args[0], args[1]);
          if (result === undefined) result = true;
          break;
        case 'callFunctionSync':
          result = player.CallFunction(args[0], args[1] || []);
          break;
        case 'callMethodSync': {
          // Generic sync method call: args = [methodName, methodArgs].
          var fn = player[args[0]];
          result = (typeof fn === 'function') ? fn.apply(player, args[1] || []) : undefined;
          break;
        }
        default:
          throw new Error('unknown sync bridge method: ' + m.method);
      }
      writeSyncResult(syncId, result == null ? null : result, null);
    } catch (e) {
      writeSyncResult(syncId, undefined, e);
    }
  });

  window.addEventListener(REQ_EVENT, async (ev) => {
    const m = ev.detail;
    if (!m) return;
    // Sync requests are handled by the synchronous listener above.
    if (m.sync) return;

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
        case 'registerCallbackForwarders': {
          // Flash->Director callbacks (getURL "event:"/"lingo:" and fscommand)
          // are functions, which can't cross the world boundary. Register
          // host-side handlers that forward each invocation to the isolated
          // world as a bridge event. The open-URL handler must decide
          // SYNCHRONOUSLY whether it claims the navigation — it does so for our
          // lingo:/event: schemes (mirroring the isolated-world logic) and
          // forwards the body for the actual Lingo dispatch there.
          const player = players.get(playerId);
          if (!player) throw new Error('player not registered: ' + playerId);
          if (typeof player.dirplayer_addOpenUrlHandler === 'function') {
            player.dirplayer_addOpenUrlHandler(function (url, target) {
              if (typeof url === 'string'
                && (url.indexOf('lingo:') === 0 || url.indexOf('event:') === 0)) {
                announceEvent(playerId, 'openUrl', { url: url, target: target });
                return true; // claimed — suppress navigation
              }
              return false; // not ours — let Ruffle's openUrlMode decide
            });
          }
          var fsReg = player.dirplayer_addFSCommandHandler || player.addFSCommandHandler;
          if (typeof fsReg === 'function') {
            fsReg.call(player, function (command, args) {
              announceEvent(playerId, 'fsCommand', { command: command, args: args });
            });
          }
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
