import { ICastMemberRef } from "dirplayer-js-api";
import { IScoreChannelInitData, ScoreSnapshot } from "../vm";

export function getScoreFrameBehaviorRef(frameNumber: number, scoreSnapshot: ScoreSnapshot) {
  return scoreSnapshot.behaviorReferences.find(
    (behavior) =>
      behavior.channelNumber === 0  && frameNumber >= behavior.startFrame && frameNumber <= behavior.endFrame
  );
}

type TAggregatedSpriteData = {
  memberRef?: ICastMemberRef;
};

export const getAggregatedSpriteDataForChannelAtFrame = (channelInitData: IScoreChannelInitData[], channel: number, frame: number) => {
  const initData = channelInitData?.filter(
    (data) => data.channelNumber === channel && data.frameIndex <= frame
  );
  return initData?.reduce<TAggregatedSpriteData | null>((result, item) => {
    return {
      ...result,
      ...(item.initData.castLib || item.initData.castMember ? {
        memberRef: [item.initData.castLib, item.initData.castMember]
      } : {})
    };
  }, null);
};
