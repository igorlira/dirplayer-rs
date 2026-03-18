/**
 * WebSocket-to-TCP Proxy for Director Multiuser Xtra
 * Runs both game server and multiuser server proxies
 *
 * Usage:
 *   node ws-tcp-proxy-all.js
 *
 * This creates:
 *   ws://127.0.0.1:3091 -> tcp://127.0.0.1:3090 (game server)
 *   ws://127.0.0.1:3081 -> tcp://127.0.0.1:3080 (multiuser server)
 */

const WebSocket = require('ws');
const net = require('net');
const http = require('http');

function createProxy(wsPort, tcpHost, tcpPort, name) {
  const wss = new WebSocket.Server({ port: wsPort });

  console.log(`[${name}] WebSocket ws://127.0.0.1:${wsPort} -> TCP ${tcpHost}:${tcpPort}`);

  wss.on('connection', (ws, req) => {
    const clientIp = req.socket.remoteAddress;
    console.log(`[${name}] New WS connection from ${clientIp}`);

    const tcp = net.createConnection({ host: tcpHost, port: tcpPort }, () => {
      console.log(`[${name}] Connected to TCP ${tcpHost}:${tcpPort}`);
    });

    tcp.on('data', (data) => {
      if (ws.readyState === WebSocket.OPEN) {
        ws.send(data);
      }
    });

    tcp.on('close', () => {
      console.log(`[${name}] TCP closed`);
      ws.close();
    });

    tcp.on('error', (err) => {
      console.error(`[${name}] TCP error: ${err.message}`);
      ws.close();
    });

    ws.on('message', (data) => {
      if (tcp.writable) {
        tcp.write(data);
      }
    });

    ws.on('close', () => {
      console.log(`[${name}] WS closed`);
      tcp.end();
    });

    ws.on('error', (err) => {
      console.error(`[${name}] WS error: ${err.message}`);
      tcp.end();
    });
  });

  wss.on('error', (err) => {
    console.error(`[${name}] Server error: ${err.message}`);
  });

  return wss;
}

console.log('WebSocket-to-TCP Proxy for Director');
console.log('====================================\n');

// Game server proxy
createProxy(3091, '127.0.0.1', 3090, 'Game');

// Multiuser server proxy
createProxy(3081, '127.0.0.1', 3080, 'Multiuser');

// Flash gateway server proxy (port 9000)
createProxy(9001, process.env.FLASH_GW_HOST || '127.0.0.1', 9000, 'FlashGateway');

// HTTP CORS reverse proxy for /sf/ requests (port 3002 -> localhost:80)
const httpProxy = http.createServer((clientReq, clientRes) => {
  // CORS headers
  clientRes.setHeader('Access-Control-Allow-Origin', '*');
  clientRes.setHeader('Access-Control-Allow-Methods', 'GET, POST, PUT, DELETE, OPTIONS');
  clientRes.setHeader('Access-Control-Allow-Headers', 'Content-Type, Accept, X-Requested-With, Authorization, content-type');
  clientRes.setHeader('Access-Control-Max-Age', '86400');

  console.log(`[HTTPProxy] ${clientReq.method} ${clientReq.url} Content-Type: ${clientReq.headers['content-type']} Content-Length: ${clientReq.headers['content-length']}`);

  if (clientReq.method === 'OPTIONS') {
    clientRes.writeHead(204);
    clientRes.end();
    return;
  }

  // Collect body to log size, then forward
  const bodyChunks = [];
  clientReq.on('data', (chunk) => bodyChunks.push(chunk));
  clientReq.on('end', () => {
    const body = Buffer.concat(bodyChunks);
    console.log(`[HTTPProxy] Body size: ${body.length} bytes, first 20 hex: ${body.slice(0, 20).toString('hex')}`);

    const options = {
      hostname: process.env.CORS_TARGET_HOST || '127.0.0.1',
      port: parseInt(process.env.CORS_TARGET_PORT || '80'),
      path: clientReq.url,
      method: clientReq.method,
      headers: {
        ...clientReq.headers,
        'content-length': body.length,
      },
    };
    delete options.headers.host;

    const proxyReq = http.request(options, (proxyRes) => {
      console.log(`[HTTPProxy] Response: ${proxyRes.statusCode} Content-Type: ${proxyRes.headers['content-type']}`);
      clientRes.writeHead(proxyRes.statusCode, proxyRes.headers);
      proxyRes.pipe(clientRes);
    });

    proxyReq.on('error', (err) => {
      console.error(`[HTTPProxy] Error: ${err.message}`);
      clientRes.writeHead(502);
      clientRes.end('Bad Gateway');
    });

    proxyReq.write(body);
    proxyReq.end();
  });
});

const corsTargetHost = process.env.CORS_TARGET_HOST || '127.0.0.1';
const corsTargetPort = process.env.CORS_TARGET_PORT || '80';
httpProxy.listen(3456, '0.0.0.0', () => {
  console.log(`[HTTPProxy] http://127.0.0.1:3456 -> http://${corsTargetHost}:${corsTargetPort} (CORS proxy)`);
});

console.log('\nReady for connections!\n');
