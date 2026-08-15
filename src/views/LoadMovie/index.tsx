import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import styles from './styles.module.css';
import logoUrl from '../../assets/logo128.png';
import ThemeToggle from '../../components/ThemeToggle';
import { load_movie_file, play, set_base_path, set_external_params, set_movie_path_override,
  set_startup_do, set_startup_do_before, set_startup_go } from 'vm-rust';
import { parseLaunchCommand, findLaunchCommandInHtml, findLegacyServerInHtml, toLegacyUrl } from '../../utils/launchCommand';
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
  // Whether this entry routes its cross-origin fetches through the CORS proxy.
  // Opt-in per movie (advanced-options checkbox); undefined/false = direct.
  useCorsProxy?: boolean;
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

function saveRecentMovie(url: string, params: ExternalParam[], fakeMoviePath?: string, useCorsProxy?: boolean): RecentMovie[] {
  const existing = loadRecentMovies().filter(m => m.url !== url);
  const updated = [{ url, params, fakeMoviePath, useCorsProxy, timestamp: Date.now() }, ...existing].slice(0, MAX_RECENT_MOVIES);
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

// --- Recent list presentation helpers -------------------------------------
// Movie URLs are long and frequently differ only in their directory (the same
// `game.dcr` living under a dozen version folders), so the list groups entries
// by folder and shows the file name on its own. Nothing here parses with `new
// URL()`: entries can be relative paths or Windows-style `file://` paths.

type MovieParts = { dir: string; file: string; query: string };

function lastSeparator(s: string): number {
  return Math.max(s.lastIndexOf('/'), s.lastIndexOf('\\'));
}

function splitMoviePath(url: string): MovieParts {
  const queryStart = url.search(/[?#]/);
  const query = queryStart >= 0 ? url.slice(queryStart) : '';
  let path = queryStart >= 0 ? url.slice(0, queryStart) : url;

  // A URL ending in a separator (…/pepworks.com/leo3d/) has no file component.
  // Name the entry after its last segment instead of rendering a blank row.
  let trailing = '';
  while (path.length > 1 && (path.endsWith('/') || path.endsWith('\\'))) {
    trailing = path.slice(-1);
    path = path.slice(0, -1);
  }

  const sep = lastSeparator(path);
  // Nothing but a scheme and a host left (https://example.com): all name.
  if (sep < 0 || /:\/?$/.test(path.slice(0, sep))) {
    return { dir: '', file: path + trailing, query };
  }
  return {
    dir: path.slice(0, sep) || path.slice(0, sep + 1),
    file: path.slice(sep + 1) + trailing,
    query,
  };
}

// Split a folder path so the last segment can be pinned while the head
// ellipsizes: truncating from the right would hide exactly the segment that
// tells two same-named movies apart.
function splitFolderTail(dir: string): { head: string; tail: string } {
  const sep = lastSeparator(dir);
  if (sep <= 0) return { head: '', tail: dir };
  return { head: dir.slice(0, sep), tail: dir.slice(sep) };
}

function formatRelativeTime(timestamp: number): string {
  const diff = Date.now() - timestamp;
  const minute = 60_000, hour = 3_600_000, day = 86_400_000;
  if (diff < minute) return 'just now';
  if (diff < hour) return `${Math.floor(diff / minute)}m ago`;
  if (diff < day) return `${Math.floor(diff / hour)}h ago`;
  if (diff < 30 * day) return `${Math.floor(diff / day)}d ago`;
  return new Date(timestamp).toLocaleDateString();
}

type MovieGroup = { dir: string; movies: { movie: RecentMovie; parts: MovieParts }[] };

// Group by folder, keeping the incoming (most-recent-first) order both for the
// entries inside a group and for the groups themselves.
function groupMoviesByFolder(movies: RecentMovie[]): MovieGroup[] {
  const groups: MovieGroup[] = [];
  const byDir = new Map<string, MovieGroup>();
  for (const movie of movies) {
    const parts = splitMoviePath(movie.url);
    let group = byDir.get(parts.dir);
    if (!group) {
      group = { dir: parts.dir, movies: [] };
      byDir.set(parts.dir, group);
      groups.push(group);
    }
    group.movies.push({ movie, parts });
  }
  return groups;
}

// Everything worth showing about an entry besides its path. Rows cap how many
// they render (see MAX_ROW_TAGS) so a movie with a dozen sw* params stays one
// row tall instead of towering over its neighbours.
type MovieTag = { key: string; label: string; accent?: boolean };
const MAX_ROW_TAGS = 4;

function movieTags(movie: RecentMovie): MovieTag[] {
  const tags: MovieTag[] = [];
  if (movie.useCorsProxy) tags.push({ key: 'proxy', label: 'proxy', accent: true });
  if (movie.fakeMoviePath) tags.push({ key: 'fakePath', label: `fakePath=${movie.fakeMoviePath}` });
  movie.params.forEach((p, i) => {
    if (p.key.trim()) tags.push({ key: `p${i}`, label: `${p.key}=${p.value}` });
  });
  return tags;
}

function matchesFilter(movie: RecentMovie, tokens: string[]): boolean {
  if (tokens.length === 0) return true;
  const haystack = [
    movie.url,
    movie.fakeMoviePath ?? '',
    ...movie.params.map(p => `${p.key}=${p.value}`),
  ].join(' ').toLowerCase();
  return tokens.every(token => haystack.includes(token));
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
  // CORS proxy is OPT-IN per movie (default OFF); persisted per recent entry.
  const [useCorsProxy, setUseCorsProxy] = useState<boolean>(false);
  const [recentFilter, setRecentFilter] = useState<string>('');
  const [collapsedFolders, setCollapsedFolders] = useState<Set<string>>(() => new Set());
  const [activeIndex, setActiveIndex] = useState<number>(-1);
  const activeItemRef = useRef<HTMLDivElement | null>(null);
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

  const loadMovieFile = useCallback(async (fullPath: string, params?: ExternalParam[], fakePath?: string, useProxy?: boolean,
    launch?: ReturnType<typeof parseLaunchCommand> | null) => {
    try {
      setIsLoading(true);
      setHasError(false);
      // The CORS proxy is OPT-IN per movie (advanced-options checkbox, persisted
      // per recent entry). Only route this movie's CROSS-ORIGIN http(s) fetches
      // through it when ENABLED and a base URL is set (e.g. Neopets DGS's
      // `preloadNetThing("http://swf.neopets.com/...")`); otherwise route direct.
      // We re-apply on EVERY load — setting when enabled, CLEARING otherwise —
      // because `__dirplayerFlashConfig.corsProxy` is a persistent window global;
      // without the clear, a proxy enabled for a PRIOR game leaked into later
      // loads (incl. previously-added recent entries) that shouldn't use it.
      const proxyEnabled = useProxy ?? useCorsProxy;
      const proxyBase = proxyEnabled ? corsProxy.trim() : '';
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
      // Projector launch-command payloads (`--do` / `--doBefore` / `--go`, plus
      // the LeechProtectionRemovalHelp flags synthesised into `--do`). These
      // MUST be installed before load_movie_file: the movie-init sequence
      // consumes them. See docs/github_wiki/Projector-Launch-Commands.md.
      if (launch) {
        if (launch.startupDoBefore) set_startup_do_before(launch.startupDoBefore);
        if (launch.startupDo) set_startup_do(launch.startupDo);
        if (launch.startupGo) set_startup_go(launch.startupGo);
        if (launch.ignored.length) {
          console.warn('[LoadMovie] launch command: ignored projector-only flags:', launch.ignored.join(', '));
        }
      }
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
  }, [autoPlay, dispatch, externalParams, fakeMoviePath, corsProxy, useCorsProxy]);

  const onLoadClick = useCallback(async () => {
    if (!movieUrl.trim()) { setHasError(true); return; }
    const updated = saveRecentMovie(movieUrl, externalParams, fakeMoviePath, useCorsProxy);
    setRecentMovies(updated);
    await loadMovieFile(movieUrl, undefined, undefined, useCorsProxy);
  }, [movieUrl, externalParams, fakeMoviePath, useCorsProxy, loadMovieFile]);

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

      // An ARCHIVE ENTRY page (9o3o / Flashpoint) publishes the whole projector
      // command line in `data-launch-command` instead of embedding the movie.
      // Prefer it: those entries are usually launched from a leech-protection
      // wrapper that is inert without the command's `--do` payload, so scraping
      // an <embed> would either find nothing or find the wrapper with no way to
      // tell it where the real game lives.
      const launchCmd = findLaunchCommandInHtml(html);
      if (launchCmd) {
        const launch = parseLaunchCommand(launchCmd);
        if (launch.movieUrl) {
          const lcParams: ExternalParam[] = Object.entries(launch.externalParams)
            .map(([key, value]) => ({ key, value }));
          // The command names the movie by its ORIGINAL url, which is usually
          // dead (miniclip.com answers a 301 to an HTML page — that arrives as
          // "Invalid codec"). The archive serves the real bytes from its legacy
          // server under a scheme-stripped mirror of that path, so rewrite onto
          // it when the entry page advertises one. Relative fetches the movie
          // makes later (gameloader.dcr, the .w3d levels) then resolve against
          // the legacy path too, since they are relative to the movie's base.
          const legacyServer = findLegacyServerInHtml(html);
          const loadUrl = toLegacyUrl(launch.movieUrl, legacyServer);
          console.log('[LoadMovie] launch command:', launchCmd);
          if (loadUrl !== launch.movieUrl) {
            console.log('[LoadMovie] legacy server:', legacyServer, '->', loadUrl);
          }
          setMovieUrl(loadUrl);
          setExternalParams(lcParams);
          if (lcParams.length > 0) setParamsExpanded(true);
          setUseCorsProxy(true);
          setRecentMovies(saveRecentMovie(loadUrl, lcParams, undefined, true));
          await loadMovieFile(loadUrl, lcParams, undefined, true, launch);
          return;
        }
        console.warn('[LoadMovie] data-launch-command names no movie; falling back to embed scrape.');
      }

      const parsed = parseShockwaveLoader(html, lu);
      if (!parsed) {
        console.error('[LoadMovie] No Director <embed>/<object> found in the loader page.');
        setHasError(true);
        return;
      }
      setMovieUrl(parsed.movieUrl);
      setExternalParams(parsed.params);
      if (parsed.params.length > 0) setParamsExpanded(true);
      // Loader mode inherently requires the proxy (the game's own cross-origin
      // fetches route through it), so force it on and persist that for the entry.
      setUseCorsProxy(true);
      setRecentMovies(saveRecentMovie(parsed.movieUrl, parsed.params, undefined, true));
      await loadMovieFile(parsed.movieUrl, parsed.params, undefined, true);
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
    // Honor the per-entry opt-in (default OFF for legacy entries with no flag).
    const proxy = movie.useCorsProxy ?? false;
    setUseCorsProxy(proxy);
    const updated = saveRecentMovie(movie.url, movie.params, movie.fakeMoviePath, proxy);
    setRecentMovies(updated);
    loadMovieFile(movie.url, movie.params, movie.fakeMoviePath, proxy);
  }, [loadMovieFile]);

  const onEditRecent = useCallback((movie: RecentMovie) => {
    setMovieUrl(movie.url);
    setExternalParams(movie.params);
    setFakeMoviePath(movie.fakeMoviePath ?? '');
    setUseCorsProxy(movie.useCorsProxy ?? false);
    if (movie.params.length > 0 || movie.fakeMoviePath) {
      setParamsExpanded(true);
    }
  }, []);

  const onRemoveRecent = useCallback((url: string) => {
    setRecentMovies(removeRecentMovie(url));
  }, []);

  const onClearRecent = useCallback(() => {
    if (!window.confirm(`Remove all ${loadRecentMovies().length} saved movies?`)) return;
    setRecentMovies(clearRecentMovies());
  }, []);

  const filterTokens = useMemo(
    () => recentFilter.toLowerCase().split(/\s+/).filter(Boolean),
    [recentFilter],
  );
  const isFiltering = filterTokens.length > 0;

  const filteredMovies = useMemo(
    () => recentMovies.filter(m => matchesFilter(m, filterTokens)),
    [recentMovies, filterTokens],
  );
  const groups = useMemo(() => groupMoviesByFolder(filteredMovies), [filteredMovies]);

  // A folder stays expanded while filtering: hiding matches behind a collapsed
  // header would make the search look like it found nothing.
  const isFolderCollapsed = useCallback(
    (dir: string) => !isFiltering && collapsedFolders.has(dir),
    [isFiltering, collapsedFolders],
  );

  const toggleFolder = useCallback((dir: string) => {
    setCollapsedFolders(prev => {
      const next = new Set(prev);
      if (!next.delete(dir)) next.add(dir);
      return next;
    });
  }, []);

  // Flattened view of everything currently rendered, in visual order — the
  // index space the arrow keys move through.
  const visibleMovies = useMemo(
    () => groups.flatMap(g => isFolderCollapsed(g.dir) ? [] : g.movies.map(m => m.movie)),
    [groups, isFolderCollapsed],
  );

  // The highlighted row must not survive a change to what is on screen,
  // otherwise Enter would load whatever entry slid into that index.
  useEffect(() => { setActiveIndex(-1); }, [recentFilter, collapsedFolders, recentMovies]);

  useEffect(() => {
    activeItemRef.current?.scrollIntoView({ block: 'nearest' });
  }, [activeIndex]);

  const onRecentKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (visibleMovies.length === 0) return;
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const delta = e.key === 'ArrowDown' ? 1 : -1;
      setActiveIndex(prev => {
        const next = prev + delta;
        if (next < 0) return visibleMovies.length - 1;
        if (next >= visibleMovies.length) return 0;
        return next;
      });
    } else if (e.key === 'Enter' && activeIndex >= 0) {
      e.preventDefault();
      onLoadRecent(visibleMovies[activeIndex]);
    } else if (e.key === 'Escape') {
      setRecentFilter('');
      setActiveIndex(-1);
    }
  }, [visibleMovies, activeIndex, onLoadRecent]);

  // A failed load never resolves whenMovieLoaded(), so loadMovieFile's `finally`
  // never runs and the form would stay disabled behind a stuck "Loading...".
  // The error landing in the store is the signal that the attempt is over.
  useEffect(() => {
    if (movieLoadError) setIsLoading(false);
  }, [movieLoadError]);

  useMountEffect(async () => {
    if (movieUrl && process.env.REACT_APP_MOVIE_AUTO_LOAD === 'true' && !isDebugSession()) {
      await loadMovieFile(movieUrl);
    }
  });

  const hasParams = externalParams.length > 0;

  // First run (or after Clear all) there is no list to fill the window, so the
  // form centers itself instead of hanging off the top edge.
  return <div className={`${styles.container} ${recentMovies.length === 0 ? styles.containerEmpty : ''}`}>
    <div className={styles.topPane}>
      <div className={styles.header}>
        <img className={styles.logo} src={logoUrl} alt="" width={40} height={40} />
        <div className={styles.brandText}>
          <h1 className={styles.title}>DirPlayer</h1>
          <div className={styles.subtitle}>Load Movie</div>
        </div>
        <div className={styles.headerActions}>
          <ThemeToggle />
        </div>
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
                    Loader / archive entry URL (optional)
                  </label>
                  <input
                    id="loaderUrl"
                    type="text"
                    className={styles.input}
                    placeholder="Loader page, or an archive entry (ooooooooo.ooo/?id=…)"
                    value={loaderUrl}
                    onChange={e => setLoaderUrl(e.currentTarget.value)}
                    disabled={isLoading}
                  />
                  <div className={styles.proxyRow}>
                    <input
                      type="text"
                      className={styles.input}
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
                  <label
                    className={styles.checkboxContainer}
                    style={{ marginTop: 10 }}
                    title="When off, this movie's cross-origin fetches go direct. Enable only for games that need the proxy (e.g. Neopets DGS). Saved per entry."
                  >
                    <input
                      type="checkbox"
                      className={styles.checkbox}
                      checked={useCorsProxy}
                      onChange={e => setUseCorsProxy(e.currentTarget.checked)}
                      disabled={isLoading}
                    />
                    Use CORS proxy for this movie
                  </label>
                  <div className={styles.hintText}>
                    CORS proxy is <strong>opt-in per movie</strong> (default off) and
                    saved with each recent entry. Enable it only for games whose
                    cross-origin fetches need proxying. Fetch &amp; Load turns it on
                    automatically. Start the proxy first: <code>node cors-proxy.cjs</code>.
                  </div>
                  <div style={{ fontSize: '0.8em', color: '#888', marginTop: 4 }}>
                    Fetch &amp; Load also reads a <code>data-launch-command</code> from an
                    archive entry page (9o3o / Flashpoint) and applies the projector
                    arguments — the movie URL, <code>--setExternalParam</code> pairs, the
                    LeechProtectionRemovalHelp flags and the <code>--do</code> payload.
                    Many archived games are launched from a wrapper movie that does
                    nothing without them.
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

        </div>

        {/* Pinned outside the scrolling card body: the primary action and any
            load error must stay visible however long the form gets. */}
        <div className={styles.cardFooter}>
          {movieLoadError && (
            <div className={styles.errorBanner}>
              {/* Same warning mark as the full-screen ErrorOverlay. */}
              <svg className={styles.errorIcon} width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/>
                <line x1="12" y1="9" x2="12" y2="13"/>
                <line x1="12" y1="17" x2="12.01" y2="17"/>
              </svg>
              <span className={styles.errorMessage}>{movieLoadError}</span>
            </div>
          )}
          {/* Action bar: settings on the left, the commit action on the right. */}
          <div className={styles.footerRow}>
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
            <button className={styles.button} onClick={onLoadClick} disabled={isLoading}>
              {isLoading ? (
                <>
                  <span className={styles.spinner} />
                  Loading
                </>
              ) : (
                <>
                  Load Movie
                </>
              )}
            </button>
          </div>
        </div>
      </div>
    </div>

    {recentMovies.length > 0 && (
      <div className={styles.recentSection} onKeyDown={onRecentKeyDown}>
        <div className={styles.recentHeader}>
          <span className={styles.recentTitle}>Recent Movies</span>
          <span className={styles.recentCount}>
            {isFiltering
              ? `${filteredMovies.length} of ${recentMovies.length}`
              : recentMovies.length}
          </span>
          <button className={styles.clearRecent} onClick={onClearRecent}>
            Clear all
          </button>
        </div>

        <div className={styles.searchRow}>
          <div className={styles.searchField}>
            <svg className={styles.searchIcon} width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" aria-hidden="true">
              <circle cx="11" cy="11" r="7" />
              <line x1="16.5" y1="16.5" x2="21" y2="21" />
            </svg>
            <input
              type="text"
              className={styles.searchInput}
              placeholder="Filter by file name, folder or parameter..."
              value={recentFilter}
              onChange={e => setRecentFilter(e.currentTarget.value)}
              spellCheck={false}
            />
          </div>
          {isFiltering && (
            <button
              className={styles.clearSearchButton}
              onClick={() => setRecentFilter('')}
              title="Clear filter"
            >
              &#10005;
            </button>
          )}
        </div>

        <div className={styles.recentList} tabIndex={0}>
          {groups.length === 0 && (
            <div className={styles.recentEmpty}>
              <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" aria-hidden="true">
                <circle cx="11" cy="11" r="7" />
                <line x1="16.5" y1="16.5" x2="21" y2="21" />
              </svg>
              <span>No movies match &ldquo;{recentFilter.trim()}&rdquo;</span>
            </div>
          )}
          {groups.map(group => {
            const collapsed = isFolderCollapsed(group.dir);
            const folder = splitFolderTail(group.dir);
            return (
              <div key={group.dir} className={styles.recentGroup}>
                <button
                  className={styles.groupHeader}
                  onClick={() => toggleFolder(group.dir)}
                  title={group.dir || 'No folder'}
                >
                  <span className={`${styles.groupArrow} ${collapsed ? '' : styles.groupArrowOpen}`}>
                    &#9654;
                  </span>
                  <span className={styles.groupPath}>
                    <span className={styles.groupPathHead}>
                      <span className={styles.groupPathHeadText}>
                        {folder.head || group.dir || 'No folder'}
                      </span>
                    </span>
                    {folder.head && <span className={styles.groupPathTail}>{folder.tail}</span>}
                  </span>
                  <span className={styles.groupCount}>{group.movies.length}</span>
                </button>
                {!collapsed && <div className={styles.groupRows}>
                  {group.movies.map(({ movie, parts }) => {
                  const isActive = visibleMovies[activeIndex] === movie;
                  const tags = movieTags(movie);
                  const shownTags = tags.slice(0, MAX_ROW_TAGS);
                  const hiddenTags = tags.slice(MAX_ROW_TAGS);
                  return (
                    <div
                      key={movie.url}
                      ref={isActive ? activeItemRef : undefined}
                      className={`${styles.recentItem} ${isActive ? styles.recentItemActive : ''}`}
                      onClick={() => onEditRecent(movie)}
                      onDoubleClick={() => onLoadRecent(movie)}
                      title={movie.url}
                    >
                      <div className={styles.recentItemBody}>
                        <div className={styles.recentItemTitleRow}>
                          <span className={styles.recentFileName}>{parts.file}</span>
                          {parts.query && <span className={styles.recentQuery}>{parts.query}</span>}
                          <span className={styles.recentTime}>{formatRelativeTime(movie.timestamp)}</span>
                        </div>
                        {tags.length > 0 && (
                          <div className={styles.recentParams}>
                            {shownTags.map(tag => (
                              <span
                                key={tag.key}
                                className={`${styles.paramTag} ${tag.accent ? styles.proxyTag : ''}`}
                                title={tag.label}
                              >
                                {tag.label}
                              </span>
                            ))}
                            {hiddenTags.length > 0 && (
                              <span
                                className={styles.paramTag}
                                title={hiddenTags.map(t => t.label).join('\n')}
                              >
                                +{hiddenTags.length}
                              </span>
                            )}
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
                  );
                  })}
                </div>}
              </div>
            );
          })}
        </div>
        <div className={styles.recentHint}>
          Click an entry to edit it, double-click or <kbd>Enter</kbd> to load.
          {' '}<kbd>&#8593;</kbd><kbd>&#8595;</kbd> to navigate.
        </div>
      </div>
    )}
  </div>;
}
