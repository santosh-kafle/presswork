//! Inline layout: gathering text, shaping it and breaking it into lines.

use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use parley::{
    Alignment, AlignmentOptions, FontStyle, FontWeight, GlyphRun, IndentOptions, InlineBox,
    InlineBoxKind, Language, Layout, LineBreakContext, LineHeight as ParleyLineHeight,
    PositionedLayoutItem, TextStyle, TextWrapMode, TreeBuilder,
    VerticalAlign as ParleyVerticalAlign, WhiteSpaceCollapse,
};
use presswork_dom::{Document, NodeId};
use presswork_style::{
    Color, ComputedStyle, FontFamilyName, LineHeight, ListStyleType, Pseudo, Side, Sides,
    TextAlign, TextDecorationLine, TextDecorations, TextTransform, VerticalAlign, WhiteSpace,
};
use read_fonts::{FontRef, TableProvider};

use super::generated::counter_text;
use super::{BoxPaint, Context, Flow, GenericFonts, WebFonts, mark};
use crate::fragment::{Fill, FontData, Glyph, Item, Link, LinkTarget, MarkKind, TextRun};
use crate::geometry::{Point, Rect};

/// How text is painted, as parley's per-span "brush": its color and the
/// lines drawn on it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Ink {
    pub color: Color,
    pub decorations: TextDecorations,
}

impl Default for Ink {
    fn default() -> Self {
        Ink {
            color: Color::BLACK,
            decorations: TextDecorations::NONE,
        }
    }
}

/// Inline content gathered for one paragraph: the text after white-space
/// processing, the style of each stretch of it, and the boxes (images)
/// that flow with it.
#[derive(Default)]
pub(super) struct Inline {
    text: String,
    spans: Vec<Span>,
    boxes: Vec<InlineObject>,
    /// The style of each inline element entered, which parley needs as
    /// the parent of the text inside: `vertical-align` is not inherited,
    /// and shifts relative to the parent's font.
    nesting: Vec<ComputedStyle>,
    /// The inline elements now open, outermost first, with their index
    /// in `nesting`.
    open: Vec<(ElementKey, usize)>,
    /// The indices in `nesting` of `open`.
    path: Rc<[usize]>,
    /// The paragraph starts its block container, so `text-indent`
    /// applies to it.
    first: bool,
    /// Inline elements with a background, borders, or horizontal
    /// margins or padding, in tree order.
    elements: Vec<InlineElement>,
    /// Elements opened with no content after them yet. Their start is
    /// placed (and their start spacer pushed) just before the next
    /// content, so that a collapsible space between them and the text
    /// before stays outside, where the line can break.
    pending: Vec<usize>,
    /// Byte offsets of the U+FFFC characters that are spacers rather
    /// than images, ascending.
    spacers: Vec<usize>,
    /// Inline elements that links can point to: where they start, and
    /// their names.
    anchors: Vec<(usize, Arc<str>)>,
}

/// An element, or one of its `::before` and `::after` boxes.
pub(super) type ElementKey = (NodeId, Option<Pseudo>);

/// A stretch of text in one style.
struct Span {
    range: Range<usize>,
    style: ComputedStyle,
    /// The inline elements it is in, as indices in [`Inline::nesting`],
    /// outermost first.
    path: Rc<[usize]>,
}

/// An inline element whose box is painted or takes horizontal space.
///
/// Its left margin, border and padding are a spacer box (an empty inline
/// object) before its content, the right ones a spacer after it. Its
/// background and borders are painted per line, around its content on
/// that line, from its font's ascent and descent plus the vertical
/// padding and borders, which do not affect the line height (CSS 2.1
/// section 10.8.1).
struct InlineElement {
    node: ElementKey,
    style: ComputedStyle,
    /// The elements it is in, and itself.
    path: Rc<[usize]>,
    /// From its start spacer (or first character) to the end of its end
    /// spacer (or last character). [`OPEN`] until started or ended: an
    /// element left open by a block inside it extends to the end.
    range: Range<usize>,
    paint: Option<BoxPaint>,
    /// Where the element links to, if it is a link.
    link: Option<LinkTarget>,
    /// Only the left and right margins are used; negative ones count as
    /// zero.
    margin: Sides<f32>,
    border: Sides<f32>,
    padding: Sides<f32>,
}

impl InlineElement {
    fn start_width(&self) -> f32 {
        self.margin.left + self.border.left + self.padding.left
    }

    fn end_width(&self) -> f32 {
        self.margin.right + self.border.right + self.padding.right
    }
}

/// An atomic box laid out inline, like a character: an image.
pub(super) struct InlineObject {
    /// Byte offset of the U+FFFC character that stands for the box in the
    /// paragraph's text. Set by [`Inline::push_box`].
    index: usize,
    /// Margin box size.
    pub width: f32,
    pub height: f32,
    /// The narrowest the box can be: zero for an image sized as a
    /// percentage of its container, which shrinks to fit; otherwise
    /// `width`. Only used when measuring.
    pub min_width: f32,
    pub vertical_align: VerticalAlign,
    /// Content relative to the top-left of the margin box.
    pub items: Vec<Item>,
    /// Distance from the top of the margin box to the baseline that
    /// sits on the line's; `None` for the bottom edge.
    pub baseline: Option<f32>,
    /// The left or right edges of an inline element: no break between
    /// it and the text next to it.
    spacer: bool,
}

/// How far, in points, a line may exceed its width and still fit.
const FIT_TOLERANCE: f32 = 0.001;

/// The start or end of an inline element not yet placed.
const OPEN: usize = usize::MAX;

/// Tab stops in preformatted text, in characters.
const TAB_SIZE: usize = 8;

/// Stands for an inline box in the text given to parley.
const OBJECT_REPLACEMENT: char = '\u{FFFC}';

