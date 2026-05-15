import React from 'react';
import ReactDOM from 'react-dom/client';

import EmbedPlayer from '../../src/components/EmbedPlayer';
import VMProvider from '../../src/components/VMProvider';
import store from '../../src/store';
import { Provider as StoreProvider } from 'react-redux';
import { installShockwavePlugin } from './plugin-polyfill';

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

function parseEmbedExternalParams(element: HTMLEmbedElement): Record<string, string> {
  const params: Record<string, string> = {};
  for (const attr of Array.from(element.attributes)) {
    const name = attr.name;
    if (/^sw\d+$/i.test(name) || /^sw[a-z]/i.test(name)) {
      params[name] = attr.value;
    }
  }
  return params;
}

function parseObjectExternalParams(params: Record<string, string | null>): Record<string, string> {
  const externalParams: Record<string, string> = {};
  for (const [name, value] of Object.entries(params)) {
    if (value == null) {
      continue;
    }
    if (/^sw\d+$/i.test(name) || /^sw[a-z]/i.test(name)) {
      externalParams[name] = value;
    }
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

function renderPlayer(
  config: PolyfillConfig,
  mount: HTMLDivElement,
  width: string,
  height: string,
  src: string,
  externalParams: Record<string, string>,
  enableGestures?: boolean
) {
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

export function initPolyfill(config: PolyfillConfig, version: string, source: 'extension' | 'polyfill') {
  const root = document.documentElement;

  // Already fully initialized — polyfill can always take over from the extension;
  // anything else (e.g. extension arriving after polyfill) must yield.
  if (root.hasAttribute(ATTR_INITIALIZED)) {
    if (source === 'polyfill' && root.getAttribute(ATTR_SOURCE) === 'extension') {
      // Force takeover: clear the initialized flag and re-register as polyfill.
      // The extension's MutationObserver will detect the source change and disconnect.
      console.log(`[DirPlayer] Polyfill v${version} taking over from extension`);
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
