//! End-to-end layout tests: HTML in, positioned items out.

use presswork_dom::parse_html;
use presswork_dom::resource::Url;
use presswork_style::Stylist;

use super::*;

fn layout_with(html: &str, options: &LayoutOptions) -> Vec<Page> {
    let (pages, failures) = with_failures(html, options);
    assert_eq!(failures, [], "every resource loads");
    pages
}

/// The pages, and the resources that failed to load.
fn with_failures(html: &str, options: &LayoutOptions) -> (Vec<Page>, Vec<LoadFailure>) {
    let fetcher = Fetcher::data_only();
    let pages = layout_fetching(&mut Engine::new(), html, options, &fetcher);
    (pages, fetcher.failures())
}

fn layout_fetching(
    engine: &mut Engine,
    html: &str,
    options: &LayoutOptions,
    fetcher: &Fetcher<'_>,
) -> Vec<Page> {
    let doc = parse_html(html.as_bytes());
    let styles = Stylist::for_document(&doc, fetcher);
    engine.layout(&doc, &styles, options, fetcher)
}

fn layout(html: &str) -> Vec<Page> {
    layout_with(html, &LayoutOptions::default())
}

fn runs(page: &Page) -> Vec<&TextRun> {
    page.items
        .iter()
        .filter_map(|item| match item {
            Item::Text(run) => Some(run),
            _ => None,
        })
        .collect()
}

fn fills(page: &Page) -> Vec<Fill> {
    page.items
        .iter()
        .filter_map(|item| match item {
            Item::Fill(fill) => Some(*fill),
            _ => None,
        })
        .collect()
}

/// The words on a page, in paint order.
fn words(page: &Page) -> Vec<&str> {
    runs(page)
        .into_iter()
        .map(|run| run.covered_text().trim())
        .collect()
}

/// The run containing `word`.
fn run_with<'a>(page: &'a Page, word: &str) -> &'a TextRun {
    runs(page)
        .into_iter()
        .find(|run| run.covered_text().contains(word))
        .unwrap_or_else(|| panic!("no run containing {word:?}"))
}

/// The right edge of a run.
fn right_edge(run: &TextRun) -> f32 {
    run.origin.x + run.glyphs.iter().map(|g| g.advance).sum::<f32>()
}

const RED: Color = Color::rgb(255, 0, 0);
const M: f32 = DEFAULT_MARGIN;

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.05
}

#[test]
fn lays_out_paragraphs_top_to_bottom() {
    let pages = layout("<h1>Invoice</h1><p>Hello <b>world</b></p>");
    assert_eq!(pages.len(), 1);
    assert_eq!(words(&pages[0]), ["Invoice", "Hello", "world"]);
    let runs = runs(&pages[0]);
    assert!(
        runs[0].origin.y < runs[1].origin.y,
        "heading above paragraph"
    );
    assert_eq!(
        runs[1].origin.y, runs[2].origin.y,
        "same line, same baseline"
    );
    assert_eq!(runs[0].font_size, 24.0);
}

#[test]
fn hides_head_and_scripts() {
    let pages = layout("<title>T</title><script>x()</script><p>Body</p>");
    assert_eq!(words(&pages[0]), ["Body"]);
}

#[test]
fn author_styles_apply() {
    let pages = layout(
        "<style>.total { color: red; font-size: 20pt } .gone { display: none }</style>
         <p class=total>Total</p><p class=gone>Hidden</p>",
    );
    let runs = runs(&pages[0]);
    assert_eq!(runs.len(), 1);
    assert_eq!((runs[0].color, runs[0].font_size), (RED, 20.0));
}

#[test]
fn color_changes_inside_a_line_keep_per_glyph_text() {
    let pages = layout("<p>ab<span style='color:red'>cd</span>ef</p>");
    let runs = runs(&pages[0]);
    let texts: Vec<&str> = runs.iter().map(|r| r.covered_text()).collect();
    assert_eq!(texts, ["ab", "cd", "ef"]);
    assert_eq!(runs[1].color, RED);
    let letters: Vec<&str> = runs[1]
        .glyphs
        .iter()
        .map(|g| &runs[1].text[g.text_range.clone()])
        .collect();
    assert_eq!(letters, ["c", "d"]);
}

#[test]
fn glyphs_map_back_to_their_own_characters() {
    let pages = layout("<p>Hello</p>");
    let run = runs(&pages[0])[0];
    let chars: Vec<&str> = run
        .glyphs
        .iter()
        .map(|g| &run.text[g.text_range.clone()])
        .collect();
    assert_eq!(chars, ["H", "e", "l", "l", "o"]);
}

#[test]
fn margins_padding_and_width_position_content() {
    let pages = layout(
        "<body style='margin:0'>
         <div style='margin-left:20pt; padding-left:10pt; border-left:5pt solid red; width:100pt'>
         <p style='margin:0; text-align:right'>x</p></div>",
    );
    let page = &pages[0];
    let x = run_with(page, "x");
    // Content box: 20 + 5 + 10 from the page margin, 100 wide.
    assert!(approx(right_edge(x), M + 35.0 + 100.0), "{x:?}");
    // The left border: 5pt wide, starting after the margin.
    let border = fills(page)[0];
    assert_eq!(border.color, RED);
    assert!(approx(border.rect.origin.x, M + 20.0) && approx(border.rect.size.width, 5.0));
}

#[test]
fn auto_margins_center_a_block() {
    let pages = layout(
        "<body style='margin:0'><div style='width:100pt; margin:0 auto; background:red'>x</div>",
    );
    let page = &pages[0];
    let background = fills(page)[0];
    let content_width = page.size.width - 2.0 * M;
    assert!(approx(
        background.rect.origin.x,
        M + (content_width - 100.0) / 2.0
    ));
    assert!(approx(background.rect.size.width, 100.0));
}

#[test]
fn vertical_margins_collapse() {
    let spaced = layout(
        "<body style='margin:0'><p style='margin:10pt 0'>a</p><p style='margin:30pt 0'>b</p>",
    );
    let tight =
        layout("<body style='margin:0'><p style='margin:10pt 0'>a</p><p style='margin:0'>b</p>");
    let gap = |pages: &[Page]| {
        let page = &pages[0];
        run_with(page, "b").origin.y - run_with(page, "a").origin.y
    };
    // max(10, 30) = 30 between the paragraphs, not 40.
    assert!(approx(gap(&spaced) - gap(&tight), 20.0));
}

#[test]
fn background_covers_the_box_and_paints_before_text() {
    let pages = layout(
        "<body style='margin:0'><div style='background:red; padding:10pt'><p style='margin:0'>x</p></div>",
    );
    let page = &pages[0];
    assert!(matches!(page.items[0], Item::Fill(_)), "background first");
    let background = fills(page)[0];
    let x = run_with(page, "x");
    assert!(approx(background.rect.origin.y, M));
    assert!(approx(
        background.rect.size.width,
        page.size.width - 2.0 * M
    ));
    assert!(background.rect.size.height > 20.0 + x.font_size * 0.9);
    assert!(approx(x.origin.x, M + 10.0));
}

#[test]
fn child_margins_collapse_through_an_undecorated_edge() {
    // No padding or border: the paragraph's margin collapses through the
    // div, and the background only covers the text.
    let pages = layout(
        "<body style='margin:0'><div style='background:red'><p style='margin:40pt 0'>x</p></div>",
    );
    let background = fills(&pages[0])[0];
    assert!(approx(background.rect.origin.y, M + 40.0), "{background:?}");
    assert!(background.rect.size.height < 30.0);
}

#[test]
fn long_documents_paginate() {
    let pages = layout(&"<p>line</p>".repeat(200));
    assert!(pages.len() > 1, "{} pages", pages.len());
    for page in &pages {
        for run in runs(page) {
            assert!(run.origin.y > M && run.origin.y < page.size.height - M);
        }
    }
}

#[test]
fn forced_page_breaks() {
    let pages = layout(
        "<p>one</p><p style='page-break-before: always'>two</p>
         <div style='break-after: page'>three</div><p>four</p>",
    );
    let per_page: Vec<Vec<&str>> = pages.iter().map(words).collect();
    assert_eq!(per_page, [vec!["one"], vec!["two", "three"], vec!["four"]]);
}

#[test]
fn backgrounds_continue_across_pages() {
    let html = format!(
        "<div style='background:red'>{}</div>",
        "<p>line</p>".repeat(80)
    );
    let pages = layout(&html);
    assert!(pages.len() > 1);
    for page in &pages {
        assert_eq!(fills(page).len(), 1, "one background piece per page");
    }
}

#[test]
fn table_cells_share_a_row_and_align() {
    let pages = layout(
        "<body style='margin:0'><table width='100%' cellspacing=0 cellpadding=0>
         <tr><td>Item</td><td align=right>Qty</td></tr></table>",
    );
    let page = &pages[0];
    let (item, qty) = (run_with(page, "Item"), run_with(page, "Qty"));
    assert_eq!(item.origin.y, qty.origin.y);
    assert!(approx(item.origin.x, M));
    assert!(approx(right_edge(qty), page.size.width - M), "{qty:?}");
}

#[test]
fn table_columns_fit_their_content() {
    let pages = layout(
        "<body style='margin:0'><table cellspacing=0 cellpadding=0>
         <tr><td>a</td><td>a much longer cell of text</td></tr>
         <tr><td>bb</td><td>c</td></tr></table>",
    );
    let page = &pages[0];
    // An auto-width table is only as wide as its content, and the second
    // column starts right after the widest cell of the first.
    let second = run_with(page, "c");
    let widest_first = right_edge(run_with(page, "bb"));
    assert!(approx(second.origin.x, widest_first), "{second:?}");
    assert!(right_edge(run_with(page, "longer")) < page.size.width / 2.0);
}

#[test]
fn wide_tables_shrink_columns_to_fit_the_page() {
    let cell = "word ".repeat(40);
    let pages = layout(&format!(
        "<table><tr><td>{cell}</td><td>{cell}</td></tr></table>"
    ));
    let page = &pages[0];
    for run in runs(page) {
        assert!(right_edge(run) <= page.size.width - M + 0.5, "{run:?}");
    }
}

#[test]
fn colspan_spans_columns() {
    let pages = layout(
        "<body style='margin:0'><table width=300 cellspacing=0 cellpadding=0>
         <tr><td>aaaa</td><td>bbbb</td><td>cccc</td></tr>
         <tr><td colspan=2 align=right>sum</td><td>t</td></tr></table>",
    );
    let page = &pages[0];
    assert!(approx(
        right_edge(run_with(page, "sum")),
        run_with(page, "cccc").origin.x
    ));
    assert!(approx(
        run_with(page, "t").origin.x,
        run_with(page, "cccc").origin.x
    ));
}

#[test]
fn rowspan_shifts_cells_in_later_rows() {
    let pages = layout("<table><tr><td rowspan=2>a</td><td>b</td></tr><tr><td>c</td></tr></table>");
    let page = &pages[0];
    assert!(approx(
        run_with(page, "c").origin.x,
        run_with(page, "b").origin.x
    ));
}

#[test]
fn cell_borders_and_backgrounds_fill_the_row() {
    let pages = layout(
        "<style>td { border: 1pt solid red; padding: 0 }</style>
         <body style='margin:0'><table cellspacing=0><tr><td>a<br>b<br>c</td><td>x</td></tr></table>",
    );
    let page = &pages[0];
    let borders = fills(page);
    // Four edges per cell, and both cells are as tall as the row.
    assert_eq!(borders.len(), 8);
    let heights: Vec<f32> = borders
        .iter()
        .filter(|f| f.rect.size.width <= 1.0)
        .map(|f| f.rect.size.height)
        .collect();
    assert_eq!(heights.len(), 4);
    assert!(
        heights.iter().all(|h| approx(*h, heights[0])),
        "{heights:?}"
    );
}

