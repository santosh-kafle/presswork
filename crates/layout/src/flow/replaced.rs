//! Replaced elements: images.
//!
//! An `<img>` is sized from the image's natural size and the `width`,
//! `height`, `max-width` and `max-height` properties (CSS 2.1 sections
//! 10.3.2, 10.4 and 10.6.2), then placed inline, as an atomic box on a
//! line, or as a block of its own.
//!
//! An image that does not load keeps its box if it was given a size, and
//! is replaced by its `alt` text otherwise.

use std::sync::Arc;

use presswork_dom::{Document, NodeId};
use presswork_style::{BoxSizing, ComputedStyle, Dimension};

use super::Flow;
use super::boxes::BoxGeometry;
use super::inline::{Inline, InlineObject};
use crate::fragment::{Image, ImageData, Item};
use crate::geometry::{Rect, Size};

/// Whether `id` is an image element.
pub(super) fn is_image(doc: &Document, id: NodeId) -> bool {
    doc.element(id).is_some_and(|element| element.name == "img")
}

/// What an image element shows.
enum Content {
    /// The image at its used content size, or an empty box of the
    /// specified size if it did not load.
    Image {
        data: Option<Arc<ImageData>>,
        size: Size,
    },
    /// The image did not load and has no specified size: its `alt` text
    /// (possibly empty) takes its place.
    Alt(String),
}

/// Whether the image's width depends on its container's, so it can shrink
/// to fit a narrow table column.
fn shrinks(style: &ComputedStyle) -> bool {
    matches!(style.width, Dimension::Percent(_)) || matches!(style.max_width, Dimension::Percent(_))
}

/// The content size of a replaced element whose image is `natural` in
/// size (`None` if it has none), in a containing block `basis` wide.
/// `basis` is `None` while measuring, when percentages count as `auto`.
/// `edges` are the horizontal and vertical border plus padding, which
/// `box-sizing: border-box` takes off the specified sizes.
fn used_size(
    style: &ComputedStyle,
    natural: Option<Size>,
    basis: Option<f32>,
    edges: Size,
) -> Size {
    let content = |value: Dimension, basis: Option<f32>, edges: f32| -> Option<f32> {
        if matches!(value, Dimension::Percent(_)) && basis.is_none() {
            return None;
        }
        let value = value.resolve(basis.unwrap_or(0.0))?;
        Some(match style.box_sizing {
            BoxSizing::ContentBox => value,
            BoxSizing::BorderBox => (value - edges).max(0.0),
        })
    };
    // A percentage height needs a containing block of definite height,
    // which a page's flow does not have.
    let width = content(style.width, basis, edges.width);
    let height = content(style.height, None, edges.height);
    let max_width = content(style.max_width, basis, edges.width).unwrap_or(f32::INFINITY);
    let max_height = content(style.max_height, None, edges.height).unwrap_or(f32::INFINITY);

    let ratio = natural
        .filter(|n| n.width > 0.0 && n.height > 0.0)
        .map(|n| n.width / n.height);
    let natural = natural.unwrap_or(Size {
        width: 0.0,
        height: 0.0,
    });
    let (width, height) = match (width, height) {
        (Some(width), Some(height)) => (width.min(max_width), height.min(max_height)),
        (Some(width), None) => {
            let width = width.min(max_width);
            let height = ratio.map_or(natural.height, |ratio| width / ratio);
            (width, height.min(max_height))
        }
        (None, Some(height)) => {
            let height = height.min(max_height);
            let width = ratio.map_or(natural.width, |ratio| height * ratio);
            (width.min(max_width), height)
        }
        (None, None) => {
            // Scaled down as a whole to fit both limits, keeping its shape.
            let scale = (max_width / natural.width)
                .min(max_height / natural.height)
                .min(1.0);
            (natural.width * scale, natural.height * scale)
        }
    };
    Size {
        width: width.max(0.0),
        height: height.max(0.0),
    }
}

impl Flow<'_, '_> {
    /// Loads and sizes the image element `id` in a containing block `width`
    /// wide.
    fn image_content(&mut self, id: NodeId, style: &ComputedStyle, width: f32) -> Content {
        let doc = self.cx.doc;
        let element = doc.element(id).expect("an image is an element");
        let data = element.attr("src").and_then(|src| self.cx.image(src));
        if data.is_none() && style.width.is_auto() && style.height.is_auto() {
            return Content::Alt(element.attr("alt").unwrap_or_default().to_owned());
        }

        let basis = (!self.is_measuring()).then_some(width);
        let border = style.border_widths();
        let padding = style
            .padding
            .map(|p| p.resolve(basis.unwrap_or(0.0)).unwrap_or(0.0).max(0.0));
        let edges = Size {
            width: border.left + padding.left + padding.right + border.right,
            height: border.top + padding.top + padding.bottom + border.bottom,
        };
        let natural = data.as_ref().map(|data| data.natural_size());
        let size = used_size(style, natural, basis, edges);
        Content::Image { data, size }
    }

    /// Adds the image element `id` to the line content `inline`. `width`
    /// is the containing block's width.
    pub(super) fn inline_image(
        &mut self,
        id: NodeId,
        style: &ComputedStyle,
        width: f32,
        inline: &mut Inline,
    ) {
        match self.image_object(id, style, width) {
            Ok(object) => inline.push_box(object, style),
            Err(alt) => inline.push_text(&alt, style),
        }
    }

