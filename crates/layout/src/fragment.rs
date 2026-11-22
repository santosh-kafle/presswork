//! The output of layout: positioned content, one list per page.

use std::fmt;
use std::ops::Range;
use std::sync::Arc;

pub use parley::FontData;
pub use presswork_style::Color;

use crate::geometry::{Point, Rect, Size};

/// One laid-out page. Coordinates are in points from the page's top-left.
/// Items are in paint order: later items draw over earlier ones.
#[derive(Debug, Clone)]
pub struct Page {
    pub size: Size,
    pub items: Vec<Item>,
}

/// A piece of positioned content.
#[derive(Debug, Clone)]
pub enum Item {
    Text(TextRun),
    Fill(Fill),
    Image(Image),
    /// A clickable area; draws nothing.
    Link(Link),
    /// A place in the document; draws nothing.
    Mark(Mark),
    /// A rounded box, or a border that is not solid.
    Shape(Shape),
}

impl Item {
    pub(crate) fn translate(mut self, dx: f32, dy: f32) -> Self {
        if let Item::Shape(shape) = &mut self {
            shape.translate(dx, dy);
            return self;
        }
        let origin = match &mut self {
            Item::Text(run) => &mut run.origin,
            Item::Fill(fill) => &mut fill.rect.origin,
            Item::Image(image) => &mut image.rect.origin,
            Item::Link(link) => &mut link.rect.origin,
            Item::Mark(mark) => &mut mark.origin,
            Item::Shape(_) => unreachable!("translated above"),
        };
        origin.x += dx;
        origin.y += dy;
        self
    }
}

/// The area of a hyperlink (`<a href>`) on a page.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub rect: Rect,
    pub target: LinkTarget,
}

/// Where a link goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// An absolute URL outside the document.
    Url(Arc<str>),
    /// An element of the document, by `id` (or `<a name>`); see
    /// [`MarkKind::Anchor`].
    Anchor(Arc<str>),
}

/// A place in the document that links and the outline point to.
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    /// The top-left of the element's first line or box.
    pub origin: Point,
    pub kind: MarkKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarkKind {
    /// An element with an `id`, or an `<a name>`.
    Anchor(Arc<str>),
    /// A heading (`<h1>` to `<h6>`), for the PDF outline and the
    /// `[section]` header variables.
    Heading { level: u8, title: Arc<str> },
}

/// A filled or stroked outline.
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    pub path: Vec<PathOp>,
    pub paint: ShapePaint,
    /// If set, only the part inside this polygon is painted.
    pub clip: Option<Vec<Point>>,
}

impl Shape {
    fn translate(&mut self, dx: f32, dy: f32) {
        let shift = |point: &mut Point| {
            point.x += dx;
            point.y += dy;
        };
        for op in &mut self.path {
            match op {
                PathOp::Move(p) | PathOp::Line(p) => shift(p),
                PathOp::Cubic(a, b, c) => {
                    shift(a);
                    shift(b);
                    shift(c);
                }
                PathOp::Close => {}
            }
        }
        for point in self.clip.iter_mut().flatten() {
            shift(point);
        }
    }
}

/// One step of a [`Shape`]'s path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathOp {
    Move(Point),
    Line(Point),
    /// A cubic Bézier curve: two control points, then the end point.
    Cubic(Point, Point, Point),
    Close,
}

/// How a [`Shape`] is painted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShapePaint {
    /// Filled. With `even_odd`, a path inside another cuts a hole (a
    /// ring).
    Fill { color: Color, even_odd: bool },
    /// Stroked `width` wide, along the path's centre. With `dash`, in
    /// dashes and gaps of those lengths; with `round`, dashes get round
    /// caps, and zero-length ones are dots.
    Stroke {
        color: Color,
        width: f32,
        dash: Option<(f32, f32)>,
        round: bool,
    },
}

/// A filled rectangle: a background or one edge of a border.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fill {
    pub rect: Rect,
    pub color: Color,
}

/// An image scaled to fill `rect`.
#[derive(Debug, Clone)]
pub struct Image {
    pub rect: Rect,
    pub data: Arc<ImageData>,
}

