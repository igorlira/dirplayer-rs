import React from 'react';
import ReactDOM from 'react-dom/client';

import EmbedPlayer from '../../src/components/EmbedPlayer';
import VMProvider from '../../src/components/VMProvider';
import store from '../../src/store';
import { Provider as StoreProvider } from 'react-redux';

export interface PolyfillConfig {
  wasmUrl: string;
  systemFontUrl: string;
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

function checkDirObject(object: HTMLObjectElement): { isDirObject: boolean; params: Partial<Record<string, string>> } {
  const paramTags = object.getElementsByTagName('param');
  const params: Partial<Record<string, string>> = Array.from(paramTags).reduce((acc, param) => {
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

function replaceDirObject(config: PolyfillConfig, element: HTMLObjectElement, params: Partial<Record<string, string>>) {
  const src = getCaseInsensitiveValue(params, 'src');
  if (!src) {
    console.error('No src attribute found on object element', element);
    return;
  }
  const { width, height } = element;
  const externalParams: Record<string, string> = {};
  for (let i = 1; i <= 30; i++) {
    const swValue = params[`sw${i}`];
    if (swValue === undefined) {
      break;
    }
    externalParams[`sw${i}`] = swValue;
  }

  const newElement = document.createElement('div');
  element.replaceWith(newElement);
  renderPlayer(config, newElement, width, height, src, externalParams);
}

function replaceDirPlayerElements(config: PolyfillConfig) {
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

declare global {
  interface Window {
    __DIRPLAYER_INITIALIZED__?: boolean;
  }
}

export function isPolyfillInitialized(): boolean {
  return !!window.__DIRPLAYER_INITIALIZED__;
}

export function initPolyfill(config: PolyfillConfig) {
  // Check if already initialized (e.g., by the Chrome extension)
  if (window.__DIRPLAYER_INITIALIZED__) {
    console.log('[DirPlayer] Polyfill already initialized, skipping');
    return null;
  }
  window.__DIRPLAYER_INITIALIZED__ = true;

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

  return observer;
}

export { checkDirEmbed, checkDirObject };
