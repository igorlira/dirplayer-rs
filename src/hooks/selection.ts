import { useMemo } from "react";
import { useAppSelector } from "../store/hooks";
import { getAggregatedSpriteDataForChannelAtFrame, getScoreFrameBehaviorRef } from "../utils/score";

export function useSelectedObjects() {
  const castSnapshots = useAppSelector((state) => state.vm.castSnapshots);
  const selectedObject = useAppSelector((state) => state.ui.selectedObject);
  const selectedSpriteNumber =
    selectedObject?.type === "sprite" && selectedObject.spriteNumber;
  const selectedSprite = useAppSelector(
    (state) =>
      (!!selectedSpriteNumber &&
        state.vm.channelSnapshots[selectedSpriteNumber]) ||
      undefined
  );
  const scoreSnapshot = useAppSelector((state) => state.vm.scoreSnapshot);
  const scoreBehaviorRef = useMemo(() => {
    if (scoreSnapshot && selectedObject?.type === "scoreBehavior") {
      return getScoreFrameBehaviorRef(selectedObject.frameNumber, scoreSnapshot);
    }
  }, [scoreSnapshot, selectedObject]);

  const memberRef = useMemo(() => {
    if (selectedObject?.type === "member") {
      return selectedObject.memberRef;
    } else if (selectedObject?.type === "scoreBehavior" && scoreBehaviorRef) {
      return [scoreBehaviorRef.castLib, scoreBehaviorRef.castMember];
    } else if (selectedObject?.type === "sprite" && selectedSprite) {
      return selectedSprite.memberRef;
    } else if (selectedObject?.type === "scoreSpan" && selectedObject.spanRef.scoreRef === 'stage' && scoreSnapshot?.channelInitData) {
      return getAggregatedSpriteDataForChannelAtFrame(scoreSnapshot.channelInitData, selectedObject.spanRef.channelNumber, selectedObject.spanRef.frameNumber)?.memberRef;
    }
  }, [selectedObject, scoreBehaviorRef, selectedSprite, scoreSnapshot]);

  const member = useMemo(() => {
    if (memberRef) {
      return castSnapshots[memberRef[0]]?.members?.[memberRef[1]];
    }
  }, [memberRef, castSnapshots]);

  return {
    scoreBehaviorRef,
    selectedSprite,
    member,
    memberRef,
  };
}