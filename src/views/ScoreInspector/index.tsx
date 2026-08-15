import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useAppDispatch, useAppSelector } from "../../store/hooks";
import { selectScoreSnapshot } from "../../store/vmSlice";
import styles from "./styles.module.css";
import classNames from "classnames";
import { player_set_debug_selected_channel, subscribe_to_channel_names, subscribe_to_score, unsubscribe_from_channel_names, unsubscribe_from_score } from "vm-rust";
import { channelSelected, scoreSpanSelected, scoreBehaviorSelected } from "../../store/uiSlice";
import { getScoreFrameBehaviorRef } from "../../utils/score";
import { getChannelCount, getFrameCount } from "../../utils/scoreIndex";
import { usePlayheadVar } from "../../utils/usePlayhead";
import { SCORE_CELL_WIDTH, SCORE_CHANNEL_ROW_HEIGHT, SCORE_LABEL_WIDTH } from "../../utils/scoreLayout";
import ExpandableButton from "../../components/ExpandableButton";
import ScoreTimeline from "../../components/ScoreTimeline";
import { ScoreSpriteSnapshot } from "../../vm";

// Shared with the timeline so frame columns line up between the two; see
// utils/scoreLayout.
const CELL_WIDTH = SCORE_CELL_WIDTH;
const LABEL_WIDTH = SCORE_LABEL_WIDTH;
const CHANNEL_ROW_HEIGHT = SCORE_CHANNEL_ROW_HEIGHT;

const ChannelRow = memo(function ChannelRow({
  channel,
  sprite,
  isSelected,
  top,
  onSelect,
}: {
  channel: number;
  sprite?: ScoreSpriteSnapshot;
  isSelected: boolean;
  top: number;
  onSelect: (channel: number) => void;
}) {
  return (
    <button
      className={classNames(
        styles.channelRow,
        channel % 2 === 0 && styles.channelRowAlt,
        isSelected && styles.selected
      )}
      style={{ top }}
      onClick={() => onSelect(channel)}
    >
      <span className={styles.channelNumber}>{channel}</span>
      {sprite?.displayName}
    </button>
  );
});

