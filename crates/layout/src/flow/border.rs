//! Painting a box's background and borders: square or rounded corners,
//! and solid, double, dashed or dotted edges.

use std::sync::Arc;

use presswork_style::{BackgroundSize, BorderStyle, Color, ComputedStyle, Corner, Dimension, Side};

use crate::fragment::{Fill, ImageData, ImageFill, Item, PathOp, Shape, ShapePaint};
use crate::geometry::{Point, Rect, Size};

/// The background and borders of a box.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BoxPaint {
    pub background: Color,
    pub image: Option<BackgroundImage>,
    /// Width and color of each border; a width of zero means no border.
    pub border: [(Side, f32, Color); 4],
    /// The style of each border, in the order of `border`.
    pub styles: [BorderStyle; 4],
    /// Horizontal and vertical radius of each corner, indexed like
    /// [`Corner::ALL`]; percentages are of the box's width and height.
    pub radii: [(Dimension, Dimension); 4],
}

/// A loaded background image and how it is laid out.
#[derive(Debug, Clone)]
pub(crate) struct BackgroundImage {
    pub data: Arc<ImageData>,
    pub repeat: (bool, bool),
    pub position: ((Dimension, bool), (Dimension, bool)),
    pub size: BackgroundSize,
}

impl PartialEq for BackgroundImage {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.data, &other.data)
            && self.repeat == other.repeat
            && self.position == other.position
            && self.size == other.size
    }
}

impl BackgroundImage {
    /// The image of `style`, given its loaded data.
    pub fn new(style: &ComputedStyle, data: Arc<ImageData>) -> Self {
        Self {
            data,
            repeat: style.background_repeat,
            position: style.background_position,
            size: style.background_size,
        }
    }

    /// The size of one copy in a positioning area `area` large (CSS
    /// Backgrounds 3, section 3.9).
    fn tile_size(&self, area: Size) -> Size {
        let natural = self.data.natural_size();
        if natural.width <= 0.0 || natural.height <= 0.0 {
            return Size::default();
        }
        let ratio = natural.width / natural.height;
        let scaled = |scale: f32| Size::new(natural.width * scale, natural.height * scale);
        match self.size {
            BackgroundSize::Cover => {
                scaled((area.width / natural.width).max(area.height / natural.height))
            }
            BackgroundSize::Contain => {
                scaled((area.width / natural.width).min(area.height / natural.height))
            }
            BackgroundSize::Explicit(width, height) => {
                let width = width.and_then(|w| w.resolve(area.width));
                let height = height.and_then(|h| h.resolve(area.height));
                match (width, height) {
                    (Some(w), Some(h)) => Size::new(w, h),
                    (Some(w), None) => Size::new(w, w / ratio),
                    (None, Some(h)) => Size::new(h * ratio, h),
                    (None, None) => natural,
                }
            }
        }
    }
}

/// Bézier control point distance for a quarter ellipse, as a fraction of
/// the radius.
const KAPPA: f32 = 0.552_284_8;

/// Below this width a double border has no room for two lines and a gap,
/// and is drawn solid (3px, as in browsers).
pub(super) const MIN_DOUBLE: f32 = 2.25;

impl BoxPaint {
    /// Paints nothing.
    pub const INVISIBLE: Self = Self {
        background: Color::TRANSPARENT,
        image: None,
        border: [
            (Side::Top, 0.0, Color::TRANSPARENT),
            (Side::Right, 0.0, Color::TRANSPARENT),
            (Side::Bottom, 0.0, Color::TRANSPARENT),
            (Side::Left, 0.0, Color::TRANSPARENT),
        ],
        styles: [BorderStyle::Solid; 4],
        radii: [(Dimension::ZERO, Dimension::ZERO); 4],
    };

    /// The paint for a box with this style and background image (loaded
    /// by the caller), or `None` if nothing is visible.
    pub fn from_style(style: &ComputedStyle, image: Option<BackgroundImage>) -> Option<Self> {
        let widths = style.border_widths();
        let border = Side::ALL.map(|side| (side, widths.get(side), style.border_color(side)));
        let visible = !style.background_color.is_transparent()
            || image.is_some()
            || border
                .iter()
                .any(|(_, width, color)| *width > 0.0 && !color.is_transparent());
        visible.then_some(Self {
            background: style.background_color,
            image,
            border,
            styles: Side::ALL.map(|side| style.border.get(side).style),
            radii: style.border_radius,
        })
    }