/// The distinct rectangles of the fills on a page: a line drawn twice at
/// the same place counts once.
fn distinct_fills(page: &Page) -> Vec<Rect> {
    let mut rects: Vec<Rect> = Vec::new();
    for fill in fills(page) {
        let same = |r: &Rect| {
            approx(r.origin.x, fill.rect.origin.x)
                && approx(r.origin.y, fill.rect.origin.y)
                && approx(r.size.width, fill.rect.size.width)
                && approx(r.size.height, fill.rect.size.height)
        };
        if !rects.iter().any(same) {
            rects.push(fill.rect);
        }
    }
    rects
}

#[test]
fn collapsed_borders_are_drawn_once() {
    let html = |collapse: &str| {
        format!(
            "<body style='margin:0'><style>table {{ border-collapse: {collapse}; border-spacing: 0 }}
             td {{ border: 1pt solid red }}</style>
             <table><tr><td>a</td><td>b</td></tr><tr><td>c</td><td>d</td></tr></table>"
        )
    };
    assert_eq!(fills(&layout(&html("separate"))[0]).len(), 16);
    let pages = layout(&html("collapse"));
    let page = &pages[0];
    let lines = distinct_fills(page);
    // Three vertical lines in each of two rows, three horizontal lines in
    // each of two columns: shared edges are one line, centered on the
    // grid line, with half of it inside each cell.
    assert_eq!(lines.len(), 12, "{lines:?}");
    let (a, b) = (run_with(page, "a"), run_with(page, "b"));
    // The table's border box starts at the page margin and holds half the
    // left line; the cell the other half, then its 1px (0.75pt) padding.
    assert!(approx(a.origin.x, M + 0.5 + 0.5 + 0.75), "{a:?}");
    let middle = lines
        .iter()
        .find(|r| r.size.width == 1.0 && r.origin.x > a.origin.x && r.origin.x < b.origin.x)
        .unwrap();
    assert!(
        approx(middle.origin.x + 1.0 + 0.75, b.origin.x),
        "{middle:?} {b:?}"
    );
}

#[test]
fn vertical_align_moves_cell_content() {
    let html = |valign: &str| {
        format!("<table><tr><td>a<br>b<br>c</td><td valign={valign}>x</td></tr></table>")
    };
    let y = |valign: &str| run_with(&layout(&html(valign))[0], "x").origin.y;
    let (top, middle, bottom) = (y("top"), y("middle"), y("bottom"));
    assert!(top < middle && middle < bottom, "{top} {middle} {bottom}");
    assert!(approx(middle - top, bottom - middle));
}

#[test]
fn table_header_repeats_on_every_page() {
    let rows = "<tr><td>row</td></tr>".repeat(120);
    let pages = layout(&format!(
        "<table><thead><tr><th>Head</th></tr></thead><tbody>{rows}</tbody></table>"
    ));
    assert!(pages.len() >= 2);
    for page in &pages {
        assert_eq!(words(page)[0], "Head");
    }
    let total: usize = pages
        .iter()
        .map(|p| words(p).iter().filter(|w| **w == "row").count())
        .sum();
    assert_eq!(total, 120, "no row lost or duplicated");
}

#[test]
fn list_markers_hang_left_of_the_text() {
    let pages = layout("<ol start=3><li>first</li><li>second</li></ol><ul><li>bullet</li></ul>");
    let page = &pages[0];
    assert_eq!(words(page), ["first", "3.", "second", "4.", "bullet", "•"]);
    let (text, marker) = (run_with(page, "first"), run_with(page, "3."));
    assert_eq!(text.origin.y, marker.origin.y);
    assert!(right_edge(marker) < text.origin.x);
}

#[test]
fn pre_keeps_line_breaks_and_does_not_wrap() {
    let long = "x".repeat(400);
    let pages = layout(&format!("<pre>one\ntwo\n{long}</pre>"));
    let runs = runs(&pages[0]);
    assert_eq!(runs.len(), 3, "three lines, the long one unwrapped");
    assert!(runs[0].origin.y < runs[1].origin.y);
}

#[test]
fn page_size_and_margins_come_from_options() {
    let options = LayoutOptions {
        page_size: Size {
            width: 300.0,
            height: 200.0,
        },
        margins: EdgeSizes {
            top: 50.0,
            right: 0.0,
            bottom: 0.0,
            left: 40.0,
        },
        images: true,
    };
    let pages = layout_with("<body style='margin:0'><p style='margin:0'>x</p>", &options);
    let page = &pages[0];
    assert_eq!(page.size, options.page_size);
    let x = run_with(page, "x");
    assert!(approx(x.origin.x, 40.0) && x.origin.y > 50.0);
}

/// Golden test: a bilingual Nepali/English invoice. Every glyph must come
/// from a font that has it: glyph 0 is `.notdef`, the "tofu" box drawn
/// when no font covers a character.
///
/// Needs a Devanagari font on the system (e.g. `fonts-noto-core`).
#[test]
fn nepali_invoice_has_no_missing_glyphs() {
    let html = include_str!("../../../tests/fixtures/invoice-ne.html");
    let pages = layout(html);
    assert_eq!(pages.len(), 1);
    let mut glyphs = 0;
    for run in runs(&pages[0]) {
        for glyph in &run.glyphs {
            glyphs += 1;
            assert_ne!(
                glyph.id,
                0,
                "missing glyph for {:?}",
                &run.text[glyph.text_range.clone()]
            );
        }
    }
    assert!(glyphs > 500, "only {glyphs} glyphs");
}

/// Golden test: an invoice styled the way real templates are (classes,
/// collapsed borders, shaded rows, `page-break-before`, a long table).
#[test]
fn styled_invoice_paginates_with_repeated_headers() {
    let html = include_str!("../../../tests/fixtures/invoice-styled.html");
    let pages = layout(html);
    assert!(pages.len() >= 3, "{} pages", pages.len());

    // The item table runs over several pages; each starts with its header.
    let with_rows: Vec<&Page> = pages
        .iter()
        .filter(|page| words(page).iter().any(|w| w.contains("Dhaka topi")))
        .collect();
    assert!(with_rows.len() >= 2);
    for page in &with_rows[1..] {
        // The header's first cell; the dots come from another font, so it
        // is more than one run.
        assert!(words(page)[0].starts_with("क्र"), "header repeats");
    }

    // The terms start their own page.
    let last = pages.last().unwrap();
    assert!(words(last)[..3].contains(&"Terms"), "{:?}", words(last));

    // Header cells are white text on a filled background.
    let header = run_with(&pages[0], "Description");
    assert_eq!(header.color, Color::WHITE);
    assert!(
        fills(&pages[0])
            .iter()
            .any(|f| f.color == Color::rgb(0x1a, 0x4e, 0x8a))
    );

    for page in &pages {
        for run in runs(page) {
            assert!(
                run.glyphs.iter().all(|g| g.id != 0),
                "missing glyph in {run:?}"
            );
            assert!(
                right_edge(run) <= page.size.width,
                "{run:?} overflows the page"
            );
        }
    }
}

/// A 2x1 PNG (one red pixel, one blue): 1.5pt by 0.75pt at its natural
/// size.
const PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAIAAAB7QOjdAAAAD0lEQVR4nGP4z8DAwPAfAAcAAf9+CLHQAAAAAElFTkSuQmCC";

fn images(page: &Page) -> Vec<Rect> {
    page.items
        .iter()
        .filter_map(|item| match item {
            Item::Image(image) => Some(image.rect),
            _ => None,
        })
        .collect()
}

#[test]
fn inline_image_sits_on_the_baseline_after_the_text() {
    let pages = layout(&format!("<p>Logo <img src='{PNG}' width=200> text</p>"));
    let [image] = images(&pages[0])[..] else {
        panic!("one image");
    };
    // 200px wide, half as tall: the 2:1 shape is kept.
    assert_eq!((image.size.width, image.size.height), (150.0, 75.0));
    let logo = run_with(&pages[0], "Logo");
    // Right after "Logo " (the run includes the space).
    assert!(approx(image.origin.x, right_edge(logo)));
    let bottom = image.origin.y + image.size.height;
    assert!(
        approx(bottom, logo.origin.y),
        "{bottom} vs {}",
        logo.origin.y
    );
    // The line grew to fit it: the text shares the image's line.
    let text = run_with(&pages[0], "text");
    assert_eq!(text.origin.y, logo.origin.y);
    assert!(approx(text.origin.x, image.origin.x + image.size.width));
}

#[test]
fn vertical_align_middle_centers_an_image_on_the_text() {
    let pages = layout(&format!(
        "<p>x <img src='{PNG}' width=80 style='vertical-align: middle'></p>"
    ));
    let image = images(&pages[0])[0];
    let baseline = run_with(&pages[0], "x").origin.y;
    // Centered half an x-height above the baseline: a few points for
    // 12pt text.
    let center = image.origin.y + image.size.height / 2.0;
    assert!(
        center < baseline && center > baseline - 5.0,
        "{center} vs {baseline}"
    );
}

#[test]
fn block_image_is_centered_by_auto_margins() {
    let pages = layout(&format!(
        "<img src='{PNG}' style='display: block; width: 100pt; margin: 0 auto'>"
    ));
    let [image] = images(&pages[0])[..] else {
        panic!("one image");
    };
    // Inside the body's default 8px (6pt) margin.
    let content = PageSize::A4.size(Orientation::Portrait).width - 2.0 * M - 12.0;
    assert!(approx(image.origin.x, M + 6.0 + (content - 100.0) / 2.0));
    assert_eq!((image.origin.y, image.size.height), (M + 6.0, 50.0));
}

#[test]
fn max_width_fits_a_large_image_into_its_table_cell() {
    let pages = layout(&format!(
        "<table><tr><td><img src='{PNG}' style='width: 4000px; max-width: 100%'></td>\
         <td>Invoice number 42</td></tr></table>"
    ));
    let [image] = images(&pages[0])[..] else {
        panic!("one image");
    };
    let page_width = pages[0].size.width;
    assert!(image.origin.x + image.size.width <= page_width - M + 0.01);
    assert!(image.size.width > 100.0, "uses the room there is");
    assert!(approx(image.size.height, image.size.width / 2.0));
    let text = run_with(&pages[0], "Invoice");
    assert!(right_edge(text) <= page_width - M + 0.01);
}

#[test]
fn missing_image_shows_its_alt_text_and_is_reported_once() {
    let (pages, failures) = with_failures(
        "<p><img src='logo.png' alt='ACME Ltd'> and again <img src=' logo.png '></p>",
        &LayoutOptions::default(),
    );
    assert_eq!(words(&pages[0]), ["ACME Ltd and again"]);
    assert!(images(&pages[0]).is_empty());
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].url, "logo.png");
    // Relative, and the document has no location.
    assert!(matches!(failures[0].error, LoadError::Failed(_)));
}

#[test]
fn missing_image_with_a_size_keeps_its_box() {
    let (pages, _) = with_failures(
        "<p><img src='logo.png' width=100 height=40 style='border: 2px solid red'>x</p>",
        &LayoutOptions::default(),
    );
    let page = &pages[0];
    let border: Vec<Fill> = fills(page).into_iter().filter(|f| f.color == RED).collect();
    assert_eq!(border.len(), 4);
    let left = border
        .iter()
        .map(|f| f.rect.origin.x)
        .fold(f32::MAX, f32::min);
    let x = run_with(page, "x");
    // 100px of content plus 2px of border each side.
    assert!(approx(x.origin.x - left, 75.0 + 3.0));
}

#[test]
fn unsupported_image_formats_are_reported() {
    let (_, failures) = with_failures(
        "<img src='data:image/svg+xml,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22/%3E'>",
        &LayoutOptions::default(),
    );
    assert_eq!(failures.len(), 1);
    assert!(
        failures[0].error.to_string().contains("SVG"),
        "{failures:?}"
    );
}

