import { PayloadAction, createSlice } from "@reduxjs/toolkit";
import { CastSnapshot, DatumRef, ICastMemberIdentifier, IVMScope, JsBridgeDatum, MemberSnapshot, ScoreSnapshot, ScoreSpriteSnapshot, ScriptInstanceId } from "../vm";
import { ICastMemberRef, JsBridgeBreakpoint, JsBridgeChunk } from "dirplayer-js-api";

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
  movieChunkList: Partial<Record<number, JsBridgeChunk>>,
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
  movieChunkList: {},
}

interface CastMemberListChangedPayload {
  castNumber: number,
  members: Record<number, MemberSnapshot>,
}

const vmSlice = createSlice({
  name: 'vm',
  initialState,
  reducers: {
    ready: (state) => {
      return {
        ...state,
        isReady: true,
      }
    },
    movieChunkListChanged: (state, action: PayloadAction<Partial<Record<number, JsBridgeChunk>>>) => {
      return {
        ...state,
        movieChunkList: action.payload,
      }
    },
    castListChanged: (state, action: PayloadAction<string[]>) => {
      return {
        ...state,
        castNames: action.payload,
      }
    },
    castLibNameChanged: (state, action: PayloadAction<{ castNumber: number, name: string }>) => {
      return {
        ...state,
        castSnapshots: {
          ...state.castSnapshots,
          [action.payload.castNumber]: {
            ...state.castSnapshots[action.payload.castNumber],
            name: action.payload.name,
          }
        },
        castNames: state.castNames.map((name, i) => i === action.payload.castNumber - 1 ? action.payload.name : name)
      }
    },
    castMemberListChanged: (state, action: PayloadAction<CastMemberListChangedPayload>) => {
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
    },
    castMemberChanged: (state, action: PayloadAction<{ memberRef: ICastMemberRef, snapshot: MemberSnapshot }>) => {
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
    },
    scoreChanged: (state, action: PayloadAction<ScoreSnapshot>) => {
      return {
        ...state,
        scoreSnapshot: action.payload
      }
    },
    frameChanged: (state, action: PayloadAction<number>) => {
      return {
        ...state,
        currentFrame: action.payload,
      }
    },
    scopeListChanged: (state, action: PayloadAction<IVMScope[]>) => {
      return {
        ...state,
        scopes: action.payload,
        datumSnapshots: {},
      }
    },
    onScriptError: (state, action: PayloadAction<string>) => {
      return {
        ...state,
        scriptError: action.payload,
      }
    },
    scriptErrorCleared: (state) => {
      return {
        ...state,
        scriptError: undefined,
      }
    },
    breakpointListChanged: (state, action: PayloadAction<JsBridgeBreakpoint[]>) => {
      return {
        ...state,
        breakpoints: action.payload,
      }
    },
    globalsChanged: (state, action: PayloadAction<Record<string, DatumRef>>) => {
      return {
        ...state,
        globals: action.payload,
      }
    },
    setTimeoutHandle: (state, action: PayloadAction<{ name: string, handle: NodeJS.Timer }>) => {
      return {
        ...state,
        timeoutHandles: {
          ...state.timeoutHandles,
          [action.payload.name]: action.payload.handle,
        }
      }
    },
    removeTimeoutHandle: (state, action: PayloadAction<string>) => {
      const newHandles = { ...state.timeoutHandles }
      delete newHandles[action.payload]
      return {
        ...state,
        timeoutHandles: newHandles,
      }
    },
    datumSnapshot: (state, action: PayloadAction<{ datumRef: DatumRef, datum: JsBridgeDatum }>) => {
      return {
        ...state,
        datumSnapshots: {
          ...state.datumSnapshots,
          [action.payload.datumRef]: action.payload.datum,
        }
      }
    },
    scriptInstanceSnapshot: (state, action: PayloadAction<{ scriptInstanceId: ScriptInstanceId, datum: JsBridgeDatum }>) => {
      return {
        ...state,
        scriptInstanceSnapshots: {
          ...state.scriptInstanceSnapshots,
          [action.payload.scriptInstanceId]: action.payload.datum,
        }
      }
    },
    channelChanged: (state, action: PayloadAction<{ channelNumber: number, channelData: ScoreSpriteSnapshot }>) => {
      return {
        ...state,
        channelSnapshots: {
          ...state.channelSnapshots,
          [action.payload.channelNumber]: action.payload.channelData,
        }
      }
    },
    channelDisplayNameChanged: (state, action: PayloadAction<{ channelNumber: number, displayName: string }>) => {
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
    },
    memberSubscribed: (state, action: PayloadAction<TMemberSubscription>) => {
      return {
        ...state,
        subscribedMemberTokens: [...state.subscribedMemberTokens, action.payload],
      }
    },
    memberUnsubscribed: (state, action: PayloadAction<string>) => {
      return {
        ...state,
        subscribedMemberTokens: state.subscribedMemberTokens.filter(t => t.id !== action.payload),
      }
    },
    movieLoaded: (state) => {
      return {
        ...state,
        isMovieLoaded: true,
      }
    },
  },
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

// Action creators are generated for each case reducer function
export const { ready, castListChanged, castLibNameChanged, castMemberListChanged, scoreChanged, frameChanged, scopeListChanged, onScriptError, breakpointListChanged, scriptErrorCleared, globalsChanged, setTimeoutHandle, removeTimeoutHandle, datumSnapshot, scriptInstanceSnapshot, channelChanged, memberSubscribed, memberUnsubscribed, castMemberChanged, channelDisplayNameChanged, movieLoaded, movieChunkListChanged } = vmSlice.actions
export default vmSlice.reducer
