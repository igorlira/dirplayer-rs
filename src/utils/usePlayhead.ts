import { RefObject, useEffect } from 'react';
import store from '../store';

/**
 * Drives a `--playhead-frame` CSS variable from the VM's current frame without
 * re-rendering anything.
 *
 * The frame changes on every tick of the movie — up to the movie's tempo, and
 * the draw loop runs as fast as 240fps. Reading it through useSelector would
 * re-render the score views (hundreds of cells) at that rate. Instead this
 * subscribes to the store directly and writes one custom property, coalesced to
 * one write per animation frame; CSS positions the playhead from there.
 *
 * Returns nothing: consumers position an element with
 * `calc((var(--playhead-frame) - 1) * var(--frame-cell-width))`.
 */
export function usePlayheadVar(ref: RefObject<HTMLElement | null>) {
  useEffect(() => {
    let frameRequest = 0;
    let pending: number | null = null;
    let lastWritten: number | null = null;

    const flush = () => {
      frameRequest = 0;
      if (pending === null || !ref.current) return;
      ref.current.style.setProperty('--playhead-frame', String(pending));
      lastWritten = pending;
      pending = null;
    };

    const schedule = (frame: number) => {
      if (frame === lastWritten) return;
      pending = frame;
      if (!frameRequest) frameRequest = requestAnimationFrame(flush);
    };

    schedule(store.getState().vm.currentFrame);
    flush();

    const unsubscribe = store.subscribe(() => {
      schedule(store.getState().vm.currentFrame);
    });

    return () => {
      unsubscribe();
      if (frameRequest) cancelAnimationFrame(frameRequest);
    };
  }, [ref]);
}
