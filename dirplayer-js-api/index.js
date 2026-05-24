let vmCallbacks = undefined;
export function registerVmCallbacks(callbacks) {
  vmCallbacks = callbacks;
}

export function onMovieLoaded(result) {
  vmCallbacks.onMovieLoaded(result)
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

// ── External Xtra Plugin System ───────────────────────────────────────────────
//
// Plugins are standard WASM modules that export a defined ABI.  The JS layer
// handles WASM instantiation and memory bridging; Rust calls the functions
// below for every plugin operation.
//
// Plugin module ABI (exports):
//   memory: WebAssembly.Memory
//   alloc(size: i32) -> i32          allocate `size` bytes, return ptr
//   dealloc(ptr: i32, size: i32)     free a previously allocated buffer
//   xtra_name_ptr() -> i32           ptr to xtra-name UTF-8 bytes
//   xtra_name_len() -> i32           length of xtra-name in bytes
//   xtra_create_instance(args_ptr: i32, args_len: i32) -> i32
//       instance id (>=0) or -1 on error (error in error buffer)
//   xtra_destroy_instance(id: i32)
//   xtra_call_handler(id: i32, name_ptr, name_len, args_ptr, args_len) -> i32
//       0=ok, -1=error; result in result buffer
//   xtra_get_result_ptr() -> i32     ptr to last JSON result
//   xtra_get_result_len() -> i32     length of last JSON result
//   xtra_get_error_ptr() -> i32      ptr to last error string
//   xtra_get_error_len() -> i32      length of last error string
//   xtra_has_async_handler(name_ptr, name_len) -> i32   0 or 1
//   xtra_has_static_handler(name_ptr, name_len) -> i32  0 or 1
//   xtra_call_static_handler(name_ptr, name_len, args_ptr, args_len) -> i32
//
// Plugin module ABI (imports) — provided by host during instantiation:
//   Rust extern "C" always places imports under the "env" module namespace
//   when targeting wasm32-unknown-unknown, so all function names below are
//   prefixed with `dirplayer_host_` exactly as declared in host_env.rs.
//   env.dirplayer_host_log(msg_ptr: i32, msg_len: i32)
//   env.dirplayer_host_random_fill(buf_ptr: i32, len: i32) -> i32  (0=ok)
//   env.dirplayer_host_storage_get(key_ptr, key_len, result_ptr, result_max_len) -> i32
//   env.dirplayer_host_storage_set(key_ptr, key_len, val_ptr, val_len) -> i32  (0=ok, -1=error)

/** @type {Map<string, {exports: WebAssembly.Exports, memory: WebAssembly.Memory}>} */
const externalXtras = new Map();

function readUtf8(memory, ptr, len) {
  return new TextDecoder().decode(new Uint8Array(memory.buffer, ptr, len));
}

function writeUtf8(exports, str) {
  const encoded = new TextEncoder().encode(str);
  const ptr = exports.alloc(encoded.length);
  new Uint8Array(exports.memory.buffer).set(encoded, ptr);
  return { ptr, len: encoded.length };
}

function makeHostImports(xtraRef) {
  return {
    // Rust extern "C" imports land under the "env" namespace for wasm32-unknown-unknown.
    // Function names match exactly what is declared in host_env.rs.
    env: {
      dirplayer_host_log(msgPtr, msgLen) {
        const text = readUtf8(xtraRef.exports.memory, msgPtr, msgLen);
        console.log('[xtra]', text);
      },
      dirplayer_host_random_fill(bufPtr, len) {
        const buf = new Uint8Array(len);
        crypto.getRandomValues(buf);
        new Uint8Array(xtraRef.exports.memory.buffer).set(buf, bufPtr);
        return 0;
      },
      dirplayer_host_storage_get(keyPtr, keyLen, resultPtr, resultMaxLen) {
        const key = readUtf8(xtraRef.exports.memory, keyPtr, keyLen);
        const value = localStorage.getItem(key);
        if (value === null) return -1;
        const encoded = new TextEncoder().encode(value);
        const actualLen = Math.min(encoded.length, resultMaxLen);
        new Uint8Array(xtraRef.exports.memory.buffer).set(encoded.slice(0, actualLen), resultPtr);
        return actualLen;
      },
      dirplayer_host_storage_set(keyPtr, keyLen, valPtr, valLen) {
        try {
          const key = readUtf8(xtraRef.exports.memory, keyPtr, keyLen);
          const val = readUtf8(xtraRef.exports.memory, valPtr, valLen);
          localStorage.setItem(key, val);
          return 0;
        } catch {
          return -1;
        }
      },
    },
  };
}

/**
 * Fetch, instantiate, and register an external xtra plugin from a URL.
 * After this resolves, call `register_external_xtra_plugin(name)` on the
 * Rust side (or let the host call it for you) to wire up the Lingo dispatch.
 * @param {string} url
 * @returns {Promise<string>} the xtra name (lowercase)
 */
export async function loadExternalXtraFromUrl(url) {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`Failed to fetch xtra plugin from ${url}: ${response.statusText}`);
  }
  const bytes = await response.arrayBuffer();
  return loadExternalXtraFromBytes(bytes);
}

/**
 * Instantiate an external xtra from a raw WASM bytes buffer.
 * @param {ArrayBuffer} bytes
 * @returns {Promise<string>} the xtra name (lowercase)
 */
