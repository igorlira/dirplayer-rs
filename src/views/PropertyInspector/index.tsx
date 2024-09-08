import TabView from "../../components/TabView";
import { useAppSelector } from "../../store/hooks";
import { CastSnapshot } from "../../vm";
import { JSONTree } from "react-json-tree";
import styles from "./styles.module.css";
import { TSelectedObject } from "../../store/uiSlice";
import { useMemo } from "react";
import { getScoreFrameBehaviorRef } from "../../utils/score";

interface PropertyInspectorProps {
  castSnapshots: Record<number, CastSnapshot>;
  selectedObject?: TSelectedObject;
}

export default function PropertyInspector({
  castSnapshots,
  selectedObject,
}: PropertyInspectorProps) {
  const selectedSpriteNumber =
    selectedObject?.type === "sprite" && selectedObject.spriteNumber;
  const selectedSprite = useAppSelector(
    (state) =>
      (!!selectedSpriteNumber &&
        state.vm.channelSnapshots[selectedSpriteNumber]) ||
      undefined
  );
  const scoreSnapshot = useAppSelector((state) => state.vm.scoreSnapshot);
  const scoreBehaviorRef = useMemo(() => {
    if (scoreSnapshot && selectedObject?.type === "scoreBehavior") {
      return getScoreFrameBehaviorRef(selectedObject.frameNumber, scoreSnapshot);
    }
  }, [scoreSnapshot, selectedObject]);

  const memberRef = useMemo(() => {
    if (selectedObject?.type === "member") {
      return selectedObject.memberRef;
    } else if (selectedObject?.type === "scoreBehavior" && scoreBehaviorRef) {
      return [scoreBehaviorRef.castLib, scoreBehaviorRef.castMember];
    } else if (selectedObject?.type === "sprite" && selectedSprite) {
      return selectedSprite.memberRef;
    }
  }, [selectedObject, scoreBehaviorRef, selectedSprite]);

  const member = useMemo(() => {
    if (memberRef) {
      return castSnapshots[memberRef[0]]?.members?.[memberRef[1]];
    }
  }, [memberRef, castSnapshots]);

  return (
    <div className={styles.container}>
      <TabView>
        {scoreBehaviorRef && (
          <TabView.Tab tabKey="scoreBehavior" title="Score Behavior">
            <JSONTree keyPath={["scoreBehavior"]} data={scoreBehaviorRef} />
          </TabView.Tab>
        )}
        {selectedObject?.type === "sprite" && (
          <TabView.Tab tabKey="sprite" title="Sprite">
            <JSONTree keyPath={["sprite"]} data={{ ...selectedSprite }} />
          </TabView.Tab>
        )}
        {member && (
          <TabView.Tab tabKey="member" title="Member">
            <JSONTree keyPath={["member"]} data={member} />
          </TabView.Tab>
        )}
      </TabView>
    </div>
  );
}
