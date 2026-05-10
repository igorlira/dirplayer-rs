import styles from "./styles.module.css";
import { TSelectedObject } from "../../store/uiSlice";
import { useSelectedObjects } from "../../hooks/selection";
import { useCallback, useMemo } from "react";
import { Layout, Model, TabNode } from "flexlayout-react";
import PropertyTable from "../../components/PropertyTable";
import MovieChunksView from "./MovieChunksView";
import RenderingOptions from "./RenderingOptions";

interface PropertyInspectorProps {
  selectedObject?: TSelectedObject;
}

export default function PropertyInspector({
  selectedObject,
}: PropertyInspectorProps) {
  const { scoreBehaviorRef, selectedSprite, member, secondaryMember } = useSelectedObjects();

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
            },
            {
              type: "tab",
              name: "Options",
              component: "options"
            }
          ]
        }
      ]
    }
  }), [scoreBehaviorRef, selectedObject, member, secondaryMember]);

  const factory = useCallback((node: TabNode) => {
    switch (node.getComponent()) {
      case "scoreBehavior":
        return <PropertyTable data={scoreBehaviorRef as unknown as Record<string, unknown>} scrollable />;
      case "sprite":
        return <PropertyTable data={{ ...selectedSprite } as unknown as Record<string, unknown>} scrollable />;
      case "member":
        return <PropertyTable data={member as unknown as Record<string, unknown>} scrollable />;
      case "secondaryMember":
        return <PropertyTable data={secondaryMember as unknown as Record<string, unknown>} scrollable />;
      case "movie":
        return <MovieChunksView />;
      case "options":
        return <RenderingOptions />;
      default:
        return null;
    }
  }, [scoreBehaviorRef, selectedSprite, member, secondaryMember]);

  return <div className={styles.container}>
    <Layout model={model} factory={factory} />
  </div>;
}
