//! Block, inline and table layout on one continuous canvas.
//!
//! Content is laid out top to bottom as if the page were infinitely tall,
//! producing a [`Canvas`]: [`Slice`]s, which are unbreakable horizontal
//! bands (a line of text, a table row), and [`Decoration`]s, the
//! backgrounds and borders of boxes that may span many slices.
//! [`crate::paginate`] then cuts the canvas into pages between slices.
//!
//! # Coordinates
//!
//! The vertical position on the canvas is an `f64`, because a long document
//! is hundreds of thousands of points tall and `f32` would lose sub-point
//! precision. Everything else is `f32` and small: horizontal positions, and
//! item positions relative to the top of their slice.
//!
//! # Modules
//!
//! - this module: the [`Flow`] state, block boxes and margin collapsing
//! - [`boxes`]: resolving the box model
//! - [`inline`]: gathering inline content, shaping and line breaking
//! - [`table`]: table layout
//! - [`replaced`]: sizing and placing images
//! - [`fonts`]: mapping CSS font families to fonts
//! - [`webfonts`]: `@font-face` fonts

mod border;
mod boxes;
mod float;
mod fonts;
pub(crate) mod generated;
mod inline;
mod inline_block;
mod position;
mod replaced;
mod table;
mod webfonts;

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use parley::{FontContext, LayoutContext};
use presswork_dom::resource::{Fetcher, LoadError};
use presswork_dom::{Document, ElementData, NodeId, NodeKind};
use presswork_style::{
    Break, BreakInside, ComputedStyle, Dimension, Display, Float, Overflow, Position, Pseudo,
    Stylist,
};

use crate::fragment::{Clip, ImageData, Item, LinkTarget, Mark, MarkKind};
use crate::geometry::{Point, Rect, Size};
use border::BackgroundImage;
pub(crate) use border::BoxPaint;
use boxes::BoxGeometry;
pub(crate) use fonts::GenericFonts;
#[cfg(test)]
pub(crate) use fonts::SYMBOL_FONTS;
use generated::PseudoBox;
pub(crate) use inline::Ink;
use inline::{ElementKey, Inline};
pub(crate) use webfonts::WebFonts;
#[cfg(test)]
pub(crate) use webfonts::tests as webfont_tests;

/// An unbreakable band of content.
#[derive(Debug)]
pub(crate) struct Slice {
    /// Canvas position of the top of the band.
    pub top: f64,
    pub height: f32,
    /// Positioned relative to the canvas's left edge and the slice's top.
    pub items: Vec<Item>,
    /// The slice must start a new page.
    pub break_before: bool,
    /// The slice should stay on the page of the slice before it.
    pub keep: Keep,
    /// Table header to repeat above this slice when it starts a page.
    pub header: Option<Rc<RepeatedHeader>>,
    /// A positioned box: painted above the content in the flow.
    pub on_top: bool,
}

/// Why a slice should not be separated from the slice before it. A page
/// break goes elsewhere if it can; see [`crate::paginate`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Keep {
    /// A small group: the first or last lines of a paragraph (`orphans`,
    /// `widows`), content after a box with `break-after: avoid` (headings)
    /// or before one with `break-before: avoid`, the first row after a
    /// table's header rows.
    pub tight: bool,
    /// Inside a box with `break-inside: avoid`, which may be large. Given
    /// up first when a group does not fit on a page.
    pub inside: bool,
}

/// Table header rows, repeated at the top of each page a table continues
/// on.
#[derive(Debug)]
pub(crate) struct RepeatedHeader {
    pub height: f32,
    /// Positioned relative to the canvas's left edge and the header's top.
    pub items: Vec<Item>,
}

/// A box's background and borders, spanning a vertical range of the
/// canvas.
#[derive(Debug)]
pub(crate) struct Decoration {
    pub top: f64,
    pub bottom: f64,
    /// Left edge of the border box.
    pub x: f32,
    pub width: f32,
    pub paint: BoxPaint,
}

