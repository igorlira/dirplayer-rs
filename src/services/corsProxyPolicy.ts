/**
 * When the Chrome extension relays a fetch through the service worker.
 * Content-script `fetch()` is page CORS; the worker has host_permissions.
 * Same-origin and localhost stay on the page (cookies / local CORS servers).
 */
export function shouldRelayCrossOriginFetch(
  url: URL,
  pageOrigin: string,
  isExtension: boolean,
): boolean {
  if (!isExtension) return false;
  try {
    if (url.origin === pageOrigin) return false;
  } catch {
    return false;
  }
  if (url.hostname === 'localhost' || url.hostname === '127.0.0.1') return false;
  return url.protocol === 'http:' || url.protocol === 'https:';
}

/**
 * Mixed-content upgrade: an https page fetching http://same-host would be
 * blocked. localhost / 127.0.0.1 stay http (dev proxies). Mutates `url` and
 * returns whether it changed.
 */
export function upgradeInsecureUrlForPage(url: URL, pageProtocol: string): boolean {
  if (
    pageProtocol === 'https:' &&
    url.protocol === 'http:' &&
    url.hostname !== 'localhost' &&
    url.hostname !== '127.0.0.1'
  ) {
    url.protocol = 'https:';
    return true;
  }
  return false;
}

/** Headers on the reconstructed Response the content script hands to WASM. */
export function corsResponseHeaders(
  contentType?: string,
  contentLength?: string,
): Record<string, string> | undefined {
  const headers: Record<string, string> = {};
  if (contentType) headers['content-type'] = contentType;
  if (contentLength) headers['content-length'] = contentLength;
  return Object.keys(headers).length ? headers : undefined;
}

/**
 * Buffered SW fallback: prefer the real Content-Length header so
 * getStreamStatus #bytesTotal is non-zero; otherwise the body size.
 */
export function corsProxyContentLength(
  header: string | null | undefined,
  bodyByteLength: number,
): string {
  return header ? header : String(bodyByteLength);
}
