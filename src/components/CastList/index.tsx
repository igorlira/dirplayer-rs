import { useState, useMemo } from "react"
import {
  ICastMemberIdentifier, castMemberIdentifier, CastSnapshot, CastMemberRecord, IScriptSnapshot,
  IScriptMemberSnapshot, memberMatches
} from "../../vm"
import classNames from "classnames"
import styles from './styles.module.css'
import _ from "lodash"

function getMemberTypeIcon(memberType?: string, scriptType?: string): string | null {
  if (!memberType) return null;
  switch (memberType) {
    case 'bitmap': return '/icons/member-types/1-bitmap.png';
    case 'filmLoop': return '/icons/member-types/2-filmloop.png';
    case 'field': return '/icons/member-types/3-field.png';
    case 'palette': return '/icons/member-types/4-palette.png';
    case 'script':
      switch (scriptType) {
        case 'score': return '/icons/member-types/11-1-script.png';
        case 'movie': return '/icons/member-types/11-3-movie.png';
        case 'parent': return '/icons/member-types/11-parent.png';
        default: return '/icons/member-types/11-1-script.png';
      }
    case 'shape': return '/icons/member-types/8-shape.png';
    case 'text': return '/icons/member-types/text.png';
    case 'sound': return '/icons/member-types/audio.png';
    default: return null;
  }
}

interface ICastMemberListItemProps {
  number: number
  name: string
  memberType?: string
  scriptType?: string
  isSelected: boolean
  onSelect: () => void
}

function CastMemberListItem({ number, name, memberType, scriptType, isSelected, onSelect }: ICastMemberListItemProps) {
  const classes = classNames({ [styles.castMemberItem]: true, [styles.selected]: isSelected })
  const icon = getMemberTypeIcon(memberType, scriptType);
  return <button className={classes} onClick={onSelect}>
    <span className={styles.memberNumberLabel}>{number}</span>
    <span className={styles.memberTypeIcon}>
      {icon && <img src={icon} alt="" />}
    </span>
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
    return Object.entries(members).filter(([, member]) =>
        memberMatches(member, filterText)
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
          memberType={member.type}
          scriptType={member.scriptType}
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
            (member) => memberMatches(member, searchQuery)
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