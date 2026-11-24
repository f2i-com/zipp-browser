//! zipp-kms: a winit event loop on Linux's DRM/KMS and evdev.
//!
//! It runs an ordinary winit [`ApplicationHandler`] full screen with no display
//! server: the window is the screen, presented by softbuffer's KMS backend,
//! and input comes straight from the kernel's event devices. This is how
//! zipp-browser runs as the whole user space of zipp-linux.
//!
//! ```ignore
//! let event_loop = zipp_kms::EventLoop::new()?;
//! let proxy = event_loop.create_proxy();          // before run_app, as winit's
//! let (width, height) = event_loop.size();         // the screen, in pixels
//! event_loop.run_app(app)?;
//! ```
//!
//! The loop keeps winit's contract where an application can tell: one
//! coalesced `RedrawRequested` per `request_redraw`, `ControlFlow::WaitUntil`
//! honoured as the poll timeout (timers depend on it), and proxy wake-ups from
//! any thread. Redraws are paced on page-flip completion: softbuffer flips
//! without waiting, and a second flip while one is pending is refused.

#![cfg(target_os = "linux")]

use std::cell::Cell;
use std::fmt;
use std::io;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use drm::control::{Device as ControlDevice, Event as DrmEvent};
use raw_window_handle as rwh_06;
use winit_core::application::ApplicationHandler;
use winit_core::cursor::{CustomCursor, CustomCursorSource};
use winit_core::error::{NotSupportedError, RequestError};
use winit_core::event::{StartCause, WindowEvent};
use winit_core::event_loop::{
    ActiveEventLoop, ControlFlow, DeviceEvents, EventLoopProxy, EventLoopProxyProvider, OwnedDisplayHandle,
};
use winit_core::monitor::MonitorHandle;
use winit_core::window::{Theme, Window, WindowAttributes, WindowId};

mod console;
mod display;
mod input;
mod keymap;
mod pointer;
mod vbox;
mod window;

pub use display::{Display, rotation_named};

/// A turn asked for (`request_rotation`): 0-3 quarter turns, 4 the panel's
/// own, 255 none.
static ROTATION_ASKED: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(255);
/// The loop's wake-up, for `request_rotation` from any thread.
static WAKE_FD: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);
/// The screen's turn now, and the panel's own (quarter turns clockwise).
static ROTATION_NOW: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
static ROTATION_PANEL: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
type RotationCallback = Box<dyn Fn(u8, (u32, u32)) + Send + Sync>;

/// Told of each turn: the quarter turns, and the size of the buffers the
/// panel scans out (what a renderer turns its picture onto).
static ON_ROTATION: std::sync::OnceLock<RotationCallback> = std::sync::OnceLock::new();

/// Turn the screen: `Some(0..=3)` quarter turns clockwise, or `None` for the
/// panel's own (as the kernel knows the panel is mounted). Applied by the
/// loop, which tells the window its new size.
pub fn request_rotation(rotation: Option<u8>) {
    ROTATION_ASKED.store(rotation.map_or(4, |r| r % 4), std::sync::atomic::Ordering::Relaxed);
    let fd = WAKE_FD.load(std::sync::atomic::Ordering::Relaxed);
    if fd >= 0 {
        let one: u64 = 1;
        unsafe { libc::write(fd, (&one as *const u64).cast(), 8) };
    }
}

/// The screen's turn now and the panel's own, in quarter turns clockwise.
pub fn rotation() -> (u8, u8) {
    (ROTATION_NOW.load(std::sync::atomic::Ordering::Relaxed), ROTATION_PANEL.load(std::sync::atomic::Ordering::Relaxed))
}

/// Be told when the screen is turned: the quarter turns, and the size of the
/// panel's buffers. Once.
pub fn on_rotation(f: impl Fn(u8, (u32, u32)) + Send + Sync + 'static) {
    let _ = ON_ROTATION.set(Box::new(f));
}

/// How long a presented frame may take to reach the screen before the next
/// redraw goes ahead anyway (a redraw that presented nothing sends no flip).
const FLIP_TIMEOUT: Duration = Duration::from_millis(50);

/// What the loop, its window and its proxies share.
pub(crate) struct Shared {
    pub(crate) display: Display,
    pub(crate) redraw: AtomicBool,
    pub(crate) title: Mutex<String>,
    pub(crate) scale_factor: f64,
    pub(crate) theme: Theme,
    proxy_woken: AtomicBool,
    wake_fd: OwnedFd,
    /// A screen size asked for (a virtual machine's window resized), for the
    /// loop to set.
    pending_size: Mutex<Option<(u32, u32)>>,
}

