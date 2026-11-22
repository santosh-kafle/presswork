//! CSS properties: identifiers, specified values and declaration parsing.
//!
//! # Adding a property
//!
//! 1. Add a variant to [`LonghandId`] and say whether it is inherited in
//!    [`LonghandId::is_inherited`].
//! 2. Add a variant carrying its specified value to [`PropertyDeclaration`]
//!    and map it in [`PropertyDeclaration::id`].
//! 3. Map its CSS name in [`longhands_of`] and parse its value in
//!    [`parse_longhand`].
//! 4. Add a field to `ComputedStyle`, then handle the property in
//!    `ComputedStyle::apply` and `ComputedStyle::copy_longhand`.
//!
//! The compiler's exhaustiveness checks point at every place that needs a
//! new arm. Shorthands only expand to longhands, in [`parse_declaration`].

use std::sync::Arc;

use cssparser::{Parser, Token};

use crate::values::{ColorValue, Dim, Invalid, Length, Parsed, Side};

/// Defines an enum of CSS keywords with a `from_keyword` parser. A variant
/// may list several keywords that map to it.
macro_rules! keyword_enum {
    (
        $(#[$meta:meta])*
        $name:ident { $($(#[$vmeta:meta])* $variant:ident = $($css:literal)|+),* $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name {
            $($(#[$vmeta])* $variant,)*
        }

        impl $name {
            /// Parses a keyword, case-insensitively.
            pub fn from_keyword(keyword: &str) -> Option<Self> {
                match keyword.to_ascii_lowercase().as_str() {
                    $($($css)|+ => Some(Self::$variant),)*
                    _ => None,
                }
            }

            #[allow(dead_code, reason = "not every keyword enum is a whole property value")]
            fn parse(input: &mut Parser<'_>) -> Parsed<Self> {
                let ident = input.expect_ident().map_err(Invalid::from)?;
                Self::from_keyword(ident).ok_or(Invalid)
            }
        }
    };
}

keyword_enum! {
    /// The `display` value, limited to what layout supports. Unsupported
    /// layout modes fall back to the closest supported one.
    Display {
        None = "none" | "table-column" | "table-column-group",
        Block = "block" | "flow-root" | "flex" | "grid",
        Inline = "inline",
        InlineBlock = "inline-block" | "inline-flex" | "inline-grid",
        ListItem = "list-item",
        Table = "table" | "inline-table",
        TableRowGroup = "table-row-group",
        TableHeaderGroup = "table-header-group",
        TableFooterGroup = "table-footer-group",
        TableRow = "table-row",
        TableCell = "table-cell",
        TableCaption = "table-caption",
    }
}

impl Display {
    /// Whether the box takes part in block layout (as opposed to inline).
    pub fn is_block_level(self) -> bool {
        !matches!(self, Display::Inline | Display::InlineBlock | Display::None)
    }
}

keyword_enum! {
    /// Horizontal alignment of inline content.
    TextAlign {
        Left = "left" | "start" | "-webkit-left" | "-moz-left",
        Right = "right" | "end" | "-webkit-right" | "-moz-right",
        Center = "center" | "-webkit-center" | "-moz-center",
        Justify = "justify",
    }
}

keyword_enum! {
    WhiteSpace {
        Normal = "normal",
        Nowrap = "nowrap",
        Pre = "pre",
        PreWrap = "pre-wrap" | "break-spaces",
        PreLine = "pre-line",
    }
}

impl WhiteSpace {
    /// Whether runs of spaces collapse to one.
    pub fn collapses_spaces(self) -> bool {
        matches!(
            self,
            WhiteSpace::Normal | WhiteSpace::Nowrap | WhiteSpace::PreLine
        )
    }

    /// Whether line breaks in the source are kept.
    pub fn preserves_newlines(self) -> bool {
        matches!(
            self,
            WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::PreLine
        )
    }

    /// Whether lines may wrap.
    pub fn wraps(self) -> bool {
        !matches!(self, WhiteSpace::Nowrap | WhiteSpace::Pre)
    }
}

keyword_enum! {
    BorderStyle {
        None = "none",
        Hidden = "hidden",
        Solid = "solid",
        Dashed = "dashed",
        Dotted = "dotted",
        Double = "double",
        Groove = "groove",
        Ridge = "ridge",
        Inset = "inset",
        Outset = "outset",
    }
}

impl BorderStyle {
    /// Whether a border with this style is drawn at all.
    pub fn is_visible(self) -> bool {
        !matches!(self, BorderStyle::None | BorderStyle::Hidden)
    }
}

keyword_enum! {
    BoxSizing {
        ContentBox = "content-box",
        BorderBox = "border-box",
    }
}

keyword_enum! {
    /// `vertical-align`. Lengths and percentages are not supported. Table
    /// cells treat the values that are not `top`, `middle` or `bottom` as
    /// `baseline`.
    VerticalAlign {
        Baseline = "baseline",
        Sub = "sub",
        Super = "super",
        TextTop = "text-top",
        TextBottom = "text-bottom",
        Middle = "middle",
        Top = "top",
        Bottom = "bottom",
    }
}

keyword_enum! {
    ListStyleType {
        None = "none",
        Disc = "disc",
        Circle = "circle",
        Square = "square",
        Decimal = "decimal",
        DecimalLeadingZero = "decimal-leading-zero",
        /// Devanagari digits, as used in Nepali and Hindi.
        Devanagari = "devanagari",
        LowerAlpha = "lower-alpha" | "lower-latin",
        UpperAlpha = "upper-alpha" | "upper-latin",
        LowerRoman = "lower-roman",
        UpperRoman = "upper-roman",
    }
}

keyword_enum! {
    /// `break-before` / `break-after` (and the legacy `page-break-*`).
    Break {
        Auto = "auto",
        Avoid = "avoid" | "avoid-page" | "avoid-column",
        Page = "page" | "always" | "left" | "right" | "recto" | "verso",
    }
}

keyword_enum! {
    /// `break-inside` (and the legacy `page-break-inside`).
    BreakInside {
        Auto = "auto",
        Avoid = "avoid" | "avoid-page" | "avoid-column",
    }
}

keyword_enum! {
    TableLayout {
        Auto = "auto",
        Fixed = "fixed",
    }
}

keyword_enum! {
    BorderCollapse {
        Separate = "separate",
        Collapse = "collapse",
    }
}

keyword_enum! {
    TextTransform {
        None = "none",
        Uppercase = "uppercase",
        Lowercase = "lowercase",
        Capitalize = "capitalize",
    }
}

/// The lines of `text-decoration-line`. Several can be set at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextDecorationLine {
    pub underline: bool,
    pub overline: bool,
    pub line_through: bool,
}

impl TextDecorationLine {
    pub const NONE: Self = Self {
        underline: false,
        overline: false,
        line_through: false,
    };
}

/// A specified `text-indent`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextIndentValue {
    /// A length or a percentage of the containing block's width.
    pub length: Dim,
    /// `hanging`: indent every line but the first.
    pub hanging: bool,
    /// `each-line`: also indent lines after a forced line break.
    pub each_line: bool,
}

keyword_enum! {
    /// The CSS-wide keywords every property accepts.
    CssWide {
        Inherit = "inherit",
        Initial = "initial",
        Unset = "unset" | "revert" | "revert-layer",
    }
}

keyword_enum! {
    GenericFamily {
        Serif = "serif" | "ui-serif",
        SansSerif = "sans-serif" | "system-ui" | "ui-sans-serif" | "cursive" | "fantasy",
        Monospace = "monospace" | "ui-monospace",
    }
}

/// One entry of a `font-family` list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontFamilyName {
    Named(String),
    Generic(GenericFamily),
}

/// A specified `font-size`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FontSize {
    Length(Length),
    /// A fraction of the parent's font size.
    Percent(f32),
    Smaller,
    Larger,
}

/// A specified `font-weight`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FontWeight {
    Weight(f32),
    Bolder,
    Lighter,
}

/// A `line-height`. Used for both specified and computed values; a computed
/// `Length` is always in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LineHeight {
    Normal,
    /// A multiple of the element's own font size, inherited as a factor.
    Number(f32),
    Length(Length),
    /// A fraction of the element's font size, inherited as a length.
    Percent(f32),
}

/// One axis of a `background-position`: an offset from the start (left
/// or top) or, with `from_end`, the far edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionOffset {
    pub length: Dim,
    pub from_end: bool,
}

/// A specified `background-size`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BackgroundSizeValue {
    Cover,
    Contain,
    /// Width and height; `None` is `auto`.
    Explicit(Option<Dim>, Option<Dim>),
}

/// A corner of a box, for `border-radius`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
}

impl Corner {
    /// In the order `border-radius` lists them, which is also their index
    /// in `ComputedStyle::border_radius`.
    pub const ALL: [Corner; 4] = [
        Corner::TopLeft,
        Corner::TopRight,
        Corner::BottomRight,
        Corner::BottomLeft,
    ];
}

