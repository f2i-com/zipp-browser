//! The mouse pointer. It goes in the GPU's cursor plane where the driver has
//! one (virtio-gpu, most real GPUs); otherwise (simpledrm, bochs) it is drawn
//! by hand into whichever framebuffer is on screen, after each page flip and
//! as it moves, with the pixels under it kept to put back. It stays hidden
//! until a mouse moves, so a touchscreen kiosk never shows one.

use std::collections::HashMap;
use std::os::fd::{AsFd, AsRawFd};
use std::sync::{Mutex, OnceLock};

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
const ARROW_MAX_CELLS: usize = (ARROW_W * ARROW_H) as usize;

fn kms_debug() -> bool {
    static DBG: OnceLock<bool> = OnceLock::new();
    *DBG.get_or_init(|| std::env::var_os("ZIPP_KMS_DEBUG").is_some())
}

/// The arrow's colour at (x, y), as 0xAARRGGBB, where it has one.
fn arrow(x: i32, y: i32) -> Option<u32> {
    if x < 0 || y < 0 {
        return None;
    }
    match ARROW.get(y as usize)?.as_bytes().get(x as usize)? {
        b'X' => Some(0xff00_0000),
        b'.' => Some(0xffff_ffff),
        _ => None,
    }
}

/// The arrow turned `rotation` quarter turns clockwise, as the picture is on
/// a panel mounted on its side: its box's size, and where its tip is in it.
fn turned_box(rotation: u8) -> ((i32, i32), (i32, i32)) {
    match rotation % 4 {
        1 => ((ARROW_H, ARROW_W), (ARROW_H - 1, 0)),
        2 => ((ARROW_W, ARROW_H), (ARROW_W - 1, ARROW_H - 1)),
        3 => ((ARROW_H, ARROW_W), (0, ARROW_W - 1)),
        _ => ((ARROW_W, ARROW_H), (0, 0)),
    }
}

/// The turned arrow's colour at (x, y) of its box.
fn turned(rotation: u8, x: i32, y: i32) -> Option<u32> {
    match rotation % 4 {
        1 => arrow(y, ARROW_H - 1 - x),
        2 => arrow(ARROW_W - 1 - x, ARROW_H - 1 - y),
        3 => arrow(ARROW_W - 1 - y, x),
        _ => arrow(x, y),
    }
}

pub(crate) struct Pointer {
    /// The cursor plane's image, when the driver has a cursor plane.
    hardware: Option<Mutex<DumbBuffer>>,
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
    /// Quarter turns clockwise the arrow is drawn turned by.
    rotation: u8,
    visible: bool,
    /// Software: where the arrow is drawn now, and what it covers.
    drawn: Option<Drawn>,
    /// Software: the framebuffers that have been on screen, mapped.
    maps: HashMap<u32, Mapping>,
}

struct Drawn {
    fb: framebuffer::Handle,
    at: (i32, i32),
    under: [u32; ARROW_MAX_CELLS],
    under_len: usize,
}

impl Pointer {
    /// `ZIPP_KMS_SOFTWARE_CURSOR=1` draws the pointer by hand even where
    /// there is a cursor plane.
    pub(crate) fn new(card: &Card, crtc: crtc::Handle, rotation: u8) -> Pointer {
        let hardware = match std::env::var_os("ZIPP_KMS_SOFTWARE_CURSOR") {
            Some(_) => None,
            None => hardware_cursor(card, crtc, rotation),
        };
        Pointer { hardware: hardware.map(Mutex::new), state: Mutex::new(State { rotation, ..State::default() }) }
    }

    /// Drop cached software-cursor framebuffer mmaps (called on modeset/resize).
    pub(crate) fn clear_maps(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.drawn = None;
        state.maps.clear();
    }