/// Laid-out content in canvas coordinates.
#[derive(Debug, Default)]
pub(crate) struct Canvas {
    /// In top-to-bottom order.
    pub slices: Vec<Slice>,
    /// In paint order: a box comes before its descendants.
    pub decorations: Vec<Decoration>,
    /// Fixed positioned boxes, relative to the page's content area, drawn
    /// on every page.
    pub fixed: Vec<Item>,
    /// The background of every page's content area (the root's or the
    /// body's, CSS Backgrounds 3 section 2.11).
    pub page_background: Option<BoxPaint>,
}

impl Canvas {
    /// Flattens a small canvas (the contents of a table cell) into items
    /// translated by `(dx, dy)`. Decorations paint first.
    pub fn into_items(self, dx: f32, dy: f32, out: &mut Vec<Item>) {
        for decoration in self.decorations {
            let rect = Rect::new(
                decoration.x + dx,
                decoration.top as f32 + dy,
                decoration.width,
                (decoration.bottom - decoration.top) as f32,
            );
            decoration.paint.fills(rect, out);
        }
        // Positioned boxes last, above the rest.
        let (top, flow): (Vec<Slice>, Vec<Slice>) =
            self.slices.into_iter().partition(|slice| slice.on_top);
        for slice in flow.into_iter().chain(top) {
            let dy = slice.top as f32 + dy;
            out.extend(slice.items.into_iter().map(|item| item.translate(dx, dy)));
        }
    }
}

/// The widest and narrowest a piece of content can be laid out, measured
/// from the left edge of its container.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Extent {
    /// Width with every optional line break taken.
    pub min: f32,
    /// Width with no optional line break taken.
    pub max: f32,
}

/// Everything layout reads from or caches in, shared by nested flows.
pub(crate) struct Context<'a> {
    pub fonts: &'a mut FontContext,
    pub generics: &'a GenericFonts,
    pub texts: &'a mut LayoutContext<Ink>,
    pub doc: &'a Document,
    pub styles: &'a Stylist,
    pub fetcher: &'a Fetcher<'a>,
    pub web_fonts: WebFonts<'a>,
    /// Whether to load images at all (wkhtmltopdf's `--no-images`).
    pub load_images: bool,
    /// Images by URL, `None` for those that failed, so each loads once.
    pub images: HashMap<String, Option<Arc<ImageData>>>,
    /// The `::before` and `::after` boxes of the document.
    pub generated: &'a generated::GeneratedContent,
    /// The page's content area: the containing block of fixed boxes.
    pub page: Size,
    /// Fixed positioned boxes laid out so far, relative to the page's
    /// content area.
    pub fixed: Vec<Item>,
    /// The element whose background covers every page instead of its own
    /// box: the root, or the body if the root has none.
    pub page_background_owner: Option<NodeId>,
}

impl Context<'_> {
    /// The image at `url`, loaded on first use. Failures are recorded by
    /// the fetcher.
    fn image(&mut self, url: &str) -> Option<Arc<ImageData>> {
        let url = url.trim();
        if url.is_empty() || !self.load_images {
            // Browsers do not load an empty `src` either.
            return None;
        }
        if let Some(image) = self.images.get(url) {
            return image.clone();
        }
        let image = self
            .fetcher
            .try_fetch(url, None)
            .and_then(|bytes| ImageData::new(bytes, url).map_err(LoadError::Failed));
        let image = match image {
            Ok(image) => Some(Arc::new(image)),
            Err(error) => {
                self.fetcher.record(url, error);
                None
            }
        };
        self.images.insert(url.to_owned(), image.clone());
        image
    }
}

/// Lays out `text` on its own as a paragraph in `style`, `width` wide, for
/// text that is not part of the document, such as a page header.
pub(crate) fn text_block(
    cx: &mut Context<'_>,
    text: &str,
    style: &ComputedStyle,
    width: f32,
) -> Canvas {
    let mut flow = Flow::new(cx);
    let mut inline = Inline::default();
    inline.push_text(text, style);
    flow.paragraph(inline, style, 0.0, width);
    flow.finish().0
}

/// What a [`Flow`] produces.
enum Mode {
    /// Positioned content.
    Layout,
    /// Only the min- and max-content widths, for table column sizing.
    Measure(Extent),
}

