import { useState } from "react";
import {
  get_renderer_backend,
  set_renderer_backend,
  is_webgl2_supported,
  get_pfr_font_enabled,
  set_pfr_font_enabled,
  get_stage_scale_snap_integer,
  set_stage_scale_snap_integer,
} from "vm-rust";
import styles from "./styles.module.css";

const STORAGE_KEY_BACKEND = "dirplayer_renderer_backend";
const STORAGE_KEY_PFR = "dirplayer_pfr_enabled";
const STORAGE_KEY_SNAP = "dirplayer_stage_scale_snap_integer";

export default function RenderingOptions() {
  const [backend, setBackend] = useState(() => get_renderer_backend());
  const [pfrEnabled, setPfrEnabled] = useState(() => get_pfr_font_enabled());
  const [snapInteger, setSnapInteger] = useState(() =>
    get_stage_scale_snap_integer(),
  );
  const webgl2Supported = is_webgl2_supported();

  const handleBackendChange = (e: React.ChangeEvent<HTMLSelectElement>) => {
    const value = e.target.value;
    try {
      set_renderer_backend(value);
      const actual = get_renderer_backend();
      setBackend(actual);
      localStorage.setItem(STORAGE_KEY_BACKEND, actual);
    } catch (err) {
      console.error("Failed to switch renderer:", err);
    }
  };

  const handlePfrToggle = (e: React.ChangeEvent<HTMLInputElement>) => {
    const enabled = e.target.checked;
    set_pfr_font_enabled(enabled);
    setPfrEnabled(enabled);
    localStorage.setItem(STORAGE_KEY_PFR, String(enabled));
  };

  const handleSnapToggle = (e: React.ChangeEvent<HTMLInputElement>) => {
    const enabled = e.target.checked;
    set_stage_scale_snap_integer(enabled);
    setSnapInteger(enabled);
    localStorage.setItem(STORAGE_KEY_SNAP, String(enabled));
  };

  return (
    <div className={styles.optionsContainer}>
      <div className={styles.optionRow}>
        <label htmlFor="renderer-backend">Renderer</label>
        <select
          id="renderer-backend"
          value={backend}
          onChange={handleBackendChange}
        >
          <option value="Canvas2D">Canvas 2D</option>
          <option value="WebGL2" disabled={!webgl2Supported}>
            WebGL2{!webgl2Supported ? " (not supported)" : ""}
          </option>
        </select>
      </div>
      <div className={styles.optionRow}>
        <label htmlFor="pfr-font-toggle">PFR font rendering</label>
        <input
          id="pfr-font-toggle"
          type="checkbox"
          checked={pfrEnabled}
          onChange={handlePfrToggle}
        />
      </div>
      {/* Only has an effect while the stage is SCALED (fullscreen, or an
          explicit swStretchStyle) — at 1:1 there is nothing to snap. */}
      <div className={styles.optionRow}>
        <label htmlFor="stage-scale-snap-toggle" title="Scale the stage by a whole number (2x, 3x) instead of the exact aspect fit. Magnified art, 3D overlays and the cursor resample evenly; the letterbox gets bigger.">
          Integer stage scale
        </label>
        <input
          id="stage-scale-snap-toggle"
          type="checkbox"
          checked={snapInteger}
          onChange={handleSnapToggle}
        />
      </div>
    </div>
  );
}
