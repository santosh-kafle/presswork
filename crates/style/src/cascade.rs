//! The cascade: deciding which declarations apply to an element.

use presswork_dom::resource::{Fetcher, Url};
use presswork_dom::{Document, NodeId};
use selectors::context::SelectorCaches;
use selectors::parser::Selector;

use crate::computed::{ComputedStyle, TextDecorations};
use crate::hints::presentational_hints;
use crate::properties::{Content, Display, LonghandId, PropertyDeclaration};
use crate::selector::{ElementRef, PseudoElement, Simple};
use crate::stylesheet::{
    Declaration, FontFace, PageRule, Stylesheet, media_matches, parse_style_attribute,
};

/// The built-in stylesheet for HTML.
const USER_AGENT_CSS: &str = include_str!("ua.css");

/// Where a stylesheet comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    UserAgent,
    /// wkhtmltopdf's `--user-style-sheet`.
    User,
    Author,
}

/// Cascade precedence, lowest first (CSS Cascade 4, section 6.1, without
/// animations or transitions).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Level {
    UserAgent,
    User,
    /// Presentational hints: author level, below every author rule.
    Hint,
    Author,
    AuthorImportant,
    UserImportant,
    UserAgentImportant,
}

/// How deep `@import`s may nest.
const MAX_IMPORT_DEPTH: usize = 16;

/// A pseudo-element that generates content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pseudo {
    Before,
    After,
}

/// One selector of a style rule with the rule's declarations.
#[derive(Debug)]
struct CascadeRule {
    origin: Origin,
    /// The pseudo-element the rule styles, or `None` for the element.
    pseudo: Option<Pseudo>,
    selector: Selector<Simple>,
    specificity: u32,
    /// Index into [`Stylist::blocks`].
    block: usize,
}

/// Specificity of a `style=""` attribute: above any selector.
const INLINE_SPECIFICITY: u32 = u32::MAX;

/// All the styles of one document; computes the style of its elements.
#[derive(Debug)]
pub struct Stylist {
    /// In source order, which is also cascade order within a level.
    rules: Vec<CascadeRule>,
    blocks: Vec<Vec<Declaration>>,
    page: PageRule,
    font_faces: Vec<FontFace>,
}

impl Default for Stylist {
    fn default() -> Self {
        Self::new()
    }
}

impl Stylist {
    /// A stylist with only the built-in user-agent stylesheet.
    pub fn new() -> Self {
        let mut stylist = Self {
            rules: Vec::new(),
            blocks: Vec::new(),
            page: PageRule::default(),
            font_faces: Vec::new(),
        };
        stylist.add(Origin::UserAgent, Stylesheet::parse(USER_AGENT_CSS));
        stylist
    }

    /// A stylist for `doc`: the user-agent stylesheet plus the document's
    /// `<style>` elements and `<link rel=stylesheet>`s that apply to print,
    /// in document order. Linked and `@import`ed stylesheets load through
    /// `fetcher`, which records those that fail.
    pub fn for_document(doc: &Document, fetcher: &Fetcher<'_>) -> Self {
        let mut stylist = Self::new();
        stylist.add_document(doc, fetcher);
        stylist
    }

    /// Adds the stylesheets of `doc`, as [`Stylist::for_document`] does.
    pub fn add_document(&mut self, doc: &Document, fetcher: &Fetcher<'_>) {
        for node in doc.descendants(doc.root()) {
            let Some(element) = doc.element(node) else {
                continue;
            };
            let is_css = element
                .attr("type")
                .is_none_or(|t| t.trim().is_empty() || t.trim().eq_ignore_ascii_case("text/css"));
            let applies = element.attr("media").is_none_or(media_matches);
            if !is_css || !applies {
                continue;
            }
            match element.name.as_str() {
                "style" => self.add_author_stylesheet(&doc.text_content(node), None, fetcher),
                "link" if is_stylesheet_link(element.attr("rel")) => {
                    if let Some(href) = element.attr("href") {
                        self.link(Origin::Author, href, None, fetcher, &mut Vec::new());
                    }
                }
                _ => {}
            }
        }
    }

