import { ScoreSnapshot } from "../vm";

export function getScoreFrameBehaviorRef(frameNumber: number, scoreSnapshot: ScoreSnapshot) {
  return scoreSnapshot.behaviorReferences.find(
    (behavior) =>
      behavior.channelNumber === 0  && frameNumber >= behavior.startFrame && frameNumber <= behavior.endFrame
  );
}
