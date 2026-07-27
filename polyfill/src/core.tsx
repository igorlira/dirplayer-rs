import React from 'react';
import ReactDOM from 'react-dom/client';

import EmbedPlayer from '../../src/components/EmbedPlayer';
import VMProvider from '../../src/components/VMProvider';
import store from '../../src/store';
import { Provider as StoreProvider } from 'react-redux';
import { installShockwavePlugin } from './plugin-polyfill';
import logoUrl from '../../src/assets/logo128.png';

// Install the fake `Shockwave for Director` entry into navigator.plugins
// at module load — BEFORE any page script runs detection. Page-level
// polyfill (standalone.tsx) and the extension content script both
// import core.tsx, so this single call covers both delivery paths.
// Idempotent: a no-op when a real plugin or a previous run already
// installed the entry.
installShockwavePlugin();

export interface PolyfillConfig {
  wasmUrl: string;
  systemFontUrl: string;
  requireClickToPlay?: boolean;
}

function compareSemver(a: string, b: string): number {
  const partsA = a.split('.').map(Number);
  const partsB = b.split('.').map(Number);
  for (let i = 0; i < Math.max(partsA.length, partsB.length); i++) {
    const numA = partsA[i] || 0;
    const numB = partsB[i] || 0;
    if (numA > numB) return 1;
    if (numA < numB) return -1;
  }
  return 0;
}

function getCaseInsensitiveValue(obj: Record<string, any>, key: string): string | undefined {
  for (const k in obj) {
    if (k.toLowerCase() === key.toLowerCase()) {
      return obj[k];
    }
  }
  return undefined;
}

const DIR_EXTENSIONS = ['.dcr', '.dxr', '.dir'];
const DIR_MIME_TYPES = ['application/x-director', 'application/x-shockwave-director'];

function hasDirExtension(url: string): boolean {
  try {
    const pathname = new URL(url).pathname.toLowerCase();
    return DIR_EXTENSIONS.some(ext => pathname.endsWith(ext));
  } catch {
    const lower = url.toLowerCase().split('?')[0].split('#')[0];
    return DIR_EXTENSIONS.some(ext => lower.endsWith(ext));
  }
}

function checkDirEmbed(element: HTMLEmbedElement): boolean {
  const type = (element.getAttribute('type') || '').toLowerCase();
  if (DIR_MIME_TYPES.includes(type)) return true;
  const src = element.src || element.getAttribute('src') || '';
  return !!src && hasDirExtension(src);
}

// Attributes stored on each player mount div for conflict-UI recovery and
// cross-world render handoff (content script and page script run in separate
// JS worlds and cannot share module-level variables — the DOM is the only
// shared channel).
const ATTR_MOUNT = 'data-dirplayer-mount';
const ATTR_MOUNT_WIDTH = 'data-dirplayer-width';
const ATTR_MOUNT_HEIGHT = 'data-dirplayer-height';
const ATTR_MOUNT_SRC = 'data-dirplayer-src';
const ATTR_MOUNT_PARAMS = 'data-dirplayer-params';
const ATTR_MOUNT_GESTURES = 'data-dirplayer-gestures';

// Dispatched (bubbling, cancelable) on a mount div by the polyfill world when
// the user picks "Browser Extension" in the conflict UI. The extension world
// listens on document and renders its own player into the mount, reading the
// movie parameters from the ATTR_MOUNT_* attributes. It calls preventDefault()
// to acknowledge — DOM events and defaultPrevented cross the world boundary,
// module state does not.
const EVENT_RENDER_AS_EXTENSION = 'dirplayer-render-as-extension';

// Set in the polyfill world when a conflict with the extension is detected.
let conflictPolyfillConfig: PolyfillConfig | null = null;
// Versions of both contenders, shown on the choice buttons. Captured at
// conflict-detection time (the extension's version is still on <html> then).
let conflictVersions: { extension: string; polyfill: string } | null = null;
// The user's pick. Applies to every player on the page, including embeds that
// show up after the choice was made.
let conflictChoice: 'extension' | 'polyfill' | null = null;
// One resolver per conflict UI currently on screen; a single click resolves
// them all so a multi-embed page doesn't ask the same question repeatedly.
const pendingConflictResolvers: Array<(choice: 'extension' | 'polyfill') => void> = [];

