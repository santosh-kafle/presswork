//! Cutting the continuous canvas into pages.

use std::rc::Rc;

use presswork_style::Side;

use crate::flow::{Canvas, RepeatedHeader, Slice};
use crate::fragment::{Item, Page};
use crate::geometry::{EdgeSizes, Rect, Size};

/// A page under construction.
struct Sheet {
    /// Canvas position shown at the top of the page's content area, below
    /// any repeated header.
    offset: f64,
    /// Height of the table header repeated at the top, if any.
    header: f32,
    boxes: Vec<Item>,
    content: Vec<Item>,
    /// Positioned boxes, painted above the content.
    top: Vec<Item>,
}

/// A repeated header taller than this fraction of the page is not
/// repeated: it would leave too little room for the rows themselves.
const MAX_HEADER_FRACTION: f32 = 0.5;

/// Places the canvas's content onto pages of `size` with `margins`.
///
/// A slice that does not fit on the current page starts a new one, unless
/// it is the first on its page (it is then too tall for any page and is
/// left to overflow). Slices kept together (see [`Keep`](crate::flow::Keep)) move to the next
/// page as a group. The space before a slice that starts a page is
/// dropped, so margins do not carry over a page break. Backgrounds and
/// borders are split where the page breaks fall.
pub(crate) fn paginate(canvas: Canvas, size: Size, margins: EdgeSizes) -> Vec<Page> {
    let available = (size.height - margins.top - margins.bottom).max(1.0);
    let sheet = |offset: f64, header: f32| Sheet {
        offset,
        header,
        boxes: Vec::new(),
        content: Vec::new(),
        top: Vec::new(),
    };

    let mut starts = page_starts(&canvas.slices, available)
        .into_iter()
        .peekable();
    let mut sheets = vec![sheet(0.0, 0.0)];
    for (index, slice) in canvas.slices.into_iter().enumerate() {
        if starts.next_if_eq(&index).is_some() {
            let header = repeated_header(&slice, available);
            let mut next = sheet(slice.top, header.as_ref().map_or(0.0, |h| h.height));
            if let Some(header) = header {
                next.content.extend(
                    header
                        .items
                        .iter()
                        .cloned()
                        .map(|item| item.translate(margins.left, margins.top)),
                );
            }
            sheets.push(next);
        }
        let current = sheets.last_mut().expect("at least one page");
        let y = margins.top + current.header + (slice.top - current.offset) as f32;
        let layer = if slice.on_top {
            &mut current.top
        } else {
            &mut current.content
        };
        layer.extend(
            slice
                .items
                .into_iter()
                .map(|item| item.translate(margins.left, y)),
        );
    }

    // Split each decoration across the pages it spans.
    for decoration in &canvas.decorations {
        for index in 0..sheets.len() {
            let start = sheets[index].offset;
            let page_end = start + f64::from(available - sheets[index].header);
            let end = match sheets.get(index + 1) {
                Some(next) => page_end.min(next.offset),
                None => page_end,
            };
            let top = decoration.top.max(start);
            let bottom = decoration.bottom.min(end);
            if bottom <= top {
                continue;
            }
            let mut paint = decoration.paint.clone();
            if decoration.top < start {
                paint = paint.without_border(Side::Top);
            }
            if decoration.bottom > end {
                paint = paint.without_border(Side::Bottom);
            }
            let sheet = &mut sheets[index];
            let rect = Rect::new(
                margins.left + decoration.x,
                margins.top + sheet.header + (top - start) as f32,
                decoration.width,
                (bottom - top) as f32,
            );
            // The whole box, for placing a background image: it may start
            // on an earlier page and end on a later one.
            let whole = Rect::new(
                rect.origin.x,
                rect.origin.y - (top - decoration.top) as f32,
                decoration.width,
                (decoration.bottom - decoration.top) as f32,
            );
            paint.fills_piece(rect, whole, &mut sheet.boxes);
        }
    }

    sheets
        .into_iter()
        .map(|sheet| {
            let mut items = Vec::new();
            if let Some(paint) = &canvas.page_background {
                let area = Rect::new(
                    margins.left,
                    margins.top,
                    size.width - margins.left - margins.right,
                    size.height - margins.top - margins.bottom,
                );
                paint.fills(area, &mut items);
            }
            items.extend(sheet.boxes);
            items.extend(sheet.content);
            items.extend(sheet.top);
            // Fixed boxes on every page, above everything.
            items.extend(
                canvas
                    .fixed
                    .iter()
                    .cloned()
                    .map(|item| item.translate(margins.left, margins.top)),
            );
            Page { size, items }
        })
        .collect()
}

/// The table header to repeat above `slice` when it starts a page, unless
/// it would take too much of the page.
fn repeated_header(slice: &Slice, available: f32) -> Option<Rc<RepeatedHeader>> {
    slice
        .header
        .clone()
        .filter(|header| header.height <= available * MAX_HEADER_FRACTION)
}

