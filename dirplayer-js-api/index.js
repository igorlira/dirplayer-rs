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
