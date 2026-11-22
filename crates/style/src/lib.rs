//! CSS parsing, cascade and computed styles.
//!
//! - [`stylesheet`] parses stylesheets and `style=""` attributes with
//!   `cssparser`; [`properties`] parses individual declarations.
//! - [`selector`] matches selectors against `presswork_dom` documents with
//!   the `selectors` crate.
//! - [`cascade`] collects the declarations that apply to an element (from
//!   the built-in user-agent stylesheet, a user stylesheet, presentational
//!   attributes, author stylesheets (loading `<link>` and `@import`ed ones)
//!   and `style=""`), orders them by origin, importance, specificity and
//!   source order, and produces a [`ComputedStyle`].
//!
//! Start with [`Stylist::for_document`], then call [`Stylist::style`] for
//! each element, top down.

pub mod cascade;
pub mod computed;
mod hints;
pub mod properties;
mod selector;
pub mod stylesheet;
pub mod values;

pub use cascade::{Pseudo, Stylist};
pub use computed::{
    BackgroundSize, BorderSide, ComputedStyle, DEFAULT_FONT_SIZE, TextDecorations, TextIndent,
};
pub use properties::{
    BorderCollapse, BorderStyle, BoxSizing, Break, BreakInside, Content, ContentItem, Corner,
    CounterChanges, Display, FontFamilyName, GenericFamily, LineHeight, ListStyleType, TableLayout,
    TextAlign, TextDecorationLine, TextTransform, VerticalAlign, WhiteSpace,
};
pub use stylesheet::{PageRule, PageSizeValue};
pub use values::{Color, ColorValue, Dimension, Length, Side, Sides};
