//! Table layout.
//!
//! A simplified version of the automatic table layout algorithm (CSS 2.1
//! section 17.5.2.2): columns are sized from the min- and max-content
//! widths of their cells and the widths given to cells and `<col>`s, then
//! each row is laid out as one unbreakable slice. `table-layout: fixed`
//! (section 17.5.2.1) takes the widths from the `<col>`s and the first row
//! alone. Header rows repeat at the top of every page the table continues
//! on.
//!
//! A cell spanning rows is laid out over all of them, growing them if it
//! is taller, and rows joined by such cells form one slice, so they stay
//! on one page.
//!
//! With `border-collapse: collapse`, the borders of cells, rows, row
//! groups, columns and the table are resolved per grid edge; see
//! [`collapse`].
//!
//! Not supported yet: `height` on the table and row groups (cells and rows
//! take it as a minimum).

mod collapse;

use std::ops::Range;
use std::rc::Rc;

use presswork_dom::NodeId;
use presswork_style::{
    BorderCollapse, BorderSide, BorderStyle, BoxSizing, Color, ColorValue, ComputedStyle,
    Dimension, Display, Sides, TableLayout, VerticalAlign,
};

use super::boxes::BoxGeometry;
use super::{BoxPaint, Canvas, Flow, RepeatedHeader};
use crate::fragment::{Fill, Item};
use crate::geometry::Rect;
use collapse::{CollapsedBorders, TableBorders};

struct Cell {
    node: NodeId,
    style: ComputedStyle,
    /// First column the cell occupies.
    column: usize,
    /// Number of columns it spans.
    span: usize,
    /// Number of rows it reserves columns for.
    row_span: usize,
    border: Sides<f32>,
    padding: Sides<f32>,
    /// Min- and max-content width of the border box.
    min: f32,
    max: f32,
    /// Its background, and its borders unless they collapse; set when
    /// the table is laid out.
    paint: Option<BoxPaint>,
}

impl Cell {
    fn horizontal_edges(&self) -> f32 {
        self.border.left + self.border.right + self.padding.left + self.padding.right
    }

    fn vertical_edges(&self) -> f32 {
        self.border.top + self.border.bottom + self.padding.top + self.padding.bottom
    }

    /// The width the cell asks for: a length (of its border box, whatever
    /// its `box-sizing`), a percentage of the table, or `auto`.
    fn width(&self) -> Dimension {
        match (self.style.width, self.style.box_sizing) {
            (Dimension::Pt(width), BoxSizing::ContentBox) => {
                Dimension::Pt(width + self.horizontal_edges())
            }
            (width, _) => width,
        }
    }

    /// The least border-box height its `height` asks for: as in browsers,
    /// a cell (and a row) grows to fit taller content. `min-height` and
    /// `max-height` do not apply to cells.
    fn min_height(&self) -> f32 {
        match (self.style.height, self.style.box_sizing) {
            (Dimension::Pt(height), BoxSizing::ContentBox) => height + self.vertical_edges(),
            (Dimension::Pt(height), BoxSizing::BorderBox) => height,
            _ => 0.0,
        }
    }
}

/// The parts of a table, from [`Flow::collect_rows`].
struct TableParts {
    captions: Vec<(NodeId, ComputedStyle)>,
    /// Header rows first, footer rows last.
    rows: Vec<Row>,
    /// The styles of the row groups, indexed by [`Row::group`].
    groups: Vec<ComputedStyle>,
    /// What the `<col>`s say of each column, in column order.
    columns: Vec<ColumnElement>,
    /// The columns of each `<colgroup>`, and its style.
    column_groups: Vec<(Range<usize>, ComputedStyle)>,
}

/// A column from a `<col>`, or from a `<colgroup>` without any.
struct ColumnElement {
    width: Dimension,
    /// The `<col>`'s style, for its borders; `None` for a group's column.
    style: Option<ComputedStyle>,
}

/// What a column's cells and `<col>` ask of its width, for the automatic
/// table layout.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Column {
    /// The narrowest and widest the content can be.
    min: f32,
    /// For a fixed-width column, its width (if wider than `min`).
    max: f32,
    /// A percentage of the table's width, as a fraction.
    percent: Option<f32>,
    /// A width in points, from a cell or a `<col>`.
    fixed: Option<f32>,
}

struct Row {
    style: ComputedStyle,
    cells: Vec<Cell>,
    /// The row belongs to a header group and repeats on each page.
    header: bool,
    /// The row group it belongs to: cells cannot span rows across groups.
    /// An index into [`TableParts::groups`], or `usize::MAX` for rows
    /// outside any group.
    group: usize,
}

