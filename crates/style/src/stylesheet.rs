//! Stylesheet parsing: style rules, `@media`, `@page`, `@import`,
//! `@font-face` and `style=""`.

use cssparser::{
    AtRuleParser, BasicParseErrorKind, CowRcStr, DeclarationParser, Delimiter, ParseError, Parser,
    ParserState, QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser, Token,
    parse_important,
};
use presswork_dom::resource::Url;
use selectors::SelectorList;

use crate::properties::{PropertyDeclaration, parse_declaration};
use crate::selector::{Simple, parse_selector_list};
use crate::values::{Invalid, Length, PT_PER_PX, Side, Sides};

/// One longhand declaration and whether it is `!important`.
#[derive(Debug, Clone, PartialEq)]
pub struct Declaration {
    pub property: PropertyDeclaration,
    pub important: bool,
}

/// A style rule: `selectors { declarations }`.
#[derive(Debug, Clone)]
pub(crate) struct StyleRule {
    pub selectors: SelectorList<Simple>,
    pub declarations: Vec<Declaration>,
}

/// The `size` descriptor of `@page`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PageSizeValue {
    /// A named size such as `A4` or `letter`.
    pub name: Option<String>,
    /// Explicit width and height in points.
    pub dimensions: Option<(f32, f32)>,
    /// `Some(true)` for `landscape`, `Some(false)` for `portrait`.
    pub landscape: Option<bool>,
}

/// The descriptors of the document's `@page` rules.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PageRule {
    pub size: Option<PageSizeValue>,
    /// Page margins in points, where given.
    pub margin: Sides<Option<f32>>,
}

impl PageRule {
    /// Overrides this rule's descriptors with those set in `later`.
    pub(crate) fn merge(&mut self, later: &PageRule) {
        if later.size.is_some() {
            self.size.clone_from(&later.size);
        }
        for side in Side::ALL {
            if let Some(margin) = later.margin.get(side) {
                self.margin.set(side, Some(margin));
            }
        }
    }
}

/// An `@font-face` rule.
#[derive(Debug, Clone, PartialEq)]
pub struct FontFace {
    pub family: String,
    /// In order of preference.
    pub sources: Vec<FontSource>,
    /// `None` for a weight range (a variable font): the font's own
    /// weight is used.
    pub weight: Option<f32>,
    pub italic: bool,
    /// The stylesheet's URL, which relative source URLs resolve against;
    /// `None` for a `<style>` element (the document's URL).
    pub base: Option<Url>,
}

/// One entry of an `@font-face` `src` list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontSource {
    /// A font file, with its `format()` hint, lowercase.
    Url { url: String, format: Option<String> },
    /// `local()`: an installed font, by full name.
    Local(String),
}

impl FontFace {
    fn new() -> Self {
        Self {
            family: String::new(),
            sources: Vec::new(),
            weight: Some(400.0),
            italic: false,
            base: None,
        }
    }
}

/// An `@import` rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Import {
    pub url: String,
    /// Whether its media query list matches print.
    pub applies: bool,
}

/// A parsed stylesheet.
#[derive(Debug, Clone, Default)]
pub struct Stylesheet {
    pub(crate) rules: Vec<StyleRule>,
    pub(crate) page: PageRule,
    pub(crate) imports: Vec<Import>,
    pub(crate) font_faces: Vec<FontFace>,
}

impl Stylesheet {
    /// Makes the URLs in declarations absolute, relative to `base`, the
    /// stylesheet's own URL: they would otherwise resolve against the
    /// document.
    pub(crate) fn resolve_urls(&mut self, base: &Url) {
        for rule in &mut self.rules {
            for declaration in &mut rule.declarations {
                if let PropertyDeclaration::BackgroundImage(Some(url)) = &mut declaration.property
                    && let Ok(absolute) = base.join(url.trim())
                {
                    *url = absolute.as_str().into();
                }
            }
        }
    }

    /// Parses a stylesheet. Invalid rules and declarations are skipped, as
    /// CSS error handling requires.
    ///
    /// `@media` rules apply when they match print media. `@import` rules
    /// are collected for the cascade to load; other at-rules are ignored.
    pub fn parse(css: &str) -> Self {
        let mut parser = Parser::new(css);
        let mut sheet = Stylesheet::default();
        let rules = StyleSheetParser::new(&mut parser, &mut RuleParser { top_level: true })
            .flatten()
            .collect();
        sheet.extend(rules);
        sheet
    }

