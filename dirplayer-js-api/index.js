let vmCallbacks = undefined;
export function registerVmCallbacks(callbacks) {
  vmCallbacks = callbacks;
}

// Resolvers for in-flight whenMovieLoaded() calls. We resolve them
// from inside onMovieLoaded so callers can await movie-load completion
// even though vm-rust's `load_movie_file` is fire-and-forget
// (it dispatches a command and returns immediately).
const _movieLoadedResolvers = [];

/// Returns a Promise that resolves the NEXT time onMovieLoaded fires
/// from vm-rust. Use this between `load_movie_file(path, false)` and
/// `resolveAndLoadMovieXtras()` / `play()` to ensure the movie's
/// metadata (incl. XTRl) has been parsed.
export function whenMovieLoaded() {
  return new Promise((resolve) => _movieLoadedResolvers.push(resolve));
}

export function onMovieLoaded(result) {
  if (vmCallbacks?.onMovieLoaded) {
    vmCallbacks.onMovieLoaded(result);
  }
  // Drain pending whenMovieLoaded() promises.
  const resolvers = _movieLoadedResolvers.splice(0);
  for (const r of resolvers) {
    try { r(result); } catch (e) { console.error('whenMovieLoaded resolver threw:', e); }
  }
}

export function onMovieLoadFailed(path, error) {
  if (vmCallbacks?.onMovieLoadFailed) {
    vmCallbacks.onMovieLoadFailed(path, error);
  } else {
    console.error('[dirplayer] Movie load failed:', path, error);
  }
}

export function onCastListChanged(castList) {
  vmCallbacks.onCastListChanged(castList)
}

export function onCastLibNameChanged(castId, name) {
  vmCallbacks.onCastLibNameChanged(castId, name)
}

export function onCastMemberListChanged(castNumber, members) {
  vmCallbacks.onCastMemberListChanged(castNumber, members)
}

export function onCastMemberChanged(...args) {
  vmCallbacks.onCastMemberChanged(...args)
}

export function onScoreChanged(snapshot) {
  vmCallbacks.onScoreChanged(snapshot)
}

export function onFrameChanged(frame) {
  vmCallbacks.onFrameChanged(frame)
}

export function onScriptError(err) {
  vmCallbacks.onScriptError(err)
}

export function onScopeListChanged(scopeList) {
  vmCallbacks.onScopeListChanged(scopeList)
}

export function onBreakpointListChanged(breakpointList) {
  vmCallbacks.onBreakpointListChanged(breakpointList)
}

export function onScriptErrorCleared() {
  vmCallbacks.onScriptErrorCleared()
}

export function onGlobalListChanged(globalList) {
  vmCallbacks.onGlobalListChanged(globalList)
}

export function onDebugMessage(message) {
  vmCallbacks.onDebugMessage(message)
}

export function onDebugContent(content) {
  vmCallbacks.onDebugContent(content)
}

export function onScheduleTimeout(name, period) {
  vmCallbacks.onScheduleTimeout(name, period)
}

export function onClearTimeout(name) {
  vmCallbacks.onClearTimeout(name)
}

export function onClearAllTimeouts() {
  vmCallbacks.onClearAllTimeouts()
}

export function onDatumSnapshot(datumRef, snapshot) {
  vmCallbacks.onDatumSnapshot(datumRef, snapshot)
}

export function onScriptInstanceSnapshot(instanceId, snapshot) {
  vmCallbacks.onScriptInstanceSnapshot(instanceId, snapshot)
}

export function onChannelChanged(channel, value) {
  vmCallbacks.onChannelChanged(channel, value)
}

export function onChannelDisplayNameChanged(channel, displayName) {
  vmCallbacks.onChannelDisplayNameChanged(channel, displayName)
}

export function onExternalEvent(event) {
  if (vmCallbacks?.onExternalEvent) {
    vmCallbacks.onExternalEvent(event);
  } else {
    console.log('externalEvent:', event);
  }
}