    /// Removes the border on `side`, and rounding from the corners on that
    /// side: the box continues there (on the next page or line).
    pub fn without_border(mut self, side: Side) -> Self {
        for edge in &mut self.border {
            if edge.0 == side {
                edge.1 = 0.0;
            }
        }
        let corners = match side {
            Side::Top => [Corner::TopLeft, Corner::TopRight],
            Side::Right => [Corner::TopRight, Corner::BottomRight],
            Side::Bottom => [Corner::BottomRight, Corner::BottomLeft],
            Side::Left => [Corner::BottomLeft, Corner::TopLeft],
        };
        for corner in corners {
            self.radii[corner as usize] = (Dimension::ZERO, Dimension::ZERO);
        }
        self
    }

    /// Appends the items for a box whose border box is `rect`.
    pub fn fills(&self, rect: Rect, out: &mut Vec<Item>) {
        self.fills_piece(rect, rect, out);
    }

    /// Appends the items for the part `piece` of a box whose whole border
    /// box is `whole`: a box split across pages. A background image is
    /// placed in the whole box, so that the pieces line up.
    pub fn fills_piece(&self, piece: Rect, whole: Rect, out: &mut Vec<Item>) {
        let radii = resolve_radii(&self.radii, piece.size);
        let square = radii.iter().all(|&(h, v)| h <= 0.0 || v <= 0.0);
        let outline = if square {
            rect_path(piece)
        } else {
            rounded_rect(piece, radii)
        };
        if !self.background.is_transparent() {
            if square {
                let Rect { origin, size } = piece;
                fill(
                    out,
                    origin.x,
                    origin.y,
                    size.width,
                    size.height,
                    self.background,
                );
            } else {
                out.push(Item::Shape(Shape {
                    path: outline.clone(),
                    paint: ShapePaint::Fill {
                        color: self.background,
                        even_odd: false,
                    },
                    clip: None,
                }));
            }
        }
        if let Some(image) = &self.image {
            self.image_fill(image, piece, whole, outline, out);
        }
        if square {
            self.square(piece, out);
        } else {
            self.rounded(piece, radii, out);
        }
    }

    /// Appends the background image: sized and positioned in the padding
    /// box of `whole`, painted within `outline` around `piece`.
    fn image_fill(
        &self,
        image: &BackgroundImage,
        piece: Rect,
        whole: Rect,
        outline: Vec<PathOp>,
        out: &mut Vec<Item>,
    ) {
        let widths = Side::ALL.map(|side| self.edge(side).0);
        let area = inset(whole, widths);
        let size = image.tile_size(area.size);
        if size.width <= 0.0 || size.height <= 0.0 {
            return;
        }
        // From the start of the area or, `from_end`, its far edge.
        let offset = |(value, from_end): (Dimension, bool), free: f32| {
            let offset = match value {
                Dimension::Percent(fraction) => fraction * free,
                other => other.resolve(free).unwrap_or(0.0),
            };
            if from_end { free - offset } else { offset }
        };
        let tile = Rect::new(
            area.origin.x + offset(image.position.0, area.size.width - size.width),
            area.origin.y + offset(image.position.1, area.size.height - size.height),
            size.width,
            size.height,
        );
        // Along an axis that does not repeat, only the one copy's band is
        // painted.
        let (mut x0, mut x1) = (piece.origin.x, piece.origin.x + piece.size.width);
        let (mut y0, mut y1) = (piece.origin.y, piece.origin.y + piece.size.height);
        if !image.repeat.0 {
            x0 = x0.max(tile.origin.x);
            x1 = x1.min(tile.origin.x + size.width);
        }
        if !image.repeat.1 {
            y0 = y0.max(tile.origin.y);
            y1 = y1.min(tile.origin.y + size.height);
        }
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let step = Size::new(
            if image.repeat.0 {
                size.width
            } else {
                size.width + piece.size.width
            },
            if image.repeat.1 {
                size.height
            } else {
                size.height + piece.size.height
            },
        );
        out.push(Item::ImageFill(ImageFill {
            clip: outline,
            rect: Rect::new(x0, y0, x1 - x0, y1 - y0),
            tile,
            step,
            data: Arc::clone(&image.data),
        }));
    }

