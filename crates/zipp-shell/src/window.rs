//! Per-window state and event handling.
//!
//! [`View`] owns a document, a renderer, and a winit window. It processes
//! winit events (keyboard, pointer, scroll, resize, etc.) and drives the
//! render loop.

use crate::WindowShellProvider;
use crate::convert_events::{
    button_source_to_dom, color_scheme_to_theme, pointer_source_to_dom,
    pointer_source_to_dom_details, theme_to_color_scheme, winit_ime_to_dom,
    winit_key_event_to_dom, winit_modifiers_to_kbt_modifiers,
};
use crate::event::{ShellProxy, create_waker};
use anyrender::WindowRenderer;
use zipp_dom::Document;
use zipp_renderer::paint_scene;
use zipp_dom::traits_src::events::{
    PointerEvent, PointerId, WheelDelta, WheelEvent, MouseEventButton,
    MouseEventButtons, PointerCoords, PointerDetails, UiEvent,
};
use zipp_dom::traits_src::navigation::NavigationOptions;
use zipp_dom::traits_src::shell::Viewport;
use winit::dpi::{LogicalPosition, PhysicalInsets, PhysicalPosition};
use winit::keyboard::PhysicalKey;

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::Waker;
use std::time::Instant;
use winit::event::{ButtonSource, ElementState, MouseButton};
use winit::event_loop::ActiveEventLoop;
use winit::window::{Theme, WindowAttributes, WindowId};
use winit::{event::Modifiers, event::WindowEvent, keyboard::KeyCode, window::Window};

#[cfg(feature = "accessibility")]
use crate::accessibility::AccessibilityState;

pub struct WindowConfig<Rend: WindowRenderer> {
    doc: Box<dyn Document>,
    attributes: WindowAttributes,
    renderer: Rend,
}

impl<Rend: WindowRenderer> WindowConfig<Rend> {
    pub fn new(doc: Box<dyn Document>, renderer: Rend) -> Self {
        Self::with_attributes(doc, renderer, WindowAttributes::default())
    }

    /// A window that opens at `size` (logical pixels) rather than the
    /// platform's default — 800x600 on Windows.
    pub fn with_size(doc: Box<dyn Document>, renderer: Rend, size: (u32, u32)) -> Self {
        let attributes = WindowAttributes::default()
            .with_surface_size(winit::dpi::LogicalSize::new(size.0, size.1));
        Self::with_attributes(doc, renderer, attributes)
    }

    pub fn with_attributes(
        doc: Box<dyn Document>,
        renderer: Rend,
        attributes: WindowAttributes,
    ) -> Self {
        WindowConfig {
            doc,
            attributes,
            renderer,
        }
    }
}

pub struct View<Rend: WindowRenderer> {
    pub doc: Box<dyn Document>,

    pub renderer: Rend,
    pub waker: Option<Waker>,

    pub proxy: ShellProxy,
    pub window: Arc<dyn Window>,

    /// The state of the keyboard modifiers (ctrl, shift, etc). Winit/Tao don't track these for us so we
    /// need to store them in order to have access to them when processing keypress events
    pub theme_override: Option<Theme>,
    pub keyboard_modifiers: Modifiers,
    pub buttons: MouseEventButtons,
    pub pointer_pos: PhysicalPosition<f64>,
    pub animation_timer: Option<Instant>,
    pub is_visible: bool,
    pub safe_area_insets: PhysicalInsets<u32>,
    /// Whether the next redraw must restyle and lay out the document first.
    /// Set by every redraw request except a paint-only one (see `redraw`).
    pub needs_resolve: Arc<AtomicBool>,
    /// When the page's next animation frame is due, while it wants frames.
    pub next_animation_frame: Option<Instant>,

    #[cfg(feature = "accessibility")]
    /// Accessibility adapter for `accesskit`.
    pub accessibility: AccessibilityState,

    // Calling request_redraw within a WindowEvent doesn't work on iOS. So on iOS we track the state
    // with a boolean and call request_redraw in about_to_wait
    //
    // See https://github.com/rust-windowing/winit/issues/3406
    #[cfg(target_os = "ios")]
    pub ios_request_redraw: std::cell::Cell<bool>,
}

