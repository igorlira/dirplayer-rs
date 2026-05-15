import { createAction } from "@reduxjs/toolkit";
import type { DebugContent, ICastMemberRef, JsBridgeBreakpoint } from "dirplayer-js-api";
import type { CastSnapshot, DatumRef, ICastMemberIdentifier, IVMScope, JsBridgeDatum, MemberSnapshot, ScoreSnapshot, ScoreSpriteSnapshot, ScriptInstanceId } from "../vm";
import { createCompatReducer } from "./createCompatReducer";

export type DebugMessageText = { type: 'text'; content: string };
export type DebugMessageBitmap = { type: 'bitmap'; width: number; height: number; data: Uint8Array };
export type DebugMessageDatum = { type: 'datum'; datumRef: DatumRef; snapshot: JsBridgeDatum };
export type DebugMessage = DebugMessageText | DebugMessageBitmap | DebugMessageDatum;

export type TMemberSubscription = {
  memberRef: ICastMemberIdentifier,
  id: string,
}

interface VMSliceState {
  isReady: boolean,
  castNames: string[],
  castSnapshots: Record<number, CastSnapshot>,
  scoreSnapshot?: ScoreSnapshot,
  currentFrame: number,
  scopes: IVMScope[],
  scriptError?: string
  breakpoints: JsBridgeBreakpoint[],
  globals: Record<string, DatumRef>,
  timeoutHandles: Record<string, NodeJS.Timer>,
  datumSnapshots: Record<DatumRef, JsBridgeDatum>,
  scriptInstanceSnapshots: Record<ScriptInstanceId, JsBridgeDatum>,
  channelSnapshots: Record<number, ScoreSpriteSnapshot>,
  subscribedMemberTokens: TMemberSubscription[],
  isMovieLoaded: boolean,
  movieLoadError?: string,
  debugMessages: DebugMessage[],
}

const initialState: VMSliceState = {
  isReady: false,
  castNames: [],
  castSnapshots: [],
  currentFrame: 1,
  scopes: [],
  breakpoints: [],
  globals: {},
  timeoutHandles: {},
  datumSnapshots: {},
  scriptInstanceSnapshots: {},
  channelSnapshots: {},
  subscribedMemberTokens: [],
  isMovieLoaded: false,
  debugMessages: [],
}

interface CastMemberListChangedPayload {
  castNumber: number,
  members: Record<number, MemberSnapshot>,
}

export const ready = createAction('vm/ready')
export const castListChanged = createAction<string[]>('vm/castListChanged')
export const castLibNameChanged = createAction<{ castNumber: number, name: string }>('vm/castLibNameChanged')
export const castMemberListChanged = createAction<CastMemberListChangedPayload>('vm/castMemberListChanged')
export const castMemberChanged = createAction<{ memberRef: ICastMemberRef, snapshot: MemberSnapshot }>('vm/castMemberChanged')
export const scoreChanged = createAction<ScoreSnapshot>('vm/scoreChanged')
export const frameChanged = createAction<number>('vm/frameChanged')
export const scopeListChanged = createAction<IVMScope[]>('vm/scopeListChanged')
export const onScriptError = createAction<string>('vm/onScriptError')
export const scriptErrorCleared = createAction('vm/scriptErrorCleared')
export const breakpointListChanged = createAction<JsBridgeBreakpoint[]>('vm/breakpointListChanged')
export const globalsChanged = createAction<Record<string, DatumRef>>('vm/globalsChanged')
export const setTimeoutHandle = createAction<{ name: string, handle: NodeJS.Timer }>('vm/setTimeoutHandle')
export const removeTimeoutHandle = createAction<string>('vm/removeTimeoutHandle')
export const datumSnapshot = createAction<{ datumRef: DatumRef, datum: JsBridgeDatum }>('vm/datumSnapshot')
export const scriptInstanceSnapshot = createAction<{ scriptInstanceId: ScriptInstanceId, datum: JsBridgeDatum }>('vm/scriptInstanceSnapshot')
export const channelChanged = createAction<{ channelNumber: number, channelData: ScoreSpriteSnapshot }>('vm/channelChanged')
export const channelDisplayNameChanged = createAction<{ channelNumber: number, displayName: string }>('vm/channelDisplayNameChanged')
export const memberSubscribed = createAction<TMemberSubscription>('vm/memberSubscribed')
export const memberUnsubscribed = createAction<string>('vm/memberUnsubscribed')
export const movieLoaded = createAction('vm/movieLoaded')
export const movieLoadFailed = createAction<string>('vm/movieLoadFailed')
export const movieUnloaded = createAction('vm/movieUnloaded')
export const debugMessageAdded = createAction<string>('vm/debugMessageAdded')
export const debugContentAdded = createAction<DebugContent>('vm/debugContentAdded')
export const debugMessagesCleared = createAction('vm/debugMessagesCleared')