    /// The width and color of the border on `side`, and its style.
    fn edge(&self, side: Side) -> (f32, Color, BorderStyle) {
        let index = Side::ALL.iter().position(|&s| s == side).unwrap_or(0);
        let (_, width, color) = self.border[index];
        (width, color, self.styles[index])
    }

    /// A box with square corners: filled rectangles, and strokes for
    /// dashed and dotted edges.
    fn square(&self, rect: Rect, out: &mut Vec<Item>) {
        let Rect { origin, size } = rect;
        let left = self.edge(Side::Left).0.min(size.width);
        let right = self.edge(Side::Right).0.min(size.width);
        for (side, width, color) in self.border {
            if width <= 0.0 || color.is_transparent() {
                continue;
            }
            let (_, _, style) = self.edge(side);
            let width = match side {
                Side::Top | Side::Bottom => width.min(size.height),
                Side::Left | Side::Right => width.min(size.width),
            };
            // The edge's rectangle, along its length: start, length, and
            // the position across it of its outer edge.
            let (x0, y0) = (origin.x, origin.y);
            let (x1, y1) = (origin.x + size.width, origin.y + size.height);
            match style {
                BorderStyle::Dashed | BorderStyle::Dotted => {
                    let dotted = style == BorderStyle::Dotted;
                    let half = width / 2.0;
                    let (from, to) = match side {
                        Side::Top => (Point::new(x0, y0 + half), Point::new(x1, y0 + half)),
                        Side::Bottom => (Point::new(x0, y1 - half), Point::new(x1, y1 - half)),
                        Side::Left => (Point::new(x0 + half, y0), Point::new(x0 + half, y1)),
                        Side::Right => (Point::new(x1 - half, y0), Point::new(x1 - half, y1)),
                    };
                    dashed_line(out, from, to, width, color, dotted);
                }
                BorderStyle::Double if width >= MIN_DOUBLE => {
                    let third = width / 3.0;
                    // The inner line stops where the inner lines of the
                    // sides meeting it do.
                    let (inner_left, inner_right) = (left * 2.0 / 3.0, right * 2.0 / 3.0);
                    match side {
                        Side::Top => {
                            fill(out, x0, y0, size.width, third, color);
                            let w = size.width - inner_left - inner_right;
                            fill(out, x0 + inner_left, y0 + 2.0 * third, w, third, color);
                        }
                        Side::Bottom => {
                            fill(out, x0, y1 - third, size.width, third, color);
                            let w = size.width - inner_left - inner_right;
                            fill(out, x0 + inner_left, y1 - width, w, third, color);
                        }
                        Side::Left => {
                            fill(out, x0, y0, third, size.height, color);
                            let (top, bottom) = self.inner_thirds();
                            let h = size.height - top - bottom;
                            fill(out, x0 + 2.0 * third, y0 + top, third, h, color);
                        }
                        Side::Right => {
                            fill(out, x1 - third, y0, third, size.height, color);
                            let (top, bottom) = self.inner_thirds();
                            let h = size.height - top - bottom;
                            fill(out, x1 - width, y0 + top, third, h, color);
                        }
                    }
                }
                _ => match side {
                    Side::Top => fill(out, x0, y0, size.width, width, color),
                    Side::Bottom => fill(out, x0, y1 - width, size.width, width, color),
                    Side::Left => fill(out, x0, y0, width, size.height, color),
                    Side::Right => fill(out, x1 - width, y0, width, size.height, color),
                },
            }
        }
    }

    /// Where the inner lines of double left and right borders start and
    /// end: two thirds into the top and bottom borders.
    fn inner_thirds(&self) -> (f32, f32) {
        (
            self.edge(Side::Top).0 * 2.0 / 3.0,
            self.edge(Side::Bottom).0 * 2.0 / 3.0,
        )
    }