    /// Adds an author stylesheet, after any added earlier. `base` is its
    /// URL, for the URLs inside it, or `None` if it is part of the
    /// document.
    pub fn add_author_stylesheet(&mut self, css: &str, base: Option<Url>, fetcher: &Fetcher<'_>) {
        let sheet = Stylesheet::parse(css);
        self.add_with_imports(Origin::Author, sheet, base, fetcher, &mut Vec::new());
    }

    /// Adds a user stylesheet (wkhtmltopdf's `--user-style-sheet`), which
    /// author styles override unless it says `!important`.
    pub fn add_user_stylesheet(&mut self, css: &str, base: Option<Url>, fetcher: &Fetcher<'_>) {
        let sheet = Stylesheet::parse(css);
        self.add_with_imports(Origin::User, sheet, base, fetcher, &mut Vec::new());
    }

    /// The `@font-face` rules of every stylesheet, in cascade order.
    pub fn font_faces(&self) -> &[FontFace] {
        &self.font_faces
    }

    /// Loads and adds the stylesheet at `href`, relative to `base`.
    /// `chain` holds the URLs of the stylesheets importing it, to stop
    /// import cycles.
    fn link(
        &mut self,
        origin: Origin,
        href: &str,
        base: Option<&Url>,
        fetcher: &Fetcher<'_>,
        chain: &mut Vec<Url>,
    ) {
        let url = fetcher.resolve(href, base).ok();
        if chain.len() >= MAX_IMPORT_DEPTH || url.as_ref().is_some_and(|url| chain.contains(url)) {
            return;
        }
        let Some(bytes) = fetcher.fetch(href, base) else {
            return;
        };
        let css = decode_css(&bytes);
        let sheet = Stylesheet::parse(&css);
        self.add_with_imports(origin, sheet, url, fetcher, chain);
    }

    /// Adds `sheet` after the stylesheets it imports.
    fn add_with_imports(
        &mut self,
        origin: Origin,
        mut sheet: Stylesheet,
        base: Option<Url>,
        fetcher: &Fetcher<'_>,
        chain: &mut Vec<Url>,
    ) {
        let imports = std::mem::take(&mut sheet.imports);
        if !imports.is_empty() {
            chain.extend(base.clone());
            for import in imports.iter().filter(|import| import.applies) {
                self.link(origin, &import.url, base.as_ref(), fetcher, chain);
            }
            if base.is_some() {
                chain.pop();
            }
        }
        for face in &mut sheet.font_faces {
            face.base.clone_from(&base);
        }
        if let Some(base) = &base {
            sheet.resolve_urls(base);
        }
        self.add(origin, sheet);
    }

    fn add(&mut self, origin: Origin, mut sheet: Stylesheet) {
        self.page.merge(&sheet.page);
        self.font_faces.append(&mut sheet.font_faces);
        for rule in sheet.rules {
            let block = self.blocks.len();
            self.blocks.push(rule.declarations);
            for selector in rule.selectors.slice() {
                let pseudo = match selector.pseudo_element() {
                    None => None,
                    Some(PseudoElement::Before) => Some(Pseudo::Before),
                    Some(PseudoElement::After) => Some(Pseudo::After),
                    // Other pseudo-elements (`::marker`) are not supported.
                    Some(PseudoElement::Other) => continue,
                };
                self.rules.push(CascadeRule {
                    origin,
                    pseudo,
                    specificity: selector.specificity(),
                    selector: selector.clone(),
                    block,
                });
            }
        }
    }

    /// The declarations of the rules for `pseudo` (or the element itself)
    /// that match `element`, with their cascade positions.
    fn matching_declarations(
        &self,
        element: ElementRef<'_>,
        pseudo: Option<Pseudo>,
    ) -> Vec<(Level, u32, PropertyDeclaration)> {
        let mut applicable = Vec::new();
        let mut caches = SelectorCaches::default();
        for rule in self.rules.iter().filter(|rule| rule.pseudo == pseudo) {
            if !element.matches(&rule.selector, pseudo.is_some(), &mut caches) {
                continue;
            }
            for declaration in &self.blocks[rule.block] {
                let level = match (rule.origin, declaration.important) {
                    (Origin::UserAgent, false) => Level::UserAgent,
                    (Origin::UserAgent, true) => Level::UserAgentImportant,
                    (Origin::User, false) => Level::User,
                    (Origin::User, true) => Level::UserImportant,
                    (Origin::Author, false) => Level::Author,
                    (Origin::Author, true) => Level::AuthorImportant,
                };
                applicable.push((level, rule.specificity, declaration.property.clone()));
            }
        }
        applicable
    }

