//! Headers and footers in the page margins, as wkhtmltopdf draws them.
//!
//! Text headers (`--header-left` and friends): one line of text just above
//! the content area, with up to three parts (left, center, right) over the
//! content width, and footers the same way just below it.
//! `--header-spacing` moves the text away from the content, and
//! `--header-line` rules a line between the two. The text does not wrap,
//! and the page margins are not enlarged to make room: callers set
//! `--margin-top` to fit their header.
//!
//! HTML headers (`--header-html`): a whole document laid out at the content
//! width, with the bottom of its `<body>` box `--header-spacing` above the
//! content area (a footer's body box starts that far below it). Its body
//! height is what wkhtmltopdf sizes the margin from when none is given, so
//! the body box fills the margin exactly and its margins fall outside.

use std::collections::HashMap;
use std::sync::Arc;

use presswork_dom::Document;
use presswork_dom::resource::Fetcher;
use presswork_style::{
    Color, ComputedStyle, FontFamilyName, GenericFamily, Stylist, TextAlign, WhiteSpace,
};

use crate::Engine;
use crate::flow;
use crate::fragment::{Fill, Item, Page};
use crate::geometry::{EdgeSizes, Rect, Size};

/// Which margin a line of text goes in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    /// Above the content area.
    Header,
    /// Below the content area.
    Footer,
}

/// How a header or footer is drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct BandStyle {
    /// Tried first; the default sans-serif font fills in for it and for
    /// characters it lacks.
    pub font_family: String,
    /// In points.
    pub font_size: f32,
    /// Whether to rule a line between the text and the content area.
    pub line: bool,
    /// Distance between the content area and the text (and the line), in
    /// points. Negative values move the text into the content area.
    pub spacing: f32,
}

/// What a header or footer says on one page. Each part may have several
/// lines; empty parts are not drawn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BandText<'a> {
    pub left: &'a str,
    pub center: &'a str,
    pub right: &'a str,
}

/// Thickness of the header and footer lines. wkhtmltopdf draws a hairline,
/// which PDF viewers show as thin as the screen allows.
const RULE: f32 = 0.5;

impl Engine {
    /// Draws a header or footer on `page`, whose content area is inside
    /// `margins`. Draws over whatever is already there.
    ///
    /// Header text hangs above the content area: its first line ends
    /// `style.spacing` above it, and further lines (from line breaks in
    /// the text) run down toward the content. Footer text stands below
    /// the content area the same way: its last line starts `spacing`
    /// below it. This is where wkhtmltopdf's Qt `drawText` puts them.
    pub fn draw_band(
        &mut self,
        page: &mut Page,
        band: Band,
        style: &BandStyle,
        text: BandText<'_>,
        margins: EdgeSizes,
    ) {
        let width = (page.size.width - margins.left - margins.right).max(0.0);
        let edge = match band {
            Band::Header => margins.top - style.spacing,
            Band::Footer => page.size.height - margins.bottom + style.spacing,
        };
        if style.line {
            page.items.push(Item::Fill(Fill {
                rect: Rect::new(margins.left, edge - RULE / 2.0, width, RULE),
                color: Color::BLACK,
            }));
        }
        let parts = [
            (text.left, TextAlign::Left),
            (text.center, TextAlign::Center),
            (text.right, TextAlign::Right),
        ];
        if parts.iter().all(|(text, _)| text.is_empty()) {
            return;
        }

        let doc = Document::new();
        let styles = Stylist::new();
        let fetcher = Fetcher::data_only();
        let generated = flow::generated::GeneratedContent::new();
        let mut cx = flow::Context {
            generated: &generated,
            fonts: &mut self.fonts,
            generics: &self.generics,
            texts: &mut self.texts,
            doc: &doc,
            styles: &styles,
            fetcher: &fetcher,
            web_fonts: flow::WebFonts::new(&[], 0),
            load_images: false,
            images: HashMap::new(),
            page: Size::default(),
            fixed: Vec::new(),
            page_background_owner: None,
        };
        let mut css = ComputedStyle::initial();
        css.font_family = Arc::new([
            FontFamilyName::Named(style.font_family.clone()),
            FontFamilyName::Generic(GenericFamily::SansSerif),
        ]);
        css.font_size = style.font_size;
        // Qt draws the text as given: spaces kept, lines broken only at
        // line breaks.
        css.white_space = WhiteSpace::Pre;

        for (text, align) in parts {
            if text.is_empty() {
                continue;
            }
            css.text_align = align;
            let canvas = flow::text_block(&mut cx, text, &css, width);
            let (Some(first), Some(last)) = (canvas.slices.first(), canvas.slices.last()) else {
                continue;
            };
            let line = first.height;
            let height = (last.top - first.top) as f32 + last.height;
            let top = match band {
                Band::Header => edge - line,
                Band::Footer => edge + line - height,
            } - first.top as f32;
            canvas.into_items(margins.left, top, &mut page.items);
        }
    }
}

