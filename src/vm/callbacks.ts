import { ICastMemberRef, JsBridgeBreakpoint, JsBridgeChunk, OnScriptErrorData, registerVmCallbacks } from "dirplayer-js-api";
import store from "../store";
import { breakpointListChanged, castLibNameChanged, castListChanged, castMemberChanged, castMemberListChanged, channelChanged, channelDisplayNameChanged, datumSnapshot, frameChanged, globalsChanged, movieChunkListChanged, movieLoaded, onScriptError, removeTimeoutHandle, scopeListChanged, scoreChanged, scriptErrorCleared, scriptInstanceSnapshot, setTimeoutHandle } from "../store/vmSlice";
import { OnMovieLoadedCallbackData, trigger_timeout } from 'vm-rust'
import { DatumRef, IVMScope, JsBridgeDatum, MemberSnapshot, ScoreSnapshot, ScoreSpriteSnapshot } from ".";
import { onMemberSelected } from "../store/uiSlice";
import { isUIShown } from "../utils/debug";

export function initVmCallbacks() {
  registerVmCallbacks({
    onMovieLoaded: (result: OnMovieLoadedCallbackData) => {
      console.log('onMovieLoaded called!', result.version, result.test_val)
      store.dispatch(movieLoaded());
    },
    onMovieChunkListChanged: (chunkList) => {
      store.dispatch(movieChunkListChanged(chunkList));
    },
    onCastListChanged: (castList: string[]) => {
      store.dispatch(castListChanged(castList));
    },
    onCastLibNameChanged: (castNumber: number, name: string) => {
      store.dispatch(castLibNameChanged({ castNumber, name }))
    },
    onCastMemberListChanged: (castNumber: number, members: any) => {
      store.dispatch(castMemberListChanged({ 
        castNumber, 
        members,
      }))
    },
    onCastMemberChanged: (memberRef: ICastMemberRef, snapshot: MemberSnapshot) => {
      store.dispatch(castMemberChanged({ memberRef, snapshot }))
    },
    onFrameChanged: (frame: number) => {
      store.dispatch(frameChanged(frame))
    },
    onScoreChanged: (snapshot: ScoreSnapshot) => {
      store.dispatch(scoreChanged({
        ...snapshot,
      }))
    },
    onScriptError: (errorObj: OnScriptErrorData) => {
      if (!isUIShown()) {
        alert(`Script error: ${errorObj.message}`);
      }
      store.dispatch(onScriptError(errorObj.message))
      store.dispatch(onMemberSelected(errorObj.script_member_ref))
    },
    onScopeListChanged: (scopes: IVMScope[]) => {
      store.dispatch(scopeListChanged(scopes))
    },
    onBreakpointListChanged: (breakpoints: JsBridgeBreakpoint[]) => {
      store.dispatch(breakpointListChanged(breakpoints))
      window.localStorage.setItem('breakpoints', JSON.stringify(breakpoints))
    },
    onScriptErrorCleared: () => {
      store.dispatch(scriptErrorCleared())
    },
    onGlobalListChanged: (globals: Record<string, any>) => {
      store.dispatch(globalsChanged(globals))
    },
    onDebugMessage: (message: string) => {
      console.log("-- ", message);
    },
    onScheduleTimeout: (timeoutName: string, periodMs: number) => {
      const handle = setInterval(() => {
        trigger_timeout(timeoutName)
      }, periodMs);
      store.dispatch(setTimeoutHandle({ name: timeoutName, handle }))
    },
    onClearTimeout: (timeoutName: string) => {
      const handle = store.getState().vm.timeoutHandles[timeoutName];
      if (handle) {
        clearInterval(handle as Parameters<typeof clearInterval>[0]);
        store.dispatch(removeTimeoutHandle(timeoutName))
      }
    },
    onClearAllTimeouts: () => {
      const handles = store.getState().vm.timeoutHandles;
      Object.keys(handles).forEach((key) => {
        clearInterval(handles[key] as Parameters<typeof clearInterval>[0]);
        store.dispatch(removeTimeoutHandle(key))
      })
      console.log("Cleared all timeouts");
    },
    onDatumSnapshot: (datumRef: DatumRef, datum: JsBridgeDatum) => {
      store.dispatch(datumSnapshot({ datumRef, datum }));
    },
    onScriptInstanceSnapshot: (scriptInstanceId: number, scriptInstance: JsBridgeDatum) => {
      store.dispatch(scriptInstanceSnapshot({ scriptInstanceId, datum: scriptInstance }));
    },
    onChannelChanged: (channelNumber: number, channelData: ScoreSpriteSnapshot) => {
      store.dispatch(channelChanged({ channelNumber, channelData }))
    },
    onChannelDisplayNameChanged: (channelNumber: number, displayName: string) => {
      store.dispatch(channelDisplayNameChanged({ channelNumber, displayName }));
    }
  });
}
