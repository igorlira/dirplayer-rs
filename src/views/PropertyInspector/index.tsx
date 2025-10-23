import { JSONTree } from "react-json-tree";
import styles from "./styles.module.css";
import { TSelectedObject } from "../../store/uiSlice";
import { useSelectedObjects } from "../../hooks/selection";
import { useAppSelector } from "../../store/hooks";
import { JsBridgeChunk } from "dirplayer-js-api";
import { ComponentProps, useCallback, useMemo } from "react";
import { Layout, Model, TabNode } from "flexlayout-react";

interface PropertyInspectorProps {
  selectedObject?: TSelectedObject;
}

export default function PropertyInspector({
  selectedObject,
}: PropertyInspectorProps) {
  const { scoreBehaviorRef, selectedSprite, member, secondaryMember } = useSelectedObjects();
  const movieChunks = useAppSelector((state) => state.vm.movieChunkList);
  const getChunkItemString = useCallback<NonNullable<ComponentProps<typeof JSONTree>['getItemString']>>((type, data, itemType, itemString, keyPath) => {
    let chunk = data as JsBridgeChunk;
    return <span>{chunk.fourcc}</span>;
  }, []);
  const mappedChunks = Object.entries(movieChunks).reduce((result, [key, value]) => {
    return {
      ...result,
      [key]: {
        ...value,
        content: {
          export: '<saveChunkContent>'
        },
      }
    }
  }, {})
  const chunkValueRenderer = useCallback<NonNullable<ComponentProps<typeof JSONTree>['valueRenderer']>>((strValue, value, ...keyPath) => {
    if (value === '<saveChunkContent>') {
      return <a href="#">(Save to file)</a>;
    } else {
      return <span>{strValue as string}</span>;
    }
  }, []);

  const model = useMemo(() => Model.fromJson({
    global: {
      tabEnableClose: false,
    },
    layout: {
      type: "row",
      children: [
        {
          type: "tabset",
          children: [
            ...(scoreBehaviorRef ? [{
              type: "tab",
              name: "Score Behavior",
              component: "scoreBehavior"
            }] : []),
            ...(selectedObject?.type === "sprite" ? [{
              type: "tab",
              name: "Sprite",
              component: "sprite"
            }] : []),
            ...(member ? [{
              type: "tab",
              name: "Member",
              component: "member"
            }] : []),
            ...(secondaryMember ? [{
              type: "tab",
              name: "Secondary Member",
              component: "secondaryMember"
            }] : []),
            {
              type: "tab",
              name: "Movie",
              component: "movie"
            }
          ]
        }
      ]
    }
  }), [scoreBehaviorRef, selectedObject, member, secondaryMember]);

  const factory = useCallback((node: TabNode) => {
    switch (node.getComponent()) {
      case "scoreBehavior":
        return <JSONTree keyPath={["scoreBehavior"]} data={scoreBehaviorRef} />;
      case "sprite":
        return <JSONTree keyPath={["sprite"]} data={{ ...selectedSprite }} />;
      case "member":
        return <JSONTree keyPath={["member"]} data={member} />;
      case "secondaryMember":
        return <JSONTree keyPath={["secondaryMember"]} data={secondaryMember} />;
      case "movie":
        return (
          <JSONTree
            keyPath={["chunks"]}
            data={mappedChunks}
            getItemString={getChunkItemString}
            valueRenderer={chunkValueRenderer}
          />
        );
      default:
        return null;
    }
  }, [scoreBehaviorRef, selectedSprite, member, secondaryMember, mappedChunks, getChunkItemString, chunkValueRenderer]);

  return <div className={styles.container}>
    <Layout model={model} factory={factory} />
  </div>;
}
