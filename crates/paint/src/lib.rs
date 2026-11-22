//! PDF output.
//!
//! Draws the pages produced by `presswork-layout` with `krilla`, which embeds
//! subsetted fonts and maps glyphs back to text so the PDF can be searched
//! and copied from, and embeds images.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use krilla::action::{Action, LinkAction};
use krilla::annotation::{Annotation, LinkAnnotation, Target};
use krilla::color::rgb;
use krilla::destination::{Destination, XyzDestination};
use krilla::error::KrillaError;
use krilla::geom::{PathBuilder, Point, Rect, Size, Transform};
use krilla::image::Image as PdfImage;
use krilla::metadata::Metadata;
use krilla::num::NormalizedF32;
use krilla::outline::{Outline, OutlineNode};
use krilla::page::PageSettings;
use krilla::paint::{Fill, FillRule, LineCap, LineJoin, Stroke, StrokeDash};
use krilla::surface::Surface;
use krilla::text::{Font, GlyphId, KrillaGlyph};
use krilla::{Data, Document};
use presswork_layout::{
    Color, FontData, Image, ImageData, ImageFormat, Item, Link, LinkTarget, MarkKind, Page, PathOp,
    Shape, ShapePaint, TextRun,
};

/// Document-level options for the generated PDF.
#[derive(Debug, Clone)]
pub struct PdfOptions {
    /// `Title` entry in the PDF metadata.
    pub title: Option<String>,
    /// `Author` entry in the PDF metadata.
    pub author: Option<String>,
    /// How many heading levels the outline (bookmarks) shows; zero for
    /// none. wkhtmltopdf's default is 4.
    pub outline_depth: usize,
    /// Whether links to other documents are clickable.
    pub external_links: bool,
    /// Whether links within the document are clickable.
    pub internal_links: bool,
}

impl Default for PdfOptions {
    fn default() -> Self {
        Self {
            title: None,
            author: None,
            outline_depth: 4,
            external_links: true,
            internal_links: true,
        }
    }
}

/// Writing the PDF failed.
#[derive(Debug)]
pub struct Error(String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "failed to write PDF: {}", self.0)
    }
}

impl std::error::Error for Error {}

/// A written PDF.
#[derive(Debug)]
pub struct Pdf {
    pub bytes: Vec<u8>,
    /// Images left out because they could not be decoded, by source.
    pub broken_images: Vec<String>,
}

/// Where each anchor is: page index and position. The first element with
/// a name wins, as in browsers.
fn anchors(pages: &[Page]) -> HashMap<&str, (usize, Point)> {
    let mut anchors = HashMap::new();
    for (index, page) in pages.iter().enumerate() {
        for item in &page.items {
            if let Item::Mark(mark) = item
                && let MarkKind::Anchor(name) = &mark.kind
            {
                anchors
                    .entry(name.as_ref())
                    .or_insert((index, Point::from_xy(mark.origin.x, mark.origin.y)));
            }
        }
    }
    anchors
}

/// The annotation that makes `link` clickable, if its target exists and
/// links of its kind are enabled.
fn link_annotation(
    link: &Link,
    anchors: &HashMap<&str, (usize, Point)>,
    options: &PdfOptions,
) -> Option<Annotation> {
    let r = link.rect;
    let rect = Rect::from_xywh(r.origin.x, r.origin.y, r.size.width, r.size.height)?;
    let target = match &link.target {
        LinkTarget::Url(url) if options.external_links => {
            Target::Action(Action::Link(LinkAction::new(url.to_string())))
        }
        LinkTarget::Anchor(name) if options.internal_links => {
            let &(page, point) = anchors.get(name.as_ref())?;
            Target::Destination(Destination::Xyz(XyzDestination::new(page, point)))
        }
        _ => return None,
    };
    Some(Annotation::new_link(
        LinkAnnotation::new(rect, target),
        None,
    ))
}

