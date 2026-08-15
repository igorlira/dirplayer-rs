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
  sliceRangesInWindow,
} from "../../utils/scoreIndex";
import { usePlayheadVar } from "../../utils/usePlayhead";
import { getScrollbarSize, SCORE_CELL_WIDTH, SCORE_LABEL_WIDTH, SCORE_ROW_HEIGHT } from "../../utils/scoreLayout";

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

const EMPTY_SPANS: IScoreSpriteSpan[] = [];

type ChannelLaneProps = {
  channel: number;
  spans: IScoreSpriteSpan[];
  firstFrame: number;
  lastFrame: number;
  selected?: ITimelineSelection;
  clickable: boolean;
  onClick: (cell: ITimelineSelection) => void;
};

/**
 * One channel's clips.
 *
 * A sprite span is drawn as a single bar across its frame range, not as one
 * element per frame. The old per-cell rendering made a continuous sprite read
 * as a row of disconnected boxes — a selected span appeared as a dozen separate
 * outlines — and left no room for the member label, which had to be crammed
 * into the 16px start cell and clipped. Drawing the span once also means the
 * DOM holds a handful of bars per row instead of one node per visible frame,
 * and the frame grid behind them is a repeating CSS gradient rather than
 * thousands of bordered divs.
 */
const ChannelLane = memo(function ChannelLane({
  channel,
  spans,
  firstFrame,
  lastFrame,
  selected,
  clickable,
  onClick,
}: ChannelLaneProps) {
  const handleLaneClick = (event: React.MouseEvent<HTMLDivElement>) => {
    const x = event.clientX - event.currentTarget.getBoundingClientRect().left;
    onClick({ channel, frame: Math.max(1, Math.floor(x / CELL_WIDTH) + 1) });
  };

  const selectedInThisChannel = selected?.channel === channel ? selected.frame : undefined;
  const emptySelected =
    selectedInThisChannel !== undefined &&
    !findSpanAtFrame(spans, selectedInThisChannel);

  return (
    <div className={styles.channelLane} onClick={handleLaneClick}>
      {sliceRangesInWindow(spans, firstFrame, lastFrame).map((span) => {
        const isSelected =
          selectedInThisChannel !== undefined &&
          selectedInThisChannel >= span.startFrame &&
          selectedInThisChannel <= span.endFrame;
        const memberRef = span.memberRef ? `${span.memberRef[0]}:${span.memberRef[1]}` : undefined;
        const label = span.memberName || memberRef;
        return (
          <div
            key={span.startFrame}
            className={classNames(
              styles.span,
              isSelected && styles.spanSelected,
              clickable && styles.spanClickable
            )}
            style={{
              left: (span.startFrame - 1) * CELL_WIDTH,
              width: (span.endFrame - span.startFrame + 1) * CELL_WIDTH,
            }}
            title={
              label
                // Unnamed members have nothing but the ref, so don't repeat it.
                ? `${span.memberName ? `${span.memberName} (${memberRef})` : `Member ${memberRef}`}` +
                  ` — frames ${span.startFrame}–${span.endFrame}`
                : undefined
            }
          >
            {label && <span className={styles.spanLabel}>{label}</span>}
          </div>
        );
      })}
      {emptySelected && (
        <div
          className={styles.emptySelection}
          style={{ left: (selectedInThisChannel - 1) * CELL_WIDTH, width: CELL_WIDTH }}
        />
      )}
    </div>
  );
});

type ScriptLaneProps = {
  lane: FrameScriptLane;
  behaviors: IScoreBehaviorReference[];
  firstFrame: number;
  lastFrame: number;
  height: number;
};

/**
 * Frame scripts, drawn the same way as sprite clips: one bar per behaviour
 * range rather than one box per frame. A script covering frames 8-12 is one
 * thing, and showing it as five identical squares said otherwise — and left
 * nowhere to name the script it points at.
 */
const ScriptLane = memo(function ScriptLane({
  lane,
  behaviors,
  firstFrame,
  lastFrame,
  height,
}: ScriptLaneProps) {
  const handleLaneClick = (event: React.MouseEvent<HTMLDivElement>) => {
    const x = event.clientX - event.currentTarget.getBoundingClientRect().left;
    lane.onSelect(Math.max(1, Math.floor(x / CELL_WIDTH) + 1));
  };

  const [selectedFrom, selectedTo] = lane.selectedRange ?? [];
  const visible = sliceRangesInWindow(behaviors, firstFrame, lastFrame);
  const selectionHitsABehavior =
    selectedFrom !== undefined && !!findRangeAtFrame(behaviors, selectedFrom);

  return (
    <div className={styles.scriptLane} style={{ height }} onClick={handleLaneClick}>
      {visible.map((behavior) => {
        const isSelected =
          selectedFrom !== undefined &&
          selectedTo !== undefined &&
          behavior.startFrame <= selectedTo &&
          behavior.endFrame >= selectedFrom;
        const memberRef = `${behavior.castLib}:${behavior.castMember}`;
        const label = behavior.memberName || memberRef;
        return (
          <div
            key={behavior.startFrame}
            className={classNames(styles.scriptBar, isSelected && styles.scriptBarSelected)}
            style={{
              left: (behavior.startFrame - 1) * CELL_WIDTH,
              width: (behavior.endFrame - behavior.startFrame + 1) * CELL_WIDTH,
            }}
            title={
              `Frame script ${behavior.memberName ? `${behavior.memberName} (${memberRef})` : memberRef}` +
              ` — frames ${behavior.startFrame}–${behavior.endFrame}`
            }
          >
            <span className={styles.spanLabel}>{label}</span>
          </div>
        );
      })}
      {selectedFrom !== undefined && !selectionHitsABehavior && (
        <div
          className={styles.emptySelection}
          style={{ left: (selectedFrom - 1) * CELL_WIDTH, width: CELL_WIDTH }}
        />
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
  // Which frames the lanes need to consider, from the ruler's own window.
  const firstVisibleFrame = (columns[0]?.index ?? 0) + 1;
  const lastVisibleFrame = (columns[columns.length - 1]?.index ?? frameCount - 1) + 1;

  return (
    <div
      ref={scrollRef}
      className={classNames(
        styles.scoreOverviewContainer,
        !showRows && styles.scoreOverviewCollapsed
      )}
      style={{
        // Collapsed, the element is sized to its header — plus whatever a
        // horizontal scrollbar occupies, or it would overflow into a vertical
        // one and hide the ruler.
        ...(showRows ? null : { height: headerHeight + getScrollbarSize() }),
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
            <ScriptLane
              lane={frameScripts}
              behaviors={frameScriptIndex}
              firstFrame={firstVisibleFrame}
              lastFrame={lastVisibleFrame}
              height={laneHeight}
            />
          )}

          {frameCount > 0 && <div className={styles.headerPlayhead} aria-hidden="true" />}

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
                <ChannelLane
                  channel={channel}
                  spans={index.spansByChannel.get(channel) ?? EMPTY_SPANS}
                  firstFrame={firstVisibleFrame}
                  lastFrame={lastVisibleFrame}
                  selected={selectedCell}
                  clickable={!!onCellClick}
                  onClick={handleCellClick}
                />
              </div>
            );
          })}
        </div>

        {frameCount > 0 && <div className={styles.playhead} aria-hidden="true" />}
      </div>
    </div>
  );
}
