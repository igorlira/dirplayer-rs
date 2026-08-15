import { memo, useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useAppDispatch, useAppSelector } from "../../store/hooks";
import { selectScoreSnapshot } from "../../store/vmSlice";
import styles from "./styles.module.css";
import classNames from "classnames";
import { player_set_debug_selected_channel, subscribe_to_channel_names, subscribe_to_score, unsubscribe_from_channel_names, unsubscribe_from_score } from "vm-rust";
import { channelSelected, scoreSpanSelected, scoreBehaviorSelected } from "../../store/uiSlice";
import { getScoreFrameBehaviorRef } from "../../utils/score";
import { getChannelCount, getFrameCount } from "../../utils/scoreIndex";
import { SCORE_CHANNEL_ROW_HEIGHT } from "../../utils/scoreLayout";
import ExpandableButton from "../../components/ExpandableButton";
import ScoreTimeline from "../../components/ScoreTimeline";
import { ScoreSpriteSnapshot } from "../../vm";

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

  const channelListRef = useRef<HTMLDivElement>(null);

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

  return (
    <div className={styles.container}>
      {/* One block: the frame-script lane and ruler live in the timeline's own
          sticky header, so there is a single ruler and a single scroller. The
          toggle collapses the channel rows and leaves the header in place. */}
      <div className={styles.timelineBlock}>
        <button
          className={styles.sectionToggle}
          onClick={() => setIsShowingscoreTimeline((shown) => !shown)}
        >
          [{isShowingscoreTimeline ? '-' : '+'}] Timeline
        </button>
        <div
          className={classNames(
            styles.timelineContainer,
            !isShowingscoreTimeline && styles.timelineContainerCollapsed
          )}
        >
          <ScoreTimeline
            frameCount={frameCount}
            channelCount={channelCount}
            spriteSpans={score?.spriteSpans}
            channelSnapshots={channelSnapshots}
            selectedChannel={selectedChannel}
            onSelectChannel={onSelectChannel}
            onCellClick={onTimelineCellClick}
            showRows={isShowingscoreTimeline}
            frameScripts={{
              behaviorReferences: score?.behaviorReferences,
              selectedRange,
              onSelect: onSelectBehavior,
            }}
          />
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
    </div>
  );
}