/// A document laid out to be drawn in a page margin: wkhtmltopdf's
/// `--header-html` and `--footer-html`.
#[derive(Debug, Clone)]
pub struct MarginDocument {
    /// Positioned from the top-left of the document.
    pub items: Vec<Item>,
    /// Top of the `<body>` element's border box, in points: its margin, or
    /// a larger margin of its first child that collapses through it.
    pub body_top: f32,
    /// Height of the `<body>` element's border box, in points.
    pub body_height: f32,
}

impl Engine {
    /// Lays out `doc` as a header or footer `width` points wide, on one
    /// unbroken canvas. Images and fonts load through `fetcher`.
    pub fn layout_margin_document(
        &mut self,
        doc: &Document,
        styles: &Stylist,
        width: f32,
        images: bool,
        fetcher: &Fetcher<'_>,
    ) -> MarginDocument {
        let generated = flow::generated::resolve(doc, styles);
        let mut cx = flow::Context {
            generated: &generated,
            fonts: &mut self.fonts,
            generics: &self.generics,
            texts: &mut self.texts,
            doc,
            styles,
            fetcher,
            web_fonts: flow::WebFonts::new(styles.font_faces(), self.layouts),
            load_images: images,
            images: HashMap::new(),
            page: Size::default(),
            fixed: Vec::new(),
            page_background_owner: None,
        };
        self.layouts += 1;
        let mut flow = flow::Flow::new(&mut cx);
        if let Some(body) = doc.find_element("body") {
            flow.track(body);
        }
        if let Some(root) = doc.document_element() {
            let style = styles.style(doc, root, None);
            flow.block(root, &style, 0.0, width.max(0.0));
        }
        let body = flow.tracked_box();
        let (canvas, height) = flow.finish();
        cx.web_fonts.unregister(cx.fonts);
        let (body_top, body_bottom) = body.unwrap_or((0.0, height));
        let mut items = Vec::new();
        canvas.into_items(0.0, 0.0, &mut items);
        // Headings and anchors in a header belong to no place in the
        // document: they would repeat on every page.
        items.retain(|item| !matches!(item, Item::Mark(_)));
        MarginDocument {
            items,
            body_top: body_top as f32,
            body_height: (body_bottom - body_top) as f32,
        }
    }
}

