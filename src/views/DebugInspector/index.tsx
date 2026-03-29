import { useEffect, useState } from "react";
import { useAppDispatch, useAppSelector } from "../../store/hooks";
import { selectGlobals, selectScopes, scopeListChanged } from "../../store/vmSlice";
import styles from "./styles.module.css";
import IconButton, { ReactIconButton } from "../../components/IconButton";
import { faWarning } from "@fortawesome/free-solid-svg-icons";
import { downloadBlob } from "../../utils/download";
import {
  resume_breakpoint,
  request_datum,
  request_script_instance_snapshot,
  trigger_alert_hook,
  step_into,
  step_over,
  step_out,
  step_over_line,
  step_into_line,
  set_eval_scope_index,
} from "vm-rust";
import ListView from "../../components/ListView";
import { onMemberSelected, selectScriptViewMode } from "../../store/uiSlice";
import { DatumRef, IScriptMemberSnapshot, ScriptInstanceId } from "../../vm";
import { Layout, TabNode } from "flexlayout-react";
import { debugLayoutModel } from "./layout";
import { VscDebugStepOver, VscDebugStepInto, VscDebugStepOut, VscDebugContinue } from "react-icons/vsc";
import { PropertyRowCustom, propertyTableStyles as pts } from "../../components/PropertyTable";

interface DebugInspectorProps {}

type DatumAccessRef =
  | {
      type: "scriptInstance";
      instanceId: ScriptInstanceId;
    }
  | {
      type: "datum";
      datumRef: DatumRef;
    };

function DatumRow({ label, datumRef }: { label?: string; datumRef: DatumAccessRef }) {
  const [isExpanded, setIsExpanded] = useState(false);
  const datum = useAppSelector((state) => {
    switch (datumRef.type) {
      case "scriptInstance":
        return state.vm.scriptInstanceSnapshots[datumRef.instanceId];
      case "datum":
        if (datumRef.datumRef === 0) {
          return { type: 'void' as const, debugDescription: '<Void>' };
        } else {
          return state.vm.datumSnapshots[datumRef.datumRef];
        }
    }
  });
  const datumLoaded = !!datum;
  useEffect(() => {
    if (!datumLoaded && datumRef) {
      switch (datumRef.type) {
        case "scriptInstance":
          request_script_instance_snapshot(datumRef.instanceId);
          break;
        case "datum":
          request_datum(datumRef.datumRef);
          break;
      }
    }
  }, [datumLoaded, datumRef]);

  if (datumRef && !datum) {
    return (
      <PropertyRowCustom label={label ?? ""}>
        <span className={pts.propNull}>Loading...</span>
      </PropertyRowCustom>
    );
  }

  const isExpandable = datum.type === "scriptInstance" || datum.type === "propList" || datum.type === "list";

  const childEntries = (): Record<string, DatumAccessRef> => {
    if (datum.type === "scriptInstance") {
      return {
        ancestor: datum.ancestor
          ? { type: "scriptInstance", instanceId: datum.ancestor }
          : { type: "datum", datumRef: 0 },
        ...Object.fromEntries(
          Object.entries(datum.properties).map(([key, value]) => [
            key, { type: "datum" as const, datumRef: value },
          ])
        ),
      };
    }
    if (datum.type === "propList") {
      return Object.fromEntries(
        Object.entries(datum.properties).map(([key, value]) => [
          key, { type: "datum" as const, datumRef: value },
        ])
      );
    }
    return {};
  };

  const childList = (): DatumRef[] => {
    if (datum.type === "list") return datum.items;
    return [];
  };

  const valueContent = (
    <>
      {isExpandable && (
        <span
          className={pts.propExpandToggle}
          onClick={(e) => { e.stopPropagation(); setIsExpanded(!isExpanded); }}
          style={{ marginRight: 4 }}
        >
          {isExpanded ? "\u25BC" : "\u25B6"}
        </span>
      )}
      <span className={pts.propString}>{datum.debugDescription}</span>
      {datum.type === "javascript" && (datum as any).bytes instanceof Uint8Array && (
        <>
          {" "}
          <button
            className={pts.propExpandToggle}
            onClick={(e) => {
              e.stopPropagation();
              downloadBlob((datum as any).bytes, "js_datum.bin");
            }}
          >
            (Save)
          </button>
        </>
      )}
    </>
  );

  return (
    <>
      <PropertyRowCustom label={label ?? ""}>
        {valueContent}
      </PropertyRowCustom>
      {isExpanded && datum.type === "list" && (
        <div className={pts.propNested}>
          {childList().map((item, i) => (
            <DatumRow key={i} label={String(i)} datumRef={{ type: "datum", datumRef: item }} />
          ))}
        </div>
      )}
      {isExpanded && (datum.type === "scriptInstance" || datum.type === "propList") && (
        <div className={pts.propNested}>
          {Object.entries(childEntries()).map(([key, ref]) => (
            <DatumRow key={key} label={key} datumRef={ref} />
          ))}
        </div>
      )}
    </>
  );
}

