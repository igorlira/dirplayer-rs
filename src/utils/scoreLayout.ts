// Geometry shared by the score inspector's frame ruler and the timeline grid.
//
// The two are separate scroll containers stacked in the same panel, so a frame
// only lines up between them if they agree on the cell width *and* on the width
// of the channel gutter the timeline reserves on its left. Keeping the numbers
// here (rather than a constant per component with a "must match" comment) is
// what makes that alignment hold.
//
// The components publish these to CSS as custom properties, so the stylesheets
// read from the same source.

/** Width of one frame column. */
export const SCORE_CELL_WIDTH = 16;

/** The timeline's channel-number gutter; the ruler pads its left to match. */
export const SCORE_LABEL_WIDTH = 30;

/** Height of a timeline channel row. */
export const SCORE_ROW_HEIGHT = 18;

/** Height of a row in the (separate, taller) Channels list. */
export const SCORE_CHANNEL_ROW_HEIGHT = 20;

let cachedScrollbarSize: number | undefined;

/**
 * Height a horizontal scrollbar takes, measured once.
 *
 * The collapsed timeline is sized to its header, and on platforms with classic
 * (non-overlay) scrollbars the horizontal bar would otherwise eat into that
 * height and push the ruler out of view behind a vertical scrollbar. Overlay
 * scrollbars — macOS by default — measure 0, which is also correct.
 */
export function getScrollbarSize(): number {
  if (cachedScrollbarSize !== undefined) return cachedScrollbarSize;
  if (typeof document === 'undefined') return 0;
  const probe = document.createElement('div');
  probe.style.cssText =
    'position:absolute;top:-9999px;width:100px;height:100px;overflow:scroll;visibility:hidden';
  document.body.appendChild(probe);
  cachedScrollbarSize = probe.offsetHeight - probe.clientHeight;
  probe.remove();
  return cachedScrollbarSize;
}