export async function loadExternalXtraFromBytes(bytes) {
  // Two-phase init: we need the imports to hold a reference to the exports,
  // but the exports aren't available until after instantiation.  We break the
  // circularity by capturing a mutable `ref` object that is filled in right
  // after `instantiate` returns.
  const xtraRef = { exports: null, memory: null };
  const imports = makeHostImports(xtraRef);

  const { instance } = await WebAssembly.instantiate(bytes, imports);
  xtraRef.exports = instance.exports;
  xtraRef.memory = instance.exports.memory;

  const namePtr = instance.exports.xtra_name_ptr();
  const nameLen = instance.exports.xtra_name_len();
  const name = readUtf8(instance.exports.memory, namePtr, nameLen).toLowerCase();

  externalXtras.set(name, xtraRef);
  console.log(`[dirplayer] External xtra loaded: ${name}`);
  return name;
}

/** Returns true if a plugin with this name (case-insensitive) has been loaded. */
export function isExternalXtraLoaded(name) {
  return externalXtras.has(name.toLowerCase());
}

/** @returns {string} JSON array of loaded xtra names */
export function getLoadedExternalXtraNames() {
  return JSON.stringify(Array.from(externalXtras.keys()));
}

/** Create a new plugin instance.  Returns JSON `{"ok":id}` or `{"err":"..."}`. */
export function externalXtraCreateInstance(name, argsJson) {
  const xtra = externalXtras.get(name.toLowerCase());
  if (!xtra) return JSON.stringify({ err: `Xtra '${name}' not loaded` });

  const { ptr: argsPtr, len: argsLen } = writeUtf8(xtra.exports, argsJson);
  const id = xtra.exports.xtra_create_instance(argsPtr, argsLen);
  xtra.exports.dealloc(argsPtr, argsLen);

  if (id < 0) {
    const errPtr = xtra.exports.xtra_get_error_ptr();
    const errLen = xtra.exports.xtra_get_error_len();
    const err = readUtf8(xtra.exports.memory, errPtr, errLen);
    return JSON.stringify({ err });
  }
  return JSON.stringify({ ok: id });
}

/** Destroy a plugin instance. */
export function externalXtraDestroyInstance(name, id) {
  const xtra = externalXtras.get(name.toLowerCase());
  if (xtra) xtra.exports.xtra_destroy_instance(id);
}

/**
 * Call a handler on a plugin instance.
 * Returns a JSON datum string or `{"__error":"..."}`.
 */
export function externalXtraCallHandler(name, id, handlerName, argsJson) {
  const xtra = externalXtras.get(name.toLowerCase());
  if (!xtra) return JSON.stringify({ __error: `Xtra '${name}' not loaded` });

  const { ptr: namePtr, len: nameLen } = writeUtf8(xtra.exports, handlerName);
  const { ptr: argsPtr, len: argsLen } = writeUtf8(xtra.exports, argsJson);

  const ret = xtra.exports.xtra_call_handler(id, namePtr, nameLen, argsPtr, argsLen);

  xtra.exports.dealloc(namePtr, nameLen);
  xtra.exports.dealloc(argsPtr, argsLen);

  if (ret < 0) {
    const errPtr = xtra.exports.xtra_get_error_ptr();
    const errLen = xtra.exports.xtra_get_error_len();
    const err = readUtf8(xtra.exports.memory, errPtr, errLen);
    return JSON.stringify({ __error: err });
  }
  const resultPtr = xtra.exports.xtra_get_result_ptr();
  const resultLen = xtra.exports.xtra_get_result_len();
  return readUtf8(xtra.exports.memory, resultPtr, resultLen);
}

/** Returns true if the named handler is async. */
export function externalXtraHasAsyncHandler(name, handlerName) {
  const xtra = externalXtras.get(name.toLowerCase());
  if (!xtra) return false;
  const { ptr, len } = writeUtf8(xtra.exports, handlerName);
  const result = xtra.exports.xtra_has_async_handler(ptr, len);
  xtra.exports.dealloc(ptr, len);
  return result !== 0;
}

/** Returns true if the named handler is static (no instance needed). */
export function externalXtraHasStaticHandler(name, handlerName) {
  const xtra = externalXtras.get(name.toLowerCase());
  if (!xtra) return false;
  const { ptr, len } = writeUtf8(xtra.exports, handlerName);
  const result = xtra.exports.xtra_has_static_handler(ptr, len);
  xtra.exports.dealloc(ptr, len);
  return result !== 0;
}

/**
 * Call a static handler on a plugin.
 * Returns a JSON datum string or `{"__error":"..."}`.
 */
export function externalXtraCallStaticHandler(name, handlerName, argsJson) {
  const xtra = externalXtras.get(name.toLowerCase());
  if (!xtra) return JSON.stringify({ __error: `Xtra '${name}' not loaded` });

  const { ptr: namePtr, len: nameLen } = writeUtf8(xtra.exports, handlerName);
  const { ptr: argsPtr, len: argsLen } = writeUtf8(xtra.exports, argsJson);

  const ret = xtra.exports.xtra_call_static_handler(namePtr, nameLen, argsPtr, argsLen);

  xtra.exports.dealloc(namePtr, nameLen);
  xtra.exports.dealloc(argsPtr, argsLen);

  if (ret < 0) {
    const errPtr = xtra.exports.xtra_get_error_ptr();
    const errLen = xtra.exports.xtra_get_error_len();
    const err = readUtf8(xtra.exports.memory, errPtr, errLen);
    return JSON.stringify({ __error: err });
  }
  const resultPtr = xtra.exports.xtra_get_result_ptr();
  const resultLen = xtra.exports.xtra_get_result_len();
  return readUtf8(xtra.exports.memory, resultPtr, resultLen);
}