    /// A box with rounded corners: the background is a rounded shape, the
    /// border the ring between it and the padding box.
    fn rounded(&self, rect: Rect, radii: [(f32, f32); 4], out: &mut Vec<Item>) {
        let edges = Side::ALL.map(|side| self.edge(side));
        let visible = |&(width, color, style): &(f32, Color, BorderStyle)| {
            width > 0.0 && !color.is_transparent() && style.is_visible()
        };
        if !edges.iter().any(visible) {
            return;
        }
        let widths = edges.map(|(width, ..)| width);
        let uniform = edges.iter().all(|edge| *edge == edges[0]) && visible(&edges[0]);
        let (width, color, style) = edges[0];

        if uniform && matches!(style, BorderStyle::Dashed | BorderStyle::Dotted) {
            let half = [width / 2.0; 4];
            let path = rounded_rect(inset(rect, half), inner_radii(radii, half));
            let perimeter = perimeter(inset(rect, half), inner_radii(radii, half));
            let dotted = style == BorderStyle::Dotted;
            out.push(Item::Shape(Shape {
                path,
                paint: ShapePaint::Stroke {
                    color,
                    width,
                    dash: Some(dash_pattern(perimeter, width, dotted, true)),
                    round: dotted,
                },
                clip: None,
            }));
            return;
        }
        if uniform && style == BorderStyle::Double && width >= MIN_DOUBLE {
            let third = [width / 3.0; 4];
            let two_thirds = [width * 2.0 / 3.0; 4];
            out.push(ring(rect, radii, [0.0; 4], third, color, None));
            out.push(ring(rect, radii, two_thirds, widths, color, None));
            return;
        }

        let same_color =
            edges.iter().filter(|e| visible(e)).all(|e| e.1 == color) && edges.iter().all(visible);
        if same_color {
            out.push(ring(rect, radii, [0.0; 4], widths, color, None));
            return;
        }
        // Each side in its own color: the ring, clipped to the side's part
        // of the box, split along the lines from the outer corners
        // through the inner ones.
        for (index, side) in Side::ALL.into_iter().enumerate() {
            if !visible(&edges[index]) {
                continue;
            }
            let clip = side_region(rect, widths, side);
            out.push(ring(
                rect,
                radii,
                [0.0; 4],
                widths,
                edges[index].1,
                Some(clip),
            ));
        }
    }
}

/// Appends a filled rectangle, if it has an area and a color.
fn fill(out: &mut Vec<Item>, x: f32, y: f32, width: f32, height: f32, color: Color) {
    if width > 0.0 && height > 0.0 && !color.is_transparent() {
        out.push(Item::Fill(Fill {
            rect: Rect::new(x, y, width, height),
            color,
        }));
    }
}

/// A dashed or dotted line from `from` to `to`, `width` thick.
pub(super) fn dashed_line(
    out: &mut Vec<Item>,
    from: Point,
    to: Point,
    width: f32,
    color: Color,
    dotted: bool,
) {
    let length = (to.x - from.x).hypot(to.y - from.y);
    if length <= 0.0 {
        return;
    }
    let (mut from, mut to) = (from, to);
    if dotted {
        // Dots are round caps on zero-length dashes: keep them inside the
        // edge, with one in each corner.
        let inset = (width / 2.0).min(length / 2.0);
        let (dx, dy) = ((to.x - from.x) / length, (to.y - from.y) / length);
        from = Point::new(from.x + dx * inset, from.y + dy * inset);
        to = Point::new(to.x - dx * inset, to.y - dy * inset);
    }
    let length = (to.x - from.x).hypot(to.y - from.y);
    out.push(Item::Shape(Shape {
        path: vec![PathOp::Move(from), PathOp::Line(to)],
        paint: ShapePaint::Stroke {
            color,
            width,
            dash: Some(dash_pattern(length, width, dotted, false)),
            round: dotted,
        },
        clip: None,
    }));
}

/// Dash and gap lengths for a dashed or dotted line `length` long and
/// `width` wide, adjusted so that a whole number of them fits: an open
/// line starts and ends with a dash (or dot); a closed one repeats evenly.
fn dash_pattern(length: f32, width: f32, dotted: bool, closed: bool) -> (f32, f32) {
    if dotted {
        // Dots one width across, as far apart again.
        let period = 2.0 * width;
        let count = (length / period).round().max(1.0);
        return (0.0, length / count);
    }
    // Dashes three widths long with gaps as long, as in browsers.
    let dash = 3.0 * width;
    if closed {
        let count = (length / (2.0 * dash)).round().max(1.0);
        let period = length / count;
        return (period / 2.0, period / 2.0);
    }
    let count = ((length + dash) / (2.0 * dash)).round().max(1.0);
    if count <= 1.0 {
        return (length, 0.0);
    }
    let unit = length / (2.0 * count - 1.0);
    (unit, unit)
}