    fn extend(&mut self, rules: Vec<Rule>) {
        for rule in rules {
            match rule {
                Rule::Style(rule) => self.rules.push(rule),
                Rule::Nested(rules) => self.extend(rules),
                Rule::Page(page) => self.page.merge(&page),
                // Imports after other rules are invalid.
                Rule::Import(import) if self.rules.is_empty() && self.font_faces.is_empty() => {
                    self.imports.push(import);
                }
                Rule::Import(_) => {}
                Rule::FontFace(face) => {
                    if !face.family.is_empty() && !face.sources.is_empty() {
                        self.font_faces.push(face);
                    }
                }
            }
        }
    }
}

/// Parses the contents of a `style=""` attribute.
pub fn parse_style_attribute(css: &str) -> Vec<Declaration> {
    let input = css;
    parse_declarations(&mut Parser::new(input))
}

/// Whether a media query list (as in `@media` or `<style media>`) matches
/// print output.
pub fn media_matches(query: &str) -> bool {
    let input = query;
    evaluate_media(&mut Parser::new(input))
}

enum Rule {
    Style(StyleRule),
    /// The rules inside a conditional group rule that applies.
    Nested(Vec<Rule>),
    Page(PageRule),
    Import(Import),
    FontFace(FontFace),
}

enum AtRulePrelude {
    /// `@media` or `@supports`, with whether its contents apply.
    Conditional(bool),
    /// `@page`, with whether it has no page selector.
    Page(bool),
    Import(Import),
    FontFace,
}

struct RuleParser {
    /// Not inside a conditional rule, where `@import` is invalid.
    top_level: bool,
}

/// Parses the rules inside a conditional group rule.
fn parse_nested_rules(input: &mut Parser<'_>) -> Vec<Rule> {
    StyleSheetParser::new(input, &mut RuleParser { top_level: false })
        .flatten()
        .collect()
}

fn skip_rest(input: &mut Parser<'_>) {
    while input.next().is_ok() {}
}

impl<'i> QualifiedRuleParser<'i> for RuleParser {
    type Prelude = SelectorList<Simple>;
    type QualifiedRule = Rule;
    type Error = Invalid;

    fn parse_prelude(
        &mut self,
        input: &mut Parser<'i>,
    ) -> Result<Self::Prelude, ParseError<Self::Error>> {
        parse_selector_list(input).map_err(ParseError::custom)
    }

    fn parse_block(
        &mut self,
        selectors: Self::Prelude,
        _start: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<Rule, ParseError<Self::Error>> {
        Ok(Rule::Style(StyleRule {
            selectors,
            declarations: parse_declarations(input),
        }))
    }
}

impl<'i> AtRuleParser<'i> for RuleParser {
    type Prelude = AtRulePrelude;
    type AtRule = Rule;
    type Error = Invalid;

    fn parse_prelude(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
    ) -> Result<Self::Prelude, ParseError<Self::Error>> {
        match name.to_ascii_lowercase().as_str() {
            "media" => Ok(AtRulePrelude::Conditional(evaluate_media(input))),
            "supports" => {
                // Feature queries are assumed to pass; unsupported
                // declarations inside are dropped individually anyway.
                skip_rest(input);
                Ok(AtRulePrelude::Conditional(true))
            }
            "page" => {
                let plain = input.is_exhausted();
                skip_rest(input);
                Ok(AtRulePrelude::Page(plain))
            }
            "import" if self.top_level => {
                let url = input.expect_url_or_string()?.as_ref().to_owned();
                // Cascade layers and import conditions are not supported:
                // the import applies unconditionally, in place.
                let _ = input.try_parse(|i| i.expect_ident_matching("layer"));
                let _ = input.try_parse(|i| {
                    i.expect_function_matching("layer")?;
                    i.parse_nested_block(|i| {
                        skip_rest(i);
                        Ok::<_, ParseError<Invalid>>(())
                    })
                });
                let _ = input.try_parse(|i| {
                    i.expect_function_matching("supports")?;
                    i.parse_nested_block(|i| {
                        skip_rest(i);
                        Ok::<_, ParseError<Invalid>>(())
                    })
                });
                let applies = evaluate_media(input);
                Ok(AtRulePrelude::Import(Import { url, applies }))
            }
            "font-face" => {
                skip_rest(input);
                Ok(AtRulePrelude::FontFace)
            }
            _ => Err(ParseError::from_basic_kind(
                BasicParseErrorKind::AtRuleInvalid,
            )),
        }
    }

