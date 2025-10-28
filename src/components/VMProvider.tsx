import {
  useEffect,
  useRef,
  createContext,
  useReducer,
  useContext,
} from "react";
import init, { add_breakpoint, set_system_font_path } from "vm-rust";
import { initVmCallbacks } from "../vm/callbacks";
import { JsBridgeBreakpoint } from "dirplayer-js-api";
import { getFullPathFromOrigin } from "../utils/path";
import { initAudioContext, initAudioBackend } from "../audio/audioInit";
import { useDispatch } from "react-redux";
import { ready } from "../store/vmSlice";

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
  const isInitCalled = useRef(false);
  useEffect(() => {
    if (isInitCalled.current) return;
    isInitCalled.current = true;

    const initVM = async () => {
      try {
        // Step 1: Initialize AudioContext (required before WASM init)
        initAudioContext();

        // Step 2: Initialize WASM and VM
        initVmCallbacks();
        if (wasmUrl) {
          await init(wasmUrl);
        } else {
          await init({});
        }
        console.log("VM initialized");

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

        // Step 5: Mark VM as ready
        send({ type: "INIT_OK" });
        dispatch(ready());
      } catch (err) {
        console.error("Failed to initialize VM:", err);
      }
    };

    const initAudioOnUserGesture = () => {
      // Initialize audio backend on first user gesture
      initAudioBackend();
      document.removeEventListener("click", initAudioOnUserGesture);
    };

    // Initialize VM immediately
    initVM();

    // Setup audio initialization on first user gesture (autoplay policy)
    document.addEventListener("click", initAudioOnUserGesture, { once: true });
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