function DatumTable({ datums }: { datums: Record<string, DatumAccessRef> }) {
  return (
    <div className={pts.propTable}>
      {Object.entries(datums).map(([key, ref]) => (
        <DatumRow key={key} label={key} datumRef={ref} />
      ))}
    </div>
  );
}

function DatumList({ items }: { items: DatumRef[] }) {
  return (
    <div className={pts.propTable}>
      {items.map((ref, i) => (
        <DatumRow key={i} label={String(i)} datumRef={{ type: "datum", datumRef: ref }} />
      ))}
    </div>
  );
}

function DebugControls() {
  const dispatch = useAppDispatch();
  const scopes = useAppSelector((state) => selectScopes(state.vm));
  const castSnapshots = useAppSelector((state) => state.vm.castSnapshots);
  const viewMode = useAppSelector((state) => selectScriptViewMode(state.ui));
  const isPaused = scopes.length > 0;

  const handleStepOver = () => {
    if (!isPaused) return;

    // In assembly mode, use instruction-level step over
    if (viewMode === 'assembly') {
      step_over();
      return;
    }

    // In lingo mode, find the bytecode indices for the current line and skip them
    const currentScope = scopes[scopes.length - 1];
    if (!currentScope) {
      step_over();
      return;
    }

    const [castLib, memberNum] = currentScope.script_member_ref;
    const castSnapshot = castSnapshots[castLib];
    const memberRecord = castSnapshot?.members?.[memberNum];
    const memberSnapshot = memberRecord?.snapshot as IScriptMemberSnapshot | undefined;

    if (!memberSnapshot || memberSnapshot.type !== 'script') {
      step_over();
      return;
    }

    const handler = memberSnapshot.script.handlers.find(
      (h) => h.name === currentScope.handler_name
    );

    if (!handler?.lingo || !handler.bytecodeToLine) {
      step_over();
      return;
    }

    // Find which lingo line the current bytecode is on
    const currentLineIndex = handler.bytecodeToLine[currentScope.bytecode_index];
    if (currentLineIndex === undefined) {
      step_over();
      return;
    }

    // Get all bytecode indices for this line
    const currentLine = handler.lingo[currentLineIndex];
    if (!currentLine) {
      step_over();
      return;
    }

    // Call step_over_line with the bytecode indices to skip (as Uint32Array for WASM)
    step_over_line(new Uint32Array(currentLine.bytecodeIndices));
  };

  const handleStepInto = () => {
    if (!isPaused) return;

    // In assembly mode, use instruction-level step into
    if (viewMode === 'assembly') {
      step_into();
      return;
    }

    // In lingo mode, find the bytecode indices for the current line and skip them
    const currentScope = scopes[scopes.length - 1];
    if (!currentScope) {
      step_into();
      return;
    }

    const [castLib, memberNum] = currentScope.script_member_ref;
    const castSnapshot = castSnapshots[castLib];
    const memberRecord = castSnapshot?.members?.[memberNum];
    const memberSnapshot = memberRecord?.snapshot as IScriptMemberSnapshot | undefined;

    if (!memberSnapshot || memberSnapshot.type !== 'script') {
      step_into();
      return;
    }

    const handler = memberSnapshot.script.handlers.find(
      (h) => h.name === currentScope.handler_name
    );

    if (!handler?.lingo || !handler.bytecodeToLine) {
      step_into();
      return;
    }

    // Find which lingo line the current bytecode is on
    const currentLineIndex = handler.bytecodeToLine[currentScope.bytecode_index];
    if (currentLineIndex === undefined) {
      step_into();
      return;
    }

    // Get all bytecode indices for this line
    const currentLine = handler.lingo[currentLineIndex];
    if (!currentLine) {
      step_into();
      return;
    }

    // Call step_into_line with the bytecode indices to skip (as Uint32Array for WASM)
    step_into_line(new Uint32Array(currentLine.bytecodeIndices));
  };

  return (
    <div className={styles.buttonContainer}>
      <ReactIconButton
        icon={VscDebugContinue}
        title="Resume"
        disabled={!isPaused}
        onClick={() => {
          // Clear the scope list when resuming to avoid showing stale stack trace
          dispatch(scopeListChanged([]));
          resume_breakpoint();
        }}
      />
      <ReactIconButton
        icon={VscDebugStepInto}
        title="Step Into"
        disabled={!isPaused}
        onClick={() => {
          // Clear the scope list when resuming to avoid showing stale stack trace
          dispatch(scopeListChanged([]));
          handleStepInto();
        }}
      />
      <ReactIconButton
        icon={VscDebugStepOver}
        title="Step Over"
        disabled={!isPaused}
        onClick={() => {
          // Clear the scope list when resuming to avoid showing stale stack trace
          dispatch(scopeListChanged([]));
          handleStepOver();
        }}
      />
      <ReactIconButton
        icon={VscDebugStepOut}
        title="Step Out"
        disabled={!isPaused}
        onClick={() => {
          // Clear the scope list when resuming to avoid showing stale stack trace
          dispatch(scopeListChanged([]));
          step_out();
        }}
      />
      <IconButton
        icon={faWarning}
        title="Trigger Alert Hook"
        onClick={() => {
          trigger_alert_hook();
        }}
      />
    </div>
  );
}