/// One part of a `content` value.
#[derive(Debug, Clone, PartialEq)]
pub enum ContentItem {
    Text(Arc<str>),
    /// `counter(name, style)`: the innermost counter of that name.
    Counter {
        name: Arc<str>,
        style: ListStyleType,
    },
    /// `counters(name, separator, style)`: all counters of that name,
    /// outermost first.
    Counters {
        name: Arc<str>,
        separator: Arc<str>,
        style: ListStyleType,
    },
    /// `attr(name)`: the element's attribute, or nothing.
    Attr(Arc<str>),
    OpenQuote,
    CloseQuote,
    NoOpenQuote,
    NoCloseQuote,
}

/// The `content` property. On `::before` and `::after`, `normal` and
/// `none` both generate nothing.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Content {
    #[default]
    Normal,
    None,
    Items(Arc<[ContentItem]>),
}

/// The value of `counter-reset`, `counter-increment` or `counter-set`:
/// counter names with their values. `none` is empty.
pub type CounterChanges = Arc<[(Arc<str>, i32)]>;

/// A longhand property.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LonghandId {
    Display,
    Color,
    BackgroundColor,
    FontFamily,
    FontSize,
    FontWeight,
    FontStyle,
    LineHeight,
    TextAlign,
    WhiteSpace,
    Margin(Side),
    Padding(Side),
    BorderWidth(Side),
    BorderStyle(Side),
    BorderColor(Side),
    Width,
    Height,
    MaxWidth,
    MaxHeight,
    BoxSizing,
    VerticalAlign,
    BorderCollapse,
    BorderSpacing,
    ListStyleType,
    BreakBefore,
    BreakAfter,
    MinHeight,
    TextDecorationLine,
    TextDecorationColor,
    TextIndent,
    TextTransform,
    LetterSpacing,
    WordSpacing,
    BreakInside,
    Orphans,
    Widows,
    TableLayout,
    Content,
    CounterReset,
    CounterIncrement,
    CounterSet,
    BorderRadius(Corner),
    BackgroundImage,
    BackgroundRepeat,
    BackgroundPosition,
    BackgroundSize,
}

impl LonghandId {
    /// Whether the property inherits from the parent by default.
    pub fn is_inherited(self) -> bool {
        use LonghandId::*;
        match self {
            Color | FontFamily | FontSize | FontWeight | FontStyle | LineHeight | TextAlign
            | WhiteSpace | BorderCollapse | BorderSpacing | ListStyleType | TextIndent
            | TextTransform | LetterSpacing | WordSpacing | Orphans | Widows => true,
            Display | BackgroundColor | Margin(_) | Padding(_) | BorderWidth(_)
            | BorderStyle(_) | BorderColor(_) | Width | Height | MaxWidth | MaxHeight
            | MinHeight | BoxSizing | VerticalAlign | BreakBefore | BreakAfter
            | TextDecorationLine | TextDecorationColor | BreakInside | TableLayout | Content
            | CounterReset | CounterIncrement | CounterSet | BorderRadius(_) | BackgroundImage
            | BackgroundRepeat | BackgroundPosition | BackgroundSize => false,
        }
    }

    /// Every longhand, for code that must visit them all.
    pub fn all() -> impl Iterator<Item = LonghandId> {
        use LonghandId::*;
        let simple = [
            Display,
            Color,
            BackgroundColor,
            FontFamily,
            FontSize,
            FontWeight,
            FontStyle,
            LineHeight,
            TextAlign,
            WhiteSpace,
            Width,
            Height,
            MaxWidth,
            MaxHeight,
            BoxSizing,
            VerticalAlign,
            BorderCollapse,
            BorderSpacing,
            ListStyleType,
            BreakBefore,
            BreakAfter,
            MinHeight,
            TextDecorationLine,
            TextDecorationColor,
            TextIndent,
            TextTransform,
            LetterSpacing,
            WordSpacing,
            BreakInside,
            Orphans,
            Widows,
            TableLayout,
            Content,
            CounterReset,
            CounterIncrement,
            CounterSet,
            BackgroundImage,
            BackgroundRepeat,
            BackgroundPosition,
            BackgroundSize,
        ];
        let sided = Side::ALL.into_iter().flat_map(|side| {
            [
                Margin(side),
                Padding(side),
                BorderWidth(side),
                BorderStyle(side),
                BorderColor(side),
            ]
        });
        let corners = Corner::ALL.into_iter().map(BorderRadius);
        simple.into_iter().chain(sided).chain(corners)
    }
}

/// A parsed declaration of one longhand: the property and its specified
/// value.
#[derive(Debug, Clone, PartialEq)]
pub enum PropertyDeclaration {
    Display(Display),
    Color(ColorValue),
    BackgroundColor(ColorValue),
    FontFamily(Arc<[FontFamilyName]>),
    FontSize(FontSize),
    FontWeight(FontWeight),
    /// `true` for `italic` and `oblique`.
    FontStyle(bool),
    LineHeight(LineHeight),
    TextAlign(TextAlign),
    WhiteSpace(WhiteSpace),
    Margin(Side, Dim),
    Padding(Side, Dim),
    BorderWidth(Side, Length),
    BorderStyle(Side, BorderStyle),
    BorderColor(Side, ColorValue),
    Width(Dim),
    Height(Dim),
    /// `none` is [`Dim::Auto`].
    MaxWidth(Dim),
    /// `none` is [`Dim::Auto`].
    MaxHeight(Dim),
    BoxSizing(BoxSizing),
    VerticalAlign(VerticalAlign),
    BorderCollapse(BorderCollapse),
    BorderSpacing(Length),
    ListStyleType(ListStyleType),
    BreakBefore(Break),
    BreakAfter(Break),
    /// `auto` is [`Dim::Auto`], which means zero.
    MinHeight(Dim),
    TextDecorationLine(TextDecorationLine),
    TextDecorationColor(ColorValue),
    TextIndent(TextIndentValue),
    TextTransform(TextTransform),
    /// `normal` is zero.
    LetterSpacing(Length),
    /// `normal` is zero.
    WordSpacing(Length),
    BreakInside(BreakInside),
    /// At least 1.
    Orphans(u32),
    /// At least 1.
    Widows(u32),
    TableLayout(TableLayout),
    Content(Content),
    CounterReset(CounterChanges),
    CounterIncrement(CounterChanges),
    CounterSet(CounterChanges),
    /// Horizontal and vertical radius.
    BorderRadius(Corner, Dim, Dim),
    /// The image's URL, as written (or resolved against its stylesheet);
    /// `None` for `none`.
    BackgroundImage(Option<Arc<str>>),
    /// Whether the image repeats horizontally and vertically.
    BackgroundRepeat(bool, bool),
    /// Horizontal and vertical offset; keywords are percentages.
    BackgroundPosition(PositionOffset, PositionOffset),
    BackgroundSize(BackgroundSizeValue),
    /// `inherit`, `initial` or `unset` for the given property.
    CssWide(LonghandId, CssWide),
}

impl PropertyDeclaration {
    /// The property this declaration sets.
    pub fn id(&self) -> LonghandId {
        use PropertyDeclaration as D;
        match self {
            D::Display(_) => LonghandId::Display,
            D::Color(_) => LonghandId::Color,
            D::BackgroundColor(_) => LonghandId::BackgroundColor,
            D::FontFamily(_) => LonghandId::FontFamily,
            D::FontSize(_) => LonghandId::FontSize,
            D::FontWeight(_) => LonghandId::FontWeight,
            D::FontStyle(_) => LonghandId::FontStyle,
            D::LineHeight(_) => LonghandId::LineHeight,
            D::TextAlign(_) => LonghandId::TextAlign,
            D::WhiteSpace(_) => LonghandId::WhiteSpace,
            D::Margin(side, _) => LonghandId::Margin(*side),
            D::Padding(side, _) => LonghandId::Padding(*side),
            D::BorderWidth(side, _) => LonghandId::BorderWidth(*side),
            D::BorderStyle(side, _) => LonghandId::BorderStyle(*side),
            D::BorderColor(side, _) => LonghandId::BorderColor(*side),
            D::Width(_) => LonghandId::Width,
            D::Height(_) => LonghandId::Height,
            D::MaxWidth(_) => LonghandId::MaxWidth,
            D::MaxHeight(_) => LonghandId::MaxHeight,
            D::BoxSizing(_) => LonghandId::BoxSizing,
            D::VerticalAlign(_) => LonghandId::VerticalAlign,
            D::BorderCollapse(_) => LonghandId::BorderCollapse,
            D::BorderSpacing(_) => LonghandId::BorderSpacing,
            D::ListStyleType(_) => LonghandId::ListStyleType,
            D::BreakBefore(_) => LonghandId::BreakBefore,
            D::BreakAfter(_) => LonghandId::BreakAfter,
            D::MinHeight(_) => LonghandId::MinHeight,
            D::TextDecorationLine(_) => LonghandId::TextDecorationLine,
            D::TextDecorationColor(_) => LonghandId::TextDecorationColor,
            D::TextIndent(_) => LonghandId::TextIndent,
            D::TextTransform(_) => LonghandId::TextTransform,
            D::LetterSpacing(_) => LonghandId::LetterSpacing,
            D::WordSpacing(_) => LonghandId::WordSpacing,
            D::BreakInside(_) => LonghandId::BreakInside,
            D::Orphans(_) => LonghandId::Orphans,
            D::Widows(_) => LonghandId::Widows,
            D::TableLayout(_) => LonghandId::TableLayout,
            D::Content(_) => LonghandId::Content,
            D::CounterReset(_) => LonghandId::CounterReset,
            D::CounterIncrement(_) => LonghandId::CounterIncrement,
            D::CounterSet(_) => LonghandId::CounterSet,
            D::BorderRadius(corner, ..) => LonghandId::BorderRadius(*corner),
            D::BackgroundImage(_) => LonghandId::BackgroundImage,
            D::BackgroundRepeat(..) => LonghandId::BackgroundRepeat,
            D::BackgroundPosition(..) => LonghandId::BackgroundPosition,
            D::BackgroundSize(_) => LonghandId::BackgroundSize,
            D::CssWide(id, _) => *id,
        }
    }
}