impl InlineObject {
    pub fn new(
        width: f32,
        height: f32,
        min_width: f32,
        vertical_align: VerticalAlign,
        items: Vec<Item>,
    ) -> Self {
        Self {
            index: 0,
            width,
            height,
            min_width,
            vertical_align,
            items,
            baseline: None,
            spacer: false,
        }
    }

    fn spacer(width: f32) -> Self {
        Self {
            spacer: true,
            ..Self::new(width, 0.0, width, VerticalAlign::Baseline, Vec::new())
        }
    }
}

impl Inline {
    /// Content that starts a block container: `text-indent` applies to it.
    /// Taking it with `std::mem::take` leaves content that does not.
    pub fn first() -> Self {
        Self {
            first: true,
            ..Self::default()
        }
    }

    /// Appends text, applying the `white-space` rules of `style`.
    pub fn push_text(&mut self, text: &str, style: &ComputedStyle) {
        let white_space = style.white_space;
        let mut start = self.text.len();
        let transformed = self.transform(text, style.text_transform);
        let mut chars = transformed.chars().peekable();
        while let Some(ch) = chars.next() {
            let collapsible = ch.is_ascii_whitespace() && white_space.collapses_spaces();
            if !collapsible && !self.pending.is_empty() && ch != '\r' {
                // Elements start before their first content.
                self.push_span(start, style);
                self.start_pending();
                start = self.text.len();
            }
            match ch {
                '\r' if chars.peek() == Some(&'\n') => {}
                '\n' | '\r' if white_space.preserves_newlines() => self.push_newline(),
                '\t' if !white_space.collapses_spaces() => {
                    let column = self
                        .text
                        .rsplit('\n')
                        .next()
                        .map_or(0, |line| line.chars().count());
                    let spaces = TAB_SIZE - column % TAB_SIZE;
                    self.text.extend(std::iter::repeat_n(' ', spaces));
                }
                _ if collapsible => {
                    if !matches!(self.last_char(), None | Some(' ' | '\n')) {
                        self.text.push(' ');
                    }
                }
                _ => self.text.push(ch),
            }
        }
        self.push_span(start, style);
    }

    /// Gives the text from `start` to the end the style `style`, inside
    /// the open elements.
    fn push_span(&mut self, start: usize, style: &ComputedStyle) {
        let path = Rc::clone(&self.path);
        self.push_span_in(start, style, path);
    }

    fn push_span_in(&mut self, start: usize, style: &ComputedStyle, path: Rc<[usize]>) {
        if self.text.len() > start {
            self.spans.push(Span {
                range: start..self.text.len(),
                style: style.clone(),
                path,
            });
        }
    }

    /// The last character of the text, not counting spacers.
    fn last_char(&self) -> Option<char> {
        let mut end = self.text.len();
        let mut spacers = self.spacers.iter().rev().peekable();
        loop {
            let ch = self.text[..end].chars().next_back()?;
            let start = end - ch.len_utf8();
            if spacers.peek() == Some(&&start) {
                spacers.next();
                end = start;
            } else {
                return Some(ch);
            }
        }
    }

    /// Enters the inline element `node`; its content follows, and
    /// [`Self::leave`] ends it. If it paints a box or takes horizontal
    /// space, it is recorded for that; percentages are of
    /// `containing_width`.
    pub fn enter(
        &mut self,
        node: ElementKey,
        style: &ComputedStyle,
        containing_width: f32,
        link: Option<LinkTarget>,
        paint: Option<BoxPaint>,
    ) {
        self.open.push((node, self.nesting.len()));
        self.nesting.push(style.clone());
        self.path = self.open.iter().map(|&(_, index)| index).collect();

        let margin = style
            .margin
            .map(|m| m.resolve(containing_width).unwrap_or(0.0).max(0.0));
        let padding = style
            .padding
            .map(|p| p.resolve(containing_width).unwrap_or(0.0).max(0.0));
        let border = style.border_widths();
        let element = InlineElement {
            node,
            style: style.clone(),
            path: Rc::clone(&self.path),
            range: OPEN..OPEN,
            paint,
            link,
            margin,
            border,
            padding,
        };
        if element.paint.is_none()
            && element.link.is_none()
            && element.start_width() <= 0.0
            && element.end_width() <= 0.0
        {
            return;
        }
        self.pending.push(self.elements.len());
        self.elements.push(element);
    }

    /// Records that the inline element starting here can be linked to as
    /// `name`.
    pub fn anchor(&mut self, name: Arc<str>) {
        self.anchors.push((self.text.len(), name));
    }

    /// Leaves the inline element `node`.
    pub fn leave(&mut self, node: ElementKey) {
        self.close_element(node);
        if self.open.last().is_some_and(|&(open, _)| open == node) {
            self.open.pop();
            self.path = self.open.iter().map(|&(_, index)| index).collect();
        }
    }

    /// The content after a block inside the open inline elements: it
    /// stays inside them for its style. Their boxes are not painted
    /// around it.
    pub fn continuation(&self) -> Inline {
        let mut next = Inline::default();
        for &(node, index) in &self.open {
            next.open.push((node, next.nesting.len()));
            next.nesting.push(self.nesting[index].clone());
        }
        next.path = next.open.iter().map(|&(_, index)| index).collect();
        next
    }

