import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import classNames from "classnames";
import { useVirtualizer } from "@tanstack/react-virtual";
import styles from "./styles.module.css";
import { IScoreSpriteSpan, ScoreSpriteSnapshot } from "../../vm";
import { buildScoreIndex, findSpanAtFrame, ScoreIndex } from "../../utils/scoreIndex";
import { usePlayheadVar } from "../../utils/usePlayhead";
import { SCORE_CELL_WIDTH, SCORE_LABEL_WIDTH, SCORE_ROW_HEIGHT } from "../../utils/scoreLayout";

// Geometry is shared with the score inspector's ruler so the two line up; see
// utils/scoreLayout. Published to CSS as custom properties below, so the
// virtualizer's arithmetic and the stylesheet can't disagree either.
const CELL_WIDTH = SCORE_CELL_WIDTH;
const ROW_HEIGHT = SCORE_ROW_HEIGHT;
const LABEL_WIDTH = SCORE_LABEL_WIDTH;
const HEADER_HEIGHT = 18;

export interface ScoreTimelineProps {
  frameCount: number;
  channelCount: number;
  spriteSpans?: IScoreSpriteSpan[];
  channelSnapshots?: Record<number, ScoreSpriteSnapshot>;
  selectedChannel?: number | false;
  onSelectChannel?: (channel: number) => void;
  onCellClick?: (cell: { channel: number; frame: number }) => void;
  /** Receives the scroll element, so a sibling ruler can be kept in step. */
  onScrollerRef?: (element: HTMLDivElement | null) => void;
  /** Fired on horizontal scroll, for the same reason. */
  onScrollLeftChange?: (scrollLeft: number) => void;
}

interface ITimelineSelection {
  channel: number;
  frame: number;
}

type CellProps = {
  channel: number;
  frame: number;
  left: number;
  index: ScoreIndex;
  selectedCell?: ITimelineSelection;
  clickable: boolean;
  onClick: (cell: ITimelineSelection) => void;
};

const TimelineCell = memo(function TimelineCell({
  channel,
  frame,
  left,
  index,
  selectedCell,
  clickable,
  onClick,
}: CellProps) {
  const spans = index.spansByChannel.get(channel);
  const span = findSpanAtFrame(spans, frame);
  const isSpanStart = span && frame === span.startFrame;
  const isSpanEnd = span && frame === span.endFrame;

  let castMember: string | null = null;
  if (isSpanStart && span.memberRef) {
    castMember = `${span.memberRef[0]}:${span.memberRef[1]}`;
  }

  const isCellSelected = selectedCell?.channel === channel && selectedCell?.frame === frame;
  const isSpanSelected =
    span &&
    selectedCell &&
    channel === selectedCell.channel &&
    selectedCell.frame >= span.startFrame &&
    selectedCell.frame <= span.endFrame;

  return (
    <div
      className={classNames(
        styles.scoreGridCell,
        span && styles.hasSprite,
        isSpanStart && styles.spanStart,
        isSpanEnd && styles.spanEnd,
        span && clickable && styles.clickable,
        isCellSelected && !span && styles.emptySelected,
        isSpanSelected && styles.spanSelected
      )}
      style={{ left }}
      title={castMember || undefined}
      onClick={() => onClick({ channel, frame })}
    >
      {isSpanStart && castMember && (
        <div className={styles.castMemberLabel}>{castMember}</div>
      )}
    </div>
  );
});

export default function ScoreTimeline({
  frameCount,
  channelCount,
  spriteSpans,
  channelSnapshots,
  selectedChannel,
  onSelectChannel,
  onCellClick,
  onScrollerRef,
  onScrollLeftChange,
}: ScoreTimelineProps) {
  const [selectedCell, setSelectedCell] = useState<ITimelineSelection>();
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    onScrollerRef?.(scrollRef.current);
    return () => onScrollerRef?.(null);
  }, [onScrollerRef]);

  // The playhead writes a CSS variable straight to the DOM; the grid itself
  // never re-renders when the movie advances a frame.
  usePlayheadVar(scrollRef);

  const index = useMemo(
    () => buildScoreIndex(spriteSpans ? { channelCount, spriteSpans, behaviorReferences: [] } : undefined),
    [channelCount, spriteSpans]
  );

  const rowVirtualizer = useVirtualizer({
    count: channelCount,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 6,
  });

  const columnVirtualizer = useVirtualizer({
    horizontal: true,
    count: frameCount,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => CELL_WIDTH,
    overscan: 12,
    paddingStart: LABEL_WIDTH,
  });

  const handleCellClick = useCallback(
    (cell: ITimelineSelection) => {
      onCellClick?.(cell);
      setSelectedCell(cell);
    },
    [onCellClick]
  );

  const columns = columnVirtualizer.getVirtualItems();
  const rows = rowVirtualizer.getVirtualItems();
  const totalWidth = LABEL_WIDTH + frameCount * CELL_WIDTH;

  return (
    <div
      ref={scrollRef}
      className={styles.scoreOverviewContainer}
      onScroll={onScrollLeftChange ? (e) => onScrollLeftChange(e.currentTarget.scrollLeft) : undefined}
      style={{
        // Consumed by the stylesheet for cell sizing and playhead placement.
        ['--frame-cell-width' as string]: `${CELL_WIDTH}px`,
        ['--score-row-height' as string]: `${ROW_HEIGHT}px`,
        ['--channel-label-width' as string]: `${LABEL_WIDTH}px`,
      }}
    >
      <div
        className={styles.scoreGrid}
        style={{ width: totalWidth, height: HEADER_HEIGHT + rowVirtualizer.getTotalSize() }}
      >
        <div className={styles.scoreGridHeader} style={{ width: totalWidth }}>
          <div className={styles.channelLabelCell}>Ch</div>
          {columns.map((column) => {
            const frame = column.index + 1;
            return (
              <div
                key={column.index}
                className={styles.scoreGridFrameCell}
                style={{ left: column.start }}
              >
                {frame === 1 || frame % 5 === 0 ? frame : "·"}
              </div>
            );
          })}
        </div>

        <div className={styles.scoreGridBody} style={{ height: rowVirtualizer.getTotalSize() }}>
          {rows.map((row) => {
            const channel = row.index + 1;
            const sprite = channelSnapshots?.[channel];
            return (
              <div
                key={row.index}
                className={classNames(styles.scoreGridRow, channel % 2 === 0 && styles.rowAlt)}
                style={{ top: row.start, width: totalWidth }}
              >
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
                {columns.map((column) => (
                  <TimelineCell
                    key={column.index}
                    channel={channel}
                    frame={column.index + 1}
                    left={column.start}
                    index={index}
                    selectedCell={selectedCell}
                    clickable={!!onCellClick}
                    onClick={handleCellClick}
                  />
                ))}
              </div>
            );
          })}
        </div>

        {frameCount > 0 && <div className={styles.playhead} aria-hidden="true" />}
      </div>
    </div>
  );
}
