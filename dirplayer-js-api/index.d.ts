import { JsBridgeDatum, ScriptInstanceId, DatumRef, ScoreSpriteSnapshot, MemberSnapshot } from "../src/vm";

export type ICastMemberRef = [number, number]

type OnScriptErrorData = {
  message: string,
  script_member_ref: ICastMemberRef,
  handler_name: string,
}

type JsBridgeBreakpoint = {
  script_name: string,
  handler_name: string,
  bytecode_index: number,
}

type JsBridgeChunk = {
  id: string,
  fourcc: string,
  len: number,
  owner?: number,
  castLib?: number,
  memberNumber?: number,
  memberName?: string,
}

export type DebugContentBitmap = { type: 'bitmap'; width: number; height: number; data: Uint8Array };
export type DebugContentDatum = { type: 'datum'; datumRef: DatumRef; snapshot: JsBridgeDatum };
export type DebugContent = DebugContentBitmap | DebugContentDatum;

type TVmCallbacks = {
  onMovieLoaded: Function,
  onMovieLoadFailed?: (path: string, error: string) => void,
  onCastListChanged: Function,
  onCastLibNameChanged: (castLib: number, name: string) => void,
  onCastMemberListChanged: Function,
  onCastMemberChanged: (memberRef: ICastMemberRef, snapshot: MemberSnapshot) => void,
  onScoreChanged: Function,
  onFrameChanged: Function,
  onScriptError: (data: OnScriptErrorData) => void,
  onScopeListChanged: Function,
  onBreakpointListChanged: (data: JsBridgeBreakpoint[]) => void,
  onScriptErrorCleared: Function,
  onGlobalListChanged: (globals: Map<string, JsBridgeDatum>) => void,
  onDebugMessage: (message: string) => void,
  onDebugContent: (content: DebugContent) => void,
  onScheduleTimeout: (timeoutName: string, periodMs: number) => void,
  onClearTimeout: (timeoutName: string) => void,
  onClearAllTimeouts: () => void,
  onDatumSnapshot: (datumRef: DatumRef, datum: JsBridgeDatum) => void,
  onScriptInstanceSnapshot: (scriptInstanceRef: ScriptInstanceId, scriptInstance: JsBridgeDatum) => void,
  onChannelChanged: (channelNumber: number, channelData: ScoreSpriteSnapshot) => void,
  onChannelDisplayNameChanged: (channelNumber: number, displayName: string) => void,
  onExternalEvent?: (event: string) => void,
  onFlashMemberLoaded?: (spriteNum: number, castLib: number, castMember: number, swfData: Uint8Array, width: number, height: number, pausedAtStart: boolean) => void,
  onFlashMemberUnloaded?: (spriteNum: number) => void,
  onStageSizeChanged?: (width: number, height: number, center: boolean) => void,
}
declare let vmCallbacks: TVmCallbacks | undefined;

export function registerVmCallbacks(callbacks: TVmCallbacks);

/**
 * Promise that resolves the next time vm-rust fires onMovieLoaded.
 * Use after `load_movie_file(...)` to wait for the actual movie to
 * finish loading (vm-rust's `load_movie_file` returns immediately
 * after dispatching a command, NOT after the load completes).
 */
export function whenMovieLoaded(): Promise<any>;

// ── External Xtra plugin loader + JS↔WASM bridge ─────────────────────

/**
 * Fetch a plugin .wasm from `url`, instantiate it, and register the
 * xtra under its `__xtra_name()` so vm-rust will route Lingo calls
 * (e.g. `new(xtra "...")`, `the xtraList`) to it.
 *
 * Available in all four host environments (dev, polyfill, extension,
 * Electron). Each host calls this with its own URL list at startup.
 *
 * URL resolution:
 *   - "http(s)://..." (or any scheme) → used as-is
 *   - "/anything"     → resolved against document.baseURI
 *   - "anything"      → resolved against the current movie base
 *                       (set via setXtraMovieBase)
 */
export function loadExternalXtra(url: string): Promise<string>;

/**
 * Set the base URL used for movie-relative xtra resolution (the
 * "bare filename" case in loadExternalXtra). Hosts call this from
 * their movie-load path BEFORE any per-movie plugin load so a
 * `xtra("foo").wasm` request resolves to `<movieBase>/foo.wasm`.
 *
 * Pass `null` or empty to clear (e.g. on movie unload).
 */
export function setXtraMovieBase(base: string | null | undefined): void;

/**
 * Set the base URL used by the `~/...` prefix in xtra URLs. Useful
 * in the polyfill / extension cases where the host JS lives at a
 * known CDN or extension location and the registry wants to point
 * at xtras served alongside the host JS — not co-located with .dcr
 * movie files. Polyfill `standalone.tsx` calls this at init with
 * its own script base URL.
 */
