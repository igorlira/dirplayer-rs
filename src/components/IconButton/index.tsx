import { IconProp } from "@fortawesome/fontawesome-svg-core";
import { FontAwesomeIcon } from "@fortawesome/react-fontawesome";
import styles from './styles.module.css'

interface IIconButtonProps {
  icon: IconProp,
  onClick: () => void
}

export default function IconButton({ icon, onClick }: IIconButtonProps) {
  return <button className={styles.iconButton} onClick={onClick}>
    <FontAwesomeIcon icon={icon} />
  </button>
}