#[test]
fn an_image_that_does_not_fit_moves_to_the_next_page() {
    let image = format!("<img src='{PNG}' style='display: block; height: 500pt'>");
    let pages = layout(&format!("{image}{image}"));
    assert_eq!(pages.len(), 2);
    assert_eq!(images(&pages[1])[0].origin.y, M);
}

/// The file and glyphs of the system font that sans-serif text uses, or
/// `None` if it is a font collection, which these tests cannot reuse.
fn system_sans() -> Option<(Vec<u8>, Vec<u32>, u64)> {
    let pages = layout("<p style='font-family: sans-serif'>Invoice 42</p>");
    let run = run_with(&pages[0], "Invoice");
    let bytes = run.font.data.data().to_vec();
    if bytes.starts_with(b"ttcf") || run.font.index != 0 {
        eprintln!("skipped: the sans-serif font is in a collection");
        return None;
    }
    let glyphs = run.glyphs.iter().map(|g| g.id).collect();
    Some((bytes, glyphs, run.font.data.id()))
}

/// A `data:` URL for `bytes`, percent-encoded.
fn data_url(bytes: &[u8]) -> String {
    let mut url = String::from("data:font/ttf,");
    for byte in bytes {
        url.push_str(&format!("%{byte:02X}"));
    }
    url
}

/// The run containing "Invoice" when `html` declares the family Brand.
fn brand_run(engine: &mut Engine, css: &str, fetcher: &Fetcher<'_>) -> (Vec<u32>, u64) {
    let html = format!("<style>{css}</style><p style='font-family: Brand, serif'>Invoice 42</p>");
    let pages = layout_fetching(engine, &html, &LayoutOptions::default(), fetcher);
    let run = run_with(&pages[0], "Invoice");
    (
        run.glyphs.iter().map(|g| g.id).collect(),
        run.font.data.id(),
    )
}

#[test]
fn font_face_fonts_are_used_and_do_not_leak_into_the_next_document() {
    let Some((bytes, glyphs, system_id)) = system_sans() else {
        return;
    };
    let css = format!(
        "@font-face {{ font-family: Brand; src: url(missing.woff2) format('woff2'), url({}) }}",
        data_url(&bytes)
    );
    let mut engine = Engine::new();
    let fetcher = Fetcher::data_only();
    let (brand_glyphs, brand_id) = brand_run(&mut engine, &css, &fetcher);
    assert_eq!(brand_glyphs, glyphs, "the @font-face font shapes the text");
    assert_ne!(brand_id, system_id, "from the document, not the system");
    assert_eq!(
        fetcher.failures(),
        [],
        "the WOFF2 source is skipped quietly"
    );

    // Unregistered once layout is done.
    let registered = engine.fonts.collection.family_by_name("presswork 0 Brand");
    assert!(registered.is_none_or(|family| family.fonts().is_empty()));

    // The same engine, a document without the rule: Brand is unknown.
    let (_, fallback_id) = brand_run(&mut engine, "", &fetcher);
    assert_ne!(fallback_id, brand_id);
}

#[test]
fn woff_fonts_are_decoded() {
    let Some((bytes, glyphs, _)) = system_sans() else {
        return;
    };
    let woff = flow::webfont_tests::encode_woff(&bytes);
    let css = format!(
        "@font-face {{ font-family: Brand; src: url({}) format('woff') }}",
        data_url(&woff)
    );
    let fetcher = Fetcher::data_only();
    let (brand_glyphs, _) = brand_run(&mut Engine::new(), &css, &fetcher);
    assert_eq!(fetcher.failures(), []);
    assert_eq!(brand_glyphs, glyphs);
}

#[test]
fn fonts_load_only_when_used_and_failures_are_reported() {
    let fetcher = Fetcher::data_only();
    let css = "@font-face { font-family: Unused; src: url(unused.ttf) }
               @font-face { font-family: Brand; src: url(data:font/ttf,nonsense) }";
    let (glyphs, _) = brand_run(&mut Engine::new(), css, &fetcher);
    assert!(glyphs.iter().all(|&g| g != 0), "falls back to serif");
    let failures = fetcher.failures();
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(failures[0].url.starts_with("data:font/ttf"));
    assert!(failures[0].error.to_string().contains("not a font"));

    // A family with only WOFF2 sources says so.
    let fetcher = Fetcher::data_only();
    let css = "@font-face { font-family: Brand; src: url(b.woff2) format('woff2') }";
    brand_run(&mut Engine::new(), css, &fetcher);
    let failures = fetcher.failures();
    assert!(
        failures[0].error.to_string().contains("not supported"),
        "{failures:?}"
    );
}

#[test]
fn no_images_shows_alt_text_without_failures() {
    let options = LayoutOptions {
        images: false,
        ..LayoutOptions::default()
    };
    let (pages, failures) = with_failures(&format!("<img src='{PNG}' alt='Logo'>"), &options);
    assert!(images(&pages[0]).is_empty());
    assert_eq!(words(&pages[0]), ["Logo"]);
    assert_eq!(failures, []);
}

/// The text each glyph of `run` maps back to.
fn glyph_texts(run: &TextRun) -> Vec<&str> {
    run.glyphs
        .iter()
        .map(|g| &run.text[g.text_range.clone()])
        .collect()
}

#[test]
fn plain_clusters_map_one_character_per_glyph() {
    let pages = layout("<p lang=ne>कृष्ण किताब</p>");
    let texts = glyph_texts(run_with(&pages[0], "कृ"));
    // कृ is the consonant and the vowel sign below it, each its own
    // glyph, so each maps to its own character.
    assert_eq!(texts[..2], ["क", "\u{943}"]);
    // The conjunct ष्ण is substituted glyphs: they keep the cluster.
    assert!(texts.contains(&"ष\u{94d}ण"), "{texts:?}");
    // ि is drawn before its consonant: both keep the cluster, so the
    // text comes out in reading order.
    let texts = glyph_texts(run_with(&pages[0], "किताब"));
    let ki: Vec<&str> = texts.into_iter().filter(|t| t.contains('ि')).collect();
    assert!(ki.len() >= 2 && ki.iter().all(|t| *t == "कि"), "{ki:?}");
}

#[test]
fn ligatures_map_to_all_their_characters() {
    let pages = layout("<p style=\"font-family: sans-serif\">fixtures office</p>");
    let run = run_with(&pages[0], "fixtures");
    let texts = glyph_texts(run);
    // Every character is covered, in order, whether or not the font has
    // ligatures: an fi ligature's glyph stands for both letters.
    assert_eq!(texts.concat(), "fixtures office", "{texts:?}");
}

/// The index in paint order of the first item matching `f`.
fn paint_index(page: &Page, f: impl Fn(&Item) -> bool) -> usize {
    page.items.iter().position(f).expect("item is painted")
}

#[test]
fn underlines_sit_below_the_baseline_and_paint_under_the_text() {
    let pages = layout("<body style='margin:0'><p><u>Total due</u> now</p>");
    let page = &pages[0];
    let total = run_with(page, "Total");
    let lines = fills(page);
    assert_eq!(lines.len(), 1, "{lines:?}");
    let line = lines[0].rect;
    assert_eq!(lines[0].color, Color::BLACK);
    assert!(line.origin.y > total.origin.y, "below the baseline");
    assert!(line.origin.y < total.origin.y + total.font_size * 0.3);
    assert!(line.size.height > 0.2 && line.size.height < 2.0, "{line:?}");
    assert!(approx(line.origin.x, total.origin.x));
    // Only under the underlined words, not the space after them.
    assert!(approx(line.origin.x + line.size.width, right_edge(total)));
    let fill = paint_index(page, |item| matches!(item, Item::Fill(_)));
    let text = paint_index(page, |item| matches!(item, Item::Text(_)));
    assert!(fill < text, "underline painted first");
}

#[test]
fn decorations_reach_descendants_in_the_decorating_color() {
    let pages = layout(
        "<body style='margin:0'><p style='color:red; text-decoration:line-through'>Old
         <span style='color:black; text-decoration:none'>price</span></p>",
    );
    let page = &pages[0];
    let lines = fills(page);
    assert_eq!(lines.len(), 2, "{lines:?}");
    let price = run_with(page, "price");
    assert_eq!(price.color, Color::BLACK);
    for line in &lines {
        assert_eq!(line.color, RED);
        assert!(line.rect.origin.y < price.origin.y, "through the text");
        assert!(line.rect.origin.y > price.origin.y - price.font_size * 0.6);
    }
    let text = paint_index(page, |item| matches!(item, Item::Text(_)));
    let fill = paint_index(page, |item| matches!(item, Item::Fill(_)));
    assert!(text < fill, "line-through painted over the text");
}

#[test]
fn underlines_skip_the_spaces_that_end_a_line() {
    let pages = layout("<body style='margin:0'><div style='width:60pt'><u>aaaa bbbb cccc</u>");
    let page = &pages[0];
    let lines = fills(page);
    let runs = runs(page);
    assert_eq!(lines.len(), runs.len());
    for (line, run) in lines.iter().zip(runs) {
        let word_end = run
            .glyphs
            .iter()
            .rposition(|g| !run.text[g.text_range.clone()].trim().is_empty())
            .unwrap()
            + 1;
        let width: f32 = run.glyphs[..word_end].iter().map(|g| g.advance).sum();
        assert!(approx(line.rect.size.width, width), "{line:?} {run:?}");
    }
}

#[test]
fn links_are_underlined_and_list_markers_are_not() {
    let pages = layout(
        "<body style='margin:0'><p><a href='https://example.com'>site</a></p>
         <ul><li style='text-decoration:underline'>Item</li></ul>",
    );
    let page = &pages[0];
    let lines = fills(page);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(lines[0].color, Color::rgb(0, 0, 0xee));
    let item = run_with(page, "Item");
    assert!(
        approx(lines[1].rect.origin.x, item.origin.x),
        "not under the marker"
    );
}

#[test]
fn letter_and_word_spacing_widen_text() {
    let width = |style: &str| {
        let pages = layout(&format!(
            "<body style='margin:0'><p style='{style}'>ab cd</p>"
        ));
        let runs = runs(&pages[0]);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].covered_text(), "ab cd");
        right_edge(runs[0]) - runs[0].origin.x
    };
    let plain = width("");
    // After each of the five characters.
    assert!(approx(width("letter-spacing: 2pt") - plain, 10.0));
    assert!(approx(width("word-spacing: 0.5em") - plain, 6.0));
}

#[test]
fn text_indent_moves_the_first_line_of_each_block() {
    let text = "word ".repeat(30);
    let pages = layout(&format!(
        "<body style='margin:0'>
         <div style='width:200pt; text-indent:10%'>{text}<p>para</p>tail</div>
         <div style='width:200pt; text-indent:20pt hanging'>{text}</div>"
    ));
    let page = &pages[0];
    let runs = runs(page);
    let first_line_y = runs[0].origin.y;
    let starts: Vec<(f32, f32)> = runs.iter().map(|r| (r.origin.x - M, r.origin.y)).collect();
    // The first line of the first block, then its continuation lines.
    assert!(approx(starts[0].0, 20.0), "{starts:?}");
    let second = starts.iter().find(|(_, y)| *y > first_line_y).unwrap();
    assert!(approx(second.0, 0.0));
    // The paragraph inherits the indent; the text after it is not a
    // first line.
    assert!(approx(run_with(page, "para").origin.x - M, 20.0));
    assert!(approx(run_with(page, "tail").origin.x - M, 0.0));
    // Hanging: all lines but the first.
    let tail_y = run_with(page, "tail").origin.y;
    let hanging: Vec<_> = starts.iter().filter(|(_, y)| *y > tail_y).collect();
    assert!(approx(hanging[0].0, 0.0), "{hanging:?}");
    assert!(hanging[1..].iter().all(|(x, _)| approx(*x, 20.0)));
}