/// Lays out one block formatting context: the document, or a table cell.
pub(crate) struct Flow<'c, 'a> {
    cx: &'c mut Context<'a>,
    mode: Mode,
    canvas: Canvas,
    /// Bottom of the content so far, before any pending margin.
    y: f64,
    /// Collapsed margin waiting to be added before the next content.
    margin: f32,
    /// Decorations whose top edge is not known yet: boxes without top
    /// border or padding start where their first content lands, after
    /// margins collapse through them.
    unplaced: Vec<usize>,
    /// Horizontal margins, borders and padding of the enclosing boxes, for
    /// [`Mode::Measure`].
    edges: f32,
    /// A list marker waiting for the first line of its list item.
    marker: Option<String>,
    /// The next slice must start a new page.
    force_break: bool,
    /// The next slice should stay with the one before it.
    keep_next: bool,
    /// An element whose border box to report, and the decoration that
    /// records it once laid out. See [`Flow::track`].
    tracked: Option<(NodeId, Option<usize>)>,
    /// The baseline of the last line laid out, the baseline of an
    /// inline-block.
    baseline: Option<f64>,
    /// The target of the block-level link being laid out: its lines are
    /// clickable.
    link: Option<LinkTarget>,
    /// Marks for the boxes just opened, with their left edges: they go
    /// at the top of the next slice, the boxes' first content.
    pending_marks: Vec<(f32, MarkKind)>,
    /// The floats placed in this flow, which content flows around.
    floats: Vec<float::FloatBox>,
    /// Slices that start above this are beside a float, and stay with it.
    float_keep_until: f64,
    /// A float was placed below later content: the slices need sorting.
    unsorted: bool,
    /// The positioned boxes open, innermost last.
    positioned: Vec<position::ContainingBlock>,
    /// Absolutely positioned boxes placed against the flow's own area.
    root_absolutes: Vec<position::Absolute>,
    /// The width of the flow's own area.
    root_width: f32,
    /// The height of the flow's own area, if not its content's: the first
    /// page for the document.
    root_height: Option<f32>,
}

/// A box entered with [`Flow::open_box`].
struct OpenBox {
    decoration: Option<usize>,
    /// The index of the box's first slice, if it has any.
    first_slice: usize,
}

impl<'c, 'a> Flow<'c, 'a> {
    pub fn new(cx: &'c mut Context<'a>) -> Self {
        Self::with_mode(cx, Mode::Layout)
    }

    fn measuring(cx: &'c mut Context<'a>) -> Self {
        Self::with_mode(cx, Mode::Measure(Extent::default()))
    }

    fn with_mode(cx: &'c mut Context<'a>, mode: Mode) -> Self {
        Self {
            cx,
            mode,
            canvas: Canvas::default(),
            y: 0.0,
            margin: 0.0,
            unplaced: Vec::new(),
            edges: 0.0,
            marker: None,
            force_break: false,
            keep_next: false,
            tracked: None,
            baseline: None,
            link: None,
            pending_marks: Vec::new(),
            floats: Vec::new(),
            float_keep_until: f64::NEG_INFINITY,
            unsorted: false,
            positioned: Vec::new(),
            root_absolutes: Vec::new(),
            root_width: 0.0,
            root_height: None,
        }
    }

    /// Makes `owner`'s background cover the pages instead of its box.
    pub fn set_page_background_owner(&mut self, owner: NodeId) {
        self.cx.page_background_owner = Some(owner);
    }

    /// Sets the area of the flow itself, which absolutely positioned boxes
    /// without a positioned ancestor are placed in: `width` wide, and as
    /// tall as `height` or else its content.
    pub fn set_root_area(&mut self, width: f32, height: Option<f32>) {
        self.root_width = width;
        self.root_height = height;
    }

    /// Asks for the canvas position of `id`'s border box, which
    /// [`Flow::tracked_box`] gives once it is laid out.
    pub fn track(&mut self, id: NodeId) {
        self.tracked = Some((id, None));
    }