impl<Rend: WindowRenderer> View<Rend> 
where
    for<'a> Rend::ScenePainter<'a>: zipp_renderer::tinyskia::BlurLayer,
{
    pub fn init(
        config: WindowConfig<Rend>,
        event_loop: &dyn ActiveEventLoop,
        proxy: &ShellProxy,
    ) -> Self {
        // We create window as invisble and then later make window visible
        // after AccessKit has initialised to avoid AccessKit panics
        let is_visible = config.attributes.visible;
        let attrs = config.attributes.with_visible(false);

        let winit_window: Arc<dyn Window> = Arc::from(event_loop.create_window(attrs).unwrap());
        #[cfg(feature = "accessibility")]
        let accessibility = AccessibilityState::new(&*winit_window, proxy.clone());

        if is_visible {
            winit_window.set_visible(true);
        }

        // Create viewport
        // TODO: account for the "safe area"
        let size = winit_window.surface_size();
        let scale = winit_window.scale_factor() as f32;
        let safe_area_insets = winit_window.safe_area();
        let theme = winit_window.theme().unwrap_or(Theme::Light);
        let color_scheme = theme_to_color_scheme(theme);
        let viewport = Viewport::new(size.width, size.height, scale, color_scheme);

        // Create shell provider
        let needs_resolve = Arc::new(AtomicBool::new(true));
        let shell_provider =
            WindowShellProvider::with_resolve_flag(winit_window.clone(), needs_resolve.clone());

        let mut doc = config.doc;
        let mut inner = doc.inner_mut();
        inner.set_viewport(viewport);
        inner.set_shell_provider(Arc::new(shell_provider));

        // If the document title is set prior to the window being created then it will
        // have been sent to a dummy ShellProvider and won't get picked up.
        // So we look for it here and set it if present.
        let title = inner.find_title_node().map(|node| node.text_content());
        if let Some(title) = title {
            winit_window.set_title(&title);
        }

        drop(inner);

        Self {
            renderer: config.renderer,
            waker: None,
            animation_timer: None,
            keyboard_modifiers: Default::default(),
            proxy: proxy.clone(),
            window: winit_window.clone(),
            doc,
            theme_override: None,
            buttons: MouseEventButtons::None,
            safe_area_insets,
            needs_resolve,
            next_animation_frame: None,
            pointer_pos: Default::default(),
            is_visible: winit_window.is_visible().unwrap_or(true),
            #[cfg(feature = "accessibility")]
            accessibility,

            #[cfg(target_os = "ios")]
            ios_request_redraw: std::cell::Cell::new(false),
        }
    }

    pub fn replace_document(&mut self, new_doc: Box<dyn Document>, retain_scroll_position: bool) {
        let inner = self.doc.inner();
        let scroll = inner.viewport_scroll();
        let viewport = inner.viewport().clone();
        let shell_provider = inner.shell_provider.clone();
        drop(inner);

        self.doc = new_doc;

        let mut inner = self.doc.inner_mut();
        inner.set_viewport(viewport);
        inner.set_shell_provider(shell_provider);
        // The new document set its title while it was parsed, before it had this
        // window's shell provider -- so, as in `init`, find it and set it here, or
        // the window keeps the first page's title for good.
        let title = inner.find_title_node().map(|node| node.text_content());
        drop(inner);
        self.window.set_title(title.as_deref().map(str::trim).unwrap_or_default());

        self.poll();
        self.request_redraw();

        if retain_scroll_position {
            self.doc.inner_mut().set_viewport_scroll(scroll);
        }
    }

    #[inline]
    pub fn theme_override(&self) -> Option<Theme> {
        self.theme_override
    }

    #[inline]
    pub fn current_theme(&self) -> Theme {
        color_scheme_to_theme(self.doc.inner().viewport().color_scheme)
    }

    pub fn set_theme_override(&mut self, theme: Option<Theme>) {
        self.theme_override = theme;
        let theme = theme.or(self.window.theme()).unwrap_or(Theme::Light);
        self.with_viewport(|v| v.color_scheme = theme_to_color_scheme(theme));
    }

    pub fn downcast_doc_mut<T: 'static>(&mut self) -> &mut T {
        (&mut *self.doc as &mut dyn Any)
            .downcast_mut::<T>()
            .unwrap()
    }

    #[inline]
    pub fn current_animation_time(&mut self) -> f64 {
        match &self.animation_timer {
            Some(start) => Instant::now().duration_since(*start).as_secs_f64(),
            None => {
                self.animation_timer = Some(Instant::now());
                0.0
            }
        }
    }
}

