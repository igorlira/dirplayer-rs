import classNames from "classnames"
import { PropsWithChildren, useEffect, useState } from "react"

import styles from './styles.module.css'

type ExpandableButtonProps = PropsWithChildren<{
  className?: string
  label: string
  onStateChange?: (expanded: boolean) => void
}>
export default function ExpandableButton({ children, className, label, onStateChange }: ExpandableButtonProps) {
  const [isExpanded, setIsExpanded] = useState(false)
  useEffect(() => onStateChange?.(isExpanded), [onStateChange, isExpanded]);
  return <div className={classNames(className, styles.container)}>
    <button onClick={() => setIsExpanded(!isExpanded)} className={styles.toggleButton}>[{isExpanded ? '-' : '+'}] {label}</button>
    {isExpanded && children}
  </div>
}
