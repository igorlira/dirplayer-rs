import { useMeasure } from "@uidotdev/usehooks";
import { useCallback, useEffect, useRef } from "react";
import {
  set_stage_size,
  player_create_canvas,
  mouse_move,
  mouse_down,
  mouse_up,
  key_down,
  key_up,
} from "vm-rust";

import styles from "./styles.module.css";

type MouseEventName = "move" | "down" | "up";
function onMouseEvent(name: MouseEventName, e: React.MouseEvent) {
  const rect = e.currentTarget.getBoundingClientRect();
  const x = e.clientX - rect.left;
  const y = e.clientY - rect.top;
  
  switch (name) {
    case "move":
      mouse_move(x, y);
      break;
    case "down":
      mouse_down(x, y);
      break;
    case "up":
      mouse_up(x, y);
      break;
  }
}

export default function Stage() {
  const [ref, { width, height }] = useMeasure();
  const isStageCanvasCreated = useRef(false);
  const containerRef = useRef<HTMLDivElement | null>(null);

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

  return (
    <div className={styles.container} ref={onContainerRef}>
      <div
        tabIndex={0}
        id="stage_canvas_container"
        onPointerMove={(e) => onMouseEvent('move', e)}
        onPointerDown={(e) => onMouseEvent('down', e)}
        onPointerUp={(e) => onMouseEvent('up', e)}
        onKeyDown={e => {
          e.preventDefault();
          key_down(e.key, e.keyCode)
        }}
        onKeyUp={e => key_up(e.key, e.keyCode)}
      ></div>
    </div>
  );
}