    fn parse_block(
        &mut self,
        prelude: Self::Prelude,
        _start: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<Rule, ParseError<Self::Error>> {
        Ok(match prelude {
            AtRulePrelude::Conditional(true) => Rule::Nested(parse_nested_rules(input)),
            AtRulePrelude::Page(true) => Rule::Page(parse_page_rule(input)),
            AtRulePrelude::FontFace => Rule::FontFace(parse_font_face(input)),
            AtRulePrelude::Conditional(false)
            | AtRulePrelude::Page(false)
            | AtRulePrelude::Import(_) => {
                skip_rest(input);
                Rule::Nested(Vec::new())
            }
        })
    }

    fn rule_without_block(
        &mut self,
        prelude: Self::Prelude,
        _start: &ParserState,
    ) -> Result<Rule, ()> {
        match prelude {
            AtRulePrelude::Import(import) => Ok(Rule::Import(import)),
            _ => Err(()),
        }
    }
}

/// Parses the descriptors of an `@font-face` rule.
fn parse_font_face(input: &mut Parser<'_>) -> FontFace {
    let mut parser = FontFaceParser(FontFace::new());
    for _ in RuleBodyParser::new(input, &mut parser) {}
    parser.0
}

struct FontFaceParser(FontFace);

impl<'i> DeclarationParser<'i> for FontFaceParser {
    type Declaration = ();
    type Error = Invalid;

    fn parse_value(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
        _start: &ParserState,
    ) -> Result<(), ParseError<Invalid>> {
        let face = &mut self.0;
        match name.to_ascii_lowercase().as_str() {
            "font-family" => {
                face.family = match input.try_parse(|i| i.expect_string_cloned()) {
                    Ok(name) => name.as_ref().to_owned(),
                    Err(_) => {
                        let mut words = Vec::new();
                        while let Ok(word) = input.expect_ident_cloned() {
                            words.push(word.as_ref().to_owned());
                        }
                        words.join(" ")
                    }
                };
            }
            "src" => {
                face.sources = input
                    .parse_comma_separated_ignoring_errors(parse_font_source)
                    .into_iter()
                    .collect();
            }
            "font-weight" => {
                let weight = |i: &mut Parser<'i>| -> Result<f32, ParseError<Invalid>> {
                    if i.try_parse(|i| i.expect_ident_matching("normal")).is_ok() {
                        return Ok(400.0);
                    }
                    if i.try_parse(|i| i.expect_ident_matching("bold")).is_ok() {
                        return Ok(700.0);
                    }
                    Ok(i.expect_number()?)
                };
                let first = weight(input)?;
                face.weight = match input.try_parse(weight) {
                    Ok(_) => None,
                    Err(_) => Some(first),
                };
            }
            "font-style" => {
                let style = input.expect_ident_cloned()?;
                face.italic = matches!(style.to_ascii_lowercase().as_str(), "italic" | "oblique");
                skip_rest(input);
            }
            _ => skip_rest(input),
        }
        Ok(())
    }
}

impl AtRuleParser<'_> for FontFaceParser {
    type Prelude = ();
    type AtRule = ();
    type Error = Invalid;
}

impl QualifiedRuleParser<'_> for FontFaceParser {
    type Prelude = ();
    type QualifiedRule = ();
    type Error = Invalid;
}

impl RuleBodyItemParser<'_, (), Invalid> for FontFaceParser {
    fn parse_declarations(&self) -> bool {
        true
    }

    fn parse_qualified(&self) -> bool {
        false
    }
}

