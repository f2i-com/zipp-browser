//! TinySkia software renderer for the Zipp browser.
//!
//! This module provides a CPU-only rendering backend built on [`tiny_skia`] for
//! path rasterization and [`softbuffer`] for window presentation. It is designed
//! to work in environments without GPU access, such as Windows RDP sessions.
//!
//! ## Architecture
//!
//! The renderer is split into two main types:
//!
//! - [`TinySkiaPainter`] -- implements the [`PaintScene`] trait. It owns a
//!   `Pixmap` framebuffer and provides fill, stroke, glyph, image, and box-shadow
//!   drawing operations. Clip regions are tracked via a push/pop stack so that
//!   child layers can be clipped to rounded rectangles.
//!
//! - [`TinySkiaWindowRenderer`] -- implements the [`WindowRenderer`] trait. It
//!   manages the softbuffer surface lifecycle (resume / suspend / resize) and
//!   converts the RGBA pixel data produced by the painter into the BGR0 format
//!   expected by the OS framebuffer.
//!
//! Glyph outlines are extracted from OpenType fonts via [`skrifa`] and converted
//! to tiny-skia paths through the [`GlyphOutlinePen`] adapter.

use std::num::NonZeroU32;
use std::sync::Arc;

use anyrender::{Glyph, NormalizedCoord, PaintScene, WindowHandle, WindowRenderer};
use kurbo::{Affine, Rect, Shape, Stroke};
use peniko::{
    color::{AlphaColor, Srgb},
    BlendMode, Fill, FontData,
};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};

use skrifa::MetadataProvider;

// ============================================================================
// Color and paint helpers
// ============================================================================

/// Convenience alias -- all color values in this module use sRGB with alpha.
type Color = AlphaColor<Srgb>;

/// Decompose a [`Color`] into its (R, G, B, A) byte components.
#[inline]
fn color_to_rgba(color: Color) -> (u8, u8, u8, u8) {
    let c = color.to_rgba8();
    (c.r, c.g, c.b, c.a)
}

/// Extract RGBA bytes from an `anyrender::PaintRef`. Non-solid paints default
/// to opaque black.
#[inline]
fn extract_rgba(paint: &anyrender::PaintRef<'_>) -> (u8, u8, u8, u8) {
    match paint {
        anyrender::PaintRef::Solid(c) => color_to_rgba(*c),
        _ => (0, 0, 0, 255),
    }
}

/// Create an anti-aliased [`Paint`] with the given RGBA color.
#[inline]
fn make_paint(r: u8, g: u8, b: u8, a: u8) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8(r, g, b, a);
    paint.anti_alias = true;
    paint
}

// ============================================================================
// Coordinate and path conversion helpers
// ============================================================================

/// Convert a kurbo [`Affine`] transform to a tiny-skia [`Transform`].
#[inline]
fn affine_to_transform(affine: Affine) -> Transform {
    let c = affine.as_coeffs();
    Transform::from_row(
        c[0] as f32,
        c[1] as f32,
        c[2] as f32,
        c[3] as f32,
        c[4] as f32,
        c[5] as f32,
    )
}

/// Convert any kurbo [`Shape`] to a tiny-skia [`Path`], flattening curves to a
/// tolerance of 0.1 device pixels.
fn shape_to_path(shape: &impl Shape) -> Option<tiny_skia::Path> {
    let mut builder = PathBuilder::new();
    for element in shape.path_elements(0.1) {
        match element {
            kurbo::PathEl::MoveTo(p) => builder.move_to(p.x as f32, p.y as f32),
            kurbo::PathEl::LineTo(p) => builder.line_to(p.x as f32, p.y as f32),
            kurbo::PathEl::QuadTo(ctrl, end) => {
                builder.quad_to(ctrl.x as f32, ctrl.y as f32, end.x as f32, end.y as f32)
            }
            kurbo::PathEl::CurveTo(c1, c2, end) => builder.cubic_to(
                c1.x as f32,
                c1.y as f32,
                c2.x as f32,
                c2.y as f32,
                end.x as f32,
                end.y as f32,
            ),
            kurbo::PathEl::ClosePath => builder.close(),
        }
    }
    builder.finish()
}

// ============================================================================
// Pixel blending
// ============================================================================