/// The indices of the slices that start a page, after the first page.
///
/// When a slice does not fit, the page breaks before the group of slices
/// it is kept with. A group that cannot fit on any page (it already starts
/// this one, or is taller than a page) first gives up its [`Keep::inside`](crate::flow::Keep::inside)
/// links (a box with `break-inside: avoid` taller than a page), and then
/// the page breaks right before the slice that does not fit. Forced breaks
/// always happen.
fn page_starts(slices: &[Slice], available: f32) -> Vec<usize> {
    let mut starts = Vec::new();
    // The first slice on the current page, the canvas position at the top
    // of its content, and the height of the header repeated above it.
    let (mut first, mut offset, mut header) = (0, 0.0, 0.0);
    let mut index = 0;
    while index < slices.len() {
        let slice = &slices[index];
        // The first slice on a page stays, even if it does not fit.
        if index > first {
            let bottom = (slice.top - offset) as f32 + header + slice.height;
            if slice.break_before || bottom > available {
                let start = if slice.break_before {
                    index
                } else {
                    group_start(slices, index, first, true, available)
                        .or_else(|| group_start(slices, index, first, false, available))
                        .unwrap_or(index)
                };
                starts.push(start);
                first = start;
                offset = slices[start].top;
                header = repeated_header(&slices[start], available).map_or(0.0, |h| h.height);
                // Lay out the moved slices again on the new page.
                index = start;
                continue;
            }
        }
        index += 1;
    }
    starts
}

