import { IconProp } from "@fortawesome/fontawesome-svg-core";
import { FontAwesomeIcon } from "@fortawesome/react-fontawesome";
import styles from './styles.module.css'
import { PropsWithChildren, useMemo } from "react";
import { IconType } from "react-icons";

interface IIconButtonProps {
  onClick: () => void,
  disabled?: boolean,
  title?: string,
  active?: boolean,
}

interface IFontAwesomeIconButtonProps extends IIconButtonProps {
  icon: IconProp,
}
export default function FontAwesomeIconButton({ icon, onClick, disabled, title, active }: IFontAwesomeIconButtonProps) {
  return <BaseIconButton onClick={onClick} disabled={disabled} title={title} active={active}>
    <FontAwesomeIcon icon={icon} />
  </BaseIconButton>
}

export function ReactIconButton({ icon: IconComponent, onClick, disabled, title }: { icon: IconType, onClick: () => void, disabled?: boolean, title?: string }) {
  const icon = useMemo(() => IconComponent({}), [IconComponent]);
  return <BaseIconButton onClick={onClick} disabled={disabled} title={title}>
    {icon}
  </BaseIconButton>
}


function BaseIconButton({ onClick, disabled, title, active, children }: PropsWithChildren<IIconButtonProps>) {
  return <button className={active ? styles.iconButtonActive : styles.iconButton} onClick={onClick} disabled={disabled} title={title}>
    {children}
  </button>
}
