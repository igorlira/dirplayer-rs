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
  len: number,
  owner?: number,
  castLib?: number,
  memberNumber?: number,
  memberName?: string,
}

export type DebugContentBitmap = { type: 'bitmap'; width: number; height: number; data: Uint8Array };
export type DebugContentDatum = { type: 'datum'; datumRef: DatumRef; snapshot: JsBridgeDatum };
export type DebugContent = DebugContentBitmap | DebugContentDatum;

type TVmCallbacks = {
  onMovieLoaded: Function,
  onMovieLoadFailed?: (path: string, error: string) => void,
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
  onDebugContent: (content: DebugContent) => void,
  onScheduleTimeout: (timeoutName: string, periodMs: number) => void,
  onClearTimeout: (timeoutName: string) => void,
  onClearAllTimeouts: () => void,
  onDatumSnapshot: (datumRef: DatumRef, datum: JsBridgeDatum) => void,
  onScriptInstanceSnapshot: (scriptInstanceRef: ScriptInstanceId, scriptInstance: JsBridgeDatum) => void,
  onChannelChanged: (channelNumber: number, channelData: ScoreSpriteSnapshot) => void,
  onChannelDisplayNameChanged: (channelNumber: number, displayName: string) => void,
  onExternalEvent?: (event: string) => void,
  onFlashMemberLoaded?: (spriteNum: number, castLib: number, castMember: number, swfData: Uint8Array, width: number, height: number, pausedAtStart: boolean) => void,
  onFlashMemberUnloaded?: (spriteNum: number) => void,
  onStageSizeChanged?: (width: number, height: number, center: boolean) => void,
}
declare let vmCallbacks: TVmCallbacks | undefined;

export function registerVmCallbacks(callbacks: TVmCallbacks);

// ── External Xtra Plugin System ───────────────────────────────────────────────

/**
 * Fetch, instantiate, and register a WASM xtra plugin from a URL.
 * After resolution, call `register_external_xtra_plugin(name)` on the Rust
 * WASM side to wire up Lingo dispatch.
 */
export function loadExternalXtraFromUrl(url: string): Promise<string>;

/**
 * Instantiate an external xtra plugin from raw WASM bytes.
 * Returns the xtra name (lowercase).
 */
export function loadExternalXtraFromBytes(bytes: ArrayBuffer): Promise<string>;

/** Returns true if a plugin with this name has been loaded. */
export function isExternalXtraLoaded(name: string): boolean;

/** Returns a JSON array string of loaded xtra names. */
export function getLoadedExternalXtraNames(): string;

export function externalXtraCreateInstance(name: string, argsJson: string): string;
export function externalXtraDestroyInstance(name: string, id: number): void;
export function externalXtraCallHandler(name: string, id: number, handlerName: string, argsJson: string): string;
export function externalXtraHasAsyncHandler(name: string, handlerName: string): boolean;
export function externalXtraHasStaticHandler(name: string, handlerName: string): boolean;
export function externalXtraCallStaticHandler(name: string, handlerName: string, argsJson: string): string;