    /// Ends the painted inline element `node`, if it was started in this
    /// paragraph: a block inside the element splits the paragraph, and
    /// the part after the block is not painted.
    fn close_element(&mut self, node: ElementKey) {
        let Some(index) = self.elements.iter().rposition(|e| e.node == node) else {
            return;
        };
        if self.pending.contains(&index) {
            // Empty: still painted, and still takes its space.
            self.start_pending();
        }
        let width = self.elements[index].end_width();
        if width > 0.0 {
            // A space at the end stays outside, after the spacer, so that
            // the spacer does not move to the next line without the text.
            let space = self.pop_collapsible_space();
            let style = self.elements[index].style.clone();
            let path = Rc::clone(&self.elements[index].path);
            self.push_object(InlineObject::spacer(width), &style, path);
            self.elements[index].range.end = self.text.len();
            if let Some(Span { style, path, .. }) = space {
                let start = self.text.len();
                self.text.push(' ');
                self.push_span_in(start, &style, path);
            }
        } else {
            self.elements[index].range.end = self.text.len();
        }
    }

    /// Starts the pending elements at the end of the text, pushing their
    /// start spacers.
    fn start_pending(&mut self) {
        for index in std::mem::take(&mut self.pending) {
            self.elements[index].range.start = self.text.len();
            let width = self.elements[index].start_width();
            if width > 0.0 {
                let style = self.elements[index].style.clone();
                let path = Rc::clone(&self.elements[index].path);
                self.push_object(InlineObject::spacer(width), &style, path);
            }
        }
    }

    /// Removes a collapsible space at the end of the text, returning its
    /// style and elements.
    fn pop_collapsible_space(&mut self) -> Option<Span> {
        let last = self.spans.last()?;
        if !self.text.ends_with(' ')
            || last.range.end != self.text.len()
            || !last.style.white_space.collapses_spaces()
        {
            return None;
        }
        let space = Span {
            range: 0..0,
            style: last.style.clone(),
            path: Rc::clone(&last.path),
        };
        self.text.pop();
        let end = self.text.len();
        self.spans.retain_mut(|span| {
            span.range.end = span.range.end.min(end);
            span.range.start < span.range.end
        });
        for element in &mut self.elements {
            if element.range.end != OPEN {
                element.range.start = element.range.start.min(end);
                element.range.end = element.range.end.min(end);
            }
        }
        Some(space)
    }

    /// Applies `text-transform` to `text`, which follows the text gathered
    /// so far: a word that `capitalize` capitalizes may have started in an
    /// earlier span.
    fn transform(&self, text: &str, transform: TextTransform) -> String {
        match transform {
            TextTransform::None => text.to_owned(),
            TextTransform::Uppercase => text.to_uppercase(),
            TextTransform::Lowercase => text.to_lowercase(),
            TextTransform::Capitalize => {
                let mut in_word = self.last_char().is_some_and(is_word_char);
                let mut out = String::with_capacity(text.len());
                for ch in text.chars() {
                    if !in_word && ch.is_alphabetic() {
                        out.extend(ch.to_uppercase());
                    } else {
                        out.push(ch);
                    }
                    in_word = is_word_char(ch);
                }
                out
            }
        }
    }

    /// Appends a forced line break (`<br>`).
    pub fn push_break(&mut self, style: &ComputedStyle) {
        self.start_pending();
        self.push_newline();
        self.push_span(self.text.len() - 1, style);
    }

    /// Appends an atomic inline box. Spaces around it are kept.
    pub fn push_box(&mut self, object: InlineObject, style: &ComputedStyle) {
        self.start_pending();
        let path = Rc::clone(&self.path);
        self.push_object(object, style, path);
    }

    fn push_object(&mut self, mut object: InlineObject, style: &ComputedStyle, path: Rc<[usize]>) {
        object.index = self.text.len();
        if object.spacer {
            self.spacers.push(object.index);
        }
        self.text.push(OBJECT_REPLACEMENT);
        self.push_span_in(object.index, style, path);
        self.boxes.push(object);
    }

    fn push_newline(&mut self) {
        // A collapsible space before a line break is removed.
        if self.text.ends_with(' ')
            && self
                .spans
                .last()
                .is_none_or(|span| span.style.white_space.collapses_spaces())
        {
            self.text.pop();
            if let Some(span) = self.spans.last_mut() {
                span.range.end = span.range.end.min(self.text.len());
            }
        }
        self.text.push('\n');
    }

    fn is_blank(&self) -> bool {
        self.text.trim().is_empty()
    }
}

/// Whether `ch` continues a word for `text-transform: capitalize`.
fn is_word_char(ch: char) -> bool {
    // Marks continue the word they attach to (Devanagari vowel signs);
    // apostrophes keep "don't" one word.
    ch.is_alphanumeric() || matches!(ch, '\'' | '’') || is_mark(ch)
}

/// A rough test for combining marks, enough to keep them inside words.
fn is_mark(ch: char) -> bool {
    matches!(ch as u32, 0x0300..=0x036F | 0x0900..=0x0903 | 0x093A..=0x094F | 0x0951..=0x0957 | 0x0962..=0x0963)
}

/// The parley style for text with the given computed style, aligned
/// with its parent as `align` says.
fn text_style<'a>(
    generics: &'a GenericFonts,
    web: &'a WebFonts<'_>,
    style: &'a ComputedStyle,
    align: ParleyVerticalAlign,
) -> TextStyle<'a, 'static, Ink> {
    let line_height = match style.line_height {
        LineHeight::Normal => ParleyLineHeight::MetricsRelative(1.0),
        LineHeight::Number(factor) => ParleyLineHeight::FontSizeRelative(factor),
        LineHeight::Percent(fraction) => ParleyLineHeight::FontSizeRelative(fraction),
        LineHeight::Length(length) => {
            ParleyLineHeight::Absolute(length.to_pt(style.font_size, style.root_font_size))
        }
    };
    let text_wrap_mode = if style.white_space.wraps() {
        TextWrapMode::Wrap
    } else {
        TextWrapMode::NoWrap
    };
    TextStyle {
        font_family: generics.stack(web, &style.font_family),
        font_size: style.font_size,
        font_weight: FontWeight::new(style.font_weight),
        font_style: if style.italic {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        },
        locale: style.lang.as_deref().and_then(|l| Language::parse(l).ok()),
        brush: Ink {
            color: style.color,
            decorations: style.text_decorations(),
        },
        line_height,
        vertical_align: align,
        text_wrap_mode,
        letter_spacing: style.letter_spacing,
        word_spacing: style.word_spacing,
        // The text arrives with white space already processed.
        white_space_collapse: WhiteSpaceCollapse::Preserve,
        ..TextStyle::default()
    }
}