// The Web Player auto-selects after this long with no user choice.
const CONFLICT_AUTO_SELECT_MS = 5000;

function resolveConflict(choice: 'extension' | 'polyfill') {
  // First resolution wins — a pending auto-select timer firing after the user
  // clicked must not flip the choice for later embeds.
  if (conflictChoice) return;
  conflictChoice = choice;
  console.log(`[DirPlayer] Conflict resolved: using ${choice === 'extension' ? 'browser extension' : 'web player'}`);
  for (const resolve of pendingConflictResolvers.splice(0)) {
    resolve(choice);
  }
}

function isCompactHeight(height: string): boolean {
  const h = parseFloat(height);
  return !isNaN(h) && h < 200;
}

// Renders the conflict choice UI into a Shadow DOM root so the page's CSS
// (e.g. `div { height: 100% }`) cannot affect our layout, and native event
// listeners on shadow elements work without React's synthetic event system.
function buildConflictShadowUI(
  shadow: ShadowRoot,
  height: string,
  onChooseExtension: () => void,
  onChoosePolyfill: () => void
) {
  const compact = isCompactHeight(height);
  const logoSize = compact ? 18 : 26;

  const style = document.createElement('style');
  style.textContent = `
    :host { display: block; width: 100%; height: 100%; background-color: #1a1a1a; }
    *, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }
    .root {
      width: 100%; height: 100%;
      position: relative;
      display: flex;
      align-items: center;
      justify-content: center;
      background-color: #1a1a1a;
      font-family: sans-serif;
    }
    .brand {
      position: absolute;
      top: ${compact ? '12px' : '28px'};
      left: 0; right: 0;
      display: flex;
      align-items: center;
      justify-content: center;
      gap: ${compact ? '6px' : '9px'};
    }
    .brand-logo { border-radius: ${compact ? '3px' : '5px'}; opacity: 0.5; display: block; }
    .brand-name {
      font-size: ${compact ? '11px' : '13px'};
      font-weight: 600;
      color: #505050;
      letter-spacing: 0.01em;
    }
    .content {
      display: flex;
      flex-direction: column;
      align-items: center;
      gap: ${compact ? '8px' : '16px'};
      padding: ${compact ? '8px' : '24px'};
      max-width: 100%;
      text-align: center;
    }
    .title { font-size: ${compact ? '12px' : '18px'}; font-weight: bold; color: #fff; }
    .desc { font-size: ${compact ? '10px' : '13px'}; color: #888; }
    .buttons { display: flex; gap: ${compact ? '8px' : '12px'}; }
    .btn {
      display: flex;
      flex-direction: column;
      align-items: center;
      gap: ${compact ? '2px' : '3px'};
      padding: ${compact ? '5px 12px' : '8px 20px'};
      font-size: ${compact ? '11px' : '13px'};
      font-family: sans-serif;
      font-weight: 600;
      border-radius: 6px;
      background: #2a2a2a;
      cursor: pointer;
      letter-spacing: 0.01em;
      line-height: 1;
      position: relative;
      overflow: hidden;
    }
    .btn:hover { background: #3a3a3a; }
    .btn-ext { color: #fff; border: 1px solid #404040; }
    .btn-poly { color: #f5a623; border: 1px solid #f5a623; }
    .btn-version {
      font-size: ${compact ? '8px' : '10px'};
      font-weight: 400;
      opacity: 0.55;
    }
    .auto-timer {
      position: absolute;
      left: 0; bottom: 0;
      height: 2px;
      width: 100%;
      background: #f5a623;
      animation: countdown ${CONFLICT_AUTO_SELECT_MS}ms linear forwards;
    }
    @keyframes countdown {
      from { width: 100%; }
      to { width: 0; }
    }
  `;

  const root = document.createElement('div');
  root.className = 'root';

  const brand = document.createElement('div');
  brand.className = 'brand';
  const logo = document.createElement('img');
  logo.src = logoUrl;
  logo.width = logoSize;
  logo.height = logoSize;
  logo.alt = '';
  logo.className = 'brand-logo';
  const brandName = document.createElement('span');
  brandName.className = 'brand-name';
  brandName.textContent = 'DirPlayer';
  brand.append(logo, brandName);

  const content = document.createElement('div');
  content.className = 'content';

  const title = document.createElement('div');
  title.className = 'title';
  title.textContent = compact ? 'Choose player' : 'Multiple players detected';
  content.appendChild(title);

  if (!compact) {
    const desc = document.createElement('div');
    desc.className = 'desc';
    desc.textContent = 'Both the browser extension and web polyfill are active. Choose which one to use:';
    content.appendChild(desc);
  }

  const buttons = document.createElement('div');
  buttons.className = 'buttons';

  const makeButton = (className: string, label: string, version: string | undefined) => {
    const btn = document.createElement('button');
    btn.className = `btn ${className}`;
    const labelEl = document.createElement('span');
    labelEl.textContent = label;
    btn.appendChild(labelEl);
    if (version) {
      const versionEl = document.createElement('span');
      versionEl.className = 'btn-version';
      versionEl.textContent = `v${version}`;
      btn.appendChild(versionEl);
    }
    return btn;
  };

  const extBtn = makeButton('btn-ext', 'Browser Extension', conflictVersions?.extension);
  const polyBtn = makeButton('btn-poly', 'Web Player', conflictVersions?.polyfill);

  // Web Player auto-selects when the countdown bar runs out; any click cancels.
  const timerBar = document.createElement('span');
  timerBar.className = 'auto-timer';
  polyBtn.appendChild(timerBar);
  const autoSelectTimer = window.setTimeout(onChoosePolyfill, CONFLICT_AUTO_SELECT_MS);

  extBtn.addEventListener('click', () => {
    clearTimeout(autoSelectTimer);
    onChooseExtension();
  });
  polyBtn.addEventListener('click', () => {
    clearTimeout(autoSelectTimer);
    onChoosePolyfill();
  });

  buttons.append(extBtn, polyBtn);
  content.appendChild(buttons);
  root.append(brand, content);
  shadow.append(style, root);
}

