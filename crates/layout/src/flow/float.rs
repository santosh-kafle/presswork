//! Floats: boxes pushed to the left or right of their containing block,
//! with the lines after them flowing around them (CSS 2.1 section 9.5).
//!
//! A float is laid out on its own, like an inline-block (tables and
//! images keep their own sizing), and placed as one unbreakable slice at
//! the current position: as high as it fits beside earlier floats, else
//! below them. The slices that follow while it is beside them stay on its
//! page if they can. Lines are broken one at a time in the space the
//! floats leave at their height.
//!
//! Simplification: a float in the middle of a paragraph is placed at the
//! top of the paragraph, not of the line it is on.

use parley::{Layout, YieldData};
use presswork_dom::NodeId;
use presswork_style::{Clear, ComputedStyle, Display, Float, VerticalAlign};

use super::inline::{FIT_TOLERANCE, Ink, InlineObject};
use super::{Flow, Keep, Slice, Source, replaced};

/// A placed float: its margin box, in canvas coordinates.
#[derive(Debug, Clone, Copy)]
pub(super) struct FloatBox {
    side: Float,
    left: f32,
    right: f32,
    top: f64,
    bottom: f64,
}

impl FloatBox {
    pub(super) fn bottom(&self) -> f64 {
        self.bottom
    }
}

impl Flow<'_, '_> {
    /// Lays out the floated element `id` in a containing block whose
    /// content box starts at `x` and is `width` wide, and places it.
    pub(super) fn float_element(&mut self, id: NodeId, style: &ComputedStyle, x: f32, width: f32) {
        let object = if replaced::is_image(self.cx.doc, id) {
            match self.image_object(id, style, width) {
                Ok(object) => object,
                // An image without a size or data floats nothing.
                Err(_) => return,
            }
        } else if style.display == Display::Table {
            self.table_object(id, style, width)
        } else {
            self.atomic_box(Source::Element(id), style, width)
        };
        if self.is_measuring() {
            self.record_extent(object.min_width, object.width);
            return;
        }
        let (width_needed, height) = (object.width, object.height);
        let top = self.y + f64::from(self.margin);
        let (left, top) = self.place_float(style.float, width_needed, height, top, x, width);
        self.floats.push(FloatBox {
            side: style.float,
            left,
            right: left + width_needed,
            top,
            bottom: top + f64::from(height),
        });
        self.float_keep_until = self.float_keep_until.max(top + f64::from(height));
        let items = object
            .items
            .into_iter()
            .map(|item| item.translate(left, 0.0))
            .collect();
        // Out of the flow: no margins, breaks or marks of its own.
        self.canvas.slices.push(Slice {
            top,
            height,
            items,
            break_before: false,
            keep: Keep::default(),
            header: None,
            on_top: false,
        });
        self.unsorted = true;
    }

    /// A table as an atomic box: as wide as its content wants, but no
    /// wider than `width`.
    pub(super) fn table_object(
        &mut self,
        id: NodeId,
        style: &ComputedStyle,
        width: f32,
    ) -> InlineObject {
        let mut probe = Flow::measuring(self.cx);
        probe.table(id, style, 0.0, width);
        let extent = probe.extent();
        let used = extent.max.min(width).max(extent.min);
        if self.is_measuring() {
            return InlineObject::new(
                extent.max,
                0.0,
                extent.min,
                VerticalAlign::Baseline,
                Vec::new(),
            );
        }
        let mut flow = Flow::new(self.cx);
        flow.table(id, style, 0.0, used);
        let (canvas, height) = flow.finish();
        let mut items = Vec::new();
        canvas.into_items(0.0, 0.0, &mut items);
        InlineObject::new(used, height as f32, used, VerticalAlign::Baseline, items)
    }

