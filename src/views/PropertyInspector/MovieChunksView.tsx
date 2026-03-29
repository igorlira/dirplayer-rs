import { JsBridgeChunk } from "dirplayer-js-api";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useAppSelector } from "../../store/hooks";
import { get_cast_chunk_list, get_movie_top_level_chunks, get_chunk_bytes, get_parsed_chunk } from "vm-rust";
import { Layout, Model, TabNode } from "flexlayout-react";
import PropertyTable from "../../components/PropertyTable";
import { downloadBlob } from "../../utils/download";
import styles from "./styles.module.css";

const MOVIE_FILE_VALUE = "movie";

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

// --- Chunk Detail Panel ---

function ChunkDetailPanel({
  chunkId,
  chunk,
  parsedData,
}: {
  chunkId: number | null;
  chunk: JsBridgeChunk | null;
  parsedData: Record<string, unknown> | null;
}) {
  if (chunkId == null || !parsedData) {
    return (
      <div className={styles.chunkDetailEmpty}>
        Select a chunk to inspect its parsed data.
      </div>
    );
  }

  return (
    <div className={styles.chunkDetailContent}>
      <div className={styles.chunkDetailTitle}>
        <span className={styles.chunkFourcc}>{chunk?.fourcc}</span>
        <span className={styles.chunkId}> #{chunkId}</span>
        {chunk?.memberName && (
          <span className={styles.chunkMember}>
            {" "}[{chunk.memberNumber}: {chunk.memberName}]
          </span>
        )}
      </div>
      <PropertyTable data={parsedData} />
    </div>
  );
}

// --- Chunk Tree ---

