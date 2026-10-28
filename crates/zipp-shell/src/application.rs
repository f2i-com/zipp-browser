//! Top-level application handler that owns all windows and dispatches events.
//!
//! [`ZippApp`] implements winit's [`ApplicationHandler`] trait, managing
//! window creation/destruction, surface lifecycle, and routing of both winit
//! window events and internal [`ShellEvent`]s.

use crate::event::{ShellEvent, ShellProxy};

use anyrender::WindowRenderer;
use std::collections::HashMap;
use std::sync::mpsc::Receiver;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::WindowId;

#[cfg(target_os = "macos")]
use winit::platform::macos::ApplicationHandlerExtMacOS;

use crate::{View, WindowConfig};
use zipp_dom::{Document, DocumentConfig};

/// Builds the document for a navigation, on the main thread: `(url, contents,
/// config, viewport)`, the viewport in CSS pixels.
///
/// Set by an embedder whose documents need more than parsing -- the browser runs
/// the page's JavaScript here and attaches its event bridge, which cannot happen
/// on the thread that fetched the page because the script state is not `Send`.
pub type DocumentBuilder = Box<dyn FnMut(String, String, DocumentConfig, (u32, u32)) -> Box<dyn Document>>;

pub struct ZippApp<Rend: WindowRenderer> {
    pub windows: HashMap<WindowId, View<Rend>>,
    pub pending_windows: Vec<WindowConfig<Rend>>,
    pub proxy: ShellProxy,
    pub event_queue: Receiver<ShellEvent>,
    document_builder: Option<DocumentBuilder>,
}

impl<Rend: WindowRenderer> ZippApp<Rend> 
where
    for<'a> Rend::ScenePainter<'a>: zipp_renderer::tinyskia::BlurLayer,
{
    pub fn new(proxy: ShellProxy, event_queue: Receiver<ShellEvent>) -> Self {
        ZippApp {
            windows: HashMap::new(),
            pending_windows: Vec::new(),
            proxy,
            event_queue,
            document_builder: None,
        }
    }

    /// Have navigations build their document with `builder` rather than by
    /// parsing `NavigationLoad::contents` as finished HTML.
    pub fn set_document_builder(&mut self, builder: DocumentBuilder) {
        self.document_builder = Some(builder);
    }

    pub fn add_window(&mut self, window_config: WindowConfig<Rend>) {
        self.pending_windows.push(window_config);
    }

    fn window_mut_by_doc_id(&mut self, doc_id: usize) -> Option<&mut View<Rend>> {
        self.windows.values_mut().find(|w| w.doc.id() == doc_id)
    }

    pub fn handle_shell_event(
        &mut self,
        _event_loop: &dyn ActiveEventLoop,
        event: ShellEvent,
    ) {
        match event {
            ShellEvent::Poll { window_id } => {
                if let Some(window) = self.windows.get_mut(&window_id) {
                    window.poll();
                };
            }
            ShellEvent::RequestRedraw { doc_id } => {
                // TODO: Handle multiple documents per window
                if let Some(window) = self.window_mut_by_doc_id(doc_id) {
                    window.request_redraw();
                }
            }

            #[cfg(feature = "accessibility")]
            ShellEvent::Accessibility { window_id, data } => {
                if let Some(window) = self.windows.get_mut(&window_id) {
                    match &*data {
                        accesskit_xplat::WindowEvent::InitialTreeRequested => {
                            window.build_accessibility_tree();
                        }
                        accesskit_xplat::WindowEvent::AccessibilityDeactivated => {
                            // TODO
                        }
                        accesskit_xplat::WindowEvent::ActionRequested(_req) => {
                            // TODO
                        }
                    }
                }
            }
            ShellEvent::Embedder(_) => {
                // Do nothing. Should be handled by embedders (if required).
            }
            ShellEvent::Navigate(_opts) => {
                // Handled by embedder's NavigationProvider
            }
            ShellEvent::ScrollToFragment { fragment } => {
                if let Some((_, view)) = self.windows.iter_mut().next() {
                    // Lay out first. The click that followed the link usually
                    // re-rendered the page too, and its brand-new nodes have no
                    // geometry until the next resolve: the target reads as y=0
                    // and the "scroll" lands back at the top.
                    let animation_time = view.current_animation_time();
                    view.doc.inner_mut().resolve(animation_time);
                    let found = view.doc.inner_mut().scroll_to_fragment(&fragment);
                    if !found {
                        eprintln!("[NAV] No element for #{fragment}; staying put");
                    }
                    view.request_redraw();
                }
            }
            ShellEvent::NavigationLoad { url, contents, retain_scroll_position, .. } => {
                let t0 = std::time::Instant::now();
                if let Some((_, view)) = self.windows.iter_mut().next() {
                    let config = view.doc.inner().clone_config_for_navigation(url.clone());
                    let t1 = std::time::Instant::now();
                    let doc: Box<dyn Document> = match self.document_builder.as_mut() {
                        Some(build) => {
                            let inner = view.doc.inner();
                            let scale = inner.viewport().scale_f64();
                            let (w, h) = inner.viewport().window_size;
                            drop(inner);
                            let css = ((w as f64 / scale) as u32, (h as f64 / scale) as u32);
                            build(url.clone(), contents.clone(), config, css)
                        }
                        None => Box::new(zipp_dom::html_parser::HtmlDocument::from_html(&contents, config)),
                    };
                    let t2 = std::time::Instant::now();
                    view.replace_document(doc, retain_scroll_position);
                    let t3 = std::time::Instant::now();
                    view.request_redraw();
                    eprintln!("[PERF] Nav: config={:.0}ms parse={:.0}ms replace={:.0}ms total={:.0}ms ({} bytes)",
                        t1.duration_since(t0).as_millis(),
                        t2.duration_since(t1).as_millis(),
                        t3.duration_since(t2).as_millis(),
                        t3.duration_since(t0).as_millis(),
                        contents.len());
                }
            }
        }
    }
}

