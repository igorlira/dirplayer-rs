import { ICastMemberRef } from "dirplayer-js-api";
import { IScoreSpriteSpan, ScoreSnapshot } from "../vm";

export function getScoreFrameBehaviorRef(frameNumber: number, scoreSnapshot: ScoreSnapshot) {
  return scoreSnapshot.behaviorReferences.find(
    (behavior) =>
      behavior.channelNumber === 0  && frameNumber >= behavior.startFrame && frameNumber <= behavior.endFrame
  );
}

/**
 * The member shown in a channel at a frame.
 *
 * This used to aggregate `channelInitData` — a per-frame table of every channel
 * mutation, tens of thousands of rows, shipped across the wasm boundary in full
 * just to answer this one question. The VM now splits sprite spans whenever the
 * member changes and puts the member on the span, so the covering span is the
 * answer and the table never has to cross at all.
 */
export function findSpriteMemberAtFrame(
  spriteSpans: IScoreSpriteSpan[],
  channel: number,
  frame: number
): ICastMemberRef | undefined {
  const span = spriteSpans.find(
    (s) => s.channelNumber === channel && frame >= s.startFrame && frame <= s.endFrame
  );
  return span?.memberRef;
}
