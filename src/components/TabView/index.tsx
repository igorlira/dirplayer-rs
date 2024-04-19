import React from "react";
import { PropsWithChildren } from "react";
import styles from "./styles.module.css";
import classNames from "classnames";

type TabItemProps = PropsWithChildren<{
  title: string;
  tabKey: string;
}>;
function TabItem({ title, children }: TabItemProps) {
  return (
    <div className={styles.tabItemContainer}>
      <h2>{title}</h2>
      {children}
    </div>
  );
}

type TabViewChild = React.ReactElement<TabItemProps> | null | undefined | false;
type TabViewProps = PropsWithChildren<{
  children: TabViewChild[];
  className?: string;
}>;
export default function TabView({ children, className }: TabViewProps) {
  const [selectedTabKey, setSelectedTabKey] = React.useState(children[0] && children[0].props.tabKey);
  const selectedTab = children.find((child) => child && child.props.tabKey === selectedTabKey);
  const tabs = children.map((child) => child && child.props).filter(Boolean);

  return (
    <div className={classNames(className, styles.tabItemContainer)}>
      <div>
        {tabs.map((tab, i) => (
          <button
            key={tab ? tab.tabKey : i}
            onClick={() => setSelectedTabKey(tab && tab.tabKey)}
            className={styles.tabButton}
          >
            {tab && tab.title}
          </button>
        ))}
      </div>
      {selectedTab}
    </div>
  );
}

TabView.Tab = TabItem;

