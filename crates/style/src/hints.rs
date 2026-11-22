//! Presentational hints: HTML attributes that act like CSS declarations.
//!
//! Templates written for wkhtmltopdf lean on these (`align`, `bgcolor`,
//! `cellpadding`, `<font>`), so they matter for compatibility. Hints sit in
//! the cascade between the user-agent stylesheet and author styles.

use presswork_dom::{Document, ElementData, NodeId};

use crate::properties::{
    BorderStyle, FontFamilyName, FontSize, GenericFamily, PropertyDeclaration, TextAlign,
    VerticalAlign, WhiteSpace,
};
use crate::values::{Color, ColorValue, Dim, Length, Side};

/// The declarations implied by the attributes of `element`.
pub(crate) fn presentational_hints(
    doc: &Document,
    node: NodeId,
    element: &ElementData,
) -> Vec<PropertyDeclaration> {
    use PropertyDeclaration as D;
    let mut hints = Vec::new();
    let name = element.name.as_str();
    let is_cell = matches!(name, "td" | "th");

    if matches!(
        name,
        "p" | "div"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "td"
            | "th"
            | "tr"
            | "thead"
            | "tbody"
            | "tfoot"
            | "caption"
    ) && let Some(align) = element.attr("align").and_then(TextAlign::from_keyword)
    {
        hints.push(D::TextAlign(align));
    }
    if name == "table"
        && let Some(align) = element.attr("align")
    {
        // A table is aligned as a box, with auto margins.
        match align.to_ascii_lowercase().as_str() {
            "center" => hints.extend([
                D::Margin(Side::Left, Dim::Auto),
                D::Margin(Side::Right, Dim::Auto),
            ]),
            "right" => hints.push(D::Margin(Side::Left, Dim::Auto)),
            _ => {}
        }
    }

    if matches!(name, "td" | "th" | "tr" | "thead" | "tbody" | "tfoot")
        && let Some(valign) = element.attr("valign").and_then(VerticalAlign::from_keyword)
    {
        hints.push(D::VerticalAlign(valign));
    }

    if matches!(
        name,
        "table" | "td" | "th" | "col" | "colgroup" | "hr" | "img"
    ) && let Some(width) = element.attr("width").and_then(parse_dimension)
    {
        hints.push(D::Width(width));
    }
    if matches!(name, "img" | "td" | "th" | "tr")
        && let Some(height) = element.attr("height").and_then(parse_dimension)
    {
        hints.push(D::Height(height));
    }
    if name == "img" {
        // `border="0"` is common in old templates and means none.
        if let Some(width) = element.attr("border").and_then(parse_pixels)
            && width > 0.0
        {
            push_border(&mut hints, Length::px(width), ColorValue::CurrentColor);
        }
        // `left` and `right` float the image, which is not supported.
        let align = element.attr("align").map(str::to_ascii_lowercase);
        let valign = match align.as_deref() {
            Some("top") => Some(VerticalAlign::Top),
            Some("texttop") => Some(VerticalAlign::TextTop),
            Some("middle" | "absmiddle" | "abscenter") => Some(VerticalAlign::Middle),
            Some("bottom" | "baseline") => Some(VerticalAlign::Baseline),
            Some("absbottom") => Some(VerticalAlign::Bottom),
            _ => None,
        };
        hints.extend(valign.map(D::VerticalAlign));
    }

    if matches!(
        name,
        "body" | "table" | "tr" | "td" | "th" | "thead" | "tbody" | "tfoot"
    ) && let Some(color) = element.attr("bgcolor").and_then(Color::from_attribute)
    {
        hints.push(D::BackgroundColor(ColorValue::Color(color)));
    }
    if matches!(name, "body" | "table" | "tr" | "td" | "th")
        && let Some(url) = element.attr("background").map(str::trim)
        && !url.is_empty()
    {
        hints.push(D::BackgroundImage(Some(url.into())));
    }

    if name == "table" {
        if let Some(width) = element.attr("border").map(parse_border_attribute)
            && width > 0.0
        {
            push_border(&mut hints, Length::px(width), GRAY);
        }
        if let Some(spacing) = element.attr("cellspacing").and_then(parse_pixels) {
            hints.push(D::BorderSpacing(Length::px(spacing)));
        }
    }

    if is_cell {
        if let Some(table) = enclosing_table(doc, node) {
            if table
                .attr("border")
                .map(parse_border_attribute)
                .unwrap_or(0.0)
                > 0.0
            {
                push_border(&mut hints, Length::px(1.0), GRAY);
            }
            if let Some(padding) = table.attr("cellpadding").and_then(parse_pixels) {
                let padding = Dim::Length(Length::px(padding));
                hints.extend(Side::ALL.map(|side| D::Padding(side, padding)));
            }
        }
        if element.attr("nowrap").is_some() {
            hints.push(D::WhiteSpace(WhiteSpace::Nowrap));
        }
    }

    if name == "font" {
        if let Some(color) = element.attr("color").and_then(Color::from_attribute) {
            hints.push(D::Color(ColorValue::Color(color)));
        }
        if let Some(face) = element.attr("face") {
            let families: Vec<FontFamilyName> = face
                .split(',')
                .map(|family| family.trim().trim_matches(['"', '\'']))
                .filter(|family| !family.is_empty())
                .map(|family| match GenericFamily::from_keyword(family) {
                    Some(generic) => FontFamilyName::Generic(generic),
                    None => FontFamilyName::Named(family.to_owned()),
                })
                .collect();
            if !families.is_empty() {
                hints.push(D::FontFamily(families.into()));
            }
        }
        if let Some(size) = element.attr("size").and_then(parse_font_size_attribute) {
            hints.push(D::FontSize(size));
        }
    }

    hints
}

