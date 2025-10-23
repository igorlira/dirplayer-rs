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

  const selectedScoreRef = useMemo(() => {
    if (selectedObject?.type === "scoreSpan" && selectedObject.spanRef.scoreRef !== 'stage') {
      return selectedObject.spanRef.scoreRef;
    }
  }, [selectedObject]);

  const selectedScoreSnapshot = useAppSelector((state) => {
    if (selectedScoreRef) {
      return castSnapshots[selectedScoreRef[0]]?.members?.[selectedScoreRef[1]];
    }
  });

  const memberRef = useMemo(() => {
    if (selectedObject?.type === "member") {
      return selectedObject.memberRef;
    } else if (selectedObject?.type === "scoreBehavior" && scoreBehaviorRef) {
      return [scoreBehaviorRef.castLib, scoreBehaviorRef.castMember];
    } else if (selectedObject?.type === "sprite" && selectedSprite) {
      return selectedSprite.memberRef;
    } else if (selectedObject?.type === "scoreSpan" && selectedObject.spanRef.scoreRef === 'stage' && scoreSnapshot?.channelInitData) {
      return getAggregatedSpriteDataForChannelAtFrame(scoreSnapshot.channelInitData, selectedObject.spanRef.channelNumber, selectedObject.spanRef.frameNumber)?.memberRef;
    } else if (selectedObject?.type === "scoreSpan" && selectedObject.spanRef.scoreRef !== 'stage') {
      return selectedObject.spanRef.scoreRef;
    }
  }, [selectedObject, scoreBehaviorRef, selectedSprite, scoreSnapshot]);

  const secondaryMemberRef = useMemo(() => {
    if (selectedObject?.type === "scoreSpan" && selectedObject.spanRef.scoreRef !== 'stage' && selectedScoreSnapshot?.snapshot?.type === 'filmLoop') {
      const memberRef = getAggregatedSpriteDataForChannelAtFrame(selectedScoreSnapshot.snapshot.score?.channelInitData || [], selectedObject.spanRef.channelNumber, selectedObject.spanRef.frameNumber)?.memberRef;
      if (memberRef) {
        const [castLib, memberNumber] = memberRef;
        const actualCastLib = (castLib === 65535 || castLib === 0) && selectedScoreRef ? selectedScoreRef[0] : castLib;
        return [actualCastLib, memberNumber];
      } else {
        return undefined;
      }
    }
  }, [selectedObject, selectedScoreSnapshot, selectedScoreRef]);

  const member = useMemo(() => {
    if (memberRef) {
      return castSnapshots[memberRef[0]]?.members?.[memberRef[1]];
    }
  }, [memberRef, castSnapshots]);

  const secondaryMember = useMemo(() => {
    if (secondaryMemberRef) {
      return castSnapshots[secondaryMemberRef[0]]?.members?.[secondaryMemberRef[1]];
    }
  }, [secondaryMemberRef, castSnapshots]);

  return {
    scoreBehaviorRef,
    selectedSprite,
    member,
    secondaryMember,
    memberRef,
    secondaryMemberRef,
  };
}