/// The radii in points, scaled down together if adjacent ones would
/// overlap (CSS Backgrounds 3, section 5.5).
fn resolve_radii(radii: &[(Dimension, Dimension); 4], size: Size) -> [(f32, f32); 4] {
    let resolved = radii.map(|(h, v)| {
        let h = h.resolve(size.width).unwrap_or(0.0).max(0.0);
        let v = v.resolve(size.height).unwrap_or(0.0).max(0.0);
        if h > 0.0 && v > 0.0 {
            (h, v)
        } else {
            (0.0, 0.0)
        }
    });
    let [tl, tr, br, bl] = resolved;
    let ratio = |length: f32, sum: f32| {
        if sum > 0.0 {
            length / sum
        } else {
            f32::INFINITY
        }
    };
    let factor = ratio(size.width, tl.0 + tr.0)
        .min(ratio(size.width, bl.0 + br.0))
        .min(ratio(size.height, tl.1 + bl.1))
        .min(ratio(size.height, tr.1 + br.1))
        .min(1.0);
    resolved.map(|(h, v)| (h * factor, v * factor))
}

/// The outline of `rect`.
fn rect_path(rect: Rect) -> Vec<PathOp> {
    let (x0, y0) = (rect.origin.x, rect.origin.y);
    let (x1, y1) = (x0 + rect.size.width, y0 + rect.size.height);
    vec![
        PathOp::Move(Point::new(x0, y0)),
        PathOp::Line(Point::new(x1, y0)),
        PathOp::Line(Point::new(x1, y1)),
        PathOp::Line(Point::new(x0, y1)),
        PathOp::Close,
    ]
}

/// `rect` shrunk by `widths` (top, right, bottom, left).
fn inset(rect: Rect, widths: [f32; 4]) -> Rect {
    let [top, right, bottom, left] = widths;
    Rect::new(
        rect.origin.x + left,
        rect.origin.y + top,
        (rect.size.width - left - right).max(0.0),
        (rect.size.height - top - bottom).max(0.0),
    )
}

/// The radii of a box inset by `widths` (top, right, bottom, left) from one
/// with `radii`.
fn inner_radii(radii: [(f32, f32); 4], widths: [f32; 4]) -> [(f32, f32); 4] {
    let [top, right, bottom, left] = widths;
    let [tl, tr, br, bl] = radii;
    let shrink =
        |(h, v): (f32, f32), across: f32, down: f32| ((h - across).max(0.0), (v - down).max(0.0));
    [
        shrink(tl, left, top),
        shrink(tr, right, top),
        shrink(br, right, bottom),
        shrink(bl, left, bottom),
    ]
}

/// The outline of `rect` with elliptical corners, clockwise from the top
/// left.
fn rounded_rect(rect: Rect, radii: [(f32, f32); 4]) -> Vec<PathOp> {
    let (x0, y0) = (rect.origin.x, rect.origin.y);
    let (x1, y1) = (x0 + rect.size.width, y0 + rect.size.height);
    let [tl, tr, br, bl] = radii;
    let p = Point::new;
    // A quarter ellipse from `start` to `end`, bulging towards `corner`.
    let arc = |start: Point, corner: Point, end: Point| {
        let c1 = p(
            start.x + (corner.x - start.x) * KAPPA,
            start.y + (corner.y - start.y) * KAPPA,
        );
        let c2 = p(
            end.x + (corner.x - end.x) * KAPPA,
            end.y + (corner.y - end.y) * KAPPA,
        );
        PathOp::Cubic(c1, c2, end)
    };
    vec![
        PathOp::Move(p(x0 + tl.0, y0)),
        PathOp::Line(p(x1 - tr.0, y0)),
        arc(p(x1 - tr.0, y0), p(x1, y0), p(x1, y0 + tr.1)),
        PathOp::Line(p(x1, y1 - br.1)),
        arc(p(x1, y1 - br.1), p(x1, y1), p(x1 - br.0, y1)),
        PathOp::Line(p(x0 + bl.0, y1)),
        arc(p(x0 + bl.0, y1), p(x0, y1), p(x0, y1 - bl.1)),
        PathOp::Line(p(x0, y0 + tl.1)),
        arc(p(x0, y0 + tl.1), p(x0, y0), p(x0 + tl.0, y0)),
        PathOp::Close,
    ]
}

