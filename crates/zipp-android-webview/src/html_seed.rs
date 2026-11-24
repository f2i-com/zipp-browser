//! HTML5 parser producing the compact JSON tree format expected by
//! `zipp-browser`'s `dom_shim.js` (`DOMParser.prototype.parseFromString` and
//! document seeding).

use std::borrow::Cow;
use std::cell::{Ref, RefCell};
use std::fmt::Write;

use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeBuilderOpts, TreeSink};
use html5ever::{local_name, ns, Attribute, ParseOpts, QualName};

#[derive(Clone)]
enum Kind {
    Document,
    Element {
        name: QualName,
        attrs: Vec<Attribute>,
        template: Option<usize>,
    },
    Text(StrTendril),
    Other,
}

struct Node {
    kind: Kind,
    parent: Option<usize>,
    children: Vec<usize>,
}

struct Sink {
    nodes: RefCell<Vec<Node>>,
}

impl Sink {
    fn push(&self, kind: Kind) -> usize {
        let mut nodes = self.nodes.borrow_mut();
        nodes.push(Node {
            kind,
            parent: None,
            children: Vec::new(),
        });
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
    nodes.push(Node {
        kind,
        parent: None,
        children: Vec::new(),
    });
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
        self.push(Kind::Element {
            name,
            attrs,
            template,
        })
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
                if let Some(&last) = nodes[*parent].children.last() {
                    if let Kind::Text(t) = &mut nodes[last].kind {
                        t.push_tendril(&text);
                        return;
                    }
                }
                nodes.push(Node {
                    kind: Kind::Text(text),
                    parent: Some(*parent),
                    children: Vec::new(),
                });
                let id = nodes.len() - 1;
                nodes[*parent].children.push(id);
            }
        }
    }

    fn append_based_on_parent_node(
        &self,
        element: &usize,
        prev_element: &usize,
        child: NodeOrText<usize>,
    ) {
        let has_parent = self.nodes.borrow()[*element].parent.is_some();
        if has_parent {
            self.append_before_sibling(element, child);
        } else {
            self.append(prev_element, child);
        }
    }

    fn append_doctype_to_document(
        &self,
        _name: StrTendril,
        _public_id: StrTendril,
        _system_id: StrTendril,
    ) {
    }

    fn get_template_contents(&self, target: &usize) -> usize {
        match &self.nodes.borrow()[*target].kind {
            Kind::Element {
                template: Some(contents),
                ..
            } => *contents,
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
                let at = nodes[parent]
                    .children
                    .iter()
                    .position(|x| x == sibling)
                    .unwrap_or(0);
                nodes[c].parent = Some(parent);
                nodes[parent].children.insert(at, c);
            }
            NodeOrText::AppendText(text) => {
                let at = nodes[parent]
                    .children
                    .iter()
                    .position(|x| x == sibling)
                    .unwrap_or(0);
                if at > 0 {
                    let prev = nodes[parent].children[at - 1];
                    if let Kind::Text(t) = &mut nodes[prev].kind {
                        t.push_tendril(&text);
                        return;
                    }
                }
                nodes.push(Node {
                    kind: Kind::Text(text),
                    parent: Some(parent),
                    children: Vec::new(),
                });
                let id = nodes.len() - 1;
                nodes[parent].children.insert(at, id);
            }
        }
    }

    fn add_attrs_if_missing(&self, target: &usize, attrs: Vec<Attribute>) {
        if let Kind::Element {
            attrs: existing, ..
        } = &mut self.nodes.borrow_mut()[*target].kind
        {
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

/// Full HTML5 parse for `DOMParser.prototype.parseFromString(html, "text/html")`.
pub(crate) fn parse_html_json(html: &str) -> Option<String> {
    let sink = Sink {
        nodes: RefCell::new(vec![Node {
            kind: Kind::Document,
            parent: None,
            children: Vec::new(),
        }]),
    };
    let opts = ParseOpts {
        tree_builder: TreeBuilderOpts {
            scripting_enabled: false,
            drop_doctype: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let nodes = html5ever::parse_document(sink, opts)
        .one(html)
        .nodes
        .into_inner();
    let is = |id: usize, tag: &html5ever::LocalName| {
        matches!(&nodes[id].kind, Kind::Element { name, .. } if name.ns == ns!(html) && name.local == *tag)
    };
    let html_el = *nodes[0].children.iter().find(|&&c| is(c, &local_name!("html")))?;
    let body = *nodes[html_el]
        .children
        .iter()
        .find(|&&c| is(c, &local_name!("body")))?;
    let mut out = String::with_capacity(html.len() / 2 + 64);
    out.push('[');
    if let Kind::Element { attrs, .. } = &nodes[body].kind {
        write_attrs(&mut out, attrs);
    }
    out.push(',');
    write_children(&mut out, &nodes, body);
    out.push(',');
    match nodes[html_el]
        .children
        .iter()
        .find(|&&c| is(c, &local_name!("head")))
    {
        Some(&head) => write_children(&mut out, &nodes, head),
        None => out.push_str("[]"),
    }
    out.push(',');
    if let Kind::Element { attrs, .. } = &nodes[html_el].kind {
        write_attrs(&mut out, attrs);
    }
    out.push(']');
    Some(out)
}

fn write_children(out: &mut String, nodes: &[Node], id: usize) {
    out.push('[');
    let mut first = true;
    for &child in &nodes[id].children {
        let start = out.len();
        if !first {
            out.push(',');
        }
        if write_node(out, nodes, child) {
            first = false;
        } else {
            out.truncate(start);
        }
    }
    out.push(']');
}

fn write_node(out: &mut String, nodes: &[Node], id: usize) -> bool {
    match &nodes[id].kind {
        Kind::Text(text) => {
            write_str(out, text);
            true
        }
        Kind::Element {
            name,
            attrs,
            template,
        } => {
            out.push('[');
            write_str(out, &name.local);
            out.push(',');
            write_attrs(out, attrs);
            out.push(',');
            write_children(out, nodes, template.unwrap_or(id));
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