    /// The image element `id` as an atomic box, margins included, in a
    /// containing block `width` wide; or its alt text if it has no image
    /// and no size.
    pub(super) fn image_object(
        &mut self,
        id: NodeId,
        style: &ComputedStyle,
        width: f32,
    ) -> Result<InlineObject, String> {
        let (data, size) = match self.image_content(id, style, width) {
            Content::Image { data, size } => (data, size),
            Content::Alt(alt) => return Err(alt),
        };
        let basis = if self.is_measuring() { 0.0 } else { width };
        // Auto margins are zero on inline boxes.
        let margin = style.margin.map(|m| m.resolve(basis).unwrap_or(0.0));
        let border = style.border_widths();
        let padding = style
            .padding
            .map(|p| p.resolve(basis).unwrap_or(0.0).max(0.0));
        let border_box = Rect::new(
            margin.left,
            margin.top,
            border.left + padding.left + size.width + padding.right + border.right,
            border.top + padding.top + size.height + padding.bottom + border.bottom,
        );
        let outer_width = (margin.left + border_box.size.width + margin.right).max(0.0);
        let outer_height = (margin.top + border_box.size.height + margin.bottom).max(0.0);

        let mut items = Vec::new();
        if !self.is_measuring() {
            if let Some(paint) = self.box_paint(style) {
                paint.fills(border_box, &mut items);
            }
            if let Some(data) = data {
                let rect = Rect::new(
                    margin.left + border.left + padding.left,
                    margin.top + border.top + padding.top,
                    size.width,
                    size.height,
                );
                items.push(Item::Image(Image { rect, data }));
            }
        }
        let min_width = if shrinks(style) {
            outer_width - size.width
        } else {
            outer_width
        };
        Ok(InlineObject::new(
            outer_width,
            outer_height,
            min_width.max(0.0),
            style.vertical_align,
            items,
        ))
    }

    /// Lays out the image element `id` as a block in a containing block
    /// whose content box starts at `x` and is `width` wide.
    pub(super) fn block_image(&mut self, id: NodeId, style: &ComputedStyle, x: f32, width: f32) {
        let (data, size) = match self.image_content(id, style, width) {
            Content::Image { data, size } => (data, size),
            Content::Alt(alt) => {
                let mut inline = Inline::default();
                inline.push_text(&alt, style);
                return self.paragraph(inline, style, x, width);
            }
        };
        let geometry = BoxGeometry::resolve(style, width, Some(size.width));
        if self.is_measuring() {
            let edges = geometry.horizontal_edges();
            let min = if shrinks(style) { 0.0 } else { size.width };
            self.record_extent(edges + min, edges + size.width);
            return;
        }

        let open = self.open_box(style, &geometry, x, false);
        self.settle_margin();
        let items = data
            .map(|data| {
                let rect = Rect::new(x + geometry.content_x(), 0.0, size.width, size.height);
                Item::Image(Image { rect, data })
            })
            .into_iter()
            .collect();
        self.push_slice(self.y, size.height, items);
        self.y += f64::from(size.height);
        self.close_box(style, &geometry, open);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NATURAL: Size = Size {
        width: 200.0,
        height: 100.0,
    };
    const NO_EDGES: Size = Size {
        width: 0.0,
        height: 0.0,
    };

    fn size(style: &ComputedStyle, basis: Option<f32>) -> (f32, f32) {
        let size = used_size(style, Some(NATURAL), basis, NO_EDGES);
        (size.width, size.height)
    }

    #[test]
    fn natural_size_by_default() {
        assert_eq!(size(&ComputedStyle::initial(), Some(500.0)), (200.0, 100.0));
    }

    #[test]
    fn one_dimension_keeps_the_aspect_ratio() {
        let mut style = ComputedStyle::initial();
        style.width = Dimension::Pt(50.0);
        assert_eq!(size(&style, Some(500.0)), (50.0, 25.0));
        style.width = Dimension::Auto;
        style.height = Dimension::Pt(50.0);
        assert_eq!(size(&style, Some(500.0)), (100.0, 50.0));
        // Both given: the image stretches.
        style.width = Dimension::Pt(10.0);
        assert_eq!(size(&style, Some(500.0)), (10.0, 50.0));
    }

    #[test]
    fn max_width_scales_down_keeping_the_shape() {
        let mut style = ComputedStyle::initial();
        style.max_width = Dimension::Percent(1.0);
        assert_eq!(size(&style, Some(100.0)), (100.0, 50.0));
        // Larger limits change nothing.
        assert_eq!(size(&style, Some(500.0)), (200.0, 100.0));
        style.max_height = Dimension::Pt(20.0);
        assert_eq!(size(&style, Some(100.0)), (40.0, 20.0));
    }

    #[test]
    fn percentages_count_as_auto_while_measuring() {
        let mut style = ComputedStyle::initial();
        style.width = Dimension::Percent(0.5);
        assert_eq!(size(&style, Some(100.0)), (50.0, 25.0));
        assert_eq!(size(&style, None), (200.0, 100.0));
        // Percentage heights always do.
        style.width = Dimension::Auto;
        style.height = Dimension::Percent(0.5);
        assert_eq!(size(&style, Some(100.0)), (200.0, 100.0));
    }

    #[test]
    fn border_box_sizing_takes_off_the_edges() {
        let mut style = ComputedStyle::initial();
        style.width = Dimension::Pt(60.0);
        style.box_sizing = BoxSizing::BorderBox;
        let edges = Size {
            width: 20.0,
            height: 20.0,
        };
        let size = used_size(&style, Some(NATURAL), Some(500.0), edges);
        assert_eq!((size.width, size.height), (40.0, 20.0));
    }

    #[test]
    fn no_natural_size_without_an_image() {
        let mut style = ComputedStyle::initial();
        style.width = Dimension::Pt(30.0);
        let size = used_size(&style, None, Some(500.0), NO_EDGES);
        assert_eq!((size.width, size.height), (30.0, 0.0));
    }
}
