//! The collapsing border model (CSS 2.1 section 17.6.2).
//!
//! Every edge of the table grid gets one border, chosen from the borders of
//! everything along it: cells, rows, row groups, columns, column groups and
//! the table. Each border is drawn once, centered on its grid line, and the
//! cells and the table give it half their width on each side.

use std::ops::Range;

use presswork_style::{BorderStyle, Color, ComputedStyle, Side, Sides};

use super::{Cell, ColumnElement, Row};
use crate::flow::border::{MIN_DOUBLE, dashed_line};
use crate::fragment::{Fill, Item};
use crate::geometry::{Point, Rect};

/// Where a border comes from. Later variants win ties (rule 4 of section
/// 17.6.2.1): a cell's border beats its row's, and so on down to the
/// table's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Origin {
    Table,
    ColumnGroup,
    Column,
    RowGroup,
    Row,
    Cell,
}

/// A border that wants an edge.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Candidate {
    width: f32,
    style: BorderStyle,
    color: Color,
    origin: Origin,
}

impl Candidate {
    fn new(style: &ComputedStyle, side: Side, origin: Origin) -> Self {
        let border = style.border.get(side);
        Self {
            width: border.width,
            style: border.style,
            color: style.border_color(side),
            origin,
        }
    }

    /// Whether `self` wins over `other`: the wider border, then the
    /// stronger style, then the more specific origin. Equal borders keep
    /// the one offered first, which is the top-left one.
    fn beats(&self, other: &Self) -> bool {
        let key = |c: &Self| (c.width, style_rank(c.style), c.origin);
        key(self).partial_cmp(&key(other)) == Some(std::cmp::Ordering::Greater)
    }
}

/// Rule 3 of section 17.6.2.1: `double` beats `solid`, which beats
/// `dashed`, and so on down to `inset`.
fn style_rank(style: BorderStyle) -> u8 {
    match style {
        BorderStyle::Double => 8,
        BorderStyle::Solid => 7,
        BorderStyle::Dashed => 6,
        BorderStyle::Dotted => 5,
        BorderStyle::Ridge => 4,
        BorderStyle::Outset => 3,
        BorderStyle::Groove => 2,
        BorderStyle::Inset => 1,
        BorderStyle::None | BorderStyle::Hidden => 0,
    }
}

/// One edge of the grid.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
enum Edge {
    /// Nothing along it has a border.
    #[default]
    Empty,
    /// Inside a cell that spans it: there is no edge.
    Internal,
    /// A `hidden` border along it suppresses every other (rule 1).
    Hidden,
    Border(Candidate),
}

impl Edge {
    fn offer(&mut self, candidate: Candidate) {
        match (*self, candidate.style) {
            (Edge::Internal | Edge::Hidden, _) => {}
            (_, BorderStyle::Hidden) => *self = Edge::Hidden,
            // `none` loses to everything (rule 2).
            (_, BorderStyle::None) => {}
            (Edge::Empty, _) => *self = Edge::Border(candidate),
            (Edge::Border(current), _) => {
                if candidate.beats(&current) {
                    *self = Edge::Border(candidate);
                }
            }
        }
    }

    /// The width drawn along the edge.
    fn width(&self) -> f32 {
        match self {
            Edge::Border(border) => border.width,
            Edge::Empty | Edge::Internal | Edge::Hidden => 0.0,
        }
    }
}

/// The resolved borders of a table's grid.
pub(super) struct CollapsedBorders {
    rows: usize,
    columns: usize,
    /// `(rows + 1) × columns`: the edge along the top of each row (the
    /// last line is the bottom of the last row), one per column.
    horizontal: Vec<Edge>,
    /// `rows × (columns + 1)`: the edge left of each column (the last is
    /// right of the last column), one per row.
    vertical: Vec<Edge>,
}