#[test]
fn text_transform_changes_the_text() {
    let text = |html: &str| {
        let pages = layout(html);
        runs(&pages[0])
            .iter()
            .map(|run| run.covered_text())
            .collect::<String>()
    };
    assert_eq!(
        text("<p style='text-transform:uppercase'>straße</p>"),
        "STRASSE"
    );
    assert_eq!(
        text("<p style='text-transform:capitalize'>hello <b>wor</b>ld don't नमस्ते</p>"),
        "Hello World Don't नमस्ते"
    );
    assert_eq!(text("<p style='text-transform:lowercase'>ABC</p>"), "abc");
}

#[test]
fn heights_make_blocks_taller_but_never_cut_content() {
    let pages = layout(
        "<body style='margin:0'>
         <div style='height:100pt; background:red'>x</div><p style='margin:0'>after</p>
         <div style='height:5pt; background:blue'>grows</div>
         <div style='min-height:40pt; padding:10pt; box-sizing:border-box; background:red'></div>",
    );
    let page = &pages[0];
    let boxes = fills(page);
    assert_eq!(boxes.len(), 3, "{boxes:?}");
    assert!(approx(boxes[0].rect.size.height, 100.0));
    let after = run_with(page, "after");
    assert!(after.origin.y > M + 100.0 && after.origin.y < M + 100.0 + 1.5 * after.font_size);
    let grows = run_with(page, "grows");
    assert!(boxes[1].rect.size.height > grows.font_size);
    // Border-box sizing counts the padding; an empty box keeps its height.
    assert!(approx(boxes[2].rect.size.height, 40.0));
}

#[test]
fn a_fixed_height_box_that_does_not_fit_moves_to_the_next_page() {
    let pages = layout(
        "<body style='margin:0'><div style='height:700pt'></div>
         <div style='height:200pt; background:red'></div>",
    );
    assert_eq!(pages.len(), 2);
    assert_eq!(fills(&pages[0]), []);
    let signature = fills(&pages[1]);
    assert_eq!(signature.len(), 1);
    assert!(approx(signature[0].rect.origin.y, M));
    assert!(approx(signature[0].rect.size.height, 200.0));
}

#[test]
fn table_cells_and_rows_take_height_as_a_minimum() {
    let pages = layout(
        "<body style='margin:0'><table cellspacing=0 cellpadding=0>
         <tr><td height=80 style='background:red'>a</td><td>b</td></tr>
         <tr style='height:50pt'><td style='background:red'>c</td></tr>
         <tr><td style='height:2pt; background:red'>tall content</td></tr></table>",
    );
    let page = &pages[0];
    let cells = fills(page);
    assert_eq!(cells.len(), 3);
    // 80px is 60pt.
    assert!(approx(cells[0].rect.size.height, 60.0), "{cells:?}");
    assert!(approx(cells[1].rect.size.height, 50.0));
    assert!(cells[2].rect.size.height > run_with(page, "tall").font_size);
}

/// The index of the page `word` is on.
fn page_of(pages: &[Page], word: &str) -> usize {
    pages
        .iter()
        .position(|page| words(page).contains(&word))
        .unwrap_or_else(|| panic!("no page has {word:?}"))
}

/// Line metrics of the default font: the distance between baselines and
/// from the top of a line to its baseline.
fn line_metrics() -> (f32, f32) {
    let pages = layout("<body style='margin:0'><p style='margin:0'>a<br>b</p>");
    let (a, b) = (run_with(&pages[0], "a"), run_with(&pages[0], "b"));
    (b.origin.y - a.origin.y, a.origin.y - M)
}

/// The page content height with the default options.
fn available() -> f32 {
    let size = LayoutOptions::default().page_size;
    size.height - 2.0 * M
}

/// Lays out `content` below an empty block that leaves `room` points at
/// the bottom of the first page.
fn below_filler(room: f32, content: &str) -> Vec<Page> {
    layout(&format!(
        "<body style='margin:0'><div style='height:{}pt'></div>{content}",
        available() - room
    ))
}

const FOUR_LINES: &str = "l1<br>l2<br>l3<br>l4</p>";

#[test]
fn widows_carry_at_least_two_lines_to_the_next_page() {
    let (pitch, _) = line_metrics();
    // Room for three of the four lines.
    let room = 3.5 * pitch;
    let pages = below_filler(room, &format!("<p style='margin:0'>{FOUR_LINES}"));
    assert_eq!(page_of(&pages, "l2"), 0);
    assert_eq!(page_of(&pages, "l3"), 1);
    let pages = below_filler(room, &format!("<p style='margin:0; widows:1'>{FOUR_LINES}"));
    assert_eq!(page_of(&pages, "l3"), 0);
    assert_eq!(page_of(&pages, "l4"), 1);
}

#[test]
fn orphans_leave_at_least_two_lines_or_none() {
    let (pitch, _) = line_metrics();
    // Room for one line.
    let room = 1.5 * pitch;
    let pages = below_filler(room, &format!("<p style='margin:0'>{FOUR_LINES}"));
    assert_eq!(page_of(&pages, "l1"), 1);
    let pages = below_filler(
        room,
        &format!("<p style='margin:0; orphans:1'>{FOUR_LINES}"),
    );
    assert_eq!(page_of(&pages, "l1"), 0);
}

#[test]
fn break_inside_avoid_moves_a_box_to_the_next_page_whole() {
    let (pitch, _) = line_metrics();
    let lines = "<p style='margin:0; orphans:1; widows:1'>l1<br>l2<br>l3</p>";
    let room = 1.5 * pitch;
    let pages = below_filler(
        room,
        &format!("<div style='page-break-inside:avoid'>{lines}</div>"),
    );
    assert_eq!(page_of(&pages, "l1"), 1);
    let pages = below_filler(room, &format!("<div>{lines}</div>"));
    assert_eq!(page_of(&pages, "l1"), 0);
}

#[test]
fn headings_stay_with_what_follows_them() {
    let (pitch, ascent) = line_metrics();
    let content = "<h2 style='margin:0'>Terms</h2><p style='margin:0'>text</p>";
    let alone = layout(&format!("<body style='margin:0'>{content}"));
    let heading_height = run_with(&alone[0], "text").origin.y - ascent - M;
    // Room for the heading but not the paragraph.
    let room = heading_height + 0.5 * pitch;
    let pages = below_filler(room, content);
    assert_eq!(page_of(&pages, "Terms"), 1);
    let pages = below_filler(
        room,
        &content.replace("margin:0'", "margin:0; break-after:auto'"),
    );
    assert_eq!(page_of(&pages, "Terms"), 0);
}

#[test]
fn table_header_rows_are_not_left_alone_at_a_page_bottom() {
    let rows = "<tr><td style='height:20pt'>row</td></tr>".repeat(3);
    let table = format!(
        "<table cellspacing=0 cellpadding=0><thead><tr><th style='height:20pt'>Head</th></tr>
         </thead>{rows}</table>"
    );
    // Room for the header row only.
    let pages = below_filler(30.0, &table);
    assert_eq!(page_of(&pages, "Head"), 1);
    assert_eq!(words(&pages[1]).iter().filter(|w| **w == "Head").count(), 1);
}

#[test]
fn fixed_width_columns_keep_their_width_in_a_wide_table() {
    let content_width = LayoutOptions::default().page_size.width - 2.0 * M;
    let row = "<td>Num</td><td>Description</td><td>Amount</td>";
    for html in [
        "<table width='100%' cellspacing=0 cellpadding=0>
         <tr><td width=40>Num</td><td>Description</td><td width=100>Amount</td></table>"
            .to_owned(),
        format!(
            "<table width='100%' cellspacing=0 cellpadding=0>
             <colgroup><col width=40><col><col style='width:75pt'></colgroup><tr>{row}</table>"
        ),
    ] {
        let pages = layout(&format!("<body style='margin:0'>{html}"));
        let page = &pages[0];
        // 40px is 30pt, 100px 75pt; the description takes the rest.
        assert!(
            approx(run_with(page, "Description").origin.x, M + 30.0),
            "{html}"
        );
        assert!(approx(
            run_with(page, "Amount").origin.x,
            M + content_width - 75.0
        ));
    }
}

#[test]
fn fixed_table_layout_ignores_the_content() {
    let pages = layout(
        "<body style='margin:0'><table style='table-layout:fixed; width:300pt'
         cellspacing=0 cellpadding=0><col style='width:50pt'>
         <tr><td>a</td><td>averyveryveryverylongword</td><td>c</td></tr></table>",
    );
    let page = &pages[0];
    // 50pt, then the other two share the remaining 250pt.
    assert!(approx(run_with(page, "averyvery").origin.x, M + 50.0));
    assert!(approx(run_with(page, "c").origin.x, M + 175.0));
}

#[test]
fn a_cell_spanning_rows_covers_them_and_grows_them() {
    let pages = layout(
        "<body style='margin:0'><table cellspacing=0 cellpadding=0>
         <tr><td rowspan=2 style='background:red; vertical-align:bottom'>a<br>a<br>a<br>a</td>
         <td style='background:blue'>b</td></tr>
         <tr><td style='background:blue'>c</td></tr>
         <tr><td>after</td></tr></table>",
    );
    let page = &pages[0];
    let (pitch, _) = line_metrics();
    let fills = fills(page);
    let red = fills.iter().find(|f| f.color == RED).unwrap().rect;
    let blue: Vec<Rect> = fills
        .iter()
        .filter(|f| f.color != RED)
        .map(|f| f.rect)
        .collect();
    // Four lines tall, over both rows, which share the height equally.
    assert!(approx(red.size.height, 4.0 * pitch), "{red:?}");
    assert!(approx(blue[0].size.height, 2.0 * pitch), "{blue:?}");
    assert!(approx(blue[1].origin.y, M + 2.0 * pitch));
    assert!(approx(blue[1].size.height, 2.0 * pitch));
    // The next row starts below the spanning cell.
    assert!(run_with(page, "after").origin.y > M + 4.0 * pitch);
    // `rowspan` rows are one slice: the cell's lines are in order and the
    // cell is on one page.
    assert_eq!(pages.len(), 1);
}

#[test]
fn vertical_align_uses_the_whole_span() {
    let pages = layout(
        "<body style='margin:0'><table cellspacing=0 cellpadding=0>
         <tr><td rowspan=2 style='vertical-align:bottom'>low</td><td>b</td></tr>
         <tr><td>c</td></tr></table>",
    );
    let page = &pages[0];
    assert!(approx(
        run_with(page, "low").origin.y,
        run_with(page, "c").origin.y
    ));
}

#[test]
fn rows_joined_by_a_rowspan_move_to_the_next_page_together() {
    let (pitch, _) = line_metrics();
    let table = "<table cellspacing=0 cellpadding=0>
         <tr><td rowspan=2>span</td><td>one</td></tr><tr><td>two</td></tr></table>";
    // Room for one row.
    let pages = below_filler(1.5 * pitch, table);
    assert_eq!(page_of(&pages, "one"), 1);
    assert_eq!(page_of(&pages, "span"), 1);
}

#[test]
fn rowspan_zero_and_overlong_spans_stop_at_the_row_group() {
    let pages = layout(
        "<body style='margin:0'><table cellspacing=0 cellpadding=0>
         <tbody><tr><td rowspan=0>a</td><td>b</td></tr><tr><td>c</td></tr></tbody>
         <tbody><tr><td>d</td><td rowspan=5>e</td></tr></tbody></table>",
    );
    let page = &pages[0];
    // c shares b's column; d starts the new group in the first column.
    assert!(approx(
        run_with(page, "c").origin.x,
        run_with(page, "b").origin.x
    ));
    assert!(approx(
        run_with(page, "d").origin.x,
        run_with(page, "a").origin.x
    ));
    assert!(approx(
        run_with(page, "e").origin.x,
        run_with(page, "b").origin.x
    ));
}