    /// The top and bottom of the tracked element's border box, if it was
    /// laid out as a block.
    pub fn tracked_box(&self) -> Option<(f64, f64)> {
        let index = self.tracked?.1?;
        let decoration = &self.canvas.decorations[index];
        Some((decoration.top, decoration.bottom))
    }

    /// Finishes the flow, returning its content and total height.
    pub fn finish(mut self) -> (Canvas, f64) {
        self.settle_margin();
        // A flow of its own contains its floats (a block formatting
        // context: the root, cells, inline-blocks, floats).
        if let Some(bottom) = self.floats_bottom() {
            self.y = self.y.max(bottom);
        }
        if !self.root_absolutes.is_empty() {
            let bottom = self.root_height.map_or(self.y, f64::from);
            self.place_root_absolutes(0.0, bottom);
        }
        if self.unsorted {
            self.canvas.slices.sort_by(|a, b| a.top.total_cmp(&b.top));
        }
        if !self.pending_marks.is_empty() {
            // Empty boxes at the end still have a place to link to.
            self.push_slice(self.y, 0.0, Vec::new());
        }
        (self.canvas, self.y)
    }

    fn is_measuring(&self) -> bool {
        matches!(self.mode, Mode::Measure(_))
    }

    /// Records content that is `min` to `max` wide inside the current box.
    fn record_extent(&mut self, min: f32, max: f32) {
        let edges = self.edges;
        if let Mode::Measure(extent) = &mut self.mode {
            extent.min = extent.min.max(edges + min);
            extent.max = extent.max.max(edges + max);
        }
    }

    fn extent(&self) -> Extent {
        match self.mode {
            Mode::Measure(extent) => extent,
            Mode::Layout => Extent::default(),
        }
    }

    fn add_margin(&mut self, margin: f32) {
        // Adjacent vertical margins collapse to the largest one.
        self.margin = self.margin.max(margin);
    }

    /// Commits the pending margin: content is about to be placed at `y`.
    fn settle_margin(&mut self) {
        self.y += f64::from(self.margin);
        self.margin = 0.0;
        for index in self.unplaced.drain(..) {
            self.canvas.decorations[index].top = self.y;
        }
    }

    /// Adds a slice `height` tall at `top`, with items relative to it.
    fn push_slice(&mut self, top: f64, height: f32, mut items: Vec<Item>) {
        items.extend(self.pending_marks.drain(..).map(|(x, kind)| mark(x, kind)));
        self.canvas.slices.push(Slice {
            top,
            height,
            items,
            break_before: std::mem::take(&mut self.force_break),
            keep: Keep {
                tight: std::mem::take(&mut self.keep_next),
                // Beside a float: on its page.
                inside: top < self.float_keep_until,
            },
            header: None,
            on_top: false,
        });
    }

    /// Keeps the last slice added with the one before it.
    fn keep_last_slice(&mut self) {
        if let Some(slice) = self.canvas.slices.last_mut() {
            slice.keep.tight = true;
        }
    }

    /// Lays out the block-level element `id` in a containing block whose
    /// content box starts at `x` and is `width` wide.
    pub fn block(&mut self, id: NodeId, style: &ComputedStyle, x: f32, width: f32) {
        if replaced::is_image(self.cx.doc, id) {
            return self.block_image(id, style, x, width);
        }
        if style.display == Display::Table {
            self.clear_floats(style.clear);
            self.mark_element(id, x);
            return self.table(id, style, x, width);
        }
        if self.cx.page_background_owner == Some(id) {
            // Its background is the pages'.
            let mut style = style.clone();
            style.background_color = presswork_style::Color::TRANSPARENT;
            style.background_image = None;
            style.background_gradient = None;
            return self.block_box(Source::Element(id), &style, x, width);
        }
        self.block_box(Source::Element(id), style, x, width);
    }

