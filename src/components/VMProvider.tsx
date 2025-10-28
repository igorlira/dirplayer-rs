import {
  useEffect,
  useRef,
  createContext,
  useReducer,
  useContext,
} from "react";
import init, { WebAudioBackend, add_breakpoint, set_system_font_path } from "vm-rust";
import { initVmCallbacks } from "../vm/callbacks";
import { JsBridgeBreakpoint } from "dirplayer-js-api";
import { getFullPathFromOrigin } from "../utils/path";

declare global {
  interface Window {
    getAudioContext: () => AudioContext;
  }
}

// Exposed audio backend reference
let audioBackend: WebAudioBackend | null = null;

interface VMProviderProps {
  children?: string | JSX.Element | JSX.Element[];
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

export default function VMProvider({ children }: VMProviderProps) {
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

    let globalAudioContext: AudioContext | null = null;

    const initAudio = async () => {
      if (!globalAudioContext) {
        globalAudioContext = new (window.AudioContext || (window as any).webkitAudioContext)();
        console.log("AudioContext created:", globalAudioContext.state);

        window.getAudioContext = () => {
          if (!globalAudioContext) throw new Error("AudioContext not initialized");
          return globalAudioContext;
        };
      }

      try {
        initVmCallbacks();
        await init({});
        console.log("VM initialized");

        // Initialize backend
        try {
          audioBackend = new WebAudioBackend(); // call the constructor
          console.log("🎵 WebAudioBackend created");

          audioBackend.resume_context(); 
          
          // 💡 FIX: Explicitly resume the AudioContext (must be done in the user gesture handler)
          if (globalAudioContext && globalAudioContext.state !== 'running') {
            console.log(`🎶 Resuming AudioContext from state: ${globalAudioContext.state}`);
            // Attempt to resume the context
            globalAudioContext.resume().catch(e => console.error("Failed to resume AudioContext:", e));
          }
        } catch (err) {
          console.error("Failed to create WebAudioBackend:", err);
        }

        set_system_font_path(getFullPathFromOrigin("charmap-system.png"))

        const savedBreakpoints = window.localStorage.getItem("breakpoints");
        if (savedBreakpoints) {
          const breakpoints: JsBridgeBreakpoint[] = JSON.parse(savedBreakpoints);
          for (const bp of breakpoints) {
            add_breakpoint(bp.script_name, bp.handler_name, bp.bytecode_index);
          }
        }

        send({ type: "INIT_OK" });
      } catch (err) {
        console.error("Failed to initialize VM or audio backend:", err);
      }

      document.removeEventListener("click", initAudio);
    };

    // 7️⃣ Wait for first user click to comply with autoplay policy
    document.addEventListener("click", initAudio, { once: true });
  }, []);
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