export function setXtraHostBase(base: string | null | undefined): void;

/**
 * Register or merge a name→URL map for movie-driven xtra resolution.
 * Keys are normalized (lowercased, ".x32" stripped); values follow
 * the same URL resolver rules as loadExternalXtra. Repeated calls
 * merge; pass `null` to clear.
 *
 * Hosts set this at boot from their config source (dev: localStorage
 * `dirplayer_xtra_registry`; polyfill: init-script attribute;
 * extension: chrome.storage; Electron: app config).
 */
export function setXtraRegistry(map: Record<string, string> | null): void;

/** Read-only snapshot of the current registry (normalized keys). */
export function getXtraRegistry(): Record<string, string>;

/**
 * Hand the bridge the vm-rust module reference. Required for hosts
 * running in pure-ESM contexts WITHOUT CommonJS resolution (the
 * browser e2e test harness uses an importmap). Webpack/Vite hosts
 * (dev, polyfill, extension, Electron) don't need to call this — the
 * lazy `require('vm-rust')` fallback resolves it.
 *
 * Call once after `await init()` from the vm-rust module.
 */
export function setVmModule(mod: unknown): void;

/**
 * Fetch a registry JSON file and merge its entries into the registry.
 * Each host bootstrap calls this after setting its own host base so
 * each environment ships its own defaults — dev: public/, polyfill:
 * alongside bundle, extension: extension root, Electron: app
 * resources.
 *
 * `path` defaults to `~/xtra-registry.json` (resolved through
 * `setXtraHostBase`). Any form `_resolveXtraUrl` understands works —
 * `~/foo.json` (host-base relative), `/foo.json` (origin root),
 * `https://...` (absolute). Hosts that need to point elsewhere pass
 * the override explicitly (e.g. polyfill reads
 * `data-xtra-registry-url` from its <script> tag).
 *
 * Missing or malformed file is non-fatal (returns null); the
 * snake_case convention fallback (`~/<name>.wasm`) still works in
 * that case.
 */
export function loadDefaultXtraRegistry(path?: string): Promise<Record<string, string> | null>;

/**
 * Resolve the currently-loaded movie's XTRl declarations against the
 * registry and load any matched plugins not already loaded. Call this
 * AFTER `load_movie_file` (which parses the XTRl) and BEFORE `play()`
 * so Lingo sees its xtras registered.
 */
export function resolveAndLoadMovieXtras(): Promise<{
  loaded: string[];
  skipped: string[];
  failed: { name: string; url: string; error: string }[];
  missing: { filename: string; displayName: string }[];
}>;

/**
 * Load several plugin URLs in parallel. Each host calls this at boot
 * with whatever URL list its environment provides (dev: localStorage
 * key `dirplayer_external_xtras`; polyfill: init-script attribute;
 * extension: chrome.storage; Electron: app config).
 */
export function loadExternalXtras(urls: string[]): Promise<string[]>;

/**
 * Resolves once every load initiated so far has finished (success or
 * failure). Movie-loading code should await this before evaluating
 * Lingo that may reference an externally-loaded xtra.
 */
export function getExternalXtrasReady(): Promise<void>;

// The four functions below are called by vm-rust via wasm-bindgen
// externs declared in `vm-rust/src/player/xtra/external.rs`. Host
// code typically does not call them directly.

export function dispatchExternalXtraStaticHandler(
  xtraName: string,
  handler: string,
  args: Uint8Array,
): Uint8Array | undefined;

export function dispatchExternalXtraInstanceHandler(
  xtraName: string,
  instanceId: number,
  handler: string,
  args: Uint8Array,
): Uint8Array | undefined;

export function createExternalXtraInstance(
  xtraName: string,
  args: Uint8Array,
): Uint8Array | undefined;

export function destroyExternalXtraInstance(xtraName: string, instanceId: number): void;

export function externalXtraHasStaticHandler(xtraName: string, handler: string): number;

/**
 * Called BY vm-rust when Lingo hits `new(xtra "X")` for a name that
 * isn't yet registered. Resolves `name` through the registry (set via
 * `setXtraRegistry`), fetches + instantiates the wasm, then signals
 * back into vm-rust via the wasm-bindgen export
 * `complete_external_xtra_load(name, success)` so the parked bytecode
 * dispatcher can resume and retry the lookup.
 *
 * Hosts don't normally call this — vm-rust drives it. Exported as part
 * of the `vmCallbacks` surface so a host can override the default
 * registry-driven implementation if it needs custom semantics (e.g. a
 * user-confirmation prompt before loading).
 */
export function onRequestXtraLoad(name: string): void;
