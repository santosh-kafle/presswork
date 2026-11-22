//! Selector parsing and matching, on top of the `selectors` crate.
//!
//! `selectors` implements the Selectors Level 4 grammar and matching
//! algorithm; this module only tells it how to walk a `presswork_dom`
//! document ([`ElementRef`]) and which pseudo-classes exist.

use std::borrow::Borrow;
use std::fmt;

use cssparser::{Parser, ToCss};
use precomputed_hash::PrecomputedHash;
use presswork_dom::{Document, ElementData, NodeId, NodeKind};
use selectors::attr::{AttrSelectorOperation, CaseSensitivity, NamespaceConstraint};
use selectors::bloom::BloomFilter;
use selectors::context::{
    MatchingContext, MatchingForInvalidation, MatchingMode, NeedsSelectorFlags, QuirksMode,
    SelectorCaches,
};
use selectors::matching::{ElementSelectorFlags, matches_selector};
use selectors::parser::{
    NonTSPseudoClass, ParseRelative, PseudoElement as PseudoElementTrait, Selector,
    SelectorParseErrorKind,
};
use selectors::{OpaqueElement, SelectorImpl, SelectorList};

use crate::values::Invalid;

/// The `SelectorImpl` for presswork documents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Simple;

/// A string usable as an identifier, name or value in a selector.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub(crate) struct CssString(String);

impl From<&str> for CssString {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl AsRef<str> for CssString {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for CssString {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl ToCss for CssString {
    fn to_css<W: fmt::Write>(&self, dest: &mut W) -> fmt::Result {
        cssparser::serialize_identifier(&self.0, dest)
    }
}

impl PrecomputedHash for CssString {
    fn precomputed_hash(&self) -> u32 {
        // FNV-1a; only used by `selectors` for its bloom filters.
        self.0.bytes().fold(0x811c_9dc5, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
        })
    }
}

/// Pseudo-classes that depend on state rather than tree structure. A
/// static document has no interaction state, so most never match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PseudoClass {
    Link,
    AnyLink,
    Checked,
    Disabled,
    Enabled,
    /// `:hover`, `:focus`, `:visited` and the like: valid, never matching.
    Never,
}

impl ToCss for PseudoClass {
    fn to_css<W: fmt::Write>(&self, dest: &mut W) -> fmt::Result {
        dest.write_str(match self {
            PseudoClass::Link => ":link",
            PseudoClass::AnyLink => ":any-link",
            PseudoClass::Checked => ":checked",
            PseudoClass::Disabled => ":disabled",
            PseudoClass::Enabled => ":enabled",
            PseudoClass::Never => ":hover",
        })
    }
}

impl NonTSPseudoClass for PseudoClass {
    fn is_active_or_hover(&self) -> bool {
        false
    }

    fn is_user_action_state(&self) -> bool {
        false
    }
}

/// Pseudo-elements. `::before` and `::after` generate content; rules for
/// others parse, so that they are valid, but are skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PseudoElement {
    Before,
    After,
    Other,
}

impl ToCss for PseudoElement {
    fn to_css<W: fmt::Write>(&self, dest: &mut W) -> fmt::Result {
        dest.write_str(match self {
            PseudoElement::Before => "::before",
            PseudoElement::After => "::after",
            PseudoElement::Other => "::marker",
        })
    }
}

impl PseudoElementTrait for PseudoElement {}

impl SelectorImpl for Simple {
    type ExtraMatchingData<'a> = ();
    type AttrValue = CssString;
    type Identifier = CssString;
    type LocalName = CssString;
    type NamespaceUrl = CssString;
    type NamespacePrefix = CssString;
    type BorrowedNamespaceUrl = str;
    type BorrowedLocalName = str;
    type NonTSPseudoClass = PseudoClass;
    type PseudoElement = PseudoElement;
}

/// Parser hooks: which pseudo-classes and pseudo-elements exist.
struct SelectorParser;

impl<'i> selectors::Parser<'i> for SelectorParser {
    type Impl = Simple;
    type Error = SelectorParseErrorKind;

    fn parse_is_and_where(&self) -> bool {
        true
    }

