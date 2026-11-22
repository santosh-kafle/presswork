//! Positioned boxes (CSS 2.1 section 9.3).
//!
//! - `relative`: laid out in the flow, then its content is shifted.
//! - `absolute`: out of the flow, laid out on its own like an inline-block
//!   and placed against its containing block, the nearest positioned
//!   ancestor laid out in the same flow, or the flow's own area (the first
//!   page for the document). Placed when that block ends, once its height
//!   is known.
//! - `fixed`: placed against the page area, on every page: the way
//!   watermarks are made for wkhtmltopdf.
//!
//! Positioned boxes paint above the content in the flow, in document
//! order; `z-index` is not supported.

use presswork_dom::NodeId;
use presswork_style::{BoxSizing, ComputedStyle, Dimension, Display, Position};

use super::{Flow, Keep, Slice, Source, replaced};
use crate::fragment::Item;

/// A positioned box that absolutely positioned descendants are placed
/// against: its padding box, once it ends.
pub(super) struct ContainingBlock {
    /// Left edge and width of the padding box.
    x: f32,
    width: f32,
    /// The decoration that records where the box's border box starts and
    /// ends, and its top and bottom border widths.
    decoration: usize,
    borders: (f32, f32),
    pending: Vec<Absolute>,
}

/// An absolutely positioned box waiting for its containing block to end.
pub(super) struct Absolute {
    /// Relative to the box's top-left, which is at `x`.
    items: Vec<Item>,
    height: f32,
    x: f32,
    /// Where it would have been in the flow, for `top: auto; bottom: auto`.
    static_y: f64,
    top: Dimension,
    bottom: Dimension,
}

impl Flow<'_, '_> {
    /// Lays out the absolutely or fixed positioned element `id`, out of
    /// the flow, and places it or keeps it for its containing block. `x`
    /// is where the containing block of the flow starts: its static
    /// position.
    pub(super) fn positioned_element(&mut self, id: NodeId, style: &ComputedStyle, x: f32) {
        if self.is_measuring() {
            // Out of the flow: it takes no room.
            return;
        }
        let fixed = style.position == Position::Fixed;
        let (cb_x, cb_width) = if fixed {
            (0.0, self.cx.page.width)
        } else {
            self.positioned
                .last()
                .map_or((0.0, self.root_width), |cb| (cb.x, cb.width))
        };
        let inset = style.inset;
        let left = inset.left.resolve(cb_width);
        let right = inset.right.resolve(cb_width);

        // With both `left` and `right`, an `auto` width fills between them.
        let mut stretched;
        let mut used = style;
        if let (Some(left), Some(right), true) = (left, right, style.width.is_auto()) {
            let margins = style.margin.left.resolve(cb_width).unwrap_or(0.0)
                + style.margin.right.resolve(cb_width).unwrap_or(0.0);
            let edges = style.border_widths();
            let padding = style
                .padding
                .map(|p| p.resolve(cb_width).unwrap_or(0.0).max(0.0));
            let extra = match style.box_sizing {
                BoxSizing::ContentBox => edges.left + edges.right + padding.left + padding.right,
                BoxSizing::BorderBox => 0.0,
            };
            stretched = style.clone();
            stretched.width = Dimension::Pt((cb_width - left - right - margins - extra).max(0.0));
            used = &stretched;
        }
        let object = if replaced::is_image(self.cx.doc, id) {
            match self.image_object(id, used, cb_width) {
                Ok(object) => object,
                Err(_) => return,
            }
        } else if used.display == Display::Table {
            self.table_object(id, used, cb_width)
        } else {
            self.atomic_box(Source::Element(id), used, cb_width)
        };

        let x = match (left, right) {
            (Some(left), _) => cb_x + left,
            (None, Some(right)) => cb_x + cb_width - right - object.width,
            (None, None) => x,
        };
        let absolute = Absolute {
            items: object.items,
            height: object.height,
            x,
            static_y: self.y + f64::from(self.margin),
            top: inset.top,
            bottom: inset.bottom,
        };
        if fixed {
            let page = self.cx.page.height;
            let y = vertical(&absolute, 0.0, f64::from(page));
            let y = y as f32;
            let items = absolute.items.into_iter().map(|item| item.translate(x, y));
            self.cx.fixed.extend(items);
        } else if let Some(cb) = self.positioned.last_mut() {
            cb.pending.push(absolute);
        } else {
            self.root_absolutes.push(absolute);
        }
    }

