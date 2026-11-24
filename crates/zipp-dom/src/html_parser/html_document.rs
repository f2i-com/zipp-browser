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
        if std::mem::take(&mut self.layout_stale) {
            let now = self.inner.animation_time;
            self.inner.resolve(now);
        }
        let mut wants_more = false;
        if let Some(handler) = self.event_handler.as_mut() {
            wants_more |= handler.animation_frame(&mut self.inner);
            self.apply_pending_rerender();
        }
        let sub_ids: Vec<usize> = self.inner.sub_document_nodes.iter().copied().collect();
        for id in sub_ids {
            if let Some(sub_doc) = self.inner.get_node_mut(id).and_then(|n| n.subdoc_mut()) {
                wants_more |= sub_doc.animation_frame();
            }
        }
        wants_more
    }

    fn run_tasks(&mut self) -> Option<std::time::Instant> {
        if std::mem::take(&mut self.layout_stale) {
            let now = self.inner.animation_time;
            self.inner.resolve(now);
        }
        let mut next = None;
        if let Some(handler) = self.event_handler.as_mut() {
            next = handler.run_tasks(&mut self.inner);
            self.apply_pending_rerender();
        }
        let sub_ids: Vec<usize> = self.inner.sub_document_nodes.iter().copied().collect();
        for id in sub_ids {
            if let Some(sub_doc) = self.inner.get_node_mut(id).and_then(|n| n.subdoc_mut())
                && let Some(due) = sub_doc.run_tasks()
            {
                next = Some(next.map_or(due, |n| n.min(due)));
            }
        }
        next
    }

    fn poll(&mut self, _task_context: Option<std::task::Context>) -> bool {
        // Only signal a redraw when resource messages actually arrived.
        // Returning `pending_images.len() > 0` previously caused the shell to
        // re-layout and repaint on every single window event (including every
        // WM_MOVE drag tick) whenever any image remained in-flight or failed.
        let mut changed = self.inner.handle_messages();
        let sub_ids: Vec<usize> = self.inner.sub_document_nodes.iter().copied().collect();
        for id in sub_ids {
            if let Some(sub_doc) = self.inner.get_node_mut(id).and_then(|n| n.subdoc_mut()) {
                changed |= sub_doc.poll(None);
            }
        }
        changed
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
                // A text field's value that its editor already shows (what
                // was just typed into it) is only noted: setting it would put
                // the editor's text again, and the caret at its end.
                let typed = doc.nodes[node_id]
                    .element_data()
                    .and_then(|e| e.text_input_data())
                    .map(|t| t.editor.raw_text().to_string());
                let mut noted = false;
                let attrs: Vec<(String, String)> = attrs
                    .into_iter()
                    .filter(|(name, value)| {
                        if name != "value" || typed.as_deref() != Some(value.as_str()) {
                            return true;
                        }
                        if let Some(el) = doc.nodes[node_id].element_data_mut() {
                            el.attrs.set(qual(name), value);
                        }
                        noted = true;
                        false
                    })
                    .collect();
                let mut mutator = crate::DocumentMutator::new(doc);
                for (name, _) in &current {
                    if !(attrs.iter().any(|(n, _)| n == name) || (noted && name == "value")) {
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
    //
    // Only a field inside what is replaced: another one is not rebuilt, and
    // has nothing to keep. Taken from every focused field, an editor ahead of
    // its `value` attribute (a key typed in the same batch as this change)
    // was not given back, and the desktop's taskbar refreshing on its timer
    // sent the terminal's caret to the start of the line in mid-word.
    let is_replaced = |doc: &BaseDocument, id: usize| {
        let mut at = doc.get_node(id).and_then(|n| n.parent);
        while let Some(parent) = at {
            if parent == node_id {
                return true;
            }
            at = doc.get_node(parent).and_then(|n| n.parent);
        }
        false
    };
    let focused_editor = doc.focus_node_id.filter(|&id| is_replaced(doc, id)).and_then(|id| {
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
    // Preserve mounted iframe sub-documents inside the replaced subtree whose
    // `src`/`srcdoc` did not change so embedded interactive apps keep their
    // live state across parent React/JS re-renders and partial DOM patches.
    let mut saved_subdocs: Vec<(String, Box<dyn Document>)> = Vec::new();
    let replaced_subs: Vec<usize> = doc
        .sub_document_nodes
        .iter()
        .copied()
        .filter(|&sid| is_replaced(doc, sid))
        .collect();
    for sid in replaced_subs {
        doc.sub_document_nodes.remove(&sid);
        let key = doc.get_node(sid).and_then(|n| {
            let el = n.element_data()?;
            el.attr(markup5ever::local_name!("srcdoc"))
                .or_else(|| el.attr(markup5ever::local_name!("src")))
                .map(str::to_string)
        });
        if let Some(key) = key
            && let Some(el) = doc.get_node_mut(sid).and_then(|n| n.element_data_mut())
            && let crate::node::SpecialElementData::SubDocument(doc_box) =
                std::mem::replace(&mut el.special_data, crate::node::SpecialElementData::None)
        {
            saved_subdocs.push((key, doc_box));
        }
    }
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
    if !saved_subdocs.is_empty()
        && let Ok(iframes) = doc.query_selector_all("iframe")
    {
        for iframe_id in iframes {
            if !is_replaced(doc, iframe_id) {
                continue;
            }
            let key = doc.get_node(iframe_id).and_then(|n| {
                let el = n.element_data()?;
                el.attr(markup5ever::local_name!("srcdoc"))
                    .or_else(|| el.attr(markup5ever::local_name!("src")))
                    .map(str::to_string)
            });
            if let Some(key) = key
                && let Some(pos) = saved_subdocs.iter().position(|(k, _)| k == &key)
            {
                let (_, doc_box) = saved_subdocs.remove(pos);
                doc.set_sub_document(iframe_id, doc_box);
            }
        }
    }
    if !scrolled.is_empty() {
        let mut by_flid: std::collections::HashMap<&str, crate::Point<f64>> =
            scrolled.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        for (_, node) in doc.nodes.iter_mut() {
            if let Some(flid) = node.attr(flid_attr.clone())
                && let Some(offset) = by_flid.remove(flid)
            {
                node.scroll_offset = offset;
                if by_flid.is_empty() {
                    break;
                }
            }
        }
    }
    // Note: `drop_node_ignoring_parent` already removes dropped nodes from
    // `animations.sets` and clears hover/focus/active/mousedown only when
    // those specific IDs are freed — preserving animations and drags on
    // untouched sibling subtrees.
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
