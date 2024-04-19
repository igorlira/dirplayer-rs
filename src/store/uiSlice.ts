import { PayloadAction, createSlice } from "@reduxjs/toolkit";
import { ICastMemberRef } from "dirplayer-js-api";

export type TSelectedObjectSprite = {
  type: 'sprite',
  spriteNumber: number
}

export type TSelectedObjectMember = {
  type: 'member',
  memberRef: ICastMemberRef
}

export type TSelectedObject = TSelectedObjectSprite | TSelectedObjectMember

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
  },
})

export const selectSelectedMemberRef = (state: UISliceState) => state.selectedObject?.type === 'member' ? state.selectedObject.memberRef : undefined

// Action creators are generated for each case reducer function
export const { onMemberSelected, channelSelected } = uiSlice.actions
export default uiSlice.reducer