/// `medium`, the initial `border-width`.
pub const BORDER_WIDTH_MEDIUM: Length = Length::Pt(2.25);

fn sided(f: fn(Side) -> LonghandId) -> Vec<LonghandId> {
    Side::ALL.into_iter().map(f).collect()
}

fn side_of(name: &str) -> Option<Side> {
    match name {
        "top" => Some(Side::Top),
        "right" => Some(Side::Right),
        "bottom" => Some(Side::Bottom),
        "left" => Some(Side::Left),
        _ => None,
    }
}

fn border_side_longhands(side: Side) -> Vec<LonghandId> {
    vec![
        LonghandId::BorderWidth(side),
        LonghandId::BorderStyle(side),
        LonghandId::BorderColor(side),
    ]
}

/// The longhands a property name (longhand or shorthand, lowercase) sets,
/// or `None` if the property is not supported.
pub fn longhands_of(name: &str) -> Option<Vec<LonghandId>> {
    use LonghandId as L;
    let one = |id| Some(vec![id]);
    match name {
        "display" => one(L::Display),
        "color" => one(L::Color),
        "background-color" => one(L::BackgroundColor),
        "background" => Some(vec![
            L::BackgroundColor,
            L::BackgroundImage,
            L::BackgroundRepeat,
            L::BackgroundPosition,
            L::BackgroundSize,
        ]),
        "background-image" => one(L::BackgroundImage),
        "background-repeat" => one(L::BackgroundRepeat),
        "background-position" => one(L::BackgroundPosition),
        "background-size" => one(L::BackgroundSize),
        "font-family" => one(L::FontFamily),
        "font-size" => one(L::FontSize),
        "font-weight" => one(L::FontWeight),
        "font-style" => one(L::FontStyle),
        "line-height" => one(L::LineHeight),
        "text-align" => one(L::TextAlign),
        "white-space" => one(L::WhiteSpace),
        "width" => one(L::Width),
        "height" => one(L::Height),
        "max-width" => one(L::MaxWidth),
        "max-height" => one(L::MaxHeight),
        "box-sizing" => one(L::BoxSizing),
        "vertical-align" => one(L::VerticalAlign),
        "border-collapse" => one(L::BorderCollapse),
        "border-spacing" => one(L::BorderSpacing),
        "list-style-type" | "list-style" => one(L::ListStyleType),
        "break-before" | "page-break-before" => one(L::BreakBefore),
        "break-after" | "page-break-after" => one(L::BreakAfter),
        "break-inside" | "page-break-inside" => one(L::BreakInside),
        "orphans" => one(L::Orphans),
        "widows" => one(L::Widows),
        "table-layout" => one(L::TableLayout),
        "content" => one(L::Content),
        "counter-reset" => one(L::CounterReset),
        "counter-increment" => one(L::CounterIncrement),
        "counter-set" => one(L::CounterSet),
        "border-radius" => Some(Corner::ALL.map(L::BorderRadius).to_vec()),
        "border-top-left-radius" => one(L::BorderRadius(Corner::TopLeft)),
        "border-top-right-radius" => one(L::BorderRadius(Corner::TopRight)),
        "border-bottom-right-radius" => one(L::BorderRadius(Corner::BottomRight)),
        "border-bottom-left-radius" => one(L::BorderRadius(Corner::BottomLeft)),
        "min-height" => one(L::MinHeight),
        "text-decoration-line" => one(L::TextDecorationLine),
        "text-decoration-color" => one(L::TextDecorationColor),
        "text-decoration" => Some(vec![L::TextDecorationLine, L::TextDecorationColor]),
        "text-indent" => one(L::TextIndent),
        "text-transform" => one(L::TextTransform),
        "letter-spacing" => one(L::LetterSpacing),
        "word-spacing" => one(L::WordSpacing),
        "font" => Some(vec![
            L::FontStyle,
            L::FontWeight,
            L::FontSize,
            L::LineHeight,
            L::FontFamily,
        ]),
        "margin" => Some(sided(L::Margin)),
        "padding" => Some(sided(L::Padding)),
        "border-width" => Some(sided(L::BorderWidth)),
        "border-style" => Some(sided(L::BorderStyle)),
        "border-color" => Some(sided(L::BorderColor)),
        "border" => Some(
            Side::ALL
                .into_iter()
                .flat_map(border_side_longhands)
                .collect(),
        ),
        _ => {
            if let Some(side) = name.strip_prefix("margin-").and_then(side_of) {
                return one(L::Margin(side));
            }
            if let Some(side) = name.strip_prefix("padding-").and_then(side_of) {
                return one(L::Padding(side));
            }
            let rest = name.strip_prefix("border-")?;
            if let Some(side) = side_of(rest) {
                return Some(border_side_longhands(side));
            }
            let (side, part) = rest.split_once('-')?;
            let side = side_of(side)?;
            match part {
                "width" => one(L::BorderWidth(side)),
                "style" => one(L::BorderStyle(side)),
                "color" => one(L::BorderColor(side)),
                _ => None,
            }
        }
    }
}

/// Parses the value of the property `name` and appends the resulting
/// longhand declarations to `out`. The whole input must be consumed.
///
/// Returns `Err` for unknown properties and invalid values; `out` may then
/// hold partial results and should be discarded by the caller.
pub fn parse_declaration(
    name: &str,
    input: &mut Parser<'_>,
    out: &mut Vec<PropertyDeclaration>,
) -> Parsed<()> {
    use PropertyDeclaration as D;
    let name = name.to_ascii_lowercase();
    let longhands = longhands_of(&name).ok_or(Invalid)?;

    let css_wide = input.try_parse(|i| {
        let keyword = CssWide::parse(i)?;
        i.expect_exhausted().map_err(Invalid::from)?;
        Ok::<_, Invalid>(keyword)
    });
    if let Ok(keyword) = css_wide {
        out.extend(longhands.into_iter().map(|id| D::CssWide(id, keyword)));
        return Ok(());
    }

    match name.as_str() {
        "margin" => four_sides(input, |i| Dim::parse(i, true, true), D::Margin, out)?,
        "padding" => four_sides(input, |i| Dim::parse(i, false, false), D::Padding, out)?,
        "border-width" => four_sides(input, parse_border_width, D::BorderWidth, out)?,
        "border-style" => four_sides(input, BorderStyle::parse, D::BorderStyle, out)?,
        "border-color" => four_sides(input, ColorValue::parse, D::BorderColor, out)?,
        "border" => parse_border(input, &Side::ALL, out)?,
        "border-top" => parse_border(input, &[Side::Top], out)?,
        "border-right" => parse_border(input, &[Side::Right], out)?,
        "border-bottom" => parse_border(input, &[Side::Bottom], out)?,
        "border-left" => parse_border(input, &[Side::Left], out)?,
        "font" => parse_font(input, out)?,
        "background" => parse_background(input, out),
        "list-style" => out.push(D::ListStyleType(parse_list_style(input))),
        "text-decoration" => parse_text_decoration(input, out)?,
        "border-radius" => parse_border_radius(input, out)?,
        _ => out.push(parse_longhand(longhands[0], input)?),
    }
    input.expect_exhausted().map_err(Invalid::from)
}

