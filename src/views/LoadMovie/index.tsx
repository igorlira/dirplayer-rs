import { useCallback, useState } from 'react';
import styles from './styles.module.css';
import { load_movie_file, play, set_base_path } from 'vm-rust';
import { useMountEffect } from '../../utils/hooks';
import { isDebugSession } from '../../utils/debug';
import { getBasePath, getFullPathFromOrigin } from '../../utils/path';
import { isElectron, openFileDialog } from '../../utils/electron';

export default function LoadMovie() {
  const defaultMovieUrl = process.env.REACT_APP_MOVIE_URL ? getFullPathFromOrigin(process.env.REACT_APP_MOVIE_URL) : '';
  const [movieUrl, setMovieUrl] = useState<string>(defaultMovieUrl || '');
  const [isLoading, setIsLoading] = useState<boolean>(false);
  const [autoPlay, setAutoPlay] = useState<boolean>(process.env.REACT_APP_MOVIE_AUTO_PLAY === 'true');
  const isInElectron = isElectron();

  const loadMovieFile = useCallback(async (fullPath: string) => {
    try {
      setIsLoading(true);
      set_base_path(getBasePath(fullPath));
      await load_movie_file(fullPath);
      if (autoPlay) {
        play();
      }
    } catch (e) {
      console.error('Failed to load movie', e);
    } finally {
      setIsLoading(false);
    }
  }, [autoPlay]);

  const onLoadClick = useCallback(async () => {
    await loadMovieFile(movieUrl);
  }, [movieUrl, loadMovieFile]);

  const onBrowseClick = useCallback(async () => {
    if (!isInElectron) return;

    try {
      const filePath = await openFileDialog();
      if (filePath) {
        // Convert to file:// URL format
        const fileUrl = `file://${filePath}`;
        setMovieUrl(fileUrl);
      }
    } catch (e) {
      console.error('[LoadMovie] Failed to open file dialog', e);
    }
  }, [isInElectron]);

  useMountEffect(async () => {
    if (movieUrl && process.env.REACT_APP_MOVIE_AUTO_LOAD === 'true' && !isDebugSession()) {
      await loadMovieFile(movieUrl);
    }
  });

  return <div className={styles.container}>
    <div className={styles.body}>
      <div className={styles.fieldContainer}>
        <label htmlFor="url">{isInElectron ? 'Movie Path' : 'Movie URL'}</label>
        <div className={styles.inputGroup}>
          <input
            id="url"
            name="url"
            type="text"
            className={styles.input}
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
      <div className={styles.checkboxContainer}>
        <input
          type="checkbox"
          id="autoPlay"
          name="autoPlay"
          className={styles.checkbox}
          disabled={isLoading}
          checked={autoPlay}
          onChange={e => setAutoPlay(e.currentTarget.checked)}
        />
        <label htmlFor="autoPlay">Auto-play</label>
      </div>
      <div className={styles.divider}></div>
      <button className={styles.button} onClick={onLoadClick} disabled={isLoading}>Load</button>
    </div>
  </div>;
}
