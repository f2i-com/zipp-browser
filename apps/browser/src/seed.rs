//! The page's own body, for the DOM shim to start from.
//!
//! Page script used to see an empty document -- a lone `<div id="root">` --
//! so a server-rendered page's scripts found none of its elements, and a
//! `<canvas>` written in the HTML could not be drawn on. This parses the page
//! with html5ever and gives the shim its body as JSON (`__zippHostCall("seed")`),
//! `[bodyAttributes, children, headChildren, htmlAttributes]`, where an element is
//! `[tag, [name, value, ...], [children], ns?]` (`ns` "s" for SVG, "m" for
//! MathML) and text is a string. The head is there for scripts that read it
//! -- its `<meta>`s, a JSON `<script>` of settings (GitHub's `#client-env`) --
//! and is never rendered from the shim.
//!
//! Left out: comments; `<noscript>`, since scripts run; and what is inside
//! `#root`, which a client-rendered app fills itself, starting from empty as it
//! always has here.

use std::borrow::Cow;
use std::cell::{Ref, RefCell};
use std::fmt::Write;

use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeBuilderOpts, TreeSink};
use html5ever::{Attribute, ParseOpts, QualName, local_name, ns};

#[derive(Clone)]
enum Kind {
    Document,
    Element { name: QualName, attrs: Vec<Attribute>, template: Option<usize> },
    Text(StrTendril),
    /// A comment, processing instruction, or a template's contents.
    Other,
}

struct Node {
    kind: Kind,
    parent: Option<usize>,
    children: Vec<usize>,
}

/// An arena of nodes; a handle is an index into it, the document is 0.
struct Sink {
    nodes: RefCell<Vec<Node>>,
}

impl Sink {
    fn push(&self, kind: Kind) -> usize {
        let mut nodes = self.nodes.borrow_mut();
        nodes.push(Node { kind, parent: None, children: Vec::new() });
        nodes.len() - 1
    }
}

fn detach(nodes: &mut [Node], id: usize) {
    if let Some(parent) = nodes[id].parent.take() {
        nodes[parent].children.retain(|&c| c != id);
    }
}

fn clone_subtree(nodes: &mut Vec<Node>, id: usize) -> usize {
    let kind = nodes[id].kind.clone();
    nodes.push(Node { kind, parent: None, children: Vec::new() });
    let copy = nodes.len() - 1;
    for child in nodes[id].children.clone() {
        let c = clone_subtree(nodes, child);
        nodes[c].parent = Some(copy);
        nodes[copy].children.push(c);
    }
    copy
}

impl TreeSink for Sink {
    type Handle = usize;
    type Output = Self;
    type ElemName<'a>
        = Ref<'a, QualName>
    where
        Self: 'a;

    fn finish(self) -> Self {
        self
    }

    fn parse_error(&self, _msg: Cow<'static, str>) {}

    fn get_document(&self) -> usize {
        0
    }

    fn elem_name<'a>(&'a self, target: &'a usize) -> Ref<'a, QualName> {
        Ref::map(self.nodes.borrow(), |nodes| match &nodes[*target].kind {
            Kind::Element { name, .. } => name,
            _ => panic!("elem_name called on a node which is not an element"),
        })
    }

    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, flags: ElementFlags) -> usize {
        let template = flags.template.then(|| self.push(Kind::Other));
        self.push(Kind::Element { name, attrs, template })
    }

    fn create_comment(&self, _text: StrTendril) -> usize {
        self.push(Kind::Other)
    }

    fn create_pi(&self, _target: StrTendril, _data: StrTendril) -> usize {
        self.push(Kind::Other)
    }

    fn append(&self, parent: &usize, child: NodeOrText<usize>) {
        let mut nodes = self.nodes.borrow_mut();
        match child {
            NodeOrText::AppendNode(c) => {
                detach(&mut nodes, c);
                nodes[c].parent = Some(*parent);
                nodes[*parent].children.push(c);
            }
            NodeOrText::AppendText(text) => {
                if let Some(&last) = nodes[*parent].children.last()
                    && let Kind::Text(t) = &mut nodes[last].kind
                {
                    t.push_tendril(&text);
                    return;
                }
                nodes.push(Node { kind: Kind::Text(text), parent: Some(*parent), children: Vec::new() });
                let id = nodes.len() - 1;
                nodes[*parent].children.push(id);
            }
        }
    }

