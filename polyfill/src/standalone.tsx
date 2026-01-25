import { initPolyfill } from './core';
import { getEmbeddedWasmUrl, getEmbeddedFontUrl } from './embedded-loader';

declare global {
  interface Window {
    DirPlayer: {
      init: () => void;
    };
  }
}

function init() {
  const config = {
    wasmUrl: getEmbeddedWasmUrl(),
    systemFontUrl: getEmbeddedFontUrl(),
  };

  // If DOM is ready, initialize immediately
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', () => {
      initPolyfill(config);
    });
  } else {
    initPolyfill(config);
  }
}

// Expose the API globally
window.DirPlayer = {
  init,
};

// Auto-initialize unless data-manual-init is present on the script tag
const currentScript = document.currentScript as HTMLScriptElement | null;
if (!currentScript?.hasAttribute('data-manual-init')) {
  init();
}
