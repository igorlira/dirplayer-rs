import Stage from "../Stage";
import LoadMovie from "../LoadMovie";
import { useSelector } from "react-redux";
import { RootState } from "../../store";
import StudioLayout from "../StudioLayout";
import ErrorOverlay from "../../components/ErrorOverlay";

interface DirStudioProps {
  showDebugUi?: boolean;
}

export default function DirStudio({
  showDebugUi,
}: DirStudioProps) {
  const isMovieLoaded = useSelector<RootState>((state) => state.vm.isMovieLoaded);
  const movieLoadError = useSelector<RootState, string | undefined>((state) => state.vm.movieLoadError);

  if (movieLoadError && !showDebugUi) {
    return <div style={{ width: '100vw', height: '100vh' }}>
      <ErrorOverlay message={movieLoadError} />
    </div>;
  }
  if (!isMovieLoaded) {
    return <LoadMovie />;
  }
  if (!showDebugUi) {
    return <div style={{ width: '100vw', height: '100vh' }}>
      <Stage enableGestures />
    </div>
  }
  return <StudioLayout />;
}
