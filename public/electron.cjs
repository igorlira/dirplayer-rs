const electron = require('electron');
const app = electron.app;
const BrowserWindow = electron.BrowserWindow;
const ipcMain = electron.ipcMain;
const dialog = electron.dialog;

const path = require('path');
const fs = require('fs');
const isDev = require('electron-is-dev').default;
const http = require('http');

let mainWindow;
let mcpServer = null;
let pendingRequests = new Map(); // requestId -> response object
let requestIdCounter = 0;

function createWindow() {
  mainWindow = new BrowserWindow({
    width: 900,
    height: 680,
    webPreferences: {
      nodeIntegration: true,
      contextIsolation: false
    }
  });
  mainWindow.loadURL(isDev ? 'http://localhost:3000' : `file://${path.join(__dirname, '../build/index.html')}`);
  mainWindow.on('closed', () => mainWindow = null);
}

app.on('ready', createWindow);

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') {
    app.quit();
  }
});

app.on('activate', () => {
  if (mainWindow === null) {
    createWindow();
  }
});

// IPC handlers for file operations
ipcMain.handle('dialog:openFile', async () => {
  const result = await dialog.showOpenDialog(mainWindow, {
    properties: ['openFile'],
    filters: [
      { name: 'Director Movies', extensions: ['dir', 'dxr', 'dcr'] },
      { name: 'All Files', extensions: ['*'] }
    ]
  });

  if (!result.canceled && result.filePaths.length > 0) {
    return result.filePaths[0];
  }
  return null;
});

// Read file from local filesystem
ipcMain.handle('fs:readFile', async (_event, filePath) => {
  try {
    const data = fs.readFileSync(filePath);
    return { success: true, data: Array.from(data) };
  } catch (error) {
    return { success: false, error: error.message };
  }
});

// ============================================================================
// MCP HTTP Server for VM debugging
// ============================================================================

function startMcpServer(port) {
  if (mcpServer) {
    console.log('MCP server already running');
    return;
  }

  try {
    mcpServer = http.createServer((req, res) => {
      // Set CORS headers
      res.setHeader('Access-Control-Allow-Origin', '*');
      res.setHeader('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
      res.setHeader('Access-Control-Allow-Headers', 'Content-Type');

      if (req.method === 'OPTIONS') {
        res.writeHead(200);
        res.end();
        return;
      }

      if (req.method === 'POST') {
        let body = '';
        req.on('data', chunk => {
          body += chunk.toString();
        });

        req.on('end', () => {
          try {
            const request = JSON.parse(body);
            const requestId = `req_${++requestIdCounter}`;

            // Store the response object to send the response later
            pendingRequests.set(requestId, res);

            // Forward request to renderer process
            if (mainWindow && !mainWindow.isDestroyed()) {
              mainWindow.webContents.send('mcp:request', { requestId, request });
            } else {
              res.writeHead(503, { 'Content-Type': 'application/json' });
              res.end(JSON.stringify({
                jsonrpc: '2.0',
                id: request.id || null,
                error: { code: -32603, message: 'VM not available' }
              }));
              pendingRequests.delete(requestId);
            }

            // Timeout after 30 seconds
            setTimeout(() => {
              if (pendingRequests.has(requestId)) {
                const pendingRes = pendingRequests.get(requestId);
                pendingRes.writeHead(504, { 'Content-Type': 'application/json' });
                pendingRes.end(JSON.stringify({
                  jsonrpc: '2.0',
                  id: request.id || null,
                  error: { code: -32603, message: 'Request timeout' }
                }));
                pendingRequests.delete(requestId);
              }
            }, 30000);

          } catch (error) {
            console.error('Error parsing MCP request:', error);
            res.writeHead(400, { 'Content-Type': 'application/json' });
            res.end(JSON.stringify({
              jsonrpc: '2.0',
              id: null,
              error: { code: -32700, message: 'Parse error' }
            }));
          }
        });
      } else {
        // GET request - return server info
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          name: 'dirplayer-vm',
          version: '1.0.0',
          protocolVersion: '2024-11-05'
        }));
      }
    });

    mcpServer.listen(port, () => {
      console.log(`MCP server listening on http://localhost:${port}`);
    });

    mcpServer.on('error', (error) => {
      console.error('MCP server error:', error);
      mcpServer = null;
    });

  } catch (error) {
    console.error('Failed to start MCP server:', error);
  }
}

function stopMcpServer() {
  if (mcpServer) {
    mcpServer.close();
    mcpServer = null;
    pendingRequests.clear();
    console.log('MCP server stopped');
  }
}

// IPC handlers for MCP server
ipcMain.on('mcp:start-server', (_event, { port }) => {
  startMcpServer(port);
});

ipcMain.on('mcp:stop-server', () => {
  stopMcpServer();
});

ipcMain.on('mcp:response', (_event, { requestId, response }) => {
  const res = pendingRequests.get(requestId);
  if (res) {
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify(response));
    pendingRequests.delete(requestId);
  }
});

// Clean up MCP server on app quit
app.on('before-quit', () => {
  stopMcpServer();
});
