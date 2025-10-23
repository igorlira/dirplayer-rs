import { useMeasure } from "@uidotdev/usehooks";
import { useCallback, useEffect, useRef, useState } from "react";
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
        style={{ transform: `scale(${scale})` }}
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
      {showControls && (
        <div className={styles.controlBar}>
          <ZoomSlider scale={scale} setScale={setScale} />
        </div>
      )}
    </div>
  );
}