export function onFlashMemberLoaded(spriteNum, castLib, castMember, swfData, width, height, pausedAtStart) {
  if (vmCallbacks?.onFlashMemberLoaded) {
    vmCallbacks.onFlashMemberLoaded(spriteNum, castLib, castMember, swfData, width, height, pausedAtStart);
  } else {
    console.log('Flash member loaded:', 'sprite#' + spriteNum, castLib, castMember, width, height, swfData.length, 'bytes', 'pausedAtStart=' + pausedAtStart);
  }
}

export function onFlashMemberUnloaded(spriteNum) {
  if (vmCallbacks?.onFlashMemberUnloaded) {
    vmCallbacks.onFlashMemberUnloaded(spriteNum);
  } else {
    console.log('Flash member unloaded: sprite#' + spriteNum);
  }
}

export function onStageSizeChanged(width, height, center) {
  if (vmCallbacks?.onStageSizeChanged) {
    vmCallbacks.onStageSizeChanged(width, height, center);
  }
}

// ─────────────────────────────────────────────────────────────────────
// External Xtra plugin loader + JS↔WASM bridge
//
// Works in every dirplayer-rs host: the dev React app, the polyfill
// bundle, the browser extension, and Electron. Each host calls
// `loadExternalXtra(url)` for whichever plugins it wants to load; the
// rest happens here. No host-specific code.
//
// Architecture:
//   plugin .wasm   <— this module —>   vm-rust wasm
//        ↑                                  ↑
//        │ dx_host_call                     │ external_xtra_host_dispatch
//        │                                  │
//   plugin imports                  vm-rust exports
//
// Postcard encoding is entirely on the Rust side. JS just shuffles
// bytes between plugin memory and vm-rust calls; it does not decode
// the wire format.

const _plugins = new Map(); // lowercaseName -> { exports, memory }
let _vmModule = null;       // lazy-loaded `vm-rust` module reference
let _xtraMovieBase = null;  // base URL used for movie-relative xtra resolution
let _xtraHostBase = null;   // base URL for "~/foo.wasm" — points at where the host JS lives
const _xtraRegistry = new Map(); // normalized-name -> url (movie XTRl resolves through here)

/// Normalize a registry/movie xtra name to its lookup key:
///   - lowercased
///   - ".x32" / ".x16" / ".xtr" extension stripped
/// So "BobbaXtra", "bobbaxtra", "BobbaXtra.x32" all key to "bobbaxtra".
function _normalizeXtraKey(name) {
  if (typeof name !== 'string') return '';
  let s = name.toLowerCase();
  s = s.replace(/\.(?:x32|x16|xtr|wasm)$/i, '');
  return s;
}

/// Set or merge the name→URL registry used to resolve a movie's XTRl
/// declarations. Each host (dev / polyfill / extension / Electron)
/// calls this at boot with whatever its config tells it. Repeated
/// calls MERGE (later wins per key); pass an empty object to clear.
///
/// URL values follow the same resolver rules as loadExternalXtra:
/// "https://..." absolute; "/path" relative to document; bare name
/// relative to current movie (only meaningful at movie-load time).
export function setXtraRegistry(map) {
  if (!map) { _xtraRegistry.clear(); return; }
  for (const [name, url] of Object.entries(map)) {
    if (typeof url !== 'string' || !url) continue;
    _xtraRegistry.set(_normalizeXtraKey(name), url);
  }
}

export function getXtraRegistry() {
  // Defensive copy so callers can't mutate the live map.
  const out = {};
  for (const [k, v] of _xtraRegistry) out[k] = v;
  return out;
}