/// What [`CollapsedBorders::resolve`] reads its borders from.
pub(super) struct TableBorders<'a> {
    pub table: &'a ComputedStyle,
    pub rows: &'a [Row],
    /// Indexed by [`Row::group`].
    pub groups: &'a [ComputedStyle],
    pub columns: &'a [ColumnElement],
    pub column_groups: &'a [(Range<usize>, ComputedStyle)],
}

impl CollapsedBorders {
    /// Chooses the border of every edge of a grid of `columns` columns.
    pub fn resolve(parts: &TableBorders<'_>, columns: usize) -> Self {
        let rows = parts.rows.len();
        let mut borders = Self {
            rows,
            columns,
            horizontal: vec![Edge::Empty; (rows + 1) * columns],
            vertical: vec![Edge::Empty; rows * (columns + 1)],
        };

        // Edges inside cells that span them do not exist.
        for (index, row) in parts.rows.iter().enumerate() {
            for cell in &row.cells {
                let (spanned_rows, spanned) = borders.extent(index, cell);
                for r in spanned_rows.clone() {
                    for c in spanned.start + 1..spanned.end {
                        *borders.v(r, c) = Edge::Internal;
                    }
                }
                for r in spanned_rows.start + 1..spanned_rows.end {
                    for c in spanned.clone() {
                        *borders.h(r, c) = Edge::Internal;
                    }
                }
            }
        }

        // Offered from the most specific origin, and top to bottom, left
        // to right, so that ties go to the top-left border.
        for (index, row) in parts.rows.iter().enumerate() {
            for cell in &row.cells {
                let (rows, columns) = borders.extent(index, cell);
                borders.offer_box(&cell.style, Origin::Cell, rows, columns);
            }
        }
        let all_columns = 0..columns;
        for (index, row) in parts.rows.iter().enumerate() {
            borders.offer_box(
                &row.style,
                Origin::Row,
                index..index + 1,
                all_columns.clone(),
            );
        }
        let mut start = 0;
        while start < rows {
            let group = parts.rows[start].group;
            let end = start
                + parts.rows[start..]
                    .iter()
                    .take_while(|row| row.group == group)
                    .count();
            if let Some(style) = parts.groups.get(group) {
                borders.offer_box(style, Origin::RowGroup, start..end, all_columns.clone());
            }
            start = end;
        }
        for (column, element) in parts.columns.iter().enumerate().take(columns) {
            if let Some(style) = &element.style {
                borders.offer_box(style, Origin::Column, 0..rows, column..column + 1);
            }
        }
        for (range, style) in parts.column_groups {
            let range = range.start.min(columns)..range.end.min(columns);
            if !range.is_empty() {
                borders.offer_box(style, Origin::ColumnGroup, 0..rows, range);
            }
        }
        borders.offer_box(parts.table, Origin::Table, 0..rows, all_columns);
        borders
    }

    /// The rows and columns a cell covers, within the grid.
    fn extent(&self, row: usize, cell: &Cell) -> (Range<usize>, Range<usize>) {
        let rows = row..(row + cell.row_span).min(self.rows);
        let start = cell.column.min(self.columns);
        (rows, start..(cell.column + cell.span).min(self.columns))
    }

    fn h(&mut self, row: usize, column: usize) -> &mut Edge {
        &mut self.horizontal[row * self.columns + column]
    }

    fn v(&mut self, row: usize, column: usize) -> &mut Edge {
        &mut self.vertical[row * (self.columns + 1) + column]
    }

    fn h_width(&self, row: usize, column: usize) -> f32 {
        self.horizontal[row * self.columns + column].width()
    }

    fn v_width(&self, row: usize, column: usize) -> f32 {
        self.vertical[row * (self.columns + 1) + column].width()
    }

    /// Offers the borders of a box covering `rows` and `columns` to the
    /// edges around it.
    fn offer_box(
        &mut self,
        style: &ComputedStyle,
        origin: Origin,
        rows: Range<usize>,
        columns: Range<usize>,
    ) {
        if rows.is_empty() || columns.is_empty() {
            return;
        }
        let candidate = |side| Candidate::new(style, side, origin);
        for column in columns.clone() {
            self.h(rows.start, column).offer(candidate(Side::Top));
            self.h(rows.end, column).offer(candidate(Side::Bottom));
        }
        for row in rows {
            self.v(row, columns.start).offer(candidate(Side::Left));
            self.v(row, columns.end).offer(candidate(Side::Right));
        }
    }