    /// Whether a document or user rule styles `::before` or `::after`.
    /// The built-in stylesheet only does for `<q>`.
    pub fn has_generated_content(&self) -> bool {
        self.rules
            .iter()
            .any(|rule| rule.pseudo.is_some() && rule.origin != Origin::UserAgent)
    }

    /// The style of the `pseudo` element of `node`, whose own style is
    /// `element_style`, if it generates content: rules for it set
    /// `content` to something other than `normal` or `none`, and
    /// `display` is not `none`.
    pub fn pseudo_style(
        &self,
        doc: &Document,
        node: NodeId,
        pseudo: Pseudo,
        element_style: &ComputedStyle,
    ) -> Option<ComputedStyle> {
        let element = ElementRef::new(doc, node)?;
        let mut applicable = self.matching_declarations(element, Some(pseudo));
        if applicable.is_empty() {
            return None;
        }
        applicable.sort_by_key(|(level, specificity, _)| (*level, *specificity));
        let mut style = ComputedStyle::inheriting(element_style);
        let is_font_size = |d: &PropertyDeclaration| d.id() == LonghandId::FontSize;
        for (_, _, declaration) in applicable.iter().filter(|(_, _, d)| is_font_size(d)) {
            style.apply(declaration, element_style);
        }
        for (_, _, declaration) in applicable.iter().filter(|(_, _, d)| !is_font_size(d)) {
            style.apply(declaration, element_style);
        }
        if style.display == Display::InlineBlock {
            style.inherited_decorations = TextDecorations::NONE;
        }
        let generates = matches!(style.content, Content::Items(_));
        (generates && style.display != Display::None).then_some(style)
    }

    /// The document's `@page` descriptors.
    pub fn page(&self) -> &PageRule {
        &self.page
    }

    /// Computes the style of `node`. `parent` is the computed style of its
    /// parent element, or `None` for the root element.
    ///
    /// Non-element nodes get the inherited style.
    pub fn style(
        &self,
        doc: &Document,
        node: NodeId,
        parent: Option<&ComputedStyle>,
    ) -> ComputedStyle {
        let initial;
        let parent = match parent {
            Some(parent) => parent,
            None => {
                initial = ComputedStyle::initial();
                &initial
            }
        };
        let mut style = ComputedStyle::inheriting(parent);
        let (Some(element), Some(data)) = (ElementRef::new(doc, node), doc.element(node)) else {
            return style;
        };

        if let Some(lang) = data.attr("lang").or_else(|| data.attr("xml:lang")) {
            let lang = lang.trim();
            style.lang = (!lang.is_empty()).then(|| lang.into());
        }

        // Gather every declaration that applies, with its cascade position.
        let mut applicable = self.matching_declarations(element, None);
        for hint in presentational_hints(doc, node, data) {
            applicable.push((Level::Hint, 0, hint));
        }
        if let Some(inline) = data.attr("style") {
            for declaration in parse_style_attribute(inline) {
                let level = if declaration.important {
                    Level::AuthorImportant
                } else {
                    Level::Author
                };
                applicable.push((level, INLINE_SPECIFICITY, declaration.property));
            }
        }

        // Stable sort: ties keep source order, so later declarations win.
        applicable.sort_by_key(|(level, specificity, _)| (*level, *specificity));

        // `em` lengths depend on the element's font size: apply it first.
        let is_root = doc.document_element() == Some(node);
        let is_font_size = |d: &PropertyDeclaration| d.id() == LonghandId::FontSize;
        for (_, _, declaration) in applicable.iter().filter(|(_, _, d)| is_font_size(d)) {
            style.apply(declaration, parent);
        }
        if is_root {
            style.root_font_size = style.font_size;
        }
        for (_, _, declaration) in applicable.iter().filter(|(_, _, d)| !is_font_size(d)) {
            style.apply(declaration, parent);
        }

        // The root element always generates a block box.
        if is_root && matches!(style.display, Display::Inline | Display::InlineBlock) {
            style.display = Display::Block;
        }
        // Decorations do not reach into atomic inlines (CSS Text
        // Decoration 3, section 2.1).
        if style.display == Display::InlineBlock {
            style.inherited_decorations = TextDecorations::NONE;
        }
        style
    }
}

