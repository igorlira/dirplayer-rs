import {
  corsProxyContentLength,
  corsResponseHeaders,
  shouldRelayCrossOriginFetch,
  upgradeInsecureUrlForPage,
} from './corsProxyPolicy';

describe('shouldRelayCrossOriginFetch', () => {
  const page = 'https://www.neopets.com';

  it('never relays outside the extension', () => {
    expect(
      shouldRelayCrossOriginFetch(new URL('https://swf.neopets.com/g.dcr'), page, false),
    ).toBe(false);
  });

  it('does not relay same-origin requests', () => {
    expect(
      shouldRelayCrossOriginFetch(
        new URL('https://www.neopets.com/games/dgs/dgs_get_game_data.phtml'),
        page,
        true,
      ),
    ).toBe(false);
  });

  it('does not relay localhost or 127.0.0.1 (page fetch / local CORS servers)', () => {
    expect(shouldRelayCrossOriginFetch(new URL('http://127.0.0.1:8012/whoami'), page, true)).toBe(
      false,
    );
    expect(shouldRelayCrossOriginFetch(new URL('http://localhost:3099/cors'), page, true)).toBe(
      false,
    );
  });

  it('relays remote cross-origin http(s)', () => {
    expect(
      shouldRelayCrossOriginFetch(new URL('https://swf.neopets.com/games/g349.dcr'), page, true),
    ).toBe(true);
    expect(
      shouldRelayCrossOriginFetch(new URL('http://cdn.example.com/asset.bin'), page, true),
    ).toBe(true);
  });

  it('does not relay non-http(s) URLs', () => {
    expect(shouldRelayCrossOriginFetch(new URL('blob:https://www.neopets.com/x'), page, true)).toBe(
      false,
    );
  });
});

describe('upgradeInsecureUrlForPage', () => {
  it('upgrades http to https on an https page', () => {
    const url = new URL('http://swf.neopets.com/games/g349.dcr');
    expect(upgradeInsecureUrlForPage(url, 'https:')).toBe(true);
    expect(url.protocol).toBe('https:');
    expect(url.href).toBe('https://swf.neopets.com/games/g349.dcr');
  });

  it('does not change https or same-scheme http pages', () => {
    const httpsUrl = new URL('https://swf.neopets.com/g.dcr');
    expect(upgradeInsecureUrlForPage(httpsUrl, 'https:')).toBe(false);
    expect(httpsUrl.protocol).toBe('https:');

    const httpUrl = new URL('http://cdn.example.com/a.bin');
    expect(upgradeInsecureUrlForPage(httpUrl, 'http:')).toBe(false);
    expect(httpUrl.protocol).toBe('http:');
  });

  it('leaves localhost and 127.0.0.1 on http for local CORS servers', () => {
    const loopback = new URL('http://127.0.0.1:8012/whoami');
    expect(upgradeInsecureUrlForPage(loopback, 'https:')).toBe(false);
    expect(loopback.protocol).toBe('http:');

    const local = new URL('http://localhost:3099/cors');
    expect(upgradeInsecureUrlForPage(local, 'https:')).toBe(false);
    expect(local.protocol).toBe('http:');
  });
});

describe('corsResponseHeaders', () => {
  it('omits headers when both are empty', () => {
    expect(corsResponseHeaders()).toBeUndefined();
    expect(corsResponseHeaders('', '')).toBeUndefined();
  });

  it('includes only the fields that were set', () => {
    expect(corsResponseHeaders('application/octet-stream')).toEqual({
      'content-type': 'application/octet-stream',
    });
    expect(corsResponseHeaders(undefined, '4096')).toEqual({
      'content-length': '4096',
    });
    expect(corsResponseHeaders('application/x-director', '2048')).toEqual({
      'content-type': 'application/x-director',
      'content-length': '2048',
    });
  });
});

describe('corsProxyContentLength', () => {
  it('prefers the real Content-Length header', () => {
    expect(corsProxyContentLength('4096', 12)).toBe('4096');
  });

  it('falls back to the reconstructed body size when the header is missing', () => {
    expect(corsProxyContentLength(null, 12)).toBe('12');
    expect(corsProxyContentLength(undefined, 0)).toBe('0');
    expect(corsProxyContentLength('', 8)).toBe('8');
  });
});
