// Isolated-world client for the main-world Ruffle bridge installed by
// `public/dirplayer-ruffle-bridge-host.js`. Provides a Promise-based
// API that mirrors the subset of the Ruffle player surface used by
// `flashPlayerManager.ts`.
//
// Why this exists: Chrome MV3 isolated worlds expose a null
// `customElements`, so Ruffle (which calls `customElements.define`
// during element registration) cannot run in the same world as
// dirplayer's content script. We register Ruffle in the main world
// instead and ferry method/property calls across via `postMessage`.

let nextRequest = 1;
const pendingRequests = new Map<
  number,
  { resolve: (v: unknown) => void; reject: (e: Error) => void }
>();

type BridgeEventHandler = (eventName: string, detail: unknown) => void;
const eventHandlers = new Map<string, Set<BridgeEventHandler>>();

window.addEventListener('message', (ev) => {
  if (ev.source !== window) return;
  const m: any = ev.data;
  if (!m) return;

  if (m.__dirplayerRuffleBridge === 'response') {
    const pending = pendingRequests.get(m.requestId);
    if (!pending) return;
    pendingRequests.delete(m.requestId);
    if (m.error) pending.reject(new Error(m.error));
    else pending.resolve(m.result);
    return;
  }

  if (m.__dirplayerRuffleBridge === 'event') {
    const handlers = eventHandlers.get(m.playerId);
    if (handlers) {
      handlers.forEach((h) => {
        try { h(m.eventName, m.detail); } catch (e) { /* ignore */ }
      });
    }
  }
});

function bridgeCall<T = unknown>(payload: Record<string, unknown>): Promise<T> {
  const requestId = nextRequest++;
  return new Promise<T>((resolve, reject) => {
    pendingRequests.set(requestId, {
      resolve: (v) => resolve(v as T),
      reject,
    });
    window.postMessage(
      { __dirplayerRuffleBridge: 'request', requestId, ...payload },
      '*',
    );
  });
}

/** True when the main-world bridge host is reachable AND has Ruffle ready. */
export async function isBridgeReady(): Promise<boolean> {
  try {
    return await bridgeCall<boolean>({ method: 'isReady' });
  } catch {
    return false;
  }
}

/**
 * Wait for the main-world bridge to report Ruffle as loaded. Polls a
 * handful of times because the bridge host script may still be parsing
 * when the first request fires.
 */
export async function waitForBridge(timeoutMs = 5000): Promise<boolean> {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    if (await isBridgeReady()) return true;
    await new Promise((r) => setTimeout(r, 50));
  }
  return false;
}

export async function bridgeCreatePlayer(): Promise<string> {
  const { playerId } = await bridgeCall<{ playerId: string }>({
    method: 'createPlayer',
  });
  return playerId;
}

export function bridgeCallMethod(
  playerId: string,
  methodName: string,
  args: unknown[] = [],
): Promise<unknown> {
  return bridgeCall({ method: 'callMethod', playerId, methodName, args });
}

export function bridgeGetProp(
  playerId: string,
  propName: string,
): Promise<unknown> {
  return bridgeCall({ method: 'getProp', playerId, propName });
}

export function bridgeSetProp(
  playerId: string,
  propName: string,
  value: unknown,
): Promise<void> {
  return bridgeCall({ method: 'setProp', playerId, propName, value });
}

export function bridgeDestroyPlayer(playerId: string): Promise<void> {
  return bridgeCall({ method: 'destroyPlayer', playerId });
}

/**
 * Find the DOM element corresponding to a bridge-managed player. The
 * main-world host stamps `data-dirplayer-bridge-id` on each player so
 * the isolated world can locate it via querySelector.
 */
export function bridgeFindElement(playerId: string): HTMLElement | null {
  return document.querySelector(
    `[data-dirplayer-bridge-id="${playerId}"]`,
  );
}

/**
 * Subscribe to events forwarded from the main-world player. Pass the
 * playerId returned from `bridgeCreatePlayer`. Returns an unsubscribe
 * function.
 */
export function bridgeOnEvent(
  playerId: string,
  handler: BridgeEventHandler,
): () => void {
  let set = eventHandlers.get(playerId);
  if (!set) {
    set = new Set();
    eventHandlers.set(playerId, set);
  }
  set.add(handler);
  return () => {
    const s = eventHandlers.get(playerId);
    if (s) {
      s.delete(handler);
      if (s.size === 0) eventHandlers.delete(playerId);
    }
  };
}

/**
 * True when this isolated world cannot use Ruffle directly because it
 * has no working CustomElementRegistry. Detected by inspecting
 * `window.customElements` — Chrome MV3 content-script isolated worlds
 * leave it null even on plain pages, while page-level main-world
 * polyfills always have it.
 *
 * Standalone (page-loaded polyfill) skips the bridge entirely; the
 * extension's content script always uses it.
 */
export function isBridgeRequired(): boolean {
  return (window as { customElements?: unknown }).customElements == null;
}
