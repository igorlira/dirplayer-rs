// Stubs for the dirplayer-js-api module.
// In production, these are provided by the Electron host.
export function onMovieLoaded() {}
export function onCastListChanged() {}
export function onCastLibNameChanged() {}
export function onCastMemberListChanged() {}
export function onCastMemberChanged() {}
export function onScoreChanged() {}
export function onChannelChanged() {}
export function onChannelDisplayNameChanged() {}
export function onFrameChanged() {}
export function onScriptError(data) {
  const msg = data?.message || JSON.stringify(data);
  console.error('[SCRIPT ERROR]', msg, data);
  if (window.__onScriptError) window.__onScriptError(msg);
}
export function onScopeListChanged() {}
export function onBreakpointListChanged() {}
export function onGlobalListChanged() {}
export function onScriptErrorCleared() {}
export function onDebugMessage() {}
export function onDebugContent() {}
export function onMovieLoadFailed() {}

// Timeout handling — mirrors the real Electron host behavior.
// Uses setInterval to call trigger_timeout, which dispatches
// TimeoutTriggered commands through the player's command loop.
const _timeoutHandles = {};
export function onScheduleTimeout(name, periodMs) {
  if (_timeoutHandles[name]) clearInterval(_timeoutHandles[name]);
  _timeoutHandles[name] = setInterval(() => {
    if (window.__wasm_trigger_timeout) window.__wasm_trigger_timeout(name);
  }, periodMs);
}
export function onClearTimeout(name) {
  if (_timeoutHandles[name]) {
    clearInterval(_timeoutHandles[name]);
    delete _timeoutHandles[name];
  }
}
export function onClearTimeouts() {
  for (const name of Object.keys(_timeoutHandles)) {
    clearInterval(_timeoutHandles[name]);
  }
}
export function onClearAllTimeouts() {
  for (const name of Object.keys(_timeoutHandles)) {
    clearInterval(_timeoutHandles[name]);
    delete _timeoutHandles[name];
  }
}

export function onDatumSnapshot() {}
export function onScriptInstanceSnapshot() {}
export function onExternalEvent() {}

// Lazy-load the Flash manager bundle the first time a Flash member
// appears. If the bundle isn't present (e.g. ruffle/ missing) the
// imports resolve to no-ops so tests without Flash still run.
let _flashManager = null;
let _flashManagerPromise = null;
function flashManager() {
  if (_flashManager) return Promise.resolve(_flashManager);
  if (!_flashManagerPromise) {
    _flashManagerPromise = import('./flashPlayerManager.bundle.js')
      .then(mod => (_flashManager = mod))
      .catch(err => {
        console.warn('[flash] bundle not available:', err?.message || err);
        return (_flashManager = { createFlashInstance: () => {}, destroyFlashInstance: () => {} });
      });
  }
  return _flashManagerPromise;
}
export function onFlashMemberLoaded(castLib, castMember, swfData, width, height) {
  const copy = new Uint8Array(swfData);
  flashManager().then(m => {
    m.createFlashInstance?.(castLib, castMember, copy, width, height)
      ?.catch?.(e => console.error('createFlashInstance failed:', e));
  });
}
export function onFlashMemberUnloaded(castLib, castMember) {
  flashManager().then(m => m.destroyFlashInstance?.(castLib, castMember));
}
export function onStageSizeChanged() {}

// External Xtra plugin bridge — delegated to the real implementation.
//
// `dirplayer-js-api-real.js` is a copy of `dirplayer-js-api/index.js`
// dropped here by `run-browser-tests.mjs`. We re-export its xtra-
// loading functions so e2e tests can actually exercise the SDK end to
// end (`new(xtra "BobbaXtra")`, registry resolution, on-demand loads).
// The HTML template calls `setVmModule(wasm)` after init so the real
// bridge can route plugin host calls back into the test wasm.
//
// Note: we forward only the xtra-bridge surface here, not the UI
// callbacks (`onMovieLoaded`, etc.). The real index.js's UI callbacks
// delegate to `vmCallbacks` which the test harness never registers —
// so they'd throw NPEs. Test mode keeps its own no-op UI stubs above.
export {
  dispatchExternalXtraStaticHandler,
  dispatchExternalXtraInstanceHandler,
  createExternalXtraInstance,
  destroyExternalXtraInstance,
  externalXtraHasStaticHandler,
  loadExternalXtra,
  loadExternalXtras,
  getExternalXtrasReady,
  onRequestXtraLoad,
  setXtraRegistry,
  getXtraRegistry,
  setXtraMovieBase,
  setXtraHostBase,
  setVmModule,
  loadDefaultXtraRegistry,
  resolveAndLoadMovieXtras,
} from './dirplayer-js-api-real.js';
