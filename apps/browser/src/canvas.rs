//! Frames that canvases drew with WebGL (webgl.rs) or WebGPU (webgpu.rs),
//! waiting to become their elements' images (`apply_canvas_frames`).

use std::cell::RefCell;

/// A frame read back from a canvas: its element's `data-flid`, size and RGBA.
pub(crate) struct Frame {
    pub flid: u32,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

thread_local! {
    static FRAMES: RefCell<Vec<Frame>> = const { RefCell::new(Vec::new()) };
}

/// A canvas's newest frame (replacing one it presented earlier).
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn push_frame(frame: Frame) {
    FRAMES.with(|f| {
        let mut frames = f.borrow_mut();
        frames.retain(|old| old.flid != frame.flid);
        frames.push(frame);
    });
}

/// The frames presented since the last call.
pub(crate) fn take_frames() -> Vec<Frame> {
    FRAMES.with(|f| std::mem::take(&mut *f.borrow_mut()))
}