/// A cell's content, laid out at its width.
struct LaidOutCell {
    canvas: Canvas,
    content_height: f32,
    /// Border-box width.
    width: f32,
}

impl Cell {
    /// The border-box height the cell needs: its content's, or its
    /// `height` if more.
    fn needed_height(&self, laid_out: &LaidOutCell) -> f32 {
        (laid_out.content_height + self.vertical_edges()).max(self.min_height())
    }
}

/// Column positions and widths, relative to the table's content box.
struct Grid {
    /// Left edge of each column.
    starts: Vec<f32>,
    widths: Vec<f32>,
    spacing: f32,
}

impl Grid {
    fn new(widths: Vec<f32>, spacing: f32) -> Self {
        let mut x = spacing;
        let starts = widths
            .iter()
            .map(|width| {
                let start = x;
                x += width + spacing;
                start
            })
            .collect();
        Self {
            starts,
            widths,
            spacing,
        }
    }

    /// The width of `span` columns starting at `column`, including the
    /// spacing between them.
    fn span_width(&self, column: usize, span: usize) -> f32 {
        let end = (column + span).min(self.widths.len());
        let columns = &self.widths[column.min(end)..end];
        columns.iter().sum::<f32>() + self.spacing * columns.len().saturating_sub(1) as f32
    }

    /// The width of all columns and the spacing around them.
    fn total_width(&self) -> f32 {
        self.widths.iter().sum::<f32>() + self.spacing * (self.widths.len() as f32 + 1.0)
    }
}

