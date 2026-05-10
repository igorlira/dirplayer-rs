/**
 * Network loader service that handles both browser fetch and Electron local file loading
 */

import { isElectron, readLocalFile, appendLocalFile } from '../utils/electron';
import { provide_net_task_data, provide_net_task_error } from 'vm-rust';

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
        let filePath = decodeURIComponent(url.replace('file://', ''))
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
      provide_net_task_error(taskId);
    }
  });

  // Listen for file write events (traceLogFile, FileIO writes)
  window.addEventListener('dirplayer:fileWrite', (event: Event) => {
    const customEvent = event as CustomEvent<{ filePath: string; content: string; append: boolean }>;
    const { filePath, content, append } = customEvent.detail;
    if (append) {
      appendLocalFile(filePath, content);
    }
  });

  console.log('[netLoader] Event listener registered successfully');
}
