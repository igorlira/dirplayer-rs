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
 * Decompress deflated data and create a blob URL
 */
function createBlobUrl(deflatedBase64: string, mimeType: string): string {
  const deflatedData = base64ToUint8Array(deflatedBase64);
  const inflatedData = inflate(deflatedData);
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
    fontBlobUrl = createBlobUrl(fontBase64, 'image/png');
  }
  return fontBlobUrl;
}
