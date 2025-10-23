import { range } from "lodash";
import classNames from "classnames";
import styles from "./styles.module.css";
import { IScoreSpriteSpan, IScoreChannelInitData, ScoreSpriteSnapshot } from "../../vm";

export interface ScoreTimelineProps {
  framesToRender: number;
  currentFrame?: number;
  channelCount: number;
  spriteSpans?: IScoreSpriteSpan[];
  channelInitData?: IScoreChannelInitData[];
  channelSnapshots?: Record<number, ScoreSpriteSnapshot>;
  selectedChannel?: number | false;
  onSelectChannel?: (channel: number) => void;
  onCellClick?: () => void;
}

export default function ScoreTimeline({
  framesToRender,
  currentFrame,
  channelCount,
  spriteSpans,
  channelInitData,
  channelSnapshots,
  selectedChannel,
  onSelectChannel,
  onCellClick,
}: ScoreTimelineProps) {
  const getSpansForChannel = (channel: number) => {
    return spriteSpans?.filter((span) => span.channelNumber === channel) || [];
  };

  const getCastMemberForChannel = (channel: number, frame: number) => {
    const initData = channelInitData?.find(
      (data) => data.channelNumber === channel && data.frameIndex === frame
    );
    if (initData) {
      return `${initData.initData.castLib}:${initData.initData.castMember}`;
    }
    return null;
  };

  return (
    <div className={styles.scoreOverviewContainer}>
      <div className={styles.scoreGrid}>
        <div className={styles.scoreGridHeader}>
          <div className={styles.channelLabelCell}>Ch</div>
          {range(1, framesToRender + 1).map((frame) => (
            <div
              key={frame}
              className={classNames(
                styles.scoreGridFrameCell,
                currentFrame === frame && styles.current
              )}
            >
              {(frame === 1 || frame % 5 === 0) ? frame : "·"}
            </div>
          ))}
        </div>
        {Array.from({ length: channelCount }, (_, i) => i + 1).map((channel) => {
          const spans = getSpansForChannel(channel);
          const sprite = channelSnapshots?.[channel];
          return (
            <div key={channel} className={styles.scoreGridRow}>
              <div
                className={classNames(
                  styles.channelLabelCell,
                  selectedChannel === channel && styles.selected
                )}
                onClick={() => onSelectChannel?.(channel)}
                title={sprite?.displayName}
              >
                {channel}
              </div>
              {range(1, framesToRender + 1).map((frame) => {
                const span = spans.find(
                  (s) => frame >= s.startFrame && frame <= s.endFrame
                );
                const isSpanStart = span && frame === span.startFrame;
                const castMember = isSpanStart ? getCastMemberForChannel(channel, frame) : null;

                const handleCellClick = () => {
                  // TODO
                };

                return (
                  <div
                    key={frame}
                    className={classNames(
                      styles.scoreGridCell,
                      span && styles.hasSprite,
                      isSpanStart && styles.spanStart,
                      currentFrame === frame && styles.currentFrame,
                      span && onCellClick && styles.clickable
                    )}
                    title={castMember || undefined}
                    onClick={handleCellClick}
                  >
                    {isSpanStart && castMember && (
                      <div className={styles.castMemberLabel}>{castMember}</div>
                    )}
                  </div>
                );
              })}
            </div>
          );
        })}
      </div>
    </div>
  );
}