impl Context<'_> {
    /// Shapes `text` into an unbroken parley layout. `style` is the block's
    /// style; `spans` override it for stretches of the text, inside the
    /// inline elements whose styles are `nesting`.
    fn shape(
        &mut self,
        text: &str,
        style: &ComputedStyle,
        spans: &[Span],
        nesting: &[ComputedStyle],
        boxes: &[InlineObject],
    ) -> Layout<Ink> {
        let styles = std::iter::once(style)
            .chain(spans.iter().map(|span| &span.style))
            .chain(nesting);
        for style in styles {
            for family in style.font_family.iter() {
                if let FontFamilyName::Named(name) = family {
                    self.web_fonts.load(name, self.fonts, self.fetcher);
                }
            }
        }

        let (generics, web) = (self.generics, &self.web_fonts);
        let root = text_style(generics, web, style, ParleyVerticalAlign::BASELINE);
        let mut builder = self.texts.tree_builder(self.fonts, 1.0, false, &root);
        if boxes.iter().any(|object| object.spacer) {
            builder.set_line_break_override(Some(&keep_objects_with_text));
        }
        let mut pusher = TextPusher {
            text,
            boxes,
            next_box: 0,
            builder,
        };
        // Only elements that move their content off the baseline need a
        // span of their own: they align it, against their parent span.
        // Everything else is flat, like a browser's shaping, which runs
        // across element boundaries when the style does not change.
        let aligned = |element: &usize| nesting[*element].vertical_align != VerticalAlign::Baseline;
        // The aligned elements whose spans are open in the builder, and
        // the text style open inside them, if any.
        let mut stack: Vec<usize> = Vec::new();
        let mut leaf: Option<TextStyle<'_, 'static, Ink>> = None;
        let mut cursor = 0;
        for span in spans {
            let range = span.range.start.max(cursor)..span.range.end;
            if range.is_empty() {
                continue;
            }
            let path: Vec<usize> = span.path.iter().copied().filter(aligned).collect();
            let style = text_style(generics, web, &span.style, ParleyVerticalAlign::BASELINE);
            let contiguous = cursor == range.start;
            if contiguous && stack == path && leaf.as_ref() == Some(&style) {
                pusher.push(range.clone());
                cursor = range.end;
                continue;
            }
            if leaf.take().is_some() {
                pusher.builder.pop_style_span();
            }
            if !contiguous {
                for _ in stack.drain(..) {
                    pusher.builder.pop_style_span();
                }
                pusher.push(cursor..range.start);
            }
            let common = stack.iter().zip(&path).take_while(|(a, b)| a == b).count();
            for _ in stack.drain(common..) {
                pusher.builder.pop_style_span();
            }
            for &element in &path[common..] {
                let element_style = &nesting[element];
                let align = vertical_align(element_style.vertical_align);
                let parley_style = text_style(generics, web, element_style, align);
                pusher.builder.push_style_span(parley_style);
                stack.push(element);
            }
            // The text's own style, in line with the innermost element.
            pusher.builder.push_style_span(style.clone());
            leaf = Some(style);
            pusher.push(range.clone());
            cursor = range.end;
        }
        if leaf.is_some() {
            pusher.builder.pop_style_span();
        }
        for _ in stack.drain(..) {
            pusher.builder.pop_style_span();
        }
        pusher.push(cursor..text.len());
        let (layout, built) = pusher.builder.build();
        debug_assert_eq!(built, text, "parley kept the text as given");
        layout
    }
}

/// Feeds a paragraph's text to parley's tree builder, turning the U+FFFC
/// characters that stand for boxes into the boxes.
struct TextPusher<'t, 'b> {
    text: &'t str,
    boxes: &'t [InlineObject],
    next_box: usize,
    builder: TreeBuilder<'b, Ink>,
}

impl TextPusher<'_, '_> {
    fn push(&mut self, range: Range<usize>) {
        let mut at = range.start;
        while let Some(object) = self.boxes.get(self.next_box)
            && object.index < range.end
        {
            if object.index >= at {
                self.builder.push_text(&self.text[at..object.index]);
                self.builder.push_inline_box(InlineBox {
                    id: self.next_box as u64,
                    kind: InlineBoxKind::InFlow,
                    // Set by the builder, to where the text is now.
                    index: 0,
                    width: object.width,
                    height: object.height,
                    baseline: object.baseline,
                    vertical_align: vertical_align(object.vertical_align),
                });
                at = object.index + OBJECT_REPLACEMENT.len_utf8();
            }
            self.next_box += 1;
        }
        self.builder.push_text(&self.text[at..range.end]);
    }
}

