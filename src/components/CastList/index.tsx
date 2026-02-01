import { useState, useMemo } from "react"
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
  forceExpanded?: boolean
  filterText?: string
}

function CastListItem({ number, name, members, selectedMemberId, onSelectMember, forceExpanded, filterText }: ICastListItemProps) {
  const [isExpanded, setExpanded] = useState(false);
  const castNumber = number;
  const showExpanded = forceExpanded || isExpanded;

  const filteredMembers = useMemo(() => {
    if (!filterText) return Object.entries(members);
    const lowerFilter = filterText.toLowerCase();
    return Object.entries(members).filter(([, member]) =>
      member.name.toLowerCase().includes(lowerFilter)
    );
  }, [members, filterText]);

  return <div className={styles.castItem} key={castNumber}>
    <button
      onClick={() => setExpanded(!isExpanded)}
      className={styles.castName}>
      {showExpanded ? "[-]" : "[+]"} {name} ({castNumber})
    </button>
    {showExpanded && <ul className={styles.castMemberList}>
      {filteredMembers.map(([memberNumberStr, member]) => {
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
  const [searchQuery, setSearchQuery] = useState("");
  const classes = classNames(styles.castList, className)
  const isSearching = searchQuery.trim().length > 0;

  return <div className={classes}>
    <div className={styles.searchContainer}>
      <input
        type="text"
        className={styles.searchInput}
        placeholder="Search cast members..."
        value={searchQuery}
        onChange={(e) => setSearchQuery(e.target.value)}
      />
      {isSearching && (
        <button className={styles.clearButton} onClick={() => setSearchQuery("")}>
          ×
        </button>
      )}
    </div>
    <ul className={styles.castListItems}>
      {castNames.map((castName, i) => {
        const castNumber = i + 1;
        const snapshot = castSnapshots[castNumber];
        const members = snapshot?.members || {};

        // When searching, check if this cast has any matching members
        const hasMatchingMembers = isSearching && Object.values(members).some(
          (member) => member.name.toLowerCase().includes(searchQuery.toLowerCase())
        );

        // Skip casts with no matching members when searching
        if (isSearching && !hasMatchingMembers) return null;

        return <CastListItem
          key={castNumber}
          number={castNumber}
          name={castName}
          members={members}
          selectedMemberId={selectedMemberId}
          onSelectMember={onSelectMember}
          forceExpanded={isSearching}
          filterText={isSearching ? searchQuery : undefined}
        />
      })}
    </ul>
  </div>
}