const DATA_PARAM_PREFIX = 'data-sw-';

function parseDataExternalParams(element: HTMLElement): Record<string, string> {
  const params: Record<string, string> = {};
  for (const attr of Array.from(element.attributes)) {
    if (attr.name.startsWith(DATA_PARAM_PREFIX)) {
      const name = attr.name.slice(DATA_PARAM_PREFIX.length);
      if (name) {
        params[name] = attr.value;
      }
    }
  }
  return params;
}

// Standard <embed>/HTML attributes the browser itself consumes. Everything
// else on a Director <embed> is a Shockwave plugin parameter — not just the
// sw1..sw9 / swRemote / swStretchStyle family but also the underscore-prefixed
// specials (`_runMode`, `_player`, `_frame`, …) and bare ones like bgColor /
// PlayerVersion. Real Shockwave exposes all of them via externalParamValue(),
// so forward all non-standard attributes verbatim. (data-* is handled
// separately by parseDataExternalParams.)
const EMBED_STANDARD_ATTRS = new Set([
  'src', 'width', 'height', 'type', 'pluginspage', 'name', 'align',
  'hspace', 'vspace', 'border', 'class', 'id', 'style', 'title',
  'tabindex', 'hidden', 'role',
]);

function parseEmbedExternalParams(element: HTMLEmbedElement): Record<string, string> {
  const params: Record<string, string> = {};
  for (const attr of Array.from(element.attributes)) {
    const name = attr.name;
    const lower = name.toLowerCase();
    if (EMBED_STANDARD_ATTRS.has(lower) || lower.startsWith('data-')) {
      continue;
    }
    params[name] = attr.value;
  }
  return params;
}

function parseObjectExternalParams(params: Record<string, string | null>): Record<string, string> {
  const externalParams: Record<string, string> = {};
  for (const [name, value] of Object.entries(params)) {
    if (value == null) {
      continue;
    }
    // `src` is the movie URL (handled separately by replaceDirObject); every
    // other <param> tag is a plugin parameter the movie may read via
    // externalParamValue() — forward them all, not just sw*-prefixed ones,
    // so `_runMode`, bgColor, PlayerVersion, etc. reach the movie.
    if (name.toLowerCase() === 'src') {
      continue;
    }
    externalParams[name] = value;
  }
  return externalParams;
}