impl Flow<'_, '_> {
    /// Shapes and line-breaks a paragraph, adding one slice per line.
    pub(super) fn paragraph(&mut self, inline: Inline, style: &ComputedStyle, x: f32, width: f32) {
        if inline.is_blank() {
            return;
        }
        let text: Arc<str> = inline.text.into();
        let boxes = inline.boxes;
        let elements = inline.elements;
        let anchors = inline.anchors;
        let mut layout = self
            .cx
            .shape(&text, style, &inline.spans, &inline.nesting, &boxes);
        if inline.first {
            let indent = style.text_indent;
            let amount = indent.length.resolve(width).unwrap_or(0.0);
            if amount != 0.0 {
                let options = IndentOptions {
                    each_line: indent.each_line,
                    hanging: indent.hanging,
                };
                layout.set_text_indent(amount, options);
            }
        }

        if self.is_measuring() {
            let max = layout.calculate_content_widths().max;
            // Boxes that can shrink count at their narrowest for the
            // minimum.
            for (index, object) in boxes.iter().enumerate() {
                if object.min_width < object.width {
                    layout.edit_inline_box(index, |b| b.width = object.min_width);
                }
            }
            let min = layout.calculate_content_widths().min;
            self.record_extent(min, max);
            return;
        }

        self.settle_margin();
        let alignment = match style.text_align {
            TextAlign::Left => Alignment::Left,
            TextAlign::Right => Alignment::Right,
            TextAlign::Center => Alignment::Center,
            TextAlign::Justify => Alignment::Justify,
        };
        layout.set_alignment(alignment, AlignmentOptions::default());
        // Content measured to fit `width` exactly (a table cell sized to
        // its max-content width) can exceed it by a rounding error, as
        // parley sums advances in another order when breaking.
        layout.break_all_lines(Some(width + FIT_TOLERANCE));

        let marker = self.marker.take();
        let count = layout.len();
        // Lines that must share a page with the line before them: the
        // first `orphans` lines stay together, and so do the last `widows`.
        let (orphans, widows) = (style.orphans as usize, style.widows as usize);
        let kept = |index: usize| index > 0 && (index < orphans || index + widows > count);
        for (index, line) in layout.lines().enumerate() {
            let metrics = line.metrics();
            let top = metrics.block_min_coord;
            let mut items = Vec::new();
            let mut cursor = ClusterCursor::default();
            let mut pieces = LinePieces::default();
            let count = line.items().count();
            for (position, item) in line.items().enumerate() {
                match item {
                    PositionedLayoutItem::GlyphRun(run) => {
                        let origin = Point {
                            x: x + run.offset(),
                            y: run.baseline() - top,
                        };
                        let text_run = text_run(&run, origin, &text, &mut cursor);
                        if !elements.is_empty() {
                            pieces.add_run(&run, &text_run, origin.y);
                        }
                        let decorations = run.style().brush.decorations;
                        if decorations.is_empty() {
                            items.push(Item::Text(text_run));
                            continue;
                        }
                        // Lines are not drawn under the spaces that end a line.
                        let trailing = if position + 1 == count && !run.run().is_rtl() {
                            trailing_space(&text_run)
                        } else {
                            0.0
                        };
                        let lines = DecorationLines::new(&run, decorations, top, trailing);
                        lines.fills(x, UNDER_TEXT, &mut items);
                        items.push(Item::Text(text_run));
                        lines.fills(x, OVER_TEXT, &mut items);
                    }
                    PositionedLayoutItem::InlineBox(placed) => {
                        let object = &boxes[placed.id as usize];
                        pieces.pieces.push(Piece {
                            text: object.index..object.index + OBJECT_REPLACEMENT.len_utf8(),
                            x: x + placed.x,
                            width: placed.width,
                            space: false,
                        });
                        items.extend(
                            object
                                .items
                                .iter()
                                .cloned()
                                .map(|item| item.translate(x + placed.x, placed.y - top)),
                        );
                    }
                }
            }
            if !elements.is_empty() {
                let baseline = metrics.baseline - top;
                let height = metrics.block_max_coord - top;
                let mut boxes = Vec::new();
                for element in &elements {
                    pieces.paint(element, &text, baseline, height, &mut boxes);
                }
                items.splice(0..0, boxes);
            }
            let line_text = line.text_range();
            for (position, name) in &anchors {
                let at_start = index == 0 && *position <= line_text.start;
                if at_start || line_text.contains(position) {
                    items.push(mark(x, MarkKind::Anchor(Arc::clone(name))));
                }
            }
            if let Some(target) = &self.link {
                // Inside a block-level link: the whole line is clickable.
                items.push(Item::Link(Link {
                    rect: Rect::new(x, 0.0, width, metrics.block_max_coord - top),
                    target: target.clone(),
                }));
            }
            if index == 0
                && let Some(marker) = &marker
            {
                items.extend(self.marker_items(marker, style, x, metrics.baseline - top));
            }
            self.push_slice(
                self.y + f64::from(top),
                metrics.block_max_coord - top,
                items,
            );
            self.baseline = Some(self.y + f64::from(metrics.baseline));
            if kept(index) {
                self.keep_last_slice();
            }
        }
        self.y += f64::from(layout.height());
    }

    /// The glyphs of a list marker, ending just left of `x` on the
    /// baseline at `baseline`.
    fn marker_items(
        &mut self,
        marker: &str,
        style: &ComputedStyle,
        x: f32,
        baseline: f32,
    ) -> Vec<Item> {
        // Markers are not underlined with the item's text.
        let style = ComputedStyle {
            white_space: WhiteSpace::Pre,
            text_decoration_line: TextDecorationLine::NONE,
            inherited_decorations: TextDecorations::NONE,
            ..style.clone()
        };
        let text: Arc<str> = marker.into();
        let mut layout = self.cx.shape(&text, &style, &[], &[], &[]);
        layout.break_all_lines(None);
        let gap = style.font_size * 0.4;
        let start = x - layout.width() - gap;

        let mut items = Vec::new();
        if let Some(line) = layout.lines().next() {
            let mut cursor = ClusterCursor::default();
            for item in line.items() {
                if let PositionedLayoutItem::GlyphRun(run) = item {
                    let origin = Point {
                        x: start + run.offset(),
                        y: baseline,
                    };
                    items.push(Item::Text(text_run(&run, origin, &text, &mut cursor)));
                }
            }
        }
        items
    }
}

