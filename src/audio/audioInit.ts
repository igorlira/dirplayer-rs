import { WebAudioBackend } from "vm-rust";

declare global {
  interface Window {
    getAudioContext: () => AudioContext;
  }
}

let globalAudioContext: AudioContext | null = null;
let audioBackend: WebAudioBackend | null = null;
let isAudioInitialized = false;

/**
 * Initialize the global AudioContext.
 * This should be called on a user gesture (e.g., click) to comply with autoplay policy.
 */
export function initAudioContext(): AudioContext {
  if (!globalAudioContext) {
    globalAudioContext = new (window.AudioContext || (window as any).webkitAudioContext)();
    console.log("AudioContext created:", globalAudioContext.state);

    window.getAudioContext = () => {
      if (!globalAudioContext) throw new Error("AudioContext not initialized");
      return globalAudioContext;
    };
  }
  return globalAudioContext;
}

/**
 * Initialize the WebAudioBackend.
 * This requires WASM to be initialized first.
 * Returns true if initialization was successful.
 */
export function initAudioBackend(): boolean {
  if (isAudioInitialized) {
    return true;
  }

  try {
    // Ensure AudioContext exists
    const context = initAudioContext();

    // Create WebAudioBackend
    audioBackend = new WebAudioBackend();
    console.log("🎵 WebAudioBackend created");

    audioBackend.resume_context();

    // Resume AudioContext if needed
    if (context.state !== 'running') {
      console.log(`🎶 Resuming AudioContext from state: ${context.state}`);
      context.resume().catch(e => console.error("Failed to resume AudioContext:", e));
    }

    isAudioInitialized = true;
    return true;
  } catch (err) {
    console.error("Failed to create WebAudioBackend:", err);
    return false;
  }
}

/**
 * Setup audio initialization on first user gesture.
 * This handles the autoplay policy requirement.
 */
export function setupAudioOnUserGesture(): void {
  const initAudio = async () => {
    initAudioBackend();
    document.removeEventListener("click", initAudio);
  };

  document.addEventListener("click", initAudio, { once: true });
}

/**
 * Get the current audio initialization status.
 */
export function isAudioReady(): boolean {
  return isAudioInitialized;
}

/**
 * Get the audio backend instance (may be null if not initialized).
 */
export function getAudioBackend(): WebAudioBackend | null {
  return audioBackend;
}