/// Alpha-composite a single source pixel onto a destination pixel in a
/// premultiplied RGBA buffer.
///
/// `dest` must point to a 4-byte slice `[R, G, B, A]` at the target position.
/// The source color is given by `(src_r, src_g, src_b, src_a)`.
///
/// When `src_a == 255` the source overwrites the destination. When `src_a == 0`
/// the destination is left untouched. Otherwise standard "source over"
/// compositing is applied.
#[inline]
fn blend_pixel(dest: &mut [u8], src_r: u8, src_g: u8, src_b: u8, src_a: u8) {
    if src_a == 255 {
        dest[0] = src_r;
        dest[1] = src_g;
        dest[2] = src_b;
        dest[3] = 255;
    } else if src_a > 0 {
        // Fixed-point integer alpha blending (avoids 6 float conversions per pixel)
        let sa = src_a as u16;
        let inv_sa = 255 - sa;
        dest[0] = ((dest[0] as u16 * inv_sa + src_r as u16 * sa + 128) >> 8) as u8;
        dest[1] = ((dest[1] as u16 * inv_sa + src_g as u16 * sa + 128) >> 8) as u8;
        dest[2] = ((dest[2] as u16 * inv_sa + src_b as u16 * sa + 128) >> 8) as u8;
        dest[3] = 255;
    }
}

// ============================================================================
// Clip region -- supports both axis-aligned rects and arbitrary (rounded) paths
// ============================================================================

/// A clip region used to restrict drawing to a sub-area of the framebuffer.
///
/// Clips are maintained as a stack inside [`TinySkiaPainter`]. When a layer is
/// pushed, its clip shape (typically a rounded rectangle) is transformed to
/// screen coordinates and stored here. The two variants provide an optimization:
///
/// - [`Rect`](ClipRegion::Rect) -- fast axis-aligned bounding-box test.
/// - [`Path`](ClipRegion::RoundedPath) -- rasterized to a 1-bit mask for
///   pixel-accurate clipping of rounded corners and arbitrary shapes.
#[derive(Clone)]
#[allow(dead_code)]
enum ClipRegion {
    /// Axis-aligned rectangular clip.
    Rect(Rect),

    /// Arbitrary path clip (e.g. a rounded rectangle). The path is stored as a
    /// `BezPath` to allow zero-allocation winding-number tests.  The bounding
    /// box is cached for a quick rejection test before the more expensive check.
    RoundedPath {
        bounding_box: Rect,
        path: kurbo::BezPath,
    },
}

impl ClipRegion {
    /// Test whether the pixel at `(x, y)` lies inside this clip region.
    #[allow(dead_code)]
    fn contains(&self, x: f64, y: f64) -> bool {
        match self {
            ClipRegion::Rect(r) => x >= r.x0 && x <= r.x1 && y >= r.y0 && y <= r.y1,
            ClipRegion::RoundedPath {
                bounding_box,
                path,
            } => {
                // Quick bounding-box rejection (with 1px tolerance)
                if x < bounding_box.x0 - 1.0
                    || x > bounding_box.x1 + 1.0
                    || y < bounding_box.y0 - 1.0
                    || y > bounding_box.y1 + 1.0
                {
                    return false;
                }
                // Full winding-number test at the pixel center (no allocation)
                path.winding(kurbo::Point::new(x + 0.5, y + 0.5)) != 0
            }
        }
    }

    /// Return the axis-aligned bounding box of this clip region.
    #[inline]
    fn bounding_box(&self) -> Rect {
        match self {
            ClipRegion::Rect(r) => *r,
            ClipRegion::RoundedPath { bounding_box, .. } => *bounding_box,
        }
    }

    /// Rasterize this clip region into a 1-bit [`tiny_skia::Mask`].
    ///
    /// Returns `None` for simple rectangular clips (which are handled via
    /// coordinate tests instead) or if mask allocation fails.
    fn to_mask(&self, width: u32, height: u32) -> Option<tiny_skia::Mask> {
        let ClipRegion::RoundedPath { path, .. } = self else {
            return None;
        };

        let clip_path = shape_to_path(path)?;
        let mut mask = tiny_skia::Mask::new(width, height)?;
        // Anti-aliased fill produces smooth clip edges on rounded corners
        mask.fill_path(
            &clip_path,
            FillRule::EvenOdd,
            true, // anti-alias
            Transform::identity(),
        );
        Some(mask)
    }
}

