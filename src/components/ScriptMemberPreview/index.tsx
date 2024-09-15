import classNames from "classnames";
import { ICastMemberIdentifier, IScriptMemberSnapshot, MemberSnapshot } from "../../vm";
import styles from "./styles.module.css";
import { useAppSelector } from "../../store/hooks";
import { selectBreakpoints } from "../../store/vmSlice";
import { toggle_breakpoint } from "vm-rust";
import { useState } from "react";
import { ICastMemberRef } from "dirplayer-js-api";

interface IScriptMemberPreviewProps {
  memberId: ICastMemberIdentifier,
  snapshot: Extract<MemberSnapshot, IScriptMemberSnapshot>;
  highlightedBytecodeIndex?: number;
  highlightedHandlerName?: string;
  backgroundScopes: [string, number, ICastMemberRef][];
}

type BytecodeLineProps = {
  isHighlighted: boolean;
  isInBackground: boolean;
  text: string;
  hasBreakpoint: boolean;
  onBreakpointClick: () => void;
};
function BytecodeLine({
  text,
  isHighlighted,
  onBreakpointClick,
  hasBreakpoint,
  isInBackground,
}: BytecodeLineProps) {
  return (
    <div className={styles.bytecodeLine}>
      <button
        className={classNames(
          styles.breakpointColumn,
          hasBreakpoint && styles.hasBreakpoint
        )}
        onClick={onBreakpointClick}
      ></button>
      <p
        className={classNames([
          styles.bytecodeLine,
          isHighlighted && styles.bytecodeLineHighlighted,
          isInBackground && styles.bytecodeLineBackground,
        ])}
      >
        {text}
      </p>
    </div>
  );
}

export default function ScriptMemberPreview({
  memberId,
  snapshot,
  highlightedBytecodeIndex,
  highlightedHandlerName,
  backgroundScopes,
}: IScriptMemberPreviewProps) {
  const breakpoints = useAppSelector((state) => selectBreakpoints(state.vm));
  const [expandedHandlerNames, setExpandedHandlerNames] = useState<string[]>(
    []
  );

  const onToggleHandler = (handlerName: string) => {
    if (expandedHandlerNames.includes(handlerName)) {
      setExpandedHandlerNames(
        expandedHandlerNames.filter((name) => name !== handlerName)
      );
    } else {
      setExpandedHandlerNames([...expandedHandlerNames, handlerName]);
    }
  };

  return (
    <div className={styles.scriptContainer}>
      {snapshot.script.handlers.map((handler) => {
        const isExpanded = expandedHandlerNames.includes(handler.name);
        const isHandlerHighlighted = highlightedHandlerName === handler.name;
        const isHandlerInBg = backgroundScopes.some(
          ([name, _, scriptMemRef]) => name === handler.name && memberId.castNumber === scriptMemRef[0] && memberId.memberNumber === scriptMemRef[1]
        );
        return (
          <div key={handler.name}>
            <button
              className={classNames(
                styles.handlerName,
                isHandlerHighlighted && styles.handlerNameHighlighted,
                isHandlerInBg && styles.handlerNameBackground
              )}
              onClick={() => onToggleHandler(handler.name)}
            >
              on {handler.name} {handler.args.join(", ")}
            </button>
            {isExpanded &&
              handler.bytecode.map((bytecode, i) => (
                <BytecodeLine
                  hasBreakpoint={breakpoints.some(
                    (bp) =>
                      bp.script_name === snapshot.name &&
                      bp.handler_name === handler.name &&
                      bp.bytecode_index === i
                  )}
                  text={bytecode.text}
                  key={bytecode.pos}
                  isHighlighted={
                    isHandlerHighlighted && highlightedBytecodeIndex === i
                  }
                  isInBackground={backgroundScopes.some(([name, idx, scriptMemRef]) => name === handler.name && idx === i && memberId.castNumber === scriptMemRef[0] && memberId.memberNumber === scriptMemRef[1])}
                  onBreakpointClick={() =>
                    toggle_breakpoint(snapshot.name, handler.name, i)
                  }
                />
              ))}
            {isExpanded && <p className={styles.handlerName}>end</p>}
            {isExpanded && <p className={styles.handlerName}>&nbsp;</p>}
          </div>
        );
      })}
    </div>
  );
}
