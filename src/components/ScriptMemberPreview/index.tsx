import classNames from "classnames";
import { ICastMemberIdentifier, ILingoLine, ILingoSpan, IScriptMemberSnapshot, LingoTokenType, MemberSnapshot } from "../../vm";
import styles from "./styles.module.css";
import { useAppSelector } from "../../store/hooks";
import { selectBreakpoints } from "../../store/vmSlice";
import { toggle_breakpoint } from "vm-rust";
import { useState } from "react";
import { ICastMemberRef } from "dirplayer-js-api";

// Map token types to CSS classes
const tokenTypeToClass: Record<LingoTokenType, string> = {
  keyword: styles.tokenKeyword,
  identifier: styles.tokenIdentifier,
  number: styles.tokenNumber,
  string: styles.tokenString,
  symbol: styles.tokenSymbol,
  operator: styles.tokenOperator,
  comment: styles.tokenComment,
  builtin: styles.tokenBuiltin,
  punctuation: styles.tokenPunctuation,
  whitespace: styles.tokenWhitespace,
};

function LingoSpan({ span }: { span: ILingoSpan }) {
  return (
    <span className={tokenTypeToClass[span.type] || ''}>
      {span.text}
    </span>
  );
}

interface IScriptMemberPreviewProps {
  memberId: ICastMemberIdentifier,
  snapshot: Extract<MemberSnapshot, IScriptMemberSnapshot>;
  highlightedBytecodeIndex?: number;
  highlightedHandlerName?: string;
  backgroundScopes: [string, number, ICastMemberRef][];
  theme?: 'light' | 'dark';
}

type ViewMode = 'assembly' | 'lingo';

type BytecodeLineProps = {
  lineNumber: number;
  isHighlighted: boolean;
  isInBackground: boolean;
  text: string;
  hasBreakpoint: boolean;
  onBreakpointClick: () => void;
};

function BytecodeLine({
  lineNumber,
  text,
  isHighlighted,
  onBreakpointClick,
  hasBreakpoint,
  isInBackground,
}: BytecodeLineProps) {
  return (
    <div
      className={classNames(
        styles.bytecodeLine,
        isHighlighted && styles.bytecodeLineHighlighted,
        isInBackground && styles.bytecodeLineBackground
      )}
    >
      <button
        className={classNames(
          styles.breakpointColumn,
          hasBreakpoint && styles.hasBreakpoint
        )}
        onClick={onBreakpointClick}
      />
      <span className={styles.lineNumber}>{lineNumber}</span>
      <span className={styles.codeContent}>
        <span className={styles.bytecodeText}>{text}</span>
      </span>
    </div>
  );
}

type LingoLineProps = {
  lineNumber: number;
  line: ILingoLine;
  isHighlighted: boolean;
  isInBackground: boolean;
  hasBreakpoint: boolean;
  canSetBreakpoint: boolean;
  onBreakpointClick: () => void;
};

function LingoLine({
  lineNumber,
  line,
  isHighlighted,
  isInBackground,
  hasBreakpoint,
  canSetBreakpoint,
  onBreakpointClick,
}: LingoLineProps) {
  return (
    <div
      className={classNames(
        styles.lingoLine,
        isHighlighted && styles.lingoLineHighlighted,
        isInBackground && styles.lingoLineBackground
      )}
    >
      <button
        className={classNames(
          styles.breakpointColumn,
          hasBreakpoint && styles.hasBreakpoint,
          !canSetBreakpoint && styles.breakpointDisabled
        )}
        onClick={onBreakpointClick}
        disabled={!canSetBreakpoint}
      />
      <span className={styles.lineNumber}>{lineNumber}</span>
      <span
        className={styles.lingoLineText}
        style={{ paddingLeft: `${8 + line.indent * 16}px` }}
      >
        {line.spans && line.spans.length > 0
          ? line.spans.map((span, i) => <LingoSpan key={i} span={span} />)
          : line.text}
      </span>
    </div>
  );
}

function ExpandIcon({ expanded }: { expanded: boolean }) {
  return (
    <span className={classNames(styles.expandIcon, expanded && styles.expandIconExpanded)}>
      ▶
    </span>
  );
}