/// Roughly the length of a rounded rectangle's outline (Ramanujan's
/// approximation for the corners).
fn perimeter(rect: Rect, radii: [(f32, f32); 4]) -> f32 {
    let [tl, tr, br, bl] = radii;
    let straight = (rect.size.width - tl.0 - tr.0).max(0.0)
        + (rect.size.width - bl.0 - br.0).max(0.0)
        + (rect.size.height - tl.1 - bl.1).max(0.0)
        + (rect.size.height - tr.1 - br.1).max(0.0);
    let quarter = |(a, b): (f32, f32)| {
        let h = ((a - b) / (a + b).max(f32::EPSILON)).powi(2);
        std::f32::consts::PI * (a + b) * (1.0 + 3.0 * h / (10.0 + (4.0 - 3.0 * h).sqrt())) / 4.0
    };
    straight + radii.iter().map(|&r| quarter(r)).sum::<f32>()
}

/// The area between `rect` inset by `from` and by `to` (top, right, bottom,
/// left), with corners following `radii`, filled with `color`.
fn ring(
    rect: Rect,
    radii: [(f32, f32); 4],
    from: [f32; 4],
    to: [f32; 4],
    color: Color,
    clip: Option<Vec<Point>>,
) -> Item {
    let mut path = rounded_rect(inset(rect, from), inner_radii(radii, from));
    path.extend(rounded_rect(inset(rect, to), inner_radii(radii, to)));
    Item::Shape(Shape {
        path,
        paint: ShapePaint::Fill {
            color,
            even_odd: true,
        },
        clip,
    })
}

/// The part of `rect` that belongs to the border on `side`: a trapezoid
/// whose slanted sides run from the outer corners through the inner ones
/// (given by the border `widths`), extended to the middle of the box.
fn side_region(rect: Rect, widths: [f32; 4], side: Side) -> Vec<Point> {
    let [top, right, bottom, left] = widths;
    let (x0, y0) = (rect.origin.x, rect.origin.y);
    let (x1, y1) = (x0 + rect.size.width, y0 + rect.size.height);
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    // From a corner along the direction (dx, dy) until the middle of the
    // box is reached across or down.
    let towards = |corner: Point, dx: f32, dy: f32| {
        let reach = |delta: f32, distance: f32| {
            if delta.abs() > 0.0 {
                distance.abs() / delta.abs()
            } else {
                f32::INFINITY
            }
        };
        let scale = reach(dx, cx - corner.x).min(reach(dy, cy - corner.y));
        if scale.is_finite() {
            Point::new(corner.x + dx * scale, corner.y + dy * scale)
        } else {
            Point::new(cx, cy)
        }
    };
    let tl = towards(Point::new(x0, y0), left, top);
    let tr = towards(Point::new(x1, y0), -right, top);
    let br = towards(Point::new(x1, y1), -right, -bottom);
    let bl = towards(Point::new(x0, y1), left, -bottom);
    match side {
        Side::Top => vec![Point::new(x0, y0), Point::new(x1, y0), tr, tl],
        Side::Right => vec![Point::new(x1, y0), Point::new(x1, y1), br, tr],
        Side::Bottom => vec![Point::new(x1, y1), Point::new(x0, y1), bl, br],
        Side::Left => vec![Point::new(x0, y1), Point::new(x0, y0), tl, bl],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashes_fit_the_line_evenly() {
        // 3pt dashes and gaps on a 1pt line: 21pt holds four dashes and
        // three gaps exactly.
        assert_eq!(dash_pattern(21.0, 1.0, false, false), (3.0, 3.0));
        let (dash, gap) = dash_pattern(20.0, 1.0, false, false);
        assert!((4.0 * dash + 3.0 * gap - 20.0).abs() < 1e-4);
        // Too short for a gap: one dash.
        assert_eq!(dash_pattern(4.0, 1.0, false, false), (4.0, 0.0));
        // Dots: zero-length dashes, spaced evenly.
        let (dot, spacing) = dash_pattern(10.0, 1.0, true, false);
        assert_eq!(dot, 0.0);
        assert!((10.0 / spacing).fract().abs() < 1e-4);
    }

    #[test]
    fn radii_shrink_together_when_they_would_overlap() {
        let pt = Dimension::Pt;
        let radii = [(pt(30.0), pt(30.0)); 4];
        let resolved = resolve_radii(&radii, Size::new(40.0, 100.0));
        assert_eq!(resolved[0], (20.0, 20.0));
        let half = [(Dimension::Percent(0.5), Dimension::Percent(0.5)); 4];
        assert_eq!(resolve_radii(&half, Size::new(40.0, 20.0))[2], (20.0, 10.0));
    }
}
