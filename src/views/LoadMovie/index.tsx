import { useCallback, useState } from 'react';
import styles from './styles.module.css';
import { load_movie_file, play, set_base_path, set_external_params } from 'vm-rust';
import { useMountEffect } from '../../utils/hooks';
import { isDebugSession } from '../../utils/debug';
import { getBasePath, getFullPathFromOrigin } from '../../utils/path';
import { isElectron, openFileDialog } from '../../utils/electron';

type ExternalParam = { key: string; value: string };

type RecentMovie = {
  url: string;
  params: ExternalParam[];
  timestamp: number;
  fileHandle?: BrowserFileHandle;
};

type BrowserFileHandle = {
  kind: 'file';
  name: string;
  queryPermission?: (descriptor?: { mode?: 'read' | 'readwrite' }) => Promise<'granted' | 'denied' | 'prompt'>;
  requestPermission?: (descriptor?: { mode?: 'read' | 'readwrite' }) => Promise<'granted' | 'denied' | 'prompt'>;
  getFile: () => Promise<File>;
};

type ShowOpenFilePicker = (options?: {
  multiple?: boolean;
  excludeAcceptAllOption?: boolean;
  types?: Array<{
    description?: string;
    accept: Record<string, string[]>;
  }>;
}) => Promise<BrowserFileHandle[]>;

const RECENT_MOVIES_DB_NAME = 'dirplayer';
const RECENT_MOVIES_STORE_NAME = 'recentMovies';
const RECENT_MOVIES_DB_VERSION = 1;
const RECENT_MOVIES_RECORD_ID = 'movies';
const MAX_RECENT_MOVIES = 50;

let recentMoviesDbPromise: Promise<IDBDatabase> | null = null;

function paramsArrayToRecord(params: ExternalParam[]): Record<string, string> {
  const record: Record<string, string> = {};
  for (const p of params) {
    if (p.key.trim()) {
      record[p.key.trim()] = p.value;
    }
  }
  return record;
}

function openRecentMoviesDb(): Promise<IDBDatabase> {
  if (recentMoviesDbPromise) {
    return recentMoviesDbPromise;
  }

  recentMoviesDbPromise = new Promise((resolve, reject) => {
    const request = window.indexedDB.open(RECENT_MOVIES_DB_NAME, RECENT_MOVIES_DB_VERSION);

    request.onupgradeneeded = () => {
      const db = request.result;
      if (!db.objectStoreNames.contains(RECENT_MOVIES_STORE_NAME)) {
        db.createObjectStore(RECENT_MOVIES_STORE_NAME, { keyPath: 'id' });
      }
    };

    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error ?? new Error('Failed to open IndexedDB'));
  });

  return recentMoviesDbPromise;
}

async function loadRecentMoviesIndexedDb(): Promise<RecentMovie[]> {
  try {
    const db = await openRecentMoviesDb();
    return await new Promise((resolve, reject) => {
      const tx = db.transaction(RECENT_MOVIES_STORE_NAME, 'readonly');
      const store = tx.objectStore(RECENT_MOVIES_STORE_NAME);
      const request = store.get(RECENT_MOVIES_RECORD_ID);

      request.onsuccess = () => {
        const result = request.result as { id: string; movies: RecentMovie[] } | undefined;
        resolve(result?.movies ?? []);
      };
      request.onerror = () => reject(request.error ?? new Error('Failed to read recent movies'));
    });
  } catch {
    return [];
  }
}

async function saveRecentMoviesIndexedDb(movies: RecentMovie[]): Promise<void> {
  const db = await openRecentMoviesDb();
  await new Promise<void>((resolve, reject) => {
    const tx = db.transaction(RECENT_MOVIES_STORE_NAME, 'readwrite');
    const store = tx.objectStore(RECENT_MOVIES_STORE_NAME);
    const request = store.put({ id: RECENT_MOVIES_RECORD_ID, movies });

    request.onsuccess = () => resolve();
    request.onerror = () => reject(request.error ?? new Error('Failed to write recent movies'));
  });
}

async function saveRecentMovieIndexedDb(url: string, params: ExternalParam[], fileHandle?: BrowserFileHandle): Promise<RecentMovie[]> {
  const existing = (await loadRecentMoviesIndexedDb()).filter(m => m.url !== url);
  const updated = [{ url, params, timestamp: Date.now(), fileHandle }, ...existing].slice(0, MAX_RECENT_MOVIES);
  await saveRecentMoviesIndexedDb(updated);
  return updated;
}

async function removeRecentMovieIndexedDb(url: string): Promise<RecentMovie[]> {
  const updated = (await loadRecentMoviesIndexedDb()).filter(m => m.url !== url);
  await saveRecentMoviesIndexedDb(updated);
  return updated;
}

async function clearRecentMoviesIndexedDb(): Promise<RecentMovie[]> {
  await saveRecentMoviesIndexedDb([]);
  return [];
}

