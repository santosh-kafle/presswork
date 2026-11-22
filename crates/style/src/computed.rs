//! Computed styles: the result of the cascade for one element.

use std::sync::Arc;

use crate::properties::{
    BORDER_WIDTH_MEDIUM, BorderCollapse, BorderStyle, BoxSizing, Break, BreakInside, Content,
    CounterChanges, CssWide, Display, FontFamilyName, FontSize, FontWeight, GenericFamily,
    LineHeight, ListStyleType, LonghandId, PropertyDeclaration, TableLayout, TextAlign,
    TextDecorationLine, TextTransform, VerticalAlign, WhiteSpace,
};
use crate::values::{Color, ColorValue, Dimension, Length, Side, Sides};

/// The browser default font size: 16px.
pub const DEFAULT_FONT_SIZE: f32 = 12.0;

/// One side of a border.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BorderSide {
    /// Specified width in points. See [`BorderSide::used_width`].
    pub width: f32,
    pub style: BorderStyle,
    pub color: ColorValue,
}

impl BorderSide {
    /// The width the border takes up: zero unless its style is visible.
    pub fn used_width(&self) -> f32 {
        if self.style.is_visible() {
            self.width
        } else {
            0.0
        }
    }
}

impl Default for BorderSide {
    fn default() -> Self {
        Self {
            width: BORDER_WIDTH_MEDIUM.to_pt(0.0, 0.0),
            style: BorderStyle::None,
            color: ColorValue::CurrentColor,
        }
    }
}

/// The lines drawn on text, each with its color.
///
/// A box's `text-decoration` applies to all the text inside it, even text
/// in descendants that set `text-decoration: none`, so each style carries
/// the decorations of its ancestors (see
/// [`ComputedStyle::text_decorations`]). Each line is drawn once: where
/// nested boxes both underline, the innermost color is used.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TextDecorations {
    pub underline: Option<Color>,
    pub overline: Option<Color>,
    pub line_through: Option<Color>,
}

impl TextDecorations {
    pub const NONE: Self = Self {
        underline: None,
        overline: None,
        line_through: None,
    };

    pub fn is_empty(&self) -> bool {
        *self == Self::NONE
    }

    /// Adds the lines of `line` in `color`.
    fn with(mut self, line: TextDecorationLine, color: Color) -> Self {
        for (set, slot) in [
            (line.underline, &mut self.underline),
            (line.overline, &mut self.overline),
            (line.line_through, &mut self.line_through),
        ] {
            if set {
                *slot = Some(color);
            }
        }
        self
    }
}

/// A computed `text-indent`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextIndent {
    /// A percentage is of the containing block's width.
    pub length: Dimension,
    pub hanging: bool,
    pub each_line: bool,
}

/// The style of one element, with every font-relative length resolved to
/// points. Percentages and `auto` remain for layout to resolve.
#[derive(Debug, Clone, PartialEq)]
pub struct ComputedStyle {
    pub display: Display,
    pub color: Color,
    pub background_color: Color,
    pub font_family: Arc<[FontFamilyName]>,
    /// Font size in points.
    pub font_size: f32,
    /// Font weight, 1 to 1000.
    pub font_weight: f32,
    pub italic: bool,
    /// `Length` is in points here.
    pub line_height: LineHeight,
    pub text_align: TextAlign,
    pub white_space: WhiteSpace,
    pub margin: Sides<Dimension>,
    pub padding: Sides<Dimension>,
    pub border: Sides<BorderSide>,
    pub width: Dimension,
    /// Images and table cells and rows use it, and blocks as a minimum:
    /// content taller than the box makes it grow instead of overflowing.
    /// A percentage behaves as `auto`, as the page has no definite height
    /// to resolve it against.
    pub height: Dimension,
    /// `Auto` means `none`.
    pub max_width: Dimension,
    /// `Auto` means `none`. Blocks are never cut shorter than their
    /// content: see `height`.
    pub max_height: Dimension,
    /// `Auto` means zero.
    pub min_height: Dimension,
    pub box_sizing: BoxSizing,
    pub vertical_align: VerticalAlign,
    pub border_collapse: BorderCollapse,
    /// Space between table cells, in points.
    pub border_spacing: f32,
    pub list_style_type: ListStyleType,
    pub break_before: Break,
    pub break_after: Break,
    pub break_inside: BreakInside,
    pub table_layout: TableLayout,
    /// The fewest lines of a paragraph to leave at the bottom of a page.
    pub orphans: u32,
    /// The fewest lines of a paragraph to carry to the top of a page.
    pub widows: u32,
    pub text_decoration_line: TextDecorationLine,
    pub text_decoration_color: ColorValue,
    pub text_indent: TextIndent,
    pub text_transform: TextTransform,
    /// Extra space after each character, in points.
    pub letter_spacing: f32,
    /// Extra space after each word, in points.
    pub word_spacing: f32,
    /// Horizontal and vertical radius of each corner, indexed like
    /// [`Corner::ALL`]. Percentages are of the border box's width and
    /// height.
    pub border_radius: [(Dimension, Dimension); 4],
    /// `content`, for `::before` and `::after`.
    pub content: Content,
    pub counter_reset: CounterChanges,
    pub counter_increment: CounterChanges,
    pub counter_set: CounterChanges,
    /// The text decorations of the ancestors, which apply to this
    /// element's text as well as its own. Not a CSS property.
    pub inherited_decorations: TextDecorations,
    /// Content language from the `lang` attribute, e.g. `"ne"`. Inherited.
    /// Not a CSS property; used for font fallback and shaping.
    pub lang: Option<Arc<str>>,
    /// The root element's font size, for `rem`. Inherited.
    pub root_font_size: f32,
}

