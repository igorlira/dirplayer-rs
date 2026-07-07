import { useCallback, useState } from 'react';
import styles from './styles.module.css';
import { load_movie_file, play, set_base_path, set_external_params, set_movie_path_override } from 'vm-rust';
import { getExternalXtrasReady, resolveAndLoadMovieXtras, setXtraMovieBase, whenMovieLoaded } from 'dirplayer-js-api';
import { useMountEffect } from '../../utils/hooks';
import { isDebugSession } from '../../utils/debug';
import { getBasePath, getFullPathFromOrigin } from '../../utils/path';
import { isElectron, openFileDialog } from '../../utils/electron';
import { APP_TITLE } from '../../constants';
import { useDispatch, useSelector } from 'react-redux';
import { RootState } from '../../store';
import { movieUnloaded } from '../../store/vmSlice';

type ExternalParam = { key: string; value: string };

type RecentMovie = {
  url: string;
  params: ExternalParam[];
  fakeMoviePath?: string;
  timestamp: number;
};

const RECENT_MOVIES_KEY = 'recentMovies';
const MAX_RECENT_MOVIES = 100;
const ENV_PARAM_PREFIX = 'REACT_APP_MOVIE_PARAM_';

function getEnvExternalParams(): ExternalParam[] {
  return Object.entries(process.env)
    .filter(([k, v]) => k.startsWith(ENV_PARAM_PREFIX) && v !== undefined)
    .map(([k, v]) => ({ key: k.slice(ENV_PARAM_PREFIX.length), value: v as string }));
}

function paramsArrayToRecord(params: ExternalParam[]): Record<string, string> {
  const record: Record<string, string> = {};
  for (const p of params) {
    if (p.key.trim()) {
      record[p.key.trim()] = p.value;
    }
  }
  return record;
}

const DEFAULT_CORS_PROXY = 'http://127.0.0.1:3099/cors?url=';

function absolutize(src: string, base: string): string {
  try { return new URL(src, base).toString(); } catch { return src; }
}