/// The outline (bookmarks) from the headings of `pages`, nested by level
/// and `depth` deep at most; `None` if there are no headings.
fn outline(pages: &[Page], depth: usize) -> Option<Outline> {
    let mut outline = Outline::new();
    let mut any = false;
    // The open headings, outermost first, with their levels.
    let mut open: Vec<(u8, OutlineNode)> = Vec::new();
    let close = |open: &mut Vec<(u8, OutlineNode)>, outline: &mut Outline| {
        let (_, node) = open.pop().expect("an open heading");
        match open.last_mut() {
            Some((_, parent)) => parent.push_child(node),
            None => outline.push_child(node),
        }
    };
    for (index, page) in pages.iter().enumerate() {
        for item in &page.items {
            let Item::Mark(mark) = item else { continue };
            let MarkKind::Heading { level, title } = &mark.kind else {
                continue;
            };
            while open.last().is_some_and(|(open, _)| open >= level) {
                close(&mut open, &mut outline);
            }
            if open.len() >= depth {
                continue;
            }
            let point = Point::from_xy(mark.origin.x, mark.origin.y);
            let node = OutlineNode::new(title.to_string(), XyzDestination::new(index, point));
            open.push((*level, node));
            any = true;
        }
    }
    while !open.is_empty() {
        close(&mut open, &mut outline);
    }
    any.then_some(outline)
}

/// Renders laid-out pages to a PDF.
pub fn render_pdf(pages: &[Page], options: &PdfOptions) -> Result<Pdf, Error> {
    if let Some(page) = pages
        .iter()
        .find(|page| PageSettings::from_wh(page.size.width, page.size.height).is_none())
    {
        return Err(Error(format!("invalid page size {:?}", page.size)));
    }
    // krilla decodes images only when the document is finished, and one
    // that fails to decode fails the whole document. Leave such images out
    // and write the document again.
    let mut skip = HashSet::new();
    loop {
        let mut images = ImageCache::new(&skip);
        match write(pages, options, &mut images) {
            Ok(bytes) => {
                return Ok(Pdf {
                    bytes,
                    broken_images: images.broken,
                });
            }
            Err(KrillaError::Image(image, _, message)) => match images.key_of(&image) {
                Some(key) if skip.insert(key) => {}
                _ => return Err(Error(format!("image could not be decoded: {message}"))),
            },
            Err(error) => return Err(Error(error.to_string())),
        }
    }
}

fn write(
    pages: &[Page],
    options: &PdfOptions,
    images: &mut ImageCache<'_>,
) -> Result<Vec<u8>, KrillaError> {
    let mut document = Document::new();
    let mut fonts = FontCache::default();
    let anchors = anchors(pages);

    for page in pages {
        let Some(settings) = PageSettings::from_wh(page.size.width, page.size.height) else {
            unreachable!("page sizes are checked before writing");
        };
        let mut pdf_page = document.start_page_with(settings);
        let mut surface = pdf_page.surface();
        let mut links = Vec::new();
        for item in &page.items {
            match item {
                Item::Text(run) => {
                    if let Some(font) = fonts.get(&run.font) {
                        draw_text(&mut surface, run, font);
                    }
                }
                Item::Image(image) => {
                    if let Some(pdf_image) = images.get(&image.data) {
                        draw_image(&mut surface, image, pdf_image);
                    }
                }
                Item::Fill(fill) => {
                    let rect = fill.rect;
                    let Some(rect) = Rect::from_xywh(
                        rect.origin.x,
                        rect.origin.y,
                        rect.size.width,
                        rect.size.height,
                    ) else {
                        continue;
                    };
                    let mut path = PathBuilder::new();
                    path.push_rect(rect);
                    if let Some(path) = path.finish() {
                        set_color(&mut surface, fill.color);
                        surface.draw_path(&path);
                    }
                }
                Item::Shape(shape) => draw_shape(&mut surface, shape),
                Item::Link(link) => links.push(link),
                Item::Mark(_) => {}
            }
        }
        surface.finish();
        for link in links {
            if let Some(annotation) = link_annotation(link, &anchors, options) {
                pdf_page.add_annotation(annotation);
            }
        }
        pdf_page.finish();
    }
    if options.outline_depth > 0
        && let Some(outline) = outline(pages, options.outline_depth)
    {
        document.set_outline(outline);
    }

    let mut metadata = Metadata::new().creator(format!("presswork {}", env!("CARGO_PKG_VERSION")));
    if let Some(title) = &options.title {
        metadata = metadata.title(title.clone());
    }
    if let Some(author) = &options.author {
        metadata = metadata.authors(vec![author.clone()]);
    }
    document.set_metadata(metadata);
    document.finish()
}