    fn parse_non_ts_pseudo_class(
        &self,
        name: &str,
    ) -> Result<PseudoClass, cssparser::ParseError<Self::Error>> {
        Ok(match name.to_ascii_lowercase().as_str() {
            "link" => PseudoClass::Link,
            "any-link" => PseudoClass::AnyLink,
            "checked" => PseudoClass::Checked,
            "disabled" => PseudoClass::Disabled,
            "enabled" => PseudoClass::Enabled,
            "hover" | "active" | "focus" | "focus-within" | "focus-visible" | "visited"
            | "target" => PseudoClass::Never,
            _ => {
                return Err(cssparser::ParseError::custom(
                    SelectorParseErrorKind::UnsupportedPseudoClassOrElement,
                ));
            }
        })
    }

    fn parse_pseudo_element(
        &self,
        name: &str,
    ) -> Result<PseudoElement, cssparser::ParseError<Self::Error>> {
        Ok(match name.to_ascii_lowercase().as_str() {
            "before" => PseudoElement::Before,
            "after" => PseudoElement::After,
            "first-line" | "first-letter" | "marker" | "placeholder" | "selection" => {
                PseudoElement::Other
            }
            _ => {
                return Err(cssparser::ParseError::custom(
                    SelectorParseErrorKind::UnsupportedPseudoClassOrElement,
                ));
            }
        })
    }
}

/// Parses a comma-separated selector list. Fails if any selector in the
/// list is invalid, which invalidates the whole rule, as CSS requires.
pub(crate) fn parse_selector_list(input: &mut Parser<'_>) -> Result<SelectorList<Simple>, Invalid> {
    SelectorList::parse(&SelectorParser, input, ParseRelative::No).map_err(Invalid::from)
}

/// An element of a document, as `selectors` sees it.
#[derive(Clone, Copy)]
pub(crate) struct ElementRef<'a> {
    doc: &'a Document,
    id: NodeId,
}

impl<'a> ElementRef<'a> {
    pub fn document(&self) -> &'a Document {
        self.doc
    }

    pub fn node(&self) -> NodeId {
        self.id
    }
}

impl fmt::Debug for ElementRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{}> {}", self.data().name, self.id)
    }
}

impl<'a> ElementRef<'a> {
    /// Wraps `id`, which must be an element of `doc`.
    pub fn new(doc: &'a Document, id: NodeId) -> Option<Self> {
        doc.element(id).map(|_| Self { doc, id })
    }

    fn data(&self) -> &'a ElementData {
        self.doc
            .element(self.id)
            .expect("ElementRef wraps an element")
    }

    fn sibling_elements(&self) -> impl DoubleEndedIterator<Item = NodeId> + 'a {
        let doc = self.doc;
        let siblings: &'a [NodeId] = match doc.node(self.id).parent {
            Some(parent) => &doc.node(parent).children,
            None => &[],
        };
        siblings
            .iter()
            .copied()
            .filter(move |&id| doc.element(id).is_some())
    }

    fn wrap(&self, id: NodeId) -> Self {
        Self { doc: self.doc, id }
    }

    /// Whether `selector` matches this element.
    /// Whether `selector` matches the element or, if `pseudo`, its
    /// pseudo-element (the selector's pseudo-element is checked by the
    /// caller).
    pub fn matches(
        &self,
        selector: &Selector<Simple>,
        pseudo: bool,
        caches: &mut SelectorCaches,
    ) -> bool {
        let quirks = if self.doc.quirks_mode {
            QuirksMode::Quirks
        } else {
            QuirksMode::NoQuirks
        };
        let mode = if pseudo {
            MatchingMode::ForStatelessPseudoElement
        } else {
            MatchingMode::Normal
        };
        let mut context = MatchingContext::new(
            mode,
            None,
            caches,
            quirks,
            NeedsSelectorFlags::No,
            MatchingForInvalidation::No,
        );
        matches_selector(selector, 0, None, *self, &mut context)
    }
}

