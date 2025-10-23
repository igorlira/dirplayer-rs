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

export type JsBridgeDatum = TJsBridgeDatumScriptInstance | TJsBridgeDatumList | TJsBridgeDatumPropList | TJsBridgeDatumVoid// | TJsBridgeDatumUnknown;

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
}

export interface IBytecodeSnapshot {
  pos: number
  text: string
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
}

export interface ScoreSnapshot {
  channelCount: number,
  behaviorReferences: IScoreBehaviorReference[]
  spriteSpans?: IScoreSpriteSpan[]
  channelInitData?: IScoreChannelInitData[]
}

export type MemberSnapshot = IBaseMemberSnapshot & (IFieldMemberSnapshot | IScriptMemberSnapshot | IBitmapMemberSnapshot | IPaletteMemberSnapshot | IUnknownMemberSnapshot | IFilmLoopMemberSnapshot)