/// Draws `image` scaled into its rectangle.
fn draw_image(surface: &mut Surface<'_>, image: &Image, pdf_image: PdfImage) {
    let rect = image.rect;
    let Some(size) = Size::from_wh(rect.size.width, rect.size.height) else {
        return;
    };
    surface.push_transform(&Transform::from_translate(rect.origin.x, rect.origin.y));
    surface.draw_image(pdf_image, size);
    surface.pop();
}

/// Images prepared for krilla, keyed by the identity of their data.
struct ImageCache<'a> {
    images: HashMap<usize, Option<PdfImage>>,
    /// Images to leave out: they failed to decode on an earlier attempt.
    skip: &'a HashSet<usize>,
    broken: Vec<String>,
}

impl<'a> ImageCache<'a> {
    fn new(skip: &'a HashSet<usize>) -> Self {
        Self {
            images: HashMap::new(),
            skip,
            broken: Vec::new(),
        }
    }

    fn key(data: &Arc<ImageData>) -> usize {
        Arc::as_ptr(data) as usize
    }

    fn get(&mut self, data: &Arc<ImageData>) -> Option<PdfImage> {
        let key = Self::key(data);
        if let Some(image) = self.images.get(&key) {
            return image.clone();
        }
        let image = if self.skip.contains(&key) {
            None
        } else {
            let bytes: Arc<dyn AsRef<[u8]> + Send + Sync> = data.bytes.clone();
            let bytes = Data::from(bytes);
            // Smooth scaling, as browsers do.
            let image = match data.format {
                ImageFormat::Png => PdfImage::from_png(bytes, true),
                ImageFormat::Jpeg => PdfImage::from_jpeg(bytes, true),
                ImageFormat::Gif => PdfImage::from_gif(bytes, true),
                ImageFormat::Webp => PdfImage::from_webp(bytes, true),
            };
            image.ok()
        };
        if image.is_none() {
            self.broken.push(data.source.clone());
        }
        self.images.insert(key, image.clone());
        image
    }

    /// The key of a krilla image made by this cache.
    fn key_of(&self, image: &PdfImage) -> Option<usize> {
        self.images
            .iter()
            .find(|(_, cached)| cached.as_ref() == Some(image))
            .map(|(key, _)| *key)
    }
}

/// Sets the fill color for the drawing operations that follow.
/// A krilla path from layout's path steps.
fn build_path(ops: &[PathOp]) -> Option<krilla::geom::Path> {
    let mut path = PathBuilder::new();
    for op in ops {
        match *op {
            PathOp::Move(p) => path.move_to(p.x, p.y),
            PathOp::Line(p) => path.line_to(p.x, p.y),
            PathOp::Cubic(a, b, c) => path.cubic_to(a.x, a.y, b.x, b.y, c.x, c.y),
            PathOp::Close => path.close(),
        }
    }
    path.finish()
}

