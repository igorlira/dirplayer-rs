import { useDispatch } from "react-redux";
import PreviewCanvas from "../../components/PreviewCanvas";
import ScoreTimeline from "../../components/ScoreTimeline";
import { useMemberSnapshot } from "../../store/hooks";
import { scoreSpanSelected } from "../../store/uiSlice";
import { ICastMemberIdentifier } from "../../vm";
import styles from "./styles.module.css";
import { Model } from "flexlayout-react";
import { Layout, TabNode } from "flexlayout-react";

interface IMemberInspectorProps {
  memberId: ICastMemberIdentifier;
}

const layoutModel = Model.fromJson({
  global: {
    rootOrientationVertical: true,
    tabEnableClose: false,
  },
  layout: {
    type: "row",
    children: [
      {
        type: "tabset",
        weight: 30,
        children: [
          {
            type: "tab",
            name: "Timeline",
            component: "timeline",
          }
        ]
      },
      {
        type: "tabset",
        weight: 70,
        children: [
          {
            type: "tab",
            name: "Preview",
            component: "preview",
          },
          {
            type: "tab",
            name: "Details",
            component: "details",
          }
        ]
      }
    ]
  }
})

export default function FilmLoopInspector({ memberId }: IMemberInspectorProps) {
  const memberSnapshot = useMemberSnapshot(memberId);
  const dispatch = useDispatch();

  if (memberSnapshot?.type !== "filmLoop") {
    return null;
  }

  const factory = (node: TabNode) => {
    if (node.getComponent() === 'preview') {
      return <PreviewCanvas />;
    } else if (node.getComponent() === 'timeline') {
      return memberSnapshot.score && <div className={styles.filmLoopTimeline}>
        <ScoreTimeline
          framesToRender={Math.min(memberSnapshot.score.spriteSpans?.reduce((max, span) => Math.max(max, span.endFrame), 0) || 30, 100)}
          channelCount={memberSnapshot.score.channelCount}
          spriteSpans={memberSnapshot.score.spriteSpans}
          channelInitData={memberSnapshot.score.channelInitData}
          onCellClick={(cell) => {
            dispatch(scoreSpanSelected({
              channelNumber: cell.channel,
              frameNumber: cell.frame,
              scoreRef: [memberId.castNumber, memberId.memberNumber]
            }))
          }}
        />
      </div>;
    } else if (node.getComponent() === 'details') {
      return <div>
        <p>{memberSnapshot.width}x{memberSnapshot.height}</p>
        <p>Reg point: {memberSnapshot.regX}x{memberSnapshot.regY}</p>
      </div>;
    }
  }

  return <Layout model={layoutModel} factory={factory} />
}
