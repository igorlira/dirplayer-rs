import { ICastMemberRef, JsBridgeBreakpoint, OnScriptErrorData, registerVmCallbacks } from "dirplayer-js-api";
import { createFlashInstance, destroyFlashInstance, initFlashBridge } from "../services/flashPlayerManager";
import store from "../store";
import { breakpointListChanged, castLibNameChanged, castListChanged, castMemberChanged, castMemberListChanged, channelChanged, channelDisplayNameChanged, datumSnapshot, debugContentAdded, debugMessageAdded, debugMessagesCleared, frameChanged, globalsChanged, movieLoaded, onScriptError, removeTimeoutHandle, scopeListChanged, scoreChanged, scriptErrorCleared, scriptInstanceSnapshot, setTimeoutHandle } from "../store/vmSlice";
import { OnMovieLoadedCallbackData, trigger_timeout, exportW3dObj, exportW3dRaw, listW3dMembers } from 'vm-rust'
import { DatumRef, IVMScope, JsBridgeDatum, MemberSnapshot, ScoreSnapshot, ScoreSpriteSnapshot } from ".";
import { onMemberSelected } from "../store/uiSlice";
import { isUIShown } from "../utils/debug";

export function initVmCallbacks() {
  // Initialize the Flash/Ruffle bridge (registers global JS functions for WASM to call)
  initFlashBridge();

  // Expose W3D debug tools on window for console access
  (window as any).exportW3dObj = exportW3dObj;
  (window as any).exportW3dRaw = exportW3dRaw;
  (window as any).listW3dMembers = listW3dMembers;

  // Expose trace log download on window
  (window as any).downloadTraceLog = () => {
    try {
      // Dynamic import to avoid TS type issues before rebuild
      const vm = require('vm-rust');
      const log = vm.get_trace_log?.();
      if (!log) {
        console.log('No trace log available (traceLogFile not set or empty)');
        return;
      }
      const blob = new Blob([log.content], { type: 'text/plain' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      const fileName = log.path.split(/[/\\]/).pop() || 'trace.log';
      a.href = url;
      a.download = fileName;
      a.click();
      URL.revokeObjectURL(url);
      console.log(`Downloaded trace log: ${fileName} (${log.content.length} bytes)`);
    } catch (e) {
      console.error('Failed to download trace log:', e);
    }
  };

  registerVmCallbacks({
    onMovieLoaded: (result: OnMovieLoadedCallbackData) => {
      // Offer trace log download if one was recorded
      try {
        const vm = require('vm-rust');
        const log = vm.get_trace_log?.();
        if (log && log.content.length > 0) {
          const fileName = log.path.split(/[/\\]/).pop() || 'trace.log';
          console.log(`Trace log available: ${fileName} (${log.content.length} bytes) - call downloadTraceLog() to save`);
        }
      } catch {}
      store.dispatch(debugMessagesCleared());
      store.dispatch(movieLoaded());
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
      console.log(message);
      store.dispatch(debugMessageAdded(message));
    },
    onDebugContent: (content) => {
      store.dispatch(debugContentAdded(content));
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
    },
    onFlashMemberLoaded: (castLib: number, castMember: number, swfData: Uint8Array, width: number, height: number) => {
      // Copy immediately - swfData is a view into WASM memory that may be invalidated
      const swfDataCopy = new Uint8Array(swfData);
      console.log(`Flash member loaded: ${castLib}:${castMember} ${width}x${height} (${swfDataCopy.length} bytes, first=[${Array.from(swfDataCopy.slice(0, 4)).join(',')}])`);
      createFlashInstance(castLib, castMember, swfDataCopy, width, height)
        .catch(e => console.error('Failed to create Flash instance:', e));
    },
    onFlashMemberUnloaded: (castLib: number, castMember: number) => {
      destroyFlashInstance(castLib, castMember);
    },
  });
}
