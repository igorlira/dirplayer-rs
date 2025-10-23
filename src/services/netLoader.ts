/**
 * Network loader service that handles both browser fetch and Electron local file loading
 */

import { isElectron, readLocalFile } from '../utils/electron';
import { provide_net_task_data } from 'vm-rust';

/**
 * Register a callback to intercept network requests and provide data from Electron
 * This should be called early in the app initialization
 */
export function initializeNetLoader() {
  if (!isElectron()) {
    return;
  }

  console.log('[netLoader] Initializing for Electron');

  // Listen for custom events from the WASM module requesting file data
  window.addEventListener('dirplayer:netRequest', async (event: Event) => {
    const customEvent = event as CustomEvent<{ taskId: number; url: string }>;
    const { taskId, url } = customEvent.detail;

    try {
      // Check if this is a local file:// URL
      if (url.startsWith('file://')) {
        // Extract the file path from the URL
        let filePath = url.replace('file://', '')
        if (process.platform === 'win32' && filePath.startsWith('/')) {
          filePath = filePath.slice(1);
        } else {
          filePath = filePath.replace(/^\/+/, '/');
        }

        // Read the file using Electron IPC
        const data = await readLocalFile(filePath);

        // Provide the data back to the WASM module
        provide_net_task_data(taskId, data);
      }
    } catch (error) {
      console.error(`[netLoader] Failed to load file from ${url}:`, error);
      // Provide empty data to indicate failure
      provide_net_task_data(taskId, new Uint8Array(0));
    }
  });

  console.log('[netLoader] Event listener registered successfully');
}
