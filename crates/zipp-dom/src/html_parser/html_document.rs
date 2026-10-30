use std::ops::{Deref, DerefMut};

use super::DocumentHtmlParser;

use crate::{
    BaseDocument, DEFAULT_CSS, DocGuard, DocGuardMut, Document, DocumentConfig,
    EventDriver, EventHandler, NoopEventHandler,
};
use crate::traits_src::events::UiEvent;

pub struct HtmlDocument {
    inner: BaseDocument,
    /// Optional JS event handler. When set, DOM events are dispatched through
    /// this handler instead of the default [`NoopEventHandler`].
    event_handler: Option<Box<dyn EventHandler>>,
    /// Names of the `<html>` attributes the page's JS last put on the real
    /// document element, so one the page later drops can be removed without
    /// touching attributes that came from the original markup (`lang`).
    js_root_attrs: Vec<String>,
    /// The body was replaced since the last layout. Until the next frame lays
    /// it out, its new nodes have no boxes, and a click there hit nothing.
    layout_stale: bool,
}

impl Deref for HtmlDocument {
    type Target = BaseDocument;
    fn deref(&self) -> &BaseDocument {
        &self.inner
    }
}
impl DerefMut for HtmlDocument {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}
impl From<HtmlDocument> for BaseDocument {
    fn from(doc: HtmlDocument) -> BaseDocument {
        doc.inner
    }
}
impl Document for HtmlDocument {
    fn inner(&self) -> DocGuard<'_> {
        DocGuard::Ref(&self.inner)
    }

    fn inner_mut(&mut self) -> DocGuardMut<'_> {
        DocGuardMut::Ref(&mut self.inner)
    }

    fn handle_ui_event(&mut self, event: UiEvent) {
        // Lay out a body the page just replaced before finding what the
        // pointer is over. A page that redraws every frame (a game) replaces
        // it between frames, and a button released in that gap was missed:
        // the click went to the page itself, which closed the menu it opened.
        // And find again what the pointer is over: the re-render forgot it,
        // and the next notches of a wheel turned over a scrolling box went to
        // the page instead.
        if std::mem::take(&mut self.layout_stale) {
            let now = self.inner.animation_time;
            self.inner.resolve(now);
            if let Some((x, y)) = self.inner.last_pointer {
                self.inner.set_hover_to(x, y);
            }
        }
        if let Some(ref mut handler) = self.event_handler {
            let handler_ref: &mut dyn EventHandler = &mut **handler;
            let mut driver = EventDriver::new(&mut self.inner, handler_ref);
            driver.handle_ui_event(event);
        } else {
            let mut driver = EventDriver::new(&mut self.inner, NoopEventHandler);
            driver.handle_ui_event(event);
        }

        self.apply_pending_rerender();
    }

    fn animation_frame(&mut self) -> bool {
        let Some(handler) = self.event_handler.as_mut() else {
            return false;
        };
        let wants_more = handler.animation_frame(&mut self.inner);
        self.apply_pending_rerender();
        wants_more
    }

    fn run_tasks(&mut self) -> Option<std::time::Instant> {
        let handler = self.event_handler.as_mut()?;
        let next = handler.run_tasks(&mut self.inner);
        self.apply_pending_rerender();
        next
    }

    fn poll(&mut self, _task_context: Option<std::task::Context>) -> bool {
        let pending_before = self.inner.pending_images.len();
        self.inner.handle_messages();
        let pending_after = self.inner.pending_images.len();
        // Request redraw whenever any image finishes loading
        if pending_before != pending_after {
            self.inner.shell_provider.request_redraw();
        }
        pending_after > 0
    }
}

