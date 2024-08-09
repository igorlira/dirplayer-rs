import { useEffect, useMemo } from 'react';
import { RootState } from '../../store';
import { useSelector } from 'react-redux'
import { load_movie_file, play, set_base_path, set_external_params } from 'vm-rust';
import { getFullPathFromOrigin, getBasePath } from '../../utils/path';
import Stage from '../../views/Stage';

type EmbedPlayerProps = {
  width: string
  height: string
  src: string
  externalParams?: Record<string, string>
};

export default function EmbedPlayer({width, height, src, externalParams}: EmbedPlayerProps) {
  const isVmReady = useSelector<RootState>(state => state.vm.isReady);
  useEffect(() => {
    async function loadMovie() {
      const fullPath = getFullPathFromOrigin(src);
      set_base_path(getBasePath(fullPath));
      set_external_params(externalParams || {});
      await load_movie_file(fullPath);
      play()
    }
    if (isVmReady) {
      loadMovie().catch(e => console.error('Failed to load movie', e))
    }
  }, [isVmReady]) // TODO: Update player when src/params change

  const [widthValue, heightValue] = useMemo(() => {
    const widthInt = parseInt(width);
    const heightInt = parseInt(height);
    if (isNaN(widthInt) || isNaN(heightInt)) {
      return [width, height];
    } else {
      return [`${widthInt}px`, `${heightInt}px`];
    }
  }, [width, height]);
  return <div style={{width: widthValue, height: heightValue}}>
    {!!isVmReady && <Stage />}
  </div>
}
