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

/// Quick heuristic check for whether raw bytes may contain an SVG document.
#[cfg(feature = "svg")]
fn looks_like_svg(bytes: &[u8]) -> bool {
    let trimmed = bytes
        .strip_prefix(&[0xEF, 0xBB, 0xBF])
        .unwrap_or(bytes);
    let trimmed = match trimmed.iter().position(|b| !b.is_ascii_whitespace()) {
        Some(i) => &trimmed[i..],
        None => return false,
    };
    if !trimmed.starts_with(b"<") {
        return false;
    }
    let head = &trimmed[..trimmed.len().min(512)];
    head.windows(4).any(|w| w.eq_ignore_ascii_case(b"<svg"))
}

/// Intrinsic pixel size `(width, height)` of an SVG document, if `bytes` is valid SVG.
#[cfg(feature = "svg")]
pub fn svg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if !looks_like_svg(bytes) {
        return None;
    }
    let tree = zipp_dom::util::parse_svg(bytes).ok()?;
    let size = tree.size();
    let w = size.width().ceil().clamp(1.0, 4096.0) as u32;
    let h = size.height().ceil().clamp(1.0, 4096.0) as u32;
    Some((w, h))
}

/// Rasterize an SVG document into a premultiplied RGBA [`tiny_skia::Pixmap`].
#[cfg(feature = "svg")]
pub fn rasterize_svg(bytes: &[u8]) -> Option<tiny_skia::Pixmap> {
    if !looks_like_svg(bytes) {
        return None;
    }
    let tree = zipp_dom::util::parse_svg(bytes).ok()?;
    let size = tree.size();
    let w = size.width().ceil().clamp(1.0, 4096.0) as u32;
    let h = size.height().ceil().clamp(1.0, 4096.0) as u32;
    let mut painter = crate::tinyskia::TinySkiaPainter::new(w, h);
    let sx = w as f64 / f64::from(size.width().max(1.0));
    let sy = h as f64 / f64::from(size.height().max(1.0));
    anyrender_svg::render_svg_tree(&mut painter, &tree, kurbo::Affine::scale_non_uniform(sx, sy));
    Some(painter.pixmap)
}

#[cfg(all(test, feature = "svg"))]
mod tests {
    use super::{rasterize_svg, svg_size};

    #[test]
    fn svg_size_and_rasterize_handle_valid_svg_and_reject_non_svg() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="12" viewBox="0 0 16 12"><rect width="16" height="12" fill="#336699"/></svg>"##;
        assert_eq!(svg_size(svg), Some((16, 12)));
        let pixmap = rasterize_svg(svg).expect("should rasterize valid svg");
        assert_eq!((pixmap.width(), pixmap.height()), (16, 12));
        // Center pixel should be opaque #336699.
        let p = pixmap.pixel(8, 6).unwrap();
        assert_eq!((p.red(), p.green(), p.blue(), p.alpha()), (0x33, 0x66, 0x99, 0xff));

        let png_sig = b"\x89PNG\r\n\x1a\nnot-an-svg";
        assert_eq!(svg_size(png_sig), None);
        assert!(rasterize_svg(png_sig).is_none());
    }
}