/// Transform a clip shape from local coordinates to screen coordinates.
///
/// Every path element is mapped through `transform`, and the result is returned
/// as a `RoundedPath` clip with a pre-computed bounding box. The bounding box
/// is tracked incrementally during the transform pass to avoid a second traversal.
fn transform_clip_shape(transform: Affine, shape: &impl Shape) -> ClipRegion {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;

    let mut track_point = |p: kurbo::Point| {
        if p.x < min_x { min_x = p.x; }
        if p.y < min_y { min_y = p.y; }
        if p.x > max_x { max_x = p.x; }
        if p.y > max_y { max_y = p.y; }
    };

    let screen_elements: Vec<kurbo::PathEl> = kurbo::BezPath::from_vec(
        shape.path_elements(0.1).collect(),
    )
    .into_iter()
    .map(|el| match el {
        kurbo::PathEl::MoveTo(p) => {
            let tp = transform * p;
            track_point(tp);
            kurbo::PathEl::MoveTo(tp)
        }
        kurbo::PathEl::LineTo(p) => {
            let tp = transform * p;
            track_point(tp);
            kurbo::PathEl::LineTo(tp)
        }
        kurbo::PathEl::QuadTo(a, b) => {
            let ta = transform * a;
            let tb = transform * b;
            track_point(ta);
            track_point(tb);
            kurbo::PathEl::QuadTo(ta, tb)
        }
        kurbo::PathEl::CurveTo(a, b, c) => {
            let ta = transform * a;
            let tb = transform * b;
            let tc = transform * c;
            track_point(ta);
            track_point(tb);
            track_point(tc);
            kurbo::PathEl::CurveTo(ta, tb, tc)
        }
        kurbo::PathEl::ClosePath => kurbo::PathEl::ClosePath,
    })
    .collect();

    let bounding_box = Rect::new(min_x, min_y, max_x, max_y);

    ClipRegion::RoundedPath {
        bounding_box,
        path: kurbo::BezPath::from_vec(screen_elements),
    }
}

// ============================================================================
// TinySkiaPainter -- the PaintScene implementation
// ============================================================================

/// CPU-based scene painter backed by a [`Pixmap`] framebuffer.
///
/// Supports fill, stroke, glyph rendering, image blitting, and box shadows.
/// Clip regions are managed via a push/pop stack that mirrors the layer
/// hierarchy produced by the layout engine.
/// Saved state for a compositing layer. Opacity layers snapshot the pixmap so
/// that newly drawn content can be blended at the requested alpha when popped.
struct LayerEntry {
    /// Previous clip region to restore on pop.
    saved_clip: Option<ClipRegion>,
    /// If this is an opacity layer (alpha < 1.0), holds the pixmap snapshot
    /// taken *before* any content was drawn into the layer, plus the alpha.
    opacity_snapshot: Option<(Pixmap, f32)>,
}

pub struct TinySkiaPainter {
    /// The RGBA pixel buffer that accumulates all drawing operations.
    pub pixmap: Pixmap,

    /// Stack of saved layer states, pushed/popped alongside layers.
    layer_stack: Vec<LayerEntry>,

    /// The currently active clip region, if any. Drawing operations outside this
    /// region are discarded or masked.
    active_clip: Option<ClipRegion>,
}

impl TinySkiaPainter {
    /// Create a new painter with the given framebuffer dimensions (minimum 1x1).
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            pixmap: Pixmap::new(width.max(1), height.max(1)).unwrap(),
            layer_stack: Vec::new(),
            active_clip: None,
        }
    }

    /// Resize the framebuffer. Ignored if either dimension is zero.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.pixmap = Pixmap::new(width, height).unwrap();
        }
    }

    /// Build a clip mask from the active clip region, sized to the current pixmap.
    fn active_clip_mask(&self) -> Option<tiny_skia::Mask> {
        self.active_clip
            .as_ref()
            .and_then(|clip| clip.to_mask(self.pixmap.width(), self.pixmap.height()))
    }
}

// ---------------------------------------------------------------------------
// PaintScene trait implementation
// ---------------------------------------------------------------------------

