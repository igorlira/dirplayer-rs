import { JsBridgeDatum, ScriptInstanceId, DatumRef, ScoreSpriteSnapshot, MemberSnapshot } from "../src/vm";

export type ICastMemberRef = [number, number]

type OnScriptErrorData = {
  message: string,
  script_member_ref: ICastMemberRef,
  handler_name: string,
}

type JsBridgeBreakpoint = {
  script_name: string,
  handler_name: string,
  bytecode_index: number,
}

type JsBridgeChunk = {
  id: string,
  fourcc: string,
}

type TVmCallbacks = {
  onMovieLoaded: Function,
  onMovieChunkListChanged: (chunks: Partial<Record<number, JsBridgeChunk>>) => void,
  onCastListChanged: Function,
  onCastLibNameChanged: (castLib: number, name: string) => void,
  onCastMemberListChanged: Function,
  onCastMemberChanged: (memberRef: ICastMemberRef, snapshot: MemberSnapshot) => void,
  onScoreChanged: Function,
  onFrameChanged: Function,
  onScriptError: (data: OnScriptErrorData) => void,
  onScopeListChanged: Function,
  onBreakpointListChanged: (data: JsBridgeBreakpoint[]) => void,
  onScriptErrorCleared: Function,
  onGlobalListChanged: (globals: Map<string, JsBridgeDatum>) => void,
  onDebugMessage: (message: string) => void,
  onScheduleTimeout: (timeoutName: string, periodMs: number) => void,
  onClearTimeout: (timeoutName: string) => void,
  onClearAllTimeouts: () => void,
  onDatumSnapshot: (datumRef: DatumRef, datum: JsBridgeDatum) => void,
  onScriptInstanceSnapshot: (scriptInstanceRef: ScriptInstanceId, scriptInstance: JsBridgeDatum) => void,
  onChannelChanged: (channelNumber: number, channelData: ScoreSpriteSnapshot) => void,
  onChannelDisplayNameChanged: (channelNumber: number, displayName: string) => void,
}
declare let vmCallbacks: TVmCallbacks | undefined;

export function registerVmCallbacks(callbacks: TVmCallbacks);
