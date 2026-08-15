import { memo, useCallback, useMemo, useRef, useState } from "react";
import classNames from "classnames";
import { useVirtualizer } from "@tanstack/react-virtual";
import styles from "./styles.module.css";
import { IScoreBehaviorReference, IScoreSpriteSpan, ScoreSpriteSnapshot } from "../../vm";
import {
  buildFrameScriptIndex,
  buildScoreIndex,
  findRangeAtFrame,
  findSpanAtFrame,
  ScoreIndex,
} from "../../utils/scoreIndex";
import { usePlayheadVar } from "../../utils/usePlayhead";
import { SCORE_CELL_WIDTH, SCORE_LABEL_WIDTH, SCORE_ROW_HEIGHT } from "../../utils/scoreLayout";

// Geometry is shared via utils/scoreLayout and published to CSS as custom
// properties below, so the virtualizer's arithmetic and the stylesheet can't
// disagree.
const CELL_WIDTH = SCORE_CELL_WIDTH;
const ROW_HEIGHT = SCORE_ROW_HEIGHT;
const LABEL_WIDTH = SCORE_LABEL_WIDTH;
const RULER_HEIGHT = 18;
const SCRIPT_LANE_HEIGHT = 20;

export type FrameScriptLane = {
  behaviorReferences?: IScoreBehaviorReference[];
  /** Frames covered by the current selection, inclusive. */
  selectedRange?: [number, number];
  onSelect: (frame: number) => void;
};

export interface ScoreTimelineProps {
  frameCount: number;
  channelCount: number;
  spriteSpans?: IScoreSpriteSpan[];
  channelSnapshots?: Record<number, ScoreSpriteSnapshot>;
  selectedChannel?: number | false;
  onSelectChannel?: (channel: number) => void;
  onCellClick?: (cell: { channel: number; frame: number }) => void;
  /** Adds the frame-script lane above the ruler. Omitted: no lane. */
  frameScripts?: FrameScriptLane;
  /** Collapses the channel rows, leaving the header (lane + ruler) in place. */
  showRows?: boolean;
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
  frameScripts,
  showRows = true,
}: ScoreTimelineProps) {
  const [selectedCell, setSelectedCell] = useState<ITimelineSelection>();
  const scrollRef = useRef<HTMLDivElement>(null);

  // The playhead writes a CSS variable straight to the DOM; the grid itself
  // never re-renders when the movie advances a frame.
  usePlayheadVar(scrollRef);

  const index = useMemo(
    () => buildScoreIndex(spriteSpans ? { channelCount, spriteSpans, behaviorReferences: [] } : undefined),
    [channelCount, spriteSpans]
  );

  const frameScriptIndex = useMemo(
    () => buildFrameScriptIndex(frameScripts?.behaviorReferences),
    [frameScripts?.behaviorReferences]
  );

  const laneHeight = frameScripts ? SCRIPT_LANE_HEIGHT : 0;
  const headerHeight = laneHeight + RULER_HEIGHT;

  const rowVirtualizer = useVirtualizer({
    count: showRows ? channelCount : 0,
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
    // Reserves the channel-number gutter so the ruler and the rows below share
    // an origin.
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
      style={{
        // Consumed by the stylesheet for cell sizing and playhead placement.
        ['--frame-cell-width' as string]: `${CELL_WIDTH}px`,
        ['--score-row-height' as string]: `${ROW_HEIGHT}px`,
        ['--channel-label-width' as string]: `${LABEL_WIDTH}px`,
      }}
    >
      <div
        className={styles.scoreGrid}
        style={{ width: totalWidth, height: headerHeight + rowVirtualizer.getTotalSize() }}
      >
        {/* Frame-script lane and ruler in one sticky header — the panel's only
            ruler. The rows below are measured against it by construction,
            rather than by keeping two scrollers in step. */}
        <div className={styles.scoreGridHeader} style={{ width: totalWidth, height: headerHeight }}>
          <div className={styles.headerCorner} style={{ height: headerHeight }}>Ch</div>

          {frameScripts && (
            <div className={styles.scriptLane} style={{ height: laneHeight }}>
              {columns.map((column) => {
                const frame = column.index + 1;
                const hasScript = !!findRangeAtFrame(frameScriptIndex, frame);
                const range = frameScripts.selectedRange;
                const isSelected = !!range && frame >= range[0] && frame <= range[1];
                return (
                  <button
                    key={column.index}
                    className={classNames(
                      styles.scriptLaneCell,
                      hasScript && styles.scripted,
                      isSelected && styles.selected
                    )}
                    style={{ left: column.start }}
                    onClick={() => frameScripts.onSelect(frame)}
                    title={hasScript ? `Frame script at frame ${frame}` : undefined}
                  />
                );
              })}
            </div>
          )}

          <div className={styles.rulerLane} style={{ top: laneHeight, height: RULER_HEIGHT }}>
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