impl HtmlDocument {
    /// Apply a re-render the event handler produced: the page's script changed
    /// the DOM (in an event handler, or an animation frame).
    fn apply_pending_rerender(&mut self) {
        // Check if the event handler produced new HTML (e.g. React state update)
        if let Some(ref mut handler) = self.event_handler {
            if let Some(html) = handler.take_pending_rerender() {
                eprintln!("[RERENDER] Updating body ({} bytes)", html.len());
                // Update only the body content, preserving <head> stylesheets.
                // Find the <body> element in the current document.
                let body_id = {
                    let root = self.inner.root_element();
                    let mut body = None;
                    if let Some(children) = root.layout_children.borrow().as_ref() {
                        for &child_id in children {
                            if let Some(el) = self.inner.get_node(child_id)
                                .and_then(|n| n.element_data())
                            {
                                if el.name.local.as_ref() == "body" {
                                    body = Some(child_id);
                                    break;
                                }
                            }
                        }
                    }
                    body
                };
                // The page's <html> attributes (its theme) must track the
                // re-render too. A body-only swap left whatever the initial load
                // put there in place forever, so dark mode could be turned on and
                // never off — the `.dark` on the document element outlived the
                // state that produced it. A theme kept in an attribute
                // (`data-theme`) could not change at all.
                if let Some(attrs) = handler.document_element_attributes() {
                    let root_id = self.inner.root_element().id;
                    let qual = |name: &str| {
                        markup5ever::QualName::new(None, markup5ever::ns!(), markup5ever::LocalName::from(name))
                    };
                    let mut mutator = crate::DocumentMutator::new(&mut self.inner);
                    for stale in &self.js_root_attrs {
                        if !attrs.iter().any(|(name, _)| name == stale) {
                            mutator.clear_attribute(root_id, qual(stale));
                        }
                    }
                    for (name, value) in &attrs {
                        mutator.set_attribute(root_id, qual(name), value);
                    }
                    drop(mutator);
                    self.js_root_attrs = attrs.into_iter().map(|(name, _)| name).collect();
                }
                if let Some(body_id) = body_id {
                    replace_children(&mut self.inner, body_id, &html);
                    // Every node id the handler cached is stale now — the body's
                    // whole subtree was dropped and re-parsed. Let it rebuild
                    // before the next event arrives, or only the FIRST
                    // interaction on a page ever works.
                    handler.document_rerendered(&mut self.inner);
                    self.inner.shell_provider.request_redraw();
                    self.layout_stale = true;
                }
            }
            // Changes made since (the handler drops those made before a
            // body it gives), applied after it.
            let patches = handler.take_pending_patches();
            if patches.is_empty() {
                return;
            }
            if apply_patches(&mut self.inner, patches) {
                // Nodes were re-made: the ids the handler cached are stale.
                handler.document_rerendered(&mut self.inner);
            }
            self.inner.shell_provider.request_redraw();
            self.layout_stale = true;
        }
    }
}

/// Apply a handler's patches in place; whether any element's children were
/// re-made (and with them node ids).
fn apply_patches(doc: &mut BaseDocument, patches: Vec<crate::DomPatch>) -> bool {
    let flid_attr = markup5ever::LocalName::from("data-flid");
    let mut nodes: std::collections::HashMap<u32, usize> = doc
        .nodes
        .iter()
        .filter_map(|(id, n)| Some((n.attr(flid_attr.clone())?.parse().ok()?, id)))
        .collect();
    let qual = |name: &str| markup5ever::QualName::new(None, markup5ever::ns!(), markup5ever::LocalName::from(name));
    let mut remade = false;
    for patch in patches {
        match patch {
            crate::DomPatch::Attributes { flid, attrs } => {
                let Some(&node_id) = nodes.get(&flid) else {
                    continue;
                };
                let Some(current) = doc.nodes.get(node_id).and_then(|n| n.attrs()) else {
                    continue;
                };
                let current: Vec<(String, String)> =
                    current.iter().map(|a| (a.name.local.to_string(), a.value.clone())).collect();
                let mut mutator = crate::DocumentMutator::new(doc);
                for (name, _) in &current {
                    if !attrs.iter().any(|(n, _)| n == name) {
                        mutator.clear_attribute(node_id, qual(name));
                    }
                }
                for (name, value) in &attrs {
                    if !current.iter().any(|(n, v)| n == name && v == value) {
                        mutator.set_attribute(node_id, qual(name), value);
                    }
                }
            }
            crate::DomPatch::Children { flid, html } => {
                let Some(&node_id) = nodes.get(&flid) else {
                    continue;
                };
                replace_children(doc, node_id, &html);
                remade = true;
                // The ids of the subtree's nodes changed.
                nodes = doc
                    .nodes
                    .iter()
                    .filter_map(|(id, n)| Some((n.attr(flid_attr.clone())?.parse().ok()?, id)))
                    .collect();
            }
        }
    }
    remade
}