/// Parses the value of a single longhand.
fn parse_longhand(id: LonghandId, input: &mut Parser<'_>) -> Parsed<PropertyDeclaration> {
    use LonghandId as L;
    use PropertyDeclaration as D;
    Ok(match id {
        L::Display => D::Display(Display::parse(input)?),
        L::Color => D::Color(ColorValue::parse(input)?),
        L::BackgroundColor => D::BackgroundColor(ColorValue::parse(input)?),
        L::FontFamily => D::FontFamily(parse_font_family(input)?),
        L::FontSize => D::FontSize(parse_font_size(input)?),
        L::FontWeight => D::FontWeight(parse_font_weight(input)?),
        L::FontStyle => D::FontStyle(parse_font_style(input)?),
        L::LineHeight => D::LineHeight(parse_line_height(input)?),
        L::TextAlign => D::TextAlign(TextAlign::parse(input)?),
        L::WhiteSpace => D::WhiteSpace(WhiteSpace::parse(input)?),
        L::Margin(side) => D::Margin(side, Dim::parse(input, true, true)?),
        L::Padding(side) => D::Padding(side, Dim::parse(input, false, false)?),
        L::BorderWidth(side) => D::BorderWidth(side, parse_border_width(input)?),
        L::BorderStyle(side) => D::BorderStyle(side, BorderStyle::parse(input)?),
        L::BorderColor(side) => D::BorderColor(side, ColorValue::parse(input)?),
        L::Width => D::Width(Dim::parse(input, true, false)?),
        L::Height => D::Height(Dim::parse(input, true, false)?),
        L::MaxWidth => D::MaxWidth(parse_max_size(input)?),
        L::MaxHeight => D::MaxHeight(parse_max_size(input)?),
        L::BoxSizing => D::BoxSizing(BoxSizing::parse(input)?),
        L::VerticalAlign => D::VerticalAlign(VerticalAlign::parse(input)?),
        L::BorderCollapse => D::BorderCollapse(BorderCollapse::parse(input)?),
        L::BorderSpacing => {
            let horizontal = Length::parse_non_negative(input)?;
            // The optional second value is the vertical spacing; one value
            // is used for both directions.
            let _ = input.try_parse(Length::parse_non_negative);
            D::BorderSpacing(horizontal)
        }
        L::ListStyleType => D::ListStyleType(ListStyleType::parse(input)?),
        L::BreakBefore => D::BreakBefore(Break::parse(input)?),
        L::BreakAfter => D::BreakAfter(Break::parse(input)?),
        L::MinHeight => D::MinHeight(Dim::parse(input, true, false)?),
        L::TextDecorationLine => D::TextDecorationLine(parse_text_decoration_line(input)?),
        L::TextDecorationColor => D::TextDecorationColor(ColorValue::parse(input)?),
        L::TextIndent => D::TextIndent(parse_text_indent(input)?),
        L::TextTransform => D::TextTransform(TextTransform::parse(input)?),
        L::LetterSpacing => D::LetterSpacing(parse_spacing(input)?),
        L::WordSpacing => D::WordSpacing(parse_spacing(input)?),
        L::BreakInside => D::BreakInside(BreakInside::parse(input)?),
        L::Orphans => D::Orphans(parse_line_count(input)?),
        L::Widows => D::Widows(parse_line_count(input)?),
        L::TableLayout => D::TableLayout(TableLayout::parse(input)?),
        L::Content => D::Content(parse_content(input)?),
        L::CounterReset => D::CounterReset(parse_counter_changes(input, 0)?),
        L::CounterIncrement => D::CounterIncrement(parse_counter_changes(input, 1)?),
        L::CounterSet => D::CounterSet(parse_counter_changes(input, 0)?),
        L::BackgroundImage => D::BackgroundImage(parse_background_image(input)?),
        L::BackgroundRepeat => {
            let (x, y) = parse_background_repeat(input)?;
            D::BackgroundRepeat(x, y)
        }
        L::BackgroundPosition => {
            let (x, y) = parse_background_position(input)?;
            D::BackgroundPosition(x, y)
        }
        L::BackgroundSize => D::BackgroundSize(parse_background_size(input)?),
        L::BorderRadius(corner) => {
            let horizontal = Dim::parse(input, false, false)?;
            let vertical = input
                .try_parse(|i| Dim::parse(i, false, false))
                .unwrap_or(horizontal);
            D::BorderRadius(corner, horizontal, vertical)
        }
    })
}

/// `max-width` / `max-height`: a length or percentage, or `none`.
fn parse_max_size(input: &mut Parser<'_>) -> Parsed<Dim> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Ok(Dim::Auto);
    }
    Dim::parse(input, false, false)
}