/// One `src` entry: `url(...) format(...)` or `local(...)`. Unknown
/// trailing functions such as `tech()` are ignored.
fn parse_font_source<'i>(input: &mut Parser<'i>) -> Result<FontSource, ParseError<Invalid>> {
    if input
        .try_parse(|i| i.expect_function_matching("local"))
        .is_ok()
    {
        let name =
            input.parse_nested_block(|i| match i.try_parse(|i| i.expect_string_cloned()) {
                Ok(name) => Ok(name.as_ref().to_owned()),
                Err(_) => {
                    let mut words = Vec::new();
                    while let Ok(word) = i.expect_ident_cloned() {
                        words.push(word.as_ref().to_owned());
                    }
                    Ok::<_, ParseError<Invalid>>(words.join(" "))
                }
            })?;
        return Ok(FontSource::Local(name));
    }
    let url = input.expect_url_or_string()?.as_ref().to_owned();
    let mut format = None;
    while !input.is_exhausted() {
        let function = input.expect_function()?.as_ref().to_ascii_lowercase();
        let value = input.parse_nested_block(|i| {
            let value = match i.next()? {
                Token::QuotedString(s) | Token::Ident(s) => s.as_ref().to_ascii_lowercase(),
                _ => String::new(),
            };
            skip_rest(i);
            Ok::<_, ParseError<Invalid>>(value)
        })?;
        if function == "format" {
            format = Some(value);
        }
    }
    Ok(FontSource::Url { url, format })
}

/// Parses a declaration block: the body of a style rule or a `style=""`.
fn parse_declarations(input: &mut Parser<'_>) -> Vec<Declaration> {
    RuleBodyParser::new(input, &mut DeclarationListParser)
        .flatten()
        .flatten()
        .collect()
}

struct DeclarationListParser;

impl<'i> DeclarationParser<'i> for DeclarationListParser {
    type Declaration = Vec<Declaration>;
    type Error = Invalid;

    fn parse_value(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
        _start: &ParserState,
    ) -> Result<Vec<Declaration>, ParseError<Invalid>> {
        let mut properties = Vec::new();
        input.parse_until_before(Delimiter::Bang, |input| {
            parse_declaration(&name, input, &mut properties).map_err(ParseError::custom)
        })?;
        let important = input.try_parse(parse_important).is_ok();
        input.expect_exhausted()?;
        Ok(properties
            .into_iter()
            .map(|property| Declaration {
                property,
                important,
            })
            .collect())
    }
}

// Declaration blocks contain no nested rules; these use the defaults,
// which reject everything.
impl AtRuleParser<'_> for DeclarationListParser {
    type Prelude = ();
    type AtRule = Vec<Declaration>;
    type Error = Invalid;
}

impl QualifiedRuleParser<'_> for DeclarationListParser {
    type Prelude = ();
    type QualifiedRule = Vec<Declaration>;
    type Error = Invalid;
}

impl RuleBodyItemParser<'_, Vec<Declaration>, Invalid> for DeclarationListParser {
    fn parse_declarations(&self) -> bool {
        true
    }

    fn parse_qualified(&self) -> bool {
        false
    }
}

/// Parses the descriptors of an `@page` rule. Margin boxes and descriptors
/// other than `size` and `margin` are ignored.
fn parse_page_rule(input: &mut Parser<'_>) -> PageRule {
    let mut parser = PageDescriptorParser(PageRule::default());
    for _ in RuleBodyParser::new(input, &mut parser) {}
    parser.0
}

struct PageDescriptorParser(PageRule);

/// A page length in points; `em` is relative to the default 12pt.
fn page_length(input: &mut Parser<'_>) -> Result<f32, Invalid> {
    Length::parse_non_negative(input).map(|l| l.to_pt(12.0, 12.0))
}

impl<'i> DeclarationParser<'i> for PageDescriptorParser {
    type Declaration = ();
    type Error = Invalid;

    fn parse_value(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
        _start: &ParserState,
    ) -> Result<(), ParseError<Invalid>> {
        let name = name.to_ascii_lowercase();
        let side = |name: &str| match name {
            "margin-top" => Some(Side::Top),
            "margin-right" => Some(Side::Right),
            "margin-bottom" => Some(Side::Bottom),
            "margin-left" => Some(Side::Left),
            _ => None,
        };
        if name == "size" {
            let size = parse_page_size(input).map_err(ParseError::custom)?;
            input.expect_exhausted()?;
            self.0.size = Some(size);
        } else if name == "margin" {
            let mut values = vec![page_length(input).map_err(ParseError::custom)?];
            while values.len() < 4
                && let Ok(value) = input.try_parse(page_length)
            {
                values.push(value);
            }
            input.expect_exhausted()?;
            let (top, right, bottom, left) = match values[..] {
                [a] => (a, a, a, a),
                [a, b] => (a, b, a, b),
                [a, b, c] => (a, b, c, b),
                _ => (values[0], values[1], values[2], values[3]),
            };
            self.0.margin = Sides {
                top: Some(top),
                right: Some(right),
                bottom: Some(bottom),
                left: Some(left),
            };
        } else if let Some(side) = side(&name) {
            let value = page_length(input).map_err(ParseError::custom)?;
            input.expect_exhausted()?;
            self.0.margin.set(side, Some(value));
        } else {
            return Err(ParseError::custom(Invalid));
        }
        Ok(())
    }
}

