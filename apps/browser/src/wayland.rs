//! A Wayland server for Linux apps' windows (navigator.zipp.wayland): the
//! programs ZIPP OS runs from the Alpine system draw their windows into
//! shared memory, as on any Wayland desktop, and their frames become the
//! images of canvases in ZIPP OS's windows; the pointer and the keys over
//! those canvases go back to them.
//!
//! It runs on a thread of its own, speaking the protocol in Rust
//! (wayland-server, no libwayland), on the socket /run/user/1000/wayland-0
//! (which pty.rs's jail shows the Linux system's apps where they expect it,
//! and which programs of the system's own use too: a browser of its own, in a
//! window), and offers what toolkits and simple clients need, with
//! buffers in shared memory only (no GPU buffers: an app's own OpenGL falls
//! back to drawing on the CPU): wl_compositor, wl_subcompositor, wl_shm,
//! wl_seat, wl_output, xdg_wm_base (windows and popups), server-side
//! decorations, and a clipboard that holds nothing.
//!
//! A surface's buffer is copied when it is committed, so the app may draw in
//! it again at once; a window's frame is its surface with its subsurfaces
//! drawn over it, cut to the window's geometry, handed to the page's thread
//! (`take_frames`) for the canvas the page shows that window in. Frame
//! callbacks are answered a frame later.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Write;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use wayland_protocols::xdg::decoration::zv1::server::zxdg_decoration_manager_v1::{self, ZxdgDecorationManagerV1};
use wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::{self, ZxdgToplevelDecorationV1};
use wayland_protocols::xdg::shell::server::xdg_popup::{self, XdgPopup};
use wayland_protocols::xdg::shell::server::xdg_positioner::{self, XdgPositioner};
use wayland_protocols::xdg::shell::server::xdg_surface::{self, XdgSurface};
use wayland_protocols::xdg::shell::server::xdg_toplevel::{self, XdgToplevel};
use wayland_protocols::xdg::shell::server::xdg_wm_base::{self, XdgWmBase};
use wayland_server::backend::{ClientData, ClientId, DisconnectReason, ObjectId};
use wayland_server::protocol::wl_buffer::WlBuffer;
use wayland_server::protocol::wl_callback::WlCallback;
use wayland_server::protocol::wl_compositor::{self, WlCompositor};
use wayland_server::protocol::wl_data_device::WlDataDevice;
use wayland_server::protocol::wl_data_device_manager::{self, WlDataDeviceManager};
use wayland_server::protocol::wl_data_source::WlDataSource;
use wayland_server::protocol::wl_keyboard::{self, WlKeyboard};
use wayland_server::protocol::wl_output::{self, WlOutput};
use wayland_server::protocol::wl_pointer::{self, WlPointer};
use wayland_server::protocol::wl_region::WlRegion;
use wayland_server::protocol::wl_seat::{self, WlSeat};
use wayland_server::protocol::wl_shm::{self, WlShm};
use wayland_server::protocol::wl_shm_pool::{self, WlShmPool};
use wayland_server::protocol::wl_subcompositor::{self, WlSubcompositor};
use wayland_server::protocol::wl_subsurface::{self, WlSubsurface};
use wayland_server::protocol::wl_surface::{self, WlSurface};
use wayland_server::protocol::wl_touch::WlTouch;
use wayland_server::{Client, DataInit, Dispatch, Display, DisplayHandle, GlobalDispatch, ListeningSocket, New, Resource, WEnum};

/// The US keyboard map clients are given (`xkbcli compile-keymap --layout us`).
static KEYMAP: &[u8] = include_bytes!("wayland_keymap.xkb");
/// How long after a frame its callbacks are answered.
const FRAME: Duration = Duration::from_millis(16);
/// Where the server listens.
const SOCKET: &str = "wayland-0";
pub(crate) const RUNTIME_DIR: &str = "/run/user/1000";
/// The user Linux apps run as (pty.rs's jail).
const USER: u32 = 1000;

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;
const BTN_SIDE: u32 = 0x113;
const BTN_EXTRA: u32 = 0x114;

// ---- the page's side --------------------------------------------------------------------

/// How many programs may have windows at once, and surfaces between them: a
/// program that opens without end costs itself, not the desktop.
const MAX_CLIENTS: usize = 64;
const MAX_SURFACES: usize = 4096;

