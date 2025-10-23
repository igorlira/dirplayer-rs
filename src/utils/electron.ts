/**
 * Utility functions for Electron environment detection and IPC communication
 */

/**
 * Check if the app is running in Electron
 */
export function isElectron(): boolean {
  // Renderer process
  if (typeof window !== 'undefined' && typeof window.process === 'object' && (window.process as any).type === 'renderer') {
    return true;
  }

  // Main process
  if (typeof process !== 'undefined' && typeof process.versions === 'object' && !!(process.versions as any).electron) {
    return true;
  }

  // Detect the user agent when the `nodeIntegration` option is set to false
  if (typeof navigator === 'object' && typeof navigator.userAgent === 'string' && navigator.userAgent.indexOf('Electron') >= 0) {
    return true;
  }

  return false;
}

/**
 * Open file dialog in Electron
 * Returns the selected file path or null if cancelled
 */
export async function openFileDialog(): Promise<string | null> {
  if (!isElectron()) {
    throw new Error('openFileDialog can only be called in Electron environment');
  }

  const { ipcRenderer } = window.require('electron');
  return await ipcRenderer.invoke('dialog:openFile');
}

/**
 * Read a file from the local filesystem in Electron
 * Returns the file data as a Uint8Array
 */
export async function readLocalFile(filePath: string): Promise<Uint8Array> {
  if (!isElectron()) {
    throw new Error('readLocalFile can only be called in Electron environment');
  }

  const { ipcRenderer } = window.require('electron');
  const result = await ipcRenderer.invoke('fs:readFile', filePath);

  if (result.success) {
    return new Uint8Array(result.data);
  } else {
    throw new Error(`Failed to read file: ${result.error}`);
  }
}
