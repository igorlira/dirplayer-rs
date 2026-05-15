import { createAction } from "@reduxjs/toolkit";
import type { ICastMemberRef } from "dirplayer-js-api";
import { createCompatReducer } from "./createCompatReducer";

export type TSelectedObjectSprite = {
  type: 'sprite',
  spriteNumber: number
}

export type TScoreSpanRef = {
  channelNumber: number,
  frameNumber: number,
  scoreRef: 'stage' | ICastMemberRef
}

export type TSelectedObjectScoreSpan = {
  type: 'scoreSpan',
  spanRef: TScoreSpanRef
}

export type TSelectedObjectMember = {
  type: 'member',
  memberRef: ICastMemberRef
}

export type TSelectedObjectScoreBehavior = {
  type: 'scoreBehavior',
  frameNumber: number
}

export type TSelectedObject = TSelectedObjectSprite | TSelectedObjectMember | TSelectedObjectScoreBehavior | TSelectedObjectScoreSpan

export type TScriptViewMode = 'lingo' | 'assembly';

interface UISliceState {
  selectedObject?: TSelectedObject
  scriptViewMode: TScriptViewMode
}

const initialState: UISliceState = {
  scriptViewMode: 'lingo'
}

export const onMemberSelected = createAction<ICastMemberRef>('ui/onMemberSelected')
export const channelSelected = createAction<number>('ui/channelSelected')
export const scoreSpanSelected = createAction<TScoreSpanRef>('ui/scoreSpanSelected')
export const scoreBehaviorSelected = createAction<{ frameNumber: number }>('ui/scoreBehaviorSelected')
export const scriptViewModeChanged = createAction<TScriptViewMode>('ui/scriptViewModeChanged')

const uiReducer = createCompatReducer(initialState, (builder) => {
  builder
    .addCase(onMemberSelected, (state, action) => {
      return {
        ...state,
        selectedObject: {
          type: 'member',
          memberRef: action.payload
        }
      }
    })
    .addCase(channelSelected, (state, action) => {
      return {
        ...state,
        selectedObject: {
          type: 'sprite',
          spriteNumber: action.payload
        }
      }
    })
    .addCase(scoreSpanSelected, (state, action) => {
      return {
        ...state,
        selectedObject: {
          type: 'scoreSpan',
          spanRef: action.payload
        }
      }
    })
    .addCase(scoreBehaviorSelected, (state, action) => {
      return {
        ...state,
        selectedObject: {
          type: 'scoreBehavior',
          frameNumber: action.payload.frameNumber
        }
      }
    })
    .addCase(scriptViewModeChanged, (state, action) => {
      return {
        ...state,
        scriptViewMode: action.payload
      }
    })
})

export const selectSelectedMemberRef = (state: UISliceState) => state.selectedObject?.type === 'member' ? state.selectedObject.memberRef : undefined
export const selectScriptViewMode = (state: UISliceState) => state.scriptViewMode

export default uiReducer
