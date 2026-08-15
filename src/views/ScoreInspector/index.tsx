import { memo, useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useAppDispatch, useAppSelector } from "../../store/hooks";
import { selectScoreSnapshot } from "../../store/vmSlice";
import styles from "./styles.module.css";
import classNames from "classnames";
import { player_set_debug_selected_channel, subscribe_to_channel_names, unsubscribe_from_channel_names } from "vm-rust";
import { channelSelected, scoreSpanSelected, scoreBehaviorSelected } from "../../store/uiSlice";
import { getScoreFrameBehaviorRef } from "../../utils/score";
import { getChannelCount, getFrameCount } from "../../utils/scoreIndex";
import { usePlayheadVar } from "../../utils/usePlayhead";
import ExpandableButton from "../../components/ExpandableButton";
import ScoreTimeline from "../../components/ScoreTimeline";
import { ScoreSpriteSnapshot } from "../../vm";

// Must match the geometry in styles.module.css.
const CELL_WIDTH = 16;
const CHANNEL_ROW_HEIGHT = 20;

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
  });

  const channelVirtualizer = useVirtualizer({
    count: isShowingChannels ? channelCount : 0,
    getScrollElement: () => channelListRef.current,
    estimateSize: () => CHANNEL_ROW_HEIGHT,
    overscan: 8,
  });

  const shouldSubscribeToChannelNames = isShowingChannels || isShowingscoreTimeline;
  useEffect(() => {
    if (shouldSubscribeToChannelNames) {
      subscribe_to_channel_names();
    }
    return () => unsubscribe_from_channel_names();
  }, [shouldSubscribeToChannelNames]);

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
  const rulerWidth = frameCount * CELL_WIDTH;

  return (
    <div className={styles.container}>
      <div
        ref={rulerRef}
        className={styles.scoreScrollContainer}
        style={{ ['--frame-cell-width' as string]: `${CELL_WIDTH}px` }}
      >
        <div className={styles.rulerInner} style={{ width: rulerWidth }}>
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
                  {frame === 1 || frame % 5 === 0 ? frame : "-"}
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
              channelInitData={score?.channelInitData}
              channelSnapshots={channelSnapshots}
              selectedChannel={selectedChannel}
              onSelectChannel={onSelectChannel}
              onCellClick={onTimelineCellClick}
            />
          )}
        </div>
      </ExpandableButton>
    </div>
  );
}
