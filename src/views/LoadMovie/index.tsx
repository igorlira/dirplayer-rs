import { useCallback, useState } from 'react';
import styles from './styles.module.css';
import { load_movie_file, play, set_base_path, set_external_params, set_movie_path_override } from 'vm-rust';
import { useMountEffect } from '../../utils/hooks';
import { isDebugSession } from '../../utils/debug';
import { getBasePath, getFullPathFromOrigin } from '../../utils/path';
import { isElectron, openFileDialog } from '../../utils/electron';

type ExternalParam = { key: string; value: string };

type RecentMovie = {
  url: string;
  params: ExternalParam[];
  fakeMoviePath?: string;
  timestamp: number;
};

const RECENT_MOVIES_KEY = 'recentMovies';
const MAX_RECENT_MOVIES = 50;

function paramsArrayToRecord(params: ExternalParam[]): Record<string, string> {
  const record: Record<string, string> = {};
  for (const p of params) {
    if (p.key.trim()) {
      record[p.key.trim()] = p.value;
    }
  }
  return record;
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
  const defaultMovieUrl = process.env.REACT_APP_MOVIE_URL ? getFullPathFromOrigin(process.env.REACT_APP_MOVIE_URL) : '';
  const [movieUrl, setMovieUrl] = useState<string>(defaultMovieUrl || '');
  const [isLoading, setIsLoading] = useState<boolean>(false);
  const [autoPlay, setAutoPlay] = useState<boolean>(process.env.REACT_APP_MOVIE_AUTO_PLAY === 'true');
  const [externalParams, setExternalParams] = useState<ExternalParam[]>([]);
  const [fakeMoviePath, setFakeMoviePath] = useState<string>('');
  const [recentMovies, setRecentMovies] = useState<RecentMovie[]>(() => loadRecentMovies());
  const [paramsExpanded, setParamsExpanded] = useState(false);
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
      set_base_path(getBasePath(fullPath));
      set_external_params(paramsArrayToRecord(params ?? externalParams));
      set_movie_path_override(fakePath ?? fakeMoviePath ?? '');
      await load_movie_file(fullPath, autoPlay);
    } catch (e) {
      console.error('Failed to load movie', e);
    } finally {
      setIsLoading(false);
    }
  }, [autoPlay, externalParams, fakeMoviePath]);

  const onLoadClick = useCallback(async () => {
    const updated = saveRecentMovie(movieUrl, externalParams, fakeMoviePath);
    setRecentMovies(updated);
    await loadMovieFile(movieUrl);
  }, [movieUrl, externalParams, fakeMoviePath, loadMovieFile]);

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
              className={styles.input}
              placeholder={isInElectron ? '/path/to/movie.dcr' : 'https://example.com/movie.dcr'}
              value={movieUrl}
              onChange={e => setMovieUrl(e.currentTarget.value)}
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
