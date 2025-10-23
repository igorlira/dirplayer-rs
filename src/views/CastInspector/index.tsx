import CastList from '../../components/CastList';
import { CastSnapshot, ICastMemberIdentifier } from '../../vm';
import styles from './styles.module.css';

interface CastInspectorProps {
  castNames: string[],
  castSnapshots: Record<number, CastSnapshot>,
  selectedMemberId?: ICastMemberIdentifier,
  onSelectMember: (id: ICastMemberIdentifier) => void,
  className?: string,
}

export default function CastInspector({ castNames, castSnapshots, selectedMemberId, onSelectMember, className }: CastInspectorProps) {
  return <CastList 
    castNames={castNames} 
    castSnapshots={castSnapshots} 
    selectedMemberId={selectedMemberId} 
    onSelectMember={onSelectMember}
    className={styles.castList}
  />;
}
