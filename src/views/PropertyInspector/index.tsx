import TabView from "../../components/TabView";
import { useAppSelector } from "../../store/hooks";
import { CastSnapshot } from "../../vm";
import { JSONTree } from "react-json-tree";
import styles from "./styles.module.css";
import { TSelectedObject } from "../../store/uiSlice";

interface PropertyInspectorProps {
  castSnapshots: Record<number, CastSnapshot>;
  selectedObject?: TSelectedObject;
}

export default function PropertyInspector({
  castSnapshots,
  selectedObject,
}: PropertyInspectorProps) {
  const selectedMemberId =
    selectedObject?.type === "member" && selectedObject.memberRef;
  const selectedMember =
    selectedMemberId &&
    castSnapshots[selectedMemberId[0]]?.members?.[selectedMemberId[1]];
  const selectedSpriteNumber =
    selectedObject?.type === "sprite" && selectedObject.spriteNumber;
  const selectedSprite = useAppSelector(
    (state) =>
      (!!selectedSpriteNumber &&
        state.vm.channelSnapshots[selectedSpriteNumber]) ||
      undefined
  );
  const selectedSpriteMember =
    selectedSprite?.memberRef &&
    castSnapshots[selectedSprite.memberRef[0]]?.members?.[
      selectedSprite.memberRef[1]
    ];

  return (
    <div className={styles.container}>
      <TabView>
        {selectedObject?.type === "sprite" && (
          <TabView.Tab tabKey="sprite" title="Sprite">
            <JSONTree keyPath={["sprite"]} data={{ ...selectedSprite }} />
          </TabView.Tab>
        )}
        {selectedObject?.type === "sprite" && selectedSpriteMember && (
          <TabView.Tab tabKey="member" title="Member">
            <JSONTree keyPath={["member"]} data={selectedSpriteMember} />
          </TabView.Tab>
        )}
        {selectedMember && (
          <TabView.Tab tabKey="member" title="Member">
            <JSONTree keyPath={["member"]} data={selectedMember} />
          </TabView.Tab>
        )}
      </TabView>
    </div>
  );
}
