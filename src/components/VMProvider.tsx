import {
  useEffect,
  createContext,
  useReducer,
  useContext,
} from "react";
import init, { add_breakpoint, set_system_font_path, set_pfr_font_enabled } from "vm-rust";
import * as wasm from "vm-rust";
import { initVmCallbacks } from "../vm/callbacks";
import {
  JsBridgeBreakpoint,
  getXtraHostBase,
  loadDefaultXtraRegistry,
  loadExternalXtras,
  setVmModule,
  setXtraHostBase,
  setXtraRegistry,
} from "dirplayer-js-api";
import { getFullPathFromOrigin } from "../utils/path";
import { initAudioContext, initAudioBackend } from "../audio/audioInit";
import { useDispatch } from "react-redux";
import { ready } from "../store/vmSlice";
import { isElectron } from "../utils/electron";
import { initMcpServer, isMcpEnabled } from "../mcp";

interface VMProviderProps {
  children?: string | JSX.Element | JSX.Element[];
  systemFontPath?: string; // Optional override for system font path (used in extension)
  wasmUrl?: string; // Optional override for WASM URL (used in extension)
}

interface PlayerVMState {
  isLoading: boolean;
}

interface PlayerVMStateAction {
  type: "INIT_OK";
}

const defaultPlayerState: PlayerVMState = {
  isLoading: true,
};
export const VMProviderContext =
  createContext<PlayerVMState>(defaultPlayerState);

// The WASM module + global VM setup must run EXACTLY ONCE per page, even
// when several VMProvider instances mount. The polyfill renders a separate
// React root + VMProvider per movie embed; wasm-bindgen's init() re-runs
// instantiation on every call, which drops the previous run's in-flight
// async closures (e.g. the system-font load) and throws "closure invoked
// recursively or after being dropped" — leaving the system font never
// loaded. A shared module-level promise gates it: the first caller runs the
// setup and the rest await the same result.
let vmGlobalInitPromise: Promise<void> | null = null;

function ensureVmGlobalInit(wasmUrl?: string, systemFontPath?: string): Promise<void> {
  if (vmGlobalInitPromise) return vmGlobalInitPromise;
  vmGlobalInitPromise = (async () => {
    // Step 1: Initialize AudioContext (required before WASM init)
    initAudioContext();

    // Step 2: Initialize WASM and VM
    initVmCallbacks();
    if (wasmUrl) {
      await init({ module_or_path: wasmUrl });
    } else {
      await init({});
    }
    console.log("VM initialized");
    // Hand the wasm module to the xtra bridge. The bridge's lazy
    // `require('vm-rust')` fallback only works under bundlers that emit
    // CommonJS interop at runtime (CRA's webpack does); the polyfill IIFE
    // bundle and the extension content script have no `require` at runtime
    // and would otherwise throw "vm-rust module not wired" on the first
    // plugin op. Calling setVmModule explicitly works under every host.
    setVmModule(wasm);
    // Dev convenience: expose the wasm module on `window.__vm`.
    (window as any).__vm = wasm;

    // Auto-load external xtras / merge the xtra registry. Hosts that set
    // their own xtra host base (polyfill / extension) before mounting are
    // left untouched so we don't clobber their origin (which would trigger
    // CORS errors on ~/xtra-registry.json).
    if (!getXtraHostBase()) {
      setXtraHostBase(document.baseURI);
      await loadDefaultXtraRegistry();
    }
    try {
      const raw = localStorage.getItem("dirplayer_external_xtras");
      if (raw) {
        const urls = JSON.parse(raw) as string[];
        if (Array.isArray(urls) && urls.length > 0) {
          loadExternalXtras(urls)
            .then((names) =>
              console.log("[dirplayer] external xtras loaded:", names.join(", ")))
            .catch((e) =>
              console.error("[dirplayer] external xtra load failed:", e));
        }
      }
    } catch (e) {
      console.warn("[dirplayer] could not parse dirplayer_external_xtras:", e);
    }
    try {
      const rawRegistry = localStorage.getItem("dirplayer_xtra_registry");
      if (rawRegistry) {
        const map = JSON.parse(rawRegistry) as Record<string, string>;
        if (map && typeof map === "object") {
          setXtraRegistry(map);
          const keys = Object.keys(map);
          if (keys.length > 0) {
            console.log("[dirplayer] xtra registry override (localStorage):", keys.join(", "));
          }
        }
      }
    } catch (e) {
      console.warn("[dirplayer] could not parse dirplayer_xtra_registry:", e);
    }

    // Step 3: Set system font
    const fontPath = systemFontPath || getFullPathFromOrigin("charmap-system.png");
    set_system_font_path(fontPath);

    // Step 4: Restore breakpoints
    const savedBreakpoints = window.localStorage.getItem("breakpoints");
    if (savedBreakpoints) {
      const breakpoints: JsBridgeBreakpoint[] = JSON.parse(savedBreakpoints);
      for (const bp of breakpoints) {
        add_breakpoint(bp.script_name, bp.handler_name, bp.bytecode_index);
      }
    }

    // Step 5: Initialize MCP server (Electron only, opt-in)
    if (isElectron()) {
      try {
        const mcpServer = initMcpServer(wasm);
        if (isMcpEnabled()) {
          mcpServer.start();
          console.log("MCP server initialized");
        }
      } catch (err) {
        console.warn("Failed to initialize MCP server:", err);
      }
    }

    // Step 6: Restore rendering options
    const savedPfr = window.localStorage.getItem("dirplayer_pfr_enabled");
    if (savedPfr !== null) {
      set_pfr_font_enabled(savedPfr === "true");
    }
  })();
  return vmGlobalInitPromise;
}

export default function VMProvider({ children, systemFontPath, wasmUrl }: VMProviderProps) {
  const dispatch = useDispatch();
  const [vmState, send] = useReducer(
    (state: PlayerVMState, action: PlayerVMStateAction) => {
      switch (action.type) {
        case "INIT_OK":
          return {
            ...state,
            isLoading: false,
          };
      }
    },
    defaultPlayerState
  );
  useEffect(() => {
    let cancelled = false;

    // Global WASM/VM setup runs once per page (see ensureVmGlobalInit);
    // every instance just awaits it and then marks its own state ready.
    ensureVmGlobalInit(wasmUrl, systemFontPath)
      .then(() => {
        if (cancelled) return;
        // Step 7: Mark VM as ready
        send({ type: "INIT_OK" });
        dispatch(ready());
      })
      .catch((err) => {
        console.error("Failed to initialize VM:", err);
      });

    const initAudioOnUserGesture = () => {
      // Initialize audio backend on first user gesture
      initAudioBackend();
      document.removeEventListener("click", initAudioOnUserGesture);
    };

    // Setup audio initialization on first user gesture (autoplay policy)
    document.addEventListener("click", initAudioOnUserGesture, { once: true });

    return () => {
      cancelled = true;
      document.removeEventListener("click", initAudioOnUserGesture);
    };
  }, [dispatch, systemFontPath, wasmUrl]);
  return (
    <div>
      {vmState.isLoading && "Loading..."}
      {!vmState.isLoading && (
        <VMProviderContext.Provider value={vmState}>
          {children}
        </VMProviderContext.Provider>
      )}
    </div>
  );
}

export function useVMState(): PlayerVMState {
  return useContext(VMProviderContext);
}
