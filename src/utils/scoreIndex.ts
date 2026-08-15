import { ICastMemberRef } from "dirplayer-js-api";
import { IScoreBehaviorReference, IScoreSpriteSpan, ScoreSnapshot } from "../vm";

// Lookup structures for the score views.
//
// The timeline asks "what is in channel C at frame F?" once per visible cell.
// Answering that by scanning `spriteSpans` — as the views used to — is
// O(cells x spans), which on a real movie (hundreds of channels, thousands of
// frames, thousands of spans) is millions of comparisons per render. This
// buckets the spans by channel once per snapshot, then answers each question
// with a binary search.

export type ScoreIndex = {
  /** Spans per channel, sorted by startFrame and non-overlapping. */
  spansByChannel: Map<number, IScoreSpriteSpan[]>;
};

export const EMPTY_SCORE_INDEX: ScoreIndex = {
  spansByChannel: new Map(),
};

export function buildScoreIndex(score?: ScoreSnapshot): ScoreIndex {
  if (!score) return EMPTY_SCORE_INDEX;

  const spansByChannel = new Map<number, IScoreSpriteSpan[]>();
  for (const span of score.spriteSpans ?? []) {
    const list = spansByChannel.get(span.channelNumber);
    if (list) list.push(span);
    else spansByChannel.set(span.channelNumber, [span]);
  }
  for (const list of Array.from(spansByChannel.values())) {
    list.sort((a, b) => a.startFrame - b.startFrame);
  }

  return { spansByChannel };
}

type FrameRange = { startFrame: number; endFrame: number };

/**
 * The range covering `frame`, by binary search. Requires the list to be sorted
 * by startFrame and non-overlapping, which both sprite spans (per channel) and
 * frame-script behaviours are.
 */
export function findRangeAtFrame<T extends FrameRange>(
  ranges: T[] | undefined,
  frame: number
): T | undefined {
  if (!ranges || ranges.length === 0) return undefined;
  let lo = 0;
  let hi = ranges.length - 1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    const range = ranges[mid];
    if (frame < range.startFrame) hi = mid - 1;
    else if (frame > range.endFrame) lo = mid + 1;
    else return range;
  }
  return undefined;
}

/** The span covering `frame` in this channel, or undefined. */
export function findSpanAtFrame(
  spans: IScoreSpriteSpan[] | undefined,
  frame: number
): IScoreSpriteSpan | undefined {
  return findRangeAtFrame(spans, frame);
}

/**
 * Frame scripts (behaviours in channel 0), sorted for findRangeAtFrame. The
 * ruler asks per visible frame, so a scan of the whole behaviour list per cell
 * would be needless work.
 */
export function buildFrameScriptIndex(
  behaviorReferences?: IScoreBehaviorReference[]
): IScoreBehaviorReference[] {
  return (behaviorReferences ?? [])
    .filter((behavior) => behavior.channelNumber === 0)
    .sort((a, b) => a.startFrame - b.startFrame);
}

/**
 * The member shown in a channel at a frame. The VM splits spans whenever the
 * member changes, so the covering span carries the answer — no separate
 * per-frame init-data table has to cross the boundary for this.
 */
export function findMemberRefAtFrame(
  spans: IScoreSpriteSpan[] | undefined,
  frame: number
): ICastMemberRef | undefined {
  return findSpanAtFrame(spans, frame)?.memberRef;
}

// Director's own ceilings, used as a sanity bound rather than as a display
// limit. A score cannot exceed 32,000 frames; channels top out at 1,000 sprite
// channels plus the reserved effects channels (tempo, palette, transition, two
// sound, script), so counts a little over 1,000 are normal and must not be
// trimmed — 1,024 leaves room for them.
//
// Anything past these is a garbage value out of a malformed score chunk, and
// the views must not size themselves from it: the virtualizer allocates
// measurement arrays proportional to the item count, so a bogus multi-billion
// frame count asks for a multi-gigabyte buffer and takes the tab down. Seen in
// the wild: a movie reporting 1,970,566,255 frames (0x7574746F — "utto", read
// out of the middle of the string "button").
export const MAX_SCORE_FRAMES = 32_000;
export const MAX_SCORE_CHANNELS = 1_024;

function clampCount(raw: number, limit: number, what: string): number {
  if (!Number.isFinite(raw) || raw <= 0) return 0;
  const value = Math.floor(raw);
  if (value > limit) {
    console.warn(
      `[score] ${what} reported as ${value}, beyond the sane limit of ${limit}. ` +
      `Clamping; the score chunk is probably malformed.`
    );
    return limit;
  }
  return value;
}

/** Frames in the score. Falls back to the data when the VM didn't report one. */
export function getFrameCount(score?: ScoreSnapshot): number {
  if (!score) return 0;
  if (score.frameCount && score.frameCount > 0) {
    return clampCount(score.frameCount, MAX_SCORE_FRAMES, 'frameCount');
  }
  let max = 1;
  for (const span of score.spriteSpans ?? []) max = Math.max(max, span.endFrame);
  for (const behavior of score.behaviorReferences ?? []) max = Math.max(max, behavior.endFrame);
  return clampCount(max, MAX_SCORE_FRAMES, 'derived frame count');
}

/** Sprite channels in the score, clamped the same way. */
export function getChannelCount(score?: ScoreSnapshot): number {
  if (!score) return 0;
  return clampCount(score.channelCount, MAX_SCORE_CHANNELS, 'channelCount');
}