/// A raster format that can be embedded in a PDF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Gif,
    Webp,
}

/// An encoded image file, with the format and pixel size read from its
/// header. The pixels themselves are only decoded when the PDF is written.
#[derive(Clone)]
pub struct ImageData {
    pub bytes: Arc<Vec<u8>>,
    pub format: ImageFormat,
    /// Size in pixels.
    pub width: u32,
    pub height: u32,
    /// Where the image came from, shortened for messages.
    pub source: String,
}

impl ImageData {
    /// Reads the format and size of an image file. `source` is its URL.
    pub fn new(bytes: Vec<u8>, source: &str) -> Result<Self, String> {
        let format = match imagesize::image_type(&bytes) {
            Ok(imagesize::ImageType::Png) => ImageFormat::Png,
            Ok(imagesize::ImageType::Jpeg) => ImageFormat::Jpeg,
            Ok(imagesize::ImageType::Gif) => ImageFormat::Gif,
            Ok(imagesize::ImageType::Webp) => ImageFormat::Webp,
            _ if looks_like_svg(&bytes) => return Err("SVG images are not supported yet".into()),
            _ => {
                return Err("not an image format presswork supports (PNG, JPEG, GIF, WebP)".into());
            }
        };
        let size = imagesize::blob_size(&bytes).map_err(|e| format!("unreadable image: {e}"))?;
        let (Ok(width), Ok(height)) = (u32::try_from(size.width), u32::try_from(size.height))
        else {
            return Err("image is too large".into());
        };
        if width == 0 || height == 0 {
            return Err("image is empty".into());
        }
        Ok(Self {
            bytes: Arc::new(bytes),
            format,
            width,
            height,
            source: presswork_dom::resource::display_url(source).into_owned(),
        })
    }

    /// The size in points at which the image displays by default: one
    /// pixel per CSS px, as browsers do.
    pub fn natural_size(&self) -> Size {
        const PT_PER_PX: f32 = 0.75;
        Size {
            width: self.width as f32 * PT_PER_PX,
            height: self.height as f32 * PT_PER_PX,
        }
    }
}

/// Whether `bytes` start like an SVG document.
fn looks_like_svg(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]);
    head.contains("<svg")
}

impl fmt::Debug for ImageData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageData")
            .field("format", &self.format)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bytes", &self.bytes.len())
            .field("source", &self.source)
            .finish()
    }
}

/// Shaped glyphs in a single font, size and color, drawn left to right
/// from `origin`, which is on the baseline.
#[derive(Clone)]
pub struct TextRun {
    pub font: FontData,
    /// Font size in points.
    pub font_size: f32,
    pub color: Color,
    pub origin: Point,
    pub glyphs: Vec<Glyph>,
    /// The text the glyphs' `text_range`s index into, so the PDF can map
    /// glyphs back to characters for copying and search.
    pub text: Arc<str>,
}

impl TextRun {
    /// The source text this run covers.
    pub fn covered_text(&self) -> &str {
        let start = self.glyphs.iter().map(|g| g.text_range.start).min();
        let end = self.glyphs.iter().map(|g| g.text_range.end).max();
        match (start, end) {
            (Some(start), Some(end)) => &self.text[start..end],
            _ => "",
        }
    }
}

impl fmt::Debug for TextRun {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text: String = self.covered_text().chars().take(40).collect();
        f.debug_struct("TextRun")
            .field("font_size", &self.font_size)
            .field("color", &self.color)
            .field("origin", &self.origin)
            .field("glyphs", &self.glyphs.len())
            .field("text", &text)
            .finish()
    }
}

/// One glyph. Lengths are in points; `y_offset` points down.
#[derive(Debug, Clone, PartialEq)]
pub struct Glyph {
    pub id: u32,
    pub advance: f32,
    pub x_offset: f32,
    pub y_offset: f32,
    /// Byte range in [`TextRun::text`] of the cluster this glyph belongs to.
    pub text_range: Range<usize>,
}
