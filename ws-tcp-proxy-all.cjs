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

console.log('\nReady for connections!\n');
