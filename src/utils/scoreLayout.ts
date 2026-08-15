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
