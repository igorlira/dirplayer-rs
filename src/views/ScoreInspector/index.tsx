import { range } from "lodash";
import { useAppDispatch, useAppSelector } from "../../store/hooks";
import { selectCurrentFrame, selectScoreSnapshot } from "../../store/vmSlice";
import styles from "./styles.module.css";
import classNames from "classnames";
import { player_set_debug_selected_channel, subscribe_to_channel_names, unsubscribe_from_channel_names } from "vm-rust";
import { channelSelected, scoreSpanSelected, scoreBehaviorSelected } from "../../store/uiSlice";
import { useEffect, useState } from "react";
import { getScoreFrameBehaviorRef } from "../../utils/score";
import ExpandableButton from "../../components/ExpandableButton";
import ScoreTimeline from "../../components/ScoreTimeline";

export default function ScoreInspector() {
  const score = useAppSelector((state) => selectScoreSnapshot(state.vm));
  const framesToRender = 30;
  const currentFrame = useAppSelector((state) => selectCurrentFrame(state.vm));
  const selectedObject = useAppSelector((state) => state.ui.selectedObject);
  const channelSnapshots = useAppSelector((state) => state.vm.channelSnapshots);
  const selectedChannel = selectedObject?.type === "sprite" && selectedObject.spriteNumber;
  const dispatch = useAppDispatch();
  const [isShowingChannels, setIsShowingChannels] = useState(false);
  const [isShowingscoreTimeline, setIsShowingscoreTimeline] = useState(false);

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

  return (
    <div className={styles.container}>
      <div className={styles.scoreScrollContainer}>
        <div className={styles.scriptHeader}>
          {range(1, framesToRender + 1).map((frame) => {
            const scriptRef = score && getScoreFrameBehaviorRef(frame, score);
            const selectedScriptRef = score && selectedObject?.type === "scoreBehavior" && getScoreFrameBehaviorRef(selectedObject.frameNumber, score);
            let selectedRange = undefined;
            if (selectedObject?.type === "scoreBehavior" && selectedScriptRef) {
              selectedRange = [selectedScriptRef.startFrame, selectedScriptRef.endFrame];
            } else if (selectedObject?.type === "scoreBehavior") {
              selectedRange = [selectedObject.frameNumber, selectedObject.frameNumber];
            }
            const isSelected = selectedRange && frame >= selectedRange[0] && frame <= selectedRange[1];
            const cellClasses = classNames(
              styles.scriptHeaderCell,
              scriptRef && styles.scripted,
              isSelected && styles.selected
            );
            return <button key={frame} className={cellClasses} onClick={() => onSelectBehavior(frame)}></button>;
          })}
        </div>
        <div className={styles.frameHeader}>
          {range(1, framesToRender + 1).map((frame) => {
            const cellClasses = classNames(
              styles.frameHeaderCell,
              currentFrame === frame && styles.current
            );
            return (
              <div key={frame} className={cellClasses}>
                {(frame === 1 || frame % 5 === 0) ? frame : "-"}
              </div>
            );
          })}
        </div>
      </div>
      <ExpandableButton label="Channels" className={styles.channelsButton} onStateChange={setIsShowingChannels}>
        <div className={styles.channelList}>
          {Array.from({ length: score?.channelCount || 0 }, (_, i) => i + 1).map(
            (channel) => {
              let sprite = channelSnapshots[channel];
              return (
                <button
                  key={channel}
                  className={classNames([
                    styles.channelRow,
                    selectedChannel === channel && styles.selected,
                  ])}
                  onClick={() => onSelectChannel(channel)}
                >
                  ({channel}) {sprite?.displayName}
                </button>
              );
            }
          )}
        </div>
      </ExpandableButton>
      <ExpandableButton label="Timeline" className={styles.scoreTimelineButton} onStateChange={setIsShowingscoreTimeline}>
        <div className={styles.timelineContainer}>
          <ScoreTimeline
            framesToRender={framesToRender}
            currentFrame={currentFrame}
            channelCount={score?.channelCount || 0}
            spriteSpans={score?.spriteSpans}
            channelInitData={score?.channelInitData}
            channelSnapshots={channelSnapshots}
            selectedChannel={selectedChannel}
            onSelectChannel={onSelectChannel}
            onCellClick={onTimelineCellClick}
          />
        </div>
      </ExpandableButton>
    </div>
  );
}
