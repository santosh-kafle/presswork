//! Layout and pagination.
//!
//! Pipeline stages owned by this crate:
//!
//! 1. **Layout** ([`flow`]): block, inline (text) and table formatting on
//!    one continuous canvas, using the computed styles from
//!    `presswork-style`. Text shaping, line breaking and font fallback come
//!    from `parley`.
//! 2. **Fragmentation** ([`paginate`]): cut the canvas into pages between
//!    lines and table rows.
//! 3. **Margin text** ([`Engine::draw_band`]): headers and footers, drawn
//!    on each page once the page count is known.
//!
//! The output is a list of positioned items per page, consumed by
//! `presswork-paint`. All geometry is expressed in typographic points
//! (1pt = 1/72 in).
//!
//! Images and `@font-face` fonts are loaded through a [`Fetcher`], whose
//! [`Loader`] decides what may be read; layout itself never touches files
//! or the network.

mod flow;
pub mod fragment;
pub mod geometry;
mod margin;
pub mod page;
mod paginate;

use std::collections::HashMap;
use std::fmt;

use parley::{FontContext, LayoutContext};
use presswork_dom::Document;
pub use presswork_dom::resource::{DataUrls, Fetcher, LoadError, LoadFailure, Loader};
use presswork_style::Stylist;

pub use fragment::{
    Color, Fill, FontData, Glyph, Image, ImageData, ImageFormat, Item, Link, LinkTarget, Mark,
    MarkKind, Page, PathOp, Shape, ShapePaint, TextRun,
};
pub use geometry::{EdgeSizes, Point, Rect, Size};
pub use margin::{Band, BandStyle, BandText, MarginDocument};
pub use page::{Orientation, PageSize};

/// Page geometry for a layout.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutOptions {
    pub page_size: Size,
    pub margins: EdgeSizes,
    /// Whether to load images; without them, `alt` text shows instead.
    pub images: bool,
}

/// wkhtmltopdf's default page margin: 10mm.
pub const DEFAULT_MARGIN: f32 = 10.0 * 72.0 / 25.4;

impl Default for LayoutOptions {
    /// A4 portrait with wkhtmltopdf's default 10mm margins.
    fn default() -> Self {
        Self {
            page_size: PageSize::A4.size(Orientation::Portrait),
            margins: EdgeSizes::uniform(DEFAULT_MARGIN),
            images: true,
        }
    }
}

/// Lays out documents. Holds the font database, which is expensive to
/// build, so reuse one engine for many documents.
pub struct Engine {
    fonts: FontContext,
    generics: flow::GenericFonts,
    texts: LayoutContext<flow::Ink>,
    /// Layouts so far, to give each its own `@font-face` names.
    layouts: u64,
}

impl fmt::Debug for Engine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Engine").finish_non_exhaustive()
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    /// Creates an engine that uses the system's fonts.
    pub fn new() -> Self {
        let mut fonts = FontContext::new();
        let generics = flow::GenericFonts::resolve(&mut fonts);
        Self {
            fonts,
            generics,
            texts: LayoutContext::new(),
            layouts: 0,
        }
    }

    /// Whether the system provided any fonts. Without them, text is laid
    /// out but no glyphs can be drawn.
    ///
    /// On Linux this is false when fontconfig is not installed: it is
    /// loaded at runtime, not linked.
    pub fn has_system_fonts(&self) -> bool {
        self.generics.any()
    }

    /// Lays out `doc`, styled by `styles`, into pages. Images and fonts
    /// load through `fetcher`, which records those that fail; use
    /// [`Fetcher::data_only`] to load only inline `data:` URLs.
    pub fn layout(
        &mut self,
        doc: &Document,
        styles: &Stylist,
        options: &LayoutOptions,
        fetcher: &Fetcher<'_>,
    ) -> Vec<Page> {
        let margins = options.margins;
        let width = (options.page_size.width - margins.left - margins.right).max(0.0);

        let generated = flow::generated::resolve(doc, styles);
        let mut context = flow::Context {
            generated: &generated,
            fonts: &mut self.fonts,
            generics: &self.generics,
            texts: &mut self.texts,
            doc,
            styles,
            fetcher,
            web_fonts: flow::WebFonts::new(styles.font_faces(), self.layouts),
            load_images: options.images,
            images: HashMap::new(),
        };
        self.layouts += 1;
        let mut flow = flow::Flow::new(&mut context);
        if let Some(root) = doc.document_element() {
            let style = styles.style(doc, root, None);
            flow.block(root, &style, 0.0, width);
        }
        let (canvas, _height) = flow.finish();
        context.web_fonts.unregister(context.fonts);
        paginate::paginate(canvas, options.page_size, margins)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod engine_tests {
    use super::Engine;

    /// The test machines have fonts; if this fails, every text test would
    /// fail in confusing ways, so fail here first with a clear message.
    #[test]
    fn system_fonts_are_available() {
        assert!(
            Engine::new().has_system_fonts(),
            "no system fonts found: install fontconfig and a font package"
        );
    }
}