fn draw_shape(surface: &mut Surface<'_>, shape: &Shape) {
    let Some(path) = build_path(&shape.path) else {
        return;
    };
    let clip = shape.clip.as_ref().and_then(|polygon| {
        let mut ops = Vec::with_capacity(polygon.len() + 1);
        for (index, &point) in polygon.iter().enumerate() {
            ops.push(if index == 0 {
                PathOp::Move(point)
            } else {
                PathOp::Line(point)
            });
        }
        ops.push(PathOp::Close);
        build_path(&ops)
    });
    if let Some(clip) = &clip {
        surface.push_clip_path(clip, &FillRule::NonZero);
    }
    match shape.paint {
        ShapePaint::Fill { color, even_odd } => {
            set_color(surface, color);
            if even_odd && let Some(fill) = surface.get_fill() {
                let mut fill = fill.clone();
                fill.rule = FillRule::EvenOdd;
                surface.set_fill(Some(fill));
            }
            surface.draw_path(&path);
        }
        ShapePaint::Stroke {
            color,
            width,
            dash,
            round,
        } => {
            let opacity =
                NormalizedF32::new(f32::from(color.a) / 255.0).unwrap_or(NormalizedF32::ONE);
            surface.set_fill(None);
            surface.set_stroke(Some(Stroke {
                paint: rgb::Color::new(color.r, color.g, color.b).into(),
                width,
                miter_limit: 4.0,
                line_cap: if round { LineCap::Round } else { LineCap::Butt },
                line_join: LineJoin::Miter,
                opacity,
                dash: dash.map(|(dash, gap)| StrokeDash {
                    array: vec![dash, gap],
                    offset: 0.0,
                }),
            }));
            surface.draw_path(&path);
            surface.set_stroke(None);
        }
    }
    if clip.is_some() {
        surface.pop();
    }
}

fn set_color(surface: &mut Surface<'_>, color: Color) {
    let opacity = NormalizedF32::new(f32::from(color.a) / 255.0).unwrap_or(NormalizedF32::ONE);
    surface.set_fill(Some(Fill {
        paint: rgb::Color::new(color.r, color.g, color.b).into(),
        opacity,
        rule: Default::default(),
    }));
}

fn draw_text(surface: &mut Surface<'_>, run: &TextRun, font: Font) {
    let size = run.font_size;
    if size <= 0.0 || run.color.is_transparent() {
        return;
    }
    set_color(surface, run.color);
    // krilla wants advances and offsets as fractions of the font size, with
    // y pointing up.
    let glyphs: Vec<KrillaGlyph> = run
        .glyphs
        .iter()
        .map(|g| {
            KrillaGlyph::new(
                GlyphId::new(g.id),
                g.advance / size,
                g.x_offset / size,
                -g.y_offset / size,
                0.0,
                g.text_range.clone(),
                None,
            )
        })
        .collect();
    surface.draw_glyphs(
        Point::from_xy(run.origin.x, run.origin.y),
        &glyphs,
        font,
        &run.text,
        size,
        false,
    );
}

/// Parsed fonts, keyed by the font blob and collection index.
#[derive(Default)]
struct FontCache {
    fonts: HashMap<(u64, u32), Option<Font>>,
}