/// The border color `<table border>` draws.
const GRAY: ColorValue = ColorValue::Color(Color::rgb(128, 128, 128));

/// A solid border on all sides.
fn push_border(hints: &mut Vec<PropertyDeclaration>, width: Length, color: ColorValue) {
    use PropertyDeclaration as D;
    for side in Side::ALL {
        hints.extend([
            D::BorderWidth(side, width),
            D::BorderStyle(side, BorderStyle::Solid),
            D::BorderColor(side, color),
        ]);
    }
}

/// The nearest `<table>` ancestor of a cell.
fn enclosing_table(doc: &Document, node: NodeId) -> Option<&ElementData> {
    let mut current = doc.node(node).parent;
    while let Some(id) = current {
        let element = doc.element(id)?;
        if element.name == "table" {
            return Some(element);
        }
        current = doc.node(id).parent;
    }
    None
}

/// A non-negative number of pixels, ignoring a trailing `px`.
fn parse_pixels(value: &str) -> Option<f32> {
    let value = value.trim();
    let value = value.strip_suffix("px").unwrap_or(value);
    value
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|v| *v >= 0.0 && v.is_finite())
}

/// `width="200"` (pixels) or `width="50%"`.
fn parse_dimension(value: &str) -> Option<Dim> {
    let value = value.trim();
    match value.strip_suffix('%') {
        Some(percent) => {
            let percent = percent.trim().parse::<f32>().ok()?;
            (percent >= 0.0 && percent.is_finite()).then_some(Dim::Percent(percent / 100.0))
        }
        None => parse_pixels(value).map(|px| Dim::Length(Length::px(px))),
    }
}

/// `border` on a table: a pixel width; present but empty or invalid means 1.
fn parse_border_attribute(value: &str) -> f32 {
    parse_pixels(value).unwrap_or(1.0)
}

