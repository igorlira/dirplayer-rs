import { useEffect } from "react";
import Stage from "../Stage";
import LoadMovie from "../LoadMovie";
import { useSelector } from "react-redux";
import { RootState } from "../../store";
import store from "../../store";
import { movieUnloaded } from "../../store/vmSlice";
import StudioLayout from "../StudioLayout";

interface DirStudioProps {
  showDebugUi?: boolean;
}

export default function DirStudio({
  showDebugUi,
}: DirStudioProps) {
  const isMovieLoaded = useSelector<RootState>((state) => state.vm.isMovieLoaded);

  useEffect(() => {
    const onPopState = () => {
      if (store.getState().vm.isMovieLoaded) {
        const handles = store.getState().vm.timeoutHandles;
        Object.values(handles).forEach((handle) => {
          clearInterval(handle as Parameters<typeof clearInterval>[0]);
        });

        store.dispatch(movieUnloaded());
        document.title = 'DirPlayer';
      }
    };
    window.addEventListener('popstate', onPopState);
    return () => window.removeEventListener('popstate', onPopState);
  }, []);

  if (!isMovieLoaded) {
    return <LoadMovie />;
  }
  if (!showDebugUi) {
    return <div style={{ width: '100vw', height: '100vh' }}>
      <Stage />
    </div>
  }
  return <StudioLayout />;
}