#[test]
fn collapsed_borders_beside_a_rowspan_are_drawn_once() {
    let cell = "style='border:1pt solid red'";
    let pages = layout(&format!(
        "<table style='border-collapse:collapse'>
         <tr><td rowspan=2 {cell}>a</td><td {cell}>b</td></tr><tr><td {cell}>c</td></tr>"
    ));
    let lines = distinct_fills(&pages[0]);
    let vertical: Vec<&Rect> = lines
        .iter()
        .filter(|r| approx(r.size.width, 1.0) && r.size.height > 1.0)
        .collect();
    // Three vertical lines (left of a, between a and b/c, right), each in
    // two row segments that meet with no gap, overlapping only where they
    // cross the 1pt horizontal line between the rows.
    let mut xs: Vec<f32> = vertical.iter().map(|r| r.origin.x).collect();
    xs.sort_by(f32::total_cmp);
    xs.dedup_by(|a, b| approx(*a, *b));
    assert_eq!(xs.len(), 3, "{vertical:?}");
    for x in xs {
        let at: Vec<_> = vertical.iter().filter(|r| approx(r.origin.x, x)).collect();
        assert_eq!(at.len(), 2);
        let first_bottom = at[0].origin.y + at[0].size.height;
        assert!(at[1].origin.y <= first_bottom && first_bottom <= at[1].origin.y + 1.01);
    }
    // No line between a's two rows.
    let horizontal = lines.iter().filter(|r| approx(r.size.height, 1.0)).count();
    assert_eq!(horizontal, 5, "{lines:?}");
}

const BLUE: Color = Color::rgb(0, 0, 255);

/// The lines of `color` on the first page: (width, height) of each.
fn lines_of(pages: &[Page], color: Color) -> Vec<Rect> {
    distinct_fills(&pages[0])
        .into_iter()
        .filter(|rect| {
            fills(&pages[0])
                .iter()
                .any(|f| f.rect == *rect && f.color == color)
        })
        .collect()
}

#[test]
fn collapsed_tables_draw_row_and_group_borders() {
    let html = |collapse: &str| {
        format!(
            "<style>table {{ border-collapse: {collapse} }}
             tr {{ border-bottom: 2pt solid red }} tbody {{ border-top: 3pt solid blue }}</style>
             <table><thead><tr><th>h</th><th>h</th></tr></thead>
             <tbody><tr><td>a</td><td>b</td></tr></tbody></table>"
        )
    };
    let pages = layout(&html("collapse"));
    // The header row's bottom edge is the body's top: the wider blue
    // border wins it, over both columns. The body row's bottom is red.
    let blue = lines_of(&pages, BLUE);
    assert_eq!(blue.len(), 2, "{blue:?}");
    assert!(blue.iter().all(|r| approx(r.size.height, 3.0)));
    let red = lines_of(&pages, RED);
    assert_eq!(red.len(), 2, "{red:?}");
    assert!(red.iter().all(|r| approx(r.size.height, 2.0)));
    // Separated borders: rows and groups have none.
    assert_eq!(fills(&layout(&html("separate"))[0]), []);
}

#[test]
fn the_widest_border_wins_a_shared_edge() {
    let pages = layout(
        "<table style='border-collapse:collapse; border:3pt solid blue'>
         <col style='border-right:2pt solid blue'>
         <tr><td style='border:1pt solid red'>a</td><td style='border:1pt solid red'>b</td>
         <td style='border:1pt solid red; border-bottom:hidden'>c</td></tr></table>",
    );
    let red = lines_of(&pages, RED);
    let blue = lines_of(&pages, BLUE);
    // Outside: the table's 3pt lines (left, top in each of the three
    // columns, bottom of a and b, right). Between a and b: the column's
    // 2pt line. Only the line between b and c is the cells' 1pt red; c's
    // bottom is hidden, which suppresses the table's border there.
    assert_eq!(red.len(), 1, "{red:?}");
    assert!(approx(red[0].size.width, 1.0));
    let mut vertical: Vec<&Rect> = blue
        .iter()
        .filter(|r| r.size.height > r.size.width)
        .collect();
    vertical.sort_by(|a, b| a.origin.x.total_cmp(&b.origin.x));
    let widths: Vec<f32> = vertical.iter().map(|r| r.size.width).collect();
    assert_eq!(widths, [3.0, 2.0, 3.0], "{blue:?}");
    let horizontal: Vec<&Rect> = blue
        .iter()
        .filter(|r| r.size.width > r.size.height)
        .collect();
    let top = horizontal
        .iter()
        .map(|r| r.origin.y)
        .fold(f32::MAX, f32::min);
    let bottom = horizontal.iter().filter(|r| r.origin.y > top + 1.0).count();
    assert_eq!(bottom, 2, "{blue:?}");
}

/// The left and right edges of `word` on a page, from its glyphs: words
/// in one style share a run with their neighbors.
fn word_edges(page: &Page, word: &str) -> (f32, f32) {
    let run = run_with(page, word);
    let covered = run.covered_text();
    let offset = covered.as_ptr() as usize - run.text.as_ptr() as usize;
    let start = offset + covered.find(word).expect("word in run");
    let end = start + word.len();
    let mut x = run.origin.x;
    let (mut left, mut right) = (f32::MAX, f32::MIN);
    for glyph in &run.glyphs {
        if glyph.text_range.start >= start && glyph.text_range.start < end {
            left = left.min(x);
            right = right.max(x + glyph.advance);
        }
        x += glyph.advance;
    }
    (left, right)
}

fn fills_of(page: &Page, color: Color) -> Vec<Rect> {
    fills(page)
        .into_iter()
        .filter(|fill| fill.color == color)
        .map(|fill| fill.rect)
        .collect()
}

#[test]
fn inline_backgrounds_cover_their_text_and_paint_first() {
    let pages =
        layout("<body style='margin:0'><p>Paid: <span style='background:red'>done</span> now");
    let page = &pages[0];
    let backgrounds = fills_of(page, RED);
    assert_eq!(backgrounds.len(), 1, "{backgrounds:?}");
    let background = backgrounds[0];
    let (left, right) = word_edges(page, "done");
    assert!(approx(background.origin.x, left), "{background:?} {left}");
    assert!(approx(background.origin.x + background.size.width, right));
    // The font's ascent and descent around the baseline.
    let baseline = run_with(page, "done").origin.y;
    assert!(background.origin.y < baseline - 8.0, "{background:?}");
    assert!(background.origin.y + background.size.height > baseline + 2.0);
    assert!(
        background.size.height < 18.0,
        "not the line height: {background:?}"
    );
    let fill = paint_index(page, |item| matches!(item, Item::Fill(_)));
    let text = paint_index(page, |item| matches!(item, Item::Text(_)));
    assert!(fill < text, "background painted first");
}

#[test]
fn inline_padding_borders_and_margins_take_space_in_the_line() {
    let plain = layout("<body style='margin:0'><p>Paid: <span>done</span> now");
    let boxed = layout(
        "<body style='margin:0'><p>Paid: <span style='padding:3pt 10pt; margin:0 5pt;
         border:2pt solid blue'>done</span> now",
    );
    let shift = |word: &str| word_edges(&boxed[0], word).0 - word_edges(&plain[0], word).0;
    assert!(approx(shift("done"), 17.0), "{}", shift("done"));
    assert!(approx(shift("now"), 34.0), "{}", shift("now"));

    let (left, right) = word_edges(&boxed[0], "done");
    let blue = Color::rgb(0, 0, 255);
    let borders = fills_of(&boxed[0], blue);
    assert_eq!(borders.len(), 4, "{borders:?}");
    let outer_left = borders.iter().map(|r| r.origin.x).fold(f32::MAX, f32::min);
    let outer_right = borders
        .iter()
        .map(|r| r.origin.x + r.size.width)
        .fold(f32::MIN, f32::max);
    assert!(approx(outer_left, left - 12.0), "{outer_left} {left}");
    assert!(approx(outer_right, right + 12.0), "{outer_right} {right}");
    // Vertical padding and borders reach above and below the text.
    let top = borders.iter().map(|r| r.origin.y).fold(f32::MAX, f32::min);
    let unpadded =
        layout("<body style='margin:0'><p>Paid: <span style='background:red'>done</span>");
    let content_top = fills_of(&unpadded[0], RED)[0].origin.y;
    assert!(approx(top, content_top - 5.0), "{top} {content_top}");
}

#[test]
fn vertical_padding_does_not_change_line_height() {
    let html = |padding: &str| {
        format!(
            "<body style='margin:0'><p style='width:80pt'>Paid <span
             style='padding:{padding}; background:red'>done</span> and some more words after</p>"
        )
    };
    let plain = layout(&html("0"));
    let padded = layout(&html("10pt 0"));
    assert_eq!(
        run_with(&plain[0], "after").origin.y,
        run_with(&padded[0], "after").origin.y
    );
}

#[test]
fn wrapped_inline_boxes_get_their_side_edges_on_the_first_and_last_lines() {
    let pages = layout(
        "<body style='margin:0'><div style='width:70pt'>Note: <span style='background:red;
         border-left:2pt solid blue; border-right:3pt solid blue; padding:0 4pt'>aaaa bbbb
         cccc dddd</span> end",
    );
    let page = &pages[0];
    let backgrounds = fills_of(page, RED);
    assert!(backgrounds.len() >= 3, "one per line: {backgrounds:?}");
    let blue = fills_of(page, Color::rgb(0, 0, 255));
    let widths: Vec<f32> = blue.iter().map(|r| r.size.width).collect();
    assert_eq!(blue.len(), 2, "{blue:?}");
    assert!(
        approx(widths[0], 2.0) && approx(widths[1], 3.0),
        "{widths:?}"
    );
    // The left edge is on the first line, beside the first word, and the
    // right one on the last line, beside the last.
    let (aaaa, _) = word_edges(page, "aaaa");
    let (_, dddd) = word_edges(page, "dddd");
    assert!(
        approx(blue[0].origin.x + 2.0 + 4.0, aaaa),
        "{blue:?} {aaaa}"
    );
    assert!(approx(blue[1].origin.x - 4.0, dddd), "{blue:?} {dddd}");
    assert!(blue[0].origin.y < blue[1].origin.y);
    // Lines that the box continues on start at the text, with no padding;
    // lines it continues from end with it, without the space.
    let (bbbb, _) = word_edges(page, "bbbb");
    let middle: Vec<&Rect> = backgrounds
        .iter()
        .filter(|r| r.origin.y > blue[0].origin.y + 1.0 && r.origin.y < blue[1].origin.y - 1.0)
        .collect();
    for line in middle {
        assert!(line.origin.x <= bbbb + 0.05, "{line:?}");
    }
}

#[test]
fn inline_padding_stays_with_its_text_at_a_line_break() {
    // At each width the span may start a line or end one; its padding
    // never sits alone at the end of a line.
    for width in (40..120).step_by(3) {
        let html = format!(
            "<body style='margin:0'><div style='width:{width}pt'>Total due <span
             style='background:red; padding:0 8pt'>12500</span> paid</div>"
        );
        let pages = layout(&html);
        let page = &pages[0];
        let (left, right) = word_edges(page, "12500");
        let backgrounds = fills_of(page, RED);
        assert_eq!(backgrounds.len(), 1, "width {width}: {backgrounds:?}");
        let background = backgrounds[0];
        assert!(approx(background.origin.x + 8.0, left), "width {width}");
        assert!(approx(
            background.origin.x + background.size.width - 8.0,
            right
        ));
    }
}

#[test]
fn nested_inline_boxes_paint_outside_in() {
    let blue = Color::rgb(0, 0, 255);
    let pages = layout(
        "<body style='margin:0'><p><span style='background:red; padding:0 6pt'>outer
         <span style='background:blue; padding:0 3pt'>inner</span> text</span>",
    );
    let page = &pages[0];
    let order: Vec<Color> = fills(page).iter().map(|f| f.color).collect();
    assert_eq!(order, vec![RED, blue]);
    let red = fills_of(page, RED)[0];
    let inner = fills_of(page, blue)[0];
    assert!(red.origin.x < inner.origin.x);
    assert!(inner.origin.x + inner.size.width < red.origin.x + red.size.width);
    let (left, _) = word_edges(page, "inner");
    assert!(approx(inner.origin.x + 3.0, left));
}