function checkDirObject(object: HTMLObjectElement): { isDirObject: boolean; params: Record<string, string | null> } {
  const paramTags = object.getElementsByTagName('param');
  const params: Record<string, string | null> = Array.from(paramTags).reduce((acc, param) => {
    const name = param.getAttribute('name') || '';
    const value = param.getAttribute('value');
    acc[name] = value;
    return acc;
  }, {} as Record<string, string | null>);
  const src = getCaseInsensitiveValue(params, 'src');
  const classId = (object.getAttribute('classid') || '').toLowerCase();
  const type = (object.getAttribute('type') || '').toLowerCase();
  const DIR_CLASSIDS = [
    'clsid:166b1bca-3f9c-11cf-8075-444553540000', // Shockwave Director
    'clsid:7fd1d18d-7787-11d2-b3f7-00600832b7c6', // Director 7+
  ];
  return {
    isDirObject: DIR_CLASSIDS.includes(classId)
      || DIR_MIME_TYPES.includes(type)
      || (!!src && hasDirExtension(src)),
    params,
  };
}

function normalizeCssSize(value: string): string {
  const trimmed = value.trim();
  return /^\d+(?:\.\d+)?$/.test(trimmed) ? `${trimmed}px` : trimmed;
}

/**
 * Tell the extension's service worker that this tab is actually running a
 * player, so it can scope its CORS response-header rule to this tab.
 *
 * That rule sets `Access-Control-Allow-Origin: *`, which is only safe for the
 * un-credentialed asset GETs an emulator makes — applied browser-wide it breaks
 * any site whose own requests are credentialed (the spec forbids the wildcard
 * when credentials mode is `include`). So the worker installs it only for tabs
 * that report a mount here.
 *
 * No-ops outside the extension: `chrome.runtime` is undefined in the page-loaded
 * polyfill build, which has no such rule to begin with.
 */
function notifyPlayerMounted(): void {
  try {
    const runtime = (globalThis as { chrome?: { runtime?: { sendMessage?: (m: unknown) => unknown; id?: string } } })
      .chrome?.runtime;
    if (!runtime?.id || !runtime.sendMessage) return;
    // Fire-and-forget; a rejected promise here (no receiver) must not surface.
    void Promise.resolve(runtime.sendMessage({ type: 'dirplayer-player-mounted' }))
      .catch(() => {});
  } catch {
    /* not in an extension context */
  }
}

function _renderPlayer(
  config: PolyfillConfig,
  mount: HTMLDivElement,
  width: string,
  height: string,
  src: string,
  externalParams: Record<string, string>,
  enableGestures?: boolean
) {
  notifyPlayerMounted();
  const root = ReactDOM.createRoot(mount);
  root.render(
    <React.StrictMode>
      <StoreProvider store={store}>
        <VMProvider systemFontPath={config.systemFontUrl} wasmUrl={config.wasmUrl}>
          <EmbedPlayer
            width={width}
            height={height}
            src={src}
            externalParams={externalParams}
            requireClickToPlay={config.requireClickToPlay}
            enableGestures={enableGestures}
          />
        </VMProvider>
      </StoreProvider>
    </React.StrictMode>
  );
}

// Asks the extension world to render its player into the mount. Returns after
// dispatching; if no (current) extension acknowledged the event, falls back to
// the polyfill player so the user isn't left with a dead mount.
function renderViaExtension(
  mount: HTMLDivElement,
  width: string,
  height: string,
  src: string,
  externalParams: Record<string, string>,
  enableGestures?: boolean
) {
  const event = new Event(EVENT_RENDER_AS_EXTENSION, { bubbles: true, cancelable: true });
  const unhandled = mount.dispatchEvent(event);
  if (unhandled) {
    console.warn('[DirPlayer] Extension did not answer the render handoff (outdated extension build?) — using web player instead');
    _renderPlayer(conflictPolyfillConfig!, mount, width, height, src, externalParams, enableGestures);
  }
}