/// Parses one to four values and assigns them to sides the way `margin`
/// does: all; vertical, horizontal; top, horizontal, bottom; or each side.
fn four_sides<T: Copy>(
    input: &mut Parser<'_>,
    mut parse: impl FnMut(&mut Parser<'_>) -> Parsed<T>,
    declare: fn(Side, T) -> PropertyDeclaration,
    out: &mut Vec<PropertyDeclaration>,
) -> Parsed<()> {
    let mut values = vec![parse(input)?];
    while values.len() < 4 {
        match input.try_parse(&mut parse) {
            Ok(value) => values.push(value),
            Err(Invalid) => break,
        }
    }
    let (top, right, bottom, left) = match values[..] {
        [a] => (a, a, a, a),
        [a, b] => (a, b, a, b),
        [a, b, c] => (a, b, c, b),
        [a, b, c, d] => (a, b, c, d),
        _ => unreachable!("one to four values"),
    };
    out.extend([
        declare(Side::Top, top),
        declare(Side::Right, right),
        declare(Side::Bottom, bottom),
        declare(Side::Left, left),
    ]);
    Ok(())
}

fn parse_border_width(input: &mut Parser<'_>) -> Parsed<Length> {
    if let Ok(length) = input.try_parse(Length::parse_non_negative) {
        return Ok(length);
    }
    let ident = input.expect_ident().map_err(Invalid::from)?;
    match ident.to_ascii_lowercase().as_str() {
        "thin" => Ok(Length::px(1.0)),
        "medium" => Ok(BORDER_WIDTH_MEDIUM),
        "thick" => Ok(Length::px(5.0)),
        _ => Err(Invalid),
    }
}

/// `border` and `border-<side>`: width, style and color in any order.
/// Omitted parts reset to their initial values.
fn parse_border(
    input: &mut Parser<'_>,
    sides: &[Side],
    out: &mut Vec<PropertyDeclaration>,
) -> Parsed<()> {
    let (mut width, mut style, mut color) = (None, None, None);
    let mut any = false;
    loop {
        if width.is_none()
            && let Ok(v) = input.try_parse(parse_border_width)
        {
            width = Some(v);
        } else if style.is_none()
            && let Ok(v) = input.try_parse(BorderStyle::parse)
        {
            style = Some(v);
        } else if color.is_none()
            && let Ok(v) = input.try_parse(ColorValue::parse)
        {
            color = Some(v);
        } else {
            break;
        }
        any = true;
    }
    if !any {
        return Err(Invalid);
    }
    for &side in sides {
        out.extend([
            PropertyDeclaration::BorderWidth(side, width.unwrap_or(BORDER_WIDTH_MEDIUM)),
            PropertyDeclaration::BorderStyle(side, style.unwrap_or(BorderStyle::None)),
            PropertyDeclaration::BorderColor(side, color.unwrap_or(ColorValue::CurrentColor)),
        ]);
    }
    Ok(())
}

fn parse_font_family(input: &mut Parser<'_>) -> Parsed<Arc<[FontFamilyName]>> {
    let families = input
        .parse_comma_separated(|i| {
            if let Ok(name) = i.try_parse(|i| i.expect_string_cloned()) {
                return Ok(FontFamilyName::Named(name.to_string()));
            }
            // An unquoted name is one or more identifiers.
            let mut name = i.expect_ident()?.to_string();
            let mut words = 1;
            while let Ok(word) = i.try_parse(|i| i.expect_ident_cloned()) {
                name.push(' ');
                name.push_str(&word);
                words += 1;
            }
            let generic = (words == 1)
                .then(|| GenericFamily::from_keyword(&name))
                .flatten();
            Ok::<_, cssparser::ParseError<Invalid>>(match generic {
                Some(generic) => FontFamilyName::Generic(generic),
                None => FontFamilyName::Named(name),
            })
        })
        .map_err(Invalid::from)?;
    Ok(families.into())
}

fn parse_font_size(input: &mut Parser<'_>) -> Parsed<FontSize> {
    if let Ok(fraction) = input.try_parse(|i| i.expect_percentage()) {
        return if fraction < 0.0 {
            Err(Invalid)
        } else {
            Ok(FontSize::Percent(fraction))
        };
    }
    if let Ok(length) = input.try_parse(Length::parse_non_negative) {
        return Ok(FontSize::Length(length));
    }
    let ident = input.expect_ident().map_err(Invalid::from)?;
    let px = |v: f32| Ok(FontSize::Length(Length::px(v)));
    match ident.to_ascii_lowercase().as_str() {
        "xx-small" => px(9.0),
        "x-small" => px(10.0),
        "small" => px(13.0),
        "medium" => px(16.0),
        "large" => px(18.0),
        "x-large" => px(24.0),
        "xx-large" => px(32.0),
        "xxx-large" => px(48.0),
        "smaller" => Ok(FontSize::Smaller),
        "larger" => Ok(FontSize::Larger),
        _ => Err(Invalid),
    }
}

fn parse_font_weight(input: &mut Parser<'_>) -> Parsed<FontWeight> {
    match input.next().map_err(Invalid::from)? {
        Token::Number { value, .. } if (1.0..=1000.0).contains(value) => {
            Ok(FontWeight::Weight(*value))
        }
        Token::Ident(ident) => match ident.to_ascii_lowercase().as_str() {
            "normal" => Ok(FontWeight::Weight(400.0)),
            "bold" => Ok(FontWeight::Weight(700.0)),
            "bolder" => Ok(FontWeight::Bolder),
            "lighter" => Ok(FontWeight::Lighter),
            _ => Err(Invalid),
        },
        _ => Err(Invalid),
    }
}

fn parse_font_style(input: &mut Parser<'_>) -> Parsed<bool> {
    let ident = input.expect_ident().map_err(Invalid::from)?;
    match ident.to_ascii_lowercase().as_str() {
        "normal" => Ok(false),
        "italic" | "oblique" => Ok(true),
        _ => Err(Invalid),
    }
}

fn parse_line_height(input: &mut Parser<'_>) -> Parsed<LineHeight> {
    if input
        .try_parse(|i| i.expect_ident_matching("normal"))
        .is_ok()
    {
        return Ok(LineHeight::Normal);
    }
    if let Ok(number) = input.try_parse(|i| i.expect_number()) {
        return if number < 0.0 {
            Err(Invalid)
        } else {
            Ok(LineHeight::Number(number))
        };
    }
    if let Ok(fraction) = input.try_parse(|i| i.expect_percentage()) {
        return if fraction < 0.0 {
            Err(Invalid)
        } else {
            Ok(LineHeight::Percent(fraction))
        };
    }
    Length::parse_non_negative(input).map(LineHeight::Length)
}

/// `font`: `[style || weight || variant]? size[/line-height] family`.
/// Omitted parts reset to their initial values. System fonts (`caption`,
/// ...) are not supported.
fn parse_font(input: &mut Parser<'_>, out: &mut Vec<PropertyDeclaration>) -> Parsed<()> {
    let (mut italic, mut weight) = (None, None);
    loop {
        if input
            .try_parse(|i| i.expect_ident_matching("normal"))
            .is_ok()
            || input
                .try_parse(|i| i.expect_ident_matching("small-caps"))
                .is_ok()
        {
            continue;
        }
        if italic.is_none()
            && let Ok(v) = input.try_parse(parse_font_style)
        {
            italic = Some(v);
        } else if weight.is_none()
            && let Ok(v) = input.try_parse(parse_font_weight)
        {
            weight = Some(v);
        } else {
            break;
        }
    }
    let size = parse_font_size(input)?;
    let line_height = if input.try_parse(|i| i.expect_delim('/')).is_ok() {
        parse_line_height(input)?
    } else {
        LineHeight::Normal
    };
    let family = parse_font_family(input)?;
    out.extend([
        PropertyDeclaration::FontStyle(italic.unwrap_or(false)),
        PropertyDeclaration::FontWeight(weight.unwrap_or(FontWeight::Weight(400.0))),
        PropertyDeclaration::FontSize(size),
        PropertyDeclaration::LineHeight(line_height),
        PropertyDeclaration::FontFamily(family),
    ]);
    Ok(())
}

/// `background`: color, image, repeat, position and size, in any order.
/// Parts that are not supported (gradients, `fixed`, the box keywords)
/// are skipped rather than invalidating the color. Unset parts get their
/// initial values, as the shorthand resets them.
fn parse_background(input: &mut Parser<'_>, out: &mut Vec<PropertyDeclaration>) {
    use PropertyDeclaration as D;
    let mut color = ColorValue::Color(crate::values::Color::TRANSPARENT);
    let mut image = None;
    let mut repeat = (true, true);
    let origin = PositionOffset {
        length: Dim::Percent(0.0),
        from_end: false,
    };
    let mut position = (origin, origin);
    let mut size = BackgroundSizeValue::Explicit(None, None);
    while !input.is_exhausted() {
        if let Ok(found) = input.try_parse(ColorValue::parse) {
            color = found;
        } else if let Ok(found) = input.try_parse(parse_background_image) {
            image = found;
        } else if let Ok(found) = input.try_parse(parse_background_repeat) {
            repeat = found;
        } else if let Ok(found) = input.try_parse(parse_background_position) {
            position = found;
            if input.try_parse(|i| i.expect_delim('/')).is_ok()
                && let Ok(found) = input.try_parse(parse_background_size)
            {
                size = found;
            }
        } else if input.next().is_err() {
            break;
        }
    }
    out.extend([
        D::BackgroundColor(color),
        D::BackgroundImage(image),
        D::BackgroundRepeat(repeat.0, repeat.1),
        D::BackgroundPosition(position.0, position.1),
        D::BackgroundSize(size),
    ]);
}

/// `background-image`: `none` or a `url()`.
fn parse_background_image(input: &mut Parser<'_>) -> Parsed<Option<Arc<str>>> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Ok(None);
    }
    let url = input.expect_url().map_err(Invalid::from)?;
    Ok(Some(url.as_ref().into()))
}

/// `background-repeat`: one keyword, or one for each axis.
fn parse_background_repeat(input: &mut Parser<'_>) -> Parsed<(bool, bool)> {
    // `space` and `round` repeat as `repeat` does, without the spacing.
    let keyword = |input: &mut Parser<'_>| -> Parsed<Option<bool>> {
        let ident = input.expect_ident().map_err(Invalid::from)?;
        match ident.to_ascii_lowercase().as_str() {
            "repeat" | "space" | "round" => Ok(Some(true)),
            "no-repeat" => Ok(Some(false)),
            "repeat-x" | "repeat-y" => Ok(None),
            _ => Err(Invalid),
        }
    };
    let state = input.state();
    let ident = input.expect_ident_cloned().map_err(Invalid::from)?;
    match ident.to_ascii_lowercase().as_str() {
        "repeat-x" => return Ok((true, false)),
        "repeat-y" => return Ok((false, true)),
        _ => input.reset(&state),
    }
    let x = keyword(input)?.ok_or(Invalid)?;
    let y = match input.try_parse(|i| keyword(i)) {
        Ok(Some(y)) => y,
        Ok(None) => return Err(Invalid),
        Err(_) => x,
    };
    Ok((x, y))
}

/// `background-position`: one or two values, each a keyword or a length
/// or percentage, or three or four values, keywords each followed by an
/// optional offset from that edge (`right 10px bottom 5px`). Keywords
/// become percentages.
fn parse_background_position(input: &mut Parser<'_>) -> Parsed<(PositionOffset, PositionOffset)> {
    #[derive(Clone, Copy, PartialEq)]
    enum Part {
        Value(Dim),
        /// An edge keyword: its axis (`Some(true)` for horizontal, `None`
        /// for `center`) and whether it is the far edge.
        Edge(Option<bool>, bool),
    }
    let part = |input: &mut Parser<'_>| -> Parsed<Part> {
        if let Ok(dim) = input.try_parse(|i| Dim::parse(i, false, true)) {
            return Ok(Part::Value(dim));
        }
        let ident = input.expect_ident().map_err(Invalid::from)?;
        Ok(match ident.to_ascii_lowercase().as_str() {
            "left" => Part::Edge(Some(true), false),
            "right" => Part::Edge(Some(true), true),
            "top" => Part::Edge(Some(false), false),
            "bottom" => Part::Edge(Some(false), true),
            "center" => Part::Edge(None, false),
            _ => return Err(Invalid),
        })
    };
    let mut parts = vec![part(input)?];
    while parts.len() < 4 {
        match input.try_parse(|i| part(i)) {
            Ok(found) => parts.push(found),
            Err(_) => break,
        }
    }
    let start = |dim| PositionOffset {
        length: dim,
        from_end: false,
    };
    let center = start(Dim::Percent(0.5));
    // A keyword alone, as a position on its axis.
    let edge = |far: bool, axis: Option<bool>| match axis {
        None => center,
        Some(_) => start(Dim::Percent(if far { 1.0 } else { 0.0 })),
    };
    let horizontal = |p: Part| matches!(p, Part::Edge(Some(true), _));
    let vertical = |p: Part| matches!(p, Part::Edge(Some(false), _));
    let as_offset = |p: Part| match p {
        Part::Value(dim) => start(dim),
        Part::Edge(axis, far) => edge(far, axis),
    };
    match parts[..] {
        [one] => Ok(if vertical(one) {
            (center, as_offset(one))
        } else {
            (as_offset(one), center)
        }),
        [a, b] => {
            if (horizontal(a) && horizontal(b)) || (vertical(a) && vertical(b)) {
                return Err(Invalid);
            }
            if vertical(a) || horizontal(b) {
                if matches!(a, Part::Value(_)) || matches!(b, Part::Value(_)) {
                    // A length is only allowed after a horizontal keyword
                    // or before a vertical one.
                    return Err(Invalid);
                }
                Ok((as_offset(b), as_offset(a)))
            } else {
                Ok((as_offset(a), as_offset(b)))
            }
        }
        _ => {
            // Keywords, each with an optional offset from its edge.
            let mut x = None;
            let mut y = None;
            let mut iter = parts.into_iter().peekable();
            while let Some(part) = iter.next() {
                let Part::Edge(axis, far) = part else {
                    return Err(Invalid);
                };
                let offset = match iter.peek() {
                    Some(&Part::Value(dim)) => {
                        iter.next();
                        if axis.is_none() {
                            return Err(Invalid);
                        }
                        Some(PositionOffset {
                            length: dim,
                            from_end: far,
                        })
                    }
                    _ => None,
                };
                let value = offset.unwrap_or_else(|| edge(far, axis));
                let slot = match axis {
                    Some(true) => &mut x,
                    Some(false) => &mut y,
                    None if x.is_none() => &mut x,
                    None => &mut y,
                };
                if slot.replace(value).is_some() {
                    return Err(Invalid);
                }
            }
            Ok((x.unwrap_or(center), y.unwrap_or(center)))
        }
    }
}