    pub(crate) fn is_visible(&self) -> bool {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).visible
    }

    /// The picture is turned another way now: the arrow with it.
    pub(crate) fn set_rotation(&self, card: &Card, crtc: crtc::Handle, rotation: u8) {
        let visible = self.state.lock().unwrap_or_else(|e| e.into_inner()).visible;
        if visible {
            self.hide(card, crtc);
        }
        self.state.lock().unwrap_or_else(|e| e.into_inner()).rotation = rotation;
        if let Some(buffer) = &self.hardware {
            draw_cursor(card, &mut buffer.lock().unwrap_or_else(|e| e.into_inner()), rotation);
        }
    }

    pub(crate) fn is_hardware(&self) -> bool {
        self.hardware.is_some()
    }

    /// The pointer moved to (x, y), and is shown if it was hidden.
    pub(crate) fn move_to(&self, card: &Card, crtc: crtc::Handle, x: i32, y: i32) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.pos = (x, y);
        #[allow(deprecated)]
        match &self.hardware {
            Some(buffer) => {
                let (_, tip) = turned_box(state.rotation);
                if !state.visible {
                    let _ = card.set_cursor2(crtc, Some(&*buffer.lock().unwrap_or_else(|e| e.into_inner())), tip);
                }
                let _ = card.move_cursor(crtc, (x, y));
            }
            None => paint(card, crtc, &mut state),
        }
        state.visible = true;
    }

    pub(crate) fn hide(&self, card: &Card, crtc: crtc::Handle) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.visible {
            return;
        }
        state.visible = false;
        #[allow(deprecated)]
        if self.hardware.is_some() {
            let _ = card.set_cursor2::<DumbBuffer>(crtc, None, (0, 0));
        } else if let Some(drawn) = state.drawn.take()
            && let Some(map) = state.maps.get(&u32::from(drawn.fb))
        {
            map.put(drawn.at, state.rotation, &drawn.under[..drawn.under_len]);
            let clip = rect(drawn.at, state.rotation, map.width, map.height);
            let _ = card.dirty_framebuffer(drawn.fb, &[clip]);
        }
    }

    /// A new frame is on screen: a drawn pointer goes on it too.
    pub(crate) fn frame_presented(&self, card: &Card, crtc: crtc::Handle) {
        if self.hardware.is_none() {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
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
    let fb_id = u32::from(fb);
    // Where the arrow's box goes: its tip at the pointer.
    let rotation = state.rotation;
    let (_, tip) = turned_box(rotation);
    let pos = (state.pos.0 - tip.0, state.pos.1 - tip.1);
    let mut damage = [ClipRect::new(0, 0, 0, 0); 2];
    let mut damage_len = 0usize;
    if let Some(drawn) = state.drawn.take() {
        // A framebuffer that has left the screen is the renderer's to draw
        // over whole; only the one still showing needs the arrow taken off.
        if drawn.fb == fb {
            if drawn.at == pos {
                state.drawn = Some(drawn);
                return;
            }
            if let Some(map) = state.maps.get(&fb_id) {
                map.put(drawn.at, rotation, &drawn.under[..drawn.under_len]);
                damage[damage_len] = rect(drawn.at, rotation, map.width, map.height);
                damage_len += 1;
            }
        }
    }
    // Cap cached mappings to active scanout buffers so resized/destroyed FBs never leak.
    if !state.maps.contains_key(&fb_id) && state.maps.len() >= 4 {
        state.maps.clear();
    }
    let map = match state.maps.entry(fb_id) {
        std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
        std::collections::hash_map::Entry::Vacant(e) => match Mapping::new(card, fb) {
            Some(map) => e.insert(map),
            None => return,
        },
    };
    let mut under = [0u32; ARROW_MAX_CELLS];
    let under_len = map.take_into(pos, rotation, &mut under);
    map.draw_arrow(pos, rotation);
    damage[damage_len] = rect(pos, rotation, map.width, map.height);
    damage_len += 1;
    // Drivers that copy to the screen (simpledrm, vmwgfx) do so for dirty areas.
    let dirty = card.dirty_framebuffer(fb, &damage[..damage_len]);
    if kms_debug() {
        eprintln!("[kms] pointer drawn at {pos:?} on framebuffer {fb_id} ({dirty:?})");
    }
    state.drawn = Some(Drawn { fb, at: pos, under, under_len });
}

fn rect((x, y): (i32, i32), rotation: u8, max_w: i32, max_h: i32) -> ClipRect {
    let clamp_x = |v: i32| v.clamp(0, max_w.min(u16::MAX as i32)) as u16;
    let clamp_y = |v: i32| v.clamp(0, max_h.min(u16::MAX as i32)) as u16;
    let ((w, h), _) = turned_box(rotation);
    ClipRect::new(clamp_x(x), clamp_y(y), clamp_x(x + w), clamp_y(y + h))
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

    /// Every on-screen pixel of the (turned) arrow's box at `at`, in order.
    fn cells(&self, at: (i32, i32), rotation: u8) -> impl Iterator<Item = (i32, i32, usize)> + '_ {
        let ((w, h), _) = turned_box(rotation);
        (0..h).flat_map(move |dy| (0..w).map(move |dx| (dx, dy))).filter_map(move |(dx, dy)| {
            let (x, y) = (at.0 + dx, at.1 + dy);
            (x >= 0 && y >= 0 && x < self.width && y < self.height).then(|| (dx, dy, y as usize * self.stride + x as usize))
        })
    }

    fn take_into(&self, at: (i32, i32), rotation: u8, out: &mut [u32; ARROW_MAX_CELLS]) -> usize {
        let mut n = 0;
        for (_, _, i) in self.cells(at, rotation) {
            if n < out.len() {
                out[n] = unsafe { self.ptr.add(i).read_volatile() };
                n += 1;
            }
        }
        n
    }

    fn put(&self, at: (i32, i32), rotation: u8, pixels: &[u32]) {
        for ((_, _, i), &px) in self.cells(at, rotation).zip(pixels) {
            unsafe { self.ptr.add(i).write_volatile(px) };
        }
    }

    fn draw_arrow(&self, at: (i32, i32), rotation: u8) {
        for (dx, dy, i) in self.cells(at, rotation) {
            if let Some(px) = turned(rotation, dx, dy) {
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

/// The (turned) arrow drawn into a cursor plane's image.
fn draw_cursor(card: &Card, buffer: &mut DumbBuffer, rotation: u8) -> bool {
    let pitch = drm::buffer::Buffer::pitch(buffer) as usize;
    let Ok(mut map) = card.map_dumb_buffer(buffer) else {
        return false;
    };
    let bytes = map.as_mut();
    bytes.fill(0);
    let ((w, h), _) = turned_box(rotation);
    for y in 0..h {
        for x in 0..w {
            if let Some(argb) = turned(rotation, x, y) {
                let at = y as usize * pitch + x as usize * 4;
                bytes[at..at + 4].copy_from_slice(&argb.to_le_bytes());
            }
        }
    }
    true
}

/// The arrow in a cursor plane, when the driver has one; hidden for now.
fn hardware_cursor(card: &Card, crtc: crtc::Handle, rotation: u8) -> Option<DumbBuffer> {
    let mut buffer = card.create_dumb_buffer((64, 64), DrmFourcc::Argb8888, 32).ok()?;
    if !draw_cursor(card, &mut buffer, rotation) {
        let _ = card.destroy_dumb_buffer(buffer);
        return None;
    }
    // Setting it proves the plane is there; it stays hidden until a mouse moves.
    let (_, tip) = turned_box(rotation);
    #[allow(deprecated)]
    {
        if card.set_cursor2(crtc, Some(&buffer), tip).is_err() {
            let _ = card.destroy_dumb_buffer(buffer);
            return None;
        }
        let _ = card.set_cursor2::<DumbBuffer>(crtc, None, (0, 0));
    }
    Some(buffer)
}