impl ComputedStyle {
    /// Every property at its initial value.
    pub fn initial() -> Self {
        Self {
            display: Display::Inline,
            color: Color::BLACK,
            background_color: Color::TRANSPARENT,
            font_family: Arc::new([FontFamilyName::Generic(GenericFamily::Serif)]),
            font_size: DEFAULT_FONT_SIZE,
            font_weight: 400.0,
            italic: false,
            line_height: LineHeight::Normal,
            text_align: TextAlign::Left,
            white_space: WhiteSpace::Normal,
            margin: Sides::uniform(Dimension::ZERO),
            padding: Sides::uniform(Dimension::ZERO),
            border: Sides::uniform(BorderSide::default()),
            width: Dimension::Auto,
            height: Dimension::Auto,
            max_width: Dimension::Auto,
            max_height: Dimension::Auto,
            box_sizing: BoxSizing::ContentBox,
            vertical_align: VerticalAlign::Baseline,
            border_collapse: BorderCollapse::Separate,
            border_spacing: 0.0,
            list_style_type: ListStyleType::Disc,
            break_before: Break::Auto,
            break_after: Break::Auto,
            break_inside: BreakInside::Auto,
            table_layout: TableLayout::Auto,
            orphans: 2,
            widows: 2,
            min_height: Dimension::Auto,
            text_decoration_line: TextDecorationLine::NONE,
            text_decoration_color: ColorValue::CurrentColor,
            text_indent: TextIndent {
                length: Dimension::ZERO,
                hanging: false,
                each_line: false,
            },
            text_transform: TextTransform::None,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            border_radius: [(Dimension::ZERO, Dimension::ZERO); 4],
            content: Content::Normal,
            counter_reset: Arc::from([]),
            counter_increment: Arc::from([]),
            counter_set: Arc::from([]),
            inherited_decorations: TextDecorations::NONE,
            lang: None,
            root_font_size: DEFAULT_FONT_SIZE,
        }
    }

    /// The starting style of a child of `parent`: inherited properties come
    /// from the parent, the rest are initial.
    pub fn inheriting(parent: &ComputedStyle) -> Self {
        let mut style = Self::initial();
        for id in LonghandId::all().filter(|id| id.is_inherited()) {
            style.copy_longhand(id, parent);
        }
        style.inherited_decorations = parent.text_decorations();
        style.lang = parent.lang.clone();
        style.root_font_size = parent.root_font_size;
        style
    }