/// Whether a `rel` attribute names a stylesheet that applies by default
/// (not an alternate one).
fn is_stylesheet_link(rel: Option<&str>) -> bool {
    let tokens: Vec<&str> = rel.unwrap_or_default().split_ascii_whitespace().collect();
    tokens.iter().any(|t| t.eq_ignore_ascii_case("stylesheet"))
        && !tokens.iter().any(|t| t.eq_ignore_ascii_case("alternate"))
}

/// Stylesheet bytes as text: UTF-8, without a byte order mark. Invalid
/// sequences become U+FFFD.
fn decode_css(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::properties::{TextAlign, VerticalAlign};
    use crate::values::{Color, Dimension};
    use presswork_dom::parse_html;

    /// The computed style of the first `target` element in `html`.
    fn style_of(html: &str, target: &str) -> ComputedStyle {
        styled(html, target, &Fetcher::data_only())
    }

    fn styled(html: &str, target: &str, fetcher: &Fetcher<'_>) -> ComputedStyle {
        let doc = parse_html(html.as_bytes());
        let stylist = Stylist::for_document(&doc, fetcher);
        let node = doc.find_element(target).expect("target exists");

        let mut chain = vec![node];
        while let Some(parent) = doc.node(*chain.last().unwrap()).parent {
            if doc.element(parent).is_none() {
                break;
            }
            chain.push(parent);
        }
        let mut style = None;
        for &id in chain.iter().rev() {
            style = Some(stylist.style(&doc, id, style.as_ref()));
        }
        style.unwrap()
    }

    const RED: Color = Color::rgb(255, 0, 0);

    #[test]
    fn pseudo_elements_get_their_own_style_when_they_generate_content() {
        let html = "<style>p::before { content: 'x'; color: red } p::after { color: red }
                    .b::after { content: 'y'; display: none }</style>
                    <p class=b>text</p>";
        let doc = parse_html(html.as_bytes());
        let stylist = Stylist::for_document(&doc, &Fetcher::data_only());
        let p = doc.find_element("p").unwrap();
        let style = style_of(html, "p");
        assert_ne!(
            style.color, RED,
            "rules for pseudo-elements do not style the element"
        );
        let before = stylist.pseudo_style(&doc, p, Pseudo::Before, &style);
        let before = before.expect("content set");
        assert_eq!(before.color, RED);
        assert_eq!(
            before.font_size, style.font_size,
            "inherits from the element"
        );
        // No content, or display: none.
        assert!(
            stylist
                .pseudo_style(&doc, p, Pseudo::After, &style)
                .is_none()
        );
        assert!(stylist.has_generated_content());
        assert!(!Stylist::new().has_generated_content());
    }
    const BLUE: Color = Color::rgb(0, 0, 255);

    #[test]
    fn user_agent_defaults() {
        let h1 = style_of("<h1>x</h1>", "h1");
        assert_eq!(h1.display, Display::Block);
        assert_eq!(h1.font_size, 24.0);
        assert_eq!(h1.font_weight, 700.0);
        assert!((h1.margin.top.resolve(0.0).unwrap() - 0.67 * 24.0).abs() < 1e-3);
        assert_eq!(
            style_of("<script>x</script>", "script").display,
            Display::None
        );
        assert_eq!(style_of("<p hidden>x</p>", "p").display, Display::None);
        assert_eq!(style_of("<body>", "body").margin.left, Dimension::Pt(6.0));
    }

    #[test]
    fn specificity_beats_source_order() {
        let html = "<style>#a { color: red } p.x { color: blue } p { color: green }</style>
                    <p id=a class=x>";
        assert_eq!(style_of(html, "p").color, RED);
    }

    #[test]
    fn later_rules_win_ties() {
        let html = "<style>p { color: red } p { color: blue }</style><p>";
        assert_eq!(style_of(html, "p").color, BLUE);
    }

    #[test]
    fn important_and_inline_styles() {
        let html = "<style>p { color: red !important } #a { color: green }</style>
                    <p id=a style='color: blue'>";
        assert_eq!(style_of(html, "p").color, RED);
        let html = "<style>#a { color: red }</style><p id=a style='color: blue'>";
        assert_eq!(style_of(html, "p").color, BLUE);
        let html = "<style>p { color: red !important }</style>
                    <p style='color: blue !important'>";
        assert_eq!(style_of(html, "p").color, BLUE);
    }

    #[test]
    fn author_rules_override_hints_and_hints_override_the_user_agent() {
        let th = style_of("<table><tr><th align=right>", "th");
        assert_eq!(th.text_align, TextAlign::Right);
        let th = style_of(
            "<style>th { text-align: left }</style><table><tr><th align=right>",
            "th",
        );
        assert_eq!(th.text_align, TextAlign::Left);
    }

    #[test]
    fn inheritance_and_em_units() {
        let html = "<style>div { font-size: 20pt; color: red } p { margin-left: 2em; font-size: 50% }</style>
                    <div><p><b>x</b></p></div>";
        let p = style_of(html, "p");
        assert_eq!(p.font_size, 10.0);
        assert_eq!(p.margin.left, Dimension::Pt(20.0));
        let b = style_of(html, "b");
        assert_eq!((b.color, b.font_size, b.font_weight), (RED, 10.0, 700.0));
        assert_eq!(b.margin.left, Dimension::ZERO);
    }

    #[test]
    fn rem_uses_the_root_font_size() {
        let html =
            "<style>html { font-size: 10pt } p { font-size: 30pt; padding-left: 2rem }</style><p>";
        assert_eq!(style_of(html, "p").padding.left, Dimension::Pt(20.0));
    }

    #[test]
    fn cells_inherit_vertical_align_from_the_row_group() {
        let html = "<table><tbody valign=bottom><tr><td>";
        assert_eq!(style_of(html, "td").vertical_align, VerticalAlign::Bottom);
        assert_eq!(
            style_of("<table><tr><td>", "td").vertical_align,
            VerticalAlign::Middle
        );
    }

    #[test]
    fn lang_is_inherited() {
        let html = "<html lang=ne><body><p><span>";
        assert_eq!(style_of(html, "span").lang.as_deref(), Some("ne"));
    }

    #[test]
    fn style_elements_for_other_media_are_ignored() {
        let html = "<style media=screen>p { color: red }</style>
                    <style media=print>p { font-size: 20pt }</style><p>";
        let p = style_of(html, "p");
        assert_eq!((p.color, p.font_size), (Color::BLACK, 20.0));
    }

    #[test]
    fn page_rule_is_collected() {
        let doc = parse_html(b"<style>@page { margin: 1in }</style>");
        assert_eq!(
            Stylist::for_document(&doc, &Fetcher::data_only())
                .page()
                .margin
                .top,
            Some(72.0)
        );
    }

    /// Serves stylesheets from a list of `(url, css)`.
    struct Files(Vec<(&'static str, &'static str)>);

    impl presswork_dom::resource::Loader for Files {
        fn load(&self, url: &Url) -> Result<Vec<u8>, presswork_dom::resource::LoadError> {
            self.0
                .iter()
                .find(|(path, _)| *path == url.as_str())
                .map(|(_, css)| css.as_bytes().to_vec())
                .ok_or_else(|| presswork_dom::resource::LoadError::Failed("not found".into()))
        }
    }

    fn site(files: Vec<(&'static str, &'static str)>) -> Files {
        Files(files)
    }

    fn base() -> Option<Url> {
        Url::parse("file:///site/invoice.html").ok()
    }

    #[test]
    fn linked_and_imported_stylesheets_apply_in_document_order() {
        let files = site(vec![
            (
                "file:///site/css/main.css",
                "@import 'base.css'; p { color: red }",
            ),
            (
                "file:///site/css/base.css",
                "p { color: blue; text-align: center }",
            ),
        ]);
        let fetcher = Fetcher::new(&files, base());
        let html = "<link rel=stylesheet href=css/main.css><p>x";
        let style = styled(html, "p", &fetcher);
        // The imported sheet comes first, so main.css wins the color.
        assert_eq!(style.color, RED);
        assert_eq!(style.text_align, TextAlign::Center);
        assert!(fetcher.failures().is_empty());

        // A later <style> wins over the linked sheet.
        let html = "<link rel='Stylesheet' href=css/main.css><style>p { color: blue }</style><p>x";
        assert_eq!(styled(html, "p", &fetcher).color, BLUE);
    }

    #[test]
    fn skips_alternate_and_non_print_links_and_reports_missing_ones() {
        let files = site(vec![("file:///site/a.css", "p { color: red }")]);
        let fetcher = Fetcher::new(&files, base());
        for html in [
            "<link rel='alternate stylesheet' href=a.css><p>x",
            "<link rel=stylesheet media=screen href=a.css><p>x",
            "<link rel=icon href=a.css><p>x",
        ] {
            assert_eq!(styled(html, "p", &fetcher).color, Color::BLACK, "{html}");
        }
        styled("<link rel=stylesheet href=missing.css><p>x", "p", &fetcher);
        let failures = fetcher.failures();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].url, "missing.css");
    }

    #[test]
    fn import_cycles_stop() {
        let files = site(vec![
            ("file:///site/a.css", "@import 'b.css'; p { color: red }"),
            (
                "file:///site/b.css",
                "@import 'a.css'; p { text-align: right }",
            ),
        ]);
        let fetcher = Fetcher::new(&files, base());
        let style = styled("<link rel=stylesheet href=a.css><p>x", "p", &fetcher);
        assert_eq!(style.color, RED);
        assert_eq!(style.text_align, TextAlign::Right);
    }

    #[test]
    fn font_faces_resolve_against_their_stylesheet() {
        let files = site(vec![(
            "file:///site/css/fonts.css",
            "@font-face { font-family: Brand; src: url(../fonts/brand.ttf) }",
        )]);
        let fetcher = Fetcher::new(&files, base());
        let doc = parse_html(
            b"<link rel=stylesheet href=css/fonts.css>\
              <style>@font-face { font-family: Inline; src: url(x.ttf) }</style>",
        );
        let stylist = Stylist::for_document(&doc, &fetcher);
        let faces = stylist.font_faces();
        assert_eq!(faces.len(), 2);
        assert_eq!(faces[0].family, "Brand");
        let base = faces[0].base.as_ref().unwrap();
        assert_eq!(
            fetcher
                .resolve("../fonts/brand.ttf", Some(base))
                .unwrap()
                .as_str(),
            "file:///site/fonts/brand.ttf"
        );
        assert_eq!(faces[1].base, None);
    }

    #[test]
    fn background_urls_resolve_against_their_stylesheet() {
        let doc = parse_html(b"<p style='background-image: url(inline.png)'>x</p><div>y</div>");
        let fetcher = Fetcher::data_only();
        let mut stylist = Stylist::for_document(&doc, &fetcher);
        stylist.add_user_stylesheet(
            "div { background: url(img/logo.png) no-repeat }",
            Url::parse("file:///site/css/print.css").ok(),
            &fetcher,
        );
        let html = doc.document_element().unwrap();
        let root = stylist.style(&doc, html, None);
        let div = stylist.style(&doc, doc.find_element("div").unwrap(), Some(&root));
        assert_eq!(
            div.background_image.as_deref(),
            Some("file:///site/css/img/logo.png")
        );
        assert_eq!(div.background_repeat, (false, false));
        // Document styles resolve against the document, later.
        let p = stylist.style(&doc, doc.find_element("p").unwrap(), Some(&root));
        assert_eq!(p.background_image.as_deref(), Some("inline.png"));
    }

    #[test]
    fn user_styles_lose_to_author_styles_unless_important() {
        let doc = parse_html(b"<style>p { color: red; text-align: right }</style><p>x");
        let fetcher = Fetcher::data_only();
        let mut stylist = Stylist::for_document(&doc, &fetcher);
        stylist.add_user_stylesheet(
            "p { color: blue; text-align: center !important; font-size: 20pt }",
            None,
            &fetcher,
        );
        let p = doc.find_element("p").unwrap();
        let body = doc.find_element("body").unwrap();
        let html = doc.document_element().unwrap();
        let root = stylist.style(&doc, html, None);
        let body = stylist.style(&doc, body, Some(&root));
        let style = stylist.style(&doc, p, Some(&body));
        assert_eq!(style.color, RED);
        assert_eq!(style.text_align, TextAlign::Center);
        assert_eq!(style.font_size, 20.0);
    }
}