    /// Where a float `width` × `height` on `side` goes, at or below `top`,
    /// in a containing block from `x`, `available` wide: the left edge of
    /// its margin box, and its top.
    fn place_float(
        &self,
        side: Float,
        width: f32,
        height: f32,
        top: f64,
        x: f32,
        available: f32,
    ) -> (f32, f64) {
        // Not above an earlier float.
        let mut y = self.floats.iter().map(|f| f.top).fold(top, f64::max);
        let height = f64::from(height);
        loop {
            let (left, right) = self.band(y, y + height, x, available);
            let blocking = self
                .floats
                .iter()
                .filter(|f| f.top < y + height.max(0.01) && f.bottom > y);
            if right - left >= width - 0.01 {
                break;
            }
            // Below the first float in the way to end.
            match blocking.map(|f| f.bottom).reduce(f64::min) {
                Some(bottom) if bottom > y => y = bottom,
                _ => break,
            }
        }
        let (left, right) = self.band(y, y + height, x, available);
        let left = match side {
            Float::Right => right - width,
            _ => left,
        };
        (left, y)
    }

    /// The horizontal space between the floats beside `top..bottom`,
    /// within `x..x + width`.
    fn band(&self, top: f64, bottom: f64, x: f32, width: f32) -> (f32, f32) {
        let (mut left, mut right) = (x, x + width);
        let bottom = bottom.max(top + 0.01);
        for float in self
            .floats
            .iter()
            .filter(|f| f.top < bottom && f.bottom > top)
        {
            match float.side {
                Float::Right => right = right.min(float.left),
                _ => left = left.max(float.right),
            }
        }
        (left, right.max(left))
    }

    /// The nearest height below `y` where floats start or end.
    fn next_float_edge(&self, y: f64) -> Option<f64> {
        self.floats
            .iter()
            .flat_map(|f| [f.top, f.bottom])
            .filter(|&edge| edge > y)
            .reduce(f64::min)
    }

    /// Whether floats reach below `y`, so that content there flows around
    /// them.
    pub(super) fn floats_below(&self, y: f64) -> bool {
        self.floats.iter().any(|f| f.bottom > y)
    }

    /// The bottom of the lowest float, if any.
    pub(super) fn floats_bottom(&self) -> Option<f64> {
        self.floats.iter().map(|f| f.bottom).reduce(f64::max)
    }

    /// Moves below the floats that `clear` names, before a box.
    pub(super) fn clear_floats(&mut self, clear: Clear) {
        let bottom = self
            .floats
            .iter()
            .filter(|f| match clear {
                Clear::None => false,
                Clear::Both => true,
                Clear::Left => f.side == Float::Left,
                Clear::Right => f.side == Float::Right,
            })
            .map(|f| f.bottom)
            .reduce(f64::max);
        if let Some(bottom) = bottom
            && bottom > self.y + f64::from(self.margin)
        {
            self.settle_margin();
            self.y = self.y.max(bottom);
        }
    }

    /// Breaks the lines of a paragraph starting at `self.y`, in a
    /// containing block from `x`, `width` wide, around the floats.
    pub(super) fn break_around_floats(&self, layout: &mut Layout<Ink>, x: f32, width: f32) {
        let top = self.y;
        let mut breaker = layout.break_lines();
        breaker
            .state_mut()
            .set_layout_max_advance(width + FIT_TOLERANCE);
        // Each line breaks at most twice: once in the space at its top,
        // and again if it turns out taller than that space lasts.
        for _ in 0..1_000_000 {
            let y = breaker.committed_y();
            let line_top = top + y;
            let (left, right) = self.band(line_top, line_top, x, width);
            let lasts = self
                .next_float_edge(line_top)
                .map_or(f32::MAX, |edge| (edge - line_top) as f32);
            let saved = breaker.state().clone();
            let state = breaker.state_mut();
            state.set_line_x(left - x);
            state.set_line_max_advance(right - left + FIT_TOLERANCE);
            state.set_line_max_height(lasts);
            match breaker.break_next() {
                None => break,
                Some(YieldData::MaxHeightExceeded(data)) => {
                    // The line reaches the next float edge: break it again
                    // in the space left all the way down.
                    breaker.revert_to(saved);
                    let bottom = line_top + f64::from(data.line_height);
                    let (left, right) = self.band(line_top, bottom, x, width);
                    let state = breaker.state_mut();
                    state.set_line_x(left - x);
                    state.set_line_max_advance(right - left + FIT_TOLERANCE);
                    state.set_line_max_height(f32::MAX);
                    if breaker.break_next().is_none() {
                        break;
                    }
                }
                Some(_) => {}
            }
        }
        breaker.finish();
    }
}
