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
                if let Some(body_id) = body_id {
                    let mut mutator = crate::DocumentMutator::new(&mut self.inner);
                    mutator.set_inner_html(body_id, &html);
                    drop(mutator);
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
