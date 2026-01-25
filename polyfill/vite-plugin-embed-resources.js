import fs from 'fs';
import path from 'path';
import { deflate } from 'pako';

const VIRTUAL_MODULE_ID = 'virtual:embedded-resources';
const RESOLVED_VIRTUAL_MODULE_ID = '\0' + VIRTUAL_MODULE_ID;

/**
 * Vite plugin that embeds resources (WASM, images) as deflated base64 strings
 */
export default function embedResources(options = {}) {
  const {
    wasmPath = 'vm-rust/pkg/vm_rust_bg.wasm',
    fontPath = 'public/charmap-system.png',
  } = options;

  return {
    name: 'embed-resources',

    resolveId(id) {
      if (id === VIRTUAL_MODULE_ID) {
        return RESOLVED_VIRTUAL_MODULE_ID;
      }
    },

    load(id) {
      if (id === RESOLVED_VIRTUAL_MODULE_ID) {
        // Read and compress WASM file
        const wasmBuffer = fs.readFileSync(path.resolve(process.cwd(), wasmPath));
        const wasmDeflated = deflate(wasmBuffer, { level: 9 });
        const wasmBase64 = Buffer.from(wasmDeflated).toString('base64');

        // Read and compress font file
        const fontBuffer = fs.readFileSync(path.resolve(process.cwd(), fontPath));
        const fontDeflated = deflate(fontBuffer, { level: 9 });
        const fontBase64 = Buffer.from(fontDeflated).toString('base64');

        console.log(`[embed-resources] WASM: ${wasmBuffer.length} bytes -> ${wasmDeflated.length} bytes (deflated) -> ${wasmBase64.length} chars (base64)`);
        console.log(`[embed-resources] Font: ${fontBuffer.length} bytes -> ${fontDeflated.length} bytes (deflated) -> ${fontBase64.length} chars (base64)`);

        return `
          export const wasmBase64 = "${wasmBase64}";
          export const fontBase64 = "${fontBase64}";
        `;
      }
    },
  };
}
