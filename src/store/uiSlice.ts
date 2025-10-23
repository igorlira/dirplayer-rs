import { PayloadAction, createSlice } from "@reduxjs/toolkit";
import { ICastMemberRef } from "dirplayer-js-api";

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

interface UISliceState {
  selectedObject?: TSelectedObject
}

const initialState: UISliceState = {

}

const uiSlice = createSlice({
  name: 'ui',
  initialState,
  reducers: {
    onMemberSelected(state, action: PayloadAction<ICastMemberRef>) {
      return {
        ...state,
        selectedObject: {
          type: 'member',
          memberRef: action.payload
        }
      }
    },
    channelSelected(state, action: PayloadAction<number>) {
      return {
        ...state,
        selectedObject: {
          type: 'sprite',
          spriteNumber: action.payload
        }
      }
    },
    scoreSpanSelected(state, action: PayloadAction<TScoreSpanRef>) {
      return {
        ...state,
        selectedObject: {
          type: 'scoreSpan',
          spanRef: action.payload
        }
      }
    },
    scoreBehaviorSelected(state, action: PayloadAction<{frameNumber: number}>) {
      return {
        ...state,
        selectedObject: {
          type: 'scoreBehavior',
          frameNumber: action.payload.frameNumber
        }
      }
    },
  },
})

export const selectSelectedMemberRef = (state: UISliceState) => state.selectedObject?.type === 'member' ? state.selectedObject.memberRef : undefined

// Action creators are generated for each case reducer function
export const { onMemberSelected, channelSelected, scoreBehaviorSelected, scoreSpanSelected } = uiSlice.actions
export default uiSlice.reducer
