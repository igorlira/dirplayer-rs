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

export function onDatumSnapshot() {}
export function onScriptInstanceSnapshot() {}
export function onExternalEvent() {}