/// The decoration lines of one glyph run, relative to its slice.
struct DecorationLines {
    decorations: TextDecorations,
    /// Left edge, relative to the paragraph's left edge, and length.
    start: f32,
    width: f32,
    /// Tops and thicknesses.
    underline: (f32, f32),
    overline: (f32, f32),
    line_through: (f32, f32),
}

impl DecorationLines {
    /// Positions the lines. `top` is the line's top; `trailing` the width
    /// of spaces at the end of the run that are left undecorated.
    ///
    /// Overlines and line-throughs are placed from the ascent of the
    /// span's primary font, as in Blink, so that they keep one height
    /// across fallback fonts: Devanagari fonts put their own strikeout
    /// much higher than Latin ones. Underlines use the run's font, whose
    /// position agrees closely enough between fonts.
    fn new(run: &GlyphRun<'_, Ink>, decorations: TextDecorations, top: f32, trailing: f32) -> Self {
        let metrics = run.run().font_metrics();
        let font_size = run.run().font_size();
        // Fonts without the metrics get a line of a typical thickness.
        let thickness = |size: f32| {
            if size > 0.0 { size } else { font_size / 16.0 }
        };
        let baseline = run.baseline() - top;
        let under = thickness(metrics.underline_size);
        // The offset is to the top of the line, upwards from the baseline.
        let underline_offset = if metrics.underline_offset != 0.0 {
            metrics.underline_offset
        } else {
            -font_size / 10.0
        };
        let ascent = match run.span_metrics().ascent {
            ascent if ascent > 0.0 => ascent,
            _ => metrics.ascent,
        };
        let strike = thickness(metrics.strikethrough_size);
        Self {
            decorations,
            start: run.offset(),
            width: (run.advance() - trailing).max(0.0),
            underline: (baseline - underline_offset, under),
            overline: (baseline - ascent, under),
            line_through: (baseline - ascent / 3.0 - strike / 2.0, strike),
        }
    }

    /// Appends the fills for the lines in `which`, shifted right by `x`.
    fn fills(&self, x: f32, which: TextDecorationLine, out: &mut Vec<Item>) {
        let lines = [
            (which.underline, self.decorations.underline, self.underline),
            (which.overline, self.decorations.overline, self.overline),
            (
                which.line_through,
                self.decorations.line_through,
                self.line_through,
            ),
        ];
        for (wanted, color, (y, thickness)) in lines {
            if let (true, Some(color)) = (wanted, color)
                && self.width > 0.0
            {
                out.push(Item::Fill(Fill {
                    rect: Rect::new(x + self.start, y, self.width, thickness),
                    color,
                }));
            }
        }
    }
}

/// Paint order: underlines and overlines go below the text, line-through
/// above it (CSS Text Decoration 3, section 2.1).
const UNDER_TEXT: TextDecorationLine = TextDecorationLine {
    underline: true,
    overline: true,
    line_through: false,
};
const OVER_TEXT: TextDecorationLine = TextDecorationLine {
    underline: false,
    overline: false,
    line_through: true,
};

/// The advance of the whitespace glyphs at the end of `run`.
fn trailing_space(run: &TextRun) -> f32 {
    run.glyphs
        .iter()
        .rev()
        .take_while(|glyph| {
            let text = &run.text[glyph.text_range.clone()];
            !text.is_empty() && text.chars().all(char::is_whitespace)
        })
        .map(|glyph| glyph.advance)
        .sum()
}

/// No soft wrap opportunity between an inline object and the text right
/// next to it, so that an element's padding stays with its first and last
/// characters. Applied only to paragraphs with spacers: it also keeps
/// images with text they touch, where browsers would break.
fn keep_objects_with_text(cx: LineBreakContext) -> Option<bool> {
    let text = |ch: char| !ch.is_whitespace();
    let joined = (cx.before == OBJECT_REPLACEMENT && text(cx.after))
        || (cx.after == OBJECT_REPLACEMENT && text(cx.before));
    joined.then_some(false)
}

/// A glyph cluster or inline object on a line.
struct Piece {
    /// The text it stands for.
    text: Range<usize>,
    /// Left edge relative to the canvas, and advance.
    x: f32,
    width: f32,
    /// Whitespace, not painted under at the end of a line.
    space: bool,
}

/// What an inline element's box on a line is sized from: the clusters
/// and objects of the line, and the font metrics of its text runs.
#[derive(Default)]
struct LinePieces {
    pieces: Vec<Piece>,
    /// Text range, primary-font ascent and descent, font size and
    /// baseline (relative to the line's top) of each glyph run.
    runs: Vec<(Range<usize>, f32, f32, f32, f32)>,
}

impl LinePieces {
    fn add_run(&mut self, run: &GlyphRun<'_, Ink>, text_run: &TextRun, baseline: f32) {
        let mut x = text_run.origin.x;
        let mut range: Option<Range<usize>> = None;
        for glyph in &text_run.glyphs {
            let cluster = glyph.text_range.clone();
            let chars = &text_run.text[cluster.clone()];
            self.pieces.push(Piece {
                text: cluster.clone(),
                x,
                width: glyph.advance,
                space: !chars.is_empty() && chars.chars().all(char::is_whitespace),
            });
            x += glyph.advance;
            range = Some(match range {
                None => cluster,
                Some(r) => r.start.min(cluster.start)..r.end.max(cluster.end),
            });
        }
        if let Some(range) = range {
            let metrics = run.span_metrics();
            let font_size = run.run().font_size();
            self.runs
                .push((range, metrics.ascent, metrics.descent, font_size, baseline));
        }
    }