    /// The background of `style` alone, with its image loaded, if it has
    /// one.
    pub fn background_paint(&mut self, style: &ComputedStyle) -> Option<BoxPaint> {
        let image = (style.background_image.as_deref())
            .and_then(|url| self.cx.image(url))
            .map(|data| BackgroundImage::new(style, data));
        let gradient = style.background_gradient.clone();
        let visible =
            !style.background_color.is_transparent() || image.is_some() || gradient.is_some();
        visible.then_some(BoxPaint {
            background: style.background_color,
            image,
            gradient,
            ..BoxPaint::INVISIBLE
        })
    }

    /// The background and borders of a box in `style`, with its background
    /// image loaded; `None` while measuring, or if nothing shows.
    fn box_paint(&mut self, style: &ComputedStyle) -> Option<BoxPaint> {
        if self.is_measuring() {
            return None;
        }
        let image = (style.background_image.as_deref())
            .and_then(|url| self.cx.image(url))
            .map(|data| BackgroundImage::new(style, data));
        BoxPaint::from_style(style, image)
    }

    /// Records the marks of the element `id`, whose box starts at `x`:
    /// its anchor name, and its heading title.
    fn mark_element(&mut self, id: NodeId, x: f32) {
        if self.is_measuring() {
            return;
        }
        let doc = self.cx.doc;
        let Some(element) = doc.element(id) else {
            return;
        };
        if let Some(name) = anchor_name(element) {
            self.pending_marks.push((x, MarkKind::Anchor(name)));
        }
        if let Some(level) = heading_level(&element.name) {
            let text = doc.text_content(id);
            let title = text.split_whitespace().collect::<Vec<_>>().join(" ");
            if !title.is_empty() {
                let kind = MarkKind::Heading {
                    level,
                    title: title.into(),
                };
                self.pending_marks.push((x, kind));
            }
        }
    }

    /// Where the element `id` links to, if it is a link (`<a href>`).
    /// Fragments of the document's own URL are anchors in it.
    fn link_target(&self, id: NodeId) -> Option<LinkTarget> {
        let element = self.cx.doc.element(id)?;
        if element.name != "a" {
            return None;
        }
        let href = element.attr("href")?.trim();
        if let Some(fragment) = href.strip_prefix('#') {
            return (!fragment.is_empty()).then(|| LinkTarget::Anchor(percent_decode(fragment)));
        }
        let fetcher = self.cx.fetcher;
        let url = fetcher.resolve(href, None).ok()?;
        if url.scheme() == "javascript" {
            return None;
        }
        if let (Some(fragment), Some(base)) = (url.fragment(), fetcher.base()) {
            let (mut url, mut base) = (url.clone(), base.clone());
            url.set_fragment(None);
            base.set_fragment(None);
            if url == base && !fragment.is_empty() {
                return Some(LinkTarget::Anchor(percent_decode(fragment)));
            }
        }
        Some(LinkTarget::Url(url.as_str().into()))
    }

