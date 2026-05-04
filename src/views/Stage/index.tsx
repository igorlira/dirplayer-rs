import { useMeasure } from "@uidotdev/usehooks";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  set_stage_size,
  player_create_canvas,
  mouse_move,
  mouse_move_delta,
  mouse_down,
  mouse_up,
  key_down,
  key_up,
  wants_pointer_lock,
  player_set_picking_mode,
  player_get_sprite_at,
  player_set_debug_selected_channel,
  is_sprite_editable_field,
} from "vm-rust";
import { useAppDispatch } from "../../store/hooks";
import { channelSelected } from "../../store/uiSlice";

import styles from "./styles.module.css";

const MIN_SCALE = 0.1;
const MAX_SCALE = 10;

function ZoomSlider({ scale, setScale }: { scale: number; setScale: (scale: number) => void }) {
  return (
    <div>
      <input
        type="range"
        min="0.5"
        max="2"
        step="0.1"
        value={scale}
        onChange={(e) => setScale(parseFloat(e.target.value))}
      />
      {Math.round(scale * 100)}%
    </div>
  );
}

type Pt = { x: number; y: number };

function MiniMap({
  stageWidth,
  stageHeight,
  viewportWidth,
  viewportHeight,
  scale,
  pan,
  onPanChange,
}: {
  stageWidth: number;
  stageHeight: number;
  viewportWidth: number;
  viewportHeight: number;
  scale: number;
  pan: Pt;
  onPanChange: (newPan: Pt) => void;
}) {
  const MAX_W = 96;
  const MAX_H = 72;
  const mapScale = Math.min(MAX_W / stageWidth, MAX_H / stageHeight);
  const mapW = stageWidth * mapScale;
  const mapH = stageHeight * mapScale;

  const viewX = -pan.x / scale;
  const viewY = -pan.y / scale;
  const viewW = viewportWidth / scale;
  const viewH = viewportHeight / scale;

  const dragRef = useRef<{ startClient: Pt; startPan: Pt } | null>(null);

  function onDown(e: React.PointerEvent) {
    e.stopPropagation();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    dragRef.current = {
      startClient: { x: e.clientX, y: e.clientY },
      startPan: pan,
    };
  }
  function onMove(e: React.PointerEvent) {
    if (!dragRef.current) return;
    e.stopPropagation();
    const dx = e.clientX - dragRef.current.startClient.x;
    const dy = e.clientY - dragRef.current.startClient.y;
    onPanChange({
      x: dragRef.current.startPan.x - (dx / mapScale) * scale,
      y: dragRef.current.startPan.y - (dy / mapScale) * scale,
    });
  }
  function onUp(e: React.PointerEvent) {
    if (!dragRef.current) return;
    e.stopPropagation();
    dragRef.current = null;
  }

  return (
    <div
      className={styles.minimap}
      onPointerDown={onDown}
      onPointerMove={onMove}
      onPointerUp={onUp}
      onPointerCancel={onUp}
    >
      <div
        className={styles.minimapInner}
        style={{ width: mapW, height: mapH }}
      >
        <div
          className={styles.minimapViewport}
          style={{
            left: viewX * mapScale,
            top: viewY * mapScale,
            width: viewW * mapScale,
            height: viewH * mapScale,
          }}
        />
      </div>
    </div>
  );
}

function PanIcon() {
  return (
    <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M18 11V6a2 2 0 0 0-4 0" />
      <path d="M14 10V4a2 2 0 0 0-4 0v2" />
      <path d="M10 10.5V6a2 2 0 0 0-4 0v8" />
      <path d="M18 8a2 2 0 1 1 4 0v6a8 8 0 0 1-8 8h-2c-2.8 0-4.5-.86-5.99-2.34l-3.6-3.6a2 2 0 0 1 2.83-2.83L7 15" />
    </svg>
  );
}