    /// Appends the background and borders of `element`'s box on this
    /// line, and its link area, if any of it is here. `baseline` is
    /// relative to the line's top, which is `line_height` tall.
    fn paint(
        &self,
        element: &InlineElement,
        text: &str,
        baseline: f32,
        line_height: f32,
        out: &mut Vec<Item>,
    ) {
        let range = element.range.start..element.range.end.min(text.len());
        let Some(last) = text.get(range.clone()).and_then(|t| t.chars().next_back()) else {
            return;
        };
        let last = range.end - last.len_utf8();
        let inside: Vec<&Piece> = self
            .pieces
            .iter()
            .filter(|piece| range.contains(&piece.text.start))
            .collect();
        if inside.is_empty() {
            return;
        }
        let first = inside.iter().any(|piece| piece.text.start == range.start);
        let last = inside.iter().any(|piece| piece.text.contains(&last));
        // Spaces at the end of the line hang outside the box.
        let content_end = self
            .pieces
            .iter()
            .filter(|piece| !piece.space)
            .map(|piece| piece.x + piece.width)
            .fold(f32::NEG_INFINITY, f32::max);
        let shown = inside
            .iter()
            .filter(|piece| !piece.space || piece.x + piece.width <= content_end);
        let (mut left, mut right) = (f32::INFINITY, f32::NEG_INFINITY);
        for piece in shown {
            left = left.min(piece.x);
            right = right.max(piece.x + piece.width);
        }
        if left > right {
            return;
        }
        if first {
            left += element.margin.left;
        }
        if last {
            right -= element.margin.right;
        }
        if let Some(target) = &element.link {
            // The whole height of the line is clickable, so that images
            // and taller text inside count too.
            out.push(Item::Link(Link {
                rect: Rect::new(left, 0.0, right - left, line_height),
                target: target.clone(),
            }));
        }
        let Some(paint) = element.paint.clone() else {
            return;
        };

        // The content area is the element's own font's ascent and
        // descent around its baseline; taken from its first text run on
        // the line, scaled if that run is a descendant's of another size.
        let font_size = element.style.font_size;
        let own = self
            .runs
            .iter()
            .find(|(text, ..)| text.start < range.end && range.start < text.end);
        let baseline = own.map_or(baseline, |run| run.4);
        let (ascent, descent) = match own.or(self.runs.first()) {
            Some(&(_, ascent, descent, size, _)) if size > 0.0 => {
                (ascent * font_size / size, descent * font_size / size)
            }
            _ => (font_size * 0.8, font_size * 0.2),
        };
        let top = baseline - ascent - element.padding.top - element.border.top;
        let bottom = baseline + descent + element.padding.bottom + element.border.bottom;
        let mut paint = paint;
        if !first {
            paint = paint.without_border(Side::Left);
        }
        if !last {
            paint = paint.without_border(Side::Right);
        }
        paint.fills(Rect::new(left, top, right - left, bottom - top), out);
    }
}

/// parley's equivalent of a `vertical-align` value.
fn vertical_align(value: VerticalAlign) -> ParleyVerticalAlign {
    match value {
        VerticalAlign::Baseline => ParleyVerticalAlign::BASELINE,
        VerticalAlign::Sub => ParleyVerticalAlign::SUB,
        VerticalAlign::Super => ParleyVerticalAlign::SUPER,
        VerticalAlign::TextTop => ParleyVerticalAlign::TEXT_TOP,
        VerticalAlign::TextBottom => ParleyVerticalAlign::TEXT_BOTTOM,
        VerticalAlign::Middle => ParleyVerticalAlign::MIDDLE,
        VerticalAlign::Top => ParleyVerticalAlign::TOP,
        VerticalAlign::Bottom => ParleyVerticalAlign::BOTTOM,
    }
}

/// The marker text for the list item `item`, e.g. `•` or `3.`.
pub(super) fn list_marker(doc: &Document, item: NodeId, style: &ComputedStyle) -> Option<String> {
    let ordinal = || -> i64 {
        let Some(parent) = doc.node(item).parent else {
            return 1;
        };
        let start = doc
            .element(parent)
            .and_then(|list| list.attr("start"))
            .and_then(|start| start.trim().parse().ok())
            .unwrap_or(1);
        let position = doc
            .children(parent)
            .filter(|&c| doc.element(c).is_some_and(|e| e.name == "li"))
            .position(|c| c == item)
            .unwrap_or(0);
        start + position as i64
    };
    Some(match style.list_style_type {
        ListStyleType::None => return None,
        bullet @ (ListStyleType::Disc | ListStyleType::Circle | ListStyleType::Square) => {
            counter_text(0, bullet)
        }
        numbered => format!("{}.", counter_text(ordinal(), numbered)),
    })
}

/// Tracks how many clusters of a shaped run earlier glyph runs on the same
/// line have used.
///
/// parley splits a shaped run into glyph runs where the style changes
/// (e.g. the color), but only exposes clusters, which know their text
/// range, per shaped run. Glyph runs of one shaped run come in order, so
/// each takes the next clusters.
#[derive(Default)]
struct ClusterCursor {
    run: Option<usize>,
    used: usize,
}

