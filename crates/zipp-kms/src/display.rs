//! The screen: a DRM device, one connected output, and a mode set on it.
//!
//! The modeset is done here, before anything is drawn, rather than left to
//! whatever lit the screen at boot (fbcon, the firmware): softbuffer's KMS
//! backend page-flips onto the CRTC it finds attached to our plane, and a
//! page flip needs a CRTC that is already scanning out a framebuffer of the
//! same format.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};

use drm::buffer::DrmFourcc;
use drm::control::dumbbuffer::DumbBuffer;
use drm::control::{connector, crtc, framebuffer, plane, Device as ControlDevice, Mode, ModeTypeFlags};
use drm::{ClientCapability, Device as BasicDevice};

use crate::pointer::Pointer;

/// An open DRM device node.
#[derive(Debug)]
pub(crate) struct Card(File);

impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}
impl BasicDevice for Card {}
impl ControlDevice for Card {}

/// The output everything is drawn to.
#[derive(Debug)]
pub struct Display {
    pub(crate) card: Card,
    pub(crate) path: PathBuf,
    pub(crate) crtc: crtc::Handle,
    pub(crate) plane: plane::Handle,
    connector: connector::Handle,
    /// The kernel driver's name (`vmwgfx`, `i915`, ...).
    driver: String,
    /// The mode set and what goes with it; a new mode (`set_size`) replaces it.
    screen: Mutex<Screen>,
    pointer: Pointer,
    /// Quarter turns clockwise the picture is turned to stand upright on the
    /// panel (0-3): ZIPP_KMS_ROTATE's, or the panel's own.
    rotation: AtomicU8,
    /// The panel's own: how it is mounted (the kernel knows it per model,
    /// a GPD Win Max's portrait panel lying on its side, say). "auto".
    panel_rotation: u8,
}

#[derive(Debug)]
struct Screen {
    mode: Mode,
    /// The size to draw at: the mode's, except that the width is widened to
    /// the driver's row pitch when that is padded. softbuffer reads a mapped
    /// buffer as rows of exactly `width` pixels.
    size: (u32, u32),
    /// The buffer the CRTC was set to scan out with the mode.
    scanout: (DumbBuffer, framebuffer::Handle),
}

