import fs from 'fs';
import path from 'path';
import { deflate } from 'pako';

const VIRTUAL_MODULE_ID = 'virtual:embedded-resources';
const RESOLVED_VIRTUAL_MODULE_ID = '\0' + VIRTUAL_MODULE_ID;

/** File name of the engine next to the bundle when it is not embedded. */
export const WASM_FILE_NAME = 'dirplayer-vm.wasm';

/**
 * Vite plugin that embeds resources (WASM, images) as deflated base64 strings.
 *
 * With `wasmFile: true` (the build sets it from DIRPLAYER_WASM_FILE=1) the
 * wasm is not embedded but emitted beside the bundle as WASM_FILE_NAME, and
 * the loader fetches it from there. Embedded, the 8 MB module travels as
 * base64 of deflate data inside the script and is inflated on the main
 * thread on every page load; as its own file the server can compress it
 * better, the browser can compile it while it downloads and cache the
 * compiled code, and a page can preload it beside the script.
 */
export default function embedResources(options = {}) {
  const {
    wasmPath = 'vm-rust/pkg/vm_rust_bg.wasm',
    fontPath = 'public/charmap-system.png',
    wasmFile = false,
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
        const wasmBuffer = fs.readFileSync(path.resolve(process.cwd(), wasmPath));
        let wasmBase64 = '';
        if (wasmFile) {
          this.emitFile({ type: 'asset', fileName: WASM_FILE_NAME, source: wasmBuffer });
          console.log(`[embed-resources] WASM: ${wasmBuffer.length} bytes -> ${WASM_FILE_NAME} (not embedded)`);
        } else {
          const wasmDeflated = deflate(wasmBuffer, { level: 9 });
          wasmBase64 = Buffer.from(wasmDeflated).toString('base64');
          console.log(`[embed-resources] WASM: ${wasmBuffer.length} bytes -> ${wasmDeflated.length} bytes (deflated) -> ${wasmBase64.length} chars (base64)`);
        }

        // Read and compress font file
        const fontBuffer = fs.readFileSync(path.resolve(process.cwd(), fontPath));
        const fontDeflated = deflate(fontBuffer, { level: 9 });
        const fontBase64 = Buffer.from(fontDeflated).toString('base64');

        console.log(`[embed-resources] Font: ${fontBuffer.length} bytes -> ${fontDeflated.length} bytes (deflated) -> ${fontBase64.length} chars (base64)`);

        return `
          export const wasmBase64 = "${wasmBase64}";
          export const fontBase64 = "${fontBase64}";
        `;
      }
    },
  };
}