export default function ScriptMemberPreview({
  memberId,
  snapshot,
  highlightedBytecodeIndex,
  highlightedHandlerName,
  backgroundScopes,
  theme = 'light',
}: IScriptMemberPreviewProps) {
  const breakpoints = useAppSelector((state) => selectBreakpoints(state.vm));
  const [expandedHandlerNames, setExpandedHandlerNames] = useState<string[]>(
    []
  );
  const [viewMode, setViewMode] = useState<ViewMode>('lingo');

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
    <div className={classNames(styles.scriptContainer, theme === 'light' && styles.light)}>
      <div className={styles.viewModeToggle}>
        <button
          className={classNames(
            styles.viewModeButton,
            viewMode === 'lingo' && styles.viewModeButtonActive
          )}
          onClick={() => setViewMode('lingo')}
        >
          Lingo
        </button>
        <button
          className={classNames(
            styles.viewModeButton,
            viewMode === 'assembly' && styles.viewModeButtonActive
          )}
          onClick={() => setViewMode('assembly')}
        >
          Assembly
        </button>
      </div>

      {snapshot.script.handlers.map((handler) => {
        const isExpanded = expandedHandlerNames.includes(handler.name);
        const isHandlerHighlighted = highlightedHandlerName === handler.name;
        const isHandlerInBg = backgroundScopes.some(
          ([name, _, scriptMemRef]) => name === handler.name && memberId.castNumber === scriptMemRef[0] && memberId.memberNumber === scriptMemRef[1]
        );

        // For Lingo view, find which line is highlighted based on bytecode index
        const highlightedLingoLine = (highlightedBytecodeIndex !== undefined && handler.bytecodeToLine)
          ? handler.bytecodeToLine[highlightedBytecodeIndex]
          : undefined;

        const argsStr = handler.args.length > 0 ? ` ${handler.args.join(", ")}` : '';

        return (
          <div key={handler.name} className={styles.handlerContainer}>
            <button
              className={classNames(
                styles.handlerHeader,
                isHandlerHighlighted && styles.handlerNameHighlighted,
                isHandlerInBg && styles.handlerNameBackground
              )}
              onClick={() => onToggleHandler(handler.name)}
            >
              <ExpandIcon expanded={isExpanded} />
              <span className={styles.handlerSignature}>
                <span className={styles.handlerKeyword}>on</span>
                {handler.name}
                {argsStr && <span className={styles.handlerArgs}>{argsStr}</span>}
              </span>
            </button>

            {isExpanded && (
              <div className={styles.codeBody}>
                {viewMode === 'assembly' &&
                  handler.bytecode.map((bytecode, i) => (
                    <BytecodeLine
                      key={bytecode.pos}
                      lineNumber={i + 1}
                      hasBreakpoint={breakpoints.some(
                        (bp) =>
                          bp.script_name === snapshot.name &&
                          bp.handler_name === handler.name &&
                          bp.bytecode_index === i
                      )}
                      text={bytecode.text}
                      isHighlighted={
                        isHandlerHighlighted && highlightedBytecodeIndex === i
                      }
                      isInBackground={backgroundScopes.some(([name, idx, scriptMemRef]) => name === handler.name && idx === i && memberId.castNumber === scriptMemRef[0] && memberId.memberNumber === scriptMemRef[1])}
                      onBreakpointClick={() =>
                        toggle_breakpoint(snapshot.name, handler.name, i)
                      }
                    />
                  ))}

                {viewMode === 'lingo' && handler.lingo &&
                  handler.lingo.map((line, lineIndex) => {
                    // Find if any bytecode in this line has a breakpoint, and which one
                    const breakpointBytecodeIndex = line.bytecodeIndices.find(bcIdx =>
                      breakpoints.some(
                        (bp) =>
                          bp.script_name === snapshot.name &&
                          bp.handler_name === handler.name &&
                          bp.bytecode_index === bcIdx
                      )
                    );
                    const hasBreakpoint = breakpointBytecodeIndex !== undefined;

                    // Check if this line is in background scope using bytecodeToLine for consistency
                    const isInBackground = !!(handler.bytecodeToLine && backgroundScopes.some(([name, idx, scriptMemRef]) =>
                      name === handler.name &&
                      handler.bytecodeToLine![idx] === lineIndex &&
                      memberId.castNumber === scriptMemRef[0] &&
                      memberId.memberNumber === scriptMemRef[1]
                    ));

                    // Use first bytecode index for setting breakpoint, but toggle existing one if present
                    const primaryBytecodeIndex = line.bytecodeIndices[0];
                    const canSetBreakpoint = primaryBytecodeIndex !== undefined;

                    return (
                      <LingoLine
                        key={lineIndex}
                        lineNumber={lineIndex + 1}
                        line={line}
                        isHighlighted={isHandlerHighlighted && highlightedLingoLine === lineIndex}
                        isInBackground={isInBackground}
                        hasBreakpoint={hasBreakpoint}
                        canSetBreakpoint={canSetBreakpoint}
                        onBreakpointClick={() => {
                          if (hasBreakpoint && breakpointBytecodeIndex !== undefined) {
                            // Toggle existing breakpoint (removes it)
                            toggle_breakpoint(snapshot.name, handler.name, breakpointBytecodeIndex);
                          } else if (canSetBreakpoint) {
                            // Add new breakpoint at first instruction
                            toggle_breakpoint(snapshot.name, handler.name, primaryBytecodeIndex);
                          }
                        }}
                      />
                    );
                  })}

                {viewMode === 'lingo' && !handler.lingo && (
                  <p className={styles.noLingo}>Lingo source not available</p>
                )}

                <div className={styles.endHandler}>
                  <span className={styles.handlerKeyword}>end</span>
                </div>
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}
