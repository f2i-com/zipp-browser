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
        if let Some(ref mut handler) = self.event_handler {
            let handler_ref: &mut dyn EventHandler = &mut **handler;
            let mut driver = EventDriver::new(&mut self.inner, handler_ref);
            driver.handle_ui_event(event);
        } else {
            let mut driver = EventDriver::new(&mut self.inner, NoopEventHandler);
            driver.handle_ui_event(event);
        }

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
                    let mut mutator = crate::DocumentMutator::new(&mut self.inner);
                    mutator.set_inner_html(body_id, &html);
                    drop(mutator);
                    // Animation state is keyed by node id and those ids are
                    // recycled by the re-parse, so stale entries would attach
                    // to whichever new element inherits the id.
                    self.inner.clear_animation_state();
                    // Hover/focus/mousedown ids point into the subtree that was
                    // just freed; the next mouse move would index a dead id.
                    self.inner.clear_interaction_state();
                    // Every node id the handler cached is stale now — the body's
                    // whole subtree was dropped and re-parsed. Let it rebuild
                    // before the next event arrives, or only the FIRST
                    // interaction on a page ever works.
                    handler.document_rerendered(&self.inner);
                    self.inner.shell_provider.request_redraw();
                }
            }
        }
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