impl FontCache {
    fn get(&mut self, data: &FontData) -> Option<Font> {
        self.fonts
            .entry((data.data.id(), data.index))
            .or_insert_with(|| {
                let bytes: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(data.data.clone());
                Font::new(bytes.into(), data.index)
            })
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use presswork_layout::{Engine, Fetcher, LayoutOptions};
    use presswork_style::Stylist;

    fn pages(html: &str) -> Vec<Page> {
        let doc = presswork_dom::parse_html(html.as_bytes());
        let fetcher = Fetcher::data_only();
        let styles = Stylist::for_document(&doc, &fetcher);
        let pages = Engine::new().layout(&doc, &styles, &LayoutOptions::default(), &fetcher);
        assert_eq!(fetcher.failures(), []);
        pages
    }

    fn contains(pdf: &[u8], needle: &[u8]) -> bool {
        pdf.windows(needle.len()).any(|w| w == needle)
    }

    /// A 2x1 PNG: one red pixel, one blue.
    const PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAIAAAB7QOjdAAAAD0lEQVR4nGP4z8DAwPAfAAcAAf9+CLHQAAAAAElFTkSuQmCC";

    #[test]
    fn writes_a_pdf_with_text() {
        let pages = pages("<p style='color: #c00; background: #eee; border: 1px solid'>Hello</p>");
        let pdf = render_pdf(
            &pages,
            &PdfOptions {
                title: Some("Test".into()),
                ..PdfOptions::default()
            },
        )
        .unwrap();
        assert!(pdf.bytes.starts_with(b"%PDF-"));
        assert!(contains(&pdf.bytes, b"Font"), "font is embedded");
    }

    fn count(pdf: &[u8], needle: &[u8]) -> usize {
        pdf.windows(needle.len()).filter(|w| *w == needle).count()
    }

    #[test]
    fn links_and_the_outline() {
        let pages = pages(
            "<h1>Invoice</h1><p><a href='#terms'>Terms</a> <a href='#missing'>x</a>
             <a href='https://example.com/pay'>Pay</a></p><h2 id=terms>Terms</h2>",
        );
        let pdf = render_pdf(&pages, &PdfOptions::default()).unwrap().bytes;
        // The link to a missing anchor is left out.
        assert_eq!(count(&pdf, b"/Subtype/Link"), 2);
        assert!(contains(&pdf, b"/URI(https://example.com/pay)"));
        assert!(contains(&pdf, b"/Outlines"));

        let options = PdfOptions {
            outline_depth: 0,
            external_links: false,
            internal_links: false,
            ..PdfOptions::default()
        };
        let pdf = render_pdf(&pages, &options).unwrap().bytes;
        assert_eq!(count(&pdf, b"/Subtype/Link"), 0);
        assert!(!contains(&pdf, b"/Outlines"));
    }

    #[test]
    fn the_outline_nests_headings_and_stops_at_its_depth() {
        let pages = pages("<h1>A</h1><h3>A.1</h3><h2>A.2</h2><h3>A.2.1</h3><h1>B</h1>");
        let titles = |depth| {
            let options = PdfOptions {
                outline_depth: depth,
                ..PdfOptions::default()
            };
            let pdf = render_pdf(&pages, &options).unwrap().bytes;
            count(&pdf, b"/Title(")
        };
        // Each outline entry has a title.
        assert_eq!(titles(4), 5);
        assert_eq!(titles(2), 4, "A.2.1 is at depth 3");
        assert_eq!(titles(1), 2);
    }

    #[test]
    fn draws_rounded_and_dashed_borders() {
        let pages = pages(
            "<div style='border:2px dashed red; border-radius:6px'>a</div>
             <div style='border-style:solid; border-width:3px; border-color:red blue;
             border-radius:8px; background:#eee'>b</div>
             <div style='border:3px dotted green'>c</div>",
        );
        let pdf = render_pdf(&pages, &PdfOptions::default()).unwrap();
        assert!(pdf.bytes.starts_with(b"%PDF-"));
    }

    #[test]
    fn embeds_images() {
        let pages = pages(&format!("<img src='{PNG}' width=200>"));
        let pdf = render_pdf(&pages, &PdfOptions::default()).unwrap();
        assert_eq!(pdf.broken_images, Vec::<String>::new());
        assert!(contains(&pdf.bytes, b"/Subtype/Image"), "image is embedded");
    }

    #[test]
    fn leaves_out_images_that_do_not_decode() {
        // A valid PNG header and size, then garbage instead of pixel data.
        let mut png = vec![0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
        png.extend([0, 0, 0, 13]);
        png.extend(b"IHDR");
        png.extend([0, 0, 0, 2, 0, 0, 0, 1, 8, 2, 0, 0, 0, 0, 0, 0, 0]);
        png.extend([0xde; 64]);
        let data = Arc::new(ImageData::new(png, "broken.png").unwrap());
        let page = Page {
            size: presswork_layout::Size {
                width: 100.0,
                height: 100.0,
            },
            items: vec![Item::Image(Image {
                rect: presswork_layout::Rect::new(0.0, 0.0, 10.0, 10.0),
                data,
            })],
        };
        let pdf = render_pdf(&[page], &PdfOptions::default()).unwrap();
        assert!(pdf.bytes.starts_with(b"%PDF-"));
        assert_eq!(pdf.broken_images, ["broken.png"]);
    }
}
