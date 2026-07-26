//! Compositing-layer management with a hard cap to prevent GPU resource exhaustion.
//!
//! Tracks how many layers are currently pushed onto the paint scene and refuses
//! to open new ones once the limit (`LAYER_LIMIT`) is reached.

use anyrender::PaintScene;
use kurbo::{Affine, Shape};
use peniko::Mix;
use std::cell::Cell;

/// Maximum number of compositing layers that may be active at once.
const LAYER_LIMIT: u32 = 1024;

/// Tracks compositing-layer usage so the renderer can stay within GPU limits.
#[derive(Default)]
pub(crate) struct LayerManager {
    /// Total number of layers that have been pushed (cumulative).
    active_count: Cell<u32>,
    /// Current nesting depth of pushed layers.
    current_depth: Cell<u32>,
    /// Number of layers that were *requested* (including those denied by the cap).
    requested_count: Cell<u32>,

    #[allow(unused)] // Only used for debugging. Enabled as required.
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

        // Check if clips are above limit
        let layers_available = self.active_count.get() <= LAYER_LIMIT;
        if !layers_available {
            return false;
        }

        // Actually push the layer
        if opacity == 1.0 {
            scene.push_clip_layer(transform, shape);
        } else {
            scene.push_layer(Mix::Normal, opacity, transform, shape);
        };

        // Update accounting
        self.active_count.update(|x| x + 1);
        self.current_depth.update(|x| x + 1);
        self.current_depth.update(|x| x.max(self.current_depth.get()));

        true
    }

    /// Pops a layer if `condition` is true (i.e. a layer was previously pushed).
    pub(crate) fn maybe_pop_layer(&self, scene: &mut impl PaintScene, condition: bool) {
        if condition {
            scene.pop_layer();
            self.current_depth.update(|x| x - 1);
        }
    }
}