#[test]
fn spaces_collapse_across_inline_edges() {
    let pages = layout(
        "<body style='margin:0'><p>a <span style='padding:0 5pt'> b </span> c
         <span style='padding:0 5pt'></span> d",
    );
    let page = &pages[0];
    for run in runs(page) {
        assert!(!run.text.contains("  "), "{:?}", run.text);
    }
    assert_eq!(words(page).join(" ").split_whitespace().count(), 4);
}

#[test]
fn inline_padding_counts_when_sizing_a_cell() {
    let pages = layout(
        "<body style='margin:0'><table style='border-collapse:collapse'><tr>
         <td style='border:1pt solid blue'><span style='background:red; padding:0 20pt'>x</span>",
    );
    let page = &pages[0];
    let background = fills_of(page, RED)[0];
    let (left, right) = word_edges(page, "x");
    assert!(approx(background.size.width, right - left + 40.0));
    let blue = fills_of(page, Color::rgb(0, 0, 255));
    let cell_right = blue
        .iter()
        .map(|r| r.origin.x + r.size.width)
        .fold(f32::MIN, f32::max);
    assert!(background.origin.x + background.size.width <= cell_right);
}

const IB: &str = "display:inline-block";

#[test]
fn inline_blocks_shrink_to_fit_and_sit_on_their_last_baseline() {
    let pages = layout(&format!(
        "<body style='margin:0'><p>Total <span style='{IB}; border:1pt solid red'>a<br>bb</span>
         due"
    ));
    let page = &pages[0];
    let borders = fills_of(page, RED);
    assert_eq!(borders.len(), 4, "{borders:?}");
    let left = borders.iter().map(|r| r.origin.x).fold(f32::MAX, f32::min);
    let right = borders
        .iter()
        .map(|r| r.origin.x + r.size.width)
        .fold(f32::MIN, f32::max);
    let (bb_left, bb_right) = word_edges(page, "bb");
    // As wide as its widest line.
    assert!(approx(left + 1.0, bb_left), "{left} {bb_left}");
    assert!(approx(right - 1.0, bb_right), "{right} {bb_right}");
    let (_, total) = word_edges(page, "Total");
    assert!(left > total && left < total + 5.0, "after the space");
    // Its last line shares the baseline of the text around it.
    assert!(approx(
        run_with(page, "bb").origin.y,
        run_with(page, "due").origin.y
    ));
    let first = runs(page)
        .into_iter()
        .find(|run| run.covered_text().trim() == "a")
        .expect("first line");
    assert!(first.origin.y < run_with(page, "bb").origin.y);
}

#[test]
fn inline_blocks_take_their_width_and_lay_out_inside_it() {
    let pages = layout(&format!(
        "<body style='margin:0'><p>Sum <span style='{IB}; width:100pt; padding:0 5pt;
         background:red; text-align:right'>1250</span> only"
    ));
    let page = &pages[0];
    let background = fills_of(page, RED)[0];
    assert!(approx(background.size.width, 110.0), "{background:?}");
    let (_, amount) = word_edges(page, "1250");
    assert!(approx(amount, background.origin.x + 105.0), "{amount}");
    let (only, _) = word_edges(page, "only");
    assert!(only > background.origin.x + 110.0);
}

#[test]
fn an_empty_inline_block_sits_on_its_bottom_edge() {
    let pages = layout(&format!(
        "<body style='margin:0'><p>Signed <span style='{IB}; width:80pt; height:30pt;
         border-bottom:1pt solid red'></span> date"
    ));
    let page = &pages[0];
    let line = fills_of(page, RED)[0];
    let baseline = run_with(page, "date").origin.y;
    assert!(
        approx(line.origin.y + line.size.height, baseline),
        "{line:?}"
    );
    assert!(approx(line.size.width, 80.0));
}

#[test]
fn inline_blocks_move_to_the_next_line_whole() {
    let pages = layout(&format!(
        "<body style='margin:0'><div style='width:90pt'>Some words here <span
         style='{IB}'>stamp text</span>"
    ));
    let page = &pages[0];
    let stamp = run_with(page, "stamp").origin.y;
    assert_eq!(run_with(page, "text").origin.y, stamp);
    assert!(stamp > run_with(page, "Some").origin.y);
}

#[test]
fn decorations_do_not_reach_into_inline_blocks() {
    let pages = layout(&format!(
        "<body style='margin:0'><p style='text-decoration:underline'>Paid <span
         style='{IB}'>stamp</span>"
    ));
    let page = &pages[0];
    let (left, right) = word_edges(page, "stamp");
    for line in fills(page) {
        let end = line.rect.origin.x + line.rect.size.width;
        assert!(
            end <= left + 0.05 || line.rect.origin.x >= right,
            "{line:?}"
        );
    }
    assert!(!fills(page).is_empty(), "Paid is underlined");
}

#[test]
fn inline_blocks_fit_a_cell_sized_to_them() {
    let pages = layout(&format!(
        "<body style='margin:0'><table><tr><td>x <span style='{IB}; border:2pt solid red;
         padding:4pt 10pt'>PAID</span></td><td>y</td></tr></table>"
    ));
    let page = &pages[0];
    assert!(approx(
        run_with(page, "x").origin.y,
        run_with(page, "PAID").origin.y
    ));
}

#[test]
fn superscripts_and_subscripts_shift_off_the_baseline() {
    let pages = layout("<body style='margin:0'><p>m<sup>2</sup> H<sub>3</sub>O");
    let page = &pages[0];
    let baseline = run_with(page, "m").origin.y;
    let sup = run_with(page, "2");
    let sub = run_with(page, "3");
    assert!(sup.origin.y < baseline - 2.0, "{} {baseline}", sup.origin.y);
    assert!(sub.origin.y > baseline + 1.0, "{} {baseline}", sub.origin.y);
    assert!(sup.font_size < run_with(page, "m").font_size, "smaller");
    assert!(approx(run_with(page, "O").origin.y, baseline));
}

#[test]
fn text_nested_in_a_superscript_stays_raised() {
    // vertical-align is not inherited: the bold text sits on the
    // superscript's baseline, not the line's.
    let pages = layout("<body style='margin:0'><p>x<sup>a<b>bold</b></sup> y");
    let page = &pages[0];
    let raised = run_with(page, "a").origin.y;
    assert!(raised < run_with(page, "x").origin.y - 2.0);
    assert!(approx(run_with(page, "bold").origin.y, raised));
}

#[test]
fn text_runs_across_elements_that_do_not_change_its_style() {
    let pages =
        layout("<body style='margin:0'><p>a<span>b</span><em style='font-style:normal'>c</em>");
    assert_eq!(words(&pages[0]), ["abc"]);
}

#[test]
fn inline_backgrounds_follow_raised_text() {
    let html = |align: &str| {
        format!(
            "<body style='margin:0'><p>x<span style='vertical-align:{align}'><span
             style='background:red'>n</span></span>"
        )
    };
    // Positions relative to the line's baseline: raised text makes the
    // line taller.
    let above_baseline = |pages: &[Page]| {
        let page = &pages[0];
        let baseline = runs(page)[0].origin.y;
        let n = run_with(page, "n");
        let n_x = n.origin.x + n.glyphs.iter().map(|g| g.advance).sum::<f32>();
        let background = fills_of(page, RED)[0];
        assert!(background.origin.x < n_x);
        (baseline - n.origin.y, baseline - background.origin.y)
    };
    let (flat_text, flat_background) = above_baseline(&layout(&html("baseline")));
    let (raised_text, raised_background) = above_baseline(&layout(&html("super")));
    assert!(approx(flat_text, 0.0));
    assert!(raised_text > 2.0, "{raised_text}");
    assert!(approx(raised_background - flat_background, raised_text));
}

/// The page's text, run by run, joined with spaces.
fn page_text(page: &Page) -> String {
    runs(page)
        .iter()
        .map(|run| run.covered_text())
        .collect::<Vec<_>>()
        .join("|")
}

#[test]
fn before_and_after_add_text_around_the_content() {
    let pages = layout(
        "<style>p::before { content: 'Total: ' } p::after { content: ' only'; color: red }</style>
         <p>Rs. 500</p>",
    );
    let page = &pages[0];
    assert_eq!(page_text(page), "Total: Rs. 500| only");
    assert_eq!(run_with(page, "only").color, RED);
}

#[test]
fn counters_number_headings_in_document_order() {
    let pages = layout(
        "<style>body { counter-reset: sec } h2 { counter-increment: sec; counter-reset: sub }
         h3 { counter-increment: sub } h2::before { content: counter(sec) '. ' }
         h3::before { content: counter(sec) '.' counter(sub) ' ' }
         .hidden { display: none }</style>
         <h2>A</h2><h3>a</h3><h3>b</h3><h2 class=hidden>X</h2><h2>B</h2><h3>c</h3>",
    );
    let text: Vec<&str> = words(&pages[0]);
    assert_eq!(text, ["1. A", "1.1 a", "1.2 b", "2. B", "2.1 c"]);
}

#[test]
fn nested_counters_and_counter_styles() {
    let pages = layout(
        "<style>ol { counter-reset: item; list-style: none }
         li { counter-increment: item } li::before { content: counters(item, '.') ') ' }
         .ne li::before { content: counter(item, devanagari) '. ' }</style>
         <ol><li>One<ol><li>Sub</li></ol></li><li>Two</li></ol>
         <ol class=ne><li>क</li><li>ख</li></ol>",
    );
    let text = words(&pages[0]);
    assert_eq!(text[..3], ["1) One", "1.1) Sub", "2) Two"]);
    // Devanagari digits and the full stop come from different fonts.
    let joined: String = runs(&pages[0]).iter().map(|r| r.covered_text()).collect();
    assert!(joined.ends_with("१. क२. ख"), "{joined}");
}

#[test]
fn counters_in_measured_table_cells_count_once() {
    // Cells are measured before they are laid out; the counter must not
    // advance twice.
    let pages = layout(
        "<style>table { counter-reset: row } tr { counter-increment: row }
         td.n::before { content: counter(row) }</style>
         <table><tr><td class=n></td><td>a</td></tr><tr><td class=n></td><td>b</td></tr></table>",
    );
    assert_eq!(words(&pages[0]), ["1", "a", "2", "b"]);
}

#[test]
fn quotes_attributes_and_block_pseudo_elements() {
    let pages = layout(
        "<style>a::after { content: ' (' attr(href) ')' }
         div::before { content: 'NOTE'; display: block; background: red }</style>
         <p><q>pay <q>now</q></q> <a href='https://x.test'>here</a></p><div>Body</div>",
    );
    let page = &pages[0];
    let text = page_text(page);
    assert!(
        text.contains("\u{201C}pay \u{2018}now\u{2019}\u{201D}"),
        "{text}"
    );
    assert!(text.contains("(https://x.test)"), "{text}");
    let note = run_with(page, "NOTE");
    let body = run_with(page, "Body");
    assert!(note.origin.y < body.origin.y, "a block of its own");
    let background = fills_of(page, RED)[0];
    assert!(background.size.width > 100.0, "full width");
}

fn links(page: &Page) -> Vec<&Link> {
    page.items
        .iter()
        .filter_map(|item| match item {
            Item::Link(link) => Some(link),
            _ => None,
        })
        .collect()
}

fn marks(pages: &[Page]) -> Vec<(usize, &Mark)> {
    pages
        .iter()
        .enumerate()
        .flat_map(|(index, page)| {
            page.items.iter().filter_map(move |item| match item {
                Item::Mark(mark) => Some((index, mark)),
                _ => None,
            })
        })
        .collect()
}