impl Flow<'_, '_> {
    /// Lays out the table `id` in a containing block whose content box
    /// starts at `x` and is `width` wide.
    pub(super) fn table(&mut self, id: NodeId, style: &ComputedStyle, x: f32, width: f32) {
        let TableParts {
            captions,
            mut rows,
            groups,
            columns: column_elements,
            column_groups,
        } = self.collect_rows(id, style);
        for (caption, caption_style) in &captions {
            self.block(*caption, caption_style, x, width);
        }

        clamp_row_spans(&mut rows);
        let columns = assign_columns(&mut rows);
        let col_widths: Vec<Dimension> = column_elements.iter().map(|c| c.width).collect();
        let collapse = style.border_collapse == BorderCollapse::Collapse;
        let spacing = if collapse { 0.0 } else { style.border_spacing };
        let gaps = spacing * (columns as f32 + 1.0);

        // With collapsed borders, cells and the table take half of each
        // border along their edges, and the borders are drawn separately.
        let collapsed = collapse.then(|| {
            let parts = TableBorders {
                table: style,
                rows: &rows,
                groups: &groups,
                columns: &column_elements,
                column_groups: &column_groups,
            };
            CollapsedBorders::resolve(&parts, columns)
        });
        let collapsed_style;
        let style = match &collapsed {
            Some(borders) => {
                for (index, row) in rows.iter_mut().enumerate() {
                    for cell in &mut row.cells {
                        cell.border = borders.cell_borders(index, cell);
                    }
                }
                collapsed_style = collapsed_table_style(style, borders.table_borders());
                &collapsed_style
            }
            None => style,
        };
        for cell in rows.iter_mut().flat_map(|row| &mut row.cells) {
            cell.paint = self.box_paint(&cell.style).map(|paint| match collapsed {
                // The table draws collapsed borders.
                Some(_) => BoxPaint {
                    background: paint.background,
                    image: paint.image,
                    ..BoxPaint::INVISIBLE
                },
                None => paint,
            });
        }

        // The table's own box. Its content width is the grid's width.
        let probe = BoxGeometry::resolve(style, width, Some(0.0));
        let edges = probe.border_box_width();
        let specified = BoxGeometry::specified_content_width(style, width, edges);

        let fixed_layout = style.table_layout == TableLayout::Fixed && !self.is_measuring();
        let widths = match specified {
            // Fixed layout needs a width to share out; without one it is
            // automatic (CSS 2.1 section 17.5.2).
            Some(specified) if fixed_layout => {
                fixed_column_widths(&rows, &col_widths, columns, spacing, specified - gaps)
            }
            _ => {
                for cell in rows.iter_mut().flat_map(|row| &mut row.cells) {
                    self.measure_cell(cell);
                }
                let constraints = column_constraints(&rows, &col_widths, columns, spacing);
                let (min_total, max_total) = (
                    constraints.iter().map(|c| c.min).sum::<f32>() + gaps,
                    constraints.iter().map(|c| c.max).sum::<f32>() + gaps,
                );
                if self.is_measuring() {
                    let outer = edges
                        + style.margin.left.resolve(0.0).unwrap_or(0.0)
                        + style.margin.right.resolve(0.0).unwrap_or(0.0);
                    let fixed = match style.width {
                        Dimension::Pt(_) => specified,
                        _ => None,
                    };
                    self.record_extent(
                        outer + fixed.unwrap_or(min_total).max(min_total),
                        outer + fixed.unwrap_or(max_total).max(min_total),
                    );
                    return;
                }
                let margins = style.margin.left.resolve(width).unwrap_or(0.0)
                    + style.margin.right.resolve(width).unwrap_or(0.0);
                let available = (width - margins - edges).max(0.0);
                let content_width = match specified {
                    Some(specified) => specified.max(min_total),
                    None => max_total.min(available).max(min_total),
                };
                distribute(&constraints, content_width - gaps)
            }
        };
        let grid = Grid::new(widths, spacing);
        let geometry = BoxGeometry::resolve(style, width, Some(grid.total_width()));
        let content_x = x + geometry.content_x();

        let open = self.open_box(style, &geometry, x, false);
        self.settle_margin();
        self.y += f64::from(spacing);

        let laid_out: Vec<Vec<LaidOutCell>> = rows
            .iter()
            .map(|row| self.layout_cells(row, &grid))
            .collect();
        let heights = row_heights(&rows, &laid_out, spacing);
        let mut laid_out = laid_out.into_iter();

        let mut header_items = Vec::new();
        let mut header_height = 0.0;
        let mut header: Option<Rc<RepeatedHeader>> = None;
        for block in row_blocks(&rows) {
            let rows_in_block = RowBlock {
                first: block.start,
                rows: &rows[block.clone()],
                heights: &heights[block.clone()],
                collapsed: collapsed.as_ref(),
            };
            let cells = laid_out.by_ref().take(block.len()).collect();
            let (height, items) = rows_in_block.paint(cells, &grid, content_x);

            let is_header = rows[block.start].header;
            if is_header {
                header_items.extend(
                    items
                        .iter()
                        .cloned()
                        .map(|item| item.translate(0.0, header_height)),
                );
                header_height += height + spacing;
            } else if header.is_none() && header_height > 0.0 {
                header = Some(Rc::new(RepeatedHeader {
                    height: header_height,
                    items: std::mem::take(&mut header_items),
                }));
            }

            self.push_slice(self.y, height, items);
            // Header rows stay together and with the first row after them,
            // so that a header is never alone at the bottom of a page.
            let after_header = block.start > 0 && rows[block.start - 1].header;
            if after_header {
                self.keep_last_slice();
            }
            if !is_header && let Some(slice) = self.canvas.slices.last_mut() {
                slice.header.clone_from(&header);
            }
            self.y += f64::from(height + spacing);
        }

        self.close_box(style, &geometry, open);
    }

