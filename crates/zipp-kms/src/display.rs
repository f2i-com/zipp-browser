//! The screen: a DRM device, one connected output, and a mode set on it.
//!
//! The modeset is done here, before anything is drawn, rather than left to
//! whatever lit the screen at boot (fbcon, the firmware): softbuffer's KMS
//! backend page-flips onto the CRTC it finds attached to our plane, and a
//! page flip needs a CRTC that is already scanning out a framebuffer of the
//! same format.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

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
    pub(crate) mode: Mode,
    /// The size to draw at: the mode's, except that the width is widened to
    /// the driver's row pitch when that is padded. softbuffer reads a mapped
    /// buffer as rows of exactly `width` pixels.
    pub(crate) size: (u32, u32),
    pointer: Pointer,
    _scanout: (DumbBuffer, framebuffer::Handle),
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
            let attempt = (|| -> io::Result<(DumbBuffer, framebuffer::Handle)> {
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
                if let Err(e) = card.set_crtc(crtc, Some(fb), (0, 0), &[connector.handle()], Some(mode)) {
                    let _ = card.destroy_framebuffer(fb);
                    let _ = card.destroy_dumb_buffer(scanout);
                    return Err(e);
                }
                Ok((scanout, fb))
            })();
            match attempt {
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
        let width = if pitch % 4 == 0 && pitch / 4 > mode_w { pitch / 4 } else { mode_w };

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

        let pointer = Pointer::new(&card, crtc);
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

        Ok(Display {
            card,
            path: path.to_path_buf(),
            crtc,
            plane,
            mode,
            size: (width, mode_h),
            pointer,
            _scanout: (scanout, fb),
        })
    }

    /// The size the application draws at, in pixels: the mode's, or wider
    /// where the driver pads its rows.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The size of what is shown: the mode's. Input and layout go by this.
    pub fn visible_size(&self) -> (u32, u32) {
        let (w, h) = self.mode.size();
        (w as u32, h as u32)
    }

    pub(crate) fn pointer_moved(&self, x: i32, y: i32) {
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
