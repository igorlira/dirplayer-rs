import classNames from "classnames";
import styles from "./styles.module.css";
import { PropsWithChildren } from "react";
import React from "react";

type ListViewItemProps = PropsWithChildren<{
  className?: string;
  style?: React.CSSProperties;
  isSelected?: boolean;
  onClick?: () => void;
}>;
function ListViewItem({ isSelected, style, children, onClick, className }: ListViewItemProps) {
  return <li
    style={style}
    className={classNames(
      styles.listItem,
      isSelected && styles.selectedListItem,
      className
    )}
  >
    <button onClick={onClick}>
      {children}
    </button>
  </li>;
}

type ListViewItemElement = React.ReactElement<ListViewItemProps> | null;
type ListViewProps = {
  selectedKey?: string;
  children: ListViewItemElement[] | ListViewItemElement | null | undefined;
  className?: string;
};
export default function ListView({ children, selectedKey, className }: ListViewProps) {
  return (
    <div className={classNames(styles.stackContainer, className)}>
      <ul>
        {React.Children.map(children, (child) => {
          return child && React.cloneElement(child, {
            isSelected: child.props.isSelected === undefined ? selectedKey === child.key : child.props.isSelected,
          })
        })}
      </ul>
    </div>
  );
}

ListView.Item = ListViewItem;