    /// Lays out a block container holding `source`.
    fn block_box(&mut self, source: Source<'_>, style: &ComputedStyle, x: f32, width: f32) {
        self.clear_floats(style.clear);
        let geometry = BoxGeometry::resolve(style, width, None);
        if self.is_measuring()
            && let Dimension::Pt(_) = style.width
        {
            // A fixed-width box is as wide as it says, whatever is inside.
            let outer = geometry.horizontal_edges() + geometry.content_width;
            self.record_extent(outer, outer);
            return;
        }
        let tracked = match source {
            Source::Element(id) => self.tracked.is_some_and(|(tracked, _)| tracked == id),
            Source::Generated(_) => false,
        };
        let link = match source {
            Source::Element(id) => {
                self.mark_element(id, x + geometry.border_box_x());
                self.link_target(id)
            }
            Source::Generated(_) => None,
        };
        let outer_link = match link {
            Some(link) => self.link.replace(link),
            None => self.link.clone(),
        };
        let min_height = geometry.min_content_height(style);
        let sized = min_height > 0.0 && !self.is_measuring();
        let positioned = style.position != Position::Static && !self.is_measuring();
        let clipped = style.overflow == Overflow::Hidden && !self.is_measuring();
        let first_float = self.floats.len();
        let (first_slice, first_decoration) =
            (self.canvas.slices.len(), self.canvas.decorations.len());
        // A box with a height needs a decoration to know where its content
        // starts, and a positioned one where it ends.
        let open = self.open_box(
            style,
            &geometry,
            x,
            tracked || sized || positioned || clipped,
        );
        if tracked && let Some((_, index)) = &mut self.tracked {
            *index = open.decoration;
        }
        if positioned && let Some(decoration) = open.decoration {
            let padding_x = x + geometry.border_box_x() + geometry.border.left;
            let padding_width =
                geometry.border_box_width() - geometry.border.left - geometry.border.right;
            let borders = (geometry.border.top, geometry.border.bottom);
            self.open_containing_block(padding_x, padding_width, decoration, borders);
        }
        self.contents(
            source,
            style,
            x + geometry.content_x(),
            geometry.content_width,
        );
        self.link = outer_link;
        if sized && let Some(index) = open.decoration {
            self.extend_to(index, geometry.top_edge(), min_height);
        }
        if style.overflow == Overflow::Hidden
            && let Some(bottom) = self.floats[first_float..]
                .iter()
                .map(|f| f.bottom())
                .reduce(f64::max)
        {
            // A new block formatting context contains its floats.
            self.settle_margin();
            self.y = self.y.max(bottom);
        }
        let decoration = open.decoration;
        self.close_box(style, &geometry, open);
        if clipped && let Some(index) = decoration {
            let padding = Rect::new(
                x + geometry.border_box_x() + geometry.border.left,
                0.0,
                geometry.border_box_width() - geometry.border.left - geometry.border.right,
                0.0,
            );
            self.clip_slices(index, padding, &geometry, first_slice);
        }
        if positioned && decoration.is_some() {
            self.close_containing_block();
            if style.position == Position::Relative {
                self.shift_relative(style, width, first_slice, first_decoration);
            }
        }
    }

    /// Clips the content laid out in slices from `first_slice` to the
    /// padding box of the box whose decoration is `decoration`, from
    /// `padding.origin.x`, `padding.size.width` wide. Links and marks stay
    /// outside the clip, where they are found.
    fn clip_slices(
        &mut self,
        decoration: usize,
        padding: Rect,
        geometry: &BoxGeometry,
        first_slice: usize,
    ) {
        let box_ = &self.canvas.decorations[decoration];
        let top = box_.top + f64::from(geometry.border.top);
        let bottom = (box_.bottom - f64::from(geometry.border.bottom)).max(top);
        for slice in &mut self.canvas.slices[first_slice..] {
            let (kept, clipped): (Vec<Item>, Vec<Item>) = std::mem::take(&mut slice.items)
                .into_iter()
                .partition(|item| matches!(item, Item::Link(_) | Item::Mark(_)));
            slice.items = kept;
            if !clipped.is_empty() {
                let rect = Rect::new(
                    padding.origin.x,
                    (top - slice.top) as f32,
                    padding.size.width,
                    (bottom - top) as f32,
                );
                slice.items.push(Item::Clip(Clip {
                    rect,
                    items: clipped,
                }));
            }
        }
    }

    /// Starts a box: top margin, border and padding, and its decoration.
    /// A `tracked` box gets a decoration even if it paints nothing, so
    /// that its position is worked out like any other's.
    fn open_box(
        &mut self,
        style: &ComputedStyle,
        geometry: &BoxGeometry,
        x: f32,
        tracked: bool,
    ) -> OpenBox {
        match style.break_before {
            Break::Page => self.force_break = true,
            Break::Avoid => self.keep_next = true,
            Break::Auto => {}
        }
        self.add_margin(geometry.margin.top);
        self.edges += geometry.horizontal_edges();

        let paint = if self.is_measuring() {
            None
        } else {
            self.box_paint(style)
                .or(tracked.then_some(BoxPaint::INVISIBLE))
        };
        let decoration = paint.map(|paint| {
            self.canvas.decorations.push(Decoration {
                top: self.y,
                bottom: self.y,
                x: x + geometry.border_box_x(),
                width: geometry.border_box_width(),
                paint,
            });
            self.canvas.decorations.len() - 1
        });

        if geometry.top_edge() > 0.0 {
            // A border or padding keeps the box's margin from collapsing
            // with its children's.
            self.settle_margin();
            if let Some(index) = decoration {
                self.canvas.decorations[index].top = self.y;
            }
            self.y += f64::from(geometry.top_edge());
        } else if let Some(index) = decoration {
            self.unplaced.push(index);
        }
        OpenBox {
            decoration,
            first_slice: self.canvas.slices.len(),
        }
    }

