//! The mouse pointer. It goes in the GPU's cursor plane where the driver has
//! one (virtio-gpu, most real GPUs); otherwise (simpledrm, bochs) it is drawn
//! by hand into whichever framebuffer is on screen, after each page flip and
//! as it moves, with the pixels under it kept to put back. It stays hidden
//! until a mouse moves, so a touchscreen kiosk never shows one.

use std::collections::HashMap;
use std::os::fd::{AsFd, AsRawFd};
use std::sync::Mutex;

use drm::buffer::DrmFourcc;
use drm::control::dumbbuffer::DumbBuffer;
use drm::control::{ClipRect, Device as ControlDevice, crtc, framebuffer};

use crate::display::Card;

const ARROW: [&str; 19] = [
    "X",
    "XX",
    "X.X",
    "X..X",
    "X...X",
    "X....X",
    "X.....X",
    "X......X",
    "X.......X",
    "X........X",
    "X.........X",
    "X..........X",
    "X......XXXXX",
    "X...X..X",
    "X..XX..X",
    "X.X  X..X",
    "XX   X..X",
    "X     X..X",
    "      XXXX",
];
const ARROW_W: i32 = 12;
const ARROW_H: i32 = ARROW.len() as i32;

/// The arrow's colour at (x, y), as 0xAARRGGBB, where it has one.
fn arrow(x: i32, y: i32) -> Option<u32> {
    match ARROW.get(y as usize)?.as_bytes().get(x as usize)? {
        b'X' => Some(0xff00_0000),
        b'.' => Some(0xffff_ffff),
        _ => None,
    }
}

pub(crate) struct Pointer {
    /// The cursor plane's image, when the driver has a cursor plane.
    hardware: Option<DumbBuffer>,
    state: Mutex<State>,
}

impl std::fmt::Debug for Pointer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pointer").field("hardware", &self.hardware.is_some()).finish_non_exhaustive()
    }
}

#[derive(Default)]
struct State {
    pos: (i32, i32),
    visible: bool,
    /// Software: where the arrow is drawn now, and what it covers.
    drawn: Option<Drawn>,
    /// Software: the framebuffers that have been on screen, mapped.
    maps: HashMap<u32, Mapping>,
}

struct Drawn {
    fb: framebuffer::Handle,
    at: (i32, i32),
    under: Vec<u32>,
}

impl Pointer {
    /// `ZIPP_KMS_SOFTWARE_CURSOR=1` draws the pointer by hand even where
    /// there is a cursor plane.
    pub(crate) fn new(card: &Card, crtc: crtc::Handle) -> Pointer {
        let hardware = match std::env::var_os("ZIPP_KMS_SOFTWARE_CURSOR") {
            Some(_) => None,
            None => hardware_cursor(card, crtc),
        };
        Pointer { hardware, state: Mutex::default() }
    }

    pub(crate) fn is_hardware(&self) -> bool {
        self.hardware.is_some()
    }

    /// The pointer moved to (x, y), and is shown if it was hidden.
    pub(crate) fn move_to(&self, card: &Card, crtc: crtc::Handle, x: i32, y: i32) {
        let mut state = self.state.lock().unwrap();
        state.pos = (x, y);
        #[allow(deprecated)]
        match &self.hardware {
            Some(buffer) => {
                if !state.visible {
                    let _ = card.set_cursor2(crtc, Some(buffer), (0, 0));
                }
                let _ = card.move_cursor(crtc, (x, y));
            }
            None => paint(card, crtc, &mut state),
        }
        state.visible = true;
    }

    pub(crate) fn hide(&self, card: &Card, crtc: crtc::Handle) {
        let mut state = self.state.lock().unwrap();
        if !state.visible {
            return;
        }
        state.visible = false;
        #[allow(deprecated)]
        if self.hardware.is_some() {
            let _ = card.set_cursor2::<DumbBuffer>(crtc, None, (0, 0));
        } else if let Some(drawn) = state.drawn.take() {
            if let Some(map) = state.maps.get(&u32::from(drawn.fb)) {
                map.put(drawn.at, &drawn.under);
                let _ = card.dirty_framebuffer(drawn.fb, &[rect(drawn.at)]);
            }
        }
    }

    /// A new frame is on screen: a drawn pointer goes on it too.
    pub(crate) fn frame_presented(&self, card: &Card, crtc: crtc::Handle) {
        if self.hardware.is_none() {
            let mut state = self.state.lock().unwrap();
            if state.visible {
                paint(card, crtc, &mut state);
            }
        }
    }
}