/// Replace an element's children with those of `html`, keeping what the
/// user had: the focused element (found again by its `data-flid` or id), a
/// text field's caret and selection, and how far each element was scrolled.
fn replace_children(doc: &mut BaseDocument, node_id: usize, html: &str) {
    // What has focus, by the script's number for it (or its
    // id, for the browser's own console line), to give focus
    // back once its node is rebuilt: a page that re-renders
    // every frame would otherwise lose it at once (a game's
    // canvas, reading the arrow keys).
    let focused = doc
        .focus_node_id
        .and_then(|id| doc.get_node(id))
        .and_then(|n| {
            let attr = |name: &str| n.attr(markup5ever::LocalName::from(name)).map(str::to_string);
            attr("data-flid")
                .map(|flid| format!("[data-flid=\"{flid}\"]"))
                .or_else(|| attr("id").map(|id| format!("#{id}")))
        });
    // And how far each scrolled element was scrolled -- a
    // console following its output, an editor -- or every
    // re-render sent them all back to the top.
    // A focused text field keeps its editor -- the caret, the
    // selection -- when the re-render gives it the same text:
    // typing re-renders the page, and the caret went back to
    // the start with every key.
    let focused_editor = doc.focus_node_id.and_then(|id| {
        let el = doc.nodes.get_mut(id)?.element_data_mut()?;
        el.text_input_data()?;
        match std::mem::take(&mut el.special_data) {
            crate::node::SpecialElementData::TextInput(editor) => Some(editor),
            other => {
                el.special_data = other;
                None
            }
        }
    });
    let flid_attr = markup5ever::LocalName::from("data-flid");
    let scrolled: Vec<(String, crate::Point<f64>)> = doc
        .nodes
        .iter()
        .filter(|(_, n)| n.scroll_offset.x != 0.0 || n.scroll_offset.y != 0.0)
        .filter_map(|(_, n)| Some((n.attr(flid_attr.clone())?.to_string(), n.scroll_offset)))
        .collect();
    let mut mutator = crate::DocumentMutator::new(doc);
    mutator.set_inner_html(node_id, html);
    drop(mutator);
    for (flid, offset) in scrolled {
        if let Ok(Some(node)) = doc.query_selector(&format!("[data-flid=\"{flid}\"]")) {
            doc.nodes[node].scroll_offset = offset;
        }
    }
    // Animation state is keyed by node id and those ids are
    // recycled by the re-parse, so stale entries would attach
    // to whichever new element inherits the id.
    doc.clear_animation_state();
    // Hover/focus/mousedown ids point into the subtree that was
    // just freed; the next mouse move would index a dead id.
    doc.clear_interaction_state();
    if let Some(selector) = focused
        && let Ok(Some(node)) = doc.query_selector(&selector)
    {
        doc.set_focus_to(node);
        // (Its own editor is made at layout, from its text.)
        if let Some(editor) = focused_editor
            && doc.text_field_initial_text(node) == editor.editor.raw_text()
            && let Some(el) = doc.nodes[node].element_data_mut()
        {
            el.special_data = crate::node::SpecialElementData::TextInput(editor);
        }
    }
}

impl HtmlDocument {
    /// Parse HTML (or XHTML) into an [`HtmlDocument`]
    pub fn from_html(html: &str, mut config: DocumentConfig) -> Self {
        if let Some(ss) = &mut config.ua_stylesheets {
            if !ss.iter().any(|s| s == DEFAULT_CSS) {
                ss.push(String::from(DEFAULT_CSS));
            }
        }
        let mut doc = BaseDocument::new(config);
        let mut mutr = doc.mutate();
        DocumentHtmlParser::parse_into_mutator(&mut mutr, html);
        drop(mutr);
        HtmlDocument {
            inner: doc,
            event_handler: None,
            js_root_attrs: Vec::new(),
            layout_stale: false,
        }
    }

    /// Install a JS event handler that will be consulted for every DOM event.
    ///
    /// When set, [`handle_ui_event`](Document::handle_ui_event) dispatches
    /// through this handler instead of the default [`NoopEventHandler`].
    pub fn set_event_handler(&mut self, handler: Box<dyn EventHandler>) {
        self.event_handler = Some(handler);
    }

    /// Convert the [`HtmlDocument`] into it's inner [`BaseDocument`]
    pub fn into_inner(self) -> BaseDocument {
        self.into()
    }
}
