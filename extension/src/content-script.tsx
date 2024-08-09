import React from 'react';
import ReactDOM from 'react-dom/client';
import init, { set_system_font_path } from 'vm-rust'
import { getCaseInsensitiveValue } from './utils';

import EmbedPlayer from '../../src/components/EmbedPlayer';
import { initVmCallbacks } from '../../src/vm/callbacks';
import store from '../../src/store';
import { ready } from '../../src/store/vmSlice';
import { Provider as StoreProvider } from 'react-redux';

const wasmUrl = chrome.runtime.getURL('vm-rust/pkg/vm_rust_bg.wasm');
const systemFontUrl = chrome.runtime.getURL('charmap-system.png');

const observer = new MutationObserver((mutations) => {
  mutations.forEach((mutation) => {
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
  });
});

replaceDirPlayerElements();
observer.observe(document.body, {
  childList: true,
  subtree: true,
});

init(wasmUrl).then(() => {
  initVmCallbacks();
  set_system_font_path(systemFontUrl);

  console.log('Wasm loaded');
  store.dispatch(ready());
})

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

function replaceDirEmbed(element: HTMLEmbedElement) {
  const {width, height, src} = element;
  const externalParams = {};
  for (let i = 1; i <= 30; i++) {
    const swValue = element.getAttribute(`sw${i}`);
    if (swValue === null) {
      break;
    }
    externalParams[`sw${i}`] = swValue;
  }

  console.log('External params:', externalParams);
  
  const newElement = document.createElement('div');
  element.replaceWith(newElement);

  const root = ReactDOM.createRoot(
    newElement
  );
  root.render(
    <React.StrictMode>
      <StoreProvider store={store}>
        <EmbedPlayer width={width} height={height} src={src} externalParams={externalParams} />
      </StoreProvider>
    </React.StrictMode>
  );
}

function replaceDirObject(element: HTMLObjectElement, params: Partial<Record<string, string>>) {
  const src = getCaseInsensitiveValue(params, 'src');
  if (!src) {
    console.error('No src attribute found on object element', element);
    return;
  }
  const {width, height} = element;
  const externalParams = {};
  for (let i = 1; i <= 30; i++) {
    const swValue = params[`sw${i}`];
    if (swValue === undefined) {
      break;
    }
    externalParams[`sw${i}`] = swValue;
  }

  console.log('Params:', params);
  console.log('External params:', externalParams);
  
  const newElement = document.createElement('div');
  element.replaceWith(newElement);

  const root = ReactDOM.createRoot(
    newElement
  );
  root.render(
    <React.StrictMode>
      <StoreProvider store={store}>
        <EmbedPlayer width={width} height={height} src={src} externalParams={externalParams} />
      </StoreProvider>
    </React.StrictMode>
  );
}

function replaceDirPlayerElements() {
  const objects = document.getElementsByTagName('object');
  if (objects.length > 0) {
    console.log(`Found ${objects.length} objects`);
    for (const object of Array.from(objects)) {
      const { isDirObject, params } = checkDirObject(object);
      if (isDirObject) {
        replaceDirObject(object, params);
      }
    }
  }

  const embeds = document.getElementsByTagName('embed');
  if (embeds.length > 0) {
    console.log(`Found ${embeds.length} embeds`);
    for (const embed of Array.from(embeds)) {
      if (checkDirEmbed(embed)) {
        replaceDirEmbed(embed);
      }
    }
  }
}

