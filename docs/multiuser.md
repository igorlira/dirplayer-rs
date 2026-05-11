# Multiuser Xtra

DirPlayer implements Director's Multiuser Xtra (MUS Xtra) as a WebSocket bridge. Movies that use `connectToNetServer` / `sendNetMessage` / `getNetMessage` work without modification — the Xtra maps the original TCP socket protocol onto a WebSocket connection you provide.

## How it works

```
Director movie (Lingo)
      │
      │  MUS Xtra calls
      ▼
DirPlayer runtime (WASM)
      │
      │  WebSocket
      ▼
Your WebSocket proxy / server
      │
      │  TCP / original MUS protocol
      ▼
Multiuser Server (or compatible backend)
```

Because browsers cannot open raw TCP sockets, you need a WebSocket-to-TCP proxy that speaks the Multiuser Server protocol on the other side. DirPlayer handles the encoding/decoding of MUS binary frames.

## WebSocket URL construction

The WebSocket URL is assembled from:

```
<scheme>://<serverID>:<port><path>
```

- **`<scheme>`** — `ws` or `wss`, determined by:
  1. The `multiuser_websocket_ssl` external param if present (see below).
  2. The page protocol: `https:` → `wss`, `http:` → `ws`.
- **`<serverID>` / `<port>`** — from `connectToNetServer` arguments.
- **`<path>`** — the `multiuser_websocket_path` external param (default: empty).

### External params

Pass these as `<embed>` / `<object>` attributes (see [Embedding](embedding.md)):

| Param | Type | Description |
|-------|------|-------------|
| `multiuser_websocket_path` | string | Path appended to the WebSocket URL. Use this to route connections through a path-based proxy. Example: `"/multiuser"` → `ws://host:1626/multiuser`. |
| `multiuser_websocket_ssl` | `"true"` / `"1"` / `"false"` / `"0"` | Force-override the SSL detection. When empty or absent, the page protocol determines `ws` vs `wss`. |

```html
<embed
  src="game.dcr"
  type="application/x-director"
  width="640" height="480"
  data-sw-multiuser_websocket_path="/mus"
  data-sw-multiuser_websocket_ssl="true"
/>
```

Or via `<param>` tags inside `<object>`:
```html
<param name="multiuser_websocket_path" value="/mus" />
<param name="multiuser_websocket_ssl"  value="true" />
```

## Socket proxy (JavaScript API)

For more flexible URL rewriting — for example when multiple movies connect to different backends and the port-to-URL mapping can't be expressed as a simple path suffix — use the `socketProxy` option in `DirPlayer.configureFlash()`:

```html
<script src="/polyfill/dirplayer-polyfill.js" data-manual-init></script>
<script>
  window.DirPlayer.configureFlash({
    socketProxy: [
      // Any connectToNetServer to host:port is rewritten to this WebSocket URL
      { host: "game.example.com", port: 1626, proxyUrl: "wss://proxy.example.com/mus" },
      { host: "chat.example.com", port: 1627, proxyUrl: "wss://proxy.example.com/chat" }
    ]
  });
  window.DirPlayer.init();
</script>
```

When a `socketProxy` entry matches the `(host, port)` from `connectToNetServer`, its `proxyUrl` is used as the WebSocket URL verbatim, ignoring the `multiuser_websocket_path` and `multiuser_websocket_ssl` params.

If no entry matches, the URL is built from the params as described above.