    /// Copies the computed value of one property from `other`.
    pub fn copy_longhand(&mut self, id: LonghandId, other: &ComputedStyle) {
        use LonghandId as L;
        match id {
            L::Display => self.display = other.display,
            L::Content => self.content = other.content.clone(),
            L::BorderRadius(corner) => {
                self.border_radius[corner as usize] = other.border_radius[corner as usize];
            }
            L::CounterReset => self.counter_reset = Arc::clone(&other.counter_reset),
            L::CounterIncrement => self.counter_increment = Arc::clone(&other.counter_increment),
            L::CounterSet => self.counter_set = Arc::clone(&other.counter_set),
            L::Color => self.color = other.color,
            L::BackgroundColor => self.background_color = other.background_color,
            L::FontFamily => self.font_family = Arc::clone(&other.font_family),
            L::FontSize => self.font_size = other.font_size,
            L::FontWeight => self.font_weight = other.font_weight,
            L::FontStyle => self.italic = other.italic,
            L::LineHeight => self.line_height = other.line_height,
            L::TextAlign => self.text_align = other.text_align,
            L::WhiteSpace => self.white_space = other.white_space,
            L::Margin(side) => self.margin.set(side, other.margin.get(side)),
            L::Padding(side) => self.padding.set(side, other.padding.get(side)),
            L::BorderWidth(side) => self.border_mut(side).width = other.border.get(side).width,
            L::BorderStyle(side) => self.border_mut(side).style = other.border.get(side).style,
            L::BorderColor(side) => self.border_mut(side).color = other.border.get(side).color,
            L::Width => self.width = other.width,
            L::Height => self.height = other.height,
            L::MaxWidth => self.max_width = other.max_width,
            L::MaxHeight => self.max_height = other.max_height,
            L::BoxSizing => self.box_sizing = other.box_sizing,
            L::VerticalAlign => self.vertical_align = other.vertical_align,
            L::BorderCollapse => self.border_collapse = other.border_collapse,
            L::BorderSpacing => self.border_spacing = other.border_spacing,
            L::ListStyleType => self.list_style_type = other.list_style_type,
            L::BreakBefore => self.break_before = other.break_before,
            L::BreakAfter => self.break_after = other.break_after,
            L::MinHeight => self.min_height = other.min_height,
            L::TextDecorationLine => self.text_decoration_line = other.text_decoration_line,
            L::TextDecorationColor => self.text_decoration_color = other.text_decoration_color,
            L::TextIndent => self.text_indent = other.text_indent,
            L::TextTransform => self.text_transform = other.text_transform,
            L::LetterSpacing => self.letter_spacing = other.letter_spacing,
            L::WordSpacing => self.word_spacing = other.word_spacing,
            L::BreakInside => self.break_inside = other.break_inside,
            L::Orphans => self.orphans = other.orphans,
            L::Widows => self.widows = other.widows,
            L::TableLayout => self.table_layout = other.table_layout,
        }
    }

    fn border_mut(&mut self, side: Side) -> &mut BorderSide {
        match side {
            Side::Top => &mut self.border.top,
            Side::Right => &mut self.border.right,
            Side::Bottom => &mut self.border.bottom,
            Side::Left => &mut self.border.left,
        }
    }