impl selectors::Element for ElementRef<'_> {
    type Impl = Simple;

    fn opaque(&self) -> OpaqueElement {
        OpaqueElement::new(self.doc.node(self.id))
    }

    fn parent_element(&self) -> Option<Self> {
        let parent = self.doc.node(self.id).parent?;
        ElementRef::new(self.doc, parent)
    }

    fn parent_node_is_shadow_root(&self) -> bool {
        false
    }

    fn containing_shadow_host(&self) -> Option<Self> {
        None
    }

    fn is_pseudo_element(&self) -> bool {
        false
    }

    fn prev_sibling_element(&self) -> Option<Self> {
        let id = self.id;
        self.sibling_elements()
            .take_while(|&sibling| sibling != id)
            .last()
            .map(|sibling| self.wrap(sibling))
    }

    fn next_sibling_element(&self) -> Option<Self> {
        let id = self.id;
        self.sibling_elements()
            .skip_while(|&sibling| sibling != id)
            .nth(1)
            .map(|sibling| self.wrap(sibling))
    }

    fn first_element_child(&self) -> Option<Self> {
        self.doc
            .children(self.id)
            .find(|&child| self.doc.element(child).is_some())
            .map(|child| self.wrap(child))
    }

    fn is_html_element_in_html_document(&self) -> bool {
        self.data().namespace == HTML_NAMESPACE
    }

    fn has_local_name(&self, local_name: &str) -> bool {
        self.data().name == local_name
    }

    fn has_namespace(&self, ns: &str) -> bool {
        self.data().namespace == ns
    }

    fn is_same_type(&self, other: &Self) -> bool {
        let (a, b) = (self.data(), other.data());
        a.name == b.name && a.namespace == b.namespace
    }

    fn attr_matches(
        &self,
        _ns: &NamespaceConstraint<&CssString>,
        local_name: &CssString,
        operation: &AttrSelectorOperation<&CssString>,
    ) -> bool {
        self.data()
            .attr(local_name.as_ref())
            .is_some_and(|value| operation.eval_str(value))
    }

    fn match_non_ts_pseudo_class(
        &self,
        pseudo_class: &PseudoClass,
        _context: &mut MatchingContext<'_, Simple>,
    ) -> bool {
        let has = |name| self.data().attr(name).is_some();
        match pseudo_class {
            PseudoClass::Link | PseudoClass::AnyLink => self.is_link(),
            PseudoClass::Checked => has("checked"),
            PseudoClass::Disabled => has("disabled"),
            PseudoClass::Enabled => {
                matches!(
                    self.data().name.as_str(),
                    "input" | "button" | "select" | "textarea"
                ) && !has("disabled")
            }
            PseudoClass::Never => false,
        }
    }

    fn match_pseudo_element(
        &self,
        _pseudo_element: &PseudoElement,
        _context: &mut MatchingContext<'_, Simple>,
    ) -> bool {
        false
    }

    fn apply_selector_flags(&self, _flags: ElementSelectorFlags) {}

    fn is_link(&self) -> bool {
        let data = self.data();
        matches!(data.name.as_str(), "a" | "area" | "link") && data.attr("href").is_some()
    }

    fn is_html_slot_element(&self) -> bool {
        false
    }

    fn has_id(&self, id: &CssString, case_sensitivity: CaseSensitivity) -> bool {
        self.data()
            .attr("id")
            .is_some_and(|value| case_sensitivity.eq(value.as_bytes(), id.0.as_bytes()))
    }

    fn has_class(&self, name: &CssString, case_sensitivity: CaseSensitivity) -> bool {
        self.data().attr("class").is_some_and(|classes| {
            classes
                .split_ascii_whitespace()
                .any(|class| case_sensitivity.eq(class.as_bytes(), name.0.as_bytes()))
        })
    }

    fn has_custom_state(&self, _name: &CssString) -> bool {
        false
    }

    fn imported_part(&self, _name: &CssString) -> Option<CssString> {
        None
    }

    fn is_part(&self, _name: &CssString) -> bool {
        false
    }

    fn is_empty(&self) -> bool {
        self.doc
            .children(self.id)
            .all(|child| match &self.doc.node(child).kind {
                NodeKind::Element(_) => false,
                NodeKind::Text(text) => text.is_empty(),
                _ => true,
            })
    }

    fn is_root(&self) -> bool {
        self.doc.document_element() == Some(self.id)
    }

    fn add_element_unique_hashes(&self, _filter: &mut BloomFilter) -> bool {
        false
    }
}

