import React from 'react';
import ReactDOM from 'react-dom/client';
import { getCaseInsensitiveValue } from './utils';

import EmbedPlayer from '../../src/components/EmbedPlayer';
import VMProvider from '../../src/components/VMProvider';
import store from '../../src/store';
import { Provider as StoreProvider } from 'react-redux';

// === PATHS ===
const wasmUrl = chrome.runtime.getURL('vm-rust/pkg/vm_rust_bg.wasm');
const systemFontUrl = chrome.runtime.getURL('charmap-system.png');

// === MUTATION OBSERVER ===
const observer = new MutationObserver((mutations) => {
  for (const mutation of mutations) {
    for (const node of mutation.addedNodes) {
      if (node instanceof HTMLElement) {
        if (node.tagName === 'EMBED' && checkDirEmbed(node as HTMLEmbedElement)) {
          replaceDirEmbed(node as HTMLEmbedElement);
        } else if (node.tagName === 'OBJECT') {
          const { isDirObject, params } = checkDirObject(node as HTMLObjectElement);
          if (isDirObject) {
            replaceDirObject(node as HTMLObjectElement, params);
          }
        }
      }
    }
  }
});

replaceDirPlayerElements();
observer.observe(document.body, { childList: true, subtree: true });

// === EMBED / OBJECT HANDLERS ===
function checkDirEmbed(element: HTMLEmbedElement) {
  return element.src.endsWith('.dcr');
}

function checkDirObject(object: HTMLObjectElement) {
  const paramTags = object.getElementsByTagName('param');
  const params: Partial<Record<string, string>> = Array.from(paramTags).reduce((acc, param) => {
    const name = param.getAttribute('name') || '';
    const value = param.getAttribute('value');
    acc[name] = value;
    return acc;
  }, {});
  const src = getCaseInsensitiveValue(params, 'src');
  const classId = object.getAttribute('classid');
  return {
    isDirObject: classId?.toLowerCase() === 'clsid:166B1BCA-3F9C-11CF-8075-444553540000'.toLowerCase() || src?.endsWith('.dcr'),
    params,
  };
}

function renderWhenReady(
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
        <VMProvider systemFontPath={systemFontUrl} wasmUrl={wasmUrl}>
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

function replaceDirEmbed(element: HTMLEmbedElement) {
  const { width, height, src } = element;
  const externalParams: Record<string, string> = {};
  for (let i = 1; i <= 30; i++) {
    const swValue = element.getAttribute(`sw${i}`);
    if (swValue === null) {
      break;
    }
    externalParams[`sw${i}`] = swValue;
  }

  const newElement = document.createElement('div');
  element.replaceWith(newElement);
  renderWhenReady(newElement, width, height, src, externalParams);
}

function replaceDirObject(element: HTMLObjectElement, params: Partial<Record<string, string>>) {
  const src = getCaseInsensitiveValue(params, 'src');
  if (!src) {
    console.error('No src attribute found on object element', element);
    return;
  }
  const {width, height} = element;
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
  renderWhenReady(newElement, width, height, src, externalParams);
}

function replaceDirPlayerElements() {
  const objects = document.getElementsByTagName('object');
  for (const object of Array.from(objects)) {
    const { isDirObject, params } = checkDirObject(object);
    if (isDirObject) {
      replaceDirObject(object, params);
    }
  }

  const embeds = document.getElementsByTagName('embed');
  for (const embed of Array.from(embeds)) {
    if (checkDirEmbed(embed)) {
      replaceDirEmbed(embed);
    }
  }
}