#[test]
fn headings_and_anchors_are_marked_where_they_land() {
    let pages = layout(
        "<body style='margin:0'><h1 id=top>Tax   <b>Invoice</b></h1><p style='height:2000px'></p>
         <h2>Terms</h2><p>Pay <a name=pay>here</a> <span id=late>soon</span>.</p>",
    );
    let marks = marks(&pages);
    let kinds: Vec<(usize, &MarkKind)> = marks.iter().map(|(page, m)| (*page, &m.kind)).collect();
    let anchor = |name: &str| MarkKind::Anchor(name.into());
    let heading = |level, title: &str| MarkKind::Heading {
        level,
        title: title.into(),
    };
    let last = pages.len() - 1;
    assert!(last > 0);
    assert_eq!(
        kinds,
        [
            (0, &anchor("top")),
            (0, &heading(1, "Tax Invoice")),
            (last, &heading(2, "Terms")),
            (last, &anchor("pay")),
            (last, &anchor("late")),
        ]
    );
    let terms = run_with(&pages[last], "Terms");
    let (_, mark) = marks[2];
    assert!(mark.origin.y <= terms.origin.y && mark.origin.y > terms.origin.y - 20.0);
}

#[test]
fn links_cover_their_text_on_each_line() {
    let pages = layout(
        "<body style='margin:0'><div style='width:80pt'>See <a href='#terms'>the payment
         terms below</a> and <a href='https://example.com/a b'>this</a>.</div>",
    );
    let page = &pages[0];
    let links = links(page);
    let internal: Vec<&&Link> = links
        .iter()
        .filter(|l| l.target == LinkTarget::Anchor("terms".into()))
        .collect();
    assert!(internal.len() >= 2, "one per line: {links:?}");
    let (left, _) = word_edges(page, "the");
    assert!(approx(internal[0].rect.origin.x, left));
    let (_, right) = word_edges(page, "below");
    assert!(
        internal
            .iter()
            .any(|l| approx(l.rect.origin.x + l.rect.size.width, right))
    );
    assert!(
        links
            .iter()
            .any(|l| l.target == LinkTarget::Url("https://example.com/a%20b".into()))
    );
}

#[test]
fn link_targets_resolve_against_the_document() {
    let base = Url::parse("file:///docs/invoice.html").unwrap();
    let fetcher = Fetcher::new(&DataUrls, Some(base));
    let html = "<p><a href='terms.html'>a</a> <a href='invoice.html#%E0%A4%95'>b</a>
                <a href='javascript:void(0)'>c</a> <a href=''>d</a> <a>e</a>";
    let pages = layout_fetching(
        &mut Engine::new(),
        html,
        &LayoutOptions::default(),
        &fetcher,
    );
    let targets: Vec<&LinkTarget> = links(&pages[0]).iter().map(|l| &l.target).collect();
    assert_eq!(
        targets,
        [
            &LinkTarget::Url("file:///docs/terms.html".into()),
            &LinkTarget::Anchor("क".into()),
            &LinkTarget::Url("file:///docs/invoice.html".into()),
        ]
    );
}

#[test]
fn block_links_make_their_lines_clickable() {
    let pages = layout(
        "<body style='margin:0'><a href='#top' style='display:block; width:200pt'>Back to top</a>",
    );
    let links = links(&pages[0]);
    assert_eq!(links.len(), 1, "{links:?}");
    assert!(approx(links[0].rect.size.width, 200.0));
}

fn shapes(page: &Page) -> Vec<&Shape> {
    page.items
        .iter()
        .filter_map(|item| match item {
            Item::Shape(shape) => Some(shape),
            _ => None,
        })
        .collect()
}