    /// Applies one declaration, computing its value. `parent` is the
    /// parent's computed style (or the initial style for the root).
    ///
    /// Font-relative lengths use `self.font_size`, so the cascade applies
    /// `font-size` declarations before all others.
    pub fn apply(&mut self, declaration: &PropertyDeclaration, parent: &ComputedStyle) {
        use PropertyDeclaration as D;
        let (font_size, root) = (self.font_size, self.root_font_size);
        let pt = |length: Length| length.to_pt(font_size, root);
        match declaration {
            D::Display(v) => self.display = *v,
            D::Content(v) => self.content = v.clone(),
            D::BorderRadius(corner, h, v) => {
                self.border_radius[*corner as usize] =
                    (h.compute(font_size, root), v.compute(font_size, root));
            }
            D::CounterReset(v) => self.counter_reset = Arc::clone(v),
            D::CounterIncrement(v) => self.counter_increment = Arc::clone(v),
            D::CounterSet(v) => self.counter_set = Arc::clone(v),
            D::Color(v) => self.color = v.resolve(parent.color),
            D::BackgroundColor(v) => self.background_color = v.resolve(self.color),
            D::FontFamily(v) => self.font_family = Arc::clone(v),
            D::FontSize(v) => {
                self.font_size = match *v {
                    // `em` in font-size is relative to the parent's size.
                    FontSize::Length(l) => l.to_pt(parent.font_size, root),
                    FontSize::Percent(p) => parent.font_size * p,
                    FontSize::Smaller => parent.font_size / 1.2,
                    FontSize::Larger => parent.font_size * 1.2,
                };
            }
            D::FontWeight(v) => {
                self.font_weight = match *v {
                    FontWeight::Weight(w) => w,
                    FontWeight::Bolder => match parent.font_weight {
                        w if w < 350.0 => 400.0,
                        w if w < 550.0 => 700.0,
                        _ => 900.0,
                    },
                    FontWeight::Lighter => match parent.font_weight {
                        w if w < 550.0 => 100.0,
                        w if w < 750.0 => 400.0,
                        _ => 700.0,
                    },
                };
            }
            D::FontStyle(v) => self.italic = *v,
            D::LineHeight(v) => {
                self.line_height = match *v {
                    LineHeight::Length(l) => LineHeight::Length(Length::Pt(pt(l))),
                    LineHeight::Percent(p) => LineHeight::Length(Length::Pt(font_size * p)),
                    other => other,
                };
            }
            D::TextAlign(v) => self.text_align = *v,
            D::WhiteSpace(v) => self.white_space = *v,
            D::Margin(side, v) => self.margin.set(*side, v.compute(font_size, root)),
            D::Padding(side, v) => self.padding.set(*side, v.compute(font_size, root)),
            D::BorderWidth(side, v) => self.border_mut(*side).width = pt(*v),
            D::BorderStyle(side, v) => self.border_mut(*side).style = *v,
            D::BorderColor(side, v) => self.border_mut(*side).color = *v,
            D::Width(v) => self.width = v.compute(font_size, root),
            D::Height(v) => self.height = v.compute(font_size, root),
            D::MaxWidth(v) => self.max_width = v.compute(font_size, root),
            D::MaxHeight(v) => self.max_height = v.compute(font_size, root),
            D::BoxSizing(v) => self.box_sizing = *v,
            D::VerticalAlign(v) => self.vertical_align = *v,
            D::BorderCollapse(v) => self.border_collapse = *v,
            D::BorderSpacing(v) => self.border_spacing = pt(*v),
            D::ListStyleType(v) => self.list_style_type = *v,
            D::BreakBefore(v) => self.break_before = *v,
            D::BreakAfter(v) => self.break_after = *v,
            D::MinHeight(v) => self.min_height = v.compute(font_size, root),
            D::TextDecorationLine(v) => self.text_decoration_line = *v,
            D::TextDecorationColor(v) => self.text_decoration_color = *v,
            D::TextIndent(v) => {
                self.text_indent = TextIndent {
                    length: v.length.compute(font_size, root),
                    hanging: v.hanging,
                    each_line: v.each_line,
                };
            }
            D::TextTransform(v) => self.text_transform = *v,
            D::LetterSpacing(v) => self.letter_spacing = pt(*v),
            D::WordSpacing(v) => self.word_spacing = pt(*v),
            D::BreakInside(v) => self.break_inside = *v,
            D::Orphans(v) => self.orphans = *v,
            D::Widows(v) => self.widows = *v,
            D::TableLayout(v) => self.table_layout = *v,
            D::CssWide(id, keyword) => {
                let inherit = match keyword {
                    CssWide::Inherit => true,
                    CssWide::Initial => false,
                    CssWide::Unset => id.is_inherited(),
                };
                if inherit {
                    self.copy_longhand(*id, parent);
                } else {
                    self.copy_longhand(*id, &ComputedStyle::initial());
                }
            }
        }
    }

    /// The color of the border on `side`, with `currentcolor` resolved.
    pub fn border_color(&self, side: Side) -> Color {
        self.border.get(side).color.resolve(self.color)
    }

    /// The lines to draw on this element's text: its ancestors' and its
    /// own.
    pub fn text_decorations(&self) -> TextDecorations {
        self.inherited_decorations.with(
            self.text_decoration_line,
            self.text_decoration_color.resolve(self.color),
        )
    }

