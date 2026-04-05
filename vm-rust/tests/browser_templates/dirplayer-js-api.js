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
export function onScriptError() {}
export function onScopeListChanged() {}
export function onBreakpointListChanged() {}
export function onGlobalListChanged() {}
export function onScriptErrorCleared() {}
export function onDebugMessage() {}
export function onDebugContent() {}

// Timeouts are fired directly by fire_pending_timeouts() in the test harness,
// not via JS setInterval, to avoid concurrent access between the command loop
// and test code. These stubs are intentionally no-ops.
export function onScheduleTimeout() {}
export function onClearTimeout() {}
export function onClearTimeouts() {}

export function onDatumSnapshot() {}
export function onScriptInstanceSnapshot() {}
export function onExternalEvent() {}
