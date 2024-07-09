import { useState } from 'react';
import CastList from '../../components/CastList';
import { CastSnapshot, ICastMemberIdentifier } from '../../vm';
import styles from './styles.module.css';
import classNames from 'classnames';

interface CastInspectorProps {
  castNames: string[],
  castSnapshots: Record<number, CastSnapshot>,
  selectedMemberId?: ICastMemberIdentifier,
  onSelectMember: (id: ICastMemberIdentifier) => void,
  className?: string,
}

export default function CastInspector({ castNames, castSnapshots, selectedMemberId, onSelectMember, className }: CastInspectorProps) {
  const [isExpanded, setIsExpanded] = useState(false)
  return <div className={classNames(className, styles.container)}>
    <button onClick={() => setIsExpanded(!isExpanded)} className={styles.toggleButton}>[{isExpanded ? '-' : '+'}] Casts</button>
    {isExpanded && <CastList 
      castNames={castNames} 
      castSnapshots={castSnapshots} 
      selectedMemberId={selectedMemberId} 
      onSelectMember={onSelectMember}
      className={styles.castList}
    />}
  </div>
}