impl Display {
    /// Open the screen: `ZIPP_KMS_DEVICE` if set, otherwise the first
    /// `/dev/dri/card*` with a connected output. `ZIPP_KMS_MODE=1280x800`
    /// picks a mode other than the output's preferred one.
    pub fn open() -> io::Result<Display> {
        if let Some(path) = std::env::var_os("ZIPP_KMS_DEVICE") {
            return Display::open_card(Path::new(&path));
        }
        let mut last_error = io::Error::new(io::ErrorKind::NotFound, "no DRM device with a connected output");
        let mut cards: Vec<PathBuf> = std::fs::read_dir("/dev/dri")?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("card")))
            .collect();
        cards.sort();
        for path in cards {
            match Display::open_card(&path) {
                Ok(display) => return Ok(display),
                Err(e) => {
                    eprintln!("[kms] {}: {e}", path.display());
                    last_error = e;
                }
            }
        }
        Err(last_error)
    }

    fn open_card(path: &Path) -> io::Result<Display> {
        let file = OpenOptions::new().read(true).write(true).custom_flags(libc::O_CLOEXEC).open(path)?;
        let card = Card(file);
        // The primary plane is the "window" softbuffer draws to; it is only
        // listed with universal planes on.
        card.set_client_capability(ClientCapability::UniversalPlanes, true)?;
        // The first opener is master already; asking again is harmless.
        let _ = card.acquire_master_lock();

        let resources = card.resource_handles()?;
        let connector = resources
            .connectors()
            .iter()
            .flat_map(|&c| card.get_connector(c, true))
            .find(|c| c.state() == connector::State::Connected && !c.modes().is_empty())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no connected output"))?;

        let crtc = connector
            .current_encoder()
            .and_then(|e| card.get_encoder(e).ok())
            .and_then(|e| e.crtc())
            .or_else(|| {
                connector
                    .encoders()
                    .iter()
                    .flat_map(|&e| card.get_encoder(e))
                    .flat_map(|e| resources.filter_crtcs(e.possible_crtcs()))
                    .next()
            })
            .ok_or_else(|| io::Error::other("no CRTC for the output"))?;

        // The first mode that can be set: a buffer for it, and the CRTC
        // scanning it out. A mode too big for the graphics memory (a 4K
        // screen on a Raspberry Pi) gives way to the next.
        let mut chosen = None;
        let mut last_error = io::Error::other("the output has no modes");
        for mode in modes_to_try(connector.modes()) {
            let (w, h) = mode.size();
            match set_mode(&card, crtc, connector.handle(), mode) {
                Ok((scanout, fb)) => {
                    chosen = Some((mode, scanout, fb));
                    break;
                }
                Err(e) => {
                    eprintln!("[kms] {w}x{h}@{}: {e}; trying another mode", mode.vrefresh());
                    last_error = e;
                }
            }
        }
        let (mode, scanout, fb) = chosen.ok_or(last_error)?;
        let (mode_w, mode_h) = mode.size();
        let (mode_w, mode_h) = (mode_w as u32, mode_h as u32);
        let pitch = drm::buffer::Buffer::pitch(&scanout);
        let width = drawn_width(&scanout, mode_w);

        // The primary plane: the one now scanning out our framebuffer.
        let planes = card.plane_handles()?;
        let plane = planes
            .iter()
            .copied()
            .find(|&p| card.get_plane(p).is_ok_and(|info| info.crtc() == Some(crtc) && info.framebuffer() == Some(fb)))
            .or_else(|| {
                planes.iter().copied().find(|&p| {
                    card.get_plane(p)
                        .is_ok_and(|info| resources.filter_crtcs(info.possible_crtcs()).contains(&crtc))
                })
            })
            .ok_or_else(|| io::Error::other("no plane for the CRTC"))?;

        let panel_rotation = panel_rotation(&card, connector.handle());
        let rotation = match std::env::var("ZIPP_KMS_ROTATE").ok().as_deref().and_then(rotation_named) {
            Some(r) => r,
            None => panel_rotation,
        };
        let pointer = Pointer::new(&card, crtc, rotation);
        eprintln!(
            "[kms] {}: {} {}x{}@{} (drawn at {}x{}, pitch {pitch}), {} cursor",
            path.display(),
            connector_name(&connector),
            mode_w,
            mode_h,
            mode.vrefresh(),
            width,
            mode_h,
            if pointer.is_hardware() { "hardware" } else { "software" },
        );
        if rotation != 0 || panel_rotation != 0 {
            eprintln!("[kms] turned {} degrees (the panel's own: {})", rotation as u32 * 90, panel_rotation as u32 * 90);
        }

        let driver = card.get_driver().map(|d| d.name().to_string_lossy().into_owned()).unwrap_or_default();
        Ok(Display {
            path: path.to_path_buf(),
            crtc,
            plane,
            connector: connector.handle(),
            driver,
            screen: Mutex::new(Screen { mode, size: (width, mode_h), scanout: (scanout, fb) }),
            pointer,
            rotation: AtomicU8::new(rotation),
            panel_rotation,
            card,
        })
    }

    fn screen(&self) -> std::sync::MutexGuard<'_, Screen> {
        self.screen.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The size the application draws at, in pixels: the mode's, or wider
    /// where the driver pads its rows. Turned, the picture's own size (the
    /// renderer turns it onto the panel, padding and all).
    pub fn size(&self) -> (u32, u32) {
        if self.rotation() != 0 {
            return self.visible_size();
        }
        self.screen().size
    }

    /// The size of what is shown: the mode's, turned as the picture is.
    /// Input and layout go by this.
    pub fn visible_size(&self) -> (u32, u32) {
        let (w, h) = self.panel_size();
        if self.rotation() % 2 == 1 { (h, w) } else { (w, h) }
    }

    /// The panel's own size, as the mode has it.
    pub fn panel_size(&self) -> (u32, u32) {
        let (w, h) = self.screen().mode.size();
        (w as u32, h as u32)
    }

    /// The size of the buffers scanned out: the mode's, or wider where the
    /// driver pads its rows.
    pub fn buffer_size(&self) -> (u32, u32) {
        self.screen().size
    }

    /// Quarter turns clockwise the picture is turned by (0-3).
    pub fn rotation(&self) -> u8 {
        self.rotation.load(Ordering::Relaxed)
    }

    /// The panel's own turn: what "auto" gives.
    pub fn panel_rotation(&self) -> u8 {
        self.panel_rotation
    }

    pub(crate) fn set_rotation(&self, rotation: u8) {
        self.rotation.store(rotation % 4, Ordering::Relaxed);
        self.pointer.set_rotation(&self.card, self.crtc, rotation % 4);
    }

    /// Where a point of the picture is on the panel.
    pub(crate) fn to_panel(&self, x: i32, y: i32) -> (i32, i32) {
        let (pw, ph) = self.panel_size();
        let (pw, ph) = (pw as i32, ph as i32);
        match self.rotation() {
            1 => (pw - 1 - y, x),
            2 => (pw - 1 - x, ph - 1 - y),
            3 => (y, ph - 1 - x),
            _ => (x, y),
        }
    }


    /// Show the screen at `width` x `height` from now on: the window a
    /// virtual machine is shown in was resized. The output's own mode of that
    /// size (vmwgfx makes one when told the new layout), or one made here.
    pub(crate) fn set_size(&self, width: u32, height: u32) -> io::Result<()> {
        if self.driver == "vmwgfx" {
            vmw_update_layout(&self.card, width, height);
        }
        let wanted = (width.min(u16::MAX as u32) as u16, height.min(u16::MAX as u32) as u16);
        let info = self.card.get_connector(self.connector, true)?;
        let mode = info
            .modes()
            .iter()
            .filter(|m| m.size() == wanted)
            .max_by_key(|m| (m.mode_type().contains(ModeTypeFlags::PREFERRED), m.vrefresh()))
            .copied()
            .unwrap_or_else(|| made_mode(wanted.0, wanted.1));
        let (scanout, fb) = set_mode(&self.card, self.crtc, self.connector, mode)?;
        let size = (drawn_width(&scanout, wanted.0 as u32), wanted.1 as u32);
        let old = std::mem::replace(&mut *self.screen(), Screen { mode, size, scanout: (scanout, fb) });
        let _ = self.card.destroy_framebuffer(old.scanout.1);
        let _ = self.card.destroy_dumb_buffer(old.scanout.0);
        eprintln!("[kms] {}x{}@{} (drawn at {}x{})", wanted.0, wanted.1, mode.vrefresh(), size.0, size.1);
        Ok(())
    }

    /// The pointer moved to (x, y) of the picture.
    pub(crate) fn pointer_moved(&self, x: i32, y: i32) {
        let (x, y) = self.to_panel(x, y);
        self.pointer.move_to(&self.card, self.crtc, x, y);
    }

    pub(crate) fn hide_pointer(&self) {
        self.pointer.hide(&self.card, self.crtc);
    }

    /// A page flip finished: the new frame is on screen.
    pub(crate) fn frame_presented(&self) {
        self.pointer.frame_presented(&self.card, self.crtc);
    }
}