/// The first slice of the group `index` is kept with, if moving the group
/// to a new page helps: the group starts after `first`, the first slice
/// on the current page, and fits on a page. `inside` says whether to
/// honor [`Keep::inside`](crate::flow::Keep::inside) links.
fn group_start(
    slices: &[Slice],
    index: usize,
    first: usize,
    inside: bool,
    available: f32,
) -> Option<usize> {
    let kept =
        |slice: &Slice| !slice.break_before && (slice.keep.tight || (inside && slice.keep.inside));
    let mut start = index;
    while start > first && kept(&slices[start]) {
        start -= 1;
    }
    if start == first {
        return None;
    }
    let mut end = index;
    while slices.get(end + 1).is_some_and(kept) {
        end += 1;
    }
    let header = repeated_header(&slices[start], available).map_or(0.0, |h| h.height);
    let height = (slices[end].top - slices[start].top) as f32 + slices[end].height + header;
    (height <= available).then_some(start)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::{BoxPaint, Decoration, Keep};
    use crate::fragment::Fill;
    use presswork_style::Color;

    const RED: Color = Color::rgb(255, 0, 0);

    fn slice(top: f64, height: f32) -> Slice {
        Slice {
            top,
            height,
            items: vec![Item::Fill(Fill {
                rect: Rect::new(0.0, 0.0, 10.0, height),
                color: Color::BLACK,
            })],
            break_before: false,
            keep: Keep::default(),
            header: None,
            on_top: false,
        }
    }

    fn canvas(slices: Vec<Slice>) -> Canvas {
        Canvas {
            slices,
            ..Canvas::default()
        }
    }

    fn fills(page: &Page) -> Vec<(f32, f32, Color)> {
        page.items
            .iter()
            .filter_map(|item| match item {
                Item::Fill(fill) => Some((fill.rect.origin.y, fill.rect.size.height, fill.color)),
                _ => None,
            })
            .collect()
    }

    fn tops(page: &Page) -> Vec<f32> {
        fills(page).into_iter().map(|(y, _, _)| y).collect()
    }

    /// 100pt of content height per page.
    const SIZE: Size = Size {
        width: 100.0,
        height: 120.0,
    };
    const MARGINS: EdgeSizes = EdgeSizes::uniform(10.0);

    #[test]
    fn breaks_before_a_slice_that_does_not_fit() {
        let pages = paginate(
            canvas(vec![slice(0.0, 40.0), slice(50.0, 40.0), slice(95.0, 40.0)]),
            SIZE,
            MARGINS,
        );
        assert_eq!(pages.len(), 2);
        assert_eq!(tops(&pages[0]), [10.0, 60.0]);
        // The gap before the third slice is dropped at the page break.
        assert_eq!(tops(&pages[1]), [10.0]);
    }

    /// Slices `height` tall, one after the other from the top; `keep` says
    /// which ones are kept with the slice before.
    fn kept(height: f32, keep: &[Keep]) -> Canvas {
        canvas(
            keep.iter()
                .enumerate()
                .map(|(i, &keep)| Slice {
                    keep,
                    ..slice(i as f64 * f64::from(height), height)
                })
                .collect(),
        )
    }

    const FREE: Keep = Keep {
        tight: false,
        inside: false,
    };
    const TIGHT: Keep = Keep {
        tight: true,
        inside: false,
    };
    const INSIDE: Keep = Keep {
        tight: false,
        inside: true,
    };

    /// The number of slices on each page.
    fn per_page(pages: &[Page]) -> Vec<usize> {
        pages.iter().map(|page| fills(page).len()).collect()
    }

    #[test]
    fn kept_slices_move_to_the_next_page_together() {
        // 100pt per page: 30pt slices fit three to a page, but the fourth
        // is kept with the third.
        let pages = paginate(kept(30.0, &[FREE, FREE, FREE, TIGHT]), SIZE, MARGINS);
        assert_eq!(per_page(&pages), [2, 2]);
        assert_eq!(tops(&pages[1]), [10.0, 40.0]);
    }

    #[test]
    fn a_group_taller_than_a_page_breaks_where_it_must() {
        let pages = paginate(
            kept(30.0, &[FREE, TIGHT, TIGHT, TIGHT, TIGHT]),
            SIZE,
            MARGINS,
        );
        assert_eq!(per_page(&pages), [3, 2]);
    }

    #[test]
    fn an_avoid_box_taller_than_a_page_still_keeps_its_lines_together() {
        // A box with break-inside: avoid over six slices, too tall for any
        // page, so it does not move; slices 2 and 3 are also kept tightly
        // (the last lines of a paragraph inside it).
        let both = Keep {
            tight: true,
            inside: true,
        };
        let pages = paginate(
            kept(30.0, &[FREE, INSIDE, INSIDE, both, INSIDE, INSIDE]),
            SIZE,
            MARGINS,
        );
        // The box splits, but not between slices 2 and 3.
        assert_eq!(per_page(&pages), [2, 3, 1]);
    }

    #[test]
    fn an_avoid_box_that_fits_moves_whole() {
        let pages = paginate(kept(30.0, &[FREE, FREE, FREE, INSIDE]), SIZE, MARGINS);
        assert_eq!(per_page(&pages), [2, 2]);
    }

    #[test]
    fn forced_breaks_beat_keeps() {
        let mut content = kept(10.0, &[FREE, TIGHT, TIGHT]);
        content.slices[1].break_before = true;
        let pages = paginate(content, SIZE, MARGINS);
        assert_eq!(per_page(&pages), [1, 2]);
    }

    #[test]
    fn oversized_slice_at_page_top_stays() {
        let pages = paginate(canvas(vec![slice(0.0, 500.0)]), SIZE, MARGINS);
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn empty_document_has_one_page() {
        let pages = paginate(Canvas::default(), SIZE, MARGINS);
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn forced_break_starts_a_new_page() {
        let mut second = slice(20.0, 10.0);
        second.break_before = true;
        let mut first = slice(0.0, 10.0);
        // A forced break before the very first slice does nothing.
        first.break_before = true;
        let pages = paginate(canvas(vec![first, second]), SIZE, MARGINS);
        assert_eq!(pages.len(), 2);
        assert_eq!(tops(&pages[1]), [10.0]);
    }

    #[test]
    fn positions_stay_exact_far_down_a_long_document() {
        // 100,000 pages down, an f32 canvas would be off by whole points.
        let top = 100_000.0 * 100.0 + 0.25;
        let pages = paginate(
            canvas(vec![
                slice(0.0, 10.0),
                slice(top, 10.0),
                slice(top + 12.5, 10.0),
            ]),
            SIZE,
            MARGINS,
        );
        assert_eq!(tops(&pages[1]), [10.0, 22.5]);
    }

    #[test]
    fn table_header_repeats_on_the_next_page() {
        let header = Rc::new(RepeatedHeader {
            height: 15.0,
            items: vec![Item::Fill(Fill {
                rect: Rect::new(0.0, 0.0, 10.0, 15.0),
                color: RED,
            })],
        });
        let mut rows: Vec<Slice> = (0..4)
            .map(|i| slice(15.0 + 30.0 * f64::from(i), 30.0))
            .collect();
        for row in &mut rows {
            row.header = Some(Rc::clone(&header));
        }
        let mut slices = vec![slice(0.0, 15.0)];
        slices.extend(rows);
        let pages = paginate(canvas(slices), SIZE, MARGINS);
        assert_eq!(pages.len(), 2);
        // Second page: the header, then the two remaining rows below it.
        assert_eq!(
            fills(&pages[1]),
            [
                (10.0, 15.0, RED),
                (25.0, 30.0, Color::BLACK),
                (55.0, 30.0, Color::BLACK)
            ]
        );
    }

    #[test]
    fn decorations_split_at_page_breaks_and_paint_first() {
        let paint = BoxPaint {
            background: RED,
            border: Side::ALL.map(|side| (side, 0.0, Color::BLACK)),
            ..BoxPaint::INVISIBLE
        };
        let mut content = canvas(vec![slice(10.0, 60.0), slice(70.0, 60.0)]);
        content.decorations.push(Decoration {
            top: 5.0,
            bottom: 135.0,
            x: 0.0,
            width: 50.0,
            paint,
        });
        let pages = paginate(content, SIZE, MARGINS);
        assert_eq!(pages.len(), 2);
        // Page 1: the box from its top to the break, then the content.
        assert_eq!(fills(&pages[0])[0], (15.0, 65.0, RED));
        // Page 2: the rest of the box, starting at the top of the page.
        assert_eq!(fills(&pages[1])[0], (10.0, 65.0, RED));
        assert_eq!(fills(&pages[1])[1].2, Color::BLACK);
    }
}