// `paint_scene` needs a painter that can blur (`filter: blur()`), which is a
// capability beyond `PaintScene`.
impl<Rend: WindowRenderer> View<Rend>
where
    for<'a> Rend::ScenePainter<'a>: zipp_renderer::tinyskia::BlurLayer,
{
    pub fn resume(&mut self) {
        let window_id = self.window_id();
        let animation_time = self.current_animation_time();

        let mut inner = self.doc.inner_mut();

        // Resolve dom
        inner.resolve(animation_time);

        // Resume renderer
        let (width, height) = inner.viewport().window_size;
        let scale = inner.viewport().scale_f64();
        self.renderer
            .resume(Arc::new(self.window.clone()), width, height);
        if !self.renderer.is_active() {
            panic!("Renderer failed to resume");
        };

        // Render
        let insets = self.safe_area_insets.to_logical(scale);
        self.renderer.render(|scene| {
            paint_scene(scene, &inner, scale, width, height, insets.left, insets.top)
        });

        // Set waker
        self.waker = Some(create_waker(&self.proxy, window_id));
    }

    pub fn suspend(&mut self) {
        self.waker = None;
        self.renderer.suspend();
    }

    pub fn poll(&mut self) -> bool {
        if let Some(waker) = &self.waker {
            let cx = std::task::Context::from_waker(waker);
            if self.doc.poll(Some(cx)) {
                #[cfg(feature = "accessibility")]
                {
                    let inner = self.doc.inner();
                    if inner.has_changes() {
                        self.accessibility.update_tree(&inner);
                    }
                }

                self.request_redraw();
                return true;
            }
        }

        false
    }

    #[inline]
    pub fn request_redraw(&self) {
        self.needs_resolve.store(true, Ordering::Release);
        if self.renderer.is_active() {
            self.window.request_redraw();
            #[cfg(target_os = "ios")]
            self.ios_request_redraw.set(true);
        }
    }

    pub fn redraw(&mut self) {
        #[cfg(target_os = "ios")]
        self.ios_request_redraw.set(false);
        let animation_time = self.current_animation_time();
        let is_visible = self.is_visible;

        // A page drawing frame by frame (a WebGL canvas) gets its next frame
        // when one is due, about every 16ms, whatever else asked for this
        // redraw. Its new pictures need only a repaint; a DOM change it made
        // asks for a full redraw itself.
        const FRAME: std::time::Duration = std::time::Duration::from_millis(16);
        let now = Instant::now();
        let animating = match self.next_animation_frame {
            Some(due) if now < due => true,
            _ => {
                let wants_more = self.doc.animation_frame();
                self.next_animation_frame = wants_more.then(|| now + FRAME);
                wants_more
            }
        };

        let mut inner = self.doc.inner_mut();
        // Restyle and lay out only when something asked for it. Without the
        // non-default `incremental` feature a resolve rebuilds the box tree and
        // lays out the whole page, 20-50ms on a big one -- too much to repeat
        // for a change only paint can see, like a video frame, or for the OS
        // asking for the window to be drawn again. Every ordinary redraw request
        // (input, resize, a loaded resource, a DOM change) sets the flag; a
        // running CSS animation resolves every frame as before.
        if self.needs_resolve.swap(false, Ordering::AcqRel) || inner.is_animating() {
            inner.resolve(animation_time);
        }

        let (width, height) = inner.viewport().window_size;
        let scale = inner.viewport().scale_f64();
        let is_animating = inner.is_animating();
        let is_blocked = inner.has_pending_critical_resources();
        let insets = self.safe_area_insets.to_logical(scale);

        if !is_blocked && is_visible {
            self.renderer.render(|scene| {
                paint_scene(scene, &inner, scale, width, height, insets.left, insets.top)
            });
        }

        drop(inner);

        if !is_blocked && is_visible && is_animating {
            self.request_redraw();
        } else if animating && is_visible {
            // Wait out the rest of the frame, then come back for the next one
            // (a repaint, not a resolve).
            if let Some(due) = self.next_animation_frame {
                let left = due.saturating_duration_since(Instant::now());
                if !left.is_zero() {
                    std::thread::sleep(left.min(FRAME));
                }
            }
            if self.renderer.is_active() {
                self.window.request_redraw();
            }
        }
    }

    #[inline]
    pub fn pointer_coords(&self, position: PhysicalPosition<f64>) -> PointerCoords {
        let inner = self.doc.inner();
        let scale = inner.viewport().scale_f64();
        let LogicalPosition::<f32> {
            x: screen_x,
            y: screen_y,
        } = position.to_logical(scale);
        let viewport_scroll_offset = inner.viewport_scroll();
        let client_x = screen_x - (self.safe_area_insets.left as f64 / scale) as f32;
        let client_y = screen_y - (self.safe_area_insets.top as f64 / scale) as f32;
        let page_x = client_x + viewport_scroll_offset.x as f32;
        let page_y = client_y + viewport_scroll_offset.y as f32;

        PointerCoords {
            screen_x,
            screen_y,
            client_x,
            client_y,
            page_x,
            page_y,
        }
    }

    #[inline]
    pub fn window_id(&self) -> WindowId {
        self.window.id()
    }

    #[inline]
    pub fn with_viewport(&mut self, cb: impl FnOnce(&mut Viewport)) {
        let mut inner = self.doc.inner_mut();
        let mut viewport = inner.viewport_mut();
        cb(&mut viewport);
        let (width, height) = viewport.window_size;
        drop(viewport);
        drop(inner);
        if width > 0 && height > 0 {
            let insets = self.safe_area_insets;
            self.renderer.set_size(
                width + insets.left + insets.right,
                height + insets.top + insets.bottom,
            );
            self.request_redraw();
        }
    }

    #[cfg(feature = "accessibility")]
    pub fn build_accessibility_tree(&mut self) {
        let inner = self.doc.inner();
        self.accessibility.update_tree(&inner);
    }

    /// Ask the embedder to go `back`, `forward` or `reload`: a navigation to
    /// `zipp-nav:<command>`, the same request the browser toolbar's buttons
    /// make. An embedder without history simply has nothing to do with it.
    fn navigate_command(&self, command: &str) {
        let Ok(url) = url::Url::parse(&format!("zipp-nav:{command}")) else {
            return;
        };
        let inner = self.doc.inner();
        let options = NavigationOptions::new(url, "text/plain".to_string(), inner.id());
        let provider = inner.navigation_provider.clone();
        drop(inner);
        provider.navigate_to(options);
    }

    pub fn handle_winit_event(&mut self, event: WindowEvent) {
        // Update accessibility focus and window size state in response to a Winit WindowEvent
        #[cfg(feature = "accessibility")]
        self.accessibility
            .process_window_event(&*self.window, &event);

        match event {
            WindowEvent::Destroyed => {}
            WindowEvent::ActivationTokenDone { .. } => {},
            WindowEvent::CloseRequested => {
                // Currently handled at the level above in application.rs
            }
            WindowEvent::RedrawRequested => {
                self.redraw();
            }
            WindowEvent::Moved(_) => {}
            WindowEvent::Occluded(is_occluded) => {
                self.is_visible = !is_occluded;
                if self.is_visible {
                    self.request_redraw();
                }
            },
            WindowEvent::SurfaceResized(physical_size) => {
                self.safe_area_insets = self.window.safe_area();
                let insets = self.safe_area_insets;
                let width = physical_size.width - insets.left - insets.right;
                let height = physical_size.height - insets.top - insets.bottom;
                self.with_viewport(|v| v.window_size = (width, height));
                self.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.with_viewport(|v| v.set_hidpi_scale(scale_factor as f32));
                self.request_redraw();
            }
            WindowEvent::ThemeChanged(theme) => {
                let color_scheme = theme_to_color_scheme(self.theme_override.unwrap_or(theme));
                let mut inner = self.doc.inner_mut();
                inner.viewport_mut().color_scheme = color_scheme;
            }
            WindowEvent::Ime(ime_event) => {
                self.doc.handle_ui_event(UiEvent::Ime(winit_ime_to_dom(ime_event)));
                self.request_redraw();
            },
            WindowEvent::ModifiersChanged(new_state) => {
                // Store new keyboard modifier (ctrl, shift, etc) state for later use
                self.keyboard_modifiers = new_state;
            }
            WindowEvent::KeyboardInput { event, .. } => {

                if let PhysicalKey::Code(key_code) = event.physical_key && event.state.is_pressed() {
                        let ctrl = self.keyboard_modifiers.state().control_key();
                        let meta = self.keyboard_modifiers.state().meta_key();
                        let alt = self.keyboard_modifiers.state().alt_key();

                        // History and reload, as in any browser. Consumed here:
                        // the page does not also see them.
                        let command = match key_code {
                            KeyCode::ArrowLeft if alt => Some("back"),
                            KeyCode::ArrowRight if alt => Some("forward"),
                            KeyCode::BrowserBack => Some("back"),
                            KeyCode::BrowserForward => Some("forward"),
                            KeyCode::F5 | KeyCode::BrowserRefresh => Some("reload"),
                            KeyCode::KeyR if ctrl | meta => Some("reload"),
                            _ => None,
                        };
                        if let Some(command) = command {
                            self.navigate_command(command);
                            return;
                        }

                        // Ctrl+L / F6: into the address bar, its text selected. The
                        // browser's address bar is `input[name=__nav_url]`, the same
                        // embedder convention as `zipp-nav:` above.
                        if (key_code == KeyCode::KeyL && (ctrl | meta)) || key_code == KeyCode::F6 {
                            let mut inner = self.doc.inner_mut();
                            if let Ok(Some(address_bar)) = inner.query_selector("input[name=__nav_url]") {
                                inner.focus_and_select_all(address_bar);
                            }
                            drop(inner);
                            self.request_redraw();
                            return;
                        }

                        // Ctrl/Super keyboard shortcuts
                        if ctrl | meta {
                            match key_code {
                                KeyCode::Equal => {
                                    self.doc.inner_mut().viewport_mut().zoom_by(0.1);
                                },
                                KeyCode::Minus => {
                                    self.doc.inner_mut().viewport_mut().zoom_by(-0.1);
                                },
                                KeyCode::Digit0 => {
                                    self.doc.inner_mut().viewport_mut().set_zoom(1.0);
                                }
                                _ => {}
                            };
                        }

                        // Alt keyboard shortcuts
                        if alt {
                            match key_code {
                                KeyCode::KeyD => {
                                    let mut inner = self.doc.inner_mut();
                                    inner.devtools_mut().toggle_show_layout();
                                    drop(inner);
                                    self.request_redraw();
                                }
                                KeyCode::KeyH => {
                                    let mut inner = self.doc.inner_mut();
                                    inner.devtools_mut().toggle_highlight_hover();
                                    drop(inner);
                                    self.request_redraw();
                                }
                                KeyCode::KeyT => self.doc.inner().print_taffy_tree(),
                                _ => {}
                            };
                        }

                }

                // Unmodified keypresses
                let key_event_data = winit_key_event_to_dom(&event, self.keyboard_modifiers.state());
                let event = if event.state.is_pressed() {
                    UiEvent::KeyDown(key_event_data)
                } else {
                    UiEvent::KeyUp(key_event_data)
                };

                self.doc.handle_ui_event(event);
            }
            WindowEvent::PointerEntered { /*device_id*/.. } => {}
            WindowEvent::PointerLeft { /*device_id*/.. } => {}
            WindowEvent::PointerMoved { position, source, primary, .. } => {
                self.pointer_pos = position;
                let event = UiEvent::PointerMove(PointerEvent {
                    id: pointer_source_to_dom(&source),
                    is_primary: primary,
                    coords: self.pointer_coords(position),
                    button: Default::default(),
                    buttons: self.buttons,
                    mods: winit_modifiers_to_kbt_modifiers(self.keyboard_modifiers.state()),
                    details: pointer_source_to_dom_details(&source)
                });
                self.doc.handle_ui_event(event);
            }
            WindowEvent::PointerButton { button, state, primary, position, .. } => {
                // The mouse's own back/forward buttons.
                if state == ElementState::Pressed {
                    let command = match &button {
                        ButtonSource::Mouse(MouseButton::Back) => Some("back"),
                        ButtonSource::Mouse(MouseButton::Forward) => Some("forward"),
                        _ => None,
                    };
                    if let Some(command) = command {
                        self.navigate_command(command);
                        return;
                    }
                }
                let id = button_source_to_dom(&button);
                let coords = self.pointer_coords(position);
                self.pointer_pos = position;
                let button = match &button {
                    ButtonSource::Mouse(mouse_button) => match mouse_button {
                        MouseButton::Left => MouseEventButton::Main,
                        MouseButton::Right => MouseEventButton::Secondary,
                        MouseButton::Middle => MouseEventButton::Auxiliary,
                        // TODO: handle other button types
                        _ => MouseEventButton::Auxiliary,
                    }
                    _ => MouseEventButton::Main,
                };

                // Clear on release rather than toggle. A press that never reached
                // us (the click that activates the window, say) made XOR turn the
                // button ON at release and leave it held, so every later mouse
                // move read as a drag and swallowed the next click.
                match state {
                    ElementState::Pressed => self.buttons.insert(button.into()),
                    ElementState::Released => self.buttons.remove(button.into()),
                }

                if id != PointerId::Mouse {
                    let event = UiEvent::PointerMove(PointerEvent {
                        id,
                        is_primary: primary,
                        coords,
                        button: Default::default(),
                        buttons: self.buttons,
                        mods: winit_modifiers_to_kbt_modifiers(self.keyboard_modifiers.state()),
                        details: PointerDetails::default()
                    });
                    self.doc.handle_ui_event(event);
                }

                let event = PointerEvent {
                    id,
                    is_primary: primary,
                    coords,
                    button,
                    buttons: self.buttons,
                    mods: winit_modifiers_to_kbt_modifiers(self.keyboard_modifiers.state()),

                    // TODO: details for pointer up/down events
                    details: PointerDetails::default(),
                };

                let event = match state {
                    ElementState::Pressed => UiEvent::PointerDown(event),
                    ElementState::Released => UiEvent::PointerUp(event),
                };

                self.doc.handle_ui_event(event);
                self.request_redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let wheel_delta = match delta {
                    winit::event::MouseScrollDelta::LineDelta(x, y) => WheelDelta::Lines(x as f64, y as f64),
                    winit::event::MouseScrollDelta::PixelDelta(pos) => WheelDelta::Pixels(pos.x, pos.y),
                };

                let event = WheelEvent {
                    delta: wheel_delta,
                    coords: self.pointer_coords(self.pointer_pos),
                    buttons: self.buttons,
                    mods: winit_modifiers_to_kbt_modifiers(self.keyboard_modifiers.state()),
                };

                self.doc.handle_ui_event(UiEvent::Wheel(event));
                // Every other input arm requests a repaint unconditionally; this
                // one relied on `handle_wheel` doing it via the shell provider,
                // and only when the offset actually changed. Any path that moved
                // the page without that predicate agreeing left the window
                // showing a stale frame — scrolling appeared to do nothing.
                self.request_redraw();
            }
            WindowEvent::Focused(_) => {}
            WindowEvent::TouchpadPressure { .. } => {}
            WindowEvent::PinchGesture { .. } => {},
            WindowEvent::PanGesture { .. } => {},
            WindowEvent::DoubleTapGesture { .. } => {},
            WindowEvent::RotationGesture { .. } => {},
            WindowEvent::DragEntered { .. } => {},
            WindowEvent::DragMoved { .. } => {},
            WindowEvent::DragDropped { .. } => {},
            WindowEvent::DragLeft { .. } => {},
        }
    }
}