/// "0", "90", "180", "270" (degrees clockwise) as quarter turns; "auto" (or
/// anything else) as none, for the panel's own.
pub fn rotation_named(name: &str) -> Option<u8> {
    match name.trim() {
        "0" => Some(0),
        "90" => Some(1),
        "180" => Some(2),
        "270" => Some(3),
        _ => None,
    }
}

/// How the panel is mounted, from the connector's "panel orientation"
/// (which the kernel sets from its quirks): the quarter turns clockwise that
/// stand the picture upright on it.
fn panel_rotation(card: &Card, connector: connector::Handle) -> u8 {
    let Ok(props) = card.get_properties(connector) else {
        return 0;
    };
    for (&id, &value) in props.iter() {
        let Ok(info) = card.get_property(id) else {
            continue;
        };
        if info.name().to_bytes() == b"panel orientation" {
            // Normal, Upside Down, Left Side Up, Right Side Up.
            return match value {
                1 => 2,
                2 => 3,
                3 => 1,
                _ => 0,
            };
        }
    }
    0
}

/// A black buffer of the mode's size, scanned out by the CRTC with the mode.
fn set_mode(card: &Card, crtc: crtc::Handle, connector: connector::Handle, mode: Mode) -> io::Result<(DumbBuffer, framebuffer::Handle)> {
    let (w, h) = mode.size();
    let mut scanout = card.create_dumb_buffer((w as u32, h as u32), DrmFourcc::Xrgb8888, 32)?;
    if let Ok(mut map) = card.map_dumb_buffer(&mut scanout) {
        map.as_mut().fill(0);
    }
    let fb = match card.add_framebuffer(&scanout, 24, 32) {
        Ok(fb) => fb,
        Err(e) => {
            let _ = card.destroy_dumb_buffer(scanout);
            return Err(e);
        }
    };
    if let Err(e) = card.set_crtc(crtc, Some(fb), (0, 0), &[connector], Some(mode)) {
        let _ = card.destroy_framebuffer(fb);
        let _ = card.destroy_dumb_buffer(scanout);
        return Err(e);
    }
    Ok((scanout, fb))
}

/// The width to draw at for a buffer shown `width` wide: its row pitch's,
/// where the driver pads rows.
fn drawn_width(scanout: &DumbBuffer, width: u32) -> u32 {
    let pitch = drm::buffer::Buffer::pitch(scanout);
    if pitch.is_multiple_of(4) && pitch / 4 > width { pitch / 4 } else { width }
}

