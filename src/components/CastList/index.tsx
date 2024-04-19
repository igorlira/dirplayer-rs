import { useState } from "react"
import { ICastMemberIdentifier, castMemberIdentifier, CastSnapshot, CastMemberRecord } from "../../vm"
import classNames from "classnames"
import styles from './styles.module.css'
import _ from "lodash"

interface ICastMemberListItemProps {
  number: number
  name: string
  isSelected: boolean
  onSelect: () => void
}

function CastMemberListItem({ number, name, isSelected, onSelect }: ICastMemberListItemProps) {
  const classes = classNames({ [styles.castMemberItem]: true, [styles.selected]: isSelected })
  return <button className={classes} onClick={onSelect}>
    <span className={styles.memberNumberLabel}>{number}</span>
    <span className={styles.memberNameLabel}>{name}</span>
  </button>
}

interface ICastListItemProps {
  number: number
  name: string
  members: Record<number, CastMemberRecord>,
  selectedMemberId?: ICastMemberIdentifier,
  onSelectMember: (memberId: ICastMemberIdentifier) => void
}

function CastListItem({ number, name, members, selectedMemberId, onSelectMember }: ICastListItemProps) {
  const [isExpanded, setExpanded] = useState(false);
  const castNumber = number;

  return <div className={styles.castItem} key={castNumber}>
    <button
      onClick={() => setExpanded(!isExpanded)}
      className={styles.castName}>
      {isExpanded ? "[-]" : "[+]"} {name} ({castNumber})
    </button>
    {isExpanded && <ul className={styles.castMemberList}>
      {Object.entries(members).map(([memberNumberStr, member]) => {
        const memberNumber = parseInt(memberNumberStr)
        const memberId = castMemberIdentifier(castNumber, memberNumber)
        const isSelected = selectedMemberId ? _.isEqual(selectedMemberId, memberId) : false

        return <CastMemberListItem 
          key={memberNumber} 
          number={memberNumber} 
          name={member.name} 
          isSelected={isSelected} 
          onSelect={() => onSelectMember(memberId)} />
      })}
    </ul>}
  </div>
}

interface ICastListProps {
  castNames: string[],
  castSnapshots: Record<number, CastSnapshot>,
  selectedMemberId?: ICastMemberIdentifier,
  onSelectMember: (memberId: ICastMemberIdentifier) => void
  className?: string,
}

export default function CastList({ castNames, castSnapshots, selectedMemberId, onSelectMember, className }: ICastListProps) {
  const classes = classNames(styles.castList, className)
  return <ul className={classes}>
    {castNames.map((castName, i) => {
      const castNumber = i + 1;
      const snapshot = castSnapshots[castNumber];
      return <CastListItem key={castNumber} number={castNumber} name={castName} members={snapshot?.members || []} selectedMemberId={selectedMemberId} onSelectMember={onSelectMember} />
    })}
  </ul>
}