    /// Finds the table's captions and rows. Rows in header groups come
    /// first and rows in footer groups last, whatever the source order.
    fn collect_rows(&mut self, table: NodeId, style: &ComputedStyle) -> TableParts {
        let doc = self.cx.doc;
        let styles = self.cx.styles;
        let mut captions = Vec::new();
        let mut columns = Vec::new();
        let mut column_groups = Vec::new();
        let mut groups = Vec::new();
        let (mut head, mut body, mut foot) = (Vec::new(), Vec::new(), Vec::new());

        let make_row = |row: NodeId, row_style: ComputedStyle, header: bool, group: usize| {
            let cells = doc
                .children(row)
                .filter_map(|cell| {
                    let element = doc.element(cell)?;
                    let cell_style = styles.style(doc, cell, Some(&row_style));
                    if cell_style.display != Display::TableCell {
                        return None;
                    }
                    let span = |name| {
                        element
                            .attr(name)
                            .and_then(|s| s.trim().parse::<usize>().ok())
                    };
                    Some(Cell {
                        node: cell,
                        column: 0,
                        span: span("colspan").unwrap_or(1).clamp(1, 1000),
                        // `rowspan=0` spans to the end of the row group; see
                        // `clamp_row_spans`.
                        row_span: match span("rowspan") {
                            Some(0) => usize::MAX,
                            span => span.unwrap_or(1).clamp(1, 65534),
                        },
                        border: cell_style.border_widths(),
                        // Percentage padding on cells is not supported.
                        padding: cell_style.padding.map(|p| p.resolve(0.0).unwrap_or(0.0)),
                        min: 0.0,
                        max: 0.0,
                        paint: None,
                        style: cell_style,
                    })
                })
                .collect();
            Row {
                style: row_style,
                cells,
                header,
                group,
            }
        };

        // A `span` attribute: how many columns a `<col>` or `<colgroup>`
        // stands for.
        let span = |node: NodeId| {
            doc.element(node)
                .and_then(|element| element.attr("span"))
                .and_then(|s| s.trim().parse::<usize>().ok())
                .unwrap_or(1)
                .clamp(1, 1000)
        };
        for child in doc.children(table) {
            let Some(element) = doc.element(child) else {
                continue;
            };
            let child_style = styles.style(doc, child, Some(style));
            // Columns are `display: none` but still give widths.
            if element.name == "colgroup" || element.name == "col" {
                let cols: Vec<NodeId> = doc
                    .children(child)
                    .filter(|&col| doc.element(col).is_some_and(|e| e.name == "col"))
                    .collect();
                let start = columns.len();
                if element.name == "col" {
                    let width = child_style.width;
                    for _ in 0..span(child) {
                        columns.push(ColumnElement {
                            width,
                            style: Some(child_style.clone()),
                        });
                    }
                    continue;
                }
                if cols.is_empty() {
                    for _ in 0..span(child) {
                        columns.push(ColumnElement {
                            width: child_style.width,
                            style: None,
                        });
                    }
                }
                for col in cols {
                    let col_style = styles.style(doc, col, Some(&child_style));
                    // A `<col>` without a width takes its group's.
                    let width = match col_style.width {
                        Dimension::Auto => child_style.width,
                        width => width,
                    };
                    for _ in 0..span(col) {
                        columns.push(ColumnElement {
                            width,
                            style: Some(col_style.clone()),
                        });
                    }
                }
                column_groups.push((start..columns.len(), child_style));
                continue;
            }
            match child_style.display {
                Display::TableCaption => captions.push((child, child_style)),
                // Rows outside any group share an anonymous one.
                Display::TableRow => body.push(make_row(child, child_style, false, usize::MAX)),
                Display::TableRowGroup | Display::TableHeaderGroup | Display::TableFooterGroup => {
                    let header = child_style.display == Display::TableHeaderGroup;
                    let group = match child_style.display {
                        Display::TableHeaderGroup => &mut head,
                        Display::TableFooterGroup => &mut foot,
                        _ => &mut body,
                    };
                    let index = groups.len();
                    for row in doc.children(child) {
                        if doc.element(row).is_none() {
                            continue;
                        }
                        let row_style = styles.style(doc, row, Some(&child_style));
                        if row_style.display == Display::TableRow {
                            group.push(make_row(row, row_style, header, index));
                        }
                    }
                    groups.push(child_style);
                }
                _ => {}
            }
        }

        head.extend(body);
        head.extend(foot);
        TableParts {
            captions,
            rows: head,
            groups,
            columns,
            column_groups,
        }
    }

    /// Measures the min- and max-content width of a cell's border box.
    fn measure_cell(&mut self, cell: &mut Cell) {
        let mut flow = Flow::measuring(self.cx);
        flow.block_contents(cell.node, &cell.style, 0.0, 0.0);
        let content = flow.extent();
        let edges = cell.horizontal_edges();
        cell.min = content.min + edges;
        cell.max = content.max + edges;
        if let Dimension::Pt(width) = cell.width() {
            // A specified width is the preferred width, but never
            // narrower than the content can go.
            cell.max = width.max(cell.min);
        }
    }

    /// Lays out the content of each cell of `row` at its width.
    fn layout_cells(&mut self, row: &Row, grid: &Grid) -> Vec<LaidOutCell> {
        row.cells
            .iter()
            .map(|cell| {
                let width = grid.span_width(cell.column, cell.span);
                let mut flow = Flow::new(self.cx);
                flow.set_root_area((width - cell.horizontal_edges()).max(0.0), None);
                flow.block_contents(
                    cell.node,
                    &cell.style,
                    0.0,
                    (width - cell.horizontal_edges()).max(0.0),
                );
                let (canvas, content_height) = flow.finish();
                LaidOutCell {
                    canvas,
                    content_height: content_height as f32,
                    width,
                }
            })
            .collect()
    }
}