export default function LoadMovie() {
  const defaultMovieUrl = process.env.REACT_APP_MOVIE_URL ? getFullPathFromOrigin(process.env.REACT_APP_MOVIE_URL) : '';
  const [movieUrl, setMovieUrl] = useState<string>(defaultMovieUrl || '');
  const [isLoading, setIsLoading] = useState<boolean>(false);
  const [autoPlay, setAutoPlay] = useState<boolean>(process.env.REACT_APP_MOVIE_AUTO_PLAY === 'true');
  const [externalParams, setExternalParams] = useState<ExternalParam[]>([]);
  const [recentMovies, setRecentMovies] = useState<RecentMovie[]>([]);
  const [selectedFileHandle, setSelectedFileHandle] = useState<BrowserFileHandle | undefined>(undefined);
  const [paramsExpanded, setParamsExpanded] = useState(false);
  const isInElectron = isElectron();
  const showOpenFilePickerFn = (window as Window & { showOpenFilePicker?: ShowOpenFilePicker }).showOpenFilePicker;
  const supportsWebBrowse = !isInElectron && typeof showOpenFilePickerFn === 'function';

  const persistRecentMovie = useCallback(async (url: string, params: ExternalParam[], fileHandle?: BrowserFileHandle) => {
    return saveRecentMovieIndexedDb(url, params, fileHandle);
  }, []);

  const removeRecentMoviePersisted = useCallback(async (url: string) => {
    return removeRecentMovieIndexedDb(url);
  }, []);

  const clearRecentMoviesPersisted = useCallback(async () => {
    return clearRecentMoviesIndexedDb();
  }, []);

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

  const loadMovieFile = useCallback(async (fullPath: string, params?: ExternalParam[]) => {
    try {
      setIsLoading(true);
      set_base_path(getBasePath(fullPath));
      set_external_params(paramsArrayToRecord(params ?? externalParams));
      await load_movie_file(fullPath, autoPlay);
    } catch (e) {
      console.error('Failed to load movie', e);
    } finally {
      setIsLoading(false);
    }
  }, [autoPlay, externalParams]);

  const loadMovieFromFileHandle = useCallback(async (fileHandle: BrowserFileHandle, params?: ExternalParam[]) => {
    const currentPermission = await fileHandle.queryPermission?.({ mode: 'read' });
    if (currentPermission !== 'granted') {
      const requestedPermission = await fileHandle.requestPermission?.({ mode: 'read' });
      if (requestedPermission !== 'granted') {
        console.warn('[LoadMovie] Read permission was not granted for selected file handle');
        return;
      }
    }

    const file = await fileHandle.getFile();
    const objectUrl = URL.createObjectURL(file);
    const urlForUi = file.name;
    const updated = await persistRecentMovie(urlForUi, params ?? externalParams, fileHandle);
    setRecentMovies(updated);
    await loadMovieFile(objectUrl, params);
  }, [externalParams, loadMovieFile, persistRecentMovie]);

  const onLoadClick = useCallback(async () => {
    if (!isInElectron && selectedFileHandle) {
      await loadMovieFromFileHandle(selectedFileHandle, externalParams);
      return;
    }

    const updated = await persistRecentMovie(movieUrl, externalParams);
    setRecentMovies(updated);
    await loadMovieFile(movieUrl);
  }, [externalParams, isInElectron, loadMovieFile, loadMovieFromFileHandle, movieUrl, persistRecentMovie, selectedFileHandle]);

  const onBrowseClick = useCallback(async () => {
    try {
      if (isInElectron) {
        const filePath = await openFileDialog();
        if (filePath) {
          setSelectedFileHandle(undefined);
          setMovieUrl(`file://${filePath}`);
        }
        return;
      }

      if (!showOpenFilePickerFn) {
        return;
      }

      const [fileHandle] = await showOpenFilePickerFn({
        multiple: false,
      });

      if (fileHandle) {
        setSelectedFileHandle(fileHandle);
        setMovieUrl(fileHandle.name);
      }
    } catch (e) {
      console.error('[LoadMovie] Failed to open file dialog', e);
    }
  }, [isInElectron, showOpenFilePickerFn]);

  const onLoadRecent = useCallback(async (movie: RecentMovie) => {
    setMovieUrl(movie.url);
    setExternalParams(movie.params);
    if (!isInElectron && movie.fileHandle) {
      setSelectedFileHandle(movie.fileHandle);
      await loadMovieFromFileHandle(movie.fileHandle, movie.params);
      return;
    }

    setSelectedFileHandle(undefined);
    const updated = await persistRecentMovie(movie.url, movie.params, movie.fileHandle);
    setRecentMovies(updated);
    await loadMovieFile(movie.url, movie.params);
  }, [isInElectron, loadMovieFile, loadMovieFromFileHandle, persistRecentMovie]);

  const onEditRecent = useCallback((movie: RecentMovie) => {
    setMovieUrl(movie.url);
    setExternalParams(movie.params);
    setSelectedFileHandle(movie.fileHandle);
    if (movie.params.length > 0) {
      setParamsExpanded(true);
    }
  }, []);

  const onRemoveRecent = useCallback(async (url: string) => {
    setRecentMovies(await removeRecentMoviePersisted(url));
  }, [removeRecentMoviePersisted]);

  const onClearRecent = useCallback(async () => {
    setRecentMovies(await clearRecentMoviesPersisted());
  }, [clearRecentMoviesPersisted]);

  useMountEffect(async () => {
    const initialRecentMovies = await loadRecentMoviesIndexedDb();
    setRecentMovies(initialRecentMovies);

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
              className={styles.input}
              placeholder={isInElectron ? '/path/to/movie.dcr' : 'https://example.com/movie.dcr'}
              value={movieUrl}
              onChange={e => {
                setMovieUrl(e.currentTarget.value);
                if (!isInElectron) {
                  setSelectedFileHandle(undefined);
                }
              }}
              disabled={isLoading}
            />
            {(isInElectron || supportsWebBrowse) && (
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
            External Params
            {hasParams && !paramsExpanded && (
              <span> ({externalParams.length})</span>
            )}
          </button>
          {paramsExpanded && (
            <div className={styles.paramsList}>
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
                {movie.params.length > 0 && (
                  <div className={styles.recentParams}>
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