    /// Half the width of the borders around a cell in row `row`: what its
    /// border box gives them. Where a side has several edges, the widest.
    pub fn cell_borders(&self, row: usize, cell: &Cell) -> Sides<f32> {
        let (rows, columns) = self.extent(row, cell);
        if rows.is_empty() || columns.is_empty() {
            return Sides::uniform(0.0);
        }
        let widest = |widths: &mut dyn Iterator<Item = f32>| widths.fold(0.0, f32::max) / 2.0;
        Sides {
            top: widest(&mut columns.clone().map(|c| self.h_width(rows.start, c))),
            bottom: widest(&mut columns.clone().map(|c| self.h_width(rows.end, c))),
            left: widest(&mut rows.clone().map(|r| self.v_width(r, columns.start))),
            right: widest(&mut rows.clone().map(|r| self.v_width(r, columns.end))),
        }
    }

    /// Half the width of the table's outer borders, which the table's
    /// border box holds: the widest along the top and bottom, and the
    /// first row's at the sides (section 17.6.2).
    pub fn table_borders(&self) -> Sides<f32> {
        let widest = |widths: &mut dyn Iterator<Item = f32>| widths.fold(0.0, f32::max) / 2.0;
        let first_row = |column| {
            if self.rows > 0 {
                self.v_width(0, column) / 2.0
            } else {
                0.0
            }
        };
        Sides {
            top: widest(&mut (0..self.columns).map(|c| self.h_width(0, c))),
            bottom: widest(&mut (0..self.columns).map(|c| self.h_width(self.rows, c))),
            left: first_row(0),
            right: first_row(self.columns),
        }
    }

    /// Draws the edges of the rows `rows`: their sides, the line along the
    /// top of each, and the line below the last. `tops` and `heights` are
    /// the rows' positions, `xs` the positions of the `columns + 1` column
    /// lines.
    ///
    /// The line between two slices is drawn by both, at the same place, so
    /// that each page shows it when the table breaks there. Lines that meet
    /// overlap; the wider one is drawn last, so that it covers the joint.
    pub fn paint(
        &self,
        rows: Range<usize>,
        tops: &[f32],
        heights: &[f32],
        xs: &[f32],
        out: &mut Vec<Item>,
    ) {
        // Each line's width, rectangle, color, style, and whether it is
        // horizontal.
        let mut lines: Vec<(f32, Fill, BorderStyle, bool)> = Vec::new();
        let mut fill = |x: f32, y: f32, width: f32, height: f32, edge: Edge, horizontal: bool| {
            if let Edge::Border(border) = edge
                && width > 0.0
                && height > 0.0
                && !border.color.is_transparent()
            {
                let fill = Fill {
                    rect: Rect::new(x, y, width, height),
                    color: border.color,
                };
                lines.push((border.width, fill, border.style, horizontal));
            }
        };

        // The widest horizontal line meeting column line `column` at the
        // top of row `row`.
        let crossing = |row: usize, column: usize| {
            let left = column.checked_sub(1).map_or(0.0, |c| self.h_width(row, c));
            let right = if column < self.columns {
                self.h_width(row, column)
            } else {
                0.0
            };
            left.max(right)
        };
        for (k, row) in rows.clone().enumerate() {
            for (column, &x) in xs.iter().enumerate().take(self.columns + 1) {
                let edge = self.vertical[row * (self.columns + 1) + column];
                let width = edge.width();
                // Reach over the horizontal lines at both ends, so that the
                // wider line covers the joint whichever slice draws it.
                let (up, down) = (crossing(row, column) / 2.0, crossing(row + 1, column) / 2.0);
                fill(
                    x - width / 2.0,
                    tops[k] - up,
                    width,
                    heights[k] + up + down,
                    edge,
                    false,
                );
            }
        }

        let last = rows.len().saturating_sub(1);
        let boundaries = (rows.start..=rows.end).zip(
            tops.iter()
                .copied()
                .chain(std::iter::once(tops[last] + heights[last])),
        );
        for (row, y) in boundaries {
            for column in 0..self.columns {
                let edge = self.horizontal[row * self.columns + column];
                let width = edge.width();
                // Reach over the vertical lines at both ends, so that
                // corners are filled.
                let reach = |column: usize| {
                    let above = row.checked_sub(1).map_or(0.0, |r| self.v_width(r, column));
                    let below = if row < self.rows {
                        self.v_width(row, column)
                    } else {
                        0.0
                    };
                    above.max(below) / 2.0
                };
                let (left, right) = (
                    xs[column] - reach(column),
                    xs[column + 1] + reach(column + 1),
                );
                fill(left, y - width / 2.0, right - left, width, edge, true);
            }
        }
        lines.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, fill, style, horizontal) in lines {
            styled_line(fill, style, horizontal, out);
        }
    }
}

