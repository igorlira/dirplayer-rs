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
  const hiddenInputRef = useRef<HTMLInputElement | null>(null);
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
        // When pointer is locked, skip — handled by document-level listener
        if (!document.pointerLockElement) {
          mouse_move(x, y);
        }
        break;
      case "down":
        // Check if tapped sprite is an editable field BEFORE dispatching mouse_down
        // (must happen synchronously in the user gesture for mobile keyboard to show)
        {
          const spriteId = player_get_sprite_at(x, y);
          const isEditable = spriteId > 0 && is_sprite_editable_field(spriteId);
          mouse_down(x, y);
          if (isEditable) {
            // Prevent the browser from focusing the stage div (tabIndex=0)
            // after our programmatic focus — otherwise it steals focus back
            // and the mobile keyboard immediately hides.
            e.preventDefault();
            // Focus hidden input to trigger mobile virtual keyboard
            hiddenInputRef.current?.focus();
          } else {
            // Blur to dismiss mobile keyboard when tapping non-editable area
            hiddenInputRef.current?.blur();
            // Return focus to stage div for regular keyboard input
            (e.currentTarget as HTMLElement).focus();
          }
        }
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
          // Handle special keys that don't produce input events
          const special = ['Enter', 'Backspace', 'Tab', 'ArrowUp', 'ArrowDown',
                           'ArrowLeft', 'ArrowRight', 'Escape', 'Delete'];
          if (special.includes(e.key)) {
            e.preventDefault();
            if (!e.repeat) key_down(e.key, e.keyCode);
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
