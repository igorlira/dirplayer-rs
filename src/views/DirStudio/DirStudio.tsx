import Stage from "../Stage";
import LoadMovie from "../LoadMovie";
import { useSelector } from "react-redux";
import { RootState } from "../../store";
import StudioLayout from "../StudioLayout";

interface DirStudioProps {
  showDebugUi?: boolean;
}

export default function DirStudio({
  showDebugUi,
}: DirStudioProps) {
  const isMovieLoaded = useSelector<RootState>((state) => state.vm.isMovieLoaded);

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