impl<Rend: WindowRenderer> ApplicationHandler for ZippApp<Rend> 
where
    for<'a> Rend::ScenePainter<'a>: zipp_renderer::tinyskia::BlurLayer,
{
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        // Resume existing windows
        for (_, view) in self.windows.iter_mut() {
            view.resume();
        }

        // Initialise pending windows
        for window_config in self.pending_windows.drain(..) {
            let mut view = View::init(window_config, event_loop, &self.proxy);
            view.resume();
            if !view.renderer.is_active() {
                continue;
            }
            self.windows.insert(view.window_id(), view);
        }
    }

    fn destroy_surfaces(&mut self, _event_loop: &dyn ActiveEventLoop) {
        for (_, view) in self.windows.iter_mut() {
            view.suspend();
        }
    }

    fn resumed(&mut self, _event_loop: &dyn ActiveEventLoop) {
        // TODO
    }

    fn suspended(&mut self, _event_loop: &dyn ActiveEventLoop) {
        // TODO
    }

    fn window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        // Exit the app when window close is requested.
        if matches!(event, WindowEvent::CloseRequested) {
            // Drop window before exiting event loop
            // See https://github.com/rust-windowing/winit/issues/4135
            let window = self.windows.remove(&window_id);
            drop(window);
            if self.windows.is_empty() {
                event_loop.exit();
            }
            return;
        }

        if let Some(window) = self.windows.get_mut(&window_id) {
            window.handle_winit_event(event);
        }

        self.proxy.send_event(ShellEvent::Poll { window_id });
    }

    fn proxy_wake_up(&mut self, event_loop: &dyn ActiveEventLoop) {
        while let Ok(event) = self.event_queue.try_recv() {
            self.handle_shell_event(event_loop, event);
        }
    }

    #[cfg(target_os = "macos")]
    fn macos_handler(&mut self) -> Option<&mut dyn ApplicationHandlerExtMacOS> {
        Some(self)
    }

    #[cfg(target_os = "ios")]
    fn about_to_wait(&mut self, _event_loop: &dyn ActiveEventLoop) {
        for view in self.windows.values_mut() {
            if view.ios_request_redraw.get() {
                view.window.request_redraw();
            }
        }
    }
}

#[cfg(target_os = "macos")]
impl<Rend: WindowRenderer> ApplicationHandlerExtMacOS for ZippApp<Rend> 
where
    for<'a> Rend::ScenePainter<'a>: zipp_renderer::tinyskia::BlurLayer,
{
    fn standard_key_binding(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        window_id: WindowId,
        action: &str,
    ) {
        let _ = event_loop;
        let _ = window_id;
        let _ = action;
    }
}