#[test]
fn dashed_and_dotted_borders_are_stroked_along_their_edges() {
    let pages = layout(
        "<body style='margin:0'><div style='width:100pt; height:20pt; border:2pt dashed red;
         border-left-style: dotted'></div>",
    );
    let page = &pages[0];
    assert!(fills(page).is_empty(), "no solid edges");
    let strokes: Vec<_> = shapes(page)
        .iter()
        .map(|shape| match shape.paint {
            ShapePaint::Stroke {
                width, dash, round, ..
            } => (width, dash.expect("dashed"), round),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(strokes.len(), 4);
    // The top edge, 104pt long: dashes of about 6pt (three widths).
    let (width, (dash, gap), round) = strokes[0];
    assert_eq!(width, 2.0);
    assert!(
        !round && (dash - 6.0).abs() < 0.5 && dash == gap,
        "{dash} {gap}"
    );
    // The left edge: dots, round caps on zero-length dashes.
    let (_, (dot, spacing), round) = strokes[3];
    assert!(
        round && dot == 0.0 && (spacing - 4.0).abs() < 0.5,
        "{spacing}"
    );
}

#[test]
fn double_borders_are_two_lines() {
    let pages = layout(
        "<body style='margin:0'><div style='width:100pt; height:20pt; border-top:6pt double red'></div>",
    );
    let lines = fills_of(&pages[0], RED);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines.iter().all(|r| approx(r.size.height, 2.0)));
    assert!(approx(lines[1].origin.y - lines[0].origin.y, 4.0));
}

#[test]
fn rounded_boxes_are_shapes() {
    let pages = layout(
        "<body style='margin:0'><div style='width:100pt; height:20pt; background:red;
         border:2pt solid blue; border-radius:8pt'></div>",
    );
    let page = &pages[0];
    assert!(fills(page).is_empty());
    let shapes = shapes(page);
    assert_eq!(shapes.len(), 2);
    assert_eq!(
        shapes[0].paint,
        ShapePaint::Fill {
            color: RED,
            even_odd: false
        }
    );
    assert_eq!(
        shapes[1].paint,
        ShapePaint::Fill {
            color: BLUE,
            even_odd: true
        },
        "the border is a ring"
    );
    let curves = shapes[0]
        .path
        .iter()
        .filter(|op| matches!(op, PathOp::Cubic(..)))
        .count();
    assert_eq!(curves, 4);
}

#[test]
fn rounded_corners_are_square_where_a_box_breaks_across_pages() {
    let options = LayoutOptions {
        page_size: Size::new(200.0, 200.0),
        margins: EdgeSizes::uniform(10.0),
        ..LayoutOptions::default()
    };
    let lines = "<p>line</p>".repeat(12);
    let html = format!(
        "<body style='margin:0'><div style='background:red; border-radius:10pt'>{lines}</div>"
    );
    let pages = layout_with(&html, &options);
    assert_eq!(pages.len(), 2);
    for page in &pages {
        let shape = shapes(page)[0];
        // Two rounded corners on each page, the others square: a
        // degenerate curve has its control point on its end point.
        let rounded = shape
            .path
            .windows(2)
            .filter(|ops| match ops {
                [PathOp::Line(a), PathOp::Cubic(_, _, c)]
                | [PathOp::Move(a), PathOp::Cubic(_, _, c)] => a != c,
                _ => false,
            })
            .count();
        assert_eq!(rounded, 2, "{:?}", shape.path);
    }
}

#[test]
fn collapsed_table_borders_keep_their_style() {
    let pages = layout(
        "<body style='margin:0'><table style='border-collapse:collapse'>
         <tr><td style='border-bottom:1pt dashed red'>a</td></tr>
         <tr><td style='border-top:6pt double blue'>b</td></tr></table>",
    );
    let page = &pages[0];
    // The line between the rows is drawn by both.
    let blue: Vec<Rect> = distinct_fills(page)
        .into_iter()
        .filter(|rect| fills_of(page, BLUE).contains(rect))
        .collect();
    assert_eq!(blue.len(), 2, "double: {blue:?}");
    // The dashed line lost to the wider double one on their shared edge.
    assert!(shapes(page).is_empty());

    let pages = layout(
        "<body style='margin:0'><table style='border-collapse:collapse'>
         <tr><td style='border-bottom:1pt dashed red'>a</td></tr><tr><td>b</td></tr></table>",
    );
    let shapes = shapes(&pages[0]);
    // Drawn by both rows, identically.
    assert!(!shapes.is_empty() && shapes.iter().all(|s| *s == shapes[0]));
    assert!(matches!(
        shapes[0].paint,
        ShapePaint::Stroke { dash: Some(_), .. }
    ));
}

/// Whether `ch` is drawn with a real glyph (not the font's missing-glyph
/// box, glyph 0) on the page.
fn has_glyph(page: &Page, ch: char) -> bool {
    runs(page).iter().any(|run| {
        run.glyphs
            .iter()
            .any(|glyph| glyph.id != 0 && run.text[glyph.text_range.clone()].contains(ch))
    })
}

#[test]
fn a_danda_after_latin_text_finds_a_devanagari_font() {
    // parley gives the danda the script of the Latin text before it; the
    // Devanagari fallback font is the last resort.
    let pages = layout("<p>Total: Rs. 500।</p>");
    assert!(has_glyph(&pages[0], '।'));
}

#[test]
fn symbols_in_latin_text_find_a_symbol_font() {
    let mut engine = Engine::new();
    let installed = super::flow::SYMBOL_FONTS
        .iter()
        .any(|name| engine.fonts.collection.family_id(name).is_some());
    if !installed {
        eprintln!("skipped: no symbol font installed");
        return;
    }
    let pages = layout_fetching(
        &mut engine,
        "<p>Paid ✔ in full</p>",
        &LayoutOptions::default(),
        &Fetcher::data_only(),
    );
    assert!(has_glyph(&pages[0], '✔'));
}

#[track_caller]
fn assert_rect(a: Rect, b: Rect) {
    let close = approx(a.origin.x, b.origin.x)
        && approx(a.origin.y, b.origin.y)
        && approx(a.size.width, b.size.width)
        && approx(a.size.height, b.size.height);
    assert!(close, "{a:?} != {b:?}");
}

fn image_fills(page: &Page) -> Vec<&ImageFill> {
    page.items
        .iter()
        .filter_map(|item| match item {
            Item::ImageFill(fill) => Some(fill),
            _ => None,
        })
        .collect()
}

#[test]
fn background_images_tile_from_the_padding_box() {
    let pages = layout(&format!(
        "<body style='margin:0'><div style='width:100pt; height:50pt; border:2pt solid;
         background:#eee url({PNG})'></div>"
    ));
    let page = &pages[0];
    let fills = image_fills(page);
    assert_eq!(fills.len(), 1);
    let fill = fills[0];
    // The PNG is 2×1 pixels: 1.5×0.75pt.
    assert_rect(fill.tile, Rect::new(M + 2.0, M + 2.0, 1.5, 0.75));
    assert_rect(fill.rect, Rect::new(M, M, 104.0, 54.0));
    assert_eq!(fill.step, fill.tile.size);
    // Painted after the color, before the borders.
    let color = paint_index(
        page,
        |item| matches!(item, Item::Fill(f) if f.rect.size.width > 100.0),
    );
    let image = paint_index(page, |item| matches!(item, Item::ImageFill(_)));
    let border = paint_index(
        page,
        |item| matches!(item, Item::Fill(f) if f.rect.size.height == 2.0),
    );
    assert!(color < image && image < border);
}

#[test]
fn background_images_are_positioned_and_sized() {
    let fill = |css: &str| {
        let pages = layout(&format!(
            "<body style='margin:0'><div style='width:100pt; height:100pt;
             background:url({PNG}) {css}'></div>"
        ));
        let fill = image_fills(&pages[0])[0].clone();
        let at = |r: Rect| Rect::new(r.origin.x - M, r.origin.y - M, r.size.width, r.size.height);
        (at(fill.tile), at(fill.rect))
    };
    let (tile, rect) = fill("no-repeat right 10pt bottom 5pt / 20pt");
    assert_rect(tile, Rect::new(70.0, 85.0, 20.0, 10.0));
    assert_rect(rect, tile);
    let (tile, rect) = fill("repeat-x 0 50% / 20pt");
    assert_rect(tile, Rect::new(0.0, 45.0, 20.0, 10.0));
    assert_rect(rect, Rect::new(0.0, 45.0, 100.0, 10.0));
    assert_rect(
        fill("center / cover").0,
        Rect::new(-50.0, 0.0, 200.0, 100.0),
    );
    assert_rect(
        fill("center / contain").0,
        Rect::new(0.0, 25.0, 100.0, 50.0),
    );
}

#[test]
fn background_images_line_up_across_pages() {
    let options = LayoutOptions {
        page_size: Size::new(200.0, 200.0),
        margins: EdgeSizes::uniform(10.0),
        ..LayoutOptions::default()
    };
    let lines = "<p>line</p>".repeat(12);
    let html = format!(
        "<body style='margin:0'><div style='background:url({PNG}) no-repeat 0 0 / 30pt'>{lines}</div>"
    );
    let pages = layout_with(&html, &options);
    assert_eq!(pages.len(), 2);
    assert_eq!(
        image_fills(&pages[0]).len(),
        1,
        "the one copy is on the first page"
    );
    assert!(image_fills(&pages[1]).is_empty());

    // Repeated, the copies on the second page continue from the box's
    // top on the first.
    let html = format!(
        "<body style='margin:0'><div style='background:url({PNG}) 0 0 / 30pt'>{lines}</div>"
    );
    let pages = layout_with(&html, &options);
    let first = image_fills(&pages[0])[0];
    let second = image_fills(&pages[1])[0];
    assert!(approx(first.tile.origin.y, first.rect.origin.y));
    assert!(
        second.tile.origin.y < second.rect.origin.y - 1.0,
        "{second:?}"
    );
}

#[test]
fn background_images_on_inline_elements_and_cells() {
    let pages = layout(&format!(
        "<body style='margin:0'><p><span style='background:url({PNG}) no-repeat; padding-left:12pt'>paid</span></p>
         <table><tr><td background='{PNG}'>a</td></tr></table>"
    ));
    assert_eq!(image_fills(&pages[0]).len(), 2);
}

#[test]
fn text_flows_around_a_left_float_and_back_under_it() {
    let text = "word ".repeat(80);
    let pages = layout(&format!(
        "<body style='margin:0'><div style='width:200pt'><div style='float:left; width:50pt;
         height:30pt; background:red'></div><p style='margin:0'>{text}</p></div>"
    ));
    let page = &pages[0];
    let float = fills_of(page, RED)[0];
    assert!(approx(float.origin.x, M) && approx(float.size.width, 50.0));
    let lines = runs(page);
    assert!(approx(lines[0].origin.x, M + 50.0), "beside the float");
    let below = lines
        .iter()
        .find(|run| run.origin.y > float.origin.y + 40.0)
        .expect("a line below the float");
    assert!(approx(below.origin.x, M), "back at the left edge");
    for run in &lines {
        assert!(right_edge(run) <= M + 200.5, "within the block");
    }
}

#[test]
fn a_right_float_shares_the_line_it_starts() {
    let pages = layout(
        "<body style='margin:0'><div style='width:300pt'><p>Total <span style='float:right'>Rs 500</span></p></div>",
    );
    let page = &pages[0];
    let total = run_with(page, "Total");
    let amount = run_with(page, "Rs");
    assert!(approx(amount.origin.y, total.origin.y), "same line");
    assert!(approx(right_edge(amount), M + 300.0), "at the right edge");
}

#[test]
fn floats_sit_side_by_side_until_they_do_not_fit() {
    let pages = layout(
        "<body style='margin:0'><div style='width:300pt'>
         <div style='float:left; width:140pt; height:20pt; background:red'></div>
         <div style='float:left; width:140pt; height:20pt; background:red'></div>
         <div style='float:left; width:140pt; height:20pt; background:red'></div></div>",
    );
    let boxes = fills_of(&pages[0], RED);
    assert_eq!(boxes.len(), 3);
    assert!(approx(boxes[1].origin.x, M + 140.0) && approx(boxes[1].origin.y, boxes[0].origin.y));
    assert!(approx(boxes[2].origin.x, M), "the third goes below");
    assert!(approx(boxes[2].origin.y, boxes[0].origin.y + 20.0));
}

#[test]
fn clear_and_clearfix_go_below_floats() {
    let pages = layout(
        "<style>.cf::after { content: ''; display: table; clear: both }</style>
         <body style='margin:0'><div class=cf style='border-bottom:1pt solid blue'>
         <div style='float:right; width:50pt; height:40pt; background:red'></div>short</div>
         <p style='clear:both; margin:0'>after</p>",
    );
    let page = &pages[0];
    let float = fills_of(page, RED)[0];
    let float_bottom = float.origin.y + float.size.height;
    let border = fills_of(page, BLUE)[0];
    assert!(
        border.origin.y >= float_bottom - 0.05,
        "the box contains its float"
    );
    assert!(run_with(page, "after").origin.y > float_bottom);
}

#[test]
fn cells_and_floated_images_and_tables() {
    let pages = layout(&format!(
        "<body style='margin:0'><table><tr><td style='background:blue'>
         <div style='float:left; width:30pt; height:60pt'></div>a</td></tr></table>
         <div style='width:300pt'><img src='{PNG}' style='float:right; width:40pt'>
         <table style='float:left; border:1pt solid red'><tr><td>x</td></tr></table>text</div>"
    ));
    let page = &pages[0];
    let cell = fills_of(page, BLUE)[0];
    assert!(
        cell.size.height >= 60.0,
        "a cell contains its floats: {cell:?}"
    );
    let image = images(page)[0];
    assert!(approx(image.origin.x + image.size.width, M + 300.0));
    let table = fills_of(page, RED);
    let table_left = table.iter().map(|r| r.origin.x).fold(f32::MAX, f32::min);
    assert!(approx(table_left, M));
    let (text, _) = word_edges(page, "text");
    let table_right = table
        .iter()
        .map(|r| r.origin.x + r.size.width)
        .fold(f32::MIN, f32::max);
    assert!(text >= table_right - 0.05, "text beside the floated table");
}

#[test]
fn absolute_boxes_are_placed_against_their_positioned_ancestor() {
    let pages = layout(
        "<body style='margin:0'><div style='position:relative; width:200pt; height:100pt;
         border:2pt solid blue'><p style='margin:0'>flow</p>
         <span style='position:absolute; right:10pt; bottom:5pt'>stamp</span>
         <div style='position:absolute; left:0; top:0; width:20pt; height:20pt; background:red'></div>
         </div><p>after</p>",
    );
    let page = &pages[0];
    // The padding box starts inside the 2pt border.
    let corner = fills_of(page, RED)[0];
    assert!(approx(corner.origin.x, M + 2.0) && approx(corner.origin.y, M + 2.0));
    let (_, right) = word_edges(page, "stamp");
    assert!(approx(right, M + 2.0 + 200.0 - 10.0), "{right}");
    let stamp = run_with(page, "stamp");
    assert!(stamp.origin.y > M + 80.0 && stamp.origin.y < M + 2.0 + 100.0);
    // Out of the flow: the paragraph is at the top, and nothing moved down.
    assert!(run_with(page, "flow").origin.y < M + 20.0);
    // Painted above the flow.
    let flow = paint_index(
        page,
        |item| matches!(item, Item::Text(r) if r.covered_text() == "flow"),
    );
    let stamp = paint_index(
        page,
        |item| matches!(item, Item::Text(r) if r.covered_text() == "stamp"),
    );
    let after = paint_index(
        page,
        |item| matches!(item, Item::Text(r) if r.covered_text() == "after"),
    );
    assert!(flow < stamp && after < stamp);
}

#[test]
fn absolute_boxes_without_a_positioned_ancestor_use_the_first_page() {
    let pages = layout(
        "<body style='margin:0'><p>text</p><div style='position:absolute; right:0; top:0;
         width:30pt; height:10pt; background:red'></div>",
    );
    let page = &pages[0];
    let content_width = page.size.width - 2.0 * M;
    let box_ = fills_of(page, RED)[0];
    assert!(approx(box_.origin.x + box_.size.width, M + content_width));
    assert!(approx(box_.origin.y, M));
}

#[test]
fn relative_boxes_shift_their_content() {
    let html = |offsets: &str| {
        format!(
            "<body style='margin:0'><div style='position:relative; {offsets}; background:red'>moved</div>"
        )
    };
    let plain = layout(&html("left:0"));
    let moved = layout(&html("left:15pt; top:4pt"));
    let a = run_with(&plain[0], "moved");
    let b = run_with(&moved[0], "moved");
    assert!(approx(b.origin.x - a.origin.x, 15.0) && approx(b.origin.y - a.origin.y, 4.0));
    let (a, b) = (fills_of(&plain[0], RED)[0], fills_of(&moved[0], RED)[0]);
    assert!(approx(b.origin.x - a.origin.x, 15.0) && approx(b.origin.y - a.origin.y, 4.0));
    // `right` moves it left when `left` is auto.
    let left = layout(&html("right:5pt"));
    assert!(approx(
        run_with(&left[0], "moved").origin.x - a.origin.x,
        -5.0
    ));
}

#[test]
fn fixed_boxes_repeat_on_every_page() {
    let lines = "<p>line</p>".repeat(120);
    let pages = layout(&format!(
        "<body style='margin:0'><div style='position:fixed; left:10pt; bottom:0;
         width:40pt; height:12pt; background:red'></div>{lines}"
    ));
    assert!(pages.len() > 1);
    for page in &pages {
        let mark = fills_of(page, RED);
        assert_eq!(mark.len(), 1);
        let bottom = page.size.height - M;
        assert!(approx(mark[0].origin.y + mark[0].size.height, bottom));
        assert!(approx(mark[0].origin.x, M + 10.0));
    }
}

#[test]
fn overflow_hidden_clips_and_contains_floats() {
    let pages = layout(
        "<body style='margin:0'><div style='width:100pt; overflow:hidden; white-space:nowrap;
         border:2pt solid blue'>a long line that does not fit in the box at all
         <div style='float:right; width:20pt; height:50pt; background:red'></div></div>",
    );
    let page = &pages[0];
    let clips: Vec<&Clip> = page
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Clip(clip) => Some(clip),
            _ => None,
        })
        .collect();
    assert!(!clips.is_empty());
    for clip in &clips {
        assert!(approx(clip.rect.origin.x, M + 2.0) && approx(clip.rect.size.width, 100.0));
    }
    // The float is inside a clip, and the box is as tall as it.
    let border_bottom = fills_of(page, BLUE)
        .iter()
        .map(|r| r.origin.y + r.size.height)
        .fold(f32::MIN, f32::max);
    assert!(border_bottom >= M + 2.0 + 50.0, "{border_bottom}");
}

#[test]
fn the_body_background_covers_every_page() {
    let lines = "<p>line</p>".repeat(120);
    let pages = layout(&format!("<body style='background:red'>{lines}"));
    assert!(pages.len() > 1);
    for page in &pages {
        let fills = fills_of(page, RED);
        assert_eq!(fills.len(), 1, "once, not again for the body box");
        let area = fills[0];
        assert!(approx(area.origin.x, M) && approx(area.origin.y, M));
        assert!(approx(area.size.width, page.size.width - 2.0 * M));
        assert!(approx(area.size.height, page.size.height - 2.0 * M));
        assert!(matches!(page.items[0], Item::Fill(_)), "painted first");
    }
}

#[test]
fn linear_gradients_run_across_the_padding_box() {
    let pages = layout(
        "<body style='margin:0'><div style='width:100pt; height:40pt;
         background:linear-gradient(to right, red, blue 75%)'></div>",
    );
    let fill = pages[0]
        .items
        .iter()
        .find_map(|item| match item {
            Item::GradientFill(fill) => Some(fill),
            _ => None,
        })
        .expect("a gradient");
    assert!(approx(fill.start.x, M) && approx(fill.end.x, M + 100.0));
    assert!(approx(fill.start.y, M + 20.0) && approx(fill.end.y, M + 20.0));
    assert_eq!(fill.stops, [(0.0, RED), (0.75, BLUE)]);
}
