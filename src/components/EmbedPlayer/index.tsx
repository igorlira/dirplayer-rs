import { useCallback, useEffect, useMemo, useState } from 'react';
import { RootState } from '../../store';
import { useSelector } from 'react-redux'
import { load_movie_file, play, set_base_path, set_external_params } from 'vm-rust';
import { getFullPathFromOrigin, getBasePath } from '../../utils/path';
import { initAudioBackend } from '../../audio/audioInit';
import Stage from '../../views/Stage';

type EmbedPlayerProps = {
  width: string
  height: string
  src: string
  externalParams?: Record<string, string>
  requireClickToPlay?: boolean
};

export default function EmbedPlayer({width, height, src, externalParams, requireClickToPlay}: EmbedPlayerProps) {
  const isVmReady = useSelector<RootState>(state => state.vm.isReady);
  const [userClicked, setUserClicked] = useState(!requireClickToPlay);

  useEffect(() => {
    async function loadMovie() {
      const fullPath = getFullPathFromOrigin(src);
      set_base_path(getBasePath(fullPath));
      set_external_params(externalParams || {});
      await load_movie_file(fullPath, true);
    }
    if (isVmReady && userClicked) {
      loadMovie().catch(e => console.error('Failed to load movie', e))
    }
  }, [isVmReady, userClicked]) // TODO: Update player when src/params change

  const handleClick = useCallback(() => {
    initAudioBackend();
    setUserClicked(true);
  }, []);

  const [widthValue, heightValue] = useMemo(() => {
    const widthInt = parseInt(width);
    const heightInt = parseInt(height);
    if (isNaN(widthInt) || isNaN(heightInt)) {
      return [width, height];
    } else {
      return [`${widthInt}px`, `${heightInt}px`];
    }
  }, [width, height]);

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

  return <div style={{width: widthValue, height: heightValue}}>
    {!!isVmReady && <Stage />}
  </div>
}
