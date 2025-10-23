import { useEffect, useState } from "react";
import { useAppDispatch, useAppSelector } from "../../store/hooks";
import { selectGlobals, selectScopes } from "../../store/vmSlice";
import styles from "./styles.module.css";
import IconButton from "../../components/IconButton";
import { faPlay, faWarning } from "@fortawesome/free-solid-svg-icons";
import {
  resume_breakpoint,
  request_datum,
  request_script_instance_snapshot,
  trigger_alert_hook,
} from "vm-rust";
import ListView from "../../components/ListView";
import { onMemberSelected } from "../../store/uiSlice";
import { DatumRef, ScriptInstanceId } from "../../vm";
import { Layout, TabNode } from "flexlayout-react";
import { debugLayoutModel } from "./layout";

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

type DatumDebugListProps = {
  label?: string;
  datumRef: DatumAccessRef;
  depth?: number;
};
function DatumDebugListItems({
  label,
  datumRef,
  depth = 0,
}: DatumDebugListProps) {
  const [isExpanded, setIsExpanded] = useState(false);
  const datum = useAppSelector((state) => {
    switch (datumRef.type) {
      case "scriptInstance":
        return state.vm.scriptInstanceSnapshots[datumRef.instanceId];
      case "datum":
        if (datumRef.datumRef === 0) {
          return {type: 'void' as const, debugDescription: '<Void>'};
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
      <ListView.Item style={{ paddingLeft: 16 * depth }}>
        Loading...
      </ListView.Item>
    );
  }
  return (
    <>
      <ListView.Item
        style={{ paddingLeft: 16 * depth }}
        onClick={() => setIsExpanded(!isExpanded)}
      >
        {label && `${label}: `}
        {datum.debugDescription}
      </ListView.Item>
      {isExpanded && datum.type === "scriptInstance" && (
        <DatumDebugTable
          datums={{
            ancestor: datum.ancestor
              ? {
                  type: "scriptInstance",
                  instanceId: datum.ancestor,
                }
              : {
                  type: "datum",
                  datumRef: 0,
                },
            ...Object.fromEntries(
              Object.entries(datum.properties).map(([key, value]) => [
                key,
                {
                  type: "datum",
                  datumRef: value,
                },
              ])
            ),
          }}
          depth={depth + 1}
        />
      )}
      {isExpanded && datum.type === "propList" && (
        <DatumDebugTable
          datums={Object.fromEntries(
            Object.entries(datum.properties).map(([key, value]) => [
              key,
              {
                type: "datum",
                datumRef: value,
              },
            ])
          )}
          depth={depth + 1}
        />
      )}
      {isExpanded &&
        datum.type === "list" &&
        datum.items.map((item, i) => (
          <DatumDebugListItems
            key={i}
            datumRef={{ type: "datum", datumRef: item }}
            depth={depth + 1}
          />
        ))}
    </>
  );
}

type DatumDebugTableProps = {
  datums: Record<string, DatumAccessRef>;
  depth?: number;
};

function DatumDebugTable({ datums, depth }: DatumDebugTableProps) {
  return (
    <>
      {Object.entries(datums).map(([key, value]) => (
        <>
          <DatumDebugListItems label={key} datumRef={value} depth={depth} />
        </>
      ))}
    </>
  );
}

function DebugControls() {
  return (
    <div className={styles.buttonContainer}>
      <IconButton
        icon={faPlay}
        onClick={() => {
          resume_breakpoint();
        }}
      />
      <IconButton
        icon={faWarning}
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
    <ListView className={styles.listContainer}>
      <DatumDebugTable
        datums={Object.fromEntries(
          Object.entries(selectedScope?.locals || {}).map(
            ([key, value]) => [
              key,
              {
                type: "datum",
                datumRef: value,
              },
            ]
          )
        )}
      />
    </ListView>
  );
}

function Args({ selectedScopeIndex }: { selectedScopeIndex?: number }) {
  const scopes = useAppSelector((state) => selectScopes(state.vm));
  const selectedScope =
    selectedScopeIndex !== undefined ? scopes[selectedScopeIndex] : undefined;

  return (
    <ListView className={styles.listContainer}>
      {selectedScope?.args.map((datum, i) => {
        return (
          <DatumDebugListItems
            key={i}
            datumRef={{ type: "datum", datumRef: datum }}
          />
        );
      })}
    </ListView>
  );
}

function Stack({ selectedScopeIndex }: { selectedScopeIndex?: number }) {
  const scopes = useAppSelector((state) => selectScopes(state.vm));
  const selectedScope =
    selectedScopeIndex !== undefined ? scopes[selectedScopeIndex] : undefined;

  return (
    <ListView className={styles.listContainer}>
      {selectedScope &&
        selectedScope.stack.map((datum, i) => {
          return (
            <DatumDebugListItems
              key={i}
              datumRef={{ type: "datum", datumRef: datum }}
            />
          );
        })}
    </ListView>
  );
}

function Globals() {
  const globals = useAppSelector((state) => selectGlobals(state.vm));
  return (
    <ListView className={styles.listContainer}>
      <DatumDebugTable
        datums={Object.fromEntries(
          Object.entries(globals).map(([key, value]) => [
            key,
            {
              type: "datum",
              datumRef: value,
            },
          ])
        )}
      />
    </ListView>
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
  return <Layout model={debugLayoutModel} factory={factory} />;
}
