import {
  useEffect,
  useRef,
  createContext,
  useReducer,
  useContext,
} from "react";
import init from "vm-rust";
import { initVmCallbacks } from "../vm/callbacks";
import { JsBridgeBreakpoint } from "dirplayer-js-api";
import { add_breakpoint, set_system_font_path } from 'vm-rust'
import { getFullPathFromOrigin } from "../utils/path";

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
    if (!isInitCalled.current) {
      isInitCalled.current = true;
      console.log("Initializing VM");

      initVmCallbacks();
      init().then((vm: Object) => {
        console.log("VM initialized", vm);
        send({ type: "INIT_OK" });

        set_system_font_path(getFullPathFromOrigin("charmap-system.png"))

        const savedBreakpoints = window.localStorage.getItem("breakpoints");
        if (savedBreakpoints) {
          const breakpoints: JsBridgeBreakpoint[] = JSON.parse(savedBreakpoints);
          for (const bp of breakpoints) {
            add_breakpoint(bp.script_name, bp.handler_name, bp.bytecode_index);
          }
        }
      });
    }
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
