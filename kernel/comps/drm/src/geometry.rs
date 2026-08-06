// SPDX-License-Identifier: MPL-2.0

/// Rectangles are checked by their right/bottom edges:
///
/// (x, y)        width        right = x + width
///    +-------------------------+
///    |                         |
///    |                         | height
///    |                         |
///    +-------------------------+
///                            bottom = y + height
///
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DrmRectU32 {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

impl DrmRectU32 {
    pub fn new(x: u32, y: u32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    pub fn x(&self) -> u32 {
        self.x
    }

    pub fn y(&self) -> u32 {
        self.y
    }

    pub fn width(&self) -> u32 {
        self.w
    }

    pub fn height(&self) -> u32 {
        self.h
    }

    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }

    pub fn right(&self) -> Option<u32> {
        self.x.checked_add(self.w)
    }

    pub fn bottom(&self) -> Option<u32> {
        self.y.checked_add(self.h)
    }

    /// Returns whether the point is inside the rectangle.
    ///
    /// The left/top edges are inclusive and the right/bottom edges are exclusive.
    pub fn contains_point(&self, x: u32, y: u32) -> bool {
        let Some(right) = self.right() else {
            return false;
        };
        let Some(bottom) = self.bottom() else {
            return false;
        };

        self.x <= x && x < right && self.y <= y && y < bottom
    }

    /// Returns whether `other` is fully contained within `self`.
    pub fn contains_rect(&self, other: &Self) -> bool {
        let Some(self_right) = self.right() else {
            return false;
        };
        let Some(self_bottom) = self.bottom() else {
            return false;
        };
        let Some(other_right) = other.right() else {
            return false;
        };
        let Some(other_bottom) = other.bottom() else {
            return false;
        };

        self.x <= other.x
            && self.y <= other.y
            && other_right <= self_right
            && other_bottom <= self_bottom
    }

    /// Returns whether the rectangle size is within the given inclusive limits.
    pub fn is_size_within(
        &self,
        min_width: u32,
        max_width: u32,
        min_height: u32,
        max_height: u32,
    ) -> bool {
        min_width <= self.w && self.w <= max_width && min_height <= self.h && self.h <= max_height
    }

    pub fn set_x(&mut self, x: u32) {
        self.x = x;
    }

    pub fn set_y(&mut self, y: u32) {
        self.y = y;
    }

    pub fn set_width(&mut self, w: u32) {
        self.w = w;
    }

    pub fn set_height(&mut self, h: u32) {
        self.h = h;
    }
}
