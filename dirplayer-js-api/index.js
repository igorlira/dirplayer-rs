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

/// Derive a "by convention" URL for an xtra name. Used as the last-resort
/// fallback when the registry (JSON file + localStorage override) has no
/// entry for `name`. Strategy: strip the extension, split camelCase into
/// snake_case, and serve `~/<snake_case>.wasm` — `~/` resolves through
/// `setXtraHostBase` so each host environment finds the wasm in its own
/// "xtras live here" directory:
///
///   "BobbaXtra"     → "~/bobba_xtra.wasm"
///   "BobbaXtra.x32" → "~/bobba_xtra.wasm"
///   "OpenURL"       → "~/open_url.wasm"      (initialism collapse)
///   "Multiusr"      → "~/multiusr.wasm"      (no internal upper)
///
/// Host bases:
///   - dev:       document.baseURI (set by VMProvider)
///   - polyfill:  <polyfill-script-base>     (set by standalone.tsx)
///   - extension: chrome-extension://<id>/xtras/  (set by content-script.tsx)
///   - electron:  document.baseURI (same as dev — Electron uses VMProvider)
///
/// Hosts that ship xtras under non-conventional filenames pin them
/// explicitly in xtra-registry.json. This fallback covers the common
/// case of "I dropped foo_xtra.wasm in the host's xtras dir and want
/// it to just work."
function _conventionUrl(name) {
  if (typeof name !== 'string' || !name) return null;
  let n = name.replace(/\.(?:x32|x16|xtr|wasm)$/i, '');
  n = n
    // Insert _ between a lowercase/digit and an uppercase (BobbaXtra → Bobba_Xtra).
    .replace(/([a-z0-9])([A-Z])/g, '$1_$2')
    // Insert _ inside a run of uppercase followed by a lowercase (HTMLParser → HTML_Parser).
    .replace(/([A-Z]+)([A-Z][a-z])/g, '$1_$2')
    .toLowerCase();
  return `~/${n}.wasm`;
}