    /// The used border widths in points.
    pub fn border_widths(&self) -> Sides<f32> {
        self.border.map(|b| b.used_width())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::values::Dim;
    use PropertyDeclaration as D;

    #[test]
    fn inherits_only_inherited_properties() {
        let mut parent = ComputedStyle::initial();
        parent.color = Color::rgb(255, 0, 0);
        parent.font_size = 20.0;
        parent.margin.top = Dimension::Pt(5.0);
        parent.background_color = Color::WHITE;
        parent.lang = Some("ne".into());

        let child = ComputedStyle::inheriting(&parent);
        assert_eq!(child.color, parent.color);
        assert_eq!(child.font_size, 20.0);
        assert_eq!(child.lang.as_deref(), Some("ne"));
        assert_eq!(child.margin.top, Dimension::ZERO);
        assert_eq!(child.background_color, Color::TRANSPARENT);
    }

    #[test]
    fn em_resolves_against_the_right_font_size() {
        let mut parent = ComputedStyle::initial();
        parent.font_size = 10.0;
        let mut style = ComputedStyle::inheriting(&parent);
        // font-size: 2em is relative to the parent.
        style.apply(&D::FontSize(FontSize::Length(Length::Em(2.0))), &parent);
        assert_eq!(style.font_size, 20.0);
        // Other properties use the element's own font size.
        style.apply(
            &D::Margin(Side::Left, Dim::Length(Length::Em(1.5))),
            &parent,
        );
        assert_eq!(style.margin.left, Dimension::Pt(30.0));
        style.apply(&D::LineHeight(LineHeight::Percent(1.5)), &parent);
        assert_eq!(style.line_height, LineHeight::Length(Length::Pt(30.0)));
    }

    #[test]
    fn css_wide_keywords() {
        let mut parent = ComputedStyle::initial();
        parent.margin.top = Dimension::Pt(7.0);
        parent.color = Color::rgb(1, 2, 3);
        let mut style = ComputedStyle::inheriting(&parent);

        style.apply(
            &D::CssWide(LonghandId::Margin(Side::Top), CssWide::Inherit),
            &parent,
        );
        assert_eq!(style.margin.top, Dimension::Pt(7.0));
        style.apply(&D::CssWide(LonghandId::Color, CssWide::Initial), &parent);
        assert_eq!(style.color, Color::BLACK);
        style.apply(&D::CssWide(LonghandId::Color, CssWide::Unset), &parent);
        assert_eq!(style.color, parent.color);
    }

    #[test]
    fn border_width_is_zero_without_a_style() {
        let mut style = ComputedStyle::initial();
        assert_eq!(style.border_widths().top, 0.0);
        style.apply(
            &D::BorderStyle(Side::Top, BorderStyle::Solid),
            &ComputedStyle::initial(),
        );
        assert_eq!(style.border_widths().top, 2.25);
        assert_eq!(style.border_color(Side::Top), Color::BLACK);
    }

    #[test]
    fn text_decorations_reach_descendants_that_unset_them() {
        let initial = ComputedStyle::initial();
        let mut parent = ComputedStyle::inheriting(&initial);
        let underline = TextDecorationLine {
            underline: true,
            ..TextDecorationLine::NONE
        };
        parent.apply(&D::TextDecorationLine(underline), &initial);
        parent.apply(
            &D::Color(ColorValue::Color(Color::rgb(255, 0, 0))),
            &initial,
        );

        let mut child = ComputedStyle::inheriting(&parent);
        // Not inherited as a property...
        assert_eq!(child.text_decoration_line, TextDecorationLine::NONE);
        child.apply(&D::Color(ColorValue::Color(Color::BLACK)), &parent);
        let strike = TextDecorationLine {
            line_through: true,
            ..TextDecorationLine::NONE
        };
        child.apply(&D::TextDecorationLine(strike), &parent);
        // ...but still drawn, in the color of the box that set it.
        assert_eq!(
            child.text_decorations(),
            TextDecorations {
                underline: Some(Color::rgb(255, 0, 0)),
                overline: None,
                line_through: Some(Color::BLACK),
            }
        );
        let grandchild = ComputedStyle::inheriting(&child);
        assert_eq!(grandchild.text_decorations(), child.text_decorations());
    }

    #[test]
    fn bolder_and_lighter_step_from_the_parent() {
        let mut parent = ComputedStyle::initial();
        let mut style = ComputedStyle::inheriting(&parent);
        style.apply(&D::FontWeight(FontWeight::Bolder), &parent);
        assert_eq!(style.font_weight, 700.0);
        parent.font_weight = 700.0;
        style.apply(&D::FontWeight(FontWeight::Lighter), &parent);
        assert_eq!(style.font_weight, 400.0);
    }
}