impl PaintScene for TinySkiaPainter {
    /// Clear the framebuffer to white and reset the clip stack.
    fn reset(&mut self) {
        // Frame counter for debugging (not currently displayed)
        static FRAME: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let _frame_number = FRAME.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        self.pixmap.fill(tiny_skia::Color::WHITE);
        self.layer_stack.clear();
        self.active_clip = None;
    }

    /// Push a compositing layer with a clip shape and optional opacity.
    ///
    /// When `alpha` < 1.0 the current pixmap is snapshotted so that on
    /// [`pop_layer`] the newly drawn content can be alpha-blended over the
    /// original. This gives correct group-opacity semantics.
    fn push_layer(
        &mut self,
        _blend: impl Into<BlendMode>,
        alpha: f32,
        transform: Affine,
        clip: &impl Shape,
    ) {
        let new_clip = transform_clip_shape(transform, clip);
        let old_clip = std::mem::replace(&mut self.active_clip, Some(new_clip));

        let opacity_snapshot = if alpha < 1.0 {
            Some((self.pixmap.clone(), alpha))
        } else {
            None
        };

        self.layer_stack.push(LayerEntry {
            saved_clip: old_clip,
            opacity_snapshot,
        });
    }

    /// Push a clip-only layer (no blend mode, full opacity).
    fn push_clip_layer(&mut self, transform: Affine, clip: &impl Shape) {
        let new_clip = transform_clip_shape(transform, clip);
        let old_clip = std::mem::replace(&mut self.active_clip, Some(new_clip));
        self.layer_stack.push(LayerEntry {
            saved_clip: old_clip,
            opacity_snapshot: None,
        });
    }

    /// Pop the most recent layer, restoring the previous clip region.
    ///
    /// For opacity layers the snapshotted pixmap is restored and the content
    /// drawn during the layer is composited on top at the layer's alpha value.
    fn pop_layer(&mut self) {
        if let Some(entry) = self.layer_stack.pop() {
            self.active_clip = entry.saved_clip;

            if let Some((snapshot, alpha)) = entry.opacity_snapshot {
                // Blend: for each pixel, result = snapshot + (current - snapshot) * alpha
                //
                // `current - snapshot` is SIGNED — the layer darkens a pixel as
                // often as it lightens one. Computed in u16 the negative case
                // wraps to ~65535 and the following multiply overflows, which
                // panicked on the first frame of any page using opacity. i32 is
                // wide enough that no intermediate can overflow: the difference
                // is within ±255 and the product within ±65025.
                let alpha = (alpha * 255.0).round().clamp(0.0, 255.0) as i32;
                let src = snapshot.data();
                let dst = self.pixmap.data_mut();
                // Both pixmaps have the same dimensions so byte counts match.
                for (d, s) in dst.iter_mut().zip(src.iter()) {
                    let snapshot = i32::from(*s);
                    let current = i32::from(*d);
                    *d = (snapshot + (current - snapshot) * alpha / 255) as u8;
                }
            }
        }
    }