/// Converts a parley glyph run into a [`TextRun`] starting at `origin`.
fn text_run(
    run: &GlyphRun<'_, Ink>,
    origin: Point,
    text: &Arc<str>,
    cursor: &mut ClusterCursor,
) -> TextRun {
    let shaped = run.run();
    if cursor.run != Some(shaped.index()) {
        *cursor = ClusterCursor {
            run: Some(shaped.index()),
            used: 0,
        };
    }

    let expected: Vec<_> = run.glyphs().collect();
    let mut glyphs = Vec::with_capacity(expected.len());
    // The text range of each cluster with glyphs, and its glyphs' indices
    // in `glyphs`.
    let mut clusters: Vec<(Range<usize>, Range<usize>)> = Vec::new();
    // Ligature continuations: the characters after the first of a
    // ligature (the i of an fi ligature), which have no glyphs of their
    // own; the ligature start has them all.
    let mut continuations: Vec<Range<usize>> = Vec::new();
    let mut used = 0;
    for cluster in shaped.visual_clusters().skip(cursor.used) {
        if cluster.is_ligature_continuation() {
            used += 1;
            continuations.push(cluster.text_range());
            continue;
        }
        if glyphs.len() >= expected.len() {
            break;
        }
        used += 1;
        let first = glyphs.len();
        glyphs.extend(cluster.glyphs().map(|g| Glyph {
            id: g.id,
            advance: g.advance,
            x_offset: g.x,
            y_offset: g.y,
            text_range: 0..0,
        }));
        clusters.push((cluster.text_range(), first..glyphs.len()));
    }
    cursor.used += used;
    // A ligature's glyphs stand for all its characters: extend each start
    // over the continuations that follow it in the text. In text order, so
    // that a three-letter ligature (ffi) grows one character at a time
    // whatever the direction.
    continuations.sort_by_key(|range| range.start);
    for continuation in continuations {
        if let Some((range, _)) = clusters
            .iter_mut()
            .find(|(range, _)| range.end == continuation.start)
        {
            range.end = continuation.end;
        }
    }
    for (range, indices) in clusters {
        for glyph in &mut glyphs[indices] {
            glyph.text_range = range.clone();
        }
    }

    let matches =
        glyphs.len() == expected.len() && glyphs.iter().zip(&expected).all(|(a, b)| a.id == b.id);
    if !matches {
        // Fall back to attributing the whole run's text to every glyph,
        // which draws correctly but makes text selection coarse.
        let range = shaped.text_range();
        glyphs = expected
            .into_iter()
            .map(|g| Glyph {
                id: g.id,
                advance: g.advance,
                x_offset: g.x,
                y_offset: g.y,
                text_range: range.clone(),
            })
            .collect();
    }

    split_plain_clusters(&mut glyphs, text, shaped.font());
    TextRun {
        font: shaped.font().clone(),
        font_size: shaped.font_size(),
        color: run.style().brush.color,
        origin,
        glyphs,
        text: Arc::clone(text),
    }
}

/// Gives each glyph of a cluster its own character when the cluster is
/// just its characters' default glyphs, in order: a consonant and the vowel
/// sign under it (कृ), a letter and a combining accent.
///
/// parley maps every glyph of a cluster to the whole cluster, which a PDF
/// can only express with `/ActualText`. Text extraction (poppler, at least)
/// then takes the cluster to end where its last glyph ends, and a final
/// mark shifted left reads as a gap: "कृष्ण" was extracted as "कृ ष्ण".
/// Mapped one character per glyph, no `/ActualText` is needed.
///
/// Clusters with substituted glyphs (conjuncts such as ष्ण) or reordered
/// ones (ि, drawn before its consonant) keep the whole-cluster mapping,
/// the only correct one for them.
fn split_plain_clusters(glyphs: &mut [Glyph], text: &str, font: &FontData) {
    // Parsed on first need: most clusters are a single glyph.
    let mut font_ref = None;
    let mut start = 0;
    while start < glyphs.len() {
        let range = glyphs[start].text_range.clone();
        let len = glyphs[start..]
            .iter()
            .take_while(|g| g.text_range == range)
            .count();
        let cluster = &mut glyphs[start..start + len];
        start += len;

        let chars = &text[range.clone()];
        if len < 2 || chars.chars().count() != len {
            continue;
        }
        let font_ref = font_ref.get_or_insert_with(|| {
            FontRef::from_index(font.data.data(), font.index)
                .ok()
                .and_then(|font| font.cmap().ok())
        });
        let Some(cmap) = font_ref else {
            return;
        };
        let plain = chars.chars().zip(cluster.iter()).all(|(ch, glyph)| {
            cmap.map_codepoint(ch)
                .is_some_and(|id| id.to_u32() == glyph.id)
        });
        if plain {
            for ((offset, ch), glyph) in chars.char_indices().zip(cluster.iter_mut()) {
                let char_start = range.start + offset;
                glyph.text_range = char_start..char_start + ch.len_utf8();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collapse(parts: &[(&str, WhiteSpace)]) -> String {
        let mut inline = Inline::default();
        for (text, white_space) in parts {
            let style = ComputedStyle {
                white_space: *white_space,
                ..ComputedStyle::initial()
            };
            inline.push_text(text, &style);
        }
        inline.text
    }

    #[test]
    fn normal_collapses_white_space_across_spans() {
        use WhiteSpace::Normal;
        assert_eq!(collapse(&[("  Hello \n\t world ", Normal)]), "Hello world ");
        assert_eq!(collapse(&[("a ", Normal), (" b", Normal)]), "a b");
    }

    #[test]
    fn pre_keeps_spaces_and_newlines_and_expands_tabs() {
        assert_eq!(
            collapse(&[("a  b\r\nc\td", WhiteSpace::Pre)]),
            "a  b\nc       d"
        );
    }

    #[test]
    fn pre_line_keeps_only_newlines() {
        assert_eq!(collapse(&[("a   b \n  c", WhiteSpace::PreLine)]), "a b\nc");
    }

    #[test]
    fn breaks_trim_the_space_before_them() {
        let style = ComputedStyle::initial();
        let mut inline = Inline::default();
        inline.push_text("one ", &style);
        inline.push_break(&style);
        inline.push_text(" two", &style);
        assert_eq!(inline.text, "one\ntwo");
        for span in &inline.spans {
            assert!(span.range.end <= inline.text.len() && span.range.start <= span.range.end);
        }
    }
}