// Extract the Director movie URL + sw* external params from a Shockwave loader
// page's HTML. Handles <embed type="application/x-director">, <object>/<param>,
// and a raw fallback scan for JS-built embeds. Returns null if no Director
// object is found.
function parseShockwaveLoader(html: string, loaderUrl: string): { movieUrl: string; params: ExternalParam[] } | null {
  const isDirector = (s: string | null) => !!s && /\.(dcr|dxr|dir)(\?|#|$)/i.test(s);
  const params: ExternalParam[] = [];
  let src = '';

  try {
    const doc = new DOMParser().parseFromString(html, 'text/html');

    // 1) <embed type="application/x-director" src=... sw1=...>
    const embed = Array.from(doc.querySelectorAll('embed')).find(e =>
      (e.getAttribute('type') || '').toLowerCase().includes('director') ||
      isDirector(e.getAttribute('src')));
    if (embed) {
      src = embed.getAttribute('src') || '';
      for (const attr of Array.from(embed.attributes)) {
        if (/^sw[0-9a-z]*$/i.test(attr.name) && attr.value) {
          params.push({ key: attr.name.toLowerCase(), value: attr.value });
        }
      }
    }

    // 2) <object> with <param name="src"/"movie"/"sw1" ...>
    if (!src) {
      for (const obj of Array.from(doc.querySelectorAll('object'))) {
        const pmap: Record<string, string> = {};
        for (const p of Array.from(obj.querySelectorAll('param'))) {
          const n = (p.getAttribute('name') || '').toLowerCase();
          if (n) pmap[n] = p.getAttribute('value') || '';
        }
        const s = pmap['src'] || pmap['movie'] || obj.getAttribute('data') || '';
        if (isDirector(s)) {
          src = s;
          for (const [n, v] of Object.entries(pmap)) {
            if (/^sw[0-9a-z]*$/i.test(n) && v) params.push({ key: n, value: v });
          }
          break;
        }
      }
    }
  } catch { /* fall through to raw scan */ }

  // 3) Raw fallback: scan for a .dcr url + sw1..sw9 string literals (JS-built).
  if (!src) {
    const m = html.match(/["']([^"']+\.(?:dcr|dxr|dir)(?:\?[^"']*)?)["']/i);
    if (m) src = m[1];
  }
  if (params.length === 0) {
    for (let i = 1; i <= 9; i++) {
      const mm = html.match(new RegExp(`sw${i}\\s*[:=]\\s*["']([^"']*)["']`, 'i'));
      if (mm) params.push({ key: `sw${i}`, value: mm[1] });
    }
  }

  if (!src) return null;
  return { movieUrl: absolutize(src, loaderUrl), params };
}

function loadRecentMovies(): RecentMovie[] {
  try {
    const raw = window.localStorage.getItem(RECENT_MOVIES_KEY);
    return raw ? JSON.parse(raw) : [];
  } catch {
    return [];
  }
}

function saveRecentMovie(url: string, params: ExternalParam[], fakeMoviePath?: string): RecentMovie[] {
  const existing = loadRecentMovies().filter(m => m.url !== url);
  const updated = [{ url, params, fakeMoviePath, timestamp: Date.now() }, ...existing].slice(0, MAX_RECENT_MOVIES);
  window.localStorage.setItem(RECENT_MOVIES_KEY, JSON.stringify(updated));
  return updated;
}

function removeRecentMovie(url: string): RecentMovie[] {
  const updated = loadRecentMovies().filter(m => m.url !== url);
  window.localStorage.setItem(RECENT_MOVIES_KEY, JSON.stringify(updated));
  return updated;
}

function clearRecentMovies(): RecentMovie[] {
  window.localStorage.removeItem(RECENT_MOVIES_KEY);
  return [];
}

export default function LoadMovie() {
  const dispatch = useDispatch();
  const movieLoadError = useSelector<RootState, string | undefined>(state => state.vm.movieLoadError);
  const defaultMovieUrl = process.env.REACT_APP_MOVIE_URL ? getFullPathFromOrigin(process.env.REACT_APP_MOVIE_URL) : '';
  const [movieUrl, setMovieUrl] = useState<string>(defaultMovieUrl || '');
  const [isLoading, setIsLoading] = useState<boolean>(false);
  const [hasError, setHasError] = useState(false);
  const [autoPlay, setAutoPlay] = useState<boolean>(process.env.REACT_APP_MOVIE_AUTO_PLAY === 'true');
  const [externalParams, setExternalParams] = useState<ExternalParam[]>(() => getEnvExternalParams());
  const [fakeMoviePath, setFakeMoviePath] = useState<string>('');
  const [recentMovies, setRecentMovies] = useState<RecentMovie[]>(() => loadRecentMovies());
  const [paramsExpanded, setParamsExpanded] = useState(() => getEnvExternalParams().length > 0);
  const [loaderUrl, setLoaderUrl] = useState<string>('');
  const [corsProxy, setCorsProxy] = useState<string>(DEFAULT_CORS_PROXY);
  const isInElectron = isElectron();

  const addParam = useCallback(() => {
    setExternalParams(prev => [...prev, { key: '', value: '' }]);
    setParamsExpanded(true);
  }, []);

  const removeParam = useCallback((index: number) => {
    setExternalParams(prev => prev.filter((_, i) => i !== index));
  }, []);

  const updateParam = useCallback((index: number, field: 'key' | 'value', val: string) => {
    setExternalParams(prev => prev.map((p, i) => i === index ? { ...p, [field]: val } : p));
  }, []);

  const loadMovieFile = useCallback(async (fullPath: string, params?: ExternalParam[], fakePath?: string) => {
    try {
      setIsLoading(true);
      setHasError(false);
      // The CORS-proxy field is the control: if it holds a base URL, route this
      // movie's CROSS-ORIGIN http(s) fetches through it (e.g. Neopets DGS's
      // `preloadNetThing("http://swf.neopets.com/...")`); if it's empty, route
      // direct. maybeCorsProxy() only rewrites cross-origin requests, so a
      // same-origin local movie is unaffected either way. We re-apply it on
      // EVERY load — setting when non-empty, and CLEARING when empty — because
      // `__dirplayerFlashConfig.corsProxy` is a persistent window global; without
      // the clear, a proxy set by a prior load (or loader mode) leaked into
      // later loads that shouldn't use it. Clear the field to disable.
      const proxyBase = corsProxy.trim();
      if (proxyBase) {
        (window as any).__dirplayerFlashConfig = {
          ...((window as any).__dirplayerFlashConfig || {}),
          corsProxy: proxyBase,
        };
      } else {
        const cfg = (window as any).__dirplayerFlashConfig;
        if (cfg && cfg.corsProxy) {
          (window as any).__dirplayerFlashConfig = { ...cfg, corsProxy: null };
        }
      }
      dispatch(movieUnloaded());
      const moviePath = getBasePath(fullPath);
      set_base_path(moviePath);
      // Make bare xtra filenames (e.g. localStorage entry "foo.wasm")
      // resolve against this movie's directory.
      setXtraMovieBase(moviePath);
      set_external_params(paramsArrayToRecord(params ?? externalParams));
      set_movie_path_override(fakePath ?? fakeMoviePath ?? '');
      document.title = `${fullPath.split('/').pop() || fullPath} - ${APP_TITLE}`;
      // Wait for any in-flight boot-time external xtra loads (the
      // localStorage URL list) before touching anything xtra-related.
      await getExternalXtrasReady();
      // Always load with autoplay=false so the metadata (incl. the
      // movie's XTRl xtra-dependency list) is parsed BEFORE any Lingo
      // runs. vm-rust's load_movie_file is fire-and-forget (dispatches
      // a command and returns immediately), so we have to await the
      // onMovieLoaded callback via whenMovieLoaded() before the XTRl
      // is actually populated — otherwise resolveAndLoadMovieXtras
      // sees an empty required-xtras list.
      const movieLoadedPromise = whenMovieLoaded();
      await load_movie_file(fullPath, false);
      await movieLoadedPromise;
      await resolveAndLoadMovieXtras();
      if (autoPlay) play();
    } catch (e) {
      console.error('Failed to load movie', e);
    } finally {
      setIsLoading(false);
    }
  }, [autoPlay, dispatch, externalParams, fakeMoviePath, corsProxy]);

  const onLoadClick = useCallback(async () => {
    if (!movieUrl.trim()) { setHasError(true); return; }
    const updated = saveRecentMovie(movieUrl, externalParams, fakeMoviePath);
    setRecentMovies(updated);
    await loadMovieFile(movieUrl);
  }, [movieUrl, externalParams, fakeMoviePath, loadMovieFile]);

  // Loader mode: fetch a Shockwave loader page through the dev CORS proxy,
  // extract the Director embed (movie URL + sw* external params), enable proxy
  // routing so the game's own cross-origin fetches also go through it, and load.
  const onLoadLoader = useCallback(async () => {
    const lu = loaderUrl.trim();
    const proxyBase = corsProxy.trim();
    if (!lu) { setHasError(true); return; }
    if (!proxyBase) { console.error('[LoadMovie] loader mode needs a CORS proxy base'); setHasError(true); return; }
    try {
      setIsLoading(true);
      setHasError(false);
      // Turn on proxy routing for the fetch interceptor (loader page + game assets).
      (window as any).__dirplayerFlashConfig = {
        ...((window as any).__dirplayerFlashConfig || {}),
        corsProxy: proxyBase,
      };
      const res = await fetch(proxyBase + encodeURIComponent(lu));
      if (!res.ok) throw new Error(`loader fetch ${res.status}`);
      const html = await res.text();
      const parsed = parseShockwaveLoader(html, lu);
      if (!parsed) {
        console.error('[LoadMovie] No Director <embed>/<object> found in the loader page.');
        setHasError(true);
        return;
      }
      setMovieUrl(parsed.movieUrl);
      setExternalParams(parsed.params);
      if (parsed.params.length > 0) setParamsExpanded(true);
      setRecentMovies(saveRecentMovie(parsed.movieUrl, parsed.params));
      await loadMovieFile(parsed.movieUrl, parsed.params);
    } catch (e) {
      console.error('[LoadMovie] Loader load failed', e);
      setHasError(true);
    } finally {
      setIsLoading(false);
    }
  }, [loaderUrl, corsProxy, loadMovieFile]);

  const onBrowseClick = useCallback(async () => {
    if (!isInElectron) return;
    try {
      const filePath = await openFileDialog();
      if (filePath) {
        setMovieUrl(`file://${filePath}`);
      }
    } catch (e) {
      console.error('[LoadMovie] Failed to open file dialog', e);
    }
  }, [isInElectron]);

  const onLoadRecent = useCallback((movie: RecentMovie) => {
    setMovieUrl(movie.url);
    setExternalParams(movie.params);
    setFakeMoviePath(movie.fakeMoviePath ?? '');
    const updated = saveRecentMovie(movie.url, movie.params, movie.fakeMoviePath);
    setRecentMovies(updated);
    loadMovieFile(movie.url, movie.params, movie.fakeMoviePath);
  }, [loadMovieFile]);

  const onEditRecent = useCallback((movie: RecentMovie) => {
    setMovieUrl(movie.url);
    setExternalParams(movie.params);
    setFakeMoviePath(movie.fakeMoviePath ?? '');
    if (movie.params.length > 0 || movie.fakeMoviePath) {
      setParamsExpanded(true);
    }
  }, []);

  const onRemoveRecent = useCallback((url: string) => {
    setRecentMovies(removeRecentMovie(url));
  }, []);

  const onClearRecent = useCallback(() => {
    setRecentMovies(clearRecentMovies());
  }, []);

  useMountEffect(async () => {
    if (movieUrl && process.env.REACT_APP_MOVIE_AUTO_LOAD === 'true' && !isDebugSession()) {
      await loadMovieFile(movieUrl);
    }
  });

  const hasParams = externalParams.length > 0;

  return <div className={styles.container}>
    <div className={styles.header}>
      <h1 className={styles.title}>DirPlayer</h1>
      <div className={styles.subtitle}>Load Movie</div>
    </div>

    <div className={styles.card}>
      <div className={styles.cardBody}>
        <div className={styles.fieldContainer}>
          <label className={styles.label} htmlFor="url">
            {isInElectron ? 'Movie Path' : 'Movie URL'}
          </label>
          <div className={styles.inputGroup}>
            <input
              id="url"
              name="url"
              type="text"
              className={`${styles.input} ${hasError ? styles.inputError : ''}`}
              placeholder={isInElectron ? '/path/to/movie.dcr' : 'https://example.com/movie.dcr'}
              value={movieUrl}
              onChange={e => { setMovieUrl(e.currentTarget.value); setHasError(false); }}
              disabled={isLoading}
            />
            {isInElectron && (
              <button
                className={styles.browseButton}
                onClick={onBrowseClick}
                disabled={isLoading}
              >
                Browse...
              </button>
            )}
          </div>
        </div>

        <div className={styles.paramsSection}>
          <button
            className={styles.paramsToggle}
            onClick={() => setParamsExpanded(prev => !prev)}
          >
            <span className={`${styles.paramsToggleArrow} ${paramsExpanded ? styles.paramsToggleArrowOpen : ''}`}>
              &#9654;
            </span>
            Advanced Options
            {(hasParams || fakeMoviePath) && !paramsExpanded && (
              <span> ({[hasParams && `${externalParams.length} params`, fakeMoviePath && 'fake path'].filter(Boolean).join(', ')})</span>
            )}
          </button>
          {paramsExpanded && (
            <div className={styles.paramsList}>
              <div className={styles.fieldContainer}>
                <label className={styles.label} htmlFor="loaderUrl">
                  Shockwave Loader URL (optional)
                </label>
                <input
                  id="loaderUrl"
                  type="text"
                  className={styles.input}
                  placeholder="https://www.neopets.com/games/dgs/play_shockwave.phtml?game_id=..."
                  value={loaderUrl}
                  onChange={e => setLoaderUrl(e.currentTarget.value)}
                  disabled={isLoading}
                />
                <div style={{ display: 'flex', gap: 8, marginTop: 6 }}>
                  <input
                    type="text"
                    className={styles.input}
                    style={{ flex: 1 }}
                    placeholder="CORS proxy base"
                    value={corsProxy}
                    onChange={e => setCorsProxy(e.currentTarget.value)}
                    disabled={isLoading}
                    title="Run: node cors-proxy.cjs"
                  />
                  <button
                    className={styles.browseButton}
                    onClick={onLoadLoader}
                    disabled={isLoading}
                  >
                    Fetch &amp; Load
                  </button>
                </div>
                <div style={{ fontSize: '0.8em', color: '#888', marginTop: 4 }}>
                  Fetches the loader page through the CORS proxy, extracts the
                  Director embed + sw params, and loads it (the game's own
                  cross-origin fetches then route through the proxy too). Start
                  the proxy first: <code>node cors-proxy.cjs</code>.
                </div>
              </div>
              <div className={styles.fieldContainer}>
                <label className={styles.label} htmlFor="fakeMoviePath">
                  Fake Movie Path (optional)
                </label>
                <input
                  id="fakeMoviePath"
                  type="text"
                  className={styles.input}
                  placeholder="https://original-server.com/path/movie.dcr"
                  value={fakeMoviePath}
                  onChange={e => setFakeMoviePath(e.currentTarget.value)}
                  disabled={isLoading}
                />
              </div>
              {externalParams.map((param, index) => (
                <div key={index} className={styles.paramRow}>
                  <input
                    type="text"
                    className={styles.paramInput}
                    placeholder="key (e.g. sw1)"
                    value={param.key}
                    onChange={e => updateParam(index, 'key', e.currentTarget.value)}
                    disabled={isLoading}
                  />
                  <input
                    type="text"
                    className={styles.paramInput}
                    placeholder="value"
                    value={param.value}
                    onChange={e => updateParam(index, 'value', e.currentTarget.value)}
                    disabled={isLoading}
                  />
                  <button
                    className={styles.removeParamButton}
                    onClick={() => removeParam(index)}
                    disabled={isLoading}
                    title="Remove parameter"
                  >
                    &#10005;
                  </button>
                </div>
              ))}
              <button
                className={styles.addParamButton}
                onClick={addParam}
                disabled={isLoading}
              >
                + Add parameter
              </button>
            </div>
          )}
        </div>

        <div className={styles.optionsRow}>
          <label className={styles.checkboxContainer}>
            <input
              type="checkbox"
              id="autoPlay"
              name="autoPlay"
              className={styles.checkbox}
              disabled={isLoading}
              checked={autoPlay}
              onChange={e => setAutoPlay(e.currentTarget.checked)}
            />
            Auto-play
          </label>
        </div>

        {movieLoadError && (
          <div style={{
            display: 'flex',
            alignItems: 'flex-start',
            gap: '10px',
            padding: '10px 14px',
            backgroundColor: '#fff3f3',
            border: '1px solid #f5c0c0',
            borderRadius: '4px',
            fontSize: '0.88em',
            color: '#a33',
          }}>
            <svg style={{ flexShrink: 0, marginTop: '1px' }} width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/>
              <line x1="12" y1="9" x2="12" y2="13"/>
              <line x1="12" y1="17" x2="12.01" y2="17"/>
            </svg>
            <span style={{ wordBreak: 'break-word' }}>{movieLoadError}</span>
          </div>
        )}
        <button className={styles.button} onClick={onLoadClick} disabled={isLoading}>
          {isLoading ? 'Loading...' : 'Load Movie'}
        </button>
      </div>
    </div>

    {recentMovies.length > 0 && (
      <div className={styles.recentSection}>
        <div className={styles.recentHeader}>
          <span className={styles.recentTitle}>Recent Movies</span>
          <button className={styles.clearRecent} onClick={onClearRecent}>
            Clear all
          </button>
        </div>
        <div className={styles.recentList}>
          {recentMovies.map((movie) => (
            <div key={movie.url} className={styles.recentItem} onClick={() => onEditRecent(movie)}>
              <div className={styles.recentItemBody}>
                <span className={styles.recentUrl} title={movie.url}>
                  {movie.url}
                </span>
                {(movie.params.length > 0 || movie.fakeMoviePath) && (
                  <div className={styles.recentParams}>
                    {movie.fakeMoviePath && (
                      <span className={styles.paramTag}>
                        fakePath={movie.fakeMoviePath}
                      </span>
                    )}
                    {movie.params
                      .filter(p => p.key.trim())
                      .map((p, i) => (
                        <span key={i} className={styles.paramTag}>
                          {p.key}={p.value}
                        </span>
                      ))}
                  </div>
                )}
              </div>
              <div className={styles.recentActions}>
                <button
                  className={styles.loadRecentButton}
                  onClick={e => { e.stopPropagation(); onLoadRecent(movie); }}
                  disabled={isLoading}
                >
                  Load
                </button>
              </div>
              <button
                className={styles.removeRecentButton}
                onClick={e => { e.stopPropagation(); onRemoveRecent(movie.url); }}
                title="Remove from recent"
              >
                &#10005;
              </button>
            </div>
          ))}
        </div>
      </div>
    )}
  </div>;
}