/// Draws the line filling `fill.rect` in `style`: dashes and dots along
/// its length, two thin lines for `double`.
fn styled_line(fill: Fill, style: BorderStyle, horizontal: bool, out: &mut Vec<Item>) {
    let Rect { origin, size } = fill.rect;
    let width = if horizontal { size.height } else { size.width };
    match style {
        BorderStyle::Dashed | BorderStyle::Dotted => {
            let (from, to) = if horizontal {
                let y = origin.y + width / 2.0;
                (
                    Point::new(origin.x, y),
                    Point::new(origin.x + size.width, y),
                )
            } else {
                let x = origin.x + width / 2.0;
                (
                    Point::new(x, origin.y),
                    Point::new(x, origin.y + size.height),
                )
            };
            let dotted = style == BorderStyle::Dotted;
            dashed_line(out, from, to, width, fill.color, dotted);
        }
        BorderStyle::Double if width >= MIN_DOUBLE => {
            let third = width / 3.0;
            let stripes = if horizontal {
                [
                    Rect::new(origin.x, origin.y, size.width, third),
                    Rect::new(origin.x, origin.y + 2.0 * third, size.width, third),
                ]
            } else {
                [
                    Rect::new(origin.x, origin.y, third, size.height),
                    Rect::new(origin.x + 2.0 * third, origin.y, third, size.height),
                ]
            };
            for rect in stripes {
                out.push(Item::Fill(Fill {
                    rect,
                    color: fill.color,
                }));
            }
        }
        _ => out.push(Item::Fill(fill)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(width: f32, style: BorderStyle, origin: Origin) -> Candidate {
        Candidate {
            width,
            style,
            color: Color::BLACK,
            origin,
        }
    }

    #[test]
    fn hidden_wins_none_loses_then_width_style_and_origin() {
        let mut edge = Edge::Empty;
        edge.offer(candidate(1.0, BorderStyle::Solid, Origin::Table));
        edge.offer(candidate(9.0, BorderStyle::None, Origin::Cell));
        assert_eq!(edge.width(), 1.0, "none loses");
        edge.offer(candidate(2.0, BorderStyle::Dotted, Origin::Table));
        assert_eq!(edge.width(), 2.0, "wider wins");
        edge.offer(candidate(2.0, BorderStyle::Double, Origin::Table));
        assert!(matches!(edge, Edge::Border(b) if b.style == BorderStyle::Double));
        edge.offer(candidate(2.0, BorderStyle::Double, Origin::Row));
        assert!(matches!(edge, Edge::Border(b) if b.origin == Origin::Row));
        edge.offer(candidate(0.0, BorderStyle::Hidden, Origin::Table));
        assert_eq!(edge, Edge::Hidden);
        edge.offer(candidate(5.0, BorderStyle::Solid, Origin::Cell));
        assert_eq!(edge, Edge::Hidden, "hidden stays");
    }
}