/// Resolve the currently-loaded movie's XTRl declarations against the
/// registry and load any matched plugins that aren't already loaded.
/// Returns a summary object describing what happened.
export async function resolveAndLoadMovieXtras() {
  const vm = _getVmModule();
  if (typeof vm.movie_required_xtras !== 'function') {
    return { skipped: [], loaded: [], failed: [], missing: [] };
  }
  const required = vm.movie_required_xtras(); // Array of { filename, displayName }
  const skipped = [];
  const missing = [];
  const toLoad = [];
  for (const entry of required) {
    const filename = entry.filename || '';
    const display = entry.displayName || '';
    const key = _normalizeXtraKey(display || filename);
    if (_plugins.has(key) || _plugins.has(_normalizeXtraKey(filename))) {
      // Already loaded under either key form.
      skipped.push(display || filename);
      continue;
    }
    // Try display-name key first, then filename stem.
    const url =
      _xtraRegistry.get(key) ||
      _xtraRegistry.get(_normalizeXtraKey(filename));
    if (!url) {
      missing.push({ filename, displayName: display });
      continue;
    }
    toLoad.push({ name: display || filename, url });
  }
  const loadResults = await Promise.allSettled(
    toLoad.map((t) => loadExternalXtra(t.url).then((name) => ({ t, name })))
  );
  const loaded = [];
  const failed = [];
  for (let i = 0; i < loadResults.length; i++) {
    const r = loadResults[i];
    if (r.status === 'fulfilled') {
      loaded.push(r.value.name);
    } else {
      failed.push({ name: toLoad[i].name, url: toLoad[i].url, error: String(r.reason) });
    }
  }
  if (missing.length || failed.length) {
    console.warn('[dirplayer] resolveAndLoadMovieXtras:', { loaded, skipped, failed, missing });
  }
  return { skipped, loaded, failed, missing };
}

/// Set the base URL that bare xtra filenames resolve against. Host code
/// (LoadMovie, EmbedPlayer, polyfill bootstrap, extension entry) should
/// call this immediately before loading a movie so any subsequent
/// loadExternalXtra("bare.wasm") resolves to "<movieBase>/bare.wasm".
///
/// Resolution rules in loadExternalXtra:
///   - "http://..." / "https://..." / "chrome-extension://..." → as-is
///   - "/anything.wasm"                  → relative to document.baseURI
///   - "~/anything.wasm"                 → relative to host base (setXtraHostBase)
///   - "anything.wasm"  (bare)           → relative to the current movie base
export function setXtraMovieBase(base) {
  _xtraMovieBase = base || null;
}

/// Set the base URL used by the "~/..." prefix in xtra URLs. Useful in
/// the polyfill / extension cases where the host JS lives at a known
/// CDN location and wants to serve its xtras alongside itself, without
/// requiring them to be co-located with the .dcr movie files.
///
/// Polyfill `standalone.tsx` calls this with `<polyfill-script-base>`
/// at init; extension / Electron hosts can call it with whatever base
/// makes sense (e.g. `chrome-extension://<id>/xtras/`).
export function setXtraHostBase(base) {
  _xtraHostBase = base || null;
}

function _resolveXtraUrl(url) {
  // Already an absolute URL (any scheme)? Use as-is.
  try {
    return new URL(url).href;
  } catch {
    /* relative — fall through */
  }
  // Host-base prefix "~/foo.wasm" → resolve against the host JS location.
  if (url.startsWith('~/')) {
    if (!_xtraHostBase) {
      throw new Error(
        `loadExternalXtra: cannot resolve '${url}' — no host base set. ` +
        `Call setXtraHostBase(...) from the host's bootstrap (e.g. ` +
        `polyfill standalone init) with the URL the host JS was loaded from.`
      );
    }
    const absHost = new URL(_xtraHostBase, document.baseURI).href;
    return new URL(url.substring(2), absHost).href;
  }
  // Absolute path → resolve against the document.
  if (url.startsWith('/')) {
    return new URL(url, document.baseURI).href;
  }
  // Bare filename → resolve against the current movie base.
  if (!_xtraMovieBase) {
    throw new Error(
      `loadExternalXtra: cannot resolve '${url}' — no movie base set. ` +
      `Bare filenames resolve against the current movie path; call ` +
      `setXtraMovieBase(...) first, OR use a leading '/' for paths ` +
      `relative to the host document, OR '~/' for paths relative to ` +
      `the host JS location, OR a full 'http(s)://' URL.`
    );
  }
  // Absolutize the movie base against the document to handle path-only
  // bases like "/movies/foo/" before resolving the bare filename.
  const absBase = new URL(_xtraMovieBase, document.baseURI).href;
  return new URL(url, absBase).href;
}