export default function Stage({ showControls }: { showControls?: boolean }) {
  const [outerMeasureRef, { width: outerWidth, height: outerHeight }] = useMeasure();
  const [stageMeasureRef, { width: stageWidth, height: stageHeight }] = useMeasure();
  const isStageCanvasCreated = useRef(false);
  const outerRef = useRef<HTMLDivElement | null>(null);
  const stageEl = useRef<HTMLDivElement | null>(null);
  const [scale, setScale] = useState(1);
  const [pan, setPan] = useState<Pt>({ x: 0, y: 0 });
  const [transformInitialized, setTransformInitialized] = useState(false);
  const [pickingMode, setPickingMode] = useState(false);
  const [panMode, setPanMode] = useState(false);
  const hiddenInputRef = useRef<HTMLInputElement | null>(null);
  const dispatch = useAppDispatch();

  const scaleRef = useRef(scale);
  const panRef = useRef(pan);
  const panModeRef = useRef(panMode);
  useEffect(() => { scaleRef.current = scale; }, [scale]);
  useEffect(() => { panRef.current = pan; }, [pan]);
  useEffect(() => { panModeRef.current = panMode; }, [panMode]);

  // outer-relative coords keyed by pointerId
  const activePointersRef = useRef<Map<number, Pt>>(new Map());
  // suppresses single-finger canvas events while any multi-touch gesture is winding down
  const suppressCanvasUntilReleaseRef = useRef(false);
  // tracks the in-progress single-finger interaction so we can deliver mouse_up
  // even if the finger lifts outside the canvas rect
  const singleTouchActiveRef = useRef(false);
  // single-finger pan (when panMode is on)
  const singlePanRef = useRef<{ startPointer: Pt; startPan: Pt } | null>(null);
  const gestureRef = useRef<{
    initialDist: number;
    initialScale: number;
    initialAnchor: Pt; // canvas-space point under the initial centroid
  } | null>(null);

  const onContainerRef = useCallback(
    (element: HTMLDivElement | null) => {
      outerRef.current = element;
      if (!element) {
        isStageCanvasCreated.current = false;
      }
      outerMeasureRef(element);
    },
    [outerMeasureRef]
  );

  const onStageRef = useCallback(
    (element: HTMLDivElement | null) => {
      stageEl.current = element;
      stageMeasureRef(element);
    },
    [stageMeasureRef]
  );

  useEffect(() => {
    if (outerWidth && outerHeight && !isStageCanvasCreated.current) {
      isStageCanvasCreated.current = true;
      player_create_canvas();
    }
  }, [outerWidth, outerHeight]);

  useEffect(() => {
    if (!outerWidth || !outerHeight) return;
    set_stage_size(outerWidth, outerHeight);
  }, [outerWidth, outerHeight]);

  // Center the stage in the viewport on first sizing. After that, the user owns
  // the transform — don't reset on subsequent size changes (e.g. window resize)
  // since that would yank their pan/zoom out from under them.
  useEffect(() => {
    if (transformInitialized) return;
    if (!outerWidth || !outerHeight || !stageWidth || !stageHeight) return;
    setPan({
      x: (outerWidth - stageWidth) / 2,
      y: (outerHeight - stageHeight) / 2,
    });
    setTransformInitialized(true);
  }, [outerWidth, outerHeight, stageWidth, stageHeight, transformInitialized]);

  // Handle pointer-locked mouse movement (events fire on document, not the div)
  useEffect(() => {
    const handleLockedMouseMove = (e: MouseEvent) => {
      if (document.pointerLockElement) {
        if (!wants_pointer_lock()) {
          document.exitPointerLock();
          return;
        }
        mouse_move_delta(e.movementX, e.movementY);
      }
    };
    // Handle keyboard during pointer lock (focus may be on canvas, not the div)
    const handleKeyDown = (e: KeyboardEvent) => {
      if (document.pointerLockElement) {
        // Don't prevent ESC — browser needs it to exit pointer lock
        if (e.key !== "Escape") e.preventDefault();
        if (!e.repeat) {
          key_down(e.key, e.keyCode);
        }
      }
    };
    const handleKeyUp = (e: KeyboardEvent) => {
      if (document.pointerLockElement) {
        key_up(e.key, e.keyCode);
      }
    };
    document.addEventListener("mousemove", handleLockedMouseMove);
    document.addEventListener("keydown", handleKeyDown);
    document.addEventListener("keyup", handleKeyUp);
    return () => {
      document.removeEventListener("mousemove", handleLockedMouseMove);
      document.removeEventListener("keydown", handleKeyDown);
      document.removeEventListener("keyup", handleKeyUp);
    };
  }, []);

  useEffect(() => {
    player_set_picking_mode(pickingMode);
  }, [pickingMode]);

  function pointerOuterPos(e: React.PointerEvent): Pt {
    const rect = outerRef.current?.getBoundingClientRect();
    return rect
      ? { x: e.clientX - rect.left, y: e.clientY - rect.top }
      : { x: 0, y: 0 };
  }

  function outerToCanvas(p: Pt): Pt {
    return {
      x: (p.x - panRef.current.x) / scaleRef.current,
      y: (p.y - panRef.current.y) / scaleRef.current,
    };
  }

  function isInsideCanvas(c: Pt): boolean {
    if (!stageWidth || !stageHeight) return false;
    return c.x >= 0 && c.y >= 0 && c.x < stageWidth && c.y < stageHeight;
  }

  function gestureCentroidAndDist(): { centroid: Pt; dist: number } {
    const pts = Array.from(activePointersRef.current.values());
    let cx = 0, cy = 0;
    for (const p of pts) { cx += p.x; cy += p.y; }
    cx /= pts.length; cy /= pts.length;
    let dist = 0;
    if (pts.length >= 2) {
      const dx = pts[0].x - pts[1].x;
      const dy = pts[0].y - pts[1].y;
      dist = Math.hypot(dx, dy);
    }
    return { centroid: { x: cx, y: cy }, dist };
  }

  function dispatchVMMouse(name: "move" | "down" | "up", canvasX: number, canvasY: number, e: React.PointerEvent) {
    if (pickingMode) {
      if (name === "move") {
        mouse_move(canvasX, canvasY);
      }
      if (name === "down") {
        const channel = player_get_sprite_at(canvasX, canvasY);
        if (channel > 0) {
          player_set_debug_selected_channel(channel);
          dispatch(channelSelected(channel));
        }
      }
      return;
    }

    switch (name) {
      case "move":
        if (!document.pointerLockElement) {
          mouse_move(canvasX, canvasY);
        }
        break;
      case "down": {
        const spriteId = player_get_sprite_at(canvasX, canvasY);
        const isEditable = spriteId > 0 && is_sprite_editable_field(spriteId);
        mouse_down(canvasX, canvasY);
        if (isEditable) {
          e.preventDefault();
          hiddenInputRef.current?.focus();
        } else if (document.activeElement === hiddenInputRef.current) {
          hiddenInputRef.current?.blur();
          outerRef.current?.focus();
        }
        if (wants_pointer_lock() && !document.pointerLockElement) {
          const canvas = stageEl.current?.querySelector("canvas");
          if (canvas) {
            canvas.requestPointerLock();
          }
        }
        break;
      }
      case "up":
        mouse_up(canvasX, canvasY);
        break;
    }
  }

  function onPointerDown(e: React.PointerEvent) {
    const p = pointerOuterPos(e);
    activePointersRef.current.set(e.pointerId, p);
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);

    if (activePointersRef.current.size >= 2) {
      // Entering a multi-touch gesture. If a single-finger interaction was in
      // progress, send mouse_up so the VM doesn't see a stuck button.
      if (singleTouchActiveRef.current) {
        const c = outerToCanvas(p);
        mouse_up(c.x, c.y);
        singleTouchActiveRef.current = false;
      }
      singlePanRef.current = null;
      const { centroid, dist } = gestureCentroidAndDist();
      gestureRef.current = {
        initialDist: dist || 1,
        initialScale: scaleRef.current,
        initialAnchor: {
          x: (centroid.x - panRef.current.x) / scaleRef.current,
          y: (centroid.y - panRef.current.y) / scaleRef.current,
        },
      };
      suppressCanvasUntilReleaseRef.current = true;
      return;
    }

    // Single-finger touch.
    if (suppressCanvasUntilReleaseRef.current) return;
    if (panModeRef.current) {
      // Hand-mode: drag pans the canvas instead of dispatching to the VM.
      singlePanRef.current = { startPointer: p, startPan: panRef.current };
      return;
    }
    const c = outerToCanvas(p);
    if (isInsideCanvas(c)) {
      singleTouchActiveRef.current = true;
      dispatchVMMouse("down", c.x, c.y, e);
    }
  }

  function onPointerMove(e: React.PointerEvent) {
    const p = pointerOuterPos(e);
    const isTracked = activePointersRef.current.has(e.pointerId);
    if (isTracked) {
      activePointersRef.current.set(e.pointerId, p);
    }

    if (isTracked && activePointersRef.current.size >= 2 && gestureRef.current) {
      const { centroid, dist } = gestureCentroidAndDist();
      const g = gestureRef.current;
      let newScale = g.initialScale * (dist / g.initialDist);
      newScale = Math.max(MIN_SCALE, Math.min(MAX_SCALE, newScale));
      setScale(newScale);
      setPan({
        x: centroid.x - g.initialAnchor.x * newScale,
        y: centroid.y - g.initialAnchor.y * newScale,
      });
      return;
    }

    if (suppressCanvasUntilReleaseRef.current) return;

    if (isTracked && singlePanRef.current) {
      const sp = singlePanRef.current;
      setPan({
        x: sp.startPan.x + (p.x - sp.startPointer.x),
        y: sp.startPan.y + (p.y - sp.startPointer.y),
      });
      return;
    }

    // Skip hover dispatch while in hand mode — rollover/cursor changes would be
    // distracting when the user is navigating, not interacting.
    if (panModeRef.current && !isTracked) return;

    // Forward to the VM. This covers both hover (mouse with no button — pointer
    // not in the tracked map) and active drags (pointer down, in the map).
    const c = outerToCanvas(p);
    dispatchVMMouse("move", c.x, c.y, e);
  }

  function onPointerUp(e: React.PointerEvent) {
    if (!activePointersRef.current.has(e.pointerId)) return;
    const lastP = pointerOuterPos(e);
    const wasMultiTouch = activePointersRef.current.size >= 2;
    activePointersRef.current.delete(e.pointerId);

    if (wasMultiTouch) {
      if (activePointersRef.current.size < 2) {
        gestureRef.current = null;
      }
      // Stay suppressed until every finger is up — otherwise lifting one of two
      // fingers would turn the remaining touch into a spurious VM drag.
      if (activePointersRef.current.size === 0) {
        suppressCanvasUntilReleaseRef.current = false;
      }
      return;
    }

    if (singlePanRef.current) {
      singlePanRef.current = null;
    } else if (singleTouchActiveRef.current) {
      const c = outerToCanvas(lastP);
      dispatchVMMouse("up", c.x, c.y, e);
      singleTouchActiveRef.current = false;
    }
    if (activePointersRef.current.size === 0) {
      suppressCanvasUntilReleaseRef.current = false;
    }
  }

  // Show minimap only when stage extends beyond the viewport — keeps it out
  // of the way for movies that fit comfortably on screen.
  const stageFitsInViewport =
    stageWidth && stageHeight && outerWidth && outerHeight &&
    pan.x >= 0 && pan.y >= 0 &&
    pan.x + stageWidth * scale <= outerWidth &&
    pan.y + stageHeight * scale <= outerHeight;
  const showMinimap = !stageFitsInViewport && !!stageWidth && !!stageHeight && !!outerWidth && !!outerHeight;

  return (
    <div
      className={styles.container}
      ref={onContainerRef}
      tabIndex={0}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
      onKeyDown={e => {
        // When the hidden input is focused (editable field tapped), let its
        // own handlers + onInput drive key dispatch. Otherwise we'd double-fire
        // every key — both here on bubble and from the input's handler.
        if (document.activeElement === hiddenInputRef.current) return;
        e.preventDefault();
        if (e.repeat) return; // Skip browser key repeats; Lingo handles held keys via keyPressed()
        key_down(e.key, e.keyCode);
      }}
      onKeyUp={e => {
        if (document.activeElement === hiddenInputRef.current) return;
        key_up(e.key, e.keyCode);
      }}
    >
      <div
        className={styles.stageWrapper}
        style={{
          transform: `translate(${pan.x}px, ${pan.y}px) scale(${scale})`,
          cursor: pickingMode ? "crosshair" : panMode ? "grab" : undefined,
        }}
      >
        <div
          ref={onStageRef}
          id="stage_canvas_container"
        />
      </div>
      <div
        className={styles.overlay}
        onPointerDown={e => e.stopPropagation()}
        onPointerMove={e => e.stopPropagation()}
        onPointerUp={e => e.stopPropagation()}
      >
        <button
          type="button"
          className={panMode ? styles.panToggleActive : styles.panToggle}
          onClick={() => setPanMode(v => !v)}
          title={panMode ? "Exit pan mode" : "Pan mode (one-finger drag pans)"}
          aria-pressed={panMode}
        >
          <PanIcon />
        </button>
        {showMinimap && stageWidth && stageHeight && outerWidth && outerHeight && (
          <MiniMap
            stageWidth={stageWidth}
            stageHeight={stageHeight}
            viewportWidth={outerWidth}
            viewportHeight={outerHeight}
            scale={scale}
            pan={pan}
            onPanChange={setPan}
          />
        )}
      </div>
      {/* Hidden input for mobile virtual keyboard support.
          When an editable field sprite is tapped, this input receives focus
          to trigger the mobile keyboard. Key events are forwarded to WASM.
          pointer-events:none ensures it never intercepts stage touches. */}
      <input
        ref={hiddenInputRef}
        style={{
          position: 'absolute',
          left: 0,
          top: 0,
          width: '1px',
          height: '1px',
          opacity: 0.01,
          pointerEvents: 'none',
          zIndex: -1,
        }}
        type="text"
        inputMode="text"
        autoComplete="off"
        autoCorrect="off"
        autoCapitalize="off"
        spellCheck={false}
        onKeyDown={e => {
          // Handle special keys that don't produce input events.
          // Allow browser key-repeat through to wasm so holding e.g. Backspace
          // continuously deletes characters at the browser's repeat cadence
          // — exactly the behaviour users expect from an editable field. The
          // outer canvas handler still blocks repeats because Lingo's
          // keyPressed() polling drives held game keys, but the hidden input
          // is only focused when an editable text/field sprite is active so
          // letting repeats through here is safe.
          const special = ['Enter', 'Backspace', 'Tab', 'ArrowUp', 'ArrowDown',
                           'ArrowLeft', 'ArrowRight', 'Escape', 'Delete'];
          if (special.includes(e.key)) {
            e.preventDefault();
            key_down(e.key, e.keyCode);
          }
          // Regular characters flow through to onInput below
        }}
        onKeyUp={e => {
          key_up(e.key, e.keyCode);
        }}
        onInput={e => {
          // Catch characters from virtual keyboards (and desktop as fallback).
          // Virtual keyboards may not fire individual keyDown events for characters.
          const input = e.currentTarget;
          const value = input.value;
          if (value) {
            // Use toUpperCase().charCodeAt(0) to match JS keyCode convention
            // (e.g. 'a' → 65 not 97) so the keyboard_map maps correctly.
            const chars = value.split('');
            for (let i = 0; i < chars.length; i++) {
              key_down(chars[i], chars[i].toUpperCase().charCodeAt(0));
            }
            input.value = '';
            // Defer key_up so the async keyDown command handler can read
            // keyboard state (the key, the keyCode) before it's cleared.
            // key_down() sets state immediately but the handler runs async.
            setTimeout(() => {
              for (let i = 0; i < chars.length; i++) {
                key_up(chars[i], chars[i].toUpperCase().charCodeAt(0));
              }
            }, 100);
          }
        }}
      />
      {showControls && (
        <div className={styles.controlBar}>
          <button
            className={pickingMode ? styles.pickButtonActive : styles.pickButton}
            onClick={() => setPickingMode(!pickingMode)}
            title="Pick sprite"
          >
            Pick
          </button>
          <ZoomSlider scale={scale} setScale={setScale} />
        </div>
      )}
    </div>
  );
}
