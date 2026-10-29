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
        let mode = pick_mode(connector.modes());

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

        let (mode_w, mode_h) = mode.size();
        let (mode_w, mode_h) = (mode_w as u32, mode_h as u32);
        let mut scanout = card.create_dumb_buffer((mode_w, mode_h), DrmFourcc::Xrgb8888, 32)?;
        let pitch = drm::buffer::Buffer::pitch(&scanout);
        let width = if pitch % 4 == 0 && pitch / 4 > mode_w { pitch / 4 } else { mode_w };
        if let Ok(mut map) = card.map_dumb_buffer(&mut scanout) {
            map.as_mut().fill(0);
        }
        let fb = card.add_framebuffer(&scanout, 24, 32)?;
        card.set_crtc(crtc, Some(fb), (0, 0), &[connector.handle()], Some(mode))?;

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

    /// The size the application draws at, in pixels.
    pub fn size(&self) -> (u32, u32) {
        self.size
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

fn pick_mode(modes: &[Mode]) -> Mode {
    if let Ok(wanted) = std::env::var("ZIPP_KMS_MODE") {
        if let Some((w, h)) = wanted.split_once('x').and_then(|(w, h)| Some((w.parse::<u16>().ok()?, h.parse::<u16>().ok()?))) {
            if let Some(mode) = modes.iter().find(|m| m.size() == (w, h)) {
                return *mode;
            }
            eprintln!("[kms] no {wanted} mode; using the preferred one");
        }
    }
    modes
        .iter()
        .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
        .copied()
        .unwrap_or(modes[0])
}

fn connector_name(info: &connector::Info) -> String {
    format!("{:?}-{}", info.interface(), info.interface_id())
}