/// `background-size`: `cover`, `contain`, or a width and height
/// (lengths, percentages or `auto`).
fn parse_background_size(input: &mut Parser<'_>) -> Parsed<BackgroundSizeValue> {
    if input
        .try_parse(|i| i.expect_ident_matching("cover"))
        .is_ok()
    {
        return Ok(BackgroundSizeValue::Cover);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("contain"))
        .is_ok()
    {
        return Ok(BackgroundSizeValue::Contain);
    }
    let size = |input: &mut Parser<'_>| -> Parsed<Option<Dim>> {
        match Dim::parse(input, true, false)? {
            Dim::Auto => Ok(None),
            dim => Ok(Some(dim)),
        }
    };
    let width = size(input)?;
    let height = input.try_parse(|i| size(i)).unwrap_or(None);
    Ok(BackgroundSizeValue::Explicit(width, height))
}

/// `text-decoration-line`: `none`, or any of `underline`, `overline`,
/// `line-through` and `blink` (which is ignored), each at most once.
fn parse_text_decoration_line(input: &mut Parser<'_>) -> Parsed<TextDecorationLine> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Ok(TextDecorationLine::NONE);
    }
    let mut line = TextDecorationLine::NONE;
    let (mut any, mut blink) = (false, false);
    loop {
        // An unknown keyword ends the list unconsumed: in the shorthand it
        // may be the style.
        let keyword = input.try_parse(|i| {
            let ident = i.expect_ident().map_err(Invalid::from)?;
            match ident.to_ascii_lowercase().as_str() {
                "underline" => Ok(0),
                "overline" => Ok(1),
                "line-through" => Ok(2),
                "blink" => Ok(3),
                _ => Err(Invalid),
            }
        });
        let flag = match keyword {
            Ok(0) => &mut line.underline,
            Ok(1) => &mut line.overline,
            Ok(2) => &mut line.line_through,
            Ok(_) => &mut blink,
            Err(Invalid) => break,
        };
        if *flag {
            return Err(Invalid);
        }
        *flag = true;
        any = true;
    }
    if any { Ok(line) } else { Err(Invalid) }
}

/// `text-decoration`: line, style, color and thickness in any order.
/// Lines are always drawn solid and at the font's thickness, so the style
/// and thickness are accepted but not used. Omitted parts reset.
fn parse_text_decoration(input: &mut Parser<'_>, out: &mut Vec<PropertyDeclaration>) -> Parsed<()> {
    const STYLES: [&str; 5] = ["solid", "double", "dotted", "dashed", "wavy"];
    let (mut line, mut color) = (None, None);
    let (mut style, mut thickness) = (false, false);
    loop {
        if line.is_none()
            && let Ok(v) = input.try_parse(parse_text_decoration_line)
        {
            line = Some(v);
        } else if !style
            && input
                .try_parse(|i| {
                    let ident = i.expect_ident()?;
                    let known = STYLES.iter().any(|s| ident.eq_ignore_ascii_case(s));
                    if known { Ok(()) } else { Err(Invalid) }
                })
                .is_ok()
        {
            style = true;
        } else if color.is_none()
            && let Ok(v) = input.try_parse(ColorValue::parse)
        {
            color = Some(v);
        } else if !thickness
            && input
                .try_parse(|i| {
                    if i.try_parse(|i| i.expect_ident_matching("from-font"))
                        .is_ok()
                    {
                        return Ok(());
                    }
                    Dim::parse(i, true, false).map(drop)
                })
                .is_ok()
        {
            thickness = true;
        } else {
            break;
        }
    }
    if line.is_none() && color.is_none() && !style && !thickness {
        return Err(Invalid);
    }
    out.extend([
        PropertyDeclaration::TextDecorationLine(line.unwrap_or(TextDecorationLine::NONE)),
        PropertyDeclaration::TextDecorationColor(color.unwrap_or(ColorValue::CurrentColor)),
    ]);
    Ok(())
}

/// `text-indent`: a length or percentage, optionally with `hanging` and
/// `each-line`, in any order.
fn parse_text_indent(input: &mut Parser<'_>) -> Parsed<TextIndentValue> {
    let (mut length, mut hanging, mut each_line) = (None, false, false);
    loop {
        if length.is_none()
            && let Ok(v) = input.try_parse(|i| Dim::parse(i, false, true))
        {
            length = Some(v);
        } else if !hanging
            && input
                .try_parse(|i| i.expect_ident_matching("hanging"))
                .is_ok()
        {
            hanging = true;
        } else if !each_line
            && input
                .try_parse(|i| i.expect_ident_matching("each-line"))
                .is_ok()
        {
            each_line = true;
        } else {
            break;
        }
    }
    Ok(TextIndentValue {
        length: length.ok_or(Invalid)?,
        hanging,
        each_line,
    })
}

/// `letter-spacing` / `word-spacing`: `normal` or a length, which may be
/// negative.
fn parse_spacing(input: &mut Parser<'_>) -> Parsed<Length> {
    if input
        .try_parse(|i| i.expect_ident_matching("normal"))
        .is_ok()
    {
        return Ok(Length::ZERO);
    }
    Length::parse(input)
}

/// `border-radius`: one to four horizontal radii, optionally `/` and one
/// to four vertical ones, assigned to the corners clockwise from the top
/// left.
fn parse_border_radius(input: &mut Parser<'_>, out: &mut Vec<PropertyDeclaration>) -> Parsed<()> {
    let radii = |input: &mut Parser<'_>| -> Parsed<[Dim; 4]> {
        let mut values = vec![Dim::parse(input, false, false)?];
        while values.len() < 4 {
            match input.try_parse(|i| Dim::parse(i, false, false)) {
                Ok(value) => values.push(value),
                Err(Invalid) => break,
            }
        }
        Ok(match values[..] {
            [a] => [a, a, a, a],
            [a, b] => [a, b, a, b],
            [a, b, c] => [a, b, c, b],
            [a, b, c, d] => [a, b, c, d],
            _ => unreachable!("one to four values"),
        })
    };
    let horizontal = radii(input)?;
    let vertical = if input.try_parse(|i| i.expect_delim('/')).is_ok() {
        radii(input)?
    } else {
        horizontal
    };
    for (index, corner) in Corner::ALL.into_iter().enumerate() {
        out.push(PropertyDeclaration::BorderRadius(
            corner,
            horizontal[index],
            vertical[index],
        ));
    }
    Ok(())
}

/// `content`: `normal`, `none`, or strings, counters, attributes and
/// quotes. Images are not supported.
fn parse_content(input: &mut Parser<'_>) -> Parsed<Content> {
    if input
        .try_parse(|i| i.expect_ident_matching("normal"))
        .is_ok()
    {
        return Ok(Content::Normal);
    }
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Ok(Content::None);
    }
    let mut items = Vec::new();
    while !input.is_exhausted() {
        let item = match input.next().map_err(Invalid::from)?.clone() {
            Token::QuotedString(text) => ContentItem::Text(text.as_ref().into()),
            Token::Ident(ident) => match ident.to_ascii_lowercase().as_str() {
                "open-quote" => ContentItem::OpenQuote,
                "close-quote" => ContentItem::CloseQuote,
                "no-open-quote" => ContentItem::NoOpenQuote,
                "no-close-quote" => ContentItem::NoCloseQuote,
                _ => return Err(Invalid),
            },
            Token::Function(name) => {
                let name = name.to_ascii_lowercase();
                input.parse_nested_block(|i| {
                    parse_content_function(&name, i)
                        .map_err(|_| i.new_error_for_next_token::<Invalid>())
                })?
            }
            _ => return Err(Invalid),
        };
        items.push(item);
    }
    if items.is_empty() {
        return Err(Invalid);
    }
    Ok(Content::Items(items.into()))
}

