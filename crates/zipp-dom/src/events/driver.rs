//! Event dispatch driver for the DOM.

use crate::{BaseDocument, Document};
use crate::traits_src::events::{
    PointerEvent, PointerId, DomEvent, DomEventData, EventState, UiEvent,
};
use std::collections::VecDeque;

pub trait EventHandler {
    fn handle_event(
        &mut self,
        chain: &[usize],
        event: &mut DomEvent,
        doc: &mut dyn Document,
        event_state: &mut EventState,
    );

    /// If the handler produced new HTML (e.g. React re-render after state update),
    /// return it here. The caller replaces the document body with it.
    fn take_pending_rerender(&mut self) -> Option<String> {
        None
    }

    /// Called after a `take_pending_rerender` result has been applied.
    ///
    /// Replacing the body destroys and recreates every node, so ANY node id the
    /// handler cached is now stale. A handler that maps DOM nodes to its own
    /// world (as the JS bridge does) has to rebuild here, or it will keep
    /// resolving events against nodes that no longer exist — the first
    /// interaction works and every one after it silently does nothing.
    fn document_rerendered(&mut self, _doc: &BaseDocument) {}

    /// The class the page has on its `<html>` element, if the handler can say.
    ///
    /// Themes live there — `:root.dark` and friends — and a body-only re-render
    /// cannot carry it, so the document has to be told separately.
    fn document_element_class(&mut self) -> Option<String> {
        None
    }
}

pub struct NoopEventHandler;
impl EventHandler for NoopEventHandler {
    fn handle_event(
        &mut self,
        _chain: &[usize],
        _event: &mut DomEvent,
        _doc: &mut dyn Document,
        _event_state: &mut EventState,
    ) {
        // Do nothing
    }
}

/// Blanket impl so a `Box<dyn EventHandler>` can be used wherever an
/// `EventHandler` is expected (e.g. stored on `HtmlDocument`).
impl EventHandler for Box<dyn EventHandler> {
    fn handle_event(
        &mut self,
        chain: &[usize],
        event: &mut DomEvent,
        doc: &mut dyn Document,
        event_state: &mut EventState,
    ) {
        (**self).handle_event(chain, event, doc, event_state)
    }
    fn take_pending_rerender(&mut self) -> Option<String> {
        (**self).take_pending_rerender()
    }
    fn document_element_class(&mut self) -> Option<String> {
        (**self).document_element_class()
    }

    fn document_rerendered(&mut self, doc: &BaseDocument) {
        (**self).document_rerendered(doc)
    }
}

/// Forwarding impl so an `&mut dyn EventHandler` reference can be passed by
/// value into [`EventDriver::new`] without moving the underlying handler.
impl EventHandler for &mut dyn EventHandler {
    fn take_pending_rerender(&mut self) -> Option<String> {
        (**self).take_pending_rerender()
    }
    fn document_element_class(&mut self) -> Option<String> {
        (**self).document_element_class()
    }

    fn document_rerendered(&mut self, doc: &BaseDocument) {
        (**self).document_rerendered(doc)
    }
    fn handle_event(
        &mut self,
        chain: &[usize],
        event: &mut DomEvent,
        doc: &mut dyn Document,
        event_state: &mut EventState,
    ) {
        (**self).handle_event(chain, event, doc, event_state)
    }
}

pub struct EventDriver<'doc, Handler: EventHandler> {
    doc: &'doc mut dyn Document,
    handler: Handler,
    queue: VecDeque<DomEvent>,
}

impl<'doc, Handler: EventHandler> EventDriver<'doc, Handler> {
    #[inline]
    pub fn new(doc: &'doc mut dyn Document, handler: Handler) -> Self {
        EventDriver {
            doc,
            handler,
            queue: VecDeque::with_capacity(4),
        }
    }

    pub fn handle_pointer_move(&mut self, event: &PointerEvent) -> Option<usize> {
        let mut doc = self.doc.inner_mut();

        let prev_hover_node_id = doc.hover_node_id;
        let changed = doc.set_hover_to(event.page_x(), event.page_y());
        let hover_node_id = doc.hover_node_id;

        drop(doc);

        if !changed {
            return prev_hover_node_id;
        }

        let doc = self.doc.inner();
        let mut old_chain = prev_hover_node_id
            .map(|id| doc.node_chain(id))
            .unwrap_or_default();
        let mut new_chain = hover_node_id
            .map(|id| doc.node_chain(id))
            .unwrap_or_default();
        old_chain.reverse();
        new_chain.reverse();

        // Find the difference in the node chain of the last hovered objected and the newest
        let old_len = old_chain.len();
        let new_len = new_chain.len();

        let first_difference_index = old_chain
            .iter()
            .zip(&new_chain)
            .position(|(old, new)| old != new)
            .unwrap_or_else(|| old_len.min(new_len));

        drop(doc);

        let is_mouse = event.is_mouse();

        if let Some(target) = prev_hover_node_id {
            self.handle_dom_event(DomEvent::new(
                target,
                DomEventData::PointerOut(event.clone()),
            ));
            if is_mouse {
                self.handle_dom_event(DomEvent::new(target, DomEventData::MouseOut(event.clone())));
            }

            // Send an mouseleave event to all old elements on the chain
            for node_id in old_chain
                .get(first_difference_index..)
                .unwrap_or(&[])
                .iter()
            {
                self.handle_dom_event(DomEvent::new(
                    *node_id,
                    DomEventData::PointerLeave(event.clone()),
                ));
                if is_mouse {
                    self.handle_dom_event(DomEvent::new(
                        *node_id,
                        DomEventData::MouseLeave(event.clone()),
                    ));
                }
            }
        }

        if let Some(target) = hover_node_id {
            self.handle_dom_event(DomEvent::new(
                target,
                DomEventData::PointerOver(event.clone()),
            ));

            if is_mouse {
                self.handle_dom_event(DomEvent::new(
                    target,
                    DomEventData::MouseOver(event.clone()),
                ));
            }

            // Send an mouseenter event to all new elements on the chain
            for node_id in new_chain
                .get(first_difference_index..)
                .unwrap_or(&[])
                .iter()
            {
                self.handle_dom_event(DomEvent::new(
                    *node_id,
                    DomEventData::PointerEnter(event.clone()),
                ));

                if is_mouse {
                    self.handle_dom_event(DomEvent::new(
                        *node_id,
                        DomEventData::MouseEnter(event.clone()),
                    ));
                }
            }
        }

        hover_node_id
    }