impl AtRuleParser<'_> for PageDescriptorParser {
    type Prelude = ();
    type AtRule = ();
    type Error = Invalid;
}

impl QualifiedRuleParser<'_> for PageDescriptorParser {
    type Prelude = ();
    type QualifiedRule = ();
    type Error = Invalid;
}

impl RuleBodyItemParser<'_, (), Invalid> for PageDescriptorParser {
    fn parse_declarations(&self) -> bool {
        true
    }

    fn parse_qualified(&self) -> bool {
        false
    }
}

/// `size: A4 landscape`, `size: 210mm 297mm`, `size: landscape`, ...
fn parse_page_size(input: &mut Parser<'_>) -> Result<PageSizeValue, Invalid> {
    let mut size = PageSizeValue::default();
    if let Ok(width) = input.try_parse(page_length) {
        let height = input.try_parse(page_length).unwrap_or(width);
        size.dimensions = Some((width, height));
        return Ok(size);
    }
    while let Ok(ident) = input.try_parse(|i| i.expect_ident_cloned()) {
        match ident.to_ascii_lowercase().as_str() {
            "landscape" => size.landscape = Some(true),
            "portrait" => size.landscape = Some(false),
            "auto" => {}
            name if size.name.is_none() => size.name = Some(name.to_owned()),
            _ => return Err(Invalid),
        }
    }
    Ok(size)
}

/// The viewport width media features are evaluated against: an A4 page.
const MEDIA_WIDTH_PX: f32 = 595.28 / PT_PER_PX;

/// Evaluates a media query list for print output, consuming the input.
///
/// Media types are honored (`print` and `all` match, `screen` does not);
/// `min-width` / `max-width` compare against an A4 page; other features
/// are assumed to match.
fn evaluate_media(input: &mut Parser<'_>) -> bool {
    let mut any = false;
    let mut empty = true;
    loop {
        let query = input.parse_until_before(Delimiter::Comma, |input| {
            let (mut negated, mut type_matches, mut features_match) = (false, true, true);
            let mut tokens = 0;
            while let Ok(token) = input.next() {
                tokens += 1;
                match token.clone() {
                    Token::Ident(ident) => match ident.to_ascii_lowercase().as_str() {
                        "not" => negated = true,
                        "only" | "and" => {}
                        "all" | "print" => type_matches = true,
                        _ => type_matches = false,
                    },
                    Token::ParenthesisBlock => {
                        let matches = input
                            .parse_nested_block(|i| {
                                Ok::<_, ParseError<Invalid>>(evaluate_feature(i))
                            })
                            .unwrap_or(true);
                        features_match &= matches;
                    }
                    _ => {}
                }
            }
            Ok::<_, ParseError<Invalid>>(
                (tokens > 0).then_some((type_matches && features_match) != negated),
            )
        });
        if let Ok(Some(matches)) = query {
            empty = false;
            any |= matches;
        }
        if input.next().is_err() {
            break;
        }
    }
    // An empty media query list matches everything.
    empty || any
}