/// What the page hears of: ("window" | "size" | "title" | "place" | "gone" |
/// "fullscreen" | "minimize" | "maximize", window, JSON).
static EVENTS: Mutex<Vec<(&'static str, u32, String)>> = Mutex::new(Vec::new());
/// Each window's newest frame, until the page shows it.
static FRAMES: Mutex<Option<HashMap<u32, crate::canvas::Frame>>> = Mutex::new(None);
static SERVER: Mutex<Option<Server>> = Mutex::new(None);

thread_local! {
    /// The canvas each window is shown in, by its `data-flid`.
    static CANVASES: RefCell<HashMap<u32, u32>> = RefCell::new(HashMap::new());
}

struct Server {
    commands: Sender<Command>,
    wake: Arc<OwnedFd>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Start the server, for a screen of this size (once; later calls find it
/// running): its socket's name, for WAYLAND_DISPLAY (with XDG_RUNTIME_DIR
/// /run/user/1000).
pub(crate) fn start(width: i32, height: i32) -> Result<String, String> {
    let mut server = lock(&SERVER);
    if server.is_some() {
        return Ok(SOCKET.into());
    }
    let dir = std::path::Path::new(RUNTIME_DIR);
    std::fs::create_dir_all(dir).map_err(|e| format!("OperationError: {}: {e}", dir.display()))?;
    let _ = std::os::unix::fs::chown(dir, Some(USER), Some(USER));
    let _ = std::fs::set_permissions(dir, std::os::unix::fs::PermissionsExt::from_mode(0o700));
    let path = dir.join(SOCKET);
    let socket = ListeningSocket::bind_absolute(path.clone()).map_err(|e| format!("OperationError: the Wayland socket: {e:?}"))?;
    // The apps are the user zipp's: theirs to connect to.
    let _ = std::os::unix::fs::chown(&path, Some(USER), Some(USER));
    // SAFETY: a new eventfd, owned from here on.
    let raw = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
    if raw < 0 {
        return Err(format!("OperationError: {}", std::io::Error::last_os_error()));
    }
    // SAFETY: just made, and no one else's.
    let wake = Arc::new(unsafe { OwnedFd::from_raw_fd(raw) });
    let (commands, receiver) = channel();
    let thread_wake = wake.clone();
    let screen = (width.clamp(320, 16384), height.clamp(200, 16384));
    std::thread::Builder::new()
        .name("wayland".into())
        .spawn(move || run(socket, receiver, thread_wake, screen))
        .map_err(|e| format!("OperationError: {e}"))?;
    *server = Some(Server { commands, wake });
    Ok(SOCKET.into())
}

/// What the page does to a window: `bind` (to the canvas with this
/// `data-flid`; 0 for none), `configure` (width, height, and its states:
/// "active", "fullscreen", comma-separated -- or "true" for active), `pointer`
/// (x, y in its frame), `leave`, `button` (DOM button, pressed), `axis`
/// (dx, dy in pixels), `key` (KeyboardEvent.code, pressed), `focus`, `close`.
pub(crate) fn call(command: &str, window: u32, a: &str, b: &str, c: &str) -> Result<(), String> {
    let n = |s: &str| s.parse::<f64>().ok().filter(|v| v.is_finite()).unwrap_or(0.0);
    let yes = |s: &str| s == "true" || s == "1";
    let command = match command {
        "bind" => {
            let flid = n(a) as u32;
            CANVASES.with(|c| {
                let mut canvases = c.borrow_mut();
                if flid == 0 {
                    canvases.remove(&window);
                } else {
                    canvases.insert(window, flid);
                }
            });
            crate::worker::wake();
            return Ok(());
        }
        "configure" => {
            let states: Vec<&str> = c.split(',').collect();
            Command::Configure {
                window,
                width: n(a) as i32,
                height: n(b) as i32,
                active: yes(c) || states.contains(&"active"),
                fullscreen: states.contains(&"fullscreen"),
            }
        }
        "pointer" => Command::Pointer { window, x: n(a), y: n(b) },
        "leave" => Command::Pointer { window: 0, x: 0.0, y: 0.0 },
        "button" => {
            let button = match n(a) as u32 {
                1 => BTN_MIDDLE,
                2 => BTN_RIGHT,
                3 => BTN_SIDE,
                4 => BTN_EXTRA,
                _ => BTN_LEFT,
            };
            Command::Button { button, pressed: yes(b) }
        }
        "axis" => Command::Axis { dx: n(a), dy: n(b) },
        "key" => match evdev(a) {
            Some(key) => Command::Key { window, key, pressed: yes(b) },
            None => return Ok(()),
        },
        "focus" => Command::Focus { window },
        "close" => Command::Close { window },
        other => return Err(format!("NotSupportedError: wayland has no {other}")),
    };
    send(command)
}

fn send(command: Command) -> Result<(), String> {
    let server = lock(&SERVER);
    let server = server.as_ref().ok_or("InvalidStateError: the Wayland server is not running")?;
    server.commands.send(command).map_err(|_| "OperationError: the Wayland server has stopped".to_string())?;
    let one = 1u64;
    // SAFETY: eight bytes to the server's eventfd.
    unsafe { libc::write(server.wake.as_raw_fd(), (&one as *const u64).cast(), 8) };
    Ok(())
}

/// What happened to the page's windows since last asked.
pub(crate) fn take_events() -> Vec<(&'static str, u32, String)> {
    let events = std::mem::take(&mut *lock(&EVENTS));
    CANVASES.with(|c| {
        let mut canvases = c.borrow_mut();
        for (kind, window, _) in &events {
            if *kind == "gone" {
                canvases.remove(window);
            }
        }
    });
    events
}

/// The new frames of windows the page shows, for their canvases.
pub(crate) fn take_frames() -> Vec<crate::canvas::Frame> {
    CANVASES.with(|c| {
        let canvases = c.borrow();
        let mut frames = lock(&FRAMES);
        let Some(frames) = frames.as_mut() else {
            return Vec::new();
        };
        let ready: Vec<u32> = frames.keys().filter(|w| canvases.contains_key(w)).copied().collect();
        ready
            .into_iter()
            .filter_map(|window| {
                let mut frame = frames.remove(&window)?;
                frame.flid = *canvases.get(&window)?;
                Some(frame)
            })
            .collect()
    })
}

/// A new page: the last one's Linux apps' windows are closed (their
/// connections ended), and nothing of theirs is shown.
pub(crate) fn new_page() {
    lock(&EVENTS).clear();
    if let Some(frames) = lock(&FRAMES).as_mut() {
        frames.clear();
    }
    CANVASES.with(|c| c.borrow_mut().clear());
    let _ = send(Command::Reset);
}

fn event(kind: &'static str, window: u32, json: serde_json::Value) {
    lock(&EVENTS).push((kind, window, json.to_string()));
    crate::worker::wake();
}

/// A KeyboardEvent.code's Linux key code.
fn evdev(code: &str) -> Option<u32> {
    const LETTERS: [u32; 26] = [30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20, 22, 47, 17, 45, 21, 44];
    if let Some(letter) = code.strip_prefix("Key").filter(|l| l.len() == 1) {
        let i = letter.as_bytes()[0].wrapping_sub(b'A');
        return LETTERS.get(i as usize).copied();
    }
    if let Some(digit) = code.strip_prefix("Digit").and_then(|d| d.parse::<u32>().ok()) {
        return (digit <= 9).then(|| if digit == 0 { 11 } else { digit + 1 });
    }
    if let Some(f) = code.strip_prefix('F').and_then(|f| f.parse::<u32>().ok()) {
        return match f {
            1..=10 => Some(58 + f),
            11 => Some(87),
            12 => Some(88),
            _ => None,
        };
    }
    Some(match code {
        "Escape" => 1,
        "Minus" => 12,
        "Equal" => 13,
        "Backspace" => 14,
        "Tab" => 15,
        "BracketLeft" => 26,
        "BracketRight" => 27,
        "Enter" => 28,
        "ControlLeft" => 29,
        "Semicolon" => 39,
        "Quote" => 40,
        "Backquote" => 41,
        "ShiftLeft" => 42,
        "Backslash" => 43,
        "Comma" => 51,
        "Period" => 52,
        "Slash" => 53,
        "ShiftRight" => 54,
        "NumpadMultiply" => 55,
        "AltLeft" => 56,
        "Space" => 57,
        "CapsLock" => 58,
        "NumLock" => 69,
        "ScrollLock" => 70,
        "Numpad7" => 71,
        "Numpad8" => 72,
        "Numpad9" => 73,
        "NumpadSubtract" => 74,
        "Numpad4" => 75,
        "Numpad5" => 76,
        "Numpad6" => 77,
        "NumpadAdd" => 78,
        "Numpad1" => 79,
        "Numpad2" => 80,
        "Numpad3" => 81,
        "Numpad0" => 82,
        "NumpadDecimal" => 83,
        "IntlBackslash" => 86,
        "IntlRo" => 89,
        "NumpadEnter" => 96,
        "ControlRight" => 97,
        "NumpadDivide" => 98,
        "PrintScreen" => 99,
        "AltRight" => 100,
        "Home" => 102,
        "ArrowUp" => 103,
        "PageUp" => 104,
        "ArrowLeft" => 105,
        "ArrowRight" => 106,
        "End" => 107,
        "ArrowDown" => 108,
        "PageDown" => 109,
        "Insert" => 110,
        "Delete" => 111,
        "NumpadEqual" => 117,
        "Pause" => 119,
        "IntlYen" => 124,
        "MetaLeft" | "OSLeft" | "SuperLeft" => 125,
        "MetaRight" | "OSRight" | "SuperRight" => 126,
        "ContextMenu" => 127,
        _ => return None,
    })
}

// ---- the server -------------------------------------------------------------------------

enum Command {
    Configure { window: u32, width: i32, height: i32, active: bool, fullscreen: bool },
    /// Window 0: the pointer left the page's windows.
    Pointer { window: u32, x: f64, y: f64 },
    Button { button: u32, pressed: bool },
    Axis { dx: f64, dy: f64 },
    Key { window: u32, key: u32, pressed: bool },
    Focus { window: u32 },
    Close { window: u32 },
    Reset,
}

fn run(socket: ListeningSocket, commands: Receiver<Command>, wake: Arc<OwnedFd>, screen: (i32, i32)) {
    let mut display = match Display::<State>::new() {
        Ok(display) => display,
        Err(e) => {
            eprintln!("[wayland] cannot start: {e}");
            return;
        }
    };
    let dh = display.handle();
    dh.create_global::<State, WlCompositor, ()>(5, ());
    dh.create_global::<State, WlSubcompositor, ()>(1, ());
    dh.create_global::<State, WlShm, ()>(1, ());
    dh.create_global::<State, WlSeat, ()>(7, ());
    dh.create_global::<State, WlOutput, ()>(4, ());
    dh.create_global::<State, XdgWmBase, ()>(3, ());
    dh.create_global::<State, ZxdgDecorationManagerV1, ()>(1, ());
    dh.create_global::<State, WlDataDeviceManager, ()>(3, ());
    let mut state = State::new(dh, screen);
    let display_fd = display.backend().poll_fd().as_raw_fd();
    eprintln!("[wayland] listening ({}x{})", screen.0, screen.1);
    loop {
        let timeout = state.tick.map_or(-1, |t| t.saturating_duration_since(Instant::now()).as_millis().min(1000) as i32);
        let mut fds = [socket.as_raw_fd(), display_fd, wake.as_raw_fd()].map(|fd| libc::pollfd { fd, events: libc::POLLIN, revents: 0 });
        // SAFETY: three pollfds of descriptors this thread holds open.
        unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout) };
        if fds[2].revents != 0 {
            let mut n = 0u64;
            // SAFETY: eight bytes from the eventfd into `n`.
            unsafe { libc::read(wake.as_raw_fd(), (&mut n as *mut u64).cast(), 8) };
        }
        while let Ok(Some(stream)) = socket.accept() {
            if state.clients.len() >= MAX_CLIENTS {
                eprintln!("[wayland] {MAX_CLIENTS} programs have windows: refused another");
                continue;
            }
            match display.handle().insert_client(stream, Arc::new(ClientState)) {
                Ok(client) => state.clients.push(client.id()),
                Err(e) => eprintln!("[wayland] a client could not connect: {e}"),
            }
        }
        if let Err(e) = display.dispatch_clients(&mut state) {
            eprintln!("[wayland] {e}");
        }
        while let Ok(command) = commands.try_recv() {
            state.command(command);
        }
        if state.tick.is_some_and(|t| t <= Instant::now()) {
            state.answer_frames();
        }
        let _ = display.flush_clients();
    }
}

struct ClientState;
impl ClientData for ClientState {}

/// Pixels, as premultiplied RGBA.
struct Pixels {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

enum Role {
    None,
    /// The surface of a window (a toplevel or a popup).
    Window(u32),
    /// Drawn over its parent, at this place in it.
    Sub { parent: ObjectId, x: i32, y: i32 },
}

struct Surface {
    wl: WlSurface,
    /// What the next commit applies: a buffer (or none), frame callbacks.
    buffer: Option<Option<WlBuffer>>,
    callbacks: Vec<WlCallback>,
    image: Option<Pixels>,
    role: Role,
    /// Its subsurfaces, bottom to top.
    children: Vec<ObjectId>,
    /// The window's own part of it (xdg_surface.set_window_geometry): no shadows.
    geometry: Option<(i32, i32, i32, i32)>,
}

struct Window {
    surface: ObjectId,
    xdg: XdgSurface,
    toplevel: Option<XdgToplevel>,
    popup: Option<XdgPopup>,
    /// A popup's window, and where it is in that window's frame.
    parent: u32,
    place: (i32, i32, i32, i32),
    title: String,
    app_id: String,
    configured: bool,
    /// The size of the frame the page was last told of (none before the first).
    shown: Option<(u32, u32)>,
    /// The size the page gives it (0: the app's choice), and whether it is the active one.
    want: (i32, i32, bool),
    /// Whether the page has it over the whole screen (its configure says so).
    fullscreen: bool,
    /// Where its frame starts in its surface.
    origin: (i32, i32),
}

#[derive(Default, Clone, Copy)]
struct Positioner {
    size: (i32, i32),
    anchor_rect: (i32, i32, i32, i32),
    anchor: u32,
    gravity: u32,
    offset: (i32, i32),
}

impl Positioner {
    /// Where the popup goes in its parent's frame, and its size.
    fn place(&self) -> (i32, i32, i32, i32) {
        // Anchor and gravity: 0 none, 1 top, 2 bottom, 3 left, 4 right,
        // 5 top left, 6 bottom left, 7 top right, 8 bottom right.
        let (ax, ay, aw, ah) = self.anchor_rect;
        let (w, h) = self.size;
        let px = match self.anchor {
            3 | 5 | 6 => ax,
            4 | 7 | 8 => ax + aw,
            _ => ax + aw / 2,
        };
        let py = match self.anchor {
            1 | 5 | 7 => ay,
            2 | 6 | 8 => ay + ah,
            _ => ay + ah / 2,
        };
        let x = match self.gravity {
            3 | 5 | 6 => px - w,
            4 | 7 | 8 => px,
            _ => px - w / 2,
        };
        let y = match self.gravity {
            1 | 5 | 7 => py - h,
            2 | 6 | 8 => py,
            _ => py - h / 2,
        };
        (x + self.offset.0, y + self.offset.1, w, h)
    }
}

/// A pool of shared memory a client draws in.
struct Mapping {
    fd: OwnedFd,
    ptr: *mut u8,
    len: usize,
}

// SAFETY: the mapping is only read, under its pool's mutex.
unsafe impl Send for Mapping {}

impl Mapping {
    fn new(fd: OwnedFd, size: i32) -> Mapping {
        let mut mapping = Mapping { fd, ptr: std::ptr::null_mut(), len: 0 };
        mapping.map(size);
        mapping
    }

    fn map(&mut self, size: i32) -> bool {
        if size <= 0 {
            return false;
        }
        // SAFETY: a new read-only shared mapping of the client's file.
        let ptr = unsafe { libc::mmap(std::ptr::null_mut(), size as usize, libc::PROT_READ, libc::MAP_SHARED, self.fd.as_raw_fd(), 0) };
        if ptr == libc::MAP_FAILED {
            return false;
        }
        self.unmap();
        self.ptr = ptr.cast();
        self.len = size as usize;
        true
    }

    fn unmap(&mut self) {
        if !self.ptr.is_null() {
            // SAFETY: the mapping made in `map`.
            unsafe { libc::munmap(self.ptr.cast(), self.len) };
            self.ptr = std::ptr::null_mut();
            self.len = 0;
        }
    }

    /// `len` bytes of the file from `offset`; None if it is not that long now.
    fn read(&self, offset: usize, len: usize) -> Option<Vec<u8>> {
        let mut out = vec![0u8; len];
        let mut got = 0;
        while got < len {
            // SAFETY: into the rest of `out`, from a descriptor this owns.
            let n = unsafe {
                libc::pread(self.fd.as_raw_fd(), out[got..].as_mut_ptr().cast(), len - got, (offset + got) as libc::off_t)
            };
            match n {
                n if n > 0 => got += n as usize,
                -1 if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted => {}
                _ => return None,
            }
        }
        Some(out)
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        self.unmap();
    }
}

struct ShmBuffer {
    pool: Arc<Mutex<Mapping>>,
    offset: usize,
    width: usize,
    height: usize,
    stride: usize,
    opaque: bool,
    valid: bool,
}

/// A buffer's pixels, copied.
fn read_buffer(buffer: &WlBuffer) -> Option<Pixels> {
    let b = buffer.data::<ShmBuffer>().filter(|b| b.valid)?;
    let map = lock(&b.pool);
    let end = b.offset.checked_add(b.stride.checked_mul(b.height - 1)?)?.checked_add(b.width * 4)?;
    if map.ptr.is_null() || end > map.len {
        return None;
    }
    // Read, not looked at through the mapping: a client can shrink its file
    // at any moment, between any check and any look, and reading a mapping
    // past the end of its file ends the program with SIGBUS -- here the
    // desktop, and every window. Reading the file just comes up short.
    let bytes = map.read(b.offset, end - b.offset)?;
    let mut rgba = Vec::with_capacity(b.width * b.height * 4);
    for row in 0..b.height {
        let line = &bytes[row * b.stride..row * b.stride + b.width * 4];
        // In memory B, G, R, A (ARGB8888, little-endian).
        for p in line.chunks_exact(4) {
            rgba.extend_from_slice(&[p[2], p[1], p[0], if b.opaque { 255 } else { p[3] }]);
        }
    }
    Some(Pixels { width: b.width, height: b.height, rgba })
}

/// Draw premultiplied pixels over premultiplied pixels, at (x, y), clipped.
fn blend(out: &mut [u8], ow: usize, oh: usize, src: &Pixels, x: i32, y: i32) {
    let x0 = x.max(0) as usize;
    let y0 = y.max(0) as usize;
    let x1 = (x as i64 + src.width as i64).clamp(0, ow as i64) as usize;
    let y1 = (y as i64 + src.height as i64).clamp(0, oh as i64) as usize;
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let sx0 = (x0 as i64 - x as i64) as usize;
    for oy in y0..y1 {
        let sy = (oy as i64 - y as i64) as usize;
        let s = (sy * src.width + sx0) * 4;
        let src_row = &src.rgba[s..s + (x1 - x0) * 4];
        let out_row = &mut out[(oy * ow + x0) * 4..(oy * ow + x1) * 4];
        for (o, s) in out_row.chunks_exact_mut(4).zip(src_row.chunks_exact(4)) {
            let a = s[3] as u32;
            if a == 255 {
                o.copy_from_slice(s);
            } else if a != 0 {
                for i in 0..4 {
                    o[i] = (s[i] as u32 + (o[i] as u32 * (255 - a) + 127) / 255).min(255) as u8;
                }
            }
        }
    }
}

/// Premultiplied to straight alpha, as canvases' images are.
fn unpremultiply(rgba: &mut [u8]) {
    for p in rgba.chunks_exact_mut(4) {
        let a = p[3] as u32;
        if a != 0 && a != 255 {
            for c in &mut p[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
}

/// The keyboard map, in a file of its own for a client to map.
fn keymap_fd() -> Option<OwnedFd> {
    // SAFETY: a new memfd, owned from here on.
    let fd = unsafe { libc::memfd_create(c"zipp-keymap".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return None;
    }
    // SAFETY: just made, and no one else's.
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.write_all(KEYMAP).ok()?;
    file.write_all(&[0]).ok()?;
    Some(file.into())
}

struct State {
    dh: DisplayHandle,
    started: Instant,
    serial: u32,
    screen: (i32, i32),
    surfaces: HashMap<ObjectId, Surface>,
    windows: HashMap<u32, Window>,
    next_window: u32,
    pointers: Vec<WlPointer>,
    keyboards: Vec<WlKeyboard>,
    /// The surface the pointer is over.
    pointer_on: Option<ObjectId>,
    /// The window the keys go to.
    keyboard_on: Option<u32>,
    keys: Vec<u32>,
    /// Modifiers held (Shift 1, Control 4, Alt 8, Super 64) and locked (Caps 2, Num 16).
    depressed: u32,
    locked: u32,
    /// Frame callbacks to answer at `tick`.
    callbacks: Vec<WlCallback>,
    tick: Option<Instant>,
    clients: Vec<ClientId>,
}

impl State {
    fn new(dh: DisplayHandle, screen: (i32, i32)) -> State {
        State {
            dh,
            started: Instant::now(),
            serial: 0,
            screen,
            surfaces: HashMap::new(),
            windows: HashMap::new(),
            next_window: 0,
            pointers: Vec::new(),
            keyboards: Vec::new(),
            pointer_on: None,
            keyboard_on: None,
            keys: Vec::new(),
            depressed: 0,
            locked: 0,
            callbacks: Vec::new(),
            tick: None,
            clients: Vec::new(),
        }
    }

    fn next_serial(&mut self) -> u32 {
        self.serial = self.serial.wrapping_add(1);
        self.serial
    }

    fn time(&self) -> u32 {
        self.started.elapsed().as_millis() as u32
    }

    fn command(&mut self, command: Command) {
        match command {
            Command::Configure { window, width, height, active, fullscreen } => {
                if let Some(w) = self.windows.get_mut(&window) {
                    w.want = (width.clamp(0, 16384), height.clamp(0, 16384), active);
                    w.fullscreen = fullscreen;
                    if w.configured && w.toplevel.is_some() {
                        self.configure(window);
                    }
                }
            }
            Command::Pointer { window, x, y } => self.pointer(window, x, y),
            Command::Button { button, pressed } => self.button(button, pressed),
            Command::Axis { dx, dy } => self.axis(dx, dy),
            Command::Key { window, key, pressed } => self.key(window, key, pressed),
            Command::Focus { window } => self.focus(window),
            Command::Close { window } => {
                if let Some(w) = self.windows.get(&window) {
                    if let Some(toplevel) = &w.toplevel {
                        toplevel.close();
                    } else if let Some(popup) = &w.popup {
                        popup.popup_done();
                    }
                }
            }
            Command::Reset => {
                for client in std::mem::take(&mut self.clients) {
                    self.dh.backend_handle().kill_client(client, DisconnectReason::ConnectionClosed);
                }
            }
        }
    }

    // ---- surfaces and windows

    fn commit(&mut self, id: &ObjectId) {
        let Some(s) = self.surfaces.get_mut(id) else {
            return;
        };
        if let Some(buffer) = s.buffer.take() {
            s.image = buffer.as_ref().and_then(read_buffer);
            // Copied: the app may draw in it again.
            if let Some(buffer) = buffer {
                buffer.release();
            }
        }
        if !s.callbacks.is_empty() {
            self.callbacks.append(&mut s.callbacks);
            self.tick.get_or_insert_with(|| Instant::now() + FRAME);
        }
        if let Some(window) = self.window_of(id) {
            self.update(window);
        }
    }

    /// The window a surface is part of.
    fn window_of(&self, id: &ObjectId) -> Option<u32> {
        let mut id = id.clone();
        for _ in 0..16 {
            match &self.surfaces.get(&id)?.role {
                Role::Window(window) => return Some(*window),
                Role::Sub { parent, .. } => id = parent.clone(),
                Role::None => return None,
            }
        }
        None
    }

    /// A window's surface changed: its first configure, or a new frame.
    fn update(&mut self, window: u32) {
        let Some((configured, surface)) = self.windows.get(&window).map(|w| (w.configured, w.surface.clone())) else {
            return;
        };
        if !configured {
            self.configure(window);
        }
        let Some((width, height, rgba, origin)) = self.compose(&surface) else {
            return;
        };
        let Some(w) = self.windows.get_mut(&window) else {
            return;
        };
        w.origin = origin;
        // The process it is, for the page to know its own (a browser it ran).
        let pid = w.xdg.client().and_then(|c| c.get_credentials(&self.dh).ok()).map_or(0, |c| c.pid);
        match w.shown {
            None => event(
                "window",
                window,
                serde_json::json!({
                    "pid": pid,
                    "kind": if w.popup.is_some() { "popup" } else { "toplevel" },
                    "parent": w.parent,
                    "x": w.place.0,
                    "y": w.place.1,
                    "title": w.title,
                    "appId": w.app_id,
                    "width": width,
                    "height": height,
                }),
            ),
            Some(size) if size != (width, height) => event("size", window, serde_json::json!({ "width": width, "height": height })),
            _ => {}
        }
        w.shown = Some((width, height));
        lock(&FRAMES).get_or_insert_with(HashMap::new).insert(window, crate::canvas::Frame { flid: 0, width, height, rgba });
        crate::worker::wake();
    }

    /// Tell a window its size and state (and a popup its place): it draws
    /// once it has been told.
    fn configure(&mut self, window: u32) {
        let serial = self.next_serial();
        let Some(w) = self.windows.get_mut(&window) else {
            return;
        };
        w.configured = true;
        if let Some(toplevel) = &w.toplevel {
            let (width, height, active) = w.want;
            let mut states = Vec::new();
            if active {
                states.extend((xdg_toplevel::State::Activated as u32).to_ne_bytes());
            }
            if w.fullscreen {
                states.extend((xdg_toplevel::State::Fullscreen as u32).to_ne_bytes());
            }
            toplevel.configure(width, height, states);
        } else if let Some(popup) = &w.popup {
            let (x, y, width, height) = w.place;
            popup.configure(x, y, width, height);
        }
        w.xdg.configure(serial);
    }

    /// A window's frame: its surface and subsurfaces, cut to its geometry.
    fn compose(&self, root: &ObjectId) -> Option<(u32, u32, Vec<u8>, (i32, i32))> {
        let s = self.surfaces.get(root)?;
        let image = s.image.as_ref()?;
        let (gx, gy, gw, gh) = match s.geometry {
            Some(g) if g.2 > 0 && g.3 > 0 => g,
            _ => (0, 0, image.width as i32, image.height as i32),
        };
        let (gw, gh) = (gw.min(8192) as usize, gh.min(8192) as usize);
        let mut out = vec![0u8; gw * gh * 4];
        self.draw(&mut out, gw, gh, root, -gx, -gy, 0);
        unpremultiply(&mut out);
        Some((gw as u32, gh as u32, out, (gx, gy)))
    }

    fn draw(&self, out: &mut [u8], ow: usize, oh: usize, id: &ObjectId, x: i32, y: i32, depth: u32) {
        let Some(s) = self.surfaces.get(id) else {
            return;
        };
        if let Some(image) = &s.image {
            blend(out, ow, oh, image, x, y);
        }
        if depth >= 8 {
            return;
        }
        for child in &s.children {
            if let Some(Role::Sub { x: cx, y: cy, .. }) = self.surfaces.get(child).map(|c| &c.role) {
                self.draw(out, ow, oh, child, x + cx, y + cy, depth + 1);
            }
        }
    }

    fn new_window(&mut self, surface: &ObjectId, xdg: &XdgSurface, parent: u32, place: (i32, i32, i32, i32)) -> u32 {
        self.next_window += 1;
        let window = self.next_window;
        if let Some(s) = self.surfaces.get_mut(surface) {
            s.role = Role::Window(window);
        }
        self.windows.insert(
            window,
            Window {
                surface: surface.clone(),
                xdg: xdg.clone(),
                toplevel: None,
                popup: None,
                parent,
                place,
                title: String::new(),
                app_id: String::new(),
                configured: false,
                shown: None,
                want: (0, 0, true),
                fullscreen: false,
                origin: (0, 0),
            },
        );
        window
    }

    fn window_gone(&mut self, window: u32) {
        let Some(w) = self.windows.remove(&window) else {
            return;
        };
        if let Some(s) = self.surfaces.get_mut(&w.surface)
            && matches!(s.role, Role::Window(x) if x == window)
        {
            s.role = Role::None;
        }
        if self.keyboard_on == Some(window) {
            self.keyboard_on = None;
        }
        if let Some(frames) = lock(&FRAMES).as_mut() {
            frames.remove(&window);
        }
        if w.shown.is_some() {
            event("gone", window, serde_json::json!({}));
        }
    }

    fn surface_gone(&mut self, id: &ObjectId) {
        let Some(s) = self.surfaces.remove(id) else {
            return;
        };
        for child in &s.children {
            if let Some(c) = self.surfaces.get_mut(child) {
                c.role = Role::None;
            }
        }
        if self.pointer_on.as_ref() == Some(id) {
            self.pointer_on = None;
        }
        match s.role {
            Role::Window(window) => self.window_gone(window),
            Role::Sub { parent, .. } => {
                if let Some(p) = self.surfaces.get_mut(&parent) {
                    p.children.retain(|c| c != id);
                }
                if let Some(window) = self.window_of(&parent) {
                    self.update(window);
                }
            }
            Role::None => {}
        }
    }

    fn set_title(&mut self, window: u32, title: Option<String>, app_id: Option<String>) {
        let Some(w) = self.windows.get_mut(&window) else {
            return;
        };
        if let Some(title) = title {
            w.title = title.chars().take(200).collect();
        }
        if let Some(app_id) = app_id {
            w.app_id = app_id.chars().take(200).collect();
        }
        if w.shown.is_some() {
            event("title", window, serde_json::json!({ "title": w.title, "appId": w.app_id }));
        }
    }

    /// Whether `parent` may take `child` as a subsurface.
    fn can_hold(&self, parent: &ObjectId, child: &ObjectId) -> bool {
        let Some(c) = self.surfaces.get(child) else {
            return false;
        };
        if !matches!(c.role, Role::None) || self.surfaces.len() >= MAX_SURFACES {
            return false;
        }
        let mut at = Some(parent.clone());
        let mut depth = 0;
        while let Some(id) = at {
            if id == *child || depth > 16 {
                return false;
            }
            depth += 1;
            at = match self.surfaces.get(&id).map(|s| &s.role) {
                Some(Role::Sub { parent, .. }) => Some(parent.clone()),
                _ => None,
            };
        }
        true
    }

    fn window_surface(&self, window: u32) -> Option<WlSurface> {
        let w = self.windows.get(&window)?;
        Some(self.surfaces.get(&w.surface)?.wl.clone())
    }

    // ---- the pointer and the keys

    fn pointers_of(&self, surface: &WlSurface) -> Vec<WlPointer> {
        self.pointers.iter().filter(|p| p.id().same_client_as(&surface.id())).cloned().collect()
    }

    fn keyboards_of(&self, surface: &WlSurface) -> Vec<WlKeyboard> {
        self.keyboards.iter().filter(|k| k.id().same_client_as(&surface.id())).cloned().collect()
    }

    /// The surface under (x, y) of a window's frame, and the point in it.
    fn surface_at(&self, window: u32, x: f64, y: f64) -> Option<(ObjectId, f64, f64)> {
        let w = self.windows.get(&window)?;
        let (x, y) = (x + w.origin.0 as f64, y + w.origin.1 as f64);
        Some(self.hit(&w.surface, x, y, 0).unwrap_or((w.surface.clone(), x, y)))
    }

    fn hit(&self, id: &ObjectId, x: f64, y: f64, depth: u32) -> Option<(ObjectId, f64, f64)> {
        let s = self.surfaces.get(id)?;
        if depth < 8 {
            for child in s.children.iter().rev() {
                if let Some(Role::Sub { x: cx, y: cy, .. }) = self.surfaces.get(child).map(|c| &c.role)
                    && let Some(hit) = self.hit(child, x - *cx as f64, y - *cy as f64, depth + 1)
                {
                    return Some(hit);
                }
            }
        }
        let image = s.image.as_ref()?;
        (x >= 0.0 && y >= 0.0 && x < image.width as f64 && y < image.height as f64).then(|| (id.clone(), x, y))
    }

    fn pointer(&mut self, window: u32, x: f64, y: f64) {
        let target = if window == 0 { None } else { self.surface_at(window, x, y) };
        let time = self.time();
        let over = target.as_ref().map(|t| t.0.clone());
        if self.pointer_on != over {
            if let Some(old) = self.pointer_on.take()
                && let Some(wl) = self.surfaces.get(&old).map(|s| s.wl.clone())
            {
                let serial = self.next_serial();
                for p in self.pointers_of(&wl) {
                    p.leave(serial, &wl);
                    if p.version() >= 5 {
                        p.frame();
                    }
                }
            }
            if let Some((id, sx, sy)) = &target
                && let Some(wl) = self.surfaces.get(id).map(|s| s.wl.clone())
            {
                let serial = self.next_serial();
                for p in self.pointers_of(&wl) {
                    p.enter(serial, &wl, *sx, *sy);
                    if p.version() >= 5 {
                        p.frame();
                    }
                }
                self.pointer_on = Some(id.clone());
            }
            return;
        }
        if let Some((id, sx, sy)) = target
            && let Some(wl) = self.surfaces.get(&id).map(|s| s.wl.clone())
        {
            for p in self.pointers_of(&wl) {
                p.motion(time, sx, sy);
                if p.version() >= 5 {
                    p.frame();
                }
            }
        }
    }

    fn button(&mut self, button: u32, pressed: bool) {
        let Some(wl) = self.pointer_on.as_ref().and_then(|id| self.surfaces.get(id)).map(|s| s.wl.clone()) else {
            return;
        };
        let serial = self.next_serial();
        let time = self.time();
        let state = if pressed { wl_pointer::ButtonState::Pressed } else { wl_pointer::ButtonState::Released };
        for p in self.pointers_of(&wl) {
            p.button(serial, time, button, state);
            if p.version() >= 5 {
                p.frame();
            }
        }
    }

    /// The wheel: the page counts a notch as 100 pixels; Wayland as 10.
    fn axis(&mut self, dx: f64, dy: f64) {
        let Some(wl) = self.pointer_on.as_ref().and_then(|id| self.surfaces.get(id)).map(|s| s.wl.clone()) else {
            return;
        };
        let time = self.time();
        for p in self.pointers_of(&wl) {
            if p.version() >= 5 {
                p.axis_source(wl_pointer::AxisSource::Wheel);
            }
            for (axis, d) in [(wl_pointer::Axis::VerticalScroll, dy), (wl_pointer::Axis::HorizontalScroll, dx)] {
                if d == 0.0 {
                    continue;
                }
                if (5..8).contains(&p.version()) {
                    let steps = (d / 100.0).round() as i32;
                    p.axis_discrete(axis, if steps == 0 { d.signum() as i32 } else { steps });
                }
                p.axis(time, axis, d / 10.0);
            }
            if p.version() >= 5 {
                p.frame();
            }
        }
    }

    fn focus(&mut self, window: u32) {
        if self.keyboard_on == Some(window) || (window != 0 && !self.windows.contains_key(&window)) {
            return;
        }
        if let Some(old) = self.keyboard_on.take()
            && let Some(wl) = self.window_surface(old)
        {
            let serial = self.next_serial();
            for k in self.keyboards_of(&wl) {
                k.leave(serial, &wl);
            }
        }
        if let Some(wl) = self.window_surface(window) {
            let serial = self.next_serial();
            for k in self.keyboards_of(&wl) {
                k.enter(serial, &wl, Vec::new());
                k.modifiers(serial, self.depressed, 0, self.locked, 0);
            }
            self.keyboard_on = Some(window);
        }
    }

    fn key(&mut self, window: u32, key: u32, pressed: bool) {
        if self.keyboard_on != Some(window) {
            self.focus(window);
        }
        let Some(wl) = self.window_surface(window) else {
            return;
        };
        if pressed {
            if self.keys.contains(&key) {
                return;
            }
            self.keys.push(key);
        } else {
            if !self.keys.contains(&key) {
                return;
            }
            self.keys.retain(|k| *k != key);
        }
        let before = (self.depressed, self.locked);
        self.depressed = self
            .keys
            .iter()
            .map(|k| match k {
                42 | 54 => 1,
                29 | 97 => 4,
                56 | 100 => 8,
                125 | 126 => 64,
                _ => 0,
            })
            .fold(0, |a, b| a | b);
        if pressed && key == 58 {
            self.locked ^= 2;
        }
        if pressed && key == 69 {
            self.locked ^= 16;
        }
        let serial = self.next_serial();
        let time = self.time();
        let state = if pressed { wl_keyboard::KeyState::Pressed } else { wl_keyboard::KeyState::Released };
        for k in self.keyboards_of(&wl) {
            k.key(serial, time, key, state);
            if (self.depressed, self.locked) != before {
                k.modifiers(serial, self.depressed, 0, self.locked, 0);
            }
        }
    }

    fn new_keyboard(&mut self, keyboard: WlKeyboard) {
        if let Some(fd) = keymap_fd() {
            keyboard.keymap(wl_keyboard::KeymapFormat::XkbV1, fd.as_fd(), KEYMAP.len() as u32 + 1);
        }
        if keyboard.version() >= 4 {
            keyboard.repeat_info(30, 500);
        }
        // Its client's window has the keys already: say so.
        if let Some(wl) = self.keyboard_on.and_then(|w| self.window_surface(w))
            && wl.id().same_client_as(&keyboard.id())
        {
            let serial = self.next_serial();
            keyboard.enter(serial, &wl, Vec::new());
            keyboard.modifiers(serial, self.depressed, 0, self.locked, 0);
        }
        self.keyboards.push(keyboard);
    }

    fn answer_frames(&mut self) {
        self.tick = None;
        let time = self.time();
        for callback in std::mem::take(&mut self.callbacks) {
            if callback.is_alive() {
                callback.done(time);
            }
        }
    }
}

// ---- the protocol -------------------------------------------------------------------------

/// Globals that are only made.
macro_rules! plain_globals {
    ($($interface:ty),*) => {$(
        impl GlobalDispatch<$interface, ()> for State {
            fn bind(_: &mut Self, _: &DisplayHandle, _: &Client, resource: New<$interface>, _: &(), init: &mut DataInit<'_, Self>) {
                init.init(resource, ());
            }
        }
    )*};
}
plain_globals!(WlCompositor, WlSubcompositor, XdgWmBase, ZxdgDecorationManagerV1, WlDataDeviceManager);

/// Objects whose requests change nothing here.
macro_rules! ignored {
    ($($interface:ty),*) => {$(
        impl Dispatch<$interface, ()> for State {
            fn request(_: &mut Self, _: &Client, _: &$interface, _: <$interface as Resource>::Request, _: &(), _: &DisplayHandle, _: &mut DataInit<'_, Self>) {}
        }
    )*};
}
ignored!(WlRegion, WlCallback, WlTouch, WlOutput, WlDataSource, WlDataDevice);

impl Dispatch<WlCompositor, ()> for State {
    fn request(state: &mut Self, _: &Client, _: &WlCompositor, request: wl_compositor::Request, _: &(), _: &DisplayHandle, init: &mut DataInit<'_, Self>) {
        match request {
            wl_compositor::Request::CreateSurface { id } => {
                let wl = init.init(id, ());
                state.surfaces.insert(
                    wl.id(),
                    Surface { wl, buffer: None, callbacks: Vec::new(), image: None, role: Role::None, children: Vec::new(), geometry: None },
                );
            }
            wl_compositor::Request::CreateRegion { id } => {
                init.init(id, ());
            }
            _ => {}
        }
    }
}

impl Dispatch<WlSurface, ()> for State {
    fn request(state: &mut Self, _: &Client, surface: &WlSurface, request: wl_surface::Request, _: &(), _: &DisplayHandle, init: &mut DataInit<'_, Self>) {
        let id = surface.id();
        match request {
            wl_surface::Request::Attach { buffer, .. } => {
                if let Some(s) = state.surfaces.get_mut(&id) {
                    s.buffer = Some(buffer);
                }
            }
            wl_surface::Request::Frame { callback } => {
                let callback = init.init(callback, ());
                if let Some(s) = state.surfaces.get_mut(&id) {
                    s.callbacks.push(callback);
                }
            }
            wl_surface::Request::Commit => state.commit(&id),
            _ => {}
        }
    }

    fn destroyed(state: &mut Self, _: ClientId, surface: &WlSurface, _: &()) {
        state.surface_gone(&surface.id());
    }
}

impl Dispatch<WlSubcompositor, ()> for State {
    fn request(state: &mut Self, _: &Client, _: &WlSubcompositor, request: wl_subcompositor::Request, _: &(), _: &DisplayHandle, init: &mut DataInit<'_, Self>) {
        if let wl_subcompositor::Request::GetSubsurface { id, surface, parent } = request {
            let (child, parent) = (surface.id(), parent.id());
            init.init(id, child.clone());
            // A tree: a surface already placed, or one under itself (drawing
            // that would go round for as long as a client liked, with the
            // desktop waiting), gets no place. It is the client's error.
            if !state.can_hold(&parent, &child) {
                return;
            }
            if let Some(s) = state.surfaces.get_mut(&child) {
                s.role = Role::Sub { parent: parent.clone(), x: 0, y: 0 };
            }
            if let Some(p) = state.surfaces.get_mut(&parent) {
                p.children.push(child);
            }
        }
    }
}

impl Dispatch<WlSubsurface, ObjectId> for State {
    fn request(state: &mut Self, _: &Client, _: &WlSubsurface, request: wl_subsurface::Request, child: &ObjectId, _: &DisplayHandle, _: &mut DataInit<'_, Self>) {
        if let wl_subsurface::Request::SetPosition { x, y } = request
            && let Some(Surface { role: Role::Sub { x: sx, y: sy, .. }, .. }) = state.surfaces.get_mut(child)
        {
            *sx = x;
            *sy = y;
        }
    }

    fn destroyed(state: &mut Self, _: ClientId, _: &WlSubsurface, child: &ObjectId) {
        let Some(s) = state.surfaces.get_mut(child) else {
            return;
        };
        if let Role::Sub { parent, .. } = std::mem::replace(&mut s.role, Role::None) {
            if let Some(p) = state.surfaces.get_mut(&parent) {
                p.children.retain(|c| c != child);
            }
            if let Some(window) = state.window_of(&parent) {
                state.update(window);
            }
        }
    }
}

impl GlobalDispatch<WlShm, ()> for State {
    fn bind(_: &mut Self, _: &DisplayHandle, _: &Client, resource: New<WlShm>, _: &(), init: &mut DataInit<'_, Self>) {
        let shm = init.init(resource, ());
        shm.format(wl_shm::Format::Argb8888);
        shm.format(wl_shm::Format::Xrgb8888);
    }
}

impl Dispatch<WlShm, ()> for State {
    fn request(_: &mut Self, _: &Client, _: &WlShm, request: wl_shm::Request, _: &(), _: &DisplayHandle, init: &mut DataInit<'_, Self>) {
        if let wl_shm::Request::CreatePool { id, fd, size } = request {
            let mapping = Mapping::new(fd, size);
            let mapped = !mapping.ptr.is_null();
            let pool = init.init(id, Arc::new(Mutex::new(mapping)));
            if !mapped {
                pool.post_error(wl_shm::Error::InvalidFd, "the pool cannot be mapped");
            }
        }
    }
}

impl Dispatch<WlShmPool, Arc<Mutex<Mapping>>> for State {
    fn request(_: &mut Self, _: &Client, pool: &WlShmPool, request: wl_shm_pool::Request, data: &Arc<Mutex<Mapping>>, _: &DisplayHandle, init: &mut DataInit<'_, Self>) {
        match request {
            wl_shm_pool::Request::CreateBuffer { id, offset, width, height, stride, format } => {
                let opaque = matches!(format, WEnum::Value(wl_shm::Format::Xrgb8888));
                let known = opaque || matches!(format, WEnum::Value(wl_shm::Format::Argb8888));
                let fits = offset >= 0
                    && width > 0
                    && height > 0
                    && width <= 16384
                    && height <= 16384
                    && stride as i64 >= width as i64 * 4
                    && offset as i64 + stride as i64 * height as i64 <= lock(data).len as i64;
                let valid = known && fits;
                init.init(
                    id,
                    ShmBuffer {
                        pool: data.clone(),
                        offset: offset.max(0) as usize,
                        width: width.max(1) as usize,
                        height: height.max(1) as usize,
                        stride: stride.max(0) as usize,
                        opaque,
                        valid,
                    },
                );
                if !known {
                    pool.post_error(wl_shm::Error::InvalidFormat, "only ARGB8888 and XRGB8888");
                } else if !fits {
                    pool.post_error(wl_shm::Error::InvalidStride, "the buffer does not fit its pool");
                }
            }
            wl_shm_pool::Request::Resize { size } => {
                if !lock(data).map(size) {
                    pool.post_error(wl_shm::Error::InvalidFd, "the pool cannot be mapped");
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<WlBuffer, ShmBuffer> for State {
    fn request(_: &mut Self, _: &Client, _: &WlBuffer, _: wayland_server::protocol::wl_buffer::Request, _: &ShmBuffer, _: &DisplayHandle, _: &mut DataInit<'_, Self>) {}
}

impl GlobalDispatch<WlSeat, ()> for State {
    fn bind(_: &mut Self, _: &DisplayHandle, _: &Client, resource: New<WlSeat>, _: &(), init: &mut DataInit<'_, Self>) {
        let seat = init.init(resource, ());
        seat.capabilities(wl_seat::Capability::Pointer | wl_seat::Capability::Keyboard);
        if seat.version() >= 2 {
            seat.name("seat0".into());
        }
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn request(state: &mut Self, _: &Client, _: &WlSeat, request: wl_seat::Request, _: &(), _: &DisplayHandle, init: &mut DataInit<'_, Self>) {
        match request {
            wl_seat::Request::GetPointer { id } => {
                let pointer = init.init(id, ());
                state.pointers.push(pointer);
            }
            wl_seat::Request::GetKeyboard { id } => {
                let keyboard = init.init(id, ());
                state.new_keyboard(keyboard);
            }
            wl_seat::Request::GetTouch { id } => {
                init.init(id, ());
            }
            _ => {}
        }
    }
}

impl Dispatch<WlPointer, ()> for State {
    fn request(_: &mut Self, _: &Client, _: &WlPointer, _: wl_pointer::Request, _: &(), _: &DisplayHandle, _: &mut DataInit<'_, Self>) {}

    fn destroyed(state: &mut Self, _: ClientId, pointer: &WlPointer, _: &()) {
        state.pointers.retain(|p| p.id() != pointer.id());
    }
}

impl Dispatch<WlKeyboard, ()> for State {
    fn request(_: &mut Self, _: &Client, _: &WlKeyboard, _: wl_keyboard::Request, _: &(), _: &DisplayHandle, _: &mut DataInit<'_, Self>) {}

    fn destroyed(state: &mut Self, _: ClientId, keyboard: &WlKeyboard, _: &()) {
        state.keyboards.retain(|k| k.id() != keyboard.id());
    }
}

impl GlobalDispatch<WlOutput, ()> for State {
    fn bind(state: &mut Self, _: &DisplayHandle, _: &Client, resource: New<WlOutput>, _: &(), init: &mut DataInit<'_, Self>) {
        let output = init.init(resource, ());
        let (width, height) = state.screen;
        output.geometry(0, 0, 0, 0, wl_output::Subpixel::Unknown, "ZIPP".into(), "ZIPP OS".into(), wl_output::Transform::Normal);
        output.mode(wl_output::Mode::Current | wl_output::Mode::Preferred, width, height, 60000);
        if output.version() >= 4 {
            output.name("ZIPP-1".into());
        }
        if output.version() >= 2 {
            output.scale(1);
            output.done();
        }
    }
}

impl Dispatch<XdgWmBase, ()> for State {
    fn request(_: &mut Self, _: &Client, _: &XdgWmBase, request: xdg_wm_base::Request, _: &(), _: &DisplayHandle, init: &mut DataInit<'_, Self>) {
        match request {
            xdg_wm_base::Request::CreatePositioner { id } => {
                init.init(id, Mutex::new(Positioner::default()));
            }
            xdg_wm_base::Request::GetXdgSurface { id, surface } => {
                init.init(id, surface.id());
            }
            _ => {}
        }
    }
}

impl Dispatch<XdgPositioner, Mutex<Positioner>> for State {
    fn request(_: &mut Self, _: &Client, _: &XdgPositioner, request: xdg_positioner::Request, data: &Mutex<Positioner>, _: &DisplayHandle, _: &mut DataInit<'_, Self>) {
        let mut p = lock(data);
        match request {
            xdg_positioner::Request::SetSize { width, height } => p.size = (width, height),
            xdg_positioner::Request::SetAnchorRect { x, y, width, height } => p.anchor_rect = (x, y, width, height),
            xdg_positioner::Request::SetAnchor { anchor: WEnum::Value(anchor) } => p.anchor = anchor as u32,
            xdg_positioner::Request::SetGravity { gravity: WEnum::Value(gravity) } => p.gravity = gravity as u32,
            xdg_positioner::Request::SetOffset { x, y } => p.offset = (x, y),
            _ => {}
        }
    }
}

impl Dispatch<XdgSurface, ObjectId> for State {
    fn request(state: &mut Self, _: &Client, xdg: &XdgSurface, request: xdg_surface::Request, surface: &ObjectId, _: &DisplayHandle, init: &mut DataInit<'_, Self>) {
        match request {
            xdg_surface::Request::GetToplevel { id } => {
                let window = state.new_window(surface, xdg, 0, (0, 0, 0, 0));
                let toplevel = init.init(id, window);
                if let Some(w) = state.windows.get_mut(&window) {
                    w.toplevel = Some(toplevel);
                }
            }
            xdg_surface::Request::GetPopup { id, parent, positioner } => {
                let parent = parent.and_then(|p| state.windows.iter().find(|(_, w)| w.xdg.id() == p.id()).map(|(&n, _)| n)).unwrap_or(0);
                let place = positioner.data::<Mutex<Positioner>>().map(|p| lock(p).place()).unwrap_or_default();
                let window = state.new_window(surface, xdg, parent, place);
                let popup = init.init(id, window);
                if let Some(w) = state.windows.get_mut(&window) {
                    w.popup = Some(popup);
                }
            }
            xdg_surface::Request::SetWindowGeometry { x, y, width, height } => {
                if let Some(s) = state.surfaces.get_mut(surface) {
                    s.geometry = Some((x, y, width, height));
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<XdgToplevel, u32> for State {
    fn request(state: &mut Self, _: &Client, _: &XdgToplevel, request: xdg_toplevel::Request, window: &u32, _: &DisplayHandle, _: &mut DataInit<'_, Self>) {
        match request {
            xdg_toplevel::Request::SetTitle { title } => state.set_title(*window, Some(title), None),
            xdg_toplevel::Request::SetAppId { app_id } => state.set_title(*window, None, Some(app_id)),
            // What the app asks of its window, for the page to do.
            xdg_toplevel::Request::SetFullscreen { .. } => event("fullscreen", *window, serde_json::json!({ "on": true })),
            xdg_toplevel::Request::UnsetFullscreen => event("fullscreen", *window, serde_json::json!({ "on": false })),
            xdg_toplevel::Request::SetMinimized => event("minimize", *window, serde_json::json!({})),
            xdg_toplevel::Request::SetMaximized => event("maximize", *window, serde_json::json!({ "on": true })),
            xdg_toplevel::Request::UnsetMaximized => event("maximize", *window, serde_json::json!({ "on": false })),
            _ => {}
        }
    }

    fn destroyed(state: &mut Self, _: ClientId, _: &XdgToplevel, window: &u32) {
        state.window_gone(*window);
    }
}

impl Dispatch<XdgPopup, u32> for State {
    fn request(state: &mut Self, _: &Client, popup: &XdgPopup, request: xdg_popup::Request, window: &u32, _: &DisplayHandle, _: &mut DataInit<'_, Self>) {
        if let xdg_popup::Request::Reposition { positioner, token } = request {
            let place = positioner.data::<Mutex<Positioner>>().map(|p| lock(p).place()).unwrap_or_default();
            let Some(w) = state.windows.get_mut(window) else {
                return;
            };
            w.place = place;
            if w.shown.is_some() {
                event("place", *window, serde_json::json!({ "x": place.0, "y": place.1 }));
            }
            popup.repositioned(token);
            state.configure(*window);
        }
    }

    fn destroyed(state: &mut Self, _: ClientId, _: &XdgPopup, window: &u32) {
        state.window_gone(*window);
    }
}

impl Dispatch<ZxdgDecorationManagerV1, ()> for State {
    fn request(_: &mut Self, _: &Client, _: &ZxdgDecorationManagerV1, request: zxdg_decoration_manager_v1::Request, _: &(), _: &DisplayHandle, init: &mut DataInit<'_, Self>) {
        if let zxdg_decoration_manager_v1::Request::GetToplevelDecoration { id, toplevel } = request {
            let window = toplevel.data::<u32>().copied().unwrap_or(0);
            let decoration = init.init(id, window);
            // ZIPP OS draws the title bar.
            decoration.configure(zxdg_toplevel_decoration_v1::Mode::ServerSide);
        }
    }
}

impl Dispatch<ZxdgToplevelDecorationV1, u32> for State {
    fn request(state: &mut Self, _: &Client, decoration: &ZxdgToplevelDecorationV1, request: zxdg_toplevel_decoration_v1::Request, window: &u32, _: &DisplayHandle, _: &mut DataInit<'_, Self>) {
        if matches!(request, zxdg_toplevel_decoration_v1::Request::SetMode { .. } | zxdg_toplevel_decoration_v1::Request::UnsetMode) {
            decoration.configure(zxdg_toplevel_decoration_v1::Mode::ServerSide);
            if state.windows.get(window).is_some_and(|w| w.configured) {
                state.configure(*window);
            }
        }
    }
}

impl Dispatch<WlDataDeviceManager, ()> for State {
    fn request(_: &mut Self, _: &Client, _: &WlDataDeviceManager, request: wl_data_device_manager::Request, _: &(), _: &DisplayHandle, init: &mut DataInit<'_, Self>) {
        match request {
            wl_data_device_manager::Request::CreateDataSource { id } => {
                init.init(id, ());
            }
            wl_data_device_manager::Request::GetDataDevice { id, .. } => {
                init.init(id, ());
            }
            _ => {}
        }
    }
}