    /// Starts a positioned box: absolutely positioned descendants are
    /// placed against its padding box, from `x`, `width` wide; its border
    /// box is recorded by decoration `decoration`.
    pub(super) fn open_containing_block(
        &mut self,
        x: f32,
        width: f32,
        decoration: usize,
        borders: (f32, f32),
    ) {
        self.positioned.push(ContainingBlock {
            x,
            width,
            decoration,
            borders,
            pending: Vec::new(),
        });
    }

    /// Ends the innermost positioned box: places the boxes waiting for it.
    pub(super) fn close_containing_block(&mut self) {
        let Some(cb) = self.positioned.pop() else {
            return;
        };
        let decoration = &self.canvas.decorations[cb.decoration];
        let top = decoration.top + f64::from(cb.borders.0);
        let bottom = (decoration.bottom - f64::from(cb.borders.1)).max(top);
        for absolute in cb.pending {
            self.place_absolute(absolute, top, bottom);
        }
    }

    /// Places the absolutely positioned boxes of the flow's own area,
    /// `top..bottom`.
    pub(super) fn place_root_absolutes(&mut self, top: f64, bottom: f64) {
        for absolute in std::mem::take(&mut self.root_absolutes) {
            self.place_absolute(absolute, top, bottom);
        }
    }

    fn place_absolute(&mut self, absolute: Absolute, top: f64, bottom: f64) {
        let y = top + vertical(&absolute, top, bottom);
        let x = absolute.x;
        let items = absolute
            .items
            .into_iter()
            .map(|item| item.translate(x, 0.0))
            .collect();
        self.canvas.slices.push(Slice {
            top: y,
            height: absolute.height,
            items,
            break_before: false,
            keep: Keep::default(),
            header: None,
            on_top: true,
        });
        self.unsorted = true;
    }

    /// Shifts what a relatively positioned box laid out by `top`, `right`,
    /// `bottom` and `left`: its slices from `first_slice` and decorations
    /// from `first_decoration`. Percentages are of `width`; vertical ones
    /// count as zero, as the containing block's height is not known yet.
    pub(super) fn shift_relative(
        &mut self,
        style: &ComputedStyle,
        width: f32,
        first_slice: usize,
        first_decoration: usize,
    ) {
        let inset = style.inset;
        let dx = match (inset.left.resolve(width), inset.right.resolve(width)) {
            (Some(left), _) => left,
            (None, Some(right)) => -right,
            (None, None) => 0.0,
        };
        let dy = match (inset.top.resolve(0.0), inset.bottom.resolve(0.0)) {
            (Some(top), _) => top,
            (None, Some(bottom)) => -bottom,
            (None, None) => 0.0,
        };
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        for slice in &mut self.canvas.slices[first_slice..] {
            let items = std::mem::take(&mut slice.items);
            slice.items = items
                .into_iter()
                .map(|item| item.translate(dx, dy))
                .collect();
        }
        for decoration in &mut self.canvas.decorations[first_decoration..] {
            decoration.x += dx;
            decoration.top += f64::from(dy);
            decoration.bottom += f64::from(dy);
        }
    }
}

/// The top of an absolutely positioned box, relative to its containing
/// block's top, which is `top..bottom` on the canvas.
fn vertical(absolute: &Absolute, top: f64, bottom: f64) -> f64 {
    let height = (bottom - top) as f32;
    match (
        absolute.top.resolve(height),
        absolute.bottom.resolve(height),
    ) {
        (Some(offset), _) => f64::from(offset),
        (None, Some(offset)) => f64::from(height - offset - absolute.height),
        (None, None) => absolute.static_y - top,
    }
}
