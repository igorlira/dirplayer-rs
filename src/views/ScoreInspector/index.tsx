import { range } from "lodash";
import { useAppDispatch, useAppSelector } from "../../store/hooks";
import { selectCurrentFrame, selectScoreSnapshot } from "../../store/vmSlice";
import styles from "./styles.module.css";
import classNames from "classnames";
import { player_set_debug_selected_channel, subscribe_to_channel_names, unsubscribe_from_channel_names } from "vm-rust";
import { channelSelected, scoreBehaviorSelected } from "../../store/uiSlice";
import { useEffect, useState } from "react";
import { getScoreFrameBehaviorRef } from "../../utils/score";
import ExpandableButton from "../../components/ExpandableButton";

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

  const getSpansForChannel = (channel: number) => {
    return score?.spriteSpans?.filter((span) => span.channelNumber === channel) || [];
  };

  const getCastMemberForChannel = (channel: number, frame: number) => {
    const initData = score?.channelInitData?.find(
      (data) => data.channelNumber === channel && data.frameIndex === frame
    );
    if (initData) {
      return `${initData.initData.castLib}:${initData.initData.castMember}`;
    }
    return null;
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
        <div className={styles.scoreTimelineContainer}>
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
            {Array.from({ length: score?.channelCount || 0 }, (_, i) => i + 1).map((channel) => {
              const spans = getSpansForChannel(channel);
              const sprite = channelSnapshots[channel];
              return (
                <div key={channel} className={styles.scoreGridRow}>
                  <div
                    className={classNames(
                      styles.channelLabelCell,
                      selectedChannel === channel && styles.selected
                    )}
                    onClick={() => onSelectChannel(channel)}
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

                    return (
                      <div
                        key={frame}
                        className={classNames(
                          styles.scoreGridCell,
                          span && styles.hasSprite,
                          isSpanStart && styles.spanStart,
                          currentFrame === frame && styles.currentFrame
                        )}
                        title={castMember || undefined}
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
      </ExpandableButton>
    </div>
  );
}