// Renders conflict UI directly into a fresh mount div (for embeds not yet
// processed by the extension when conflict was detected). Mirrors the way
// EmbedPlayer presents its error overlay: an EmbedPlayer-style root box
// (inline size, position:relative, in-flow) with an inset:0 shadow host
// filling it. The mount itself is left unstyled — styling it changes how page
// CSS lays it out (e.g. flex shells that absolutely-position the mount and
// stretch its children), which is what caused the choice UI to sit in a
// different box than the player it replaces.
function renderConflictDirectly(
  mount: HTMLDivElement,
  width: string,
  height: string,
  src: string,
  externalParams: Record<string, string>,
  enableGestures?: boolean
) {
  const w = normalizeCssSize(width);
  const h = normalizeCssSize(height);
  const uiHost = document.createElement('div');
  uiHost.style.cssText = `width:${w};height:${h};position:relative;background-color:#000;`;
  const overlayHost = document.createElement('div');
  overlayHost.style.cssText = 'position:absolute;inset:0;';
  uiHost.appendChild(overlayHost);
  mount.appendChild(uiHost);

  pendingConflictResolvers.push((choice) => {
    uiHost.remove();
    if (choice === 'extension') {
      renderViaExtension(mount, width, height, src, externalParams, enableGestures);
    } else {
      _renderPlayer(conflictPolyfillConfig!, mount, width, height, src, externalParams, enableGestures);
    }
  });

  buildConflictShadowUI(
    overlayHost.attachShadow({ mode: 'open' }),
    height,
    () => resolveConflict('extension'),
    () => resolveConflict('polyfill')
  );
}

// Injects an absolute overlay over an already-rendered extension player mount.
function injectConflictOverlay(
  mount: HTMLDivElement,
  width: string,
  height: string,
  src: string,
  externalParams: Record<string, string>,
  enableGestures?: boolean
) {
  // Save and restore position so we don't permanently change mount's layout
  // context (which can shift absolutely-positioned page siblings like sizer imgs).
  const savedPosition = mount.style.position;
  mount.style.position = 'relative';

  const w = normalizeCssSize(width);
  const h = normalizeCssSize(height);
  const uiHost = document.createElement('div');
  uiHost.style.cssText = `position:absolute;top:0;left:0;width:${w};height:${h};z-index:9999;`;
  mount.appendChild(uiHost);

  pendingConflictResolvers.push((choice) => {
    uiHost.remove();
    mount.style.position = savedPosition;
    if (choice === 'polyfill') {
      // Clear the extension's DOM from the mount, then render polyfill player.
      while (mount.firstChild) mount.removeChild(mount.firstChild);
      _renderPlayer(conflictPolyfillConfig!, mount, width, height, src, externalParams, enableGestures);
    }
    // choice === 'extension': the extension player is still rendered
    // underneath — removing the overlay reveals it.
  });

  buildConflictShadowUI(
    uiHost.attachShadow({ mode: 'open' }),
    height,
    () => resolveConflict('extension'),
    () => resolveConflict('polyfill')
  );
}

function renderPlayer(
  config: PolyfillConfig,
  mount: HTMLDivElement,
  width: string,
  height: string,
  src: string,
  externalParams: Record<string, string>,
  enableGestures?: boolean
) {
  if (conflictPolyfillConfig && !conflictChoice) {
    renderConflictDirectly(mount, width, height, src, externalParams, enableGestures);
    return;
  }
  if (conflictChoice === 'extension') {
    // The user already picked the extension — route embeds that appear after
    // the choice straight to it instead of asking again.
    renderViaExtension(mount, width, height, src, externalParams, enableGestures);
    return;
  }
  _renderPlayer(config, mount, width, height, src, externalParams, enableGestures);
}

function resolveDimensionValue(
  attrValue: string | null,
  styleValue: string,
  rectValue: number,
  fallback: string
): string {
  if (attrValue && attrValue.trim()) {
    return attrValue.trim();
  }
  if (styleValue && styleValue !== 'auto' && styleValue !== '0px') {
    return styleValue;
  }
  if (rectValue > 0) {
    return `${Math.round(rectValue)}px`;
  }
  return fallback;
}

