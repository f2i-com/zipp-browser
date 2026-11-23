//! Renderer for the Zipp browser engine.
//!
//! Paints a [`zipp_dom::BaseDocument`] by pushing [`anyrender`] drawing commands into
//! an impl [`anyrender::PaintScene`].
//!
//! The renderer walks the laid-out DOM tree and emits paint commands for backgrounds, borders,
//! text, images, SVGs, canvases, and sub-documents. It also handles clipping layers, opacity,
//! CSS transforms, and debug overlays.

#![allow(clippy::collapsible_if)]

mod color;
mod debug_overlay;
mod gradient;
mod kurbo_css;
mod layers;
mod render;
mod sizing;
mod text;
pub mod tinyskia;

use anyrender::PaintScene;
use zipp_dom::{BaseDocument, util::Color};
use render::DocumentPainter;

/// Paint a [`zipp_dom::BaseDocument`] by pushing drawing commands into
/// an impl [`anyrender::PaintScene`].
///
/// This function assumes that the styles and layout in the [`BaseDocument`] are already
/// resolved. Please ensure that this is the case before trying to paint.
///
/// The implementation of [`PaintScene`] is responsible for handling the commands that are pushed into it.
/// Generally this will involve executing them to draw a rasterized image/texture. But in some cases it may choose to
/// transform them to a vector format (e.g. SVG/PDF) or serialize them in raw form for later use.
pub fn paint_scene(
    scene: &mut (impl PaintScene + crate::tinyskia::BlurLayer),
    dom: &BaseDocument,
    scale: f64,
    width: u32,
    height: u32,
    x_offset: u32,
    y_offset: u32,
) {
    let generator =
        DocumentPainter::new(dom, scale, width, height, x_offset as f64, y_offset as f64);
    generator.paint_scene(scene);
}

/// Partial-scene repaint starting from `target_nodes` (such as updated opaque `<canvas>`
/// elements): skips all painting prior to the first target node while preserving ancestor
/// clips/scroll offsets, then paints the target nodes and any subsequent overlays normally.
#[allow(clippy::too_many_arguments)]
pub fn paint_scene_from_nodes(
    scene: &mut (impl PaintScene + crate::tinyskia::BlurLayer),
    dom: &BaseDocument,
    scale: f64,
    width: u32,
    height: u32,
    x_offset: u32,
    y_offset: u32,
    target_nodes: &[usize],
) {
    let mut generator =
        DocumentPainter::new(dom, scale, width, height, x_offset as f64, y_offset as f64);
    generator.set_skip_until_nodes(target_nodes);
    generator.paint_scene(scene);
}

const SELECTION_COLOR: Color = Color::from_rgb8(180, 213, 255);