    /// Makes the content of the box whose decoration is `index` at least
    /// `height` tall, with an empty slice below the content if it falls
    /// short. The slice keeps the space together on one page, and moves an
    /// empty box (a signature field) to the next page whole if it does
    /// not fit.
    fn extend_to(&mut self, index: usize, top_edge: f32, height: f32) {
        // The last child's bottom margin stays inside a box with a height.
        // Settling also places a box that is still empty.
        self.settle_margin();
        let content_top = self.canvas.decorations[index].top + f64::from(top_edge);
        let missing = content_top + f64::from(height) - self.y;
        if missing > 0.0 {
            self.push_slice(self.y, missing as f32, Vec::new());
            self.y += missing;
        }
    }

    /// Ends a box: bottom padding, border and margin.
    fn close_box(&mut self, style: &ComputedStyle, geometry: &BoxGeometry, open: OpenBox) {
        if geometry.bottom_edge() > 0.0 {
            self.settle_margin();
            self.y += f64::from(geometry.bottom_edge());
        }
        if let Some(index) = open.decoration {
            // Still unplaced: the box is empty and has no height.
            if let Some(position) = self.unplaced.iter().position(|&i| i == index) {
                self.unplaced.remove(position);
                self.canvas.decorations[index].top = self.y;
            }
            self.canvas.decorations[index].bottom = self.y;
        }
        if style.break_inside == BreakInside::Avoid {
            let inner = (open.first_slice + 1).min(self.canvas.slices.len());
            for slice in &mut self.canvas.slices[inner..] {
                slice.keep.inside = true;
            }
        }
        self.edges -= geometry.horizontal_edges();
        self.add_margin(geometry.margin.bottom);
        match style.break_after {
            Break::Page => self.force_break = true,
            Break::Avoid => self.keep_next = true,
            Break::Auto => {}
        }
    }

    /// Lays out what a block container holds; its content box starts at
    /// `x` and is `width` wide.
    fn contents(&mut self, source: Source<'_>, style: &ComputedStyle, x: f32, width: f32) {
        match source {
            Source::Element(id) => self.block_contents(id, style, x, width),
            Source::Generated(text) => {
                let mut inline = Inline::first();
                inline.push_text(text, style);
                self.paragraph(inline, style, x, width);
            }
        }
    }

    /// Lays out the children of the block container `id`, whose content
    /// box starts at `x` and is `width` wide.
    fn block_contents(&mut self, id: NodeId, style: &ComputedStyle, x: f32, width: f32) {
        if style.display == Display::ListItem {
            self.marker = inline::list_marker(self.cx.doc, id, style);
        }
        let mut inline = Inline::first();
        self.children(id, style, x, width, &mut inline);
        self.paragraph(inline, style, x, width);
        // An item without text has nowhere to put its marker.
        self.marker = None;
    }