/// Rows painted together as one slice: rows joined by cells spanning
/// them, or a single row.
struct RowBlock<'a> {
    /// The index of the first row in the table.
    first: usize,
    rows: &'a [Row],
    heights: &'a [f32],
    /// The table's borders, if they collapse.
    collapsed: Option<&'a CollapsedBorders>,
}

impl RowBlock<'_> {
    /// Paints the rows' backgrounds, the cells' backgrounds and borders,
    /// then the cells' content. Returns the height and the items,
    /// positioned relative to the canvas's left edge and the top of the
    /// first row.
    fn paint(
        &self,
        laid_out: Vec<Vec<LaidOutCell>>,
        grid: &Grid,
        content_x: f32,
    ) -> (f32, Vec<Item>) {
        let spacing = grid.spacing;
        let mut tops = Vec::with_capacity(self.heights.len());
        let mut y = 0.0;
        for height in self.heights {
            tops.push(y);
            y += height + spacing;
        }
        let total_height = (y - spacing).max(0.0);
        // The height of `count` rows from row `k` of the block.
        let span_height = |k: usize, count: usize| {
            self.heights[k..k + count].iter().sum::<f32>() + spacing * (count - 1) as f32
        };
        let cell_x = |cell: &Cell| content_x + grid.starts.get(cell.column).copied().unwrap_or(0.0);

        let mut items = Vec::new();
        // Row backgrounds first: a cell spanning rows paints over the rows
        // below its own.
        for ((row, &top), &height) in self.rows.iter().zip(&tops).zip(self.heights) {
            if !row.style.background_color.is_transparent() && !grid.widths.is_empty() {
                let width = grid.total_width() - 2.0 * spacing;
                items.push(Item::Fill(Fill {
                    rect: Rect::new(content_x + spacing, top, width, height),
                    color: row.style.background_color,
                }));
            }
        }

        // Cell backgrounds, and borders unless they collapse.
        for (k, (row, cells)) in self.rows.iter().zip(&laid_out).enumerate() {
            for (cell, laid_out) in row.cells.iter().zip(cells) {
                if let Some(paint) = &cell.paint {
                    let rect = Rect::new(
                        cell_x(cell),
                        tops[k],
                        laid_out.width,
                        span_height(k, cell.row_span),
                    );
                    paint.fills(rect, &mut items);
                }
            }
        }

        if let Some(borders) = self.collapsed {
            let mut xs: Vec<f32> = grid.starts.iter().map(|start| content_x + start).collect();
            xs.push(content_x + grid.total_width());
            let rows = self.first..self.first + self.rows.len();
            borders.paint(rows, &tops, self.heights, &xs, &mut items);
        }

        for (k, (row, cells)) in self.rows.iter().zip(laid_out).enumerate() {
            for (cell, laid_out) in row.cells.iter().zip(cells) {
                let height = span_height(k, cell.row_span);
                let free = (height - cell.vertical_edges() - laid_out.content_height).max(0.0);
                let shift = match cell.style.vertical_align {
                    VerticalAlign::Middle => free / 2.0,
                    VerticalAlign::Bottom => free,
                    // Baseline alignment across cells is not implemented.
                    _ => 0.0,
                };
                laid_out.canvas.into_items(
                    cell_x(cell) + cell.border.left + cell.padding.left,
                    tops[k] + cell.border.top + cell.padding.top + shift,
                    &mut items,
                );
            }
        }
        (total_height, items)
    }
}

/// The style a table with collapsed borders lays out its own box with: no
/// padding (section 17.6.2), and `borders` (half its outer collapsed
/// borders) as transparent borders, which take room without being drawn.
fn collapsed_table_style(style: &ComputedStyle, borders: Sides<f32>) -> ComputedStyle {
    let mut style = style.clone();
    style.padding = Sides::uniform(Dimension::ZERO);
    style.border = borders.map(|width| BorderSide {
        width,
        style: BorderStyle::Solid,
        color: ColorValue::Color(Color::TRANSPARENT),
    });
    style
}

/// Shortens row spans that reach past the end of their row group, as
/// `rowspan=0` always does: a cell spans to the end of its group at most.
fn clamp_row_spans(rows: &mut [Row]) {
    // The index just past the last row of each row's group.
    let mut ends = vec![rows.len(); rows.len()];
    for i in (0..rows.len().saturating_sub(1)).rev() {
        if rows[i + 1].group == rows[i].group {
            ends[i] = ends[i + 1];
        } else {
            ends[i] = i + 1;
        }
    }
    for (i, row) in rows.iter_mut().enumerate() {
        for cell in &mut row.cells {
            cell.row_span = cell.row_span.min(ends[i] - i);
        }
    }
}

