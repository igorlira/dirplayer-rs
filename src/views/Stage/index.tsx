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
} from "vm-rust";
import { useAppDispatch } from "../../store/hooks";
import { channelSelected } from "../../store/uiSlice";

import styles from "./styles.module.css";

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

export default function Stage({ showControls }: { showControls?: boolean }) {
  const [ref, { width, height }] = useMeasure();
  const isStageCanvasCreated = useRef(false);
  const containerRef = useRef<HTMLDivElement | null>(null);
  const [scale, setScale] = useState(1);
  const [pickingMode, setPickingMode] = useState(false);
  const dispatch = useAppDispatch();

  const onContainerRef = useCallback(
    (element: HTMLDivElement | null) => {
      containerRef.current = element;
      if (!element) {
        isStageCanvasCreated.current = false;
      }
      ref(element);
    },
    [ref]
  );

  useEffect(() => {
    if (width && height && !isStageCanvasCreated.current) {
      isStageCanvasCreated.current = true;
      player_create_canvas();
    }
  }, [width, height]);

  useEffect(() => {
    if (!width || !height) return;
    set_stage_size(width, height);
  }, [width, height]);

  // No synthetic ESC on pointer lock release — the game's own ESC key detection handles it

  useEffect(() => {
    player_set_picking_mode(pickingMode);
  }, [pickingMode]);

  function onMouseEvent(name: "move" | "down" | "up", e: React.PointerEvent) {
    const rect = e.currentTarget.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;

    if (pickingMode) {
      // Always forward mouse_move so the renderer can track the cursor for hover highlight
      if (name === "move") {
        mouse_move(x, y);
      }
      if (name === "down") {
        const channel = player_get_sprite_at(x, y);
        if (channel > 0) {
          player_set_debug_selected_channel(channel);
          dispatch(channelSelected(channel));
        }
      }
      return;
    }

    switch (name) {
      case "move":
        // When pointer is locked, use movementX/Y as deltas
        if (document.pointerLockElement) {
          const nativeEvent = e.nativeEvent as MouseEvent;
          mouse_move_delta(nativeEvent.movementX, nativeEvent.movementY);
        } else {
          mouse_move(x, y);
        }
        break;
      case "down":
        mouse_down(x, y);
        // Request pointer lock if the game wants it (cursor=200)
        if (wants_pointer_lock() && !document.pointerLockElement) {
          const target = e.currentTarget as HTMLElement;
          const canvas = target.querySelector("canvas");
          if (canvas) {
            canvas.requestPointerLock();
          }
        }
        break;
      case "up":
        mouse_up(x, y);
        break;
    }
  }

  return (
    <div className={styles.container} ref={onContainerRef}>
      <div
        style={{ transform: scale !== 1 ? `scale(${scale})` : undefined, cursor: pickingMode ? 'crosshair' : undefined }}
        tabIndex={0}
        id="stage_canvas_container"
        onPointerMove={(e) => onMouseEvent('move', e)}
        onPointerDown={(e) => onMouseEvent('down', e)}
        onPointerUp={(e) => onMouseEvent('up', e)}
        onKeyDown={e => {
          e.preventDefault();
          if (e.repeat) return; // Skip browser key repeats; Lingo handles held keys via keyPressed()
          key_down(e.key, e.keyCode)
        }}
        onKeyUp={e => key_up(e.key, e.keyCode)}
      ></div>
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
