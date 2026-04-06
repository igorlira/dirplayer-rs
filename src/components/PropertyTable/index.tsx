import { ReactNode, useState } from "react";
import { downloadBlob } from "../../utils/download";
import styles from "./styles.module.css";

export { styles as propertyTableStyles };

function PropertyValue({ value }: { value: unknown }) {
  const [expanded, setExpanded] = useState(false);

  if (value === null || value === undefined) {
    return <span className={styles.propNull}>null</span>;
  }
  if (typeof value === "boolean") {
    return <span className={styles.propBool}>{value ? "true" : "false"}</span>;
  }
  if (typeof value === "number") {
    return <span className={styles.propNumber}>{value}</span>;
  }
  if (typeof value === "string") {
    if (value.startsWith("#") && (value.length === 7 || value.length === 4)) {
      return (
        <span className={styles.propColor}>
          <span className={styles.propColorSwatch} style={{ backgroundColor: value }} />
          {value}
        </span>
      );
    }
    return <span className={styles.propString}>{value}</span>;
  }
  if (Array.isArray(value)) {
    if (value.length === 0) {
      return <span className={styles.propNull}>[] (empty)</span>;
    }
    // For arrays of simple values (strings, numbers), show inline if short
    const allSimple = value.every(
      (v) => typeof v === "string" || typeof v === "number" || typeof v === "boolean"
    );
    if (allSimple && value.length <= 8) {
      return (
        <span className={styles.propArray}>
          [{value.map((v, i) => (
            <span key={i}>
              {i > 0 && ", "}
              <PropertyValue value={v} />
            </span>
          ))}]
        </span>
      );
    }
    return (
      <span>
        <span
          className={styles.propExpandToggle}
          onClick={(e) => { e.stopPropagation(); setExpanded(!expanded); }}
        >
          {expanded ? "\u25BC" : "\u25B6"} Array[{value.length}]
        </span>
        {expanded && (
          <div className={styles.propNested}>
            {value.map((item, i) => (
              <PropertyRow key={i} label={String(i)} value={item} />
            ))}
          </div>
        )}
      </span>
    );
  }
  if (value instanceof Uint8Array) {
    return (
      <span>
        <span className={styles.propNull}>&lt;{value.length} bytes&gt;</span>
        {" "}
        <button
          className={styles.propExpandToggle}
          onClick={() => {
            downloadBlob(value, "data.bin");
          }}
        >
          (Save)
        </button>
      </span>
    );
  }
  if (typeof value === "object") {
    const entries = Object.entries(value as Record<string, unknown>);
    if (entries.length === 0) {
      return <span className={styles.propNull}>{"{}"}</span>;
    }
    return (
      <span>
        <span
          className={styles.propExpandToggle}
          onClick={(e) => { e.stopPropagation(); setExpanded(!expanded); }}
        >
          {expanded ? "\u25BC" : "\u25B6"} Object{`{${entries.length}}`}
        </span>
        {expanded && (
          <div className={styles.propNested}>
            {entries.map(([key, val]) => (
              <PropertyRow key={key} label={key} value={val} />
            ))}
          </div>
        )}
      </span>
    );
  }
  return <span>{String(value)}</span>;
}

function PropertyRow({ label, value }: { label: string; value: unknown }) {
  return (
    <div className={styles.propRow}>
      <span className={styles.propKey}>{label}</span>
      <span className={styles.propValue}>
        <PropertyValue value={value} />
      </span>
    </div>
  );
}

/** A row with a label and custom children as the value. */
export function PropertyRowCustom({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className={styles.propRow}>
      <span className={styles.propKey}>{label}</span>
      <span className={styles.propValue}>{children}</span>
    </div>
  );
}

export default function PropertyTable({ data, scrollable }: { data: Record<string, unknown>; scrollable?: boolean }) {
  const entries = Object.entries(data);
  const content = (
    <div className={styles.propTable}>
      {entries.map(([key, value]) => (
        <PropertyRow key={key} label={key} value={value} />
      ))}
    </div>
  );

  if (scrollable) {
    return <div className={styles.propTableScrollable}>{content}</div>;
  }
  return content;
}
