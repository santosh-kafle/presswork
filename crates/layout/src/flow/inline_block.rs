//! `display: inline-block`: a block container laid out on its own, then
//! placed in a line like an image.

use presswork_style::{ComputedStyle, Dimension};

use super::boxes::BoxGeometry;
use super::inline::{Inline, InlineObject};
use super::{Flow, Source};
use crate::geometry::Rect;

impl Flow<'_, '_> {
    /// Lays out an inline-block holding `source` in a containing block `width` wide
    /// and adds it to `inline`.
    ///
    /// Without a `width` it shrinks to fit its content (CSS 2.1 section
    /// 10.3.9): as wide as its content wants, but no wider than the line
    /// and no narrower than its content can go. It sits on the line by its
    /// last line's baseline, or by its bottom margin edge if it has no
    /// lines (section 10.8.1), and is never split across pages.
    pub(super) fn inline_block(
        &mut self,
        source: Source<'_>,
        style: &ComputedStyle,
        width: f32,
        inline: &mut Inline,
    ) {
        let measuring = self.is_measuring();
        // Percentages have no basis while measuring.
        let basis = if measuring { 0.0 } else { width };
        let percent = |value: Dimension| measuring && matches!(value, Dimension::Percent(_));
        // Auto margins are zero on inline boxes.
        let margin = style.margin.map(|m| m.resolve(basis).unwrap_or(0.0));
        let border = style.border_widths();
        let padding = style
            .padding
            .map(|p| p.resolve(basis).unwrap_or(0.0).max(0.0));
        let edges = border.left + padding.left + padding.right + border.right;
        let outside = margin.left + margin.right + edges;

        let specified = if percent(style.width) {
            None
        } else {
            BoxGeometry::specified_content_width(style, basis, edges)
        };
        let max_width = if percent(style.max_width) {
            None
        } else {
            BoxGeometry::content_size(style.max_width, style, basis, edges)
        };
        let cap = |width: f32| max_width.map_or(width, |max| width.min(max)).max(0.0);
        let (min, preferred) = match specified {
            Some(width) => (width, width),
            None => self.measure_contents(source, style),
        };

        if measuring {
            let object = InlineObject::new(
                (cap(preferred) + outside).max(0.0),
                0.0,
                (cap(min) + outside).max(0.0),
                style.vertical_align,
                Vec::new(),
            );
            return inline.push_box(object, style);
        }

        let content_width = match specified {
            Some(width) => cap(width),
            None => cap(preferred.min((width - outside).max(min))),
        };
        let geometry = BoxGeometry {
            margin,
            border,
            padding,
            content_width,
        };
        let link = match source {
            Source::Element(id) => self.link_target(id),
            Source::Generated(_) => None,
        };
        let mut flow = Flow::new(self.cx);
        flow.link = link;
        flow.contents(source, style, 0.0, content_width);
        let baseline = flow.baseline;
        let (canvas, height) = flow.finish();
        let content_height = (height as f32).max(geometry.min_content_height(style));

        let content_top = margin.top + border.top + padding.top;
        let border_box = Rect::new(
            margin.left,
            margin.top,
            geometry.border_box_width(),
            border.top + padding.top + content_height + padding.bottom + border.bottom,
        );
        let mut items = Vec::new();
        if let Some(paint) = self.box_paint(style) {
            paint.fills(border_box, &mut items);
        }
        canvas.into_items(geometry.content_x(), content_top, &mut items);

        // Summed as when measuring, so that a line sized to the measured
        // width still fits the box.
        let outer_width = (content_width + outside).max(0.0);
        let outer_height = (margin.top + border_box.size.height + margin.bottom).max(0.0);
        let mut object = InlineObject::new(
            outer_width,
            outer_height,
            outer_width,
            style.vertical_align,
            items,
        );
        object.baseline = baseline.map(|baseline| content_top + baseline as f32);
        inline.push_box(object, style);
    }

    /// The min- and max-content widths of what a block container holds.
    fn measure_contents(&mut self, source: Source<'_>, style: &ComputedStyle) -> (f32, f32) {
        let mut flow = Flow::measuring(self.cx);
        flow.contents(source, style, 0.0, 0.0);
        let extent = flow.extent();
        (extent.min, extent.max)
    }
}
