#!/usr/bin/env node
import http from "node:http";
import https from "node:https";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, "..", "vm-rust", "target", "browser_runner");
const PORT = Number(process.env.BROWSER_RUNNER_PORT || 9101);
const HOST = process.env.BROWSER_RUNNER_HOST || "127.0.0.1";

// ---------------------------------------------------------------------------
// Same-origin reverse proxy for a movie's live backend.
//
// A movie that talks to a real server does it from the PAGE, and the page is
// served from 127.0.0.1:9101 -- so every such call is cross-origin and the
// browser applies CORS to it. That is a rule about the page's origin, not about
// reachability: Coke Studios' `/sf/status` answers 200 with a real session
// cookie to curl and is still discarded by the browser, because the response
// carries no `Access-Control-Allow-Origin`. `/sf/gateway` is worse -- it answers
// a 302, and a redirect fails a CORS preflight outright, whatever it redirects
// to.
//
// Neither is something the movie or dirplayer can fix, and both depend on how
// somebody else's vhost is configured today (Coke Studios' moved behind
// Cloudflare and lost its CORS headers). So proxy the prefix instead: the page
// calls `http://127.0.0.1:9101/sf/...`, which is same-origin and exempt from
// CORS entirely, and this server makes the cross-origin hop itself. Node is not
// a browser, so nothing here enforces CORS.
//
// Configured through the environment so no host is baked in:
//
//   E2E_PROXY=/sf=https://decibel.fun,/other=http://host:8080
//
// `scripts/run-browser-tests.mjs` builds this from the `[[flash.fetch_rewrite]]`
// blocks in the movie's TOML, so a movie that already declares a rewrite gets
// the proxy for free.
//
// Redirects are FOLLOWED here (up to 5), which is the other half of the fix:
// the browser could not follow the `/sf/gateway` 302 on a preflight, but this
// server can, and the page only ever sees the final response.
// ---------------------------------------------------------------------------
const PROXY_ROUTES = (process.env.E2E_PROXY || "")
  .split(",")
  .map((entry) => entry.trim())
  .filter(Boolean)
  .map((entry) => {
    const eq = entry.indexOf("=");
    if (eq === -1) return null;
    // A leading slash is optional: Git Bash on Windows rewrites a value that
    // looks like a POSIX path, so `/sf=https://host` reaches Node as
    // `C:\Program Files\Git\sf=https;\host`. Accepting `sf=https://host`
    // keeps the format usable from every shell (and `export` before the run,
    // rather than an inline `VAR=x cmd` prefix, avoids the mangling entirely).
    let prefix = entry.slice(0, eq).trim();
    const target = entry.slice(eq + 1).trim();
    if (!prefix || !target) return null;
    if (!prefix.startsWith("/")) prefix = "/" + prefix;
    try {
      return { prefix, target: new URL(target) };
    } catch {
      console.warn(`E2E_PROXY: ignoring unparseable target for ${prefix}: ${target}`);
      return null;
    }
  })
  .filter(Boolean);

function matchProxyRoute(urlPath) {
  const bare = urlPath.split("?")[0];
  // Longest prefix wins, so /sf/gateway can be routed apart from /sf later.
  let best = null;
  for (const route of PROXY_ROUTES) {
    if (bare === route.prefix || bare.startsWith(route.prefix + "/")) {
      if (!best || route.prefix.length > best.prefix.length) best = route;
    }
  }
  return best;
}

function proxyRequest(req, res, route, depth = 0, overrideUrl = null) {
  if (depth > 5) {
    res.writeHead(508);
    res.end("E2E_PROXY: too many redirects");
    return;
  }

  const upstream = overrideUrl
    ? overrideUrl
    : new URL(req.url, route.target.origin);
  const client = upstream.protocol === "https:" ? https : http;

  // Forward the request headers, minus the ones that describe the HOP rather
  // than the request. `host` must become the upstream's or a name-based vhost
  // serves the wrong site; `origin`/`referer` are dropped so the upstream sees
  // an ordinary same-site call rather than one it might reject on origin.
  const headers = { ...req.headers };
  delete headers.host;
  delete headers.origin;
  delete headers.referer;
  delete headers.connection;
  delete headers["accept-encoding"]; // keep the body uncompressed and pass-through simple
  headers.host = upstream.host;

  const upReq = client.request(
    {
      protocol: upstream.protocol,
      hostname: upstream.hostname,
      port: upstream.port || (upstream.protocol === "https:" ? 443 : 80),
      method: req.method,
      path: upstream.pathname + upstream.search,
      headers,
    },
    (upRes) => {
      const status = upRes.statusCode || 502;
      const location = upRes.headers.location;
      if (status >= 300 && status < 400 && location) {
        // Follow it here. The browser could not: a redirect is not allowed on a
        // CORS preflight, which is precisely what broke /sf/gateway.
        upRes.resume();
        let next;
        try {
          next = new URL(location, upstream);
        } catch {
          res.writeHead(502);
          res.end("E2E_PROXY: bad redirect target");
          return;
        }
        proxyRequest(req, res, route, depth + 1, next);
        return;
      }

      // Strip the upstream's own CORS and transport headers and answer as
      // same-origin, which is what the page asked for.
      const out = { ...upRes.headers };
      delete out["access-control-allow-origin"];
      delete out["access-control-allow-credentials"];
      delete out["transfer-encoding"];
      delete out.connection;
      out["cache-control"] = "no-store";
      res.writeHead(status, out);
      upRes.pipe(res);
    }
  );

  upReq.on("error", (err) => {
    console.warn(`E2E_PROXY ${req.method} ${req.url} -> ${upstream.href}: ${err.message}`);
    if (!res.headersSent) res.writeHead(502);
    res.end(`E2E_PROXY upstream error: ${err.message}`);
  });

  req.pipe(upReq);
}

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".js": "application/javascript; charset=utf-8",
  ".mjs": "application/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".wasm": "application/wasm",
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".jpeg": "image/jpeg",
  ".gif": "image/gif",
  ".svg": "image/svg+xml",
  ".txt": "text/plain; charset=utf-8",
};

function resolveSafe(urlPath) {
  const decoded = decodeURIComponent(urlPath.split("?")[0]);
  const rel = decoded.replace(/^\/+/, "");
  const full = path.resolve(ROOT, rel || "index.html");
  if (!full.startsWith(ROOT)) return null;
  return full;
}

const server = http.createServer((req, res) => {
  const route = matchProxyRoute(req.url || "/");
  if (route) {
    proxyRequest(req, res, route);
    return;
  }
  const fullPath = resolveSafe(req.url || "/");
  if (!fullPath) {
    res.writeHead(403);
    res.end("Forbidden");
    return;
  }
  fs.stat(fullPath, (err, stat) => {
    if (err) {
      res.writeHead(404);
      res.end("Not found");
      return;
    }
    const target = stat.isDirectory() ? path.join(fullPath, "index.html") : fullPath;
    fs.readFile(target, (readErr, data) => {
      if (readErr) {
        res.writeHead(404);
        res.end("Not found");
        return;
      }
      const ext = path.extname(target).toLowerCase();
      res.writeHead(200, {
        "Content-Type": MIME[ext] || "application/octet-stream",
        "Cache-Control": "no-store",
      });
      res.end(data);
    });
  });
});

server.listen(PORT, HOST, () => {
  console.log(`Serving ${ROOT} at http://${HOST}:${PORT}`);
  for (const route of PROXY_ROUTES) {
    console.log(`  proxying ${route.prefix} -> ${route.target.origin}`);
  }
});