/// The height of each row: its tallest cell that spans only it, or its
/// `height`. Then cells spanning several rows that are taller than those
/// rows make them taller, in proportion to their heights; shorter spans
/// first, so that longer ones see the result.
fn row_heights(rows: &[Row], laid_out: &[Vec<LaidOutCell>], spacing: f32) -> Vec<f32> {
    let mut heights: Vec<f32> = rows
        .iter()
        .zip(laid_out)
        .map(|(row, cells)| {
            let tallest = row
                .cells
                .iter()
                .zip(cells)
                .filter(|(cell, _)| cell.row_span == 1)
                .map(|(cell, laid_out)| cell.needed_height(laid_out))
                .fold(0.0, f32::max);
            match row.style.height {
                Dimension::Pt(min) => tallest.max(min),
                _ => tallest,
            }
        })
        .collect();

    let mut spanning: Vec<(usize, &Cell, &LaidOutCell)> = rows
        .iter()
        .zip(laid_out)
        .enumerate()
        .flat_map(|(index, (row, cells))| {
            row.cells
                .iter()
                .zip(cells)
                .filter(|(cell, _)| cell.row_span > 1)
                .map(move |(cell, laid_out)| (index, cell, laid_out))
        })
        .collect();
    spanning.sort_by_key(|(_, cell, _)| cell.row_span);
    for (index, cell, laid_out) in spanning {
        let spanned = &mut heights[index..index + cell.row_span];
        let total: f32 = spanned.iter().sum();
        let available = total + spacing * (cell.row_span - 1) as f32;
        let extra = cell.needed_height(laid_out) - available;
        if extra > 0.0 {
            let count = spanned.len() as f32;
            for height in spanned {
                *height += if total > 0.0 {
                    extra * *height / total
                } else {
                    extra / count
                };
            }
        }
    }
    heights
}

/// Groups rows into blocks that cells spanning rows do not cross.
fn row_blocks(rows: &[Row]) -> Vec<Range<usize>> {
    let mut blocks = Vec::new();
    let (mut start, mut end) = (0, 0);
    for (index, row) in rows.iter().enumerate() {
        end = row
            .cells
            .iter()
            .map(|cell| index + cell.row_span)
            .fold(end.max(index + 1), usize::max);
        if end == index + 1 {
            blocks.push(start..end);
            start = end;
        }
    }
    blocks
}

/// Assigns each cell its first column, skipping columns still occupied by
/// cells spanning rows from above. Returns the number of columns.
fn assign_columns(rows: &mut [Row]) -> usize {
    // For each column, how many more rows a cell from above occupies it.
    let mut occupied: Vec<usize> = Vec::new();
    for row in rows.iter_mut() {
        let mut column = 0;
        for cell in &mut row.cells {
            while occupied.get(column).is_some_and(|&rows| rows > 0) {
                column += 1;
            }
            cell.column = column;
            if occupied.len() < column + cell.span {
                occupied.resize(column + cell.span, 0);
            }
            occupied[column..column + cell.span].fill(cell.row_span);
            column += cell.span;
        }
        for rows in &mut occupied {
            *rows = rows.saturating_sub(1);
        }
    }
    occupied.len()
}

/// What each column asks of its width, from its cells and the widths of
/// the `<col>`s in `cols`.
fn column_constraints(
    rows: &[Row],
    cols: &[Dimension],
    columns: usize,
    spacing: f32,
) -> Vec<Column> {
    let mut constraints = vec![Column::default(); columns];
    let cells = || rows.iter().flat_map(|row| &row.cells);
    let widen = |column: &mut Column, width: Dimension| match width {
        Dimension::Pt(width) => column.fixed = Some(column.fixed.unwrap_or(0.0).max(width)),
        Dimension::Percent(fraction) => {
            column.percent = Some(column.percent.unwrap_or(0.0).max(fraction));
        }
        Dimension::Auto => {}
    };

    for (column, &width) in constraints.iter_mut().zip(cols) {
        widen(column, width);
    }
    for cell in cells().filter(|cell| cell.span == 1) {
        let column = &mut constraints[cell.column];
        column.min = column.min.max(cell.min);
        column.max = column.max.max(cell.max);
        widen(column, cell.width());
    }
    // A spanning cell widens its columns equally if they are too narrow.
    for cell in cells().filter(|cell| cell.span > 1) {
        let range = cell.column..(cell.column + cell.span).min(columns);
        let count = range.len() as f32;
        if count == 0.0 {
            continue;
        }
        let between = spacing * (count - 1.0);
        let spanned = &mut constraints[range];
        let fields: [fn(&mut Column) -> &mut f32; 2] = [|c| &mut c.min, |c| &mut c.max];
        for (field, needed) in fields.into_iter().zip([cell.min, cell.max]) {
            let current = spanned.iter_mut().map(|c| *field(c)).sum::<f32>() + between;
            if needed > current {
                let extra = (needed - current) / count;
                spanned.iter_mut().for_each(|c| *field(c) += extra);
            }
        }
    }
    for column in &mut constraints {
        // A fixed width is the column's preferred width, whatever the
        // content would like, but it never squeezes the content.
        if let Some(fixed) = column.fixed {
            column.max = fixed;
        }
        column.max = column.max.max(column.min);
    }
    constraints
}