export default function ScoreInspector() {
  const score = useAppSelector((state) => selectScoreSnapshot(state.vm));
  const selectedObject = useAppSelector((state) => state.ui.selectedObject);
  const channelSnapshots = useAppSelector((state) => state.vm.channelSnapshots);
  const selectedChannel = selectedObject?.type === "sprite" && selectedObject.spriteNumber;
  const dispatch = useAppDispatch();
  const [isShowingChannels, setIsShowingChannels] = useState(false);
  const [isShowingscoreTimeline, setIsShowingscoreTimeline] = useState(false);

  // The whole score, not a hardcoded window of it.
  const frameCount = useMemo(() => getFrameCount(score), [score]);
  const channelCount = useMemo(() => getChannelCount(score), [score]);

  const rulerRef = useRef<HTMLDivElement>(null);
  const channelListRef = useRef<HTMLDivElement>(null);

  // Frame ticks move a CSS variable, never a React tree.
  usePlayheadVar(rulerRef);

  const frameVirtualizer = useVirtualizer({
    horizontal: true,
    count: frameCount,
    getScrollElement: () => rulerRef.current,
    estimateSize: () => CELL_WIDTH,
    overscan: 16,
    // Leaves room for the gutter, so frame N lands at the same x as frame N in
    // the timeline's grid below.
    paddingStart: LABEL_WIDTH,
  });

  const channelVirtualizer = useVirtualizer({
    count: isShowingChannels ? channelCount : 0,
    getScrollElement: () => channelListRef.current,
    estimateSize: () => CHANNEL_ROW_HEIGHT,
    overscan: 8,
  });

  // The score snapshot is only pushed while this inspector is mounted.
  useEffect(() => {
    subscribe_to_score();
    return () => unsubscribe_from_score();
  }, []);

  const shouldSubscribeToChannelNames = isShowingChannels || isShowingscoreTimeline;
  useEffect(() => {
    if (shouldSubscribeToChannelNames) {
      subscribe_to_channel_names();
    }
    return () => unsubscribe_from_channel_names();
  }, [shouldSubscribeToChannelNames]);

  const timelineScrollerRef = useRef<HTMLDivElement | null>(null);
  const isSyncingScroll = useRef(false);

  const syncScroll = useCallback((from: HTMLElement | null, to: HTMLElement | null) => {
    if (!from || !to || isSyncingScroll.current) return;
    if (to.scrollLeft === from.scrollLeft) return;
    // Assigning scrollLeft fires the other element's scroll handler, which
    // would bounce straight back here.
    isSyncingScroll.current = true;
    to.scrollLeft = from.scrollLeft;
    requestAnimationFrame(() => { isSyncingScroll.current = false; });
  }, []);

  const onRulerScroll = useCallback(() => {
    syncScroll(rulerRef.current, timelineScrollerRef.current);
  }, [syncScroll]);

  const onTimelineScrollLeft = useCallback(() => {
    syncScroll(timelineScrollerRef.current, rulerRef.current);
  }, [syncScroll]);

  const onTimelineScrollerRef = useCallback((element: HTMLDivElement | null) => {
    timelineScrollerRef.current = element;
    // Adopt whatever the ruler is already showing when the panel opens.
    if (element && rulerRef.current) element.scrollLeft = rulerRef.current.scrollLeft;
  }, []);

  const onSelectChannel = (channel: number) => {
    player_set_debug_selected_channel(channel);
    dispatch(channelSelected(channel));
  };

  const onSelectBehavior = (behavior: any) => {
    dispatch(scoreBehaviorSelected({ frameNumber: behavior }));
  };

  const onTimelineCellClick = ({ channel, frame }: { channel: number; frame: number }) => {
    dispatch(scoreSpanSelected({ channelNumber: channel, frameNumber: frame, scoreRef: 'stage' }));
  };

  // Which frames the selected behaviour spans, so the strip can shade them.
  const selectedRange = useMemo<[number, number] | undefined>(() => {
    if (selectedObject?.type !== "scoreBehavior") return undefined;
    const ref = score && getScoreFrameBehaviorRef(selectedObject.frameNumber, score);
    return ref ? [ref.startFrame, ref.endFrame] : [selectedObject.frameNumber, selectedObject.frameNumber];
  }, [score, selectedObject]);

  const frameColumns = frameVirtualizer.getVirtualItems();
  const rulerWidth = LABEL_WIDTH + frameCount * CELL_WIDTH;

  return (
    <div className={styles.container}>
      <div
        ref={rulerRef}
        className={styles.scoreScrollContainer}
        onScroll={onRulerScroll}
        style={{
          ['--frame-cell-width' as string]: `${CELL_WIDTH}px`,
          ['--channel-label-width' as string]: `${LABEL_WIDTH}px`,
        }}
      >
        <div className={styles.rulerInner} style={{ width: rulerWidth }}>
          {/* Mirrors the timeline's channel-number column so the two strips
              share an origin; sticky for the same reason that one is. */}
          <div className={styles.rulerGutter} />
          <div className={styles.scriptHeader}>
            {frameColumns.map((column) => {
              const frame = column.index + 1;
              const scriptRef = score && getScoreFrameBehaviorRef(frame, score);
              const isSelected = selectedRange && frame >= selectedRange[0] && frame <= selectedRange[1];
              return (
                <button
                  key={column.index}
                  className={classNames(
                    styles.scriptHeaderCell,
                    scriptRef && styles.scripted,
                    isSelected && styles.selected
                  )}
                  style={{ left: column.start }}
                  onClick={() => onSelectBehavior(frame)}
                />
              );
            })}
          </div>
          <div className={styles.frameHeader}>
            {frameColumns.map((column) => {
              const frame = column.index + 1;
              return (
                <div
                  key={column.index}
                  className={styles.frameHeaderCell}
                  style={{ left: column.start }}
                >
                  {frame === 1 || frame % 5 === 0 ? frame : "·"}
                </div>
              );
            })}
          </div>
          {frameCount > 0 && <div className={styles.rulerPlayhead} aria-hidden="true" />}
        </div>
      </div>

      <ExpandableButton label="Channels" className={styles.channelsButton} onStateChange={setIsShowingChannels}>
        <div ref={channelListRef} className={styles.channelList}>
          <div style={{ position: 'relative', height: channelVirtualizer.getTotalSize() }}>
            {channelVirtualizer.getVirtualItems().map((row) => {
              const channel = row.index + 1;
              return (
                <ChannelRow
                  key={row.index}
                  channel={channel}
                  sprite={channelSnapshots[channel]}
                  isSelected={selectedChannel === channel}
                  top={row.start}
                  onSelect={onSelectChannel}
                />
              );
            })}
          </div>
        </div>
      </ExpandableButton>

      <ExpandableButton label="Timeline" className={styles.scoreTimelineButton} onStateChange={setIsShowingscoreTimeline}>
        <div className={styles.timelineContainer}>
          {isShowingscoreTimeline && (
            <ScoreTimeline
              frameCount={frameCount}
              channelCount={channelCount}
              spriteSpans={score?.spriteSpans}
              channelSnapshots={channelSnapshots}
              selectedChannel={selectedChannel}
              onSelectChannel={onSelectChannel}
              onCellClick={onTimelineCellClick}
              onScrollerRef={onTimelineScrollerRef}
              onScrollLeftChange={onTimelineScrollLeft}
            />
          )}
        </div>
      </ExpandableButton>
    </div>
  );
}
