import CastList from '../../components/CastList';
import { CastSnapshot, ICastMemberIdentifier } from '../../vm';
import styles from './styles.module.css';
import classNames from 'classnames';
import ExpandableButton from '../../components/ExpandableButton';

interface CastInspectorProps {
  castNames: string[],
  castSnapshots: Record<number, CastSnapshot>,
  selectedMemberId?: ICastMemberIdentifier,
  onSelectMember: (id: ICastMemberIdentifier) => void,
  className?: string,
}

export default function CastInspector({ castNames, castSnapshots, selectedMemberId, onSelectMember, className }: CastInspectorProps) {
  return <ExpandableButton className={classNames(className, styles.container)} label='Casts'>
    <CastList 
      castNames={castNames} 
      castSnapshots={castSnapshots} 
      selectedMemberId={selectedMemberId} 
      onSelectMember={onSelectMember}
      className={styles.castList}
    />
  </ExpandableButton>
}