function ChunkTreeNode({
  chunkId,
  chunk,
  childrenMap,
  chunks,
  depth,
  filterText,
  matchingIds,
  selectedChunkId,
  onSave,
  onSelect,
}: {
  chunkId: number;
  chunk: JsBridgeChunk;
  childrenMap: Record<number, number[]>;
  chunks: Partial<Record<number, JsBridgeChunk>>;
  depth: number;
  filterText: string;
  matchingIds: Set<number> | null;
  selectedChunkId: number | null;
  onSave: (chunkId: number, fourcc: string) => void;
  onSelect: (chunkId: number) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const children = useMemo(() => childrenMap[chunkId] || [], [childrenMap, chunkId]);
  const hasChildren = children.length > 0;

  // When filtering, auto-expand nodes that have matching descendants
  const isAutoExpanded = matchingIds !== null && hasChildren;
  const isExpanded = isAutoExpanded || expanded;

  const visibleChildren = useMemo(() => {
    if (!isExpanded) return [];
    if (matchingIds === null) return children;
    return children.filter((id) => matchingIds.has(id));
  }, [isExpanded, children, matchingIds]);

  const isDirectMatch =
    matchingIds !== null &&
    filterText &&
    (chunk.fourcc.toLowerCase().includes(filterText) ||
      String(chunkId).includes(filterText) ||
      (chunk.memberName && chunk.memberName.toLowerCase().includes(filterText)));

  const isSelected = selectedChunkId === chunkId;

  return (
    <>
      <div
        className={`${styles.chunkNode} ${isDirectMatch ? styles.chunkNodeMatch : ""} ${isSelected ? styles.chunkNodeSelected : ""}`}
        style={{ paddingLeft: depth * 16 + 4 }}
        onClick={() => onSelect(chunkId)}
      >
        <span
          className={styles.chunkExpander}
          onClick={(e) => {
            e.stopPropagation();
            setExpanded(!expanded);
          }}
        >
          {hasChildren ? (isExpanded ? "\u25BC" : "\u25B6") : " "}
        </span>
        <span className={styles.chunkFourcc}>{chunk.fourcc}</span>
        <span className={styles.chunkId}>#{chunkId}</span>
        {chunk.memberName && (
          <span className={styles.chunkMember}>
            [{chunk.memberNumber}: {chunk.memberName}]
          </span>
        )}
        <span className={styles.chunkSize}>{formatSize(chunk.len)}</span>
        <button
          className={styles.chunkSave}
          onClick={(e) => {
            e.stopPropagation();
            onSave(chunkId, chunk.fourcc);
          }}
          title="Save chunk content to file"
        >
          (Save)
        </button>
      </div>
      {isExpanded &&
        visibleChildren.map((childId) => {
          const childChunk = chunks[childId];
          if (!childChunk) return null;
          return (
            <ChunkTreeNode
              key={childId}
              chunkId={childId}
              chunk={childChunk}
              childrenMap={childrenMap}
              chunks={chunks}
              depth={depth + 1}
              filterText={filterText}
              matchingIds={matchingIds}
              selectedChunkId={selectedChunkId}
              onSave={onSave}
              onSelect={onSelect}
            />
          );
        })}
    </>
  );
}

// --- Chunk Tree Panel (toolbar + tree) ---

function ChunkTreePanel({
  filterText,
  setFilterText,
  selectedSource,
  setSelectedSource,
  castNames,
  visibleRoots,
  chunks,
  childrenMap,
  matchingIds,
  selectedChunkId,
  onSave,
  onSelect,
}: {
  filterText: string;
  setFilterText: (v: string) => void;
  selectedSource: string;
  setSelectedSource: (v: string) => void;
  castNames: string[];
  visibleRoots: number[];
  chunks: Partial<Record<number, JsBridgeChunk>>;
  childrenMap: Record<number, number[]>;
  matchingIds: Set<number> | null;
  selectedChunkId: number | null;
  onSave: (chunkId: number, fourcc: string) => void;
  onSelect: (chunkId: number) => void;
}) {
  const lowerFilter = filterText.toLowerCase().trim();

  return (
    <div className={styles.movieChunksContainer}>
      <div className={styles.chunkToolbar}>
        <select
          className={styles.chunkCastFilter}
          value={selectedSource}
          onChange={(e) => setSelectedSource(e.target.value)}
        >
          <option value={MOVIE_FILE_VALUE}>Movie file</option>
          {castNames.map((name, i) => (
            <option key={i + 1} value={i + 1}>
              {name || `Cast ${i + 1}`}
            </option>
          ))}
        </select>
        <input
          className={styles.chunkSearchInput}
          type="text"
          placeholder="Filter by fourcc, ID, or member name..."
          value={filterText}
          onChange={(e) => setFilterText(e.target.value)}
        />
        {filterText && (
          <button
            className={styles.chunkSearchClear}
            onClick={() => setFilterText("")}
          >
            &times;
          </button>
        )}
      </div>
      <div className={styles.chunkTree}>
        {visibleRoots.length === 0 && (
          <div style={{ padding: 8, color: "#999", fontSize: 12 }}>
            {filterText ? "No chunks match the filter." : "No chunks found."}
          </div>
        )}
        {visibleRoots.map((id) => {
          const chunk = chunks[id];
          if (!chunk) return null;
          return (
            <ChunkTreeNode
              key={id}
              chunkId={id}
              chunk={chunk}
              childrenMap={childrenMap}
              chunks={chunks}
              depth={0}
              filterText={lowerFilter}
              matchingIds={matchingIds}
              selectedChunkId={selectedChunkId}
              onSave={onSave}
              onSelect={onSelect}
            />
          );
        })}
      </div>
    </div>
  );
}

// --- Main Component ---

export default function MovieChunksView() {
  const [filterText, setFilterText] = useState("");
  const [selectedSource, setSelectedSource] = useState<string>(MOVIE_FILE_VALUE);
  const [chunks, setChunks] = useState<Partial<Record<number, JsBridgeChunk>>>({});
  const [selectedChunkId, setSelectedChunkId] = useState<number | null>(null);
  const [parsedData, setParsedData] = useState<Record<string, unknown> | null>(null);
  const castNames = useAppSelector((state) => state.vm.castNames);
  const isMovieLoaded = useAppSelector((state) => state.vm.isMovieLoaded);

  // Fetch chunks when source changes
  useEffect(() => {
    if (!isMovieLoaded) {
      setChunks({});
      return;
    }

    try {
      if (selectedSource === MOVIE_FILE_VALUE) {
        const result = get_movie_top_level_chunks();
        setChunks(result || {});
      } else {
        const castNumber = Number(selectedSource);
        if (castNumber > 0) {
          const result = get_cast_chunk_list(castNumber);
          setChunks(result || {});
        } else {
          setChunks({});
        }
      }
    } catch (e) {
      console.error("Failed to fetch chunks", e);
      setChunks({});
    }
    // Clear selection when source changes
    setSelectedChunkId(null);
    setParsedData(null);
  }, [selectedSource, isMovieLoaded]);

  // The cast number to use for WASM calls (0 = main movie file)
  const castNumber = selectedSource === MOVIE_FILE_VALUE ? 0 : Number(selectedSource);

  const handleSave = useCallback((chunkId: number, fourcc: string) => {
    try {
      const bytes = get_chunk_bytes(castNumber, chunkId);
      if (bytes) {
        downloadBlob(bytes, `${fourcc.trim()}_${chunkId}.bin`);
      } else {
        console.warn("No data found for chunk", chunkId);
      }
    } catch (e) {
      console.error("Failed to save chunk", chunkId, e);
    }
  }, [castNumber]);

  const handleSelect = useCallback((chunkId: number) => {
    setSelectedChunkId(chunkId);
    try {
      const data = get_parsed_chunk(castNumber, chunkId);
      setParsedData(data as Record<string, unknown>);
    } catch (e) {
      console.error("Failed to parse chunk", chunkId, e);
      setParsedData({ error: String(e) });
    }
  }, [castNumber]);

  // Build parent-child map and find root chunks
  const { childrenMap, rootIds } = useMemo(() => {
    const childrenMap: Record<number, number[]> = {};
    const rootIds: number[] = [];
    const allIds = Object.keys(chunks).map(Number);

    allIds.forEach((id) => {
      const chunk = chunks[id];
      if (!chunk) return;
      if (chunk.owner != null && chunks[chunk.owner] != null) {
        if (!childrenMap[chunk.owner]) childrenMap[chunk.owner] = [];
        childrenMap[chunk.owner].push(id);
      } else {
        rootIds.push(id);
      }
    });

    // Sort children and roots by ID
    Object.keys(childrenMap).forEach((key) => {
      childrenMap[Number(key)].sort((a, b) => a - b);
    });
    rootIds.sort((a, b) => a - b);

    return { childrenMap, rootIds };
  }, [chunks]);

  // When filtering, compute which IDs match and which ancestors need to be visible
  const matchingIds = useMemo<Set<number> | null>(() => {
    const lower = filterText.toLowerCase().trim();
    if (lower.length === 0) return null;

    const directMatches = new Set<number>();
    Object.entries(chunks).forEach(([idStr, chunk]) => {
      if (!chunk) return;
      const id = Number(idStr);

      if (
        chunk.fourcc.toLowerCase().includes(lower) ||
        String(id).includes(lower) ||
        (chunk.memberName && chunk.memberName.toLowerCase().includes(lower))
      ) {
        directMatches.add(id);
      }
    });

    // Walk up from each match to include all ancestors
    const visible = new Set<number>(Array.from(directMatches));
    Array.from(directMatches).forEach((id) => {
      let current = id;
      while (true) {
        const chunk = chunks[current];
        if (!chunk || chunk.owner == null || chunks[chunk.owner] == null) break;
        if (visible.has(chunk.owner)) break;
        visible.add(chunk.owner);
        current = chunk.owner;
      }
    });

    return visible;
  }, [filterText, chunks]);

  const visibleRoots = useMemo(() => {
    if (matchingIds === null) return rootIds;
    return rootIds.filter((id) => matchingIds.has(id));
  }, [rootIds, matchingIds]);

  const selectedChunk = selectedChunkId != null ? chunks[selectedChunkId] ?? null : null;

  const layoutModel = useMemo(() => Model.fromJson({
    global: {
      rootOrientationVertical: true,
      tabEnableClose: false,
    },
    layout: {
      type: "row",
      children: [
        {
          type: "tabset",
          weight: 60,
          children: [
            {
              type: "tab",
              name: "Chunks",
              component: "chunkTree",
            },
          ],
        },
        {
          type: "tabset",
          weight: 40,
          children: [
            {
              type: "tab",
              name: "Parsed Data",
              component: "chunkDetail",
            },
          ],
        },
      ],
    },
  }), []);

  // Use a ref to always have the latest state in the factory without recreating it
  const stateRef = useRef({
    filterText, setFilterText, selectedSource, setSelectedSource,
    castNames, visibleRoots, chunks, childrenMap, matchingIds,
    selectedChunkId, selectedChunk, parsedData,
    handleSave, handleSelect,
  });
  stateRef.current = {
    filterText, setFilterText, selectedSource, setSelectedSource,
    castNames, visibleRoots, chunks, childrenMap, matchingIds,
    selectedChunkId, selectedChunk, parsedData,
    handleSave, handleSelect,
  };

  const factory = useCallback((node: TabNode) => {
    const s = stateRef.current;
    switch (node.getComponent()) {
      case "chunkTree":
        return (
          <ChunkTreePanel
            filterText={s.filterText}
            setFilterText={s.setFilterText}
            selectedSource={s.selectedSource}
            setSelectedSource={s.setSelectedSource}
            castNames={s.castNames}
            visibleRoots={s.visibleRoots}
            chunks={s.chunks}
            childrenMap={s.childrenMap}
            matchingIds={s.matchingIds}
            selectedChunkId={s.selectedChunkId}
            onSave={s.handleSave}
            onSelect={s.handleSelect}
          />
        );
      case "chunkDetail":
        return (
          <ChunkDetailPanel
            chunkId={s.selectedChunkId}
            chunk={s.selectedChunk}
            parsedData={s.parsedData}
          />
        );
      default:
        return null;
    }
  }, []);

  // Force layout re-render when state changes
  const [, forceUpdate] = useState(0);
  useEffect(() => {
    forceUpdate((n) => n + 1);
  }, [filterText, selectedSource, chunks, selectedChunkId, parsedData, castNames, visibleRoots, childrenMap, matchingIds]);

  return (
    <div className={styles.movieChunksContainer}>
      <Layout model={layoutModel} factory={factory} />
    </div>
  );
}
