import { useEffect } from 'react';
import store from '../../store';
import { Provider as StoreProvider } from 'react-redux'
import { load_movie_file, play, set_base_path } from 'vm-rust';
import { getFullPathFromOrigin, getBasePath } from '../../utils/path';
import Stage from '../../views/Stage';

type EmbedPlayerProps = {
  width: string
  height: string
  src: string
};

export default function EmbedPlayer({width, height, src}: EmbedPlayerProps) {
  useEffect(() => {
    async function loadMovie() {
      const fullPath = getFullPathFromOrigin(src);
      set_base_path(getBasePath(fullPath));
      await load_movie_file(fullPath);
      play()
    }
    loadMovie().catch(e => console.error('Failed to load movie', e))
  })
  return <div style={{width, height}}>
    <StoreProvider store={store}>
      <Stage />
    </StoreProvider>
  </div>
}