/// The arguments of `counter()`, `counters()` or `attr()` in `content`.
fn parse_content_function(name: &str, input: &mut Parser<'_>) -> Parsed<ContentItem> {
    let counter_style = |i: &mut Parser<'_>| -> Parsed<ListStyleType> {
        if i.try_parse(|i| i.expect_comma()).is_ok() {
            ListStyleType::parse(i)
        } else {
            Ok(ListStyleType::Decimal)
        }
    };
    let item = match name {
        "counter" => {
            let name = input.expect_ident_cloned().map_err(Invalid::from)?;
            ContentItem::Counter {
                name: name.as_ref().into(),
                style: counter_style(input)?,
            }
        }
        "counters" => {
            let name = input.expect_ident_cloned().map_err(Invalid::from)?;
            input.expect_comma().map_err(Invalid::from)?;
            let separator = input.expect_string_cloned().map_err(Invalid::from)?;
            ContentItem::Counters {
                name: name.as_ref().into(),
                separator: separator.as_ref().into(),
                style: counter_style(input)?,
            }
        }
        "attr" => {
            let name = input.expect_ident_cloned().map_err(Invalid::from)?;
            ContentItem::Attr(name.to_ascii_lowercase().into())
        }
        _ => return Err(Invalid),
    };
    input.expect_exhausted().map_err(Invalid::from)?;
    Ok(item)
}

/// `counter-reset`, `counter-increment`, `counter-set`: `none`, or counter
/// names, each optionally followed by an integer (else `default`).
fn parse_counter_changes(input: &mut Parser<'_>, default: i32) -> Parsed<CounterChanges> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Ok(Arc::from([]));
    }
    let mut changes = Vec::new();
    while !input.is_exhausted() {
        let name = input.expect_ident_cloned().map_err(Invalid::from)?;
        if matches!(
            name.to_ascii_lowercase().as_str(),
            "none" | "initial" | "inherit" | "unset" | "default"
        ) {
            return Err(Invalid);
        }
        let value = input.try_parse(|i| i.expect_integer()).unwrap_or(default);
        changes.push((Arc::from(name.as_ref()), value));
    }
    if changes.is_empty() {
        return Err(Invalid);
    }
    Ok(changes.into())
}

/// `orphans` / `widows`: a positive integer.
fn parse_line_count(input: &mut Parser<'_>) -> Parsed<u32> {
    let count = input.expect_integer().map_err(Invalid::from)?;
    u32::try_from(count)
        .ok()
        .filter(|&count| count >= 1)
        .ok_or(Invalid)
}

