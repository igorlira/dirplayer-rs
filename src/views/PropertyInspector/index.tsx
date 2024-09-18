import TabView from "../../components/TabView";
import { JSONTree } from "react-json-tree";
import styles from "./styles.module.css";
import { TSelectedObject } from "../../store/uiSlice";
import { useSelectedObjects } from "../../hooks/selection";
import { useAppSelector } from "../../store/hooks";
import { JsBridgeChunk } from "dirplayer-js-api";
import { ComponentProps } from "react";

interface PropertyInspectorProps {
  selectedObject?: TSelectedObject;
}

export default function PropertyInspector({
  selectedObject,
}: PropertyInspectorProps) {
  const { scoreBehaviorRef, selectedSprite, member } = useSelectedObjects();
  const movieChunks = useAppSelector((state) => state.vm.movieChunkList);
  const getChunkItemString: ComponentProps<typeof JSONTree>['getItemString'] = (type, data, itemType, itemString, keyPath) => {
    let chunk = data as JsBridgeChunk;
    return <span>{chunk.fourcc} ({chunk.id})</span>;
  }

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
        <TabView.Tab tabKey="movie" title="Movie">
          <JSONTree keyPath={["chunks"]} data={movieChunks} getItemString={getChunkItemString} />
        </TabView.Tab>
      </TabView>
    </div>
  );
}
