use log::warn;

use crate::console_warn;

#[derive(Clone, Debug)]
pub struct IntRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

pub type IntRectTuple = (i32, i32, i32, i32);

impl IntRect {
    pub const fn from(l: i32, t: i32, r: i32, b: i32) -> IntRect {
        return IntRect {
            left: l,
            top: t,
            right: r,
            bottom: b,
        };
    }

    pub const fn from_size(x: i32, y: i32, width: i32, height: i32) -> IntRect {
        return IntRect::from(x, y, x + width, y + height);
    }

    pub const fn from_tuple(rect: IntRectTuple) -> IntRect {
        return IntRect::from(rect.0, rect.1, rect.2, rect.3);
    }

    pub fn from_quad(
        top_left: (i32, i32),
        top_right: (i32, i32),
        bottom_right: (i32, i32),
        bottom_left: (i32, i32),
    ) -> IntRect {
        // For axis-aligned rectangles (including flipped ones):
        // - Top edge: top_left.y should equal top_right.y
        // - Bottom edge: bottom_left.y should equal bottom_right.y
        // - Left edge: top_left.x should equal bottom_left.x
        // - Right edge: top_right.x should equal bottom_right.x

        // Validate it's an axis-aligned quad
        if top_left.1 != top_right.1
            || bottom_left.1 != bottom_right.1
            || top_left.0 != bottom_left.0
            || top_right.0 != bottom_right.0
        {
            warn!("INVALID IntRect::from_quad - not axis-aligned: TL({}, {}), TR({}, {}), BR({}, {}), BL({}, {})", 
        top_left.0, top_left.1, top_right.0, top_right.1,
        bottom_right.0, bottom_right.1, bottom_left.0, bottom_left.1);
            return IntRect::from(0, 0, 0, 0);
        }

        // For flipped rectangles, the "left" might actually be on the right side
        // We need to preserve this to indicate flipping
        // Just use the actual corner positions regardless of whether they're "correct"
        IntRect {
            left: top_left.0,       // May be > right if horizontally flipped
            top: top_left.1,        // May be > bottom if vertically flipped
            right: top_right.0,     // May be < left if horizontally flipped
            bottom: bottom_right.1, // May be < top if vertically flipped
        }
    }

    pub const fn width(&self) -> i32 {
        return self.right - self.left;
    }

    pub const fn height(&self) -> i32 {
        return self.bottom - self.top;
    }

    pub fn intersect(&self, other: &IntRect) -> IntRect {
        let left = self.left.max(other.left);
        let top = self.top.max(other.top);
        let right = self.right.min(other.right);
        let bottom = self.bottom.min(other.bottom);

        if right < left || bottom < top {
            // No intersection
            return IntRect::from(0, 0, 0, 0);
        }

        return IntRect::from(left, top, right, bottom);
    }
}