    /// Fill a shape with a brush (solid color or image).
    ///
    /// For image brushes, pixels are sampled from the source image and
    /// alpha-composited directly into the framebuffer. For solid colors, the
    /// shape is rasterized through tiny-skia with the active clip mask applied.
    fn fill<'a>(
        &mut self,
        _style: Fill,
        transform: Affine,
        brush: impl Into<anyrender::PaintRef<'a>>,
        _brush_transform: Option<Affine>,
        shape: &impl Shape,
    ) {
        let paint_ref = brush.into();

        match &paint_ref {
            anyrender::PaintRef::Image(img) => {
                self.fill_with_image(img, transform, shape);
            }
            _ => {
                let (r, g, b, a) = extract_rgba(&paint_ref);
                if let Some(path) = shape_to_path(shape) {
                    let mask = self.active_clip_mask();
                    self.pixmap.fill_path(
                        &path,
                        &make_paint(r, g, b, a),
                        FillRule::Winding,
                        affine_to_transform(transform),
                        mask.as_ref(),
                    );
                }
            }
        }
    }

    /// Stroke a shape outline.
    fn stroke<'a>(
        &mut self,
        style: &Stroke,
        transform: Affine,
        brush: impl Into<anyrender::PaintRef<'a>>,
        _brush_transform: Option<Affine>,
        shape: &impl Shape,
    ) {
        let (r, g, b, a) = extract_rgba(&brush.into());
        if let Some(path) = shape_to_path(shape) {
            let stroke = tiny_skia::Stroke {
                width: style.width as f32,
                line_cap: tiny_skia::LineCap::Round,
                line_join: tiny_skia::LineJoin::Round,
                ..Default::default()
            };
            let mask = self.active_clip_mask();
            self.pixmap.stroke_path(
                &path,
                &make_paint(r, g, b, a),
                &stroke,
                affine_to_transform(transform),
                mask.as_ref(),
            );
        }
    }

    /// Draw a sequence of font glyphs by extracting outlines from the font via
    /// skrifa and converting them to tiny-skia paths.
    fn draw_glyphs<'a, 's: 'a>(
        &'s mut self,
        font: &'a FontData,
        font_size: f32,
        _hint: bool,
        normalized_coords: &'a [NormalizedCoord],
        _style: impl Into<peniko::StyleRef<'a>>,
        brush: impl Into<anyrender::PaintRef<'a>>,
        _alpha: f32,
        transform: Affine,
        _glyph_transform: Option<Affine>,
        glyphs: impl Iterator<Item = Glyph>,
    ) {
        let (r, g, b, a) = extract_rgba(&brush.into());
        let paint = make_paint(r, g, b, a);

        // Parse the font binary to access glyph outlines
        let font_bytes: &[u8] = font.data.as_ref();
        let Ok(font_ref) = skrifa::FontRef::from_index(font_bytes, font.index) else {
            return;
        };

        // Build variation coordinates for variable fonts
        let variation_coords: Vec<skrifa::instance::NormalizedCoord> = normalized_coords
            .iter()
            .map(|&c| skrifa::instance::NormalizedCoord::from_bits(c))
            .collect();
        let size = skrifa::instance::Size::new(font_size);
        let location = skrifa::instance::LocationRef::new(&variation_coords);

        let outlines = font_ref.outline_glyphs();
        let skia_transform = affine_to_transform(transform);

        // Read units-per-em for potential manual scaling (not used when skrifa's
        // DrawSettings handles scaling via Size, but kept for diagnostics).
        use read_fonts::TableProvider;
        let _upem = font_ref
            .head()
            .map(|h| h.units_per_em() as f32)
            .unwrap_or(1000.0);

        for glyph in glyphs {
            let glyph_id = skrifa::GlyphId::new(glyph.id);

            let Some(outline) = outlines.get(glyph_id) else {
                continue;
            };

            let mut path_builder = PathBuilder::new();

            // GlyphOutlinePen with scale=1.0 because skrifa's DrawSettings
            // already handles the units-to-pixels conversion via `size`.
            let mut pen = GlyphOutlinePen {
                builder: &mut path_builder,
                scale: 1.0,
                glyph_x: glyph.x,
                glyph_y: glyph.y,
            };

            let settings = skrifa::outline::DrawSettings::unhinted(size, location);
            if outline.draw(settings, &mut pen).is_ok() {
                if let Some(path) = path_builder.finish() {
                    self.pixmap
                        .fill_path(&path, &paint, FillRule::Winding, skia_transform, None);
                }
            }
        }
    }

    /// Draw a standalone image, applying the affine transform for positioning
    /// and scaling, with clip mask support for rounded containers.
    ///
    /// The transform is interpreted as:
    /// - `coeffs[0]` (sx): horizontal scale factor
    /// - `coeffs[3]` (sy): vertical scale factor
    /// - `coeffs[4]` (tx): destination X offset in pixels
    /// - `coeffs[5]` (ty): destination Y offset in pixels
    fn draw_image(&mut self, image: peniko::ImageBrushRef, transform: Affine) {
        let image_width = image.image.width;
        let image_height = image.image.height;
        let source_pixels: &[u8] = image.image.data.as_ref();

        // Extract scale and translation from the transform matrix
        let coeffs = transform.as_coeffs();
        let scale_x = coeffs[0] as f32;
        let scale_y = coeffs[3] as f32;
        let dest_x = coeffs[4] as f32;
        let dest_y = coeffs[5] as f32;

        // Destination rectangle dimensions (the scaled image size on screen)
        let dest_width = (image_width as f32 * scale_x).max(1.0);
        let dest_height = (image_height as f32 * scale_y).max(1.0);

        let framebuffer_width = self.pixmap.width() as i32;
        let framebuffer_height = self.pixmap.height() as i32;

        // Ratio to map from destination pixels back to source pixels
        let sample_ratio_x = image_width as f32 / dest_width;
        let sample_ratio_y = image_height as f32 / dest_height;

        // Prepare clip mask. For rounded containers (e.g. avatar circles), the
        // clip region is rasterized into a 1-bit mask. Each destination pixel is
        // tested against this mask and skipped if outside the clip.
        // Pre-compute from &self.active_clip before taking &mut via data_mut().
        let clip_mask = self
            .active_clip
            .as_ref()
            .and_then(|c| c.to_mask(framebuffer_width as u32, framebuffer_height as u32));
        let clip_bbox = self.active_clip.as_ref().map(|c| c.bounding_box());

        let framebuffer = self.pixmap.data_mut();

        // Iterate over every pixel in the destination rectangle
        for row in 0..(dest_height as i32) {
            let pixel_y = dest_y as i32 + row;
            if pixel_y < 0 || pixel_y >= framebuffer_height {
                continue;
            }

            for col in 0..(dest_width as i32) {
                let pixel_x = dest_x as i32 + col;
                if pixel_x < 0 || pixel_x >= framebuffer_width {
                    continue;
                }

                // Clip test: prefer the rasterized mask for accurate rounded
                // clipping; fall back to bounding-box rejection otherwise.
                if let Some(ref mask) = clip_mask {
                    let mask_index = (pixel_y as u32) * (framebuffer_width as u32) + (pixel_x as u32);
                    if (mask_index as usize) < mask.data().len()
                        && mask.data()[mask_index as usize] == 0
                    {
                        continue;
                    }
                } else if let Some(ref bbox) = clip_bbox {
                    if (pixel_x as f64) < bbox.x0
                        || (pixel_x as f64) > bbox.x1
                        || (pixel_y as f64) < bbox.y0
                        || (pixel_y as f64) > bbox.y1
                    {
                        continue;
                    }
                }

                // Map destination pixel back to the nearest source pixel
                let src_col = ((col as f32 * sample_ratio_x) as u32).min(image_width - 1);
                let src_row = ((row as f32 * sample_ratio_y) as u32).min(image_height - 1);

                let source_index = ((src_row * image_width + src_col) * 4) as usize;
                if source_index + 3 >= source_pixels.len() {
                    continue;
                }

                let dest_index = ((pixel_y * framebuffer_width + pixel_x) * 4) as usize;
                if dest_index + 3 >= framebuffer.len() {
                    continue;
                }

                // SAFETY: bounds verified above (source_index+3 < len, dest_index+3 < len)
                unsafe {
                    let sr = *source_pixels.get_unchecked(source_index);
                    let sg = *source_pixels.get_unchecked(source_index + 1);
                    let sb = *source_pixels.get_unchecked(source_index + 2);
                    let sa = *source_pixels.get_unchecked(source_index + 3);
                    blend_pixel(
                        framebuffer.get_unchecked_mut(dest_index..dest_index + 4),
                        sr, sg, sb, sa,
                    );
                }
            }
        }
    }

    /// Draw a box shadow as a semi-transparent expanded rectangle.
    fn draw_box_shadow(
        &mut self,
        transform: Affine,
        rect: Rect,
        color: Color,
        radius: f64,
        _std_dev: f64,
    ) {
        let (r, g, b, a) = color_to_rgba(color);
        let shadow_alpha = (a as f32 * 0.3) as u8;

        // Expand the rectangle by the blur radius on all sides
        let shadow_rect = tiny_skia::Rect::from_xywh(
            (rect.x0 - radius) as f32,
            (rect.y0 - radius) as f32,
            (rect.width() + radius * 2.0) as f32,
            (rect.height() + radius * 2.0) as f32,
        );

        if let Some(skia_rect) = shadow_rect {
            let mask = self.active_clip_mask();
            self.pixmap.fill_rect(
                skia_rect,
                &make_paint(r, g, b, shadow_alpha),
                affine_to_transform(transform),
                mask.as_ref(),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Image fill helper (used by the fill() method for image brushes)
// ---------------------------------------------------------------------------

impl TinySkiaPainter {
    /// Blit an image brush into the framebuffer, scaling it to fit the bounding
    /// box of the target shape.
    fn fill_with_image(
        &mut self,
        img: &peniko::ImageBrushRef,
        transform: Affine,
        shape: &impl Shape,
    ) {
        let image_width = img.image.width;
        let image_height = img.image.height;
        let source_pixels: &[u8] = img.image.data.as_ref();

        // The shape's bounding box defines the destination rectangle
        let bounds = shape.bounding_box();
        let coeffs = transform.as_coeffs();
        let dest_x = coeffs[4] as f32 + bounds.x0 as f32;
        let dest_y = coeffs[5] as f32 + bounds.y0 as f32;
        let dest_width = bounds.width() as f32;
        let dest_height = bounds.height() as f32;

        let framebuffer_width = self.pixmap.width() as i32;
        let framebuffer_height = self.pixmap.height() as i32;

        // Ratio to map from destination pixels back to source pixels
        let sample_ratio_x = image_width as f32 / dest_width;
        let sample_ratio_y = image_height as f32 / dest_height;

        let framebuffer = self.pixmap.data_mut();

        for row in 0..(dest_height as i32) {
            let pixel_y = dest_y as i32 + row;
            if pixel_y < 0 || pixel_y >= framebuffer_height {
                continue;
            }

            for col in 0..(dest_width as i32) {
                let pixel_x = dest_x as i32 + col;
                if pixel_x < 0 || pixel_x >= framebuffer_width {
                    continue;
                }

                let src_col = ((col as f32 * sample_ratio_x) as u32).min(image_width - 1);
                let src_row = ((row as f32 * sample_ratio_y) as u32).min(image_height - 1);

                let source_index = ((src_row * image_width + src_col) * 4) as usize;
                if source_index + 3 >= source_pixels.len() {
                    continue;
                }

                let dest_index =
                    ((pixel_y * framebuffer_width + pixel_x) * 4) as usize;
                if dest_index + 3 >= framebuffer.len() {
                    continue;
                }

                // SAFETY: bounds verified above (source_index+3 < len, dest_index+3 < len)
                unsafe {
                    let sr = *source_pixels.get_unchecked(source_index);
                    let sg = *source_pixels.get_unchecked(source_index + 1);
                    let sb = *source_pixels.get_unchecked(source_index + 2);
                    let sa = *source_pixels.get_unchecked(source_index + 3);
                    blend_pixel(
                        framebuffer.get_unchecked_mut(dest_index..dest_index + 4),
                        sr, sg, sb, sa,
                    );
                }
            }
        }
    }
}

// ============================================================================
// GlyphOutlinePen -- converts skrifa glyph outlines to tiny-skia paths
// ============================================================================

/// Adapter that receives glyph outline drawing commands from skrifa and appends
/// them to a tiny-skia [`PathBuilder`].
///
/// Each coordinate is translated by `(glyph_x, glyph_y)` -- the glyph's
/// on-screen baseline position -- and optionally scaled by `scale` (though in
/// practice this is 1.0 because skrifa's `DrawSettings` with a `Size` already
/// handles the units-to-pixels conversion).
///
/// The Y axis is flipped (`y` is subtracted) because font outlines use an
/// upward Y axis while screen coordinates go downward.
struct GlyphOutlinePen<'a> {
    builder: &'a mut PathBuilder,
    scale: f32,
    glyph_x: f32,
    glyph_y: f32,
}

impl skrifa::outline::OutlinePen for GlyphOutlinePen<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.builder.move_to(
            self.glyph_x + x * self.scale,
            self.glyph_y - y * self.scale,
        );
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.builder.line_to(
            self.glyph_x + x * self.scale,
            self.glyph_y - y * self.scale,
        );
    }

    fn quad_to(&mut self, ctrl_x: f32, ctrl_y: f32, end_x: f32, end_y: f32) {
        self.builder.quad_to(
            self.glyph_x + ctrl_x * self.scale,
            self.glyph_y - ctrl_y * self.scale,
            self.glyph_x + end_x * self.scale,
            self.glyph_y - end_y * self.scale,
        );
    }

    fn curve_to(
        &mut self,
        ctrl1_x: f32,
        ctrl1_y: f32,
        ctrl2_x: f32,
        ctrl2_y: f32,
        end_x: f32,
        end_y: f32,
    ) {
        self.builder.cubic_to(
            self.glyph_x + ctrl1_x * self.scale,
            self.glyph_y - ctrl1_y * self.scale,
            self.glyph_x + ctrl2_x * self.scale,
            self.glyph_y - ctrl2_y * self.scale,
            self.glyph_x + end_x * self.scale,
            self.glyph_y - end_y * self.scale,
        );
    }

    fn close(&mut self) {
        self.builder.close();
    }
}

