import { useAppDispatch, useAppSelector, useMemberSubscriptions } from "../../store/hooks";
import CastInspector from "../CastInspector";
import styles from "./styles.module.css";
import { ICastMemberIdentifier } from "../../vm";
import MemberInspector from "../MemberInspector";
import ScoreInspector from "../ScoreInspector";
import PlaybackControls from "../../components/PlaybackControls";
import DebugInspector from "../DebugInspector";
import { selectScriptError } from "../../store/vmSlice";
import { onMemberSelected, selectSelectedMemberRef } from "../../store/uiSlice";
import Stage from "../Stage";
import { player_set_preview_member_ref } from "vm-rust";
import PropertyInspector from "../PropertyInspector";
import LoadMovie from "../LoadMovie";
import { useSelector } from "react-redux";
import { RootState } from "../../store";
import { useSelectedObjects } from "../../hooks/selection";

interface DirStudioProps {
  showDebugUi?: boolean;
}

export default function DirStudio({
  showDebugUi,
}: DirStudioProps) {
  const castSnapshots = useAppSelector((state) => state.vm.castSnapshots);
  const { memberRef: selectedMemberRef } = useSelectedObjects();
  const selectedMemberId: ICastMemberIdentifier | undefined =
    selectedMemberRef && {
      castNumber: selectedMemberRef[0],
      memberNumber: selectedMemberRef[1],
    };

  const dispatch = useAppDispatch();
  const setSelectedMemberId = (memberId: ICastMemberIdentifier) => {
    player_set_preview_member_ref(memberId.castNumber, memberId.memberNumber);
    dispatch(onMemberSelected([memberId.castNumber, memberId.memberNumber]));
  };

  const isMovieLoaded = useSelector<RootState>((state) => state.vm.isMovieLoaded);

  useMemberSubscriptions();

  const castNames = useAppSelector((state) => state.vm.castNames);
  const scriptError = useAppSelector((state) => selectScriptError(state.vm));
  const selectedObject = useAppSelector((state) => state.ui.selectedObject);

  if (!isMovieLoaded) {
    return <LoadMovie />;
  }
  if (!showDebugUi) {
    return <div style={{ width: '100vw', height: '100vh' }}>
      <Stage />
    </div>
  }
  return (
    <div className={styles.container}>
      <div className={styles.leftContainer}>
        <PlaybackControls />
        <ScoreInspector />
        <CastInspector
          castNames={castNames}
          castSnapshots={castSnapshots}
          selectedMemberId={selectedMemberId}
          onSelectMember={setSelectedMemberId}
          className={styles.castInspector}
        />
      </div>
      <div className={styles.centerContainer}>
        <div className={styles.stageContainer}>
          <Stage />
        </div>
        <div className={styles.bottomWrapper}>
          {scriptError && <p className={styles.scriptError}>{scriptError}</p>}
          <div className={styles.bottomContainer}>
            <DebugInspector />
            {selectedMemberId && (
              <MemberInspector memberId={selectedMemberId} />
            )}
          </div>
        </div>
      </div>
      <div className={styles.rightContainer}>
        <PropertyInspector
          selectedObject={selectedObject}
        />
      </div>
    </div>
  );
}