impl MarginDocument {
    /// Draws the document on `page`, whose content area is inside
    /// `margins`, over whatever is already there.
    ///
    /// A header's body box ends `spacing` above the content area, and a
    /// footer's starts `spacing` below it.
    ///
    /// wkhtmltopdf's code moves the whole document by the body height, so
    /// a body margin (8px by default) may push its header into the content
    /// and its footer off the page. Placing the body box instead is the
    /// same for documents without a body margin, and keeps the others
    /// inside the margin they were sized for.
    pub fn draw(&self, page: &mut Page, band: Band, spacing: f32, margins: EdgeSizes) {
        let body = match band {
            Band::Header => margins.top - spacing - self.body_height,
            Band::Footer => page.size.height - margins.bottom + spacing,
        };
        let top = body - self.body_top;
        page.items.extend(
            self.items
                .iter()
                .cloned()
                .map(|item| item.translate(margins.left, top)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Size;

    const SIZE: Size = Size {
        width: 200.0,
        height: 300.0,
    };
    const MARGINS: EdgeSizes = EdgeSizes::uniform(30.0);

    fn style(line: bool, spacing: f32) -> BandStyle {
        BandStyle {
            font_family: "Arial".into(),
            font_size: 12.0,
            line,
            spacing,
        }
    }

    fn blank() -> Page {
        Page {
            size: SIZE,
            items: Vec::new(),
        }
    }

    /// Text runs as (text, x, baseline).
    fn runs(page: &Page) -> Vec<(String, f32, f32)> {
        page.items
            .iter()
            .filter_map(|item| match item {
                Item::Text(run) => {
                    let start = run.glyphs.first()?.text_range.start;
                    let end = run.glyphs.iter().map(|g| g.text_range.end).max()?;
                    Some((run.text[start..end].to_owned(), run.origin.x, run.origin.y))
                }
                _ => None,
            })
            .collect()
    }

    fn fills(page: &Page) -> Vec<Rect> {
        page.items
            .iter()
            .filter_map(|item| match item {
                Item::Fill(fill) => Some(fill.rect),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn header_sits_above_the_content_in_three_parts() {
        let mut engine = Engine::new();
        let mut page = blank();
        let text = BandText {
            left: "Invoice",
            center: "ACME",
            right: "1/3",
        };
        engine.draw_band(&mut page, Band::Header, &style(false, 0.0), text, MARGINS);
        let runs = runs(&page);
        assert_eq!(runs.len(), 3, "{runs:?}");
        let [(left, lx, ly), (center, cx, cy), (right, rx, ry)] = &runs[..] else {
            unreachable!()
        };
        assert_eq!((&**left, &**center, &**right), ("Invoice", "ACME", "1/3"));
        // One baseline, inside the top margin, close to the content.
        assert!(ly == cy && cy == ry, "{runs:?}");
        assert!(*ly > 30.0 - 18.0 && *ly < 30.0, "baseline {ly}");
        // Left at the content's left edge, the others centered and flush
        // right within the content width.
        assert_eq!(*lx, 30.0);
        assert!(*cx > 80.0 && *cx < 100.0, "center at {cx}");
        assert!(*rx > 140.0 && *rx < 170.0, "right at {rx}");
    }

    #[test]
    fn footer_sits_below_the_content() {
        let mut engine = Engine::new();
        let mut page = blank();
        let text = BandText {
            center: "Page 2",
            ..BandText::default()
        };
        engine.draw_band(&mut page, Band::Footer, &style(false, 0.0), text, MARGINS);
        let [(_, _, baseline)] = runs(&page)[..] else {
            panic!("one run expected")
        };
        // The text's line starts at the content's bottom edge; fonts'
        // ascent is often a little more than their size.
        assert!(
            baseline > 270.0 + 6.0 && baseline < 270.0 + 18.0,
            "{baseline}"
        );
    }

    #[test]
    fn spacing_moves_text_and_line_away_from_the_content() {
        let mut engine = Engine::new();
        let text = BandText {
            left: "x",
            ..BandText::default()
        };
        let mut close = blank();
        engine.draw_band(&mut close, Band::Header, &style(true, 0.0), text, MARGINS);
        let mut spaced = blank();
        engine.draw_band(&mut spaced, Band::Header, &style(true, 5.0), text, MARGINS);
        assert_eq!(runs(&close)[0].2 - runs(&spaced)[0].2, 5.0);
        assert_eq!(
            fills(&close),
            [Rect::new(30.0, 30.0 - RULE / 2.0, 140.0, RULE)]
        );
        assert_eq!(fills(&spaced)[0].origin.y, 25.0 - RULE / 2.0);

        let mut footer = blank();
        engine.draw_band(&mut footer, Band::Footer, &style(true, 5.0), text, MARGINS);
        assert_eq!(fills(&footer)[0].origin.y, 275.0 - RULE / 2.0);
    }

    #[test]
    fn line_alone_draws_without_text() {
        let mut engine = Engine::new();
        let mut page = blank();
        let empty = BandText::default();
        engine.draw_band(&mut page, Band::Footer, &style(true, 0.0), empty, MARGINS);
        assert_eq!(fills(&page).len(), 1);
        assert!(runs(&page).is_empty());

        let mut page = blank();
        engine.draw_band(&mut page, Band::Footer, &style(false, 0.0), empty, MARGINS);
        assert!(page.items.is_empty());
    }

    #[test]
    fn devanagari_falls_back_to_a_font_that_has_it() {
        let mut engine = Engine::new();
        let mut page = blank();
        let text = BandText {
            right: "पृष्ठ २",
            ..BandText::default()
        };
        engine.draw_band(&mut page, Band::Footer, &style(false, 0.0), text, MARGINS);
        let glyphs: Vec<u32> = page
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Text(run) => Some(run.glyphs.iter().map(|g| g.id)),
                _ => None,
            })
            .flatten()
            .collect();
        assert!(!glyphs.is_empty() && !glyphs.contains(&0), "{glyphs:?}");
    }

    fn margin_document(html: &str) -> MarginDocument {
        let doc = presswork_dom::parse_html(html.as_bytes());
        let fetcher = Fetcher::data_only();
        let styles = Stylist::for_document(&doc, &fetcher);
        Engine::new().layout_margin_document(&doc, &styles, 140.0, true, &fetcher)
    }

    #[test]
    fn body_height_leaves_out_the_body_margins() {
        let block = "<div style=\"padding-top: 20pt; background: red\"></div>";
        let bare = margin_document(&format!("<body style=\"margin: 0\">{block}"));
        assert_eq!(bare.body_height, 20.0);
        // The default 8px body margin is outside the box.
        let default = margin_document(block);
        assert_eq!((default.body_top, default.body_height), (6.0, 20.0));
        // Padding is inside it; a child's margin collapses through the
        // body's top and stays outside.
        let padded = margin_document(&format!(
            "<body style=\"margin: 0; padding-bottom: 5pt\"><div style=\"margin-top: 30pt\"></div>{block}"
        ));
        assert_eq!((padded.body_top, padded.body_height), (30.0, 25.0));
    }

    #[test]
    fn body_margins_stay_outside_the_page_margin() {
        // A paragraph's 1em margin collapses through the body's 8px one.
        let doc =
            margin_document("<p style=\"padding-top: 10pt; margin: 12pt 0; background: red\"></p>");
        assert_eq!((doc.body_top, doc.body_height), (12.0, 10.0));
        let mut page = blank();
        doc.draw(&mut page, Band::Header, 0.0, MARGINS);
        doc.draw(&mut page, Band::Footer, 0.0, MARGINS);
        assert_eq!(
            fills(&page),
            [
                // Indented by the body's 8px side margins.
                Rect::new(36.0, 20.0, 128.0, 10.0),
                Rect::new(36.0, 270.0, 128.0, 10.0)
            ]
        );
    }

    #[test]
    fn header_documents_end_above_the_content_and_footers_start_below() {
        let doc = margin_document(
            "<body style=\"margin: 0\"><div style=\"padding-top: 20pt; background: red\"></div>",
        );
        let mut page = blank();
        doc.draw(&mut page, Band::Header, 4.0, MARGINS);
        doc.draw(&mut page, Band::Footer, 4.0, MARGINS);
        assert_eq!(
            fills(&page),
            [
                Rect::new(30.0, 30.0 - 4.0 - 20.0, 140.0, 20.0),
                Rect::new(30.0, 270.0 + 4.0, 140.0, 20.0)
            ]
        );
    }
}