fn evaluate_feature(input: &mut Parser<'_>) -> bool {
    let Ok(name) = input.expect_ident_cloned() else {
        skip_rest(input);
        return true;
    };
    let name = name.to_ascii_lowercase();
    if input.expect_colon().is_err() {
        skip_rest(input);
        return true;
    }
    let width = Length::parse(input).map(|l| l.to_pt(12.0, 12.0) / PT_PER_PX);
    skip_rest(input);
    match (name.as_str(), width) {
        ("min-width", Ok(width)) => MEDIA_WIDTH_PX >= width,
        ("max-width", Ok(width)) => MEDIA_WIDTH_PX <= width,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::values::{Color, ColorValue};
    use PropertyDeclaration as D;

    #[test]
    fn parses_rules_and_skips_invalid_ones() {
        let sheet = Stylesheet::parse(
            "p { color: red; bogus: 1; margin-top: 2pt }
             p >> { color: blue }
             @import url(x.css);
             @font-face { font-family: X }
             h1, h2 { display: none !important }",
        );
        assert_eq!(sheet.rules.len(), 2);
        assert_eq!(sheet.rules[0].declarations.len(), 2);
        assert_eq!(sheet.rules[1].selectors.slice().len(), 2);
        assert!(sheet.rules[1].declarations[0].important);
    }

    #[test]
    fn style_attribute() {
        let decls = parse_style_attribute("color: #00f; width: 50% !important; nonsense");
        assert_eq!(
            decls,
            [
                Declaration {
                    property: D::Color(ColorValue::Color(Color::rgb(0, 0, 255))),
                    important: false
                },
                Declaration {
                    property: D::Width(crate::values::Dim::Percent(0.5)),
                    important: true
                },
            ]
        );
    }

    #[test]
    fn media_rules_apply_for_print_only() {
        let sheet = Stylesheet::parse(
            "@media print { a { color: red } }
             @media screen { b { color: red } }
             @media screen, print { c { color: red } }
             @media not screen { d { color: red } }
             @media (max-width: 600px) { e { color: red } }
             @media all and (min-width: 600px) { f { color: red } }
             @supports (display: grid) { g { color: red } }",
        );
        assert_eq!(sheet.rules.len(), 5);
    }

    #[test]
    fn media_attribute() {
        assert!(media_matches(""));
        assert!(media_matches("print"));
        assert!(media_matches("screen, print"));
        assert!(!media_matches("screen"));
        assert!(!media_matches("only screen and (min-width: 100px)"));
    }

    #[test]
    fn page_rules_merge_in_order() {
        let sheet = Stylesheet::parse(
            "@page { size: A4 landscape; margin: 1in 2in }
             @page :first { margin: 0 }
             @page { margin-top: 36pt }",
        );
        let size = sheet.page.size.unwrap();
        assert_eq!(size.name.as_deref(), Some("a4"));
        assert_eq!(size.landscape, Some(true));
        assert_eq!(
            sheet.page.margin,
            Sides {
                top: Some(36.0),
                right: Some(144.0),
                bottom: Some(72.0),
                left: Some(144.0)
            }
        );

        let sheet = Stylesheet::parse("@page { size: 100mm 200mm }");
        let (w, h) = sheet.page.size.unwrap().dimensions.unwrap();
        assert!((w - 283.46).abs() < 0.01 && (h - 566.93).abs() < 0.01);
    }

    #[test]
    fn parses_font_faces() {
        let sheet = Stylesheet::parse(
            "@font-face {
                font-family: 'Noto Sans Devanagari';
                src: local(Mangal), url(a.woff2) format('woff2'),
                     url(\"a.ttf\") format(\"truetype\") tech(variations);
                font-weight: bold;
                font-style: italic;
                unicode-range: U+0900-097F;
            }
            @font-face { font-family: Brand Sans; src: url(b.otf); font-weight: 100 900 }
            @font-face { src: url(no-family.ttf) }",
        );
        let faces = sheet.font_faces;
        assert_eq!(faces.len(), 2);
        assert_eq!(faces[0].family, "Noto Sans Devanagari");
        assert_eq!(
            faces[0].sources,
            [
                FontSource::Local("Mangal".into()),
                FontSource::Url {
                    url: "a.woff2".into(),
                    format: Some("woff2".into())
                },
                FontSource::Url {
                    url: "a.ttf".into(),
                    format: Some("truetype".into())
                },
            ]
        );
        assert_eq!((faces[0].weight, faces[0].italic), (Some(700.0), true));
        assert_eq!(faces[1].family, "Brand Sans");
        assert_eq!((faces[1].weight, faces[1].italic), (None, false));
    }

    #[test]
    fn imports_only_at_the_top() {
        let sheet = Stylesheet::parse(
            "@charset 'utf-8'; @import url(a.css); @import 'b.css' screen;
             @import 'c.css' print, screen;
             @media print { @import 'nested.css'; }
             p {} @import 'late.css';",
        );
        let imports: Vec<(&str, bool)> = sheet
            .imports
            .iter()
            .map(|i| (i.url.as_str(), i.applies))
            .collect();
        assert_eq!(
            imports,
            [("a.css", true), ("b.css", false), ("c.css", true)]
        );
    }
}
