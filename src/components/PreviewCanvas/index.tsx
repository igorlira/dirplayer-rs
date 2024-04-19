import { useEffect, useState } from "react";
import { player_set_preview_parent } from "vm-rust";

export default function PreviewCanvas() {
  const [isMounted, setIsMounted] = useState(false);
  const onBitmapPreviewRef = (ref: HTMLDivElement | null) => {
    setIsMounted(!!ref);
  };
  useEffect(() => {
    if (isMounted) {
      player_set_preview_parent("#bitmapPreview");
    }
    return () => {
      player_set_preview_parent("");
    };
  }, [isMounted]);

  return <div id="bitmapPreview" ref={onBitmapPreviewRef}></div>;
}