function resolveReplacementSize(element: HTMLElement): { width: string; height: string } {
  const rect = element.getBoundingClientRect();
  const computed = window.getComputedStyle(element);
  const fallbackWidth = (element as HTMLObjectElement | HTMLEmbedElement).width || '';
  const fallbackHeight = (element as HTMLObjectElement | HTMLEmbedElement).height || '';
  return {
    width: resolveDimensionValue(element.getAttribute('width'), computed.width, rect.width, fallbackWidth),
    height: resolveDimensionValue(element.getAttribute('height'), computed.height, rect.height, fallbackHeight),
  };
}

function replaceDirEmbed(config: PolyfillConfig, element: HTMLEmbedElement) {
  let { src } = element;
  if (!src) {
    src = element.getAttribute('data-src') || '';
  }
  const externalParams: Record<string, string> = parseEmbedExternalParams(element);
  Object.assign(externalParams, parseDataExternalParams(element));

  const enableGestures = element.hasAttribute('data-enable-gestures')
    || (element.parentElement?.tagName === 'OBJECT' && element.parentElement.hasAttribute('data-enable-gestures'))
    || undefined;

  let size = resolveReplacementSize(element);
  const newElement = document.createElement('div');
  if (element.parentElement && element.parentElement.tagName === 'OBJECT') {
    // If the EMBED is inside an OBJECT, replace the OBJECT instead
    const objectElement = element.parentElement as HTMLObjectElement;
    size = resolveReplacementSize(objectElement);
    element.parentElement.replaceWith(newElement);
  } else {
    element.replaceWith(newElement);
  }
  newElement.setAttribute(ATTR_MOUNT, 'true');
  newElement.setAttribute(ATTR_MOUNT_WIDTH, size.width);
  newElement.setAttribute(ATTR_MOUNT_HEIGHT, size.height);
  newElement.setAttribute(ATTR_MOUNT_SRC, src);
  newElement.setAttribute(ATTR_MOUNT_PARAMS, JSON.stringify(externalParams));
  if (enableGestures) newElement.setAttribute(ATTR_MOUNT_GESTURES, 'true');
  renderPlayer(config, newElement, size.width, size.height, src, externalParams, enableGestures);
}

function replaceDirObject(config: PolyfillConfig, element: HTMLObjectElement, params: Record<string, string | null>) {
  const src = getCaseInsensitiveValue(params, 'src');
  if (!src) {
    console.error('No src attribute found on object element', element);
    return;
  }
  const size = resolveReplacementSize(element);
  const externalParams: Record<string, string> = parseObjectExternalParams(params);
  Object.assign(externalParams, parseDataExternalParams(element));

  const enableGestures = element.hasAttribute('data-enable-gestures')
    || getCaseInsensitiveValue(params, 'enableGestures') === 'true'
    || undefined;

  const newElement = document.createElement('div');
  element.replaceWith(newElement);
  newElement.setAttribute(ATTR_MOUNT, 'true');
  newElement.setAttribute(ATTR_MOUNT_WIDTH, size.width);
  newElement.setAttribute(ATTR_MOUNT_HEIGHT, size.height);
  newElement.setAttribute(ATTR_MOUNT_SRC, src);
  newElement.setAttribute(ATTR_MOUNT_PARAMS, JSON.stringify(externalParams));
  if (enableGestures) newElement.setAttribute(ATTR_MOUNT_GESTURES, 'true');
  renderPlayer(config, newElement, size.width, size.height, src, externalParams, enableGestures);
}

function extractNoscriptElements() {
  const noscripts = document.getElementsByTagName('noscript');
  for (const noscript of Array.from(noscripts)) {
    const parser = new DOMParser();
    const doc = parser.parseFromString(noscript.innerHTML, 'text/html');

    const objects = doc.getElementsByTagName('object');
    for (const object of Array.from(objects)) {
      const { isDirObject } = checkDirObject(object);
      if (isDirObject) {
        // Move the parsed object into the live DOM, replacing the <noscript>
        const liveObject = document.adoptNode(object);
        noscript.replaceWith(liveObject);
        return; // noscript is gone, stop iterating its contents
      }
    }

    const embeds = doc.getElementsByTagName('embed');
    for (const embed of Array.from(embeds)) {
      if (checkDirEmbed(embed)) {
        const liveEmbed = document.adoptNode(embed);
        noscript.replaceWith(liveEmbed);
        return;
      }
    }
  }
}