    pub fn handle_ui_event(&mut self, event: UiEvent) {
        let doc = self.doc.inner();

        let mut should_clear_hover = false;
        let mut hover_node_id = doc.hover_node_id;
        let focussed_node_id = doc.focus_node_id;
        drop(doc);

        // Update document input state (hover, focus, active, etc)
        match &event {
            UiEvent::PointerMove(event) => {
                hover_node_id = self.handle_pointer_move(event);
            }
            UiEvent::PointerDown(event) => {
                hover_node_id = self.handle_pointer_move(event);
                let mut doc = self.doc.inner_mut();
                doc.active_node();
                doc.set_mousedown_node_id(hover_node_id);
            }
            UiEvent::PointerUp(event) => {
                hover_node_id = self.handle_pointer_move(event);
                let mut doc = self.doc.inner_mut();
                doc.unactive_node();

                if event.is_primary && matches!(event.id, PointerId::Finger(_)) {
                    should_clear_hover = true;
                }
            }
            _ => {}
        };

        let target = match event {
            UiEvent::PointerMove(_) => hover_node_id,
            UiEvent::PointerUp(_) => hover_node_id,
            UiEvent::PointerDown(_) => hover_node_id,
            UiEvent::Wheel(_) => hover_node_id,
            UiEvent::KeyUp(_) => focussed_node_id,
            UiEvent::KeyDown(_) => focussed_node_id,
            UiEvent::Ime(_) => focussed_node_id,
        };
        let target = target.unwrap_or_else(|| self.doc.inner().root_element().id);

        match event {
            UiEvent::PointerMove(data) => {
                self.handle_pointer_event(
                    target,
                    data,
                    DomEventData::PointerMove,
                    DomEventData::MouseMove,
                );
            }
            UiEvent::PointerUp(data) => {
                self.handle_pointer_event(
                    target,
                    data,
                    DomEventData::PointerUp,
                    DomEventData::MouseUp,
                );
            }
            UiEvent::PointerDown(data) => {
                self.handle_pointer_event(
                    target,
                    data,
                    DomEventData::PointerDown,
                    DomEventData::MouseDown,
                );
            }
            UiEvent::Wheel(data) => {
                self.handle_dom_event(DomEvent::new(target, DomEventData::Wheel(data)))
            }
            UiEvent::KeyUp(data) => {
                self.handle_dom_event(DomEvent::new(target, DomEventData::KeyUp(data)))
            }
            UiEvent::KeyDown(data) => {
                self.handle_dom_event(DomEvent::new(target, DomEventData::KeyDown(data)))
            }
            UiEvent::Ime(data) => {
                self.handle_dom_event(DomEvent::new(target, DomEventData::Ime(data)))
            }
        };

        // Update document input state (hover, focus, active, etc)
        if should_clear_hover {
            self.doc.inner_mut().clear_hover();
        }
    }

    #[inline]
    pub fn handle_dom_event(&mut self, event: DomEvent) {
        self.queue.push_back(event);
        self.process_queue();
    }

    fn handle_pointer_event(
        &mut self,
        target: usize,
        data: PointerEvent,
        make_ptr_data: impl FnOnce(PointerEvent) -> DomEventData,
        make_mouse_data: impl FnOnce(PointerEvent) -> DomEventData,
    ) {
        let mut ptr_event = DomEvent::new(target, make_ptr_data(data.clone()));
        let mut event_state = EventState::default();
        event_state = self.run_handler_event(&mut ptr_event, event_state);
        if !event_state.is_cancelled() && data.is_mouse() {
            let mut mouse_event = DomEvent::new(target, make_mouse_data(data));
            event_state = self.run_handler_event(&mut mouse_event, event_state);
        }
        if !event_state.is_cancelled() {
            self.run_default_action(&mut ptr_event);
        }
        self.process_queue();
    }

    fn process_queue(&mut self) {
        while let Some(mut event) = self.queue.pop_front() {
            let event_state = self.run_handler_event(&mut event, EventState::default());
            if !event_state.is_cancelled() {
                self.run_default_action(&mut event);
            }
        }
    }

    fn run_handler_event(
        &mut self,
        event: &mut DomEvent,
        initial_event_state: EventState,
    ) -> EventState {
        let chain = if event.bubbles {
            let doc = self.doc.inner();
            doc.node_chain(event.target)
        } else {
            vec![event.target]
        };

        let mut event_state = initial_event_state;
        self.handler
            .handle_event(&chain, event, self.doc, &mut event_state);

        event_state
    }

    fn run_default_action(&mut self, event: &mut DomEvent) {
        let mut doc = self.doc.inner_mut();
        doc.handle_dom_event(event, |new_evt| self.queue.push_back(new_evt));
    }
}
