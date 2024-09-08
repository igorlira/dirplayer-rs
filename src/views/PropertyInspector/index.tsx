import TabView from "../../components/TabView";
import { JSONTree } from "react-json-tree";
import styles from "./styles.module.css";
import { TSelectedObject } from "../../store/uiSlice";
import { useSelectedObjects } from "../../hooks/selection";

interface PropertyInspectorProps {
  selectedObject?: TSelectedObject;
}

export default function PropertyInspector({
  selectedObject,
}: PropertyInspectorProps) {
  const { scoreBehaviorRef, selectedSprite, member } = useSelectedObjects();

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