function replaceDirPlayerElements(config: PolyfillConfig) {
  // Extract Director elements hidden inside <noscript> tags first
  extractNoscriptElements();

  const objects = document.getElementsByTagName('object');
  for (const object of Array.from(objects)) {
    const { isDirObject, params } = checkDirObject(object);
    if (isDirObject) {
      replaceDirObject(config, object, params);
    }
  }

  const embeds = document.getElementsByTagName('embed');
  for (const embed of Array.from(embeds)) {
    if (checkDirEmbed(embed)) {
      replaceDirEmbed(config, embed);
    }
  }
}

// DOM attributes on <html> are used for cross-world coordination between the
// Chrome extension (isolated world) and the page's polyfill script (main world),
// since window globals are not shared across content script worlds.
const ATTR_VERSION = 'data-dirplayer-version';
const ATTR_SOURCE = 'data-dirplayer-source';
const ATTR_INITIALIZED = 'data-dirplayer-initialized';

export function isPolyfillInitialized(): boolean {
  return document.documentElement.hasAttribute(ATTR_INITIALIZED);
}

function stealEmbedSrc(embed: HTMLEmbedElement) {
  // Save the resolved src into data-src so replaceDirEmbed can still find it,
  // then strip the src attribute so the browser never starts the resource download.
  const resolved = embed.src;
  if (resolved && !embed.hasAttribute('data-src')) {
    embed.setAttribute('data-src', resolved);
  }
  embed.removeAttribute('src');
}

function handleAddedNode(config: PolyfillConfig, node: Node) {
  if (!(node instanceof HTMLElement)) return;
  if (node.tagName === 'EMBED' && checkDirEmbed(node as HTMLEmbedElement)) {
    stealEmbedSrc(node as HTMLEmbedElement);
    replaceDirEmbed(config, node as HTMLEmbedElement);
    return;
  }
  if (node.tagName === 'OBJECT') {
    const { isDirObject, params } = checkDirObject(node as HTMLObjectElement);
    if (isDirObject) {
      replaceDirObject(config, node as HTMLObjectElement, params);
      return;
    }
  }
  // Node is a container — scan its descendants for embeds/objects
  for (const embed of Array.from(node.getElementsByTagName('embed'))) {
    if (checkDirEmbed(embed as HTMLEmbedElement)) {
      stealEmbedSrc(embed as HTMLEmbedElement);
      replaceDirEmbed(config, embed as HTMLEmbedElement);
    }
  }
  for (const object of Array.from(node.getElementsByTagName('object'))) {
    const { isDirObject, params } = checkDirObject(object as HTMLObjectElement);
    if (isDirObject) {
      replaceDirObject(config, object as HTMLObjectElement, params);
    }
  }
}

// Registered once in the extension world so the polyfill world can hand a
// mount over to it when the user picks "Browser Extension" in the conflict UI.
let extensionRenderListenerInstalled = false;

function installExtensionRenderListener(config: PolyfillConfig) {
  if (extensionRenderListenerInstalled) return;
  extensionRenderListenerInstalled = true;
  document.addEventListener(EVENT_RENDER_AS_EXTENSION, (event) => {
    const mount = event.target;
    if (!(mount instanceof HTMLDivElement) || !mount.hasAttribute(ATTR_MOUNT)) return;
    // Acknowledge across the world boundary: the dispatcher sees
    // dispatchEvent() return false and knows the extension took the mount.
    event.preventDefault();
    const width = mount.getAttribute(ATTR_MOUNT_WIDTH) || '100%';
    const height = mount.getAttribute(ATTR_MOUNT_HEIGHT) || '100%';
    const src = mount.getAttribute(ATTR_MOUNT_SRC) || '';
    const externalParams = JSON.parse(mount.getAttribute(ATTR_MOUNT_PARAMS) || '{}') as Record<string, string>;
    const enableGestures = mount.hasAttribute(ATTR_MOUNT_GESTURES) || undefined;
    console.log('[DirPlayer] Extension rendering mount handed over by conflict UI');
    _renderPlayer(config, mount, width, height, src, externalParams, enableGestures);
  }, true);
}