/// Draw the arrow at the pointer's position on the framebuffer on screen,
/// first putting back what it covered if it was drawn there before.
fn paint(card: &Card, crtc: crtc::Handle, state: &mut State) {
    let Some(fb) = card.get_crtc(crtc).ok().and_then(|c| c.framebuffer()) else { return };
    let pos = state.pos;
    let mut damage = Vec::new();
    if let Some(drawn) = state.drawn.take() {
        // A framebuffer that has left the screen is the renderer's to draw
        // over whole; only the one still showing needs the arrow taken off.
        if drawn.fb == fb {
            if drawn.at == pos {
                state.drawn = Some(drawn);
                return;
            }
            if let Some(map) = state.maps.get(&u32::from(fb)) {
                map.put(drawn.at, &drawn.under);
                damage.push(rect(drawn.at));
            }
        }
    }
    let map = match state.maps.entry(u32::from(fb)) {
        std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
        std::collections::hash_map::Entry::Vacant(e) => match Mapping::new(card, fb) {
            Some(map) => e.insert(map),
            None => return,
        },
    };
    let under = map.take(pos);
    map.draw_arrow(pos);
    damage.push(rect(pos));
    // Drivers that copy to the screen (simpledrm) do so for dirty areas.
    let dirty = card.dirty_framebuffer(fb, &damage);
    if std::env::var_os("ZIPP_KMS_DEBUG").is_some() {
        eprintln!("[kms] pointer drawn at {pos:?} on framebuffer {} ({dirty:?})", u32::from(fb));
    }
    state.drawn = Some(Drawn { fb, at: pos, under });
}

fn rect((x, y): (i32, i32)) -> ClipRect {
    let clamp = |v: i32| v.clamp(0, u16::MAX as i32) as u16;
    ClipRect::new(clamp(x), clamp(y), clamp(x + ARROW_W), clamp(y + ARROW_H))
}

/// A framebuffer's pixels, mapped: 32 bits each, rows `stride` apart.
struct Mapping {
    ptr: *mut u32,
    len: usize,
    stride: usize,
    width: i32,
    height: i32,
}

// SAFETY: the mapping is only touched with the pointer's lock held.
unsafe impl Send for Mapping {}

impl Mapping {
    fn new(card: &Card, fb: framebuffer::Handle) -> Option<Mapping> {
        let fail = |what: &str| eprintln!("[kms] pointer: framebuffer {}: {what}", u32::from(fb));
        let info = card.get_framebuffer(fb).inspect_err(|e| fail(&format!("GETFB: {e}"))).ok()?;
        let Some(handle) = info.buffer() else {
            fail("no buffer handle");
            return None;
        };
        let (width, height) = info.size();
        let pitch = info.pitch() as usize;
        let map = drm_ffi::mode::dumbbuffer::map(card.as_fd(), u32::from(handle), 0, 0)
            .inspect_err(|e| fail(&format!("MAP_DUMB: {e}")))
            .ok()?;
        let len = pitch * height as usize;
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                card.as_fd().as_raw_fd(),
                map.offset as libc::off_t,
            )
        };
        (ptr != libc::MAP_FAILED).then(|| Mapping {
            ptr: ptr.cast(),
            len,
            stride: pitch / 4,
            width: width as i32,
            height: height as i32,
        })
    }

    /// Every on-screen pixel of the arrow's box at `at`, in order.
    fn cells(&self, at: (i32, i32)) -> impl Iterator<Item = (i32, i32, usize)> + '_ {
        (0..ARROW_H).flat_map(move |dy| (0..ARROW_W).map(move |dx| (dx, dy))).filter_map(move |(dx, dy)| {
            let (x, y) = (at.0 + dx, at.1 + dy);
            (x >= 0 && y >= 0 && x < self.width && y < self.height).then(|| (dx, dy, y as usize * self.stride + x as usize))
        })
    }

    fn take(&self, at: (i32, i32)) -> Vec<u32> {
        self.cells(at).map(|(_, _, i)| unsafe { self.ptr.add(i).read_volatile() }).collect()
    }

    fn put(&self, at: (i32, i32), pixels: &[u32]) {
        for ((_, _, i), &px) in self.cells(at).zip(pixels) {
            unsafe { self.ptr.add(i).write_volatile(px) };
        }
    }

    fn draw_arrow(&self, at: (i32, i32)) {
        for (dx, dy, i) in self.cells(at) {
            if let Some(px) = arrow(dx, dy) {
                unsafe { self.ptr.add(i).write_volatile(px) };
            }
        }
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        unsafe { libc::munmap(self.ptr.cast(), self.len) };
    }
}

/// The arrow in a cursor plane, when the driver has one; hidden for now.
fn hardware_cursor(card: &Card, crtc: crtc::Handle) -> Option<DumbBuffer> {
    let mut buffer = card.create_dumb_buffer((64, 64), DrmFourcc::Argb8888, 32).ok()?;
    let pitch = drm::buffer::Buffer::pitch(&buffer) as usize;
    {
        let mut map = card.map_dumb_buffer(&mut buffer).ok()?;
        let bytes = map.as_mut();
        bytes.fill(0);
        for y in 0..ARROW_H {
            for x in 0..ARROW_W {
                if let Some(argb) = arrow(x, y) {
                    let at = y as usize * pitch + x as usize * 4;
                    bytes[at..at + 4].copy_from_slice(&argb.to_le_bytes());
                }
            }
        }
    }
    // Setting it proves the plane is there; it stays hidden until a mouse moves.
    #[allow(deprecated)]
    {
        card.set_cursor2(crtc, Some(&buffer), (0, 0)).ok()?;
        let _ = card.set_cursor2::<DumbBuffer>(crtc, None, (0, 0));
    }
    Some(buffer)
}
