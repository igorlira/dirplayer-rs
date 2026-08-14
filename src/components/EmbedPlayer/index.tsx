import { useCallback, useEffect, useMemo, useState } from 'react';
import { RootState } from '../../store';
import { useSelector } from 'react-redux'
import {
  load_movie_file, play, set_base_path, set_external_params,
  set_startup_do, set_startup_do_before, set_startup_go,
} from 'vm-rust';
import { parseLaunchCommand, findLaunchCommand } from '../../utils/launchCommand';
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
  /**
   * A Shockwave Projector (SPR.exe) launch command, as archive front-ends
   * publish in `data-launch-command`. When present its `--do` / `--doBefore` /
   * `--go` payloads and LeechProtectionRemovalHelp flags are applied before the
   * movie loads, and it supplies the movie URL if `src` is empty — many
   * archived entries only make sense started from their wrapper movie.
   * See docs/github_wiki/Projector-Launch-Commands.md.
   */
  launchCommand?: string
  requireClickToPlay?: boolean
  enableGestures?: boolean
};

export default function EmbedPlayer({width, height, src, externalParams, launchCommand, requireClickToPlay, enableGestures}: EmbedPlayerProps) {
  const isVmReady = useSelector<RootState>(state => state.vm.isReady);
  const movieLoadError = useSelector<RootState, string | undefined>(state => state.vm.movieLoadError);
  const [userClicked, setUserClicked] = useState(!requireClickToPlay);

  const normalizeCssSize = useCallback((value: string) => {
    const trimmed = value.trim();
    return /^\d+(?:\.\d+)?$/.test(trimmed) ? `${trimmed}px` : trimmed;
  }, []);

  useEffect(() => {
    async function loadMovie() {
      // A projector launch command, if the host page published one. It can
      // name the movie (archived entries are usually launched from a wrapper),
      // carry external params, and carry Lingo payloads that MUST be installed
      // before the load — the movie-init sequence consumes them.
      //
      // Falls back to scanning the page for `data-launch-command`, which is how
      // archive front-ends publish it. Doing the lookup HERE rather than
      // threading a prop down means both entry points get it: the polyfill
      // (core.tsx -> _renderPlayer) and the extension, which mount this
      // component through different call chains. The attribute is page-level,
      // and on such a front-end the embed IS the entry it describes.
      const command = launchCommand ?? findLaunchCommand(document) ?? null;
      const launch = command ? parseLaunchCommand(command) : null;

      const fullPath = getFullPathFromOrigin(src || launch?.movieUrl || '');
      const moviePath = getBasePath(fullPath);
      set_base_path(moviePath);
      // Bare xtra filenames resolve against this movie's directory.
      setXtraMovieBase(moviePath);
      // Embed-level params win over the launch command's: the host page's
      // `<param>` / `data-sw-…` values are the more specific statement of
      // intent, and a curator's command is the general one.
      set_external_params({ ...(launch?.externalParams || {}), ...(externalParams || {}) });
      if (launch) {
        if (launch.startupDoBefore) set_startup_do_before(launch.startupDoBefore);
        if (launch.startupDo) set_startup_do(launch.startupDo);
        if (launch.startupGo) set_startup_go(launch.startupGo);
        if (launch.ignored.length) {
          console.warn('[DirPlayer] launch command: ignored projector-only flags:', launch.ignored.join(', '));
        }
      }
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