// Some Rust toolchains use big-endian for the (ptr<<32)|len pack; ours
// (wasm32-unknown-unknown release) uses little-endian. We construct the
// u64 the same way the SDK does in `abi::pack`.
function _packPtr(ptr, len) {
  return (BigInt(ptr) << 32n) | BigInt(len);
}
function _unpackPtr(packed) {
  return [Number(packed >> 32n), Number(packed & 0xFFFFFFFFn)];
}

function _writePluginBytes(plugin, bytes) {
  if (bytes.length === 0) return 0;
  const ptr = plugin.exports.__plugin_alloc(bytes.length);
  new Uint8Array(plugin.exports.memory.buffer, ptr, bytes.length).set(bytes);
  return ptr;
}

function _readPluginBytes(plugin, ptr, len) {
  if (ptr === 0 || len === 0) return new Uint8Array(0);
  // Copy out: the plugin may dealloc the source buffer immediately after.
  return new Uint8Array(plugin.exports.memory.buffer, ptr, len).slice();
}

function _readPackedAndDealloc(plugin, packed) {
  const [ptr, len] = _unpackPtr(packed);
  if (ptr === 0) return new Uint8Array(0);
  const out = _readPluginBytes(plugin, ptr, len);
  plugin.exports.__plugin_dealloc(ptr, len);
  return out;
}

function _getVmModule() {
  if (_vmModule) return _vmModule;
  // Lazy require so the bridge doesn't pull in vm-rust just for being
  // imported. Each host should already have vm-rust loaded by the time
  // the first plugin dispatch fires.
  _vmModule = require('vm-rust');
  return _vmModule;
}

// Tracks the in-flight load of every URL ever passed to loadExternalXtra,
// so a movie can `await getExternalXtrasReady()` before evaluating
// scripts that reference an external xtra. Resolves to `null` when no
// loads have been initiated, otherwise to a Promise<string[]> of names.
const _pendingLoads = [];

/// Load every URL in `urls` in parallel. Resolves with the array of
/// loaded xtra names (same order as input). Hosts call this at boot
/// with whichever URLs they want available (dev=localStorage list,
/// polyfill=init-script, extension=chrome.storage, Electron=app config).
export function loadExternalXtras(urls) {
  if (!urls || urls.length === 0) return Promise.resolve([]);
  // Each loadExternalXtra() already pushes itself to _pendingLoads.
  return Promise.all(urls.map((u) => loadExternalXtra(u)));
}

/// Resolves once every loadExternalXtra/s call initiated so far has
/// finished (either resolved or rejected — does not throw on failure).
/// Movie-loading code can await this to ensure plugin-using scripts see
/// their xtras registered. Returns immediately if no loads are pending.
export async function getExternalXtrasReady() {
  if (_pendingLoads.length === 0) return;
  await Promise.allSettled(_pendingLoads.slice());
}

/// Public API. Fetches a plugin .wasm from the URL, instantiates it
/// with the `dirplayer_xtra_host::dx_host_call` import wired to vm-rust,
/// and registers the resulting xtra by name. The returned Promise
/// resolves with the xtra name (so the host can confirm which plugin
/// loaded). The promise is also tracked by `getExternalXtrasReady`
/// regardless of whether you await it directly.
export function loadExternalXtra(url) {
  const p = _loadExternalXtraInner(url);
  _pendingLoads.push(p);
  return p;
}

