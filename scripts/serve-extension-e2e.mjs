#!/usr/bin/env node
/**
 * HTTP fixture for extension CORS e2e.
 *
 * Listens on 127.0.0.1 but is reached as player.dirplayer.test / cdn.dirplayer.test
 * via Chromium --host-resolver-rules. No Access-Control-Allow-Origin on the CDN
 * host: a page-privileged fetch is CORS-blocked, so success means the SW relay ran.
 */
import http from "node:http";
import { logE2e } from "./prepare-extension-e2e.mjs";
import {
  PORT,
  PLAYER_HOST,
  CDN_ORIGIN,
  FAST_PATH,
  SLOW_PATH,
  FAST_PAYLOAD,
  SLOW_PAYLOAD,
  SLOW_CHUNK_COUNT,
  SLOW_CHUNK_DELAY_MS,
  SHAPES_PAYLOAD,
  SHAPES_FAST_PATH,
  SHAPES_SLOW_PATH,
  EMBED_FAST_PATH,
  EMBED_SLOW_PATH,
} from "../tests/extension-e2e/payloads.mjs";

const HOST_PAGE = `<!doctype html>
<html>
  <head><meta charset="utf-8"><title>DirPlayer extension CORS e2e</title></head>
  <body><p>extension cors e2e host</p></body>
</html>
`;

function embedPage(moviePath) {
  const src = `${CDN_ORIGIN}${moviePath}`;
  return `<!doctype html>
<html>
  <head><meta charset="utf-8"><title>DirPlayer extension movie e2e</title></head>
  <body>
    <embed src="${src}" type="application/x-director" width="640" height="480">
  </body>
</html>
`;
}

function hostnameOf(req) {
  const host = req.headers.host || "";
  return host.split(":")[0].toLowerCase();
}

function pathnameOf(req) {
  try {
    return new URL(req.url || "/", "http://127.0.0.1").pathname;
  } catch {
    return "/";
  }
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function sendHtml(res, html) {
  res.writeHead(200, {
    "Content-Type": "text/html; charset=utf-8",
    "Cache-Control": "no-store",
  });
  res.end(html);
}

function sendFast(res, body) {
  res.writeHead(200, {
    "Content-Type": "application/x-director",
    "Content-Length": String(body.length),
    "Cache-Control": "no-store",
  });
  res.end(body);
}

async function sendSlow(res, body) {
  res.writeHead(200, {
    "Content-Type": "application/x-director",
    "Content-Length": String(body.length),
    "Cache-Control": "no-store",
  });
  res.flushHeaders();
  await delay(SLOW_CHUNK_DELAY_MS);
  const chunkSize = Math.ceil(body.length / SLOW_CHUNK_COUNT);
  for (let i = 0; i < SLOW_CHUNK_COUNT; i++) {
    const start = i * chunkSize;
    const end = Math.min(start + chunkSize, body.length);
    if (start >= body.length) break;
    res.write(body.subarray(start, end));
    if (end < body.length) await delay(SLOW_CHUNK_DELAY_MS);
  }
  res.end();
}

function sendMissingMovie(res) {
  res.writeHead(404, { "Content-Type": "text/plain" });
  res.end("Missing corpus movie public/dcr_dirplayer_test_movies/D8_5_00001_shapes_1.dcr");
}

const server = http.createServer((req, res) => {
  const path = pathnameOf(req);
  const host = hostnameOf(req);
  const onPlayer = host === PLAYER_HOST || host === "127.0.0.1" || host === "localhost";

  if (onPlayer && (path === "/" || path === "/index.html")) {
    sendHtml(res, HOST_PAGE);
    return;
  }

  if (onPlayer && path === EMBED_FAST_PATH) {
    sendHtml(res, embedPage(SHAPES_FAST_PATH));
    return;
  }

  if (onPlayer && path === EMBED_SLOW_PATH) {
    sendHtml(res, embedPage(SHAPES_SLOW_PATH));
    return;
  }

  if (path === FAST_PATH) {
    sendFast(res, FAST_PAYLOAD);
    return;
  }

  if (path === SLOW_PATH) {
    void sendSlow(res, SLOW_PAYLOAD);
    return;
  }

  if (path === SHAPES_FAST_PATH) {
    if (!SHAPES_PAYLOAD) {
      sendMissingMovie(res);
      return;
    }
    sendFast(res, SHAPES_PAYLOAD);
    return;
  }

  if (path === SHAPES_SLOW_PATH) {
    if (!SHAPES_PAYLOAD) {
      sendMissingMovie(res);
      return;
    }
    void sendSlow(res, SHAPES_PAYLOAD);
    return;
  }

  res.writeHead(404, { "Content-Type": "text/plain" });
  res.end("Not found");
});

server.listen(PORT, "127.0.0.1", () => {
  logE2e(`Fixture HTTP server listening on http://127.0.0.1:${PORT}`);
  if (!SHAPES_PAYLOAD) {
    logE2e("shapes corpus movie not found — movie-load specs will skip");
  }
});