    fn append_based_on_parent_node(&self, element: &usize, prev_element: &usize, child: NodeOrText<usize>) {
        let has_parent = self.nodes.borrow()[*element].parent.is_some();
        if has_parent {
            self.append_before_sibling(element, child);
        } else {
            self.append(prev_element, child);
        }
    }

    fn append_doctype_to_document(&self, _name: StrTendril, _public_id: StrTendril, _system_id: StrTendril) {}

    fn get_template_contents(&self, target: &usize) -> usize {
        match &self.nodes.borrow()[*target].kind {
            Kind::Element { template: Some(contents), .. } => *contents,
            _ => panic!("get_template_contents called on a node which is not a template"),
        }
    }

    fn same_node(&self, x: &usize, y: &usize) -> bool {
        x == y
    }

    fn set_quirks_mode(&self, _mode: QuirksMode) {}

    fn append_before_sibling(&self, sibling: &usize, new_node: NodeOrText<usize>) {
        let mut nodes = self.nodes.borrow_mut();
        let Some(parent) = nodes[*sibling].parent else {
            return;
        };
        match new_node {
            NodeOrText::AppendNode(c) => {
                detach(&mut nodes, c);
                let at = nodes[parent].children.iter().position(|x| x == sibling).unwrap_or(0);
                nodes[c].parent = Some(parent);
                nodes[parent].children.insert(at, c);
            }
            NodeOrText::AppendText(text) => {
                let at = nodes[parent].children.iter().position(|x| x == sibling).unwrap_or(0);
                if at > 0 {
                    let prev = nodes[parent].children[at - 1];
                    if let Kind::Text(t) = &mut nodes[prev].kind {
                        t.push_tendril(&text);
                        return;
                    }
                }
                nodes.push(Node { kind: Kind::Text(text), parent: Some(parent), children: Vec::new() });
                let id = nodes.len() - 1;
                nodes[parent].children.insert(at, id);
            }
        }
    }

    fn add_attrs_if_missing(&self, target: &usize, attrs: Vec<Attribute>) {
        if let Kind::Element { attrs: existing, .. } = &mut self.nodes.borrow_mut()[*target].kind {
            for attr in attrs {
                if !existing.iter().any(|e| e.name == attr.name) {
                    existing.push(attr);
                }
            }
        }
    }

    fn remove_from_parent(&self, target: &usize) {
        detach(&mut self.nodes.borrow_mut(), *target);
    }

    fn reparent_children(&self, node: &usize, new_parent: &usize) {
        let mut nodes = self.nodes.borrow_mut();
        let children = std::mem::take(&mut nodes[*node].children);
        for &c in &children {
            nodes[c].parent = Some(*new_parent);
        }
        nodes[*new_parent].children.extend(children);
    }

    fn clone_subtree(&self, node: &usize) -> usize {
        clone_subtree(&mut self.nodes.borrow_mut(), *node)
    }
}

/// The page's body as the shim's seed (see the module docs), or `None` when
/// the page has no body.
pub(crate) fn body_json(html: &str) -> Option<String> {
    parse_html_json_inner(html, false)
}

/// Full HTML5 document parse for `DOMParser.prototype.parseFromString(html, "text/html")`,
/// preserving `#root` children and `<noscript>` elements.
pub(crate) fn parse_html_json(html: &str) -> Option<String> {
    parse_html_json_inner(html, true)
}

fn parse_html_json_inner(html: &str, keep_all: bool) -> Option<String> {
    let sink = Sink { nodes: RefCell::new(vec![Node { kind: Kind::Document, parent: None, children: Vec::new() }]) };
    let opts = ParseOpts {
        tree_builder: TreeBuilderOpts { scripting_enabled: !keep_all, drop_doctype: true, ..Default::default() },
        ..Default::default()
    };
    let nodes = html5ever::parse_document(sink, opts).one(html).nodes.into_inner();
    let is = |id: usize, tag: &html5ever::LocalName| matches!(&nodes[id].kind, Kind::Element { name, .. } if name.ns == ns!(html) && name.local == *tag);
    let html_el = *nodes[0].children.iter().find(|&&c| is(c, &local_name!("html")))?;
    let body = *nodes[html_el].children.iter().find(|&&c| is(c, &local_name!("body")))?;
    let mut out = String::with_capacity(html.len() / 2);
    out.push('[');
    if let Kind::Element { attrs, .. } = &nodes[body].kind {
        write_attrs(&mut out, attrs);
    }
    out.push(',');
    write_children(&mut out, &nodes, body, keep_all);
    out.push(',');
    match nodes[html_el].children.iter().find(|&&c| is(c, &local_name!("head"))) {
        Some(&head) => write_children(&mut out, &nodes, head, keep_all),
        None => out.push_str("[]"),
    }
    out.push(',');
    if let Kind::Element { attrs, .. } = &nodes[html_el].kind {
        write_attrs(&mut out, attrs);
    }
    out.push(']');
    Some(out)
}

