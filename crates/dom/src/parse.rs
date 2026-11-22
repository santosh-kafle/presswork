//! HTML parsing with `html5ever` into the arena [`Document`].

use std::borrow::Cow;
use std::cell::RefCell;

use html5ever::interface::{ElemName, ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::{Attribute as HtmlAttribute, LocalName, Namespace, QualName};

use crate::{Attribute, Document, ElementData, NodeId, NodeKind};

/// Parses an HTML document, following the HTML5 parsing algorithm.
///
/// Invalid UTF-8 is replaced with U+FFFD. Character encodings other than
/// UTF-8 are not detected yet.
pub fn parse_html(html: &[u8]) -> Document {
    let html = html.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(html);
    let text = String::from_utf8_lossy(html);
    html5ever::parse_document(Sink::default(), Default::default()).one(text.as_ref())
}

#[derive(Default)]
struct Sink {
    doc: RefCell<Document>,
}

/// An element name, as html5ever asks for it.
#[derive(Debug)]
struct Name {
    ns: Namespace,
    local: LocalName,
}

impl ElemName for Name {
    fn ns(&self) -> &Namespace {
        &self.ns
    }

    fn local_name(&self) -> &LocalName {
        &self.local
    }
}

fn convert_attributes(attrs: Vec<HtmlAttribute>) -> Vec<Attribute> {
    attrs
        .into_iter()
        .map(|a| Attribute {
            name: match &a.name.prefix {
                Some(prefix) => format!("{prefix}:{}", a.name.local),
                None => a.name.local.to_string(),
            },
            value: a.value.to_string(),
        })
        .collect()
}

impl Sink {
    fn insert(&self, parent: NodeId, before: Option<NodeId>, child: NodeOrText<NodeId>) {
        let mut doc = self.doc.borrow_mut();
        let child = match child {
            NodeOrText::AppendNode(node) => node,
            NodeOrText::AppendText(text) => {
                // Merge with the adjacent text node, as html5ever requires.
                let previous = match before {
                    Some(sibling) => doc.previous_sibling(sibling),
                    None => doc.node(parent).children.last().copied(),
                };
                if let Some(previous) = previous
                    && let NodeKind::Text(existing) = &mut doc.node_mut(previous).kind
                {
                    existing.push_str(&text);
                    return;
                }
                doc.create(NodeKind::Text(text.to_string()))
            }
        };
        match before {
            Some(sibling) => doc.insert_before(sibling, child),
            None => {
                // Elements nested deeper than the limit become siblings
                // of the deepest allowed, as in Chrome.
                let mut parent = parent;
                if doc.element(child).is_some() {
                    while depth(&doc, parent) >= MAX_DEPTH
                        && let Some(up) = doc.node(parent).parent
                    {
                        parent = up;
                    }
                }
                doc.append_child(parent, child)
            }
        }
    }
}

/// How deep elements may nest: Chrome's limit. Layout recurses per level,
/// so this bounds its stack and time.
pub const MAX_DEPTH: usize = 512;

/// The number of ancestors of `node`, up to the limit.
fn depth(doc: &Document, node: NodeId) -> usize {
    let mut depth = 0;
    let mut current = doc.node(node).parent;
    while let Some(parent) = current {
        depth += 1;
        if depth > MAX_DEPTH {
            break;
        }
        current = doc.node(parent).parent;
    }
    depth
}

impl TreeSink for Sink {
    type Handle = NodeId;
    type Output = Document;
    type ElemName<'a> = Name;

    fn finish(self) -> Document {
        self.doc.into_inner()
    }

    fn parse_error(&self, _msg: Cow<'static, str>) {}

    fn get_document(&self) -> NodeId {
        NodeId::ROOT
    }

    fn elem_name<'a>(&'a self, target: &'a NodeId) -> Name {
        match &self.doc.borrow().node(*target).kind {
            NodeKind::Element(data) => Name {
                ns: Namespace::from(data.namespace.as_str()),
                local: LocalName::from(data.name.as_str()),
            },
            _ => panic!("elem_name called on a non-element node"),
        }
    }

    fn create_element(
        &self,
        name: QualName,
        attrs: Vec<HtmlAttribute>,
        flags: ElementFlags,
    ) -> NodeId {
        let mut doc = self.doc.borrow_mut();
        let id = doc.create(NodeKind::Element(ElementData {
            name: name.local.to_string(),
            namespace: name.ns.to_string(),
            attributes: convert_attributes(attrs),
        }));
        if flags.template {
            let contents = doc.create(NodeKind::Fragment);
            doc.template_contents.insert(id, contents);
        }
        id
    }

    fn create_comment(&self, text: StrTendril) -> NodeId {
        self.doc
            .borrow_mut()
            .create(NodeKind::Comment(text.to_string()))
    }

    fn create_pi(&self, _target: StrTendril, data: StrTendril) -> NodeId {
        self.doc
            .borrow_mut()
            .create(NodeKind::Comment(data.to_string()))
    }

    fn append(&self, parent: &NodeId, child: NodeOrText<NodeId>) {
        self.insert(*parent, None, child);
    }

    fn append_based_on_parent_node(
        &self,
        element: &NodeId,
        prev_element: &NodeId,
        child: NodeOrText<NodeId>,
    ) {
        let has_parent = self.doc.borrow().node(*element).parent.is_some();
        if has_parent {
            self.append_before_sibling(element, child);
        } else {
            self.append(prev_element, child);
        }
    }

    fn append_doctype_to_document(&self, _: StrTendril, _: StrTendril, _: StrTendril) {}

    fn get_template_contents(&self, target: &NodeId) -> NodeId {
        self.doc.borrow().template_contents[target]
    }

    fn same_node(&self, x: &NodeId, y: &NodeId) -> bool {
        x == y
    }

    fn set_quirks_mode(&self, mode: QuirksMode) {
        self.doc.borrow_mut().quirks_mode = mode != QuirksMode::NoQuirks;
    }

    fn append_before_sibling(&self, sibling: &NodeId, new_node: NodeOrText<NodeId>) {
        let parent = self
            .doc
            .borrow()
            .node(*sibling)
            .parent
            .expect("sibling has a parent");
        self.insert(parent, Some(*sibling), new_node);
    }

    fn add_attrs_if_missing(&self, target: &NodeId, attrs: Vec<HtmlAttribute>) {
        let mut doc = self.doc.borrow_mut();
        if let NodeKind::Element(data) = &mut doc.node_mut(*target).kind {
            for attr in convert_attributes(attrs) {
                if data.attr(&attr.name).is_none() {
                    data.attributes.push(attr);
                }
            }
        }
    }

    fn remove_from_parent(&self, target: &NodeId) {
        self.doc.borrow_mut().detach(*target);
    }

    fn reparent_children(&self, node: &NodeId, new_parent: &NodeId) {
        let mut doc = self.doc.borrow_mut();
        let children = std::mem::take(&mut doc.node_mut(*node).children);
        for child in children {
            doc.node_mut(child).parent = None;
            doc.append_child(*new_parent, child);
        }
    }
}

/// The HTML namespace URL.
#[cfg(test)]
pub(crate) fn html_namespace() -> String {
    html5ever::ns!(html).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(doc: &Document, id: NodeId) -> Vec<String> {
        doc.children(id)
            .map(|c| match &doc.node(c).kind {
                NodeKind::Element(e) => e.name.clone(),
                NodeKind::Text(t) => format!("#{t}"),
                NodeKind::Comment(_) => "<!---->".into(),
                other => format!("{other:?}"),
            })
            .collect()
    }

    #[test]
    fn deep_nesting_stops_at_the_limit() {
        let html = format!("{}x{}", "<div>".repeat(2000), "</div>".repeat(2000));
        let doc = crate::parse_html(html.as_bytes());
        let divs: Vec<NodeId> = doc
            .descendants(doc.root())
            .filter(|&n| doc.element(n).is_some_and(|e| e.name == "div"))
            .collect();
        assert_eq!(divs.len(), 2000, "every element is kept");
        let deepest = divs.iter().map(|&n| depth(&doc, n)).max().unwrap();
        assert_eq!(deepest, MAX_DEPTH);
        assert!(doc.text_content(doc.root()).contains('x'));
    }

    #[test]
    fn builds_implied_structure() {
        let doc = parse_html(b"<p>Hello <b>world</b>");
        let html = doc.document_element().unwrap();
        assert_eq!(names(&doc, html), ["head", "body"]);
        let body = doc.find_element("body").unwrap();
        let p = doc.children(body).next().unwrap();
        assert_eq!(names(&doc, p), ["#Hello ", "b"]);
    }

    #[test]
    fn merges_adjacent_text_and_keeps_attributes() {
        let doc = parse_html(b"<div class=a id=x>a&amp;b</div>");
        let div = doc.find_element("div").unwrap();
        let NodeKind::Element(data) = &doc.node(div).kind else {
            panic!()
        };
        assert_eq!(data.attr("class"), Some("a"));
        assert_eq!(data.attr("id"), Some("x"));
        assert_eq!(data.namespace, html_namespace());
        assert_eq!(names(&doc, div), ["#a&b"]);
    }

    #[test]
    fn fosters_misnested_table_content() {
        // Text directly inside <table> is moved in front of the table.
        let doc = parse_html(b"<table>oops<tr><td>1</td></tr></table>");
        let body = doc.find_element("body").unwrap();
        assert_eq!(names(&doc, body), ["#oops", "table"]);
    }

    #[test]
    fn reads_title_and_handles_utf8() {
        let doc = parse_html("\u{feff}<title> बिल  १ </title><p>नमस्ते".as_bytes());
        assert_eq!(doc.title().as_deref(), Some("बिल १"));
    }

    #[test]
    fn template_contents_are_not_children() {
        let doc = parse_html(b"<template><p>hidden</p></template>");
        let template = doc.find_element("template").unwrap();
        assert_eq!(doc.children(template).count(), 0);
        assert!(doc.find_element("p").is_none());
    }
}