impl Shared {
    /// Interrupt the loop's poll, from any thread.
    pub(crate) fn wake(&self) {
        let one: u64 = 1;
        unsafe { libc::write(self.wake_fd.as_raw_fd(), (&one as *const u64).cast(), 8) };
    }
}

impl fmt::Debug for Shared {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Shared").field("display", &self.display.path).finish_non_exhaustive()
    }
}

impl rwh_06::HasDisplayHandle for Shared {
    fn display_handle(&self) -> Result<rwh_06::DisplayHandle<'_>, rwh_06::HandleError> {
        let raw = rwh_06::RawDisplayHandle::Drm(rwh_06::DrmDisplayHandle::new(
            self.display.card.as_fd().as_raw_fd(),
        ));
        // SAFETY: the descriptor stays open as long as `self` does.
        Ok(unsafe { rwh_06::DisplayHandle::borrow_raw(raw) })
    }
}

#[derive(Debug)]
struct Proxy(Arc<Shared>);

impl EventLoopProxyProvider for Proxy {
    fn wake_up(&self) {
        self.0.proxy_woken.store(true, Ordering::Release);
        self.0.wake();
    }
}

/// The handle an application sees while the loop runs.
struct Target {
    shared: Arc<Shared>,
    control_flow: Cell<ControlFlow>,
    exit: Cell<bool>,
    window: Cell<Option<WindowId>>,
}

impl fmt::Debug for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("zipp_kms::ActiveEventLoop").finish_non_exhaustive()
    }
}

impl ActiveEventLoop for Target {
    fn create_proxy(&self) -> EventLoopProxy {
        EventLoopProxy::new(Arc::new(Proxy(self.shared.clone())))
    }

    fn create_window(&self, _attributes: WindowAttributes) -> Result<Box<dyn Window>, RequestError> {
        // Every window is the whole screen; there is only ever one.
        if self.window.get().is_some() {
            return Err(RequestError::NotSupported(NotSupportedError::new("one window: the screen")));
        }
        let id = WindowId::from_raw(1);
        self.window.set(Some(id));
        Ok(Box::new(window::KmsWindow { id, shared: self.shared.clone() }))
    }

    fn create_custom_cursor(&self, _source: CustomCursorSource) -> Result<CustomCursor, RequestError> {
        Err(RequestError::NotSupported(NotSupportedError::new("custom cursors")))
    }

    fn available_monitors(&self) -> Box<dyn Iterator<Item = MonitorHandle>> {
        Box::new(std::iter::empty())
    }

    fn primary_monitor(&self) -> Option<MonitorHandle> {
        None
    }

    fn listen_device_events(&self, _allowed: DeviceEvents) {}

    fn system_theme(&self) -> Option<Theme> {
        Some(self.shared.theme)
    }

    fn set_control_flow(&self, control_flow: ControlFlow) {
        self.control_flow.set(control_flow);
    }

    fn control_flow(&self) -> ControlFlow {
        self.control_flow.get()
    }

    fn exit(&self) {
        self.exit.set(true);
    }

    fn exiting(&self) -> bool {
        self.exit.get()
    }

    fn owned_display_handle(&self) -> OwnedDisplayHandle {
        OwnedDisplayHandle::new(self.shared.clone())
    }

    fn rwh_06_handle(&self) -> &dyn rwh_06::HasDisplayHandle {
        &*self.shared
    }
}

/// The screen and the input devices, ready to run an application on.
pub struct EventLoop {
    target: Target,
    input: input::Input,
    _console: Option<console::Console>,
}

