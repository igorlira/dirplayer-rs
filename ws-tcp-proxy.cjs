/**
 * WebSocket-to-TCP Proxy for Director Multiuser Xtra
 *
 * This proxy allows browser-based WebSocket connections to communicate
 * with raw TCP servers (like the Woodpecker game server).
 *
 * Usage:
 *   node ws-tcp-proxy.js
 *
 * Or with custom ports:
 *   node ws-tcp-proxy.js --ws-port=3091 --tcp-port=3090
 *   node ws-tcp-proxy.js --ws-port=3081 --tcp-port=3080
 *
 * Default configuration:
 *   WebSocket port 3091 -> TCP port 3090 (game server)
 *   WebSocket port 3081 -> TCP port 3080 (multiuser server)
 */

const WebSocket = require('ws');
const net = require('net');

// Parse command line arguments
const args = process.argv.slice(2);
let wsPort = 3091;
let tcpPort = 3090;
let tcpHost = '127.0.0.1';

for (const arg of args) {
  if (arg.startsWith('--ws-port=')) {
    wsPort = parseInt(arg.split('=')[1]);
  } else if (arg.startsWith('--tcp-port=')) {
    tcpPort = parseInt(arg.split('=')[1]);
  } else if (arg.startsWith('--tcp-host=')) {
    tcpHost = arg.split('=')[1];
  }
}

const wss = new WebSocket.Server({ port: wsPort });

console.log(`WebSocket-to-TCP Proxy`);
console.log(`  WebSocket: ws://127.0.0.1:${wsPort}`);
console.log(`  TCP:       ${tcpHost}:${tcpPort}`);
console.log(`  Ready for connections...`);

wss.on('connection', (ws, req) => {
  const clientIp = req.socket.remoteAddress;
  console.log(`[WS] New connection from ${clientIp}`);

  // Create TCP connection to the game server
  const tcp = net.createConnection({ host: tcpHost, port: tcpPort }, () => {
    console.log(`[TCP] Connected to ${tcpHost}:${tcpPort}`);
  });

  // Forward TCP data to WebSocket
  tcp.on('data', (data) => {
    if (ws.readyState === WebSocket.OPEN) {
      // console.log(`[TCP->WS] ${data.length} bytes`);
      ws.send(data);
    }
  });

  tcp.on('close', () => {
    console.log(`[TCP] Connection closed`);
    ws.close();
  });

  tcp.on('error', (err) => {
    console.error(`[TCP] Error: ${err.message}`);
    ws.close();
  });

  // Forward WebSocket data to TCP
  ws.on('message', (data) => {
    // console.log(`[WS->TCP] ${data.length} bytes`);
    if (tcp.writable) {
      tcp.write(data);
    }
  });

  ws.on('close', () => {
    console.log(`[WS] Connection closed`);
    tcp.end();
  });

  ws.on('error', (err) => {
    console.error(`[WS] Error: ${err.message}`);
    tcp.end();
  });
});

wss.on('error', (err) => {
  console.error(`[Server] Error: ${err.message}`);
});

console.log(`\nTo use with dirplayer, update the external_params in mod.rs:`);
console.log(`  connection.info.port=${wsPort} (instead of ${tcpPort})`);