const vmReducer = createCompatReducer(initialState, (builder) => {
  builder
    .addCase(ready, (state) => {
      return {
        ...state,
        isReady: true,
      }
    })
    .addCase(castListChanged, (state, action) => {
      return {
        ...state,
        castNames: action.payload,
      }
    })
    .addCase(castLibNameChanged, (state, action) => {
      return {
        ...state,
        castSnapshots: {
          ...state.castSnapshots,
          [action.payload.castNumber]: {
            ...(state.castSnapshots[action.payload.castNumber] as (CastSnapshot & { name?: string }) | undefined),
            name: action.payload.name,
          } as CastSnapshot,
        },
        castNames: state.castNames.map((name, i) => i === action.payload.castNumber - 1 ? action.payload.name : name)
      }
    })
    .addCase(castMemberListChanged, (state, action) => {
      return {
        ...state,
        castSnapshots: {
          ...state.castSnapshots,
          [action.payload.castNumber]: {
            number: action.payload.castNumber,
            members: action.payload.members
          }
        }
      }
    })
    .addCase(castMemberChanged, (state, action) => {
      const castLibNum = action.payload.memberRef[0]
      const memberNum = action.payload.memberRef[1]
      return {
        ...state,
        castSnapshots: {
          ...state.castSnapshots,
          [castLibNum]: {
            ...state.castSnapshots[castLibNum],
            members: {
              ...state.castSnapshots[castLibNum].members,
              [memberNum]: {
                ...state.castSnapshots[castLibNum].members[memberNum],
                snapshot: action.payload.snapshot,
              }
            }
          },
        }
      }
    })
    .addCase(scoreChanged, (state, action) => {
      return {
        ...state,
        scoreSnapshot: action.payload
      }
    })
    .addCase(frameChanged, (state, action) => {
      return {
        ...state,
        currentFrame: action.payload,
      }
    })
    .addCase(scopeListChanged, (state, action) => {
      return {
        ...state,
        scopes: action.payload,
        datumSnapshots: {},
      }
    })
    .addCase(onScriptError, (state, action) => {
      return {
        ...state,
        scriptError: action.payload,
      }
    })
    .addCase(scriptErrorCleared, (state) => {
      return {
        ...state,
        scriptError: undefined,
      }
    })
    .addCase(breakpointListChanged, (state, action) => {
      return {
        ...state,
        breakpoints: action.payload,
      }
    })
    .addCase(globalsChanged, (state, action) => {
      return {
        ...state,
        globals: action.payload,
      }
    })
    .addCase(setTimeoutHandle, (state, action) => {
      return {
        ...state,
        timeoutHandles: {
          ...state.timeoutHandles,
          [action.payload.name]: action.payload.handle,
        }
      }
    })
    .addCase(removeTimeoutHandle, (state, action) => {
      const newHandles = { ...state.timeoutHandles }
      delete newHandles[action.payload]
      return {
        ...state,
        timeoutHandles: newHandles,
      }
    })
    .addCase(datumSnapshot, (state, action) => {
      return {
        ...state,
        datumSnapshots: {
          ...state.datumSnapshots,
          [action.payload.datumRef]: action.payload.datum,
        }
      }
    })
    .addCase(scriptInstanceSnapshot, (state, action) => {
      return {
        ...state,
        scriptInstanceSnapshots: {
          ...state.scriptInstanceSnapshots,
          [action.payload.scriptInstanceId]: action.payload.datum,
        }
      }
    })
    .addCase(channelChanged, (state, action) => {
      return {
        ...state,
        channelSnapshots: {
          ...state.channelSnapshots,
          [action.payload.channelNumber]: action.payload.channelData,
        }
      }
    })
    .addCase(channelDisplayNameChanged, (state, action) => {
      return {
        ...state,
        channelSnapshots: {
          ...state.channelSnapshots,
          [action.payload.channelNumber]: {
            ...state.channelSnapshots[action.payload.channelNumber],
            displayName: action.payload.displayName,
          }
        }
      }
    })
    .addCase(memberSubscribed, (state, action) => {
      return {
        ...state,
        subscribedMemberTokens: [...state.subscribedMemberTokens, action.payload],
      }
    })
    .addCase(memberUnsubscribed, (state, action) => {
      return {
        ...state,
        subscribedMemberTokens: state.subscribedMemberTokens.filter(t => t.id !== action.payload),
      }
    })
    .addCase(movieLoaded, (state) => {
      return {
        ...state,
        isMovieLoaded: true,
        movieLoadError: undefined,
      }
    })
    .addCase(movieLoadFailed, (state, action) => {
      return {
        ...state,
        movieLoadError: action.payload,
      }
    })
    .addCase(movieUnloaded, (state) => {
      return {
        ...initialState,
        isReady: state.isReady,
      }
    })
    .addCase(debugMessageAdded, (state, action) => {
      return {
        ...state,
        debugMessages: [...state.debugMessages, { type: 'text' as const, content: action.payload }],
      }
    })
    .addCase(debugContentAdded, (state, action) => {
      return {
        ...state,
        debugMessages: [...state.debugMessages, action.payload as DebugMessage],
      }
    })
    .addCase(debugMessagesCleared, (state) => {
      return {
        ...state,
        debugMessages: [],
      }
    })
})

export const selectCastSnapshot = (state: VMSliceState, number: number) => state.castSnapshots[number]
export const selectMemberSnapshotById = (state: VMSliceState, id: ICastMemberIdentifier) => selectMemberSnapshot(state, id.castNumber, id.memberNumber)
export const selectMemberSnapshot = (state: VMSliceState, castNumber: number, memberNumber: number): MemberSnapshot | undefined => selectCastSnapshot(state, castNumber).members[String(memberNumber)]?.snapshot
export const selectScoreSnapshot = (state: VMSliceState): ScoreSnapshot | undefined => state.scoreSnapshot
export const selectCurrentFrame = (state: VMSliceState) => state.currentFrame
export const selectScopes = (state: VMSliceState) => state.scopes
export const selectCurrentScope = (state: VMSliceState) => state.scopes.at(state.scopes.length - 1)
export const selectScriptError = (state: VMSliceState) => state.scriptError
export const selectBreakpoints = (state: VMSliceState, scriptName?: string) => state.breakpoints.filter(b => !scriptName || b.script_name === scriptName)
export const selectGlobals = (state: VMSliceState) => state.globals
export const selectDebugMessages = (state: VMSliceState) => state.debugMessages

export default vmReducer