async function _loadExternalXtraInner(url) {
  const resolved = _resolveXtraUrl(url);
  const wasmBytes = await fetch(resolved).then((r) => {
    if (!r.ok) throw new Error(`loadExternalXtra: HTTP ${r.status} for ${resolved}`);
    return r.arrayBuffer();
  });

  // The plugin is instantiated BEFORE we know its name. We need the
  // import satisfied at this point, but we don't have a `plugin` object
  // to read memory from yet — patch it after instantiation.
  const pluginSlot = { exports: null };
  const imports = {
    dirplayer_xtra_host: {
      dx_host_call: (opId, argsPtr, argsLen) => {
        if (!pluginSlot.exports) return 0n;
        const argsBytes = _readPluginBytes(pluginSlot, argsPtr, argsLen);
        const result = _getVmModule().external_xtra_host_dispatch(opId, argsBytes);
        // result is Uint8Array (possibly empty for void sentinel).
        if (!result || result.length === 0) return 0n;
        const ptr = _writePluginBytes(pluginSlot, result);
        return _packPtr(ptr, result.length);
      },
    },
  };

  const { instance } = await WebAssembly.instantiate(wasmBytes, imports);
  pluginSlot.exports = instance.exports;

  // Read the xtra name out of the plugin.
  const namePacked = instance.exports.__xtra_name();
  const nameBytes = _readPackedAndDealloc(pluginSlot, namePacked);
  const name = new TextDecoder().decode(nameBytes);
  if (!name) throw new Error(`loadExternalXtra(${url}): plugin returned empty xtra name`);

  _plugins.set(name.toLowerCase(), pluginSlot);

  // Tell vm-rust about the new xtra so manager.rs can route to it.
  _getVmModule().register_external_xtra(name);

  return name;
}

// ─── Bridge functions called by vm-rust extern declarations ──────────

export function dispatchExternalXtraStaticHandler(xtraName, handler, args) {
  const plugin = _plugins.get(xtraName.toLowerCase());
  if (!plugin) return undefined;

  const handlerBytes = new TextEncoder().encode(handler);
  const handlerPtr = _writePluginBytes(plugin, handlerBytes);
  const argsPtr = _writePluginBytes(plugin, args);
  const packed = plugin.exports.__xtra_call_static_handler(
    handlerPtr, handlerBytes.length, argsPtr, args.length,
  );
  const result = _readPackedAndDealloc(plugin, packed);
  plugin.exports.__plugin_dealloc(handlerPtr, handlerBytes.length);
  plugin.exports.__plugin_dealloc(argsPtr, args.length);
  return result;
}

export function dispatchExternalXtraInstanceHandler(xtraName, instanceId, handler, args) {
  const plugin = _plugins.get(xtraName.toLowerCase());
  if (!plugin) return undefined;

  const handlerBytes = new TextEncoder().encode(handler);
  const handlerPtr = _writePluginBytes(plugin, handlerBytes);
  const argsPtr = _writePluginBytes(plugin, args);
  const packed = plugin.exports.__xtra_call_handler(
    instanceId, handlerPtr, handlerBytes.length, argsPtr, args.length,
  );
  const result = _readPackedAndDealloc(plugin, packed);
  plugin.exports.__plugin_dealloc(handlerPtr, handlerBytes.length);
  plugin.exports.__plugin_dealloc(argsPtr, args.length);
  return result;
}

export function createExternalXtraInstance(xtraName, args) {
  const plugin = _plugins.get(xtraName.toLowerCase());
  if (!plugin) return undefined;

  const argsPtr = _writePluginBytes(plugin, args);
  const packed = plugin.exports.__xtra_create_instance(argsPtr, args.length);
  const result = _readPackedAndDealloc(plugin, packed);
  plugin.exports.__plugin_dealloc(argsPtr, args.length);
  return result;
}

export function destroyExternalXtraInstance(xtraName, instanceId) {
  const plugin = _plugins.get(xtraName.toLowerCase());
  if (!plugin) return;
  plugin.exports.__xtra_destroy_instance(instanceId);
}

export function externalXtraHasStaticHandler(xtraName, handler) {
  const plugin = _plugins.get(xtraName.toLowerCase());
  if (!plugin) return 0;
  const handlerBytes = new TextEncoder().encode(handler);
  const handlerPtr = _writePluginBytes(plugin, handlerBytes);
  const has = plugin.exports.__xtra_has_static_handler(handlerPtr, handlerBytes.length);
  plugin.exports.__plugin_dealloc(handlerPtr, handlerBytes.length);
  return has;
}
