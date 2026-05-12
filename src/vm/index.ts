import { ICastMemberRef } from "dirplayer-js-api";

export type DatumRef = number;
export type ScriptInstanceId = number;

export type TJsBridgeDatumBase = {
  debugDescription: string;
}

export type TJsBridgeDatumScriptInstance = TJsBridgeDatumBase & {
  type: 'scriptInstance',
  properties: Record<string, DatumRef>,
  ancestor: DatumRef | null,
}

export type TJsBridgeDatumList = TJsBridgeDatumBase & {
  type: 'list',
  items: DatumRef[],
}

export type TJsBridgeDatumPropList = TJsBridgeDatumBase & {
  type: 'propList',
  properties: Record<string, DatumRef>,
}

export type TJsBridgeDatumUnknown = TJsBridgeDatumBase & {
  // type: string,
  properties: Record<string, DatumRef>,
}

export type TJsBridgeDatumVoid = TJsBridgeDatumBase & {
  type: 'void',
}

export type TJsBridgeDatumJavaScript = TJsBridgeDatumBase & {
  type: 'javascript',
  size: number,
  bytes: Uint8Array,
}

export type JsBridgeDatum = TJsBridgeDatumScriptInstance | TJsBridgeDatumList | TJsBridgeDatumPropList | TJsBridgeDatumVoid | TJsBridgeDatumJavaScript// | TJsBridgeDatumUnknown;

export interface IVMScope {
  script_member_ref: ICastMemberRef,
  bytecode_index: number,
  handler_name: string,
  locals: Record<string, DatumRef>,
  args: DatumRef[],
  stack: DatumRef[],
}

export interface ICastMemberIdentifier {
  castNumber: number;
  memberNumber: number;
}

export function memberRefEquals(a: ICastMemberRef, b: ICastMemberRef): boolean {
  return a[0] === b[0] && a[1] === b[1]
}

export function memberRefEqualsSafe(a: ICastMemberRef | ICastMemberIdentifier | null | undefined, b: ICastMemberRef | ICastMemberIdentifier | null | undefined): boolean {
  if (!a || !b) {
    return !a && !b
  }

  let aRef: ICastMemberRef
  let bRef: ICastMemberRef

  if ('castNumber' in a) {
    aRef = [a.castNumber, a.memberNumber]
  } else {
    aRef = a
  }
  if ('castNumber' in b) {
    bRef = [b.castNumber, b.memberNumber]
  } else {
    bRef = b
  }

  return memberRefEquals(aRef, bRef)
}

export function castMemberIdentifier(castNumber: number, memberNumber: number): ICastMemberIdentifier {
  return { castNumber, memberNumber }
}

export interface CastMemberRecord {
  name: string;
  type?: string;
  scriptType?: string;
  snapshot?: MemberSnapshot;
}

export interface CastSnapshot {
  number: number;
  members: Record<string, CastMemberRecord>
}

export interface IBaseMemberSnapshot {
  number: number;
  name: string;
}

export interface IFieldMemberSnapshot {
  type: 'field'
  text: string
}

export interface IBitmapMemberSnapshot {
  type: 'bitmap'
  width: number
  height: number
  bitDepth: number
  paletteRef: number
  regX: number
  regY: number
}

export interface IScriptMemberSnapshot {
  type: 'script'
  name: string
  scriptType: 'movie' | 'parent' | 'score'
  /**
   * 'lingo' for the original Director Lingo syntax, 'javascript' when the
   * cast member was authored with JavaScript syntax (Director MX 2004+).
   * For JS scripts, `script.handlers` is synthesised from the SpiderMonkey
   * disassembly with each top-level JS function shown as a separate handler.
   */
  scriptSyntax?: 'lingo' | 'javascript'
  script: IScriptSnapshot
}

export interface IPaletteMemberSnapshot {
  type: 'palette'
  name: string
  colors?: [number, number, number][]
  paletteRef: number
}

export interface IFilmLoopMemberSnapshot {
  type: 'filmLoop'
  width: number
  height: number
  regX: number
  regY: number
  score?: ScoreSnapshot
}

export interface IScriptSnapshot {
  handlers: IHandlerSnapshot[]
}

export interface IHandlerSnapshot {
  name: string
  args: string[],
  bytecode: IBytecodeSnapshot[],
  lingo?: ILingoLine[],
  bytecodeToLine?: Record<number, number>,
}

export interface IBytecodeSnapshot {
  pos: number
  text: string
}

export type LingoTokenType =
  | 'keyword'
  | 'identifier'
  | 'number'
  | 'string'
  | 'symbol'
  | 'operator'
  | 'comment'
  | 'builtin'
  | 'punctuation'
  | 'whitespace';

export interface ILingoSpan {
  text: string
  type: LingoTokenType
}

export interface ILingoLine {
  text: string
  indent: number
  bytecodeIndices: number[]
  spans: ILingoSpan[]
}

export interface IFlashMemberSnapshot {
  type: 'flash'
  regX: number
  regY: number
  dataSize: number
  width?: number
  height?: number
  directToStage?: boolean
  imageEnabled?: boolean
  soundEnabled?: boolean
  pausedAtStart?: boolean
  loop?: boolean
  isStatic?: boolean
  preload?: boolean
  originMode?: string
  playbackMode?: string
  scaleMode?: string
  streamMode?: string
  quality?: string
  eventPassMode?: string
  clickMode?: string
  sourceFileName?: string
}

export interface IShockwave3dMemberSnapshot {
  type: 'shockwave3d'
  regX: number
  regY: number
  dataSize: number
  width: number
  height: number
  directToStage: boolean
  animationEnabled: boolean
  preload: boolean
  loop: boolean
  duration: number
  cameraPosition?: [number, number, number]
  cameraRotation?: [number, number, number]
  bgColor?: string
  ambientColor?: string
  hasScene: boolean
}

export interface IFontMemberSnapshot {
  type: 'font'
}

export interface IUnknownMemberSnapshot {
  type: 'unknown'
}

export interface IScoreBehaviorReference {
  startFrame: number
  endFrame: number
  castLib: number
  castMember: number
  channelNumber: number
}

export interface IScoreSpriteSpan {
  startFrame: number
  endFrame: number
  channelNumber: number
}

export interface IScoreChannelInitData {
  frameIndex: number
  channelNumber: number
  initData: {
    spriteType: number
    castLib: number
    castMember: number
    width: number
    height: number
    locH: number
    locV: number
    unk1: number
    unk2: number
  }
}

export interface ScoreSpriteSnapshot {
  displayName: string
  memberRef: ICastMemberRef
  scriptInstanceList: number[]
  width: number
  height: number
  locH: number
  locV: number
  color: string
  bgColor: string
  ink: number
  blend: number
}

export interface ScoreSnapshot {
  channelCount: number,
  behaviorReferences: IScoreBehaviorReference[]
  spriteSpans?: IScoreSpriteSpan[]
  channelInitData?: IScoreChannelInitData[]
}

export type MemberSnapshot = IBaseMemberSnapshot & (IFieldMemberSnapshot | IScriptMemberSnapshot | IBitmapMemberSnapshot | IPaletteMemberSnapshot | IFontMemberSnapshot | IFlashMemberSnapshot | IShockwave3dMemberSnapshot | IUnknownMemberSnapshot | IFilmLoopMemberSnapshot)