/// Divides `available` width among columns.
///
/// Percentage columns get their share first. Then, if there is room,
/// fixed-width columns get their width and the other columns their
/// max-content width, and only those other columns share any surplus, in
/// proportion to their max-content width. With less room, the other
/// columns shrink toward their min-content width before fixed-width ones
/// do. No column is ever narrower than its min-content width.
fn distribute(columns: &[Column], available: f32) -> Vec<f32> {
    let mut widths = vec![0.0; columns.len()];
    let mut remaining = available;
    for (i, column) in columns.iter().enumerate() {
        if let Some(fraction) = column.percent {
            widths[i] = (fraction * available).max(column.min);
            remaining -= widths[i];
        }
    }

    let (fixed, auto): (Vec<usize>, Vec<usize>) = (0..columns.len())
        .filter(|&i| columns[i].percent.is_none())
        .partition(|&i| columns[i].fixed.is_some());
    let total = |set: &[usize], field: fn(&Column) -> f32| -> f32 {
        set.iter().map(|&i| field(&columns[i])).sum()
    };
    let (auto_min, fixed_max) = (total(&auto, |c| c.min), total(&fixed, |c| c.max));
    if auto.is_empty() {
        fit(columns, &fixed, remaining, &mut widths);
    } else if remaining >= fixed_max + auto_min {
        for &i in &fixed {
            widths[i] = columns[i].max;
        }
        fit(columns, &auto, remaining - fixed_max, &mut widths);
    } else {
        for &i in &auto {
            widths[i] = columns[i].min;
        }
        fit(columns, &fixed, remaining - auto_min, &mut widths);
    }
    widths
}

/// Sizes the columns in `set` to share `space`: their max-content widths
/// plus a share of any surplus in proportion to them, or with too little
/// room, widths between min- and max-content, never below min-content.
fn fit(columns: &[Column], set: &[usize], space: f32, widths: &mut [f32]) {
    let min_total: f32 = set.iter().map(|&i| columns[i].min).sum();
    let max_total: f32 = set.iter().map(|&i| columns[i].max).sum();
    for &i in set {
        let Column { min, max, .. } = columns[i];
        widths[i] = if space >= max_total {
            let surplus = space - max_total;
            let share = if max_total > 0.0 {
                max / max_total
            } else {
                1.0 / set.len() as f32
            };
            max + surplus * share
        } else if space > min_total {
            let t = (space - min_total) / (max_total - min_total);
            min + (max - min) * t
        } else {
            min
        };
    }
}