/// `list-style`: only the marker type is used.
fn parse_list_style(input: &mut Parser<'_>) -> ListStyleType {
    let mut style = ListStyleType::Disc;
    while let Ok(token) = input.next() {
        if let Token::Ident(ident) = token
            && let Some(found) = ListStyleType::from_keyword(ident)
        {
            style = found;
        }
    }
    style
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::values::Color;
    use PropertyDeclaration as D;

    fn parse(name: &str, value: &str) -> Parsed<Vec<PropertyDeclaration>> {
        let mut parser = Parser::new(value);
        let mut out = Vec::new();
        parse_declaration(name, &mut parser, &mut out)?;
        Ok(out)
    }

    #[test]
    fn parses_border_radius() {
        let pt = |v: f32| Dim::Length(Length::Pt(v * 0.75));
        assert_eq!(
            parse("border-radius", "4px 8px / 2px"),
            Ok(vec![
                D::BorderRadius(Corner::TopLeft, pt(4.0), pt(2.0)),
                D::BorderRadius(Corner::TopRight, pt(8.0), pt(2.0)),
                D::BorderRadius(Corner::BottomRight, pt(4.0), pt(2.0)),
                D::BorderRadius(Corner::BottomLeft, pt(8.0), pt(2.0)),
            ])
        );
        assert_eq!(
            parse("border-bottom-left-radius", "50%"),
            Ok(vec![D::BorderRadius(
                Corner::BottomLeft,
                Dim::Percent(0.5),
                Dim::Percent(0.5)
            )])
        );
        assert!(parse("border-radius", "-1px").is_err());
        assert!(parse("border-radius", "1px / ").is_err());
    }

    #[test]
    fn parses_content() {
        let text = |t: &str| ContentItem::Text(t.into());
        assert_eq!(
            parse(
                "content",
                r#"counter(sec) ". " counters(item, ".", lower-roman) attr(HREF)"#
            ),
            Ok(vec![D::Content(Content::Items(
                vec![
                    ContentItem::Counter {
                        name: "sec".into(),
                        style: ListStyleType::Decimal
                    },
                    text(". "),
                    ContentItem::Counters {
                        name: "item".into(),
                        separator: ".".into(),
                        style: ListStyleType::LowerRoman
                    },
                    ContentItem::Attr("href".into()),
                ]
                .into()
            ))])
        );
        assert_eq!(
            parse("content", "open-quote close-quote"),
            Ok(vec![D::Content(Content::Items(
                vec![ContentItem::OpenQuote, ContentItem::CloseQuote].into()
            ))])
        );
        assert_eq!(
            parse("content", "none"),
            Ok(vec![D::Content(Content::None)])
        );
        assert_eq!(
            parse("content", "normal"),
            Ok(vec![D::Content(Content::Normal)])
        );
        assert!(parse("content", "url(a.png)").is_err());
        assert!(parse("content", "counter(a, banana)").is_err());
        assert!(parse("content", "\"a\" none").is_err());
    }

    #[test]
    fn parses_counter_changes() {
        let changes = |list: &[(&str, i32)]| -> CounterChanges {
            list.iter()
                .map(|(name, value)| (Arc::from(*name), *value))
                .collect()
        };
        assert_eq!(
            parse("counter-reset", "sec sub 3"),
            Ok(vec![D::CounterReset(changes(&[("sec", 0), ("sub", 3)]))])
        );
        assert_eq!(
            parse("counter-increment", "item -2 row"),
            Ok(vec![D::CounterIncrement(changes(&[
                ("item", -2),
                ("row", 1)
            ]))])
        );
        assert_eq!(
            parse("counter-set", "none"),
            Ok(vec![D::CounterSet(changes(&[]))])
        );
        assert!(parse("counter-reset", "3").is_err());
    }

    #[test]
    fn parses_keywords_case_insensitively() {
        assert_eq!(
            parse("DISPLAY", "Table-Cell"),
            Ok(vec![D::Display(Display::TableCell)])
        );
        assert_eq!(
            parse("display", "flex"),
            Ok(vec![D::Display(Display::Block)])
        );
        assert!(parse("display", "banana").is_err());
        assert!(parse("unknown-property", "1").is_err());
    }

    #[test]
    fn rejects_trailing_garbage() {
        assert!(parse("color", "red blue").is_err());
        assert!(parse("width", "10px 20px").is_err());
    }

    #[test]
    fn margin_shorthand_expands_like_css() {
        let pt = |v| Dim::Length(Length::Pt(v));
        assert_eq!(
            parse("margin", "1pt 2pt 3pt"),
            Ok(vec![
                D::Margin(Side::Top, pt(1.0)),
                D::Margin(Side::Right, pt(2.0)),
                D::Margin(Side::Bottom, pt(3.0)),
                D::Margin(Side::Left, pt(2.0)),
            ])
        );
        assert_eq!(
            parse("margin", "0 auto").unwrap()[1],
            D::Margin(Side::Right, Dim::Auto)
        );
        assert!(parse("padding", "auto").is_err());
        assert!(parse("margin", "1pt 2pt 3pt 4pt 5pt").is_err());
    }

    #[test]
    fn border_shorthand_accepts_any_order_and_resets() {
        let red = ColorValue::Color(Color::rgb(255, 0, 0));
        assert_eq!(
            parse("border-bottom", "solid red 2pt"),
            Ok(vec![
                D::BorderWidth(Side::Bottom, Length::Pt(2.0)),
                D::BorderStyle(Side::Bottom, BorderStyle::Solid),
                D::BorderColor(Side::Bottom, red),
            ])
        );
        let all = parse("border", "1px solid").unwrap();
        assert_eq!(all.len(), 12);
        assert_eq!(all[2], D::BorderColor(Side::Top, ColorValue::CurrentColor));
        assert_eq!(
            parse("border", "none").unwrap()[0],
            D::BorderWidth(Side::Top, BORDER_WIDTH_MEDIUM)
        );
        assert!(parse("border", "").is_err());
    }

    #[test]
    fn font_shorthand() {
        let decls = parse(
            "font",
            "italic bold 12pt/1.5 \"Noto Sans\", Arial Black, sans-serif",
        )
        .unwrap();
        assert_eq!(
            decls,
            vec![
                D::FontStyle(true),
                D::FontWeight(FontWeight::Weight(700.0)),
                D::FontSize(FontSize::Length(Length::Pt(12.0))),
                D::LineHeight(LineHeight::Number(1.5)),
                D::FontFamily(
                    vec![
                        FontFamilyName::Named("Noto Sans".into()),
                        FontFamilyName::Named("Arial Black".into()),
                        FontFamilyName::Generic(GenericFamily::SansSerif),
                    ]
                    .into()
                ),
            ]
        );
        assert!(parse("font", "bold").is_err());
    }

    #[test]
    fn css_wide_keywords_apply_to_every_longhand() {
        assert_eq!(
            parse("color", "inherit"),
            Ok(vec![D::CssWide(LonghandId::Color, CssWide::Inherit)])
        );
        assert_eq!(parse("margin", "initial").unwrap().len(), 4);
        assert_eq!(parse("border", "unset").unwrap().len(), 12);
    }

    #[test]
    fn background_shorthand_sets_every_part() {
        let pct = |p| PositionOffset {
            length: Dim::Percent(p),
            from_end: false,
        };
        let all = |color: Color,
                   image: Option<&str>,
                   repeat: (bool, bool),
                   position: (PositionOffset, PositionOffset),
                   size: BackgroundSizeValue| {
            Ok(vec![
                D::BackgroundColor(ColorValue::Color(color)),
                D::BackgroundImage(image.map(Arc::from)),
                D::BackgroundRepeat(repeat.0, repeat.1),
                D::BackgroundPosition(position.0, position.1),
                D::BackgroundSize(size),
            ])
        };
        let auto = BackgroundSizeValue::Explicit(None, None);
        assert_eq!(
            parse("background", "#eee"),
            all(
                Color::rgb(238, 238, 238),
                None,
                (true, true),
                (pct(0.0), pct(0.0)),
                auto
            )
        );
        assert_eq!(
            parse("background", "url(x.png) no-repeat right top / contain red"),
            all(
                Color::rgb(255, 0, 0),
                Some("x.png"),
                (false, false),
                (pct(1.0), pct(0.0)),
                BackgroundSizeValue::Contain
            )
        );
        // Unsupported parts do not lose the color.
        assert_eq!(
            parse("background", "linear-gradient(red, blue) fixed #fff"),
            all(Color::WHITE, None, (true, true), (pct(0.0), pct(0.0)), auto)
        );
    }

    #[test]
    fn background_longhands() {
        let px = |v: f32| Dim::Length(Length::Pt(v * 0.75));
        let at = |length, from_end| PositionOffset { length, from_end };
        assert_eq!(
            parse("background-image", "url('logo.png')"),
            Ok(vec![D::BackgroundImage(Some("logo.png".into()))])
        );
        assert_eq!(
            parse("background-repeat", "repeat-y"),
            Ok(vec![D::BackgroundRepeat(false, true)])
        );
        assert_eq!(
            parse("background-repeat", "repeat no-repeat"),
            Ok(vec![D::BackgroundRepeat(true, false)])
        );
        assert_eq!(
            parse("background-position", "center bottom"),
            Ok(vec![D::BackgroundPosition(
                at(Dim::Percent(0.5), false),
                at(Dim::Percent(1.0), false)
            )])
        );
        assert_eq!(
            parse("background-position", "10px bottom"),
            Ok(vec![D::BackgroundPosition(
                at(px(10.0), false),
                at(Dim::Percent(1.0), false)
            )])
        );
        assert_eq!(
            parse("background-position", "right 4px bottom 10%"),
            Ok(vec![D::BackgroundPosition(
                at(px(4.0), true),
                at(Dim::Percent(0.1), true)
            )])
        );
        assert_eq!(
            parse("background-position", "top 5px center"),
            Ok(vec![D::BackgroundPosition(
                at(Dim::Percent(0.5), false),
                at(px(5.0), false)
            )])
        );
        assert_eq!(
            parse("background-position", "center"),
            Ok(vec![D::BackgroundPosition(
                at(Dim::Percent(0.5), false),
                at(Dim::Percent(0.5), false)
            )])
        );
        assert!(parse("background-position", "left right").is_err());
        assert!(parse("background-position", "center 4px top").is_err());
        assert!(parse("background-position", "bottom 10px").is_err());
        assert_eq!(
            parse("background-size", "50% auto"),
            Ok(vec![D::BackgroundSize(BackgroundSizeValue::Explicit(
                Some(Dim::Percent(0.5)),
                None
            ))])
        );
    }

    #[test]
    fn legacy_page_break_aliases() {
        assert_eq!(
            parse("page-break-before", "always"),
            Ok(vec![D::BreakBefore(Break::Page)])
        );
        assert_eq!(
            parse("break-after", "page"),
            Ok(vec![D::BreakAfter(Break::Page)])
        );
    }

    #[test]
    fn break_inside_orphans_and_widows() {
        assert_eq!(
            parse("page-break-inside", "avoid"),
            Ok(vec![D::BreakInside(BreakInside::Avoid)])
        );
        assert_eq!(
            parse("break-inside", "avoid-page"),
            Ok(vec![D::BreakInside(BreakInside::Avoid)])
        );
        assert!(parse("break-inside", "page").is_err());
        assert_eq!(parse("orphans", "3"), Ok(vec![D::Orphans(3)]));
        assert!(parse("widows", "0").is_err());
        assert!(parse("widows", "1.5").is_err());
    }

    #[test]
    fn sizes_and_vertical_align() {
        assert_eq!(parse("max-width", "none"), Ok(vec![D::MaxWidth(Dim::Auto)]));
        assert_eq!(
            parse("max-width", "100%"),
            Ok(vec![D::MaxWidth(Dim::Percent(1.0))])
        );
        assert!(parse("max-height", "auto").is_err());
        assert_eq!(parse("height", "auto"), Ok(vec![D::Height(Dim::Auto)]));
        assert!(parse("height", "-1px").is_err());
        assert_eq!(
            parse("vertical-align", "text-top"),
            Ok(vec![D::VerticalAlign(VerticalAlign::TextTop)])
        );
    }

    #[test]
    fn text_decoration_shorthand() {
        let red = ColorValue::Color(Color::rgb(255, 0, 0));
        let under = TextDecorationLine {
            underline: true,
            ..TextDecorationLine::NONE
        };
        assert_eq!(
            parse("text-decoration", "red wavy underline 2px"),
            Ok(vec![
                D::TextDecorationLine(under),
                D::TextDecorationColor(red)
            ])
        );
        assert_eq!(
            parse("text-decoration", "none"),
            Ok(vec![
                D::TextDecorationLine(TextDecorationLine::NONE),
                D::TextDecorationColor(ColorValue::CurrentColor),
            ])
        );
        assert_eq!(
            parse("text-decoration-line", "line-through UNDERLINE"),
            Ok(vec![D::TextDecorationLine(TextDecorationLine {
                line_through: true,
                ..under
            })])
        );
        assert!(parse("text-decoration-line", "underline underline").is_err());
        assert!(parse("text-decoration-line", "none underline").is_err());
        assert!(parse("text-decoration", "").is_err());
    }

    #[test]
    fn text_indent_spacing_and_transform() {
        assert_eq!(
            parse("text-indent", "hanging -2em"),
            Ok(vec![D::TextIndent(TextIndentValue {
                length: Dim::Length(Length::Em(-2.0)),
                hanging: true,
                each_line: false,
            })])
        );
        assert!(parse("text-indent", "hanging").is_err());
        assert_eq!(
            parse("letter-spacing", "normal"),
            Ok(vec![D::LetterSpacing(Length::ZERO)])
        );
        assert_eq!(
            parse("word-spacing", "-1pt"),
            Ok(vec![D::WordSpacing(Length::Pt(-1.0))])
        );
        assert_eq!(
            parse("text-transform", "Uppercase"),
            Ok(vec![D::TextTransform(TextTransform::Uppercase)])
        );
        assert_eq!(
            parse("min-height", "auto"),
            Ok(vec![D::MinHeight(Dim::Auto)])
        );
        assert!(parse("min-height", "-1px").is_err());
    }

    #[test]
    fn every_longhand_is_listed_once() {
        let all: Vec<_> = LonghandId::all().collect();
        let unique: std::collections::HashSet<_> = all.iter().collect();
        assert_eq!(all.len(), unique.len());
        assert_eq!(all.len(), 40 + 4 * 5 + 4);
    }
}