fn write_children(out: &mut String, nodes: &[Node], id: usize, keep_all: bool) {
    out.push('[');
    let mut first = true;
    for &child in &nodes[id].children {
        let start = out.len();
        if !first {
            out.push(',');
        }
        if write_node(out, nodes, child, keep_all) {
            first = false;
        } else {
            out.truncate(start);
        }
    }
    out.push(']');
}

/// One node, or false (and nothing usable written) when it is left out.
fn write_node(out: &mut String, nodes: &[Node], id: usize, keep_all: bool) -> bool {
    match &nodes[id].kind {
        Kind::Text(text) => {
            write_str(out, text);
            true
        }
        Kind::Element { name, attrs, template } => {
            if !keep_all && name.ns == ns!(html) && name.local == local_name!("noscript") {
                return false;
            }
            out.push('[');
            write_str(out, &name.local);
            out.push(',');
            write_attrs(out, attrs);
            out.push(',');
            let is_root = !keep_all && attrs.iter().any(|a| a.name.local == local_name!("id") && &*a.value == "root");
            if is_root {
                out.push_str("[]");
            } else {
                write_children(out, nodes, template.unwrap_or(id), keep_all);
            }
            if name.ns == ns!(svg) {
                out.push_str(",\"s\"");
            } else if name.ns == ns!(mathml) {
                out.push_str(",\"m\"");
            }
            out.push(']');
            true
        }
        Kind::Document | Kind::Other => false,
    }
}

fn write_attrs(out: &mut String, attrs: &[Attribute]) {
    out.push('[');
    for (i, attr) in attrs.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        match &attr.name.prefix {
            Some(prefix) => write_str(out, &format!("{prefix}:{}", attr.name.local)),
            None => write_str(out, &attr.name.local),
        }
        out.push(',');
        write_str(out, &attr.value);
    }
    out.push(']');
}

/// A JSON string, which is also a JS string literal: U+2028 and U+2029 are
/// escaped too.
fn write_str(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{2028}' || c == '\u{2029}' => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::body_json;

    #[test]
    fn seeds_the_body() {
        let json = body_json(
            "<!doctype html><html lang=en><head><meta name=release content=1></head><body class=\"x\"><h1 id=a>Hi &amp; \"you\"</h1><!-- c -->\
             <canvas width=640></canvas><noscript>no</noscript><div id=root><p>ssr</p></div>\
             <svg viewBox=\"0 0 1 1\"><path d=\"M0\"/></svg></body></html>",
        )
        .unwrap();
        assert_eq!(
            json,
            "[[\"class\",\"x\"],[[\"h1\",[\"id\",\"a\"],[\"Hi & \\\"you\\\"\"]],\
             [\"canvas\",[\"width\",\"640\"],[]],[\"div\",[\"id\",\"root\"],[]],\
             [\"svg\",[\"viewBox\",\"0 0 1 1\"],[[\"path\",[\"d\",\"M0\"],[],\"s\"]],\"s\"]],\
             [[\"meta\",[\"name\",\"release\",\"content\",\"1\"],[]]],[\"lang\",\"en\"]]"
        );
    }

    #[test]
    fn a_page_without_markup_still_has_a_body() {
        assert_eq!(body_json("").as_deref(), Some("[[],[],[],[]]"));
    }

    #[test]
    fn parse_html_json_preserves_root_children_and_noscript() {
        let json = super::parse_html_json(
            "<!doctype html><html><head><title>T</title></head><body><div id=root><p>ok</p></div><noscript>fb</noscript></body></html>",
        )
        .unwrap();
        assert!(json.contains("[\"div\",[\"id\",\"root\"],[[\"p\",[],[\"ok\"]]]]"), "{json}");
        assert!(json.contains("[\"noscript\",[],[\"fb\"]]"), "{json}");
        assert!(json.contains("[\"title\",[],[\"T\"]]"), "{json}");
    }
}
