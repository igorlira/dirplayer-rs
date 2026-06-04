/// <reference path="./virtual-modules.d.ts" />
import { inflate } from 'pako';
import { wasmBase64, fontBase64 } from 'virtual:embedded-resources';

/**
 * Decode base64 string to Uint8Array
 */
function base64ToUint8Array(base64: string): Uint8Array {
  const binaryString = atob(base64);
  const bytes = new Uint8Array(binaryString.length);
  for (let i = 0; i < binaryString.length; i++) {
    bytes[i] = binaryString.charCodeAt(i);
  }
  return bytes;
}

/**
 * Decompress deflated data and create a blob URL.
 *
 * Throws on empty/invalid embedded data rather than silently producing a
 * blob whose bytes can't be decoded — a bad embed otherwise surfaces far
 * away as an opaque `createImageBitmap`/WASM panic (the system-font blob
 * fetch resolves OK, then decoding rejects). Failing here points straight
 * at the build step that produced the embed.
 */
function createBlobUrl(deflatedBase64: string, mimeType: string): string {
  if (!deflatedBase64) {
    throw new Error(
      `[DirPlayer] Embedded ${mimeType} resource is empty — the embed-resources ` +
        `build plugin did not run or the source asset was missing.`,
    );
  }
  const deflatedData = base64ToUint8Array(deflatedBase64);
  const inflatedData = inflate(deflatedData);
  if (!inflatedData || inflatedData.length === 0) {
    throw new Error(`[DirPlayer] Embedded ${mimeType} resource inflated to 0 bytes.`);
  }
  // Sanity-check PNG magic for image embeds so a corrupt font fails loudly.
  if (mimeType === 'image/png') {
    const PNG_MAGIC = [0x89, 0x50, 0x4e, 0x47];
    const ok = PNG_MAGIC.every((b, i) => inflatedData[i] === b);
    if (!ok) {
      throw new Error(
        `[DirPlayer] Embedded PNG resource is not a valid PNG (bad magic bytes).`,
      );
    }
  }
  const blob = new Blob([inflatedData], { type: mimeType });
  return URL.createObjectURL(blob);
}

// Lazily create blob URLs on first access
let wasmBlobUrl: string | null = null;
let fontBlobUrl: string | null = null;

export function getEmbeddedWasmUrl(): string {
  if (!wasmBlobUrl) {
    wasmBlobUrl = createBlobUrl(wasmBase64, 'application/wasm');
  }
  return wasmBlobUrl;
}

export function getEmbeddedFontUrl(): string {
  if (!fontBlobUrl) {
    try {
      fontBlobUrl = createBlobUrl(fontBase64, 'image/png');
    } catch (e) {
      // The system font is non-fatal: returning '' lets VMProvider fall back
      // to its static `charmap-system.png` path, and the WASM side now
      // degrades gracefully if even that is missing.
      console.warn('[DirPlayer] Failed to build embedded system-font blob:', e);
      return '';
    }
  }
  return fontBlobUrl;
}