// ============================================================================
// TinySkiaWindowRenderer -- softbuffer presentation (works without GPU / on RDP)
// ============================================================================

/// Window renderer that uses [`TinySkiaPainter`] for CPU rasterization and
/// [`softbuffer`] for presenting the framebuffer to the OS window surface.
///
/// The lifecycle is:
/// 1. [`resume`](WindowRenderer::resume) -- creates the softbuffer context and
///    surface from the window handle.
/// 2. [`render`](WindowRenderer::render) -- clears the painter, invokes the
///    draw callback, then copies pixels from the RGBA pixmap to the surface
///    buffer (converting RGBA to BGR0 format).
/// 3. [`suspend`](WindowRenderer::suspend) -- releases the surface and context.
pub struct TinySkiaWindowRenderer {
    painter: TinySkiaPainter,
    context: Option<softbuffer::Context<Arc<dyn WindowHandle>>>,
    surface: Option<softbuffer::Surface<Arc<dyn WindowHandle>, Arc<dyn WindowHandle>>>,
    width: u32,
    height: u32,
}

impl TinySkiaWindowRenderer {
    pub fn new() -> Self {
        Self {
            painter: TinySkiaPainter::new(1, 1),
            context: None,
            surface: None,
            width: 1,
            height: 1,
        }
    }
}