/// Column widths for `table-layout: fixed` (CSS 2.1 section 17.5.2.1),
/// sharing `available`: a `<col>`'s width, else the width of the first
/// row's cell in the column (split over its columns if it spans several);
/// the remaining columns share what is left equally. Content is not
/// measured. If every column has a width and they fall short, they grow
/// in proportion.
fn fixed_column_widths(
    rows: &[Row],
    cols: &[Dimension],
    columns: usize,
    spacing: f32,
    available: f32,
) -> Vec<f32> {
    let resolve = |width: Dimension| width.resolve(available).map(|w| w.max(0.0));
    let mut widths: Vec<Option<f32>> = (0..columns)
        .map(|i| cols.get(i).copied().and_then(resolve))
        .collect();
    if let Some(first) = rows.first() {
        for cell in &first.cells {
            let range = cell.column..(cell.column + cell.span).min(columns);
            if range.is_empty() || widths[range.clone()].iter().any(Option::is_some) {
                continue;
            }
            if let Some(width) = resolve(cell.width()) {
                let count = range.len() as f32;
                let each = ((width - spacing * (count - 1.0)) / count).max(0.0);
                widths[range].fill(Some(each));
            }
        }
    }

    let known: f32 = widths.iter().flatten().sum();
    let unknown = widths.iter().filter(|w| w.is_none()).count();
    let rest = if unknown > 0 {
        ((available - known) / unknown as f32).max(0.0)
    } else {
        0.0
    };
    let scale = if unknown == 0 && known > 0.0 && known < available {
        available / known
    } else {
        1.0
    };
    widths
        .into_iter()
        .map(|width| width.map_or(rest, |width| width * scale))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: &[f32], b: &[f32]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(a, b)| (a - b).abs() < 0.01)
    }

    fn auto(min: f32, max: f32) -> Column {
        Column {
            min,
            max,
            ..Column::default()
        }
    }

    fn fixed(min: f32, width: f32) -> Column {
        Column {
            min,
            max: width.max(min),
            fixed: Some(width),
            percent: None,
        }
    }

    #[test]
    fn columns_get_max_content_when_it_fits() {
        let widths = distribute(&[auto(10.0, 30.0), auto(20.0, 60.0)], 90.0);
        assert!(approx(&widths, &[30.0, 60.0]), "{widths:?}");
    }

    #[test]
    fn surplus_is_shared_in_proportion() {
        let widths = distribute(&[auto(10.0, 30.0), auto(20.0, 60.0)], 180.0);
        assert!(approx(&widths, &[60.0, 120.0]), "{widths:?}");
    }

    #[test]
    fn tight_space_interpolates_between_min_and_max() {
        let widths = distribute(&[auto(10.0, 30.0), auto(20.0, 60.0)], 60.0);
        assert!(approx(&widths, &[20.0, 40.0]), "{widths:?}");
    }

    #[test]
    fn columns_never_shrink_below_min_content() {
        let widths = distribute(&[auto(10.0, 30.0), auto(20.0, 60.0)], 5.0);
        assert!(approx(&widths, &[10.0, 20.0]), "{widths:?}");
    }

    #[test]
    fn percentage_columns_take_their_share_first() {
        let percent = Column {
            percent: Some(0.75),
            ..auto(10.0, 50.0)
        };
        let widths = distribute(&[percent, auto(10.0, 50.0)], 200.0);
        assert!(approx(&widths, &[150.0, 50.0]), "{widths:?}");
    }

    #[test]
    fn fixed_columns_keep_their_width_and_auto_ones_take_the_rest() {
        // The usual invoice row: a narrow number, the description, the amount.
        let columns = [fixed(10.0, 30.0), auto(20.0, 80.0), fixed(30.0, 75.0)];
        let widths = distribute(&columns, 400.0);
        assert!(approx(&widths, &[30.0, 295.0, 75.0]), "{widths:?}");
        // Tight: the description gives way first...
        let widths = distribute(&columns, 125.0);
        assert!(approx(&widths, &[30.0, 20.0, 75.0]), "{widths:?}");
        // ...then the fixed columns, down to their content.
        let widths = distribute(&columns, 80.0);
        assert!(approx(&widths, &[16.154, 20.0, 43.846]), "{widths:?}");
    }

    #[test]
    fn fixed_columns_share_the_surplus_when_nothing_else_can() {
        let widths = distribute(&[fixed(0.0, 50.0), fixed(0.0, 150.0)], 400.0);
        assert!(approx(&widths, &[100.0, 300.0]), "{widths:?}");
    }

    #[test]
    fn fixed_layout_shares_what_widths_leave_equally() {
        let widths = fixed_column_widths(&[], &[Dimension::Pt(100.0)], 3, 0.0, 300.0);
        assert!(approx(&widths, &[100.0, 100.0, 100.0]), "{widths:?}");
        let widths = fixed_column_widths(
            &[],
            &[Dimension::Percent(0.25), Dimension::Pt(50.0)],
            2,
            0.0,
            300.0,
        );
        // Every column has a width and they fall short: they grow.
        assert!(approx(&widths, &[180.0, 120.0]), "{widths:?}");
    }

    #[test]
    fn grid_positions_columns_with_spacing() {
        let grid = Grid::new(vec![10.0, 20.0, 30.0], 2.0);
        assert_eq!(grid.starts, [2.0, 14.0, 36.0]);
        assert_eq!(grid.span_width(0, 2), 32.0);
        assert_eq!(grid.span_width(2, 5), 30.0);
        assert_eq!(grid.total_width(), 68.0);
    }
}
