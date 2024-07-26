import React from 'react';
import ReactDOM from 'react-dom/client';
import init, { set_system_font_path } from 'vm-rust'

import EmbedPlayer from '../../src/components/EmbedPlayer';
import { initVmCallbacks } from '../../src/vm/callbacks';
console.log('DirPlayer extension content script loaded', import.meta.url);

const wasmUrl = import.meta.url + '/../../vm-rust/pkg/vm_rust_bg.wasm';
const systemFontUrl = import.meta.url + '/../../charmap-system.png';

init(wasmUrl).then(() => {
  initVmCallbacks();
  set_system_font_path(systemFontUrl);

  console.log('Wasm loaded');
  replaceDirPlayerElements();
})

function replaceDirEmbed(element: HTMLEmbedElement) {
  const newElement = document.createElement('div');
  element.replaceWith(newElement);

  const {width, height, src} = element;

  const root = ReactDOM.createRoot(
    newElement
  );
  root.render(
    <React.StrictMode>
      <EmbedPlayer width={width} height={height} src={src} />
    </React.StrictMode>
  );
}

function replaceDirPlayerElements() {
  const embeds = document.getElementsByTagName('embed');
  console.log('Found embeds:', embeds);
  for (const embed of Array.from(embeds)) {
    console.log(embed.src);
    if (embed.src.endsWith('.dcr')) {
      replaceDirEmbed(embed);
    }
  }
}