impl EventLoop {
    /// Take the screen and the input devices. See [`Display::open`] for which
    /// screen. `ZIPP_KMS_SCALE` sets the scale factor (default 1), and
    /// `ZIPP_COLOR_SCHEME=dark` the theme the window reports.
    pub fn new() -> io::Result<EventLoop> {
        let display = Display::open()?;
        let mut input = input::Input::new(display.visible_size());
        input.set_screen(display.visible_size(), display.panel_size(), display.rotation());
        ROTATION_NOW.store(display.rotation(), std::sync::atomic::Ordering::Relaxed);
        ROTATION_PANEL.store(display.panel_rotation(), std::sync::atomic::Ordering::Relaxed);
        let wake_fd = unsafe {
            let fd = libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC);
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            OwnedFd::from_raw_fd(fd)
        };
        let scale_factor = std::env::var("ZIPP_KMS_SCALE").ok().and_then(|s| s.parse().ok()).unwrap_or(1.0);
        let dark = std::env::var("ZIPP_COLOR_SCHEME").is_ok_and(|s| s.eq_ignore_ascii_case("dark"));
        let shared = Arc::new(Shared {
            display,
            redraw: AtomicBool::new(false),
            title: Mutex::new(String::new()),
            scale_factor,
            theme: if dark { Theme::Dark } else { Theme::Light },
            proxy_woken: AtomicBool::new(false),
            wake_fd,
            pending_size: Mutex::new(None),
        });
        WAKE_FD.store(shared.wake_fd.as_raw_fd(), std::sync::atomic::Ordering::Relaxed);
        // In VirtualBox the screen follows its window, unless a mode was
        // named (ZIPP_KMS_MODE).
        if std::env::var_os("ZIPP_KMS_MODE").is_none() {
            let to = shared.clone();
            vbox::watch(move |w, h| {
                *to.pending_size.lock().unwrap_or_else(|e| e.into_inner()) = Some((w, h));
                to.wake();
            });
        }
        Ok(EventLoop {
            target: Target {
                shared,
                control_flow: Cell::new(ControlFlow::Wait),
                exit: Cell::new(false),
                window: Cell::new(None),
            },
            input,
            _console: console::Console::take(),
        })
    }

    /// The screen's size in pixels: what the application should lay out for.
    pub fn size(&self) -> (u32, u32) {
        self.target.shared.display.visible_size()
    }

    /// The quarter turns clockwise the picture is turned by onto the panel,
    /// and the size of the buffers the panel scans out: what a renderer
    /// drawing at `size()` turns its picture onto.
    pub fn screen_transform(&self) -> (u8, (u32, u32)) {
        let display = &self.target.shared.display;
        (display.rotation(), display.buffer_size())
    }

    pub fn scale_factor(&self) -> f64 {
        self.target.shared.scale_factor
    }

    pub fn create_proxy(&self) -> EventLoopProxy {
        self.target.create_proxy()
    }

    /// Run `app` until it calls `exit()`.
    pub fn run_app<A: ApplicationHandler>(mut self, mut app: A) -> io::Result<()> {
        let target = &self.target;
        let shared = target.shared.clone();
        let mut events = Vec::new();

        app.new_events(target, StartCause::Init);
        app.can_create_surfaces(target);
        // Creating the surface presented the first frame.
        let mut flip_due: Option<Instant> = target.window.get().map(|_| Instant::now() + FLIP_TIMEOUT);
        // A window is exposed as it appears: every winit platform sends it a
        // RedrawRequested then, unasked, and applications wait for that one
        // (zipp-browser starts a page's requestAnimationFrame loop from it).
        if target.window.get().is_some() {
            shared.redraw.store(true, Ordering::Release);
        }

        // ZIPP_KMS_STATS: how many frames are drawn each second, in the log.
        let stats = std::env::var_os("ZIPP_KMS_STATS").is_some();
        let (mut frames, mut counted_since) = (0u32, Instant::now());
        loop {
            if let Some(id) = target.window.get()
                && flip_due.is_none()
                && shared.redraw.swap(false, Ordering::AcqRel)
            {
                if stats {
                    frames += 1;
                    if counted_since.elapsed() >= Duration::from_secs(1) {
                        eprintln!("[kms] {frames} frames in {:.2} s", counted_since.elapsed().as_secs_f64());
                        (frames, counted_since) = (0, Instant::now());
                    }
                }
                app.window_event(target, id, WindowEvent::RedrawRequested);
                flip_due = Some(Instant::now() + FLIP_TIMEOUT);
                // A drawn pointer goes on the new frame at once: the flip
                // is queued, the CRTC already names its framebuffer, and
                // the pointer's update is committed after it.
                shared.display.frame_presented();
            }
            app.about_to_wait(target);
            if target.exit.get() {
                break;
            }

            // Sleep until there is input, a wake-up, a flip, the time the
            // application asked for, or a redraw can go ahead.
            let now = Instant::now();
            let requested = match target.control_flow.get() {
                ControlFlow::Poll => Some(now),
                ControlFlow::Wait => None,
                ControlFlow::WaitUntil(t) => Some(t),
            };
            let mut deadline = requested;
            if shared.redraw.load(Ordering::Acquire) {
                let ready = flip_due.unwrap_or(now);
                deadline = Some(deadline.map_or(ready, |d| d.min(ready)));
            }
            let timeout_ms = deadline.map_or(-1, |d| {
                let left = d.saturating_duration_since(now);
                left.as_millis().min(i32::MAX as u128) as i32 + i32::from(left.subsec_micros() % 1000 != 0)
            });

            let drm_fd = shared.display.card.as_fd().as_raw_fd();
            let input_fds = self.input.fds();
            let mut fds: Vec<libc::pollfd> = [drm_fd, shared.wake_fd.as_raw_fd()]
                .into_iter()
                .chain(input_fds.iter().copied())
                .map(|fd| libc::pollfd { fd, events: libc::POLLIN, revents: 0 })
                .collect();
            let n = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout_ms) };
            if n < 0 {
                let e = io::Error::last_os_error();
                if e.kind() != io::ErrorKind::Interrupted {
                    return Err(e);
                }
            }

            let cause = match requested {
                Some(t) if Instant::now() >= t && n == 0 => StartCause::ResumeTimeReached { start: now, requested_resume: t },
                Some(t) if n > 0 => StartCause::WaitCancelled { start: now, requested_resume: Some(t) },
                None if n > 0 => StartCause::WaitCancelled { start: now, requested_resume: None },
                _ => StartCause::Poll,
            };
            app.new_events(target, cause);

            // The previous frame reached the screen: its flip completed, or the
            // wait for one ran out (some drivers never report it).
            let was_pending = flip_due.is_some();
            if fds[0].revents & libc::POLLIN != 0
                && let Ok(drm_events) = shared.display.card.receive_events()
                && drm_events.into_iter().any(|e| matches!(e, DrmEvent::PageFlip(_)))
            {
                flip_due = None;
            }
            if flip_due.is_some_and(|due| Instant::now() >= due) {
                flip_due = None;
            }
            if was_pending && flip_due.is_none() {
                shared.display.frame_presented();
            }

            if fds[1].revents & libc::POLLIN != 0 {
                let mut count = 0u64;
                unsafe { libc::read(shared.wake_fd.as_raw_fd(), (&mut count as *mut u64).cast(), 8) };
            }
            if shared.proxy_woken.swap(false, Ordering::AcqRel) {
                app.proxy_wake_up(target);
            }

            // A new turn for the picture on the panel: the window's new size.
            let asked = ROTATION_ASKED.swap(255, Ordering::AcqRel);
            if asked != 255 && let Some(id) = target.window.get() {
                let display = &shared.display;
                let rotation = if asked == 4 { display.panel_rotation() } else { asked };
                display.set_rotation(rotation);
                ROTATION_NOW.store(rotation, Ordering::Relaxed);
                self.input.set_screen(display.visible_size(), display.panel_size(), rotation);
                if let Some(f) = ON_ROTATION.get() {
                    f(rotation, display.buffer_size());
                }
                let (dw, dh) = display.size();
                eprintln!("[kms] turned {} degrees: {dw}x{dh}", rotation as u32 * 90);
                app.window_event(target, id, WindowEvent::SurfaceResized(dpi::PhysicalSize::new(dw, dh)));
                shared.redraw.store(true, Ordering::Release);
                flip_due = None;
            }

            // A new size for the screen: its mode, then the window's.
            let wanted = shared.pending_size.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let (Some((w, h)), Some(id)) = (wanted, target.window.get())
                && shared.display.visible_size() != (w, h)
            {
                let (w, h) = (w.clamp(640, 7680), h.clamp(400, 4320));
                match shared.display.set_size(w, h) {
                    Ok(()) => {
                        let display = &shared.display;
                        self.input.set_screen(display.visible_size(), display.panel_size(), display.rotation());
                        if let Some(f) = ON_ROTATION.get() {
                            f(display.rotation(), display.buffer_size());
                        }
                        let (dw, dh) = shared.display.size();
                        app.window_event(target, id, WindowEvent::SurfaceResized(dpi::PhysicalSize::new(dw, dh)));
                        shared.redraw.store(true, Ordering::Release);
                        flip_due = None;
                    }
                    Err(e) => eprintln!("[kms] {w}x{h}: {e}"),
                }
            }

            if fds[2..].iter().any(|p| p.revents & (libc::POLLIN | libc::POLLERR | libc::POLLHUP) != 0) {
                let before = self.input.pointer;
                self.input.read(&mut events);
                if self.input.pointer != before {
                    // A finger needs no arrow under it; a mouse does.
                    if self.input.touched {
                        shared.display.hide_pointer();
                    } else {
                        shared.display.pointer_moved(self.input.pointer.0 as i32, self.input.pointer.1 as i32);
                    }
                }
                if let Some(id) = target.window.get() {
                    for event in events.drain(..) {
                        app.window_event(target, id, event);
                    }
                }
                events.clear();
            }
        }
        Ok(())
    }
}
