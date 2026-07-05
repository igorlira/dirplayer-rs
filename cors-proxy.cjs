#!/usr/bin/env node
// Generic dev CORS proxy for DirPlayer "loader mode".
//
// Lets the localhost dev UI fetch a Shockwave game (its loader page, the
// .dcr, external casts, images, sounds, and high-score POSTs) from a live
// site that doesn't send CORS headers. It fetches the target server-side and
// re-serves it with permissive CORS, forwarding method / body / most headers.
//
//   GET  /cors?url=<encoded absolute url>
//   POST /cors?url=<encoded absolute url>   (body forwarded)
//
// This is a DEV-ONLY tool: it will proxy any URL, so run it only on localhost.
// It is entirely separate from ws-tcp-proxy-all.cjs and is opt-in — the dev UI
// only routes through it when loader mode is active.
//
//   node cors-proxy.cjs            # listens on 127.0.0.1:3099
//   PORT=4000 node cors-proxy.cjs  # custom port

const http = require("http");
const https = require("https");
const { URL } = require("url");

const PORT = parseInt(process.env.PORT || "3099", 10);
const HOST = process.env.HOST || "127.0.0.1";
const MAX_REDIRECTS = 8;

// Hop-by-hop headers must not be forwarded, plus a few that break re-serving.
const STRIP_REQ_HEADERS = new Set([
  "host", "origin", "referer", "connection", "keep-alive",
  "proxy-authenticate", "proxy-authorization", "te", "trailer",
  "transfer-encoding", "upgrade", "accept-encoding", // let node negotiate/pass-through identity
]);
const STRIP_RES_HEADERS = new Set([
  "connection", "keep-alive", "transfer-encoding", "content-encoding",
  "content-length", "content-security-policy", "content-security-policy-report-only",
  "x-frame-options", "cross-origin-opener-policy", "cross-origin-embedder-policy",
  "cross-origin-resource-policy", "set-cookie", // set-cookie can't cross origin usefully anyway
]);

function corsHeaders(reqHeaders) {
  return {
    "access-control-allow-origin": reqHeaders.origin || "*",
    "access-control-allow-methods": "GET, POST, PUT, DELETE, OPTIONS, HEAD",
    "access-control-allow-headers":
      reqHeaders["access-control-request-headers"] || "*",
    "access-control-allow-credentials": "true",
    "access-control-expose-headers": "*",
    "access-control-max-age": "86400",
  };
}

function fail(res, reqHeaders, code, msg) {
  res.writeHead(code, { "content-type": "text/plain", ...corsHeaders(reqHeaders) });
  res.end(msg);
}

function proxyRequest(targetUrl, clientReq, res, bodyChunks, redirectsLeft) {
  let target;
  try {
    target = new URL(targetUrl);
  } catch {
    return fail(res, clientReq.headers, 400, `Bad target url: ${targetUrl}`);
  }
  if (target.protocol !== "http:" && target.protocol !== "https:") {
    return fail(res, clientReq.headers, 400, `Unsupported protocol: ${target.protocol}`);
  }

  const headers = {};
  for (const [k, v] of Object.entries(clientReq.headers)) {
    if (!STRIP_REQ_HEADERS.has(k.toLowerCase())) headers[k] = v;
  }
  headers["host"] = target.host;
  // Present as a normal same-site request to the target.
  headers["referer"] = target.origin + "/";
  headers["origin"] = target.origin;

  const lib = target.protocol === "https:" ? https : http;
  const upstream = lib.request(
    target,
    { method: clientReq.method, headers },
    (up) => {
      const status = up.statusCode || 502;
      // Follow redirects server-side so the browser never sees a cross-origin 3xx.
      if (
        status >= 300 && status < 400 && up.headers.location &&
        redirectsLeft > 0
      ) {
        up.resume(); // drain
        const next = new URL(up.headers.location, target).toString();
        return proxyRequest(next, clientReq, res, bodyChunks, redirectsLeft - 1);
      }

      const outHeaders = { ...corsHeaders(clientReq.headers) };
      for (const [k, v] of Object.entries(up.headers)) {
        if (!STRIP_RES_HEADERS.has(k.toLowerCase())) outHeaders[k] = v;
      }
      res.writeHead(status, outHeaders);
      up.pipe(res);
    }
  );

  upstream.on("error", (err) => {
    fail(res, clientReq.headers, 502, `Upstream error: ${err.message}`);
  });

  if (bodyChunks && bodyChunks.length) upstream.end(Buffer.concat(bodyChunks));
  else upstream.end();
}

const server = http.createServer((req, res) => {
  // CORS preflight.
  if (req.method === "OPTIONS") {
    res.writeHead(204, corsHeaders(req.headers));
    return res.end();
  }

  let parsed;
  try {
    parsed = new URL(req.url, `http://${HOST}:${PORT}`);
  } catch {
    return fail(res, req.headers, 400, "Bad request url");
  }

  if (parsed.pathname === "/" || parsed.pathname === "/health") {
    res.writeHead(200, { "content-type": "text/plain", ...corsHeaders(req.headers) });
    return res.end("DirPlayer CORS proxy. Use /cors?url=<encoded absolute url>\n");
  }

  if (parsed.pathname !== "/cors") {
    return fail(res, req.headers, 404, "Not found. Use /cors?url=<encoded absolute url>");
  }

  const target = parsed.searchParams.get("url");
  if (!target) {
    return fail(res, req.headers, 400, "Missing ?url= parameter");
  }

  const chunks = [];
  req.on("data", (c) => chunks.push(c));
  req.on("end", () => proxyRequest(target, req, res, chunks, MAX_REDIRECTS));
  req.on("error", () => fail(res, req.headers, 400, "Request stream error"));
});

server.listen(PORT, HOST, () => {
  console.log(`[cors-proxy] listening on http://${HOST}:${PORT}/cors?url=<encoded url>`);
});
