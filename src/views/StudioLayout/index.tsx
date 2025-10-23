import { Layout, TabNode } from 'flexlayout-react';
import 'flexlayout-react/style/light.css';  
import Stage from '../Stage';
import ScoreInspector from '../ScoreInspector';
import PlaybackControls from '../../components/PlaybackControls';
import CastInspector from '../CastInspector';
import { useAppDispatch, useAppSelector, useMemberSubscriptions } from '../../store/hooks';
import { useEffect } from 'react';
import { onMemberSelected } from '../../store/uiSlice';
import { player_set_preview_member_ref } from 'vm-rust';
import { ICastMemberIdentifier } from '../../vm';
import { useSelectedObjects } from '../../hooks/selection';
import { selectScriptError } from '../../store/vmSlice';

import styles from "./styles.module.css";
import DebugInspector from '../DebugInspector';
import MemberInspector from '../MemberInspector';
import PropertyInspector from '../PropertyInspector';
import { studioLayoutModel } from './layout';

const StudioLayout = () => {
  const castSnapshots = useAppSelector((state) => state.vm.castSnapshots);
  const { memberRef: selectedMemberRef } = useSelectedObjects();
  const selectedMemberId: ICastMemberIdentifier | undefined =
    selectedMemberRef && {
      castNumber: selectedMemberRef[0],
      memberNumber: selectedMemberRef[1],
    };

  const dispatch = useAppDispatch();
  const setSelectedMemberId = (memberId: ICastMemberIdentifier) => {
    dispatch(onMemberSelected([memberId.castNumber, memberId.memberNumber]));
  };

  useEffect(() => {
    if (selectedMemberId) {
      player_set_preview_member_ref(selectedMemberId.castNumber, selectedMemberId.memberNumber);
    }
  }, [selectedMemberId]);

  useMemberSubscriptions();
  
  const castNames = useAppSelector((state) => state.vm.castNames);
  const scriptError = useAppSelector((state) => selectScriptError(state.vm));
  const selectedObject = useAppSelector((state) => state.ui.selectedObject);
  
  const factory = (node: TabNode) => {
    const component = node.getComponent();

    if (component === "placeholder") {
      return <div>{node.getName()}</div>;
    } else if (component === "stage") {
      return <Stage showControls />
    } else if (component === "score") {
      return <ScoreInspector />;
    } else if (component === "playback") {
      return <PlaybackControls />;
    } else if (component === "cast") {
      return <CastInspector
        castNames={castNames}
        castSnapshots={castSnapshots}
        selectedMemberId={selectedMemberId}
        onSelectMember={setSelectedMemberId}
        className={styles.castInspector}
      />;
    } else if (component === "debug") {
      return <div className={styles.debugContainer}>
        {scriptError && <p className={styles.scriptError}>{scriptError}</p>}
        <div className={styles.bottomContainer}>
          <DebugInspector />
        </div>
      </div>
    } else if (component === "member") {
      return selectedMemberId ? <MemberInspector memberId={selectedMemberId} /> : null;
    } else if (component === "properties") {
      return <PropertyInspector selectedObject={selectedObject} />;
    }
  }

  return (
    <Layout
      model={studioLayoutModel}
      factory={factory} 
    />
  );
}

export default StudioLayout;