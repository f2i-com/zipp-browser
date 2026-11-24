//! Frames that canvases drew with WebGL (webgl.rs) or WebGPU (webgpu.rs),
//! waiting to become their elements' images (`apply_canvas_frames`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

/// A frame read back from a canvas: its element's `data-flid`, size and RGBA.
pub(crate) struct Frame {
    pub flid: u32,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// A frame a canvas is showing: its size and pixels.
pub(crate) type Shown = (u32, u32, Arc<Vec<u8>>);

thread_local! {
    static FRAMES: RefCell<Vec<(u64, Frame)>> = const { RefCell::new(Vec::new()) };
    /// What each canvas of a document scope is showing, keyed by `(scope, data-flid)`.
    /// Scoping isolates top-level pages and embedded `<iframe>` sub-documents so
    /// matching `data-flid`s never collide or steal frames from each other.
    static SHOWN: RefCell<HashMap<(u64, u32), Shown>> = RefCell::new(HashMap::new());
}

/// A canvas's newest frame (replacing one it presented earlier in the same scope).
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn push_frame(frame: Frame) {
    let scope = crate::async_fetch::current_scope();
    FRAMES.with(|f| {
        let mut frames = f.borrow_mut();
        frames.retain(|(s, old)| *s != scope || old.flid != frame.flid);
        frames.push((scope, frame));
    });
}

/// The frames presented for `scope` since the last call.
/// Never steals frames belonging to a different non-zero scope (even when `scope == 0`).
pub(crate) fn take_frames_for(scope: u64) -> Vec<(u32, Shown)> {
    let matching: Vec<(u64, Frame)> = FRAMES.with(|f| {
        let mut frames = f.borrow_mut();
        let mut mine = Vec::new();
        let mut rest = Vec::new();
        for item in frames.drain(..) {
            let matches = if scope == 0 {
                item.0 == 0
            } else {
                item.0 == scope || item.0 == 0
            };
            if matches {
                mine.push(item);
            } else {
                rest.push(item);
            }
        }
        *frames = rest;
        mine
    });
    SHOWN.with(|s| {
        let mut shown = s.borrow_mut();
        matching
            .into_iter()
            .map(|(sc, f)| {
                let frame = (f.width, f.height, Arc::new(f.rgba));
                let key_scope = if sc != 0 { sc } else { scope };
                shown.insert((key_scope, f.flid), frame.clone());
                (f.flid, frame)
            })
            .collect()
    })
}

/// Every frame shown by canvases in `scope`.
pub(crate) fn shown_frames_for(scope: u64) -> Vec<(u32, Shown)> {
    SHOWN.with(|s| {
        s.borrow()
            .iter()
            .filter(|((sc, _), _)| if scope == 0 { *sc == 0 } else { *sc == scope || *sc == 0 })
            .map(|((_, flid), frame)| (*flid, frame.clone()))
            .collect()
    })
}

/// Drop pending frames from any scope other than `scope` (called when building
/// a fresh top-level document so unconsumed frames from a prior page cannot
/// leak, while keeping the new page's initial frames intact).
pub(crate) fn retain_scope(scope: u64) {
    FRAMES.with(|f| f.borrow_mut().retain(|(s, _)| *s == scope));
}

/// Drop cached canvas frames belonging to an unmounted iframe scope.
pub(crate) fn forget_scope(scope: u64) {
    if scope == 0 {
        return;
    }
    FRAMES.with(|f| f.borrow_mut().retain(|(s, _)| *s != scope));
    SHOWN.with(|s| s.borrow_mut().retain(|(sc, _), _| *sc != scope));
}

/// A new page: nothing it has is showing anything yet.
/// Only clears `SHOWN`; pending frames in `FRAMES` drawn during initial page
/// load are kept for `apply_canvas_frames_for` right after this call.
pub(crate) fn forget_shown() {
    SHOWN.with(|s| s.borrow_mut().clear());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::async_fetch::ScopeGuard;

    fn dummy_frame(flid: u32, tag: u8) -> Frame {
        Frame { flid, width: 2, height: 2, rgba: vec![tag; 16] }
    }

    #[test]
    fn initial_frames_survive_forget_shown_and_isolate_scopes() {
        forget_shown();
        retain_scope(0);

        // Stale frame from an earlier page (scope 901) + fresh page frame (scope 902).
        {
            let _g = ScopeGuard::enter(901);
            push_frame(dummy_frame(5, 0xAA));
        }
        {
            let _g = ScopeGuard::enter(902);
            push_frame(dummy_frame(5, 0xBB));
        }

        // build_document sequence for scope 902: stale scope 901 is dropped,
        // scope 902's initial wallpaper/canvas frame survives and becomes shown.
        forget_shown();
        retain_scope(902);
        let taken = take_frames_for(902);
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].0, 5);
        assert_eq!(taken[0].1.2[0], 0xBB);

        // Embedded iframe (scope 903) with identical data-flid=5 stays isolated.
        {
            let _g = ScopeGuard::enter(903);
            push_frame(dummy_frame(5, 0xCC));
        }
        assert!(take_frames_for(902).is_empty());
        let iframe_taken = take_frames_for(903);
        assert_eq!(iframe_taken.len(), 1);
        assert_eq!(iframe_taken[0].1.2[0], 0xCC);

        // Re-applying frames per scope returns each document's own canvas pixels.
        let p_shown = shown_frames_for(902);
        assert_eq!(p_shown.len(), 1);
        assert_eq!(p_shown[0].1.2[0], 0xBB);
        let i_shown = shown_frames_for(903);
        assert_eq!(i_shown.len(), 1);
        assert_eq!(i_shown[0].1.2[0], 0xCC);

        // Unmounting iframe forgets only scope 903, leaving parent 902 intact.
        forget_scope(903);
        assert!(shown_frames_for(903).is_empty());
        assert_eq!(shown_frames_for(902).len(), 1);

        forget_shown();
        retain_scope(0);
    }
}