    /// Walks the children of `id`, gathering inline content into `inline`
    /// and laying out block-level children as they come.
    fn children(
        &mut self,
        id: NodeId,
        style: &ComputedStyle,
        x: f32,
        width: f32,
        inline: &mut Inline,
    ) {
        let doc = self.cx.doc;
        let generated = self.cx.generated.get(&id);
        if let Some(before) = generated.and_then(|g| g.get(Pseudo::Before)) {
            self.pseudo_box((id, Some(Pseudo::Before)), before, style, x, width, inline);
        }
        for child in doc.children(id) {
            match &doc.node(child).kind {
                NodeKind::Text(text) => inline.push_text(text, style),
                NodeKind::Element(element) => {
                    let child_style = self.cx.styles.style(doc, child, Some(style));
                    let display = child_style.display;
                    if matches!(child_style.position, Position::Absolute | Position::Fixed)
                        && display != Display::None
                    {
                        self.positioned_element(child, &child_style, x);
                        continue;
                    }
                    if child_style.float != Float::None && display != Display::None {
                        self.float_element(child, &child_style, x, width);
                        continue;
                    }
                    if matches!(display, Display::Inline | Display::InlineBlock)
                        && let Some(name) = anchor_name(element)
                    {
                        inline.anchor(name);
                    }
                    match child_style.display {
                        Display::None => {}
                        Display::Inline if element.name == "br" => inline.push_break(&child_style),
                        Display::Inline | Display::InlineBlock
                            if replaced::is_image(doc, child) =>
                        {
                            self.inline_image(child, &child_style, width, inline);
                        }
                        Display::Inline => {
                            let link = self.link_target(child);
                            let paint = self.box_paint(&child_style);
                            inline.enter((child, None), &child_style, width, link, paint);
                            self.children(child, &child_style, x, width, inline);
                            inline.leave((child, None));
                        }
                        Display::InlineBlock => {
                            let source = Source::Element(child);
                            self.inline_block(source, &child_style, width, inline);
                        }
                        _ => {
                            let rest = inline.continuation();
                            self.paragraph(std::mem::replace(inline, rest), style, x, width);
                            self.block(child, &child_style, x, width);
                        }
                    }
                }
                _ => {}
            }
        }
        if let Some(after) = generated.and_then(|g| g.get(Pseudo::After)) {
            self.pseudo_box((id, Some(Pseudo::After)), after, style, x, width, inline);
        }
    }

    /// Lays out a `::before` or `::after` box among the children of the
    /// element whose style is `parent_style`.
    fn pseudo_box(
        &mut self,
        key: ElementKey,
        pseudo: &PseudoBox,
        parent_style: &ComputedStyle,
        x: f32,
        width: f32,
        inline: &mut Inline,
    ) {
        let style = &pseudo.style;
        match style.display {
            Display::None => {}
            Display::Inline => {
                let paint = self.box_paint(style);
                inline.enter(key, style, width, None, paint);
                inline.push_text(&pseudo.text, style);
                inline.leave(key);
            }
            Display::InlineBlock => {
                self.inline_block(Source::Generated(&pseudo.text), style, width, inline);
            }
            _ => {
                let rest = inline.continuation();
                self.paragraph(std::mem::replace(inline, rest), parent_style, x, width);
                self.block_box(Source::Generated(&pseudo.text), style, x, width);
            }
        }
    }
}

/// An item marking a place at the top of a slice, `x` from the canvas's
/// left edge.
pub(super) fn mark(x: f32, kind: MarkKind) -> Item {
    Item::Mark(Mark {
        origin: Point { x, y: 0.0 },
        kind,
    })
}

/// The name links reach `element` by: its `id`, or the `name` of an
/// `<a>`.
fn anchor_name(element: &ElementData) -> Option<Arc<str>> {
    let name = element.attr("id").or_else(|| {
        (element.name == "a")
            .then(|| element.attr("name"))
            .flatten()
    })?;
    (!name.is_empty()).then(|| name.into())
}

/// The outline level of a heading element (`h1` is 1).
fn heading_level(name: &str) -> Option<u8> {
    match name.as_bytes() {
        [b'h', level @ b'1'..=b'6'] => Some(level - b'0'),
        _ => None,
    }
}

/// Decodes `%XX` escapes in a URL fragment (an `id` in Devanagari is
/// percent-encoded in a resolved URL).
fn percent_decode(text: &str) -> Arc<str> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(high), Some(low)) = (
                bytes.get(i + 1).copied().and_then(hex),
                bytes.get(i + 2).copied().and_then(hex),
            )
        {
            out.push((high * 16 + low) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into()
}

/// What a block container holds: an element's children, or the text of
/// a generated box.
#[derive(Debug, Clone, Copy)]
pub(super) enum Source<'g> {
    Element(NodeId),
    Generated(&'g str),
}
