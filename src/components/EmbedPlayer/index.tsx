import { useCallback, useEffect, useMemo, useState } from 'react';
import { RootState } from '../../store';
import { useSelector } from 'react-redux'
import { load_movie_file, play, set_base_path, set_external_params } from 'vm-rust';
import { getExternalXtrasReady, resolveAndLoadMovieXtras, setXtraMovieBase, whenMovieLoaded } from 'dirplayer-js-api';
import { getFullPathFromOrigin, getBasePath } from '../../utils/path';
import { initAudioBackend } from '../../audio/audioInit';
import Stage from '../../views/Stage';
import ShadowPortal from '../ShadowPortal';
import ErrorOverlay from '../ErrorOverlay';

type EmbedPlayerProps = {
  width: string
  height: string
  src: string
  externalParams?: Record<string, string>
  requireClickToPlay?: boolean
  enableGestures?: boolean
};

export default function EmbedPlayer({width, height, src, externalParams, requireClickToPlay, enableGestures}: EmbedPlayerProps) {
  const isVmReady = useSelector<RootState>(state => state.vm.isReady);
  const movieLoadError = useSelector<RootState, string | undefined>(state => state.vm.movieLoadError);
  const [userClicked, setUserClicked] = useState(!requireClickToPlay);

  const normalizeCssSize = useCallback((value: string) => {
    const trimmed = value.trim();
    return /^\d+(?:\.\d+)?$/.test(trimmed) ? `${trimmed}px` : trimmed;
  }, []);

  useEffect(() => {
    async function loadMovie() {
      const fullPath = getFullPathFromOrigin(src);
      const moviePath = getBasePath(fullPath);
      set_base_path(moviePath);
      // Bare xtra filenames resolve against this movie's directory.
      setXtraMovieBase(moviePath);
      set_external_params(externalParams || {});
      // Boot-time eager xtras must be loaded first.
      await getExternalXtrasReady();
      // Always load with autoplay=false so the XTRl chunk is parsed
      // BEFORE Lingo runs; resolve through the registry, then play().
      // load_movie_file is fire-and-forget — wait on onMovieLoaded
      // before trying to read the parsed XTRl.
      const movieLoadedPromise = whenMovieLoaded();
      await load_movie_file(fullPath, false);
      await movieLoadedPromise;
      await resolveAndLoadMovieXtras();
      play();
    }
    if (isVmReady && userClicked) {
      loadMovie().catch(e => console.error('Failed to load movie', e))
    }
  }, [isVmReady, userClicked]) // TODO: Update player when src/params change

  const handleClick = useCallback(() => {
    initAudioBackend();
    setUserClicked(true);
  }, []);

  const [widthValue, heightValue] = useMemo(
    () => [normalizeCssSize(width), normalizeCssSize(height)],
    [height, normalizeCssSize, width]
  );

  if (!userClicked) {
    return (
      <div
        onClick={handleClick}
        style={{
          width: widthValue,
          height: heightValue,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          backgroundColor: '#000',
          cursor: 'pointer',
          position: 'relative',
        }}
      >
        <div style={{
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          gap: '12px',
          color: '#fff',
          userSelect: 'none',
        }}>
          <svg width="64" height="64" viewBox="0 0 64 64" fill="none">
            <circle cx="32" cy="32" r="30" stroke="#fff" strokeWidth="3" fill="rgba(255,255,255,0.1)" />
            <polygon points="26,20 26,44 46,32" fill="#fff" />
          </svg>
          <span style={{ fontSize: '14px', fontFamily: 'sans-serif', opacity: 0.8 }}>
            Click to Play
          </span>
        </div>
      </div>
    );
  }

  return (
    <div style={{ width: widthValue, height: heightValue, position: 'relative', backgroundColor: '#000' }}>
      {!!isVmReady && !movieLoadError && <Stage enableGestures={enableGestures} />}
      {movieLoadError && (
        <ShadowPortal style={{ position: 'absolute', inset: 0, zIndex: 9999 }}>
          <ErrorOverlay message={movieLoadError} compact />
        </ShadowPortal>
      )}
    </div>
  );
}
