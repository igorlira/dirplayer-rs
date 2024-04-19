import { useEffect, useRef, useState } from "react";
import { load_movie_file, set_base_path } from "vm-rust";
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
import { player_set_preview_member_ref, play } from "vm-rust";
import PropertyInspector from "../PropertyInspector";

interface DirStudioProps {
  initialMovieFileName?: string;
  showDebugUi?: boolean;
  autoPlay?: boolean;
}

function useMountEffect(effect: () => void) {
  const isMounted = useRef(false);
  useEffect(() => {
    if (!isMounted.current) {
      effect();
      isMounted.current = true;
    }
  }, []);
}

export default function DirStudio({
  initialMovieFileName,
  showDebugUi,
  autoPlay,
}: DirStudioProps) {
  const castSnapshots = useAppSelector((state) => state.vm.castSnapshots);
  const selectedMemberRef = useAppSelector((state) =>
    selectSelectedMemberRef(state.ui)
  );
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

  async function loadInitialMovie() {
    const initialMovieFileName = "dcr/habbo.dcr";
    if (initialMovieFileName) {
      console.log("Loading movie", initialMovieFileName);
      const dir = await load_movie_file(initialMovieFileName);
      if (autoPlay) {
        play();
      }
      console.log("Loaded movie", dir);
    }
  }

  useMountEffect(() => {
    const pathComponents = window.location.pathname.split("/");
    if (pathComponents.length > 0) {
      pathComponents.pop();
    }

    const basePath = window.location.origin + pathComponents.join("/");
    set_base_path(basePath);
    loadInitialMovie();
  });

  useMemberSubscriptions();

  const castNames = useAppSelector((state) => state.vm.castNames);
  const scriptError = useAppSelector((state) => selectScriptError(state.vm));
  const selectedObject = useAppSelector((state) => state.ui.selectedObject);

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
          castSnapshots={castSnapshots}
        />
      </div>
    </div>
  );
}
