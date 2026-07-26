//! Compositing-layer management with a hard cap to prevent GPU resource exhaustion.
//!
//! Tracks how many layers are currently pushed onto the paint scene and refuses
//! to open new ones once the limit (`LAYER_LIMIT`) is reached.

use anyrender::PaintScene;
use kurbo::{Affine, Shape};
use peniko::Mix;
use std::cell::Cell;

/// Maximum NESTING DEPTH of compositing layers.
///
/// This bounds a recursive resource (each level costs stack and, on a GPU
/// backend, a render target), which is why it is a depth limit and not a count
/// of how many layers a frame draws in total. A long page legitimately pushes
/// and pops many thousands of clips; it never nests them thousands deep.
const LAYER_LIMIT: u32 = 1024;

/// Tracks compositing-layer usage so the renderer can stay within GPU limits.
#[derive(Default)]
pub(crate) struct LayerManager {
    /// Current nesting depth of pushed layers — the value the budget tests.
    current_depth: Cell<u32>,
    /// Number of layers that were *requested* (including those denied by the cap).
    requested_count: Cell<u32>,

    /// High-water mark of `current_depth`, for debugging.
    #[allow(unused)]
    layer_depth_used: Cell<u32>,
}

impl LayerManager {
    /// Conditionally pushes a compositing layer, runs `paint_layer`, then pops.
    pub(crate) fn maybe_with_layer<S: PaintScene, F: FnOnce(&mut S)>(
        &self,
        scene: &mut S,
        condition: bool,
        opacity: f32,
        transform: Affine,
        shape: &impl Shape,
        paint_layer: F,
    ) {
        let layer_used = self.maybe_push_layer(scene, condition, opacity, transform, shape);
        paint_layer(scene);
        self.maybe_pop_layer(scene, layer_used);
    }

    /// Pushes a layer if `condition` is true and the layer budget has not been
    /// exhausted. Returns `true` when a layer was actually pushed.
    pub(crate) fn maybe_push_layer(
        &self,
        scene: &mut impl PaintScene,
        condition: bool,
        opacity: f32,
        transform: Affine,
        shape: &impl Shape,
    ) -> bool {
        if !condition {
            return false;
        }
        self.requested_count.update(|x| x + 1);

        // Refuse only when nesting is genuinely too deep.
        //
        // This used to test `active_count`, which was incremented on push and
        // never decremented on pop — a cumulative tally of every layer the frame
        // had drawn. On any page with more than 1024 clipped boxes it latched
        // false partway down, and from that point every clip was silently
        // skipped for the rest of the frame: overflow:hidden stopped clipping,
        // rounded corners stopped masking, and (now that text honours the clip)
        // glyphs started escaping their boxes — all with no diagnostic.
        if self.current_depth.get() >= LAYER_LIMIT {
            return false;
        }

        // Actually push the layer
        if opacity == 1.0 {
            scene.push_clip_layer(transform, shape);
        } else {
            scene.push_layer(Mix::Normal, opacity, transform, shape);
        };

        // Update accounting
        let depth = self.current_depth.get() + 1;
        self.current_depth.set(depth);
        self.layer_depth_used.update(|hwm| hwm.max(depth));

        true
    }

    /// Pops a layer if `condition` is true (i.e. a layer was previously pushed).
    pub(crate) fn maybe_pop_layer(&self, scene: &mut impl PaintScene, condition: bool) {
        if condition {
            scene.pop_layer();
            // saturating: a stray pop must not wrap the depth to u32::MAX and
            // wedge the budget closed for the rest of the frame.
            self.current_depth.update(|x| x.saturating_sub(1));
        }
    }
}