impl WindowRenderer for TinySkiaWindowRenderer {
    type ScenePainter<'a> = TinySkiaPainter;

    /// Initialize the softbuffer surface for the given window.
    fn resume(&mut self, window: Arc<dyn WindowHandle>, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        self.painter.resize(width, height);

        let ctx = softbuffer::Context::new(window.clone()).unwrap();
        let mut surface = softbuffer::Surface::new(&ctx, window).unwrap();
        let _ = surface.resize(
            NonZeroU32::new(width.max(1)).unwrap(),
            NonZeroU32::new(height.max(1)).unwrap(),
        );

        self.context = Some(ctx);
        self.surface = Some(surface);
    }

    /// Release the softbuffer surface and context.
    fn suspend(&mut self) {
        self.surface = None;
        self.context = None;
    }

    /// Returns `true` if the renderer has an active surface.
    fn is_active(&self) -> bool {
        self.surface.is_some()
    }

    /// Resize the framebuffer and surface to new dimensions.
    fn set_size(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        self.painter.resize(width, height);

        if let Some(ref mut surface) = self.surface {
            let _ = surface.resize(
                NonZeroU32::new(width.max(1)).unwrap(),
                NonZeroU32::new(height.max(1)).unwrap(),
            );
        }
    }

    /// Execute the draw callback on the painter, then present the result.
    ///
    /// The painter's RGBA pixel data is converted to the BGR0 format expected
    /// by the softbuffer surface: each 32-bit word is `0x00RRGGBB`.
    fn render<F: FnOnce(&mut Self::ScenePainter<'_>)>(&mut self, draw_fn: F) {
        self.painter.reset();
        draw_fn(&mut self.painter);

        let Some(ref mut surface) = self.surface else {
            return;
        };
        let Ok(mut buffer) = surface.buffer_mut() else {
            return;
        };

        // Convert RGBA (tiny-skia) to BGR0 (softbuffer)
        let rgba_pixels = self.painter.pixmap.data();
        let pixel_count = buffer.len().min((self.width * self.height) as usize);

        for (i, chunk) in rgba_pixels.chunks_exact(4).enumerate().take(pixel_count) {
            // Pack as 0x00RRGGBB (red in bits 16-23, green in 8-15, blue in 0-7)
            buffer[i] = ((chunk[0] as u32) << 16)
                | ((chunk[1] as u32) << 8)
                | (chunk[2] as u32);
        }

        let _ = buffer.present();
    }
}