/// The namespace of HTML elements.
const HTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";

#[cfg(test)]
mod tests {
    use super::*;
    use presswork_dom::parse_html;

    fn matching(html: &str, selector: &str) -> Vec<String> {
        let doc = parse_html(html.as_bytes());
        let input = selector;
        let list = parse_selector_list(&mut Parser::new(input)).expect("valid selector");
        let mut caches = SelectorCaches::default();
        doc.descendants(doc.root())
            .filter_map(|id| ElementRef::new(&doc, id))
            .filter(|element| {
                list.slice()
                    .iter()
                    .any(|selector| element.matches(selector, false, &mut caches))
            })
            .map(|element| {
                let data = element.data();
                match data.attr("id") {
                    Some(id) => format!("{}#{id}", data.name),
                    None => data.name.clone(),
                }
            })
            .collect()
    }

    const DOC: &str = r#"<!DOCTYPE html>
        <div id=a class="box Big"><p id=p1>one</p><p id=p2 class=note>two</p>
        <a id=l href="x">link</a><a id=n>no link</a></div>
        <table><tr id=r1><td id=c1></td><td id=c2>x</td></tr><tr id=r2><td id=c3></td></tr></table>"#;

    #[test]
    fn type_class_and_id_selectors() {
        assert_eq!(matching(DOC, "p"), ["p#p1", "p#p2"]);
        assert_eq!(matching(DOC, "P.note"), ["p#p2"]);
        assert_eq!(matching(DOC, ".box"), ["div#a"]);
        assert_eq!(matching(DOC, ".big"), [] as [&str; 0]);
        assert_eq!(matching(DOC, "#p1, #c2"), ["p#p1", "td#c2"]);
    }

    #[test]
    fn class_matching_is_case_insensitive_in_quirks_mode() {
        // No doctype puts the document in quirks mode.
        assert_eq!(matching("<p class=Big>", ".big"), ["p"]);
    }

    #[test]
    fn combinators() {
        assert_eq!(matching(DOC, "div > p"), ["p#p1", "p#p2"]);
        assert_eq!(matching(DOC, "table td"), ["td#c1", "td#c2", "td#c3"]);
        assert_eq!(matching(DOC, "p + p"), ["p#p2"]);
        assert_eq!(matching(DOC, "p ~ a"), ["a#l", "a#n"]);
        assert_eq!(matching(DOC, "body > p"), [] as [&str; 0]);
    }

    #[test]
    fn structural_pseudo_classes() {
        assert_eq!(matching(DOC, "td:first-child"), ["td#c1", "td#c3"]);
        assert_eq!(matching(DOC, "td:last-child"), ["td#c2", "td#c3"]);
        assert_eq!(matching(DOC, "tr:nth-child(even)"), ["tr#r2"]);
        assert_eq!(matching(DOC, "td:empty"), ["td#c1", "td#c3"]);
        assert_eq!(matching(DOC, "p:not(.note)"), ["p#p1"]);
        assert_eq!(
            matching(DOC, ":is(p, td):first-child"),
            ["p#p1", "td#c1", "td#c3"]
        );
        assert_eq!(matching(DOC, ":root"), ["html"]);
    }

    #[test]
    fn attribute_selectors_and_links() {
        assert_eq!(matching(DOC, "[href]"), ["a#l"]);
        assert_eq!(matching(DOC, "[class~=box]"), ["div#a"]);
        assert_eq!(matching(DOC, "[id^=c]"), ["td#c1", "td#c2", "td#c3"]);
        assert_eq!(matching(DOC, "a:link"), ["a#l"]);
        assert_eq!(matching(DOC, "a:hover"), [] as [&str; 0]);
    }

    #[test]
    fn invalid_selectors_are_rejected() {
        for selector in ["p >", "p:unknown-thing", "p, , a", "::nonsense"] {
            let input = selector;
            assert!(
                parse_selector_list(&mut Parser::new(input)).is_err(),
                "{selector}"
            );
        }
    }
}
