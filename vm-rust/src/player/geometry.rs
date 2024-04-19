use crate::console_warn;

pub struct IntRect {
  pub left: i16,
  pub top: i16,
  pub right: i16,
  pub bottom: i16,
}

pub type IntRectTuple = (i16, i16, i16, i16);

impl IntRect {
  pub const fn from(l: i16, t: i16, r: i16, b: i16) -> IntRect {
    return IntRect { left: l, top: t, right: r, bottom: b };
  }

  pub const fn from_size(x: i16, y: i16, width: i16, height: i16) -> IntRect {
    return IntRect::from(x, y, x + width, y + height);
  }

  pub const fn from_tuple(rect: IntRectTuple) -> IntRect {
    return IntRect::from(rect.0, rect.1, rect.2, rect.3);
  }

  pub fn from_quad(
    top_left: (i16, i16),
    top_right: (i16, i16),
    bottom_right: (i16, i16),
    bottom_left: (i16, i16),
  ) -> IntRect {
    if top_left.1 != top_right.1 || top_right.0 != bottom_right.0 || bottom_right.1 != bottom_left.1 || bottom_left.0 != top_left.0 {
      console_warn!("INVALID IntRect::from_quad(({}, {}), ({}, {}), ({}, {}), ({}, {}))", top_left.0, top_left.1, top_right.0, top_right.1, bottom_right.0, bottom_right.1, bottom_left.0, bottom_left.1);
      return IntRect::from(0, 0, 0, 0);
    }

    return IntRect {
      left: top_left.0,
      top: top_left.1,
      right: bottom_right.0,
      bottom: bottom_right.1,
    };
  }

  pub const fn width(&self) -> i16 {
    return self.right - self.left;
  }

  pub const fn height(&self) -> i16 {
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