/// `<font size>`: 1 to 7, or relative to 3 with a sign.
fn parse_font_size_attribute(value: &str) -> Option<FontSize> {
    let value = value.trim();
    let number: i32 = value.trim_start_matches('+').parse().ok()?;
    let size = if value.starts_with(['+', '-']) {
        3 + number
    } else {
        number
    };
    let px = match size.clamp(1, 7) {
        1 => 10.0,
        2 => 13.0,
        3 => 16.0,
        4 => 18.0,
        5 => 24.0,
        6 => 32.0,
        _ => 48.0,
    };
    Some(FontSize::Length(Length::px(px)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use PropertyDeclaration as D;
    use presswork_dom::parse_html;

    fn hints(html: &str, element: &str) -> Vec<PropertyDeclaration> {
        let doc = parse_html(html.as_bytes());
        let node = doc.find_element(element).expect("element exists");
        presentational_hints(&doc, node, doc.element(node).unwrap())
    }

    #[test]
    fn align_and_valign() {
        assert_eq!(
            hints("<table><tr><td align=RIGHT valign=top>", "td"),
            [
                D::TextAlign(TextAlign::Right),
                D::VerticalAlign(VerticalAlign::Top)
            ]
        );
        assert_eq!(
            hints("<table align=center>", "table"),
            [
                D::Margin(Side::Left, Dim::Auto),
                D::Margin(Side::Right, Dim::Auto)
            ]
        );
        assert_eq!(hints("<span align=right>", "span"), []);
    }

    #[test]
    fn background_attribute() {
        assert_eq!(
            hints("<table><tr><td background='paper.png'>", "td"),
            [D::BackgroundImage(Some("paper.png".into()))]
        );
        assert_eq!(hints("<p background='paper.png'>", "p"), []);
    }

    #[test]
    fn width_and_bgcolor() {
        assert_eq!(
            hints("<table width='100%' bgcolor=eeeeee>", "table"),
            [
                D::Width(Dim::Percent(1.0)),
                D::BackgroundColor(ColorValue::Color(Color::rgb(238, 238, 238)))
            ]
        );
        assert_eq!(
            hints("<table><tr><td width=120>", "td"),
            [D::Width(Dim::Length(Length::Pt(90.0)))]
        );
        assert_eq!(
            hints("<table><colgroup><col width='25%'>", "col"),
            [D::Width(Dim::Percent(0.25))]
        );
        assert_eq!(
            hints("<table><tr height=40><td>", "tr"),
            [D::Height(Dim::Length(Length::Pt(30.0)))]
        );
    }

    #[test]
    fn table_border_and_cellpadding_reach_cells() {
        let cell = hints("<table border=2 cellpadding=4><tr><td>", "td");
        assert_eq!(cell.len(), 12 + 4);
        assert_eq!(cell[0], D::BorderWidth(Side::Top, Length::Pt(0.75)));
        assert_eq!(
            cell[12],
            D::Padding(Side::Top, Dim::Length(Length::Pt(3.0)))
        );

        let table = hints("<table border=2 cellspacing=0>", "table");
        assert_eq!(table[0], D::BorderWidth(Side::Top, Length::Pt(1.5)));
        assert_eq!(table[12], D::BorderSpacing(Length::Pt(0.0)));
        assert_eq!(hints("<table border=0><tr><td>", "td"), []);
    }

    #[test]
    fn image_attributes() {
        use crate::values::Dim;
        let img = hints(
            "<img width=120 height='50%' border=0 align=absmiddle>",
            "img",
        );
        assert_eq!(
            img,
            [
                D::Width(Dim::Length(Length::px(120.0))),
                D::Height(Dim::Percent(0.5)),
                D::VerticalAlign(VerticalAlign::Middle),
            ]
        );
        // A non-zero border takes the text color.
        let img = hints("<img border=2>", "img");
        assert!(img.contains(&D::BorderColor(Side::Top, ColorValue::CurrentColor)));
        assert!(img.contains(&D::BorderWidth(Side::Left, Length::px(2.0))));
    }

    #[test]
    fn font_element() {
        assert_eq!(
            hints("<font color=red face='Arial, sans-serif' size=5>", "font"),
            [
                D::Color(ColorValue::Color(Color::rgb(255, 0, 0))),
                D::FontFamily(
                    vec![
                        FontFamilyName::Named("Arial".into()),
                        FontFamilyName::Generic(GenericFamily::SansSerif)
                    ]
                    .into()
                ),
                D::FontSize(FontSize::Length(Length::Pt(18.0))),
            ]
        );
        assert_eq!(
            hints("<font size=+1>", "font"),
            [D::FontSize(FontSize::Length(Length::Pt(13.5)))]
        );
    }
}
