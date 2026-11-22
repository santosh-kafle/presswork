//! Geometry primitives in points.

/// A position in points, origin at the top-left of the page.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// A width and height in points.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

/// An axis-aligned rectangle.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    pub origin: Point,
    pub size: Size,
}

impl Rect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            origin: Point { x, y },
            size: Size { width, height },
        }
    }

    /// Returns a rectangle shrunk by `edges` on each side.
    pub fn inset(self, edges: EdgeSizes) -> Self {
        Rect::new(
            self.origin.x + edges.left,
            self.origin.y + edges.top,
            (self.size.width - edges.left - edges.right).max(0.0),
            (self.size.height - edges.top - edges.bottom).max(0.0),
        )
    }
}

/// Sizes for the four sides of a box (margin, border or padding).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct EdgeSizes {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl EdgeSizes {
    /// The same size on all four sides.
    pub const fn uniform(v: f32) -> Self {
        Self {
            top: v,
            right: v,
            bottom: v,
            left: v,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inset_shrinks_rect() {
        let r = Rect::new(0.0, 0.0, 100.0, 50.0).inset(EdgeSizes::uniform(10.0));
        assert_eq!(r, Rect::new(10.0, 10.0, 80.0, 30.0));
    }

    #[test]
    fn inset_never_goes_negative() {
        let r = Rect::new(0.0, 0.0, 10.0, 10.0).inset(EdgeSizes::uniform(20.0));
        assert_eq!(r.size, Size::default());
    }
}
