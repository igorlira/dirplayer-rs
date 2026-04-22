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