function Scopes({ selectedScopeIndex, setSelectedScopeIndex }: { selectedScopeIndex?: number, setSelectedScopeIndex: (i: number) => void }) {
  const scopes = useAppSelector((state) => selectScopes(state.vm));
  const dispatch = useAppDispatch();

  const onSelectScope = (index: number) => {
    setSelectedScopeIndex(index);
    set_eval_scope_index(index);
    const scope = scopes[index];
    dispatch(onMemberSelected(scope.script_member_ref));
  };

  return <ListView
    selectedKey={selectedScopeIndex?.toString()}
    className={styles.listContainer}
  >
    {scopes
      .map((scope, scopeIndex) => {
        return (
          <ListView.Item
            key={
              scope.script_member_ref[0] +
              "-" +
              scope.script_member_ref[1] +
              "-" +
              scopeIndex
            }
            isSelected={selectedScopeIndex === scopeIndex}
            onClick={() => onSelectScope(scopeIndex)}
          >
            {/* {castNames[scope.script_member_ref[0] - 1]} - {casts[scope.script_member_ref[0] - 1].members[scope.script_member_ref[1]].name} - on {scope.handler_name} */}
            on {scope.handler_name}
          </ListView.Item>
        );
      })
      .reverse()}
  </ListView>
}

function Locals({ selectedScopeIndex }: { selectedScopeIndex?: number }) {
  const scopes = useAppSelector((state) => selectScopes(state.vm));
  const selectedScope =
    selectedScopeIndex !== undefined ? scopes[selectedScopeIndex] : undefined;
  return (
    <div className={pts.propTableScrollable}>
      <DatumTable
        datums={Object.fromEntries(
          Object.entries(selectedScope?.locals || {}).map(
            ([key, value]) => [
              key,
              { type: "datum" as const, datumRef: value },
            ]
          )
        )}
      />
    </div>
  );
}

function Args({ selectedScopeIndex }: { selectedScopeIndex?: number }) {
  const scopes = useAppSelector((state) => selectScopes(state.vm));
  const selectedScope =
    selectedScopeIndex !== undefined ? scopes[selectedScopeIndex] : undefined;

  return (
    <div className={pts.propTableScrollable}>
      <DatumList items={selectedScope?.args || []} />
    </div>
  );
}

function Stack({ selectedScopeIndex }: { selectedScopeIndex?: number }) {
  const scopes = useAppSelector((state) => selectScopes(state.vm));
  const selectedScope =
    selectedScopeIndex !== undefined ? scopes[selectedScopeIndex] : undefined;

  return (
    <div className={pts.propTableScrollable}>
      <DatumList items={selectedScope?.stack || []} />
    </div>
  );
}

function Globals() {
  const globals = useAppSelector((state) => selectGlobals(state.vm));
  return (
    <div className={pts.propTableScrollable}>
      <DatumTable
        datums={Object.fromEntries(
          Object.entries(globals).map(([key, value]) => [
            key,
            { type: "datum" as const, datumRef: value },
          ])
        )}
      />
    </div>
  );
}

export default function DebugInspector({}: DebugInspectorProps) {
  const [selectedScopeIndex, setSelectedScopeIndex] = useState<number>();

  const factory = (node: TabNode) => {
    const component = node.getComponent();
    if (component === 'controls') {
      return <DebugControls />;
    } else if (component === 'scopes') {
      return <Scopes selectedScopeIndex={selectedScopeIndex} setSelectedScopeIndex={setSelectedScopeIndex} />;
    } else if (component === 'locals') {
      return <Locals selectedScopeIndex={selectedScopeIndex} />;
    } else if (component === 'args') {
      return <Args selectedScopeIndex={selectedScopeIndex} />;
    } else if (component === 'stack') {
      return <Stack selectedScopeIndex={selectedScopeIndex} />;
    } else if (component === 'globals') {
      return <Globals />;
    } else {
      return null;
    }
  }
  return <div style={{ textAlign: 'left'}} ><Layout model={debugLayoutModel} factory={factory} /></div>;
}