/// Fetch a registry JSON file and merge its entries into the registry.
/// Each host bootstrap calls this after setting its own
/// `setXtraHostBase(...)`. Missing or malformed file is non-fatal —
/// the convention fallback (`~/<snake>.wasm`) still works.
///
/// `path` defaults to `~/xtra-registry.json` (host-base relative).
/// Pass another path (any form `_resolveXtraUrl` understands —
/// `~/...`, `/...`, `https://...`, bare movie-relative) to point
/// elsewhere. Each host has its own override hook layered on top:
///   - polyfill: `data-xtra-registry-url` script attribute
///   - dev/electron/extension: pass programmatically if needed
///
/// Returns a Promise that resolves to the parsed map (or null on miss),
/// so hosts can await this before issuing the first movie load.
export async function loadDefaultXtraRegistry(path = '~/xtra-registry.json') {
  let url;
  try {
    url = _resolveXtraUrl(path);
  } catch (e) {
    // No host base set when the path needs one (e.g. "~/..."). Bail
    // quietly; convention fallback will also be unavailable.
    console.warn(`[dirplayer] loadDefaultXtraRegistry: cannot resolve ${path}, skipping`);
    return null;
  }
  try {
    const resp = await fetch(url, { cache: 'no-cache' });
    if (!resp.ok) {
      // 404 is fine — the file is optional.
      return null;
    }
    const map = await resp.json();
    if (!map || typeof map !== 'object' || Array.isArray(map)) return null;
    setXtraRegistry(map);
    const keys = Object.keys(map);
    if (keys.length > 0) {
      console.log(`[dirplayer] xtra registry primed from ${url}: ${keys.join(', ')}`);
    }
    return map;
  } catch (e) {
    console.warn(`[dirplayer] could not load ${url}:`, e);
    return null;
  }
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
    // Try display-name key first, then filename stem. Convention
    // fallback (`~/<snake>.wasm`) is intentionally NOT consulted here:
    // an average movie's XTRl declares Director's built-in xtras
    // (Multiusr, Font Xtra, Shockwave 3D Asset, INETURL, ...) which
    // the host handles natively. Speculatively fetching every one of
    // those as a wasm would 404 the lot and dump 10+ failures into the
    // console on every movie load.
    //
    // Convention fallback DOES fire from `onRequestXtraLoad` — that
    // path runs only when Lingo *explicitly* references an unknown
    // xtra by name, so a 404 there is a genuine "movie expected a
    // plugin we don't have" signal worth surfacing.
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
  // `failed` = real load error (registry hit a URL, fetch/instantiate
  // blew up). Worth a console.warn.
  //
  // `missing` = XTRl declared an xtra with no registry match. For most
  // movies this is normal — the XTRl lists Director's built-in xtras
  // (Multiusr, Font Xtra, Shockwave 3D Asset, INETURL, ...) which the
  // host handles natively without a wasm plugin. Surfacing those as a
  // yellow warning every movie load is just noise; demote to debug so
  // it shows under DevTools' "Verbose" level when triaging a movie
  // that actually needs a plugin loaded.
  if (failed.length) {
    console.warn('[dirplayer] resolveAndLoadMovieXtras: failed to load xtras', { loaded, failed, missing });
  } else if (missing.length) {
    console.debug('[dirplayer] resolveAndLoadMovieXtras:', { loaded, skipped, missing });
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

// ── On-demand load callback (called by vm-rust on unknown xtra) ───────
//
// vm-rust fires `onRequestXtraLoad(name)` from `request_xtra_load` when
// Lingo executes `new(xtra "name")` for a name that isn't registered.
// We resolve the name through the registry, load the .wasm if matched,
// then call `complete_external_xtra_load(name, success)` back into
// vm-rust so the parked bytecode handler can resume.
//
// One name → at most one in-flight load. Repeat triggers (e.g. multiple
// concurrent waiters from inside vm-rust) are coalesced on the Rust side
// already (only the first call into here fires for a given name), but
// we still guard against a second call from a different source firing
// while a load is in flight.
const _onDemandInFlight = new Set();

export function onRequestXtraLoad(name) {
  const key = (name || '').toLowerCase();
  if (!key) {
    _completeOnDemandLoad(name, false);
    return;
  }
  if (_plugins.has(key)) {
    // Already loaded — signal success immediately. (Shouldn't normally
    // happen, but vm-rust's request_xtra_load fast-paths registered
    // names; this is the defence-in-depth catch.)
    _completeOnDemandLoad(name, true);
    return;
  }
  if (_onDemandInFlight.has(key)) {
    // Another caller already started this load. The completion handler
    // there will signal all waiters when it finishes.
    return;
  }
  // Resolution order: explicit registry pin first; then the snake_case
  // convention (BobbaXtra → /bobba_xtra.wasm). This lets a clean profile
  // pick up wasms dropped in public/ without any localStorage seeding.
  const url =
    _xtraRegistry.get(_normalizeXtraKey(key)) ||
    _conventionUrl(name);
  if (!url) {
    console.warn(`[dirplayer] onRequestXtraLoad: no registry entry or convention URL for '${name}'`);
    _completeOnDemandLoad(name, false);
    return;
  }
  _onDemandInFlight.add(key);
  loadExternalXtra(url)
    .then((loadedName) => {
      _onDemandInFlight.delete(key);
      console.log(`[dirplayer] on-demand loaded '${loadedName}' from ${url}`);
      _completeOnDemandLoad(name, true);
    })
    .catch((err) => {
      _onDemandInFlight.delete(key);
      console.error(`[dirplayer] on-demand load failed for '${name}' (${url}):`, err);
      _completeOnDemandLoad(name, false);
    });
}

function _completeOnDemandLoad(name, success) {
  try {
    _getVmModule().complete_external_xtra_load(name, success);
  } catch (e) {
    console.error('[dirplayer] complete_external_xtra_load threw:', e);
  }
}