/// A 60 Hz mode of this size, for a virtual screen that takes any (real
/// screens are only given their own).
fn made_mode(width: u16, height: u16) -> Mode {
    let (htotal, vtotal) = (width.saturating_add(160), height.saturating_add(30));
    let mut name = [0; 32];
    for (to, from) in name.iter_mut().zip(format!("{width}x{height}").bytes()) {
        *to = from as std::ffi::c_char;
    }
    Mode::from(drm_ffi::drm_mode_modeinfo {
        clock: (htotal as u32 * vtotal as u32 * 60) / 1000,
        hdisplay: width,
        hsync_start: width.saturating_add(48),
        hsync_end: width.saturating_add(80),
        htotal,
        hskew: 0,
        vdisplay: height,
        vsync_start: height.saturating_add(3),
        vsync_end: height.saturating_add(8),
        vtotal,
        vscan: 0,
        vrefresh: 60,
        flags: 0,
        type_: 1 << 5, // DRM_MODE_TYPE_USERDEF
        name,
    })
}

/// Tell vmwgfx (VirtualBox's VMSVGA, VMware) the screen's new size, which
/// it then offers as the output's preferred mode.
fn vmw_update_layout(card: &Card, width: u32, height: u32) {
    #[repr(C)]
    struct Rect {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
    }
    #[repr(C)]
    struct Layout {
        num_outputs: u32,
        pad64: u32,
        rects: u64,
    }
    let rect = Rect { x: 0, y: 0, w: width, h: height };
    let mut layout = Layout { num_outputs: 1, pad64: 0, rects: &rect as *const Rect as u64 };
    // DRM_IOW(DRM_COMMAND_BASE + DRM_VMW_UPDATE_LAYOUT, struct drm_vmw_update_layout_arg)
    let request = (1u32 << 30) | ((std::mem::size_of::<Layout>() as u32) << 16) | ((b'd' as u32) << 8) | (0x40 + 20);
    // SAFETY: the layout and its one rectangle outlive the call.
    let rc = unsafe { libc::ioctl(card.as_fd().as_raw_fd(), request as _, &mut layout as *mut Layout) };
    if rc != 0 {
        eprintln!("[kms] vmwgfx layout {width}x{height}: {}", io::Error::last_os_error());
    }
}

/// The most pixels a mode is chosen with unless asked for more: every frame
/// is drawn on the CPU, and a 4K one is four times the work of this.
const MOST_PIXELS: u32 = 1920 * 1080;

/// The modes to try, best first. `ZIPP_KMS_MODE=WIDTHxHEIGHT` names one, and
/// `preferred` the output's own; otherwise the output's preferred mode if it
/// is no bigger than 1920x1080, or else the biggest mode that is (at the
/// highest refresh rate). The rest follow, biggest first, should those fail.
fn modes_to_try(modes: &[Mode]) -> Vec<Mode> {
    let pixels = |m: &Mode| m.size().0 as u32 * m.size().1 as u32;
    let preferred = modes.iter().find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED)).copied();
    let mut by_size: Vec<Mode> = modes.to_vec();
    by_size.sort_by_key(|m| std::cmp::Reverse((pixels(m), m.vrefresh())));
    let mut order: Vec<Mode> = Vec::new();
    match std::env::var("ZIPP_KMS_MODE").as_deref() {
        Ok("preferred") => order.extend(preferred),
        Ok(wanted) => {
            let size = wanted.split_once('x').and_then(|(w, h)| Some((w.parse::<u16>().ok()?, h.parse::<u16>().ok()?)));
            match size.and_then(|size| by_size.iter().find(|m| m.size() == size)) {
                Some(mode) => order.push(*mode),
                None => eprintln!("[kms] no {wanted} mode"),
            }
        }
        Err(_) => {}
    }
    if order.is_empty() {
        match preferred.filter(|m| pixels(m) <= MOST_PIXELS) {
            Some(mode) => order.push(mode),
            None => order.extend(by_size.iter().find(|m| pixels(m) <= MOST_PIXELS).copied()),
        }
    }
    order.extend(preferred);
    order.extend(by_size);
    let mut seen = Vec::new();
    order.retain(|m| {
        let key = (m.size(), m.vrefresh(), m.clock());
        if seen.contains(&key) {
            false
        } else {
            seen.push(key);
            true
        }
    });
    order
}

fn connector_name(info: &connector::Info) -> String {
    format!("{:?}-{}", info.interface(), info.interface_id())
}