export function initPolyfill(config: PolyfillConfig, version: string, source: 'extension' | 'polyfill') {
  const root = document.documentElement;

  // The extension world must be able to serve conflict handoffs regardless of
  // how ownership negotiation below turns out.
  if (source === 'extension') {
    installExtensionRenderListener(config);
  }

  // Already fully initialized — detect extension/polyfill conflict and show
  // the choice UI; anything else (two of the same source) must yield.
  if (root.hasAttribute(ATTR_INITIALIZED)) {
    if (source === 'polyfill' && root.getAttribute(ATTR_SOURCE) === 'extension') {
      console.log(`[DirPlayer] Conflict: polyfill v${version} vs extension — showing choice UI`);
      conflictPolyfillConfig = config;
      // ATTR_VERSION still holds the extension's version here — the polyfill
      // only overwrites it when it re-registers ownership below.
      conflictVersions = {
        extension: root.getAttribute(ATTR_VERSION) || '',
        polyfill: version,
      };

      // Overlay already-rendered extension players with the choice UI.
      for (const mount of Array.from(document.querySelectorAll<HTMLDivElement>(`[${ATTR_MOUNT}]`))) {
        const mw = mount.getAttribute(ATTR_MOUNT_WIDTH) || '100%';
        const mh = mount.getAttribute(ATTR_MOUNT_HEIGHT) || '100%';
        const ms = mount.getAttribute(ATTR_MOUNT_SRC) || '';
        const mp = JSON.parse(mount.getAttribute(ATTR_MOUNT_PARAMS) || '{}') as Record<string, string>;
        const mg = mount.hasAttribute(ATTR_MOUNT_GESTURES) || undefined;
        injectConflictOverlay(mount, mw, mh, ms, mp, mg);
      }

      // Kill extension ownership so its observer disconnects; fall through so
      // the polyfill's scan handles any embeds the extension hadn't reached yet.
      root.removeAttribute(ATTR_INITIALIZED);
    } else {
      console.log(`[DirPlayer] Already initialized, skipping ${source} v${version}`);
      return;
    }
  }

  const existingVersion = root.getAttribute(ATTR_VERSION);
  const existingSource = root.getAttribute(ATTR_SOURCE);

  if (existingVersion && existingSource) {
    const cmp = compareSemver(version, existingVersion);
    // New candidate wins if: higher version, or same version and source is polyfill
    const newWins = cmp > 0 || (cmp === 0 && source === 'polyfill');
    if (!newWins) {
      console.log(`[DirPlayer] ${source} v${version} deferred to ${existingSource} v${existingVersion}`);
      return;
    }
    console.log(`[DirPlayer] ${source} v${version} takes priority over ${existingSource} v${existingVersion}`);
  }

  root.setAttribute(ATTR_VERSION, version);
  root.setAttribute(ATTR_SOURCE, source);
  root.setAttribute(ATTR_INITIALIZED, 'true');
  console.log(`[DirPlayer] Initializing with ${source} v${version}`);

  // Set up the MutationObserver IMMEDIATELY — it fires as a microtask when an
  // embed/object is inserted, which is before the browser's resource loader runs
  // as a macrotask. This prevents the .dcr file from being downloaded.
  const observer = new MutationObserver((mutations) => {
    if (root.getAttribute(ATTR_SOURCE) !== source) {
      observer.disconnect();
      return;
    }
    for (const mutation of mutations) {
      for (const node of mutation.addedNodes) {
        handleAddedNode(config, node);
      }
    }
  });
  observer.observe(document.documentElement || document.body, { childList: true, subtree: true });

  // Scan elements already in the DOM. Deferred only when the DOM isn't ready yet.
  const scanExisting = () => {
    if (root.getAttribute(ATTR_SOURCE) !== source) return;
    extractNoscriptElements();
    replaceDirPlayerElements(config);
  };
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', scanExisting, { once: true });
  } else {
    scanExisting();
  }
}

export { checkDirEmbed, checkDirObject };
