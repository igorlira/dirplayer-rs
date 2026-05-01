import React from 'react';
import ReactDOM from 'react-dom/client';

import EmbedPlayer from '../../src/components/EmbedPlayer';
import VMProvider from '../../src/components/VMProvider';
import store from '../../src/store';
import { Provider as StoreProvider } from 'react-redux';

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

function checkDirEmbed(element: HTMLEmbedElement): boolean {
  return element.src.endsWith('.dcr');
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
  const classId = object.getAttribute('classid');
  return {
    isDirObject: classId?.toLowerCase() === 'clsid:166B1BCA-3F9C-11CF-8075-444553540000'.toLowerCase() || !!src?.endsWith('.dcr'),
    params,
  };
}

function renderPlayer(
  config: PolyfillConfig,
  mount: HTMLDivElement,
  width: string,
  height: string,
  src: string,
  externalParams: Record<string, string>
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
          />
        </VMProvider>
      </StoreProvider>
    </React.StrictMode>
  );
}

function replaceDirEmbed(config: PolyfillConfig, element: HTMLEmbedElement) {
  let { width, height, src } = element;
  if (!src) {
    src = element.getAttribute('data-src') || '';
  }
  const externalParams: Record<string, string> = {};
  for (let i = 1; i <= 30; i++) {
    const swValue = element.getAttribute(`sw${i}`);
    if (swValue === null) {
      break;
    }
    externalParams[`sw${i}`] = swValue;
  }

  const newElement = document.createElement('div');
  if (element.parentElement && element.parentElement.tagName === 'OBJECT') {
    // If the EMBED is inside an OBJECT, replace the OBJECT instead
    const objectElement = element.parentElement as HTMLObjectElement;
    if (objectElement.width) {
      width = objectElement.width;
    }
    if (objectElement.height) {
      height = objectElement.height;
    }
    element.parentElement.replaceWith(newElement);
  } else {
    element.replaceWith(newElement);
  }
  renderPlayer(config, newElement, width, height, src, externalParams);
}

function replaceDirObject(config: PolyfillConfig, element: HTMLObjectElement, params: Record<string, string | null>) {
  const src = getCaseInsensitiveValue(params, 'src');
  if (!src) {
    console.error('No src attribute found on object element', element);
    return;
  }
  const { width, height } = element;
  const externalParams: Record<string, string> = {};
  for (let i = 1; i <= 30; i++) {
    const swValue = params[`sw${i}`];
    if (swValue === undefined || swValue === null) {
      break;
    }
    externalParams[`sw${i}`] = swValue;
  }

  const newElement = document.createElement('div');
  element.replaceWith(newElement);
  renderPlayer(config, newElement, width, height, src, externalParams);
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

function performInit(config: PolyfillConfig, source: string, version: string) {
  const root = document.documentElement;

  // Re-check at execution time: am I still the winner?
  if (root.getAttribute(ATTR_SOURCE) !== source) return;
  if (root.hasAttribute(ATTR_INITIALIZED)) return;

  root.setAttribute(ATTR_INITIALIZED, 'true');
  console.log(`[DirPlayer] Initializing with ${source} v${version}`);

  // Replace existing elements
  replaceDirPlayerElements(config);

  // Observe for new elements
  const observer = new MutationObserver((mutations) => {
    for (const mutation of mutations) {
      for (const node of mutation.addedNodes) {
        if (node instanceof HTMLElement) {
          if (node.tagName === 'EMBED' && checkDirEmbed(node as HTMLEmbedElement)) {
            replaceDirEmbed(config, node as HTMLEmbedElement);
          } else if (node.tagName === 'OBJECT') {
            const { isDirObject, params } = checkDirObject(node as HTMLObjectElement);
            if (isDirObject) {
              replaceDirObject(config, node as HTMLObjectElement, params);
            }
          }
        }
      }
    }
  });

  observer.observe(document.documentElement || document.body, { childList: true, subtree: true });
}

export function initPolyfill(config: PolyfillConfig, version: string, source: 'extension' | 'polyfill') {
  const root = document.documentElement;

  // Already fully initialized, too late to compete
  if (root.hasAttribute(ATTR_INITIALIZED)) {
    console.log(`[DirPlayer] Already initialized, skipping ${source} v${version}`);
    return;
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

  // Register as the current candidate via DOM attributes (visible across worlds)
  root.setAttribute(ATTR_VERSION, version);
  root.setAttribute(ATTR_SOURCE, source);

  // Schedule deferred initialization
  const doInit = () => performInit(config, source, version);
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', doInit, { once: true });
  } else {
    setTimeout(doInit, 0);
  }
}

export { checkDirEmbed, checkDirObject };
