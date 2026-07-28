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

/// Build a tiny-skia paint for any [`anyrender::PaintRef`], using a real
/// gradient shader when the brush is a gradient.
///
/// Falling back to a flat colour for gradients is what made every
/// `linear-gradient(...)` on the web paint as an opaque black box: gradients are
/// built correctly all the way through `gradient::to_peniko_gradient`, and were
/// then discarded here.
///
/// `transform` is the shape's transform: tiny-skia resolves gradient geometry in
/// the shader's own space, so the same transform has to be handed to the shader
/// for the gradient to line up with the shape it fills.
fn make_brush_paint<'a>(
    paint_ref: &anyrender::PaintRef<'a>,
    transform: Transform,
) -> Paint<'static> {
    let anyrender::PaintRef::Gradient(gradient) = paint_ref else {
        let (r, g, b, a) = extract_rgba(paint_ref);
        return make_paint(r, g, b, a);
    };

    let stops: Vec<tiny_skia::GradientStop> = gradient
        .stops
        .iter()
        .map(|stop| {
            let c = stop.color.to_alpha_color::<Srgb>().to_rgba8();
            tiny_skia::GradientStop::new(
                stop.offset,
                tiny_skia::Color::from_rgba8(c.r, c.g, c.b, c.a),
            )
        })
        .collect();

    // A shader needs at least two stops; with fewer, paint the single colour.
    if stops.len() < 2 {
        let (r, g, b, a) = gradient
            .stops
            .first()
            .map(|s| color_to_rgba(s.color.to_alpha_color::<Srgb>()))
            .unwrap_or((0, 0, 0, 255));
        return make_paint(r, g, b, a);
    }

    let mode = match gradient.extend {
        peniko::Extend::Pad => tiny_skia::SpreadMode::Pad,
        peniko::Extend::Repeat => tiny_skia::SpreadMode::Repeat,
        peniko::Extend::Reflect => tiny_skia::SpreadMode::Reflect,
    };

    let shader = match gradient.kind {
        peniko::GradientKind::Linear(pos) => tiny_skia::LinearGradient::new(
            tiny_skia::Point::from_xy(pos.start.x as f32, pos.start.y as f32),
            tiny_skia::Point::from_xy(pos.end.x as f32, pos.end.y as f32),
            stops,
            mode,
            transform,
        ),
        // tiny-skia's radial shader is single-circle, so the two-circle (focal)
        // form collapses to the end circle — exact for the overwhelmingly common
        // `radial-gradient(...)`, which has a zero-radius start circle.
        peniko::GradientKind::Radial(pos) => tiny_skia::RadialGradient::new(
            tiny_skia::Point::from_xy(pos.start_center.x as f32, pos.start_center.y as f32),
            tiny_skia::Point::from_xy(pos.end_center.x as f32, pos.end_center.y as f32),
            pos.end_radius.max(f32::EPSILON),
            stops,
            mode,
            transform,
        ),
        // tiny-skia has no sweep/conic shader, and faking one with a linear
        // sweep looks plainly wrong. Fall through to the mid-stop colour below.
        peniko::GradientKind::Sweep(_) => None,
    };

    match shader {
        Some(shader) => Paint {
            shader,
            anti_alias: true,
            ..Default::default()
        },
        None => {
            // Shader construction failed (degenerate geometry, or conic above):
            // fall back to the middle stop rather than black.
            let (r, g, b, a) = gradient
                .stops
                .get(gradient.stops.len() / 2)
                .map(|s| color_to_rgba(s.color.to_alpha_color::<Srgb>()))
                .unwrap_or((0, 0, 0, 255));
            make_paint(r, g, b, a)
        }
    }
}

// ============================================================================
// Coordinate and path conversion helpers
// ============================================================================

/// A shape's bounding box in device space.
#[inline]
fn device_bounds(shape: &impl Shape, transform: Affine) -> Rect {
    let b = shape.bounding_box();
    let corners = [
        transform * kurbo::Point::new(b.x0, b.y0),
        transform * kurbo::Point::new(b.x1, b.y0),
        transform * kurbo::Point::new(b.x0, b.y1),
        transform * kurbo::Point::new(b.x1, b.y1),
    ];
    let (mut x0, mut y0, mut x1, mut y1) =
        (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in corners {
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    Rect::new(x0, y0, x1, y1)
}

/// The transform a gradient shader should carry: the BRUSH transform only.
///
/// Gradient geometry is expressed in brush space (peniko emits a unit circle /
/// unit line and puts the sizing in this transform), and `brush_transform` maps
/// that into user space.
///
/// It must NOT be pre-composed with the shape transform: tiny-skia's
/// `fill_path`/`stroke_path` already apply their own `transform` argument to the
/// shader as well as the path (`painter.rs`: `paint.shader.transform(ts)`, which
/// `post_concat`s it). Composing here too applied the shape transform twice —
/// a unit-circle radial ended up scaled and translated far off-screen, which is
/// why radial gradients painted nothing at all.
#[inline]
fn brush_space(brush_transform: Option<Affine>) -> Transform {
    affine_to_transform(brush_transform.unwrap_or(Affine::IDENTITY))
}

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

    /// A clip nested inside another: a pixel must be inside BOTH.
    ///
    /// CSS clips nest, so a clipped box inside a rounded `overflow:hidden`
    /// ancestor must be confined to the intersection. Only built when at least
    /// one side has curves — rectangle-on-rectangle nesting (the common case)
    /// collapses to a single `Rect` in `ClipRegion::intersect`.
    Intersection(Box<ClipRegion>, Box<ClipRegion>),
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
            ClipRegion::Intersection(a, b) => a.contains(x, y) && b.contains(x, y),
        }
    }

    /// Combine two clips, collapsing to a plain rectangle whenever possible.
    ///
    /// Rect-on-rect is exact and free, and it is what almost all nesting is:
    /// most clipped boxes are ordinary `overflow:hidden` rectangles. Keeping
    /// those out of the `Intersection`/mask path is what makes correct nesting
    /// affordable — a mask costs a full-framebuffer rasterization.
    fn intersect(outer: ClipRegion, inner: ClipRegion) -> ClipRegion {
        if let (ClipRegion::Rect(a), ClipRegion::Rect(b)) = (&outer, &inner) {
            return ClipRegion::Rect(a.intersect(*b));
        }
        // A side whose bounding box already contains the other's constrains
        // nothing when it is a plain rectangle, so it can be dropped.
        let (obox, ibox) = (outer.bounding_box(), inner.bounding_box());
        if matches!(inner, ClipRegion::Rect(_)) && contains_box(ibox, obox) {
            return outer;
        }
        if matches!(outer, ClipRegion::Rect(_)) && contains_box(obox, ibox) {
            return inner;
        }
        ClipRegion::Intersection(Box::new(outer), Box::new(inner))
    }

    /// Return the axis-aligned bounding box of this clip region.
    #[inline]
    fn bounding_box(&self) -> Rect {
        match self {
            ClipRegion::Rect(r) => *r,
            ClipRegion::RoundedPath { bounding_box, .. } => *bounding_box,
            ClipRegion::Intersection(a, b) => a.bounding_box().intersect(b.bounding_box()),
        }
    }

    /// Rasterize this clip region into a 1-bit [`tiny_skia::Mask`].
    ///
    /// Returns `None` for simple rectangular clips (which are handled via
    /// coordinate tests instead) or if mask allocation fails.
    fn to_mask(&self, width: u32, height: u32) -> Option<tiny_skia::Mask> {
        match self {
            ClipRegion::Rect(r) => {
                let clip_path = shape_to_path(r)?;
                let mut mask = tiny_skia::Mask::new(width, height)?;
                mask.fill_path(&clip_path, FillRule::EvenOdd, true, Transform::identity());
                Some(mask)
            }
            ClipRegion::RoundedPath { path, .. } => {
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
            // Seed with the intersected bounding box, then cut each curved side
            // out of it.
            ClipRegion::Intersection(a, b) => {
                let mut mask = tiny_skia::Mask::new(width, height)?;
                let seed = shape_to_path(&self.bounding_box())?;
                mask.fill_path(&seed, FillRule::EvenOdd, true, Transform::identity());
                a.intersect_into(&mut mask);
                b.intersect_into(&mut mask);
                Some(mask)
            }
        }
    }

    /// Cut this region out of `mask`. Rectangles are already covered by the
    /// seeded bounding box, so only curved sides do work.
    fn intersect_into(&self, mask: &mut tiny_skia::Mask) {
        match self {
            ClipRegion::Rect(_) => {}
            ClipRegion::RoundedPath { path, .. } => {
                if let Some(path) = shape_to_path(path) {
                    mask.intersect_path(&path, FillRule::EvenOdd, true, Transform::identity());
                }
            }
            ClipRegion::Intersection(a, b) => {
                a.intersect_into(mask);
                b.intersect_into(mask);
            }
        }
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

    // An axis-aligned rectangle becomes a `Rect`, not a path.
    //
    // This matters far beyond tidiness: a `RoundedPath` clip forces a
    // full-framebuffer `tiny_skia::Mask` to be rasterized, while a `Rect` is a
    // handful of comparisons. Almost every clip on a real page is a plain
    // `overflow:hidden` rectangle, and returning `RoundedPath` unconditionally
    // meant every one of them paid for a mask — and made nesting them
    // prohibitively expensive.
    if is_axis_aligned_rect(&screen_elements, bounding_box) {
        return ClipRegion::Rect(bounding_box);
    }

    ClipRegion::RoundedPath {
        bounding_box,
        path: kurbo::BezPath::from_vec(screen_elements),
    }
}

/// Whether a transformed path is exactly its own axis-aligned bounding box.
///
/// True when the path has no curves and every vertex sits on a corner of the
/// bounding box — i.e. it is the rectangle itself, possibly closed.
fn is_axis_aligned_rect(elements: &[kurbo::PathEl], bbox: Rect) -> bool {
    // Sub-pixel tolerance: transforms introduce rounding, and a clip that is a
    // rectangle to within a hundredth of a pixel is a rectangle.
    const EPS: f64 = 0.01;
    let mut corners = 0usize;
    for el in elements {
        let p = match el {
            kurbo::PathEl::MoveTo(p) | kurbo::PathEl::LineTo(p) => *p,
            kurbo::PathEl::ClosePath => continue,
            // Any curve means real geometry (a rounded corner, an ellipse).
            _ => return false,
        };
        let on_x = (p.x - bbox.x0).abs() < EPS || (p.x - bbox.x1).abs() < EPS;
        let on_y = (p.y - bbox.y0).abs() < EPS || (p.y - bbox.y1).abs() < EPS;
        if !on_x || !on_y {
            return false;
        }
        corners += 1;
    }
    // 4 corners, or 5 when the path repeats its start point to close.
    (4..=5).contains(&corners)
}

/// Whether `outer` fully contains `inner`, within the same sub-pixel tolerance.
#[inline]
fn contains_box(outer: Rect, inner: Rect) -> bool {
    outer.x0 <= inner.x0 + 0.01
        && outer.y0 <= inner.y0 + 0.01
        && outer.x1 >= inner.x1 - 0.01
        && outer.y1 >= inner.y1 - 0.01
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
    /// The previous clip's rasterized mask, restored with it.
    saved_mask: Option<Arc<tiny_skia::Mask>>,
    /// If this is an opacity layer (alpha < 1.0), holds the pixmap snapshot
    /// taken *before* any content was drawn into the layer, plus the alpha.
    opacity_snapshot: Option<(Pixmap, f32)>,
}

pub struct TinySkiaPainter {
    /// The RGBA pixel buffer that accumulates all drawing operations.
    pub pixmap: Pixmap,

    /// Stack of saved layer states, pushed/popped alongside layers.
    layer_stack: Vec<LayerEntry>,
    /// Offscreen surfaces captured for `filter: blur()`, with their radii.
    blur_stack: Vec<(Pixmap, f32)>,

    /// The currently active clip region, if any. Drawing operations outside this
    /// region are discarded or masked.
    active_clip: Option<ClipRegion>,

    /// `active_clip` rasterized, or `None` when the clip is rectangular and
    /// needs no mask.
    ///
    /// Cached because every fill, stroke and glyph run consults it; rasterizing
    /// per draw call made painting a real page take seconds. `Arc` so a draw
    /// site can hold it while `&mut self.pixmap` is borrowed without copying a
    /// full-framebuffer mask.
    clip_mask: Option<Arc<tiny_skia::Mask>>,
}

impl TinySkiaPainter {
    /// Create a new painter with the given framebuffer dimensions (minimum 1x1).
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            pixmap: Pixmap::new(width.max(1), height.max(1)).unwrap(),
            layer_stack: Vec::new(),
            blur_stack: Vec::new(),
            active_clip: None,
            clip_mask: None,
        }
    }

    /// Resize the framebuffer. Ignored if either dimension is zero.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.pixmap = Pixmap::new(width, height).unwrap();
        }
    }

    /// Build a clip mask from the active clip region, sized to the current pixmap.
    /// The clip mask to pass to a draw whose device-space bounds are `bounds`.
    ///
    /// Returns `None` — meaning "no clipping needed" — when the shape lies
    /// wholly inside the clip, which is the usual case for content drawn inside
    /// its own clipped box. That matters because a mask is a full-framebuffer
    /// allocation: building one per draw made painting a real page take
    /// seconds, and building one per clipped BOX is still wasteful when nothing
    /// ever crosses the boundary.
    ///
    /// The mask is memoized per clip, so a page where many draws do cross pays
    /// for one rasterization per clip rather than one per draw.
    fn clip_mask_for(&mut self, bounds: Rect) -> Option<Arc<tiny_skia::Mask>> {
        let clip = self.active_clip.as_ref()?;
        // A rectangular clip that fully contains the shape constrains nothing.
        // (Only sound for rectangles: a rounded clip can cut a shape that sits
        // inside its bounding box.)
        if matches!(clip, ClipRegion::Rect(_)) && contains_box(clip.bounding_box(), bounds) {
            return None;
        }
        if self.clip_mask.is_none() {
            let (w, h) = (self.pixmap.width(), self.pixmap.height());
            // Re-borrow: `to_mask` needs `&self.active_clip` while `self` is &mut.
            self.clip_mask = self
                .active_clip
                .as_ref()
                .and_then(|c| c.to_mask(w, h))
                .map(Arc::new);
        }
        self.clip_mask.clone()
    }

    /// Make `new_clip` active, INTERSECTED with whatever is already active, and
    /// return the previous (clip, mask) so `pop_layer` can restore them.
    ///
    /// Intersecting rather than replacing is what makes nested clips correct: a
    /// clipped box inside a rounded `overflow:hidden` ancestor must be confined
    /// to both, or it paints over its parent's corners.
    fn push_clip(
        &mut self,
        new_clip: ClipRegion,
    ) -> (Option<ClipRegion>, Option<Arc<tiny_skia::Mask>>) {
        let old_clip = self.active_clip.take();
        let old_mask = self.clip_mask.take();
        let combined = match old_clip.clone() {
            Some(existing) => ClipRegion::intersect(existing, new_clip),
            None => new_clip,
        };
        // Left unrasterized: `clip_mask_for` builds it on first genuine need.
        self.clip_mask = None;
        self.active_clip = Some(combined);
        (old_clip, old_mask)
    }
}

// ---------------------------------------------------------------------------
// PaintScene trait implementation
// ---------------------------------------------------------------------------

/// Painting an element through a blur.
///
/// `anyrender`'s `PaintScene` has no notion of a filter, so this cannot be
/// threaded through `push_layer`. It is a separate capability the renderer
/// requires of its painter instead.
pub trait BlurLayer {
    /// Begin capturing into an offscreen surface to be blurred on pop.
    fn push_blur_layer(&mut self, radius_px: f32);
    /// Blur what was captured and composite it over the surface below.
    fn pop_blur_layer(&mut self);
    /// Blur what is ALREADY painted within `rect` — `backdrop-filter`.
    fn apply_backdrop_blur(&mut self, radius_px: f32, rect: (f32, f32, f32, f32));
}

/// Blur `pixmap` in place with three box passes.
///
/// Three successive box blurs approximate a Gaussian closely enough that the
/// difference is invisible for the decorative glows this exists to serve, and
/// each pass is a running sum — O(pixels), independent of radius, where a true
/// Gaussian would be O(pixels x radius).
///
/// Operates on PREMULTIPLIED RGBA, which is what tiny-skia stores. That matters:
/// averaging un-premultiplied colour would drag the colour of fully transparent
/// pixels into its neighbours and fringe the edges.
fn box_blur(pixmap: &mut tiny_skia::Pixmap, radius: f32) {
    let w = pixmap.width() as usize;
    let h = pixmap.height() as usize;
    if w == 0 || h == 0 || radius < 0.5 {
        return;
    }
    // Box width for three passes approximating a Gaussian of this sigma.
    let sigma = radius;
    let box_w = ((sigma * 3.0 * (2.0 * std::f32::consts::PI).sqrt() / 4.0) + 0.5) as usize;
    let r = (box_w / 2).max(1);
    if r == 0 {
        return;
    }

    let data = pixmap.data_mut();
    let mut src: Vec<u8> = data.to_vec();
    let mut dst: Vec<u8> = vec![0; data.len()];

    for _ in 0..3 {
        // Horizontal.
        for y in 0..h {
            let row = y * w * 4;
            let mut acc = [0u32; 4];
            // Prime the window with the clamped left edge.
            for i in 0..=r {
                let x = i.min(w - 1);
                for c in 0..4 {
                    acc[c] += src[row + x * 4 + c] as u32;
                }
            }
            for c in 0..4 {
                acc[c] += src[row + c] as u32 * r as u32;
            }
            let count = (2 * r + 1) as u32;
            for x in 0..w {
                for c in 0..4 {
                    dst[row + x * 4 + c] = (acc[c] / count) as u8;
                }
                let add = (x + r + 1).min(w - 1);
                let sub = x.saturating_sub(r);
                for c in 0..4 {
                    acc[c] += src[row + add * 4 + c] as u32;
                    acc[c] -= src[row + sub * 4 + c] as u32;
                }
            }
        }
        std::mem::swap(&mut src, &mut dst);

        // Vertical.
        for x in 0..w {
            let col = x * 4;
            let mut acc = [0u32; 4];
            for i in 0..=r {
                let y = i.min(h - 1);
                for c in 0..4 {
                    acc[c] += src[y * w * 4 + col + c] as u32;
                }
            }
            for c in 0..4 {
                acc[c] += src[col + c] as u32 * r as u32;
            }
            let count = (2 * r + 1) as u32;
            for y in 0..h {
                for c in 0..4 {
                    dst[y * w * 4 + col + c] = (acc[c] / count) as u8;
                }
                let add = (y + r + 1).min(h - 1);
                let sub = y.saturating_sub(r);
                for c in 0..4 {
                    acc[c] += src[add * w * 4 + col + c] as u32;
                    acc[c] -= src[sub * w * 4 + col + c] as u32;
                }
            }
        }
        std::mem::swap(&mut src, &mut dst);
    }
    data.copy_from_slice(&src);
}

impl BlurLayer for TinySkiaPainter {
    fn push_blur_layer(&mut self, radius_px: f32) {
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        // Draw into a fresh transparent surface so the blur sees ONLY this
        // element. Blurring in place would smear whatever was already painted
        // underneath it.
        let saved = std::mem::replace(
            &mut self.pixmap,
            Pixmap::new(w.max(1), h.max(1)).unwrap(),
        );
        self.blur_stack.push((saved, radius_px));
    }

    fn apply_backdrop_blur(&mut self, radius_px: f32, rect: (f32, f32, f32, f32)) {
        // `backdrop-filter` blurs the BACKDROP — what is already on the surface
        // behind this element — not the element itself. Without it a frosted
        // panel, whose own background is deliberately near-transparent because
        // the blur is meant to supply the substance, just looks see-through.
        let (x0, y0, x1, y1) = rect;
        // Pad by the radius so the blur has real pixels to pull from at the
        // edges rather than smearing the clamped border inward.
        let pad = radius_px.ceil() as i32 + 1;
        let px0 = (x0.floor() as i32 - pad).max(0);
        let py0 = (y0.floor() as i32 - pad).max(0);
        let px1 = (x1.ceil() as i32 + pad).min(self.pixmap.width() as i32);
        let py1 = (y1.ceil() as i32 + pad).min(self.pixmap.height() as i32);
        let (w, h) = ((px1 - px0).max(0) as u32, (py1 - py0).max(0) as u32);
        if w == 0 || h == 0 {
            return;
        }
        let Some(mut region) = Pixmap::new(w, h) else { return };
        region.draw_pixmap(
            -px0,
            -py0,
            self.pixmap.as_ref(),
            &tiny_skia::PixmapPaint::default(),
            tiny_skia::Transform::identity(),
            None,
        );
        box_blur(&mut region, radius_px);
        // Write back only the element's own box, so the padding used to feed
        // the blur does not bleed over its neighbours.
        let keep = tiny_skia::IntRect::from_ltrb(
            x0.floor() as i32 - px0,
            y0.floor() as i32 - py0,
            x1.ceil() as i32 - px0,
            y1.ceil() as i32 - py0,
        );
        let Some(keep) = keep else { return };
        let Some(cropped) = region.clone_rect(keep) else { return };
        self.pixmap.draw_pixmap(
            x0.floor() as i32,
            y0.floor() as i32,
            cropped.as_ref(),
            &tiny_skia::PixmapPaint::default(),
            tiny_skia::Transform::identity(),
            None,
        );
    }

    fn pop_blur_layer(&mut self) {
        if let Some((saved, radius)) = self.blur_stack.pop() {
            let mut layer = std::mem::replace(&mut self.pixmap, saved);
            box_blur(&mut layer, radius);
            self.pixmap.draw_pixmap(
                0,
                0,
                layer.as_ref(),
                &tiny_skia::PixmapPaint::default(),
                tiny_skia::Transform::identity(),
                None,
            );
        }
    }
}

impl PaintScene for TinySkiaPainter {
    /// Clear the framebuffer to white and reset the clip stack.
    fn reset(&mut self) {
        // Frame counter for debugging (not currently displayed)
        static FRAME: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let _frame_number = FRAME.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        self.pixmap.fill(tiny_skia::Color::WHITE);
        self.layer_stack.clear();
        self.active_clip = None;
        self.clip_mask = None;
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
        let (old_clip, old_mask) = self.push_clip(transform_clip_shape(transform, clip));

        let opacity_snapshot = if alpha < 1.0 {
            Some((self.pixmap.clone(), alpha))
        } else {
            None
        };

        self.layer_stack.push(LayerEntry {
            saved_clip: old_clip,
            saved_mask: old_mask,
            opacity_snapshot,
        });
    }

    /// Push a clip-only layer (no blend mode, full opacity).
    fn push_clip_layer(&mut self, transform: Affine, clip: &impl Shape) {
        let (old_clip, old_mask) = self.push_clip(transform_clip_shape(transform, clip));
        self.layer_stack.push(LayerEntry {
            saved_clip: old_clip,
            saved_mask: old_mask,
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
            self.clip_mask = entry.saved_mask;

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
        brush_transform: Option<Affine>,
        shape: &impl Shape,
    ) {
        let paint_ref = brush.into();

        match &paint_ref {
            anyrender::PaintRef::Image(img) => {
                self.fill_with_image(img, transform, shape);
            }
            _ => {
                if let Some(path) = shape_to_path(shape) {
                    let ts = affine_to_transform(transform);
                    // A gradient's geometry is expressed in BRUSH space, which
                    // `brush_transform` maps into user space; the shape transform
                    // then maps user space to the device. The shader therefore
                    // needs both composed. Dropping the brush transform left
                    // radial gradients with a ~1px radius at the origin — they
                    // rendered as nothing at all.
                    let paint = make_brush_paint(&paint_ref, brush_space(brush_transform));
                    let mask = self.clip_mask_for(device_bounds(shape, transform));
                    self.pixmap
                        .fill_path(&path, &paint, FillRule::Winding, ts, mask.as_deref());
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
        brush_transform: Option<Affine>,
        shape: &impl Shape,
    ) {
        let paint_ref = brush.into();
        if let Some(path) = shape_to_path(shape) {
            let stroke = tiny_skia::Stroke {
                width: style.width as f32,
                line_cap: tiny_skia::LineCap::Round,
                line_join: tiny_skia::LineJoin::Round,
                ..Default::default()
            };
            let ts = affine_to_transform(transform);
            let paint = make_brush_paint(&paint_ref, brush_space(brush_transform));
            let mask =
                self.clip_mask_for(device_bounds(shape, transform).inflate(style.width, style.width));
            self.pixmap
                .stroke_path(&path, &paint, &stroke, ts, mask.as_deref());
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

        // Collect the run so its extent can be measured before drawing. The
        // clip mask is then requested ONCE for the whole run — it is a
        // full-framebuffer allocation, so per-glyph would be ruinous — and only
        // when the run actually reaches outside the clip.
        //
        // Text used to pass `None` here, making it the one primitive that
        // ignored clipping: a 1x1 `overflow:hidden` sr-only box printed its
        // "Skip to content" across the page.
        let glyphs: Vec<Glyph> = glyphs.collect();
        let run_bounds = {
            let (mut x0, mut y0, mut x1, mut y1) = (
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            );
            for g in &glyphs {
                x0 = x0.min(g.x as f64);
                y0 = y0.min(g.y as f64);
                x1 = x1.max(g.x as f64);
                y1 = y1.max(g.y as f64);
            }
            if x0.is_finite() {
                // Pad by the em size: glyph origins are baseline-left, so the
                // run extends above, below and past the last origin.
                let pad = font_size as f64;
                device_bounds(
                    &Rect::new(x0 - pad, y0 - pad, x1 + pad, y1 + pad),
                    transform,
                )
            } else {
                Rect::new(0.0, 0.0, 0.0, 0.0)
            }
        };
        let clip_mask = self.clip_mask_for(run_bounds);

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
                    self.pixmap.fill_path(
                        &path,
                        &paint,
                        FillRule::Winding,
                        skia_transform,
                        clip_mask.as_deref(),
                    );
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

    /// Draw a box shadow: the (rounded) box filled with the shadow colour and
    /// blurred by the CSS blur radius.
    ///
    /// This used to fill a hard-edged rectangle at a fixed 30% of the shadow's
    /// alpha, inflated by the element's BORDER RADIUS while ignoring the blur
    /// entirely. That made every shadow the wrong colour, and made a blur-only
    /// shadow (`0 0 30px`, by far the most common form) invisible: with no
    /// blur and no spread the flat rect coincides exactly with the border box,
    /// which the caller's `shadow_clip` annulus punches out.
    ///
    /// `radius` is the box's corner radius and `std_dev` the CSS blur radius,
    /// both in device pixels. CSS specifies the shadow to ramp across a band
    /// the width of the blur radius, which is a Gaussian of half that sigma.
    fn draw_box_shadow(
        &mut self,
        transform: Affine,
        rect: Rect,
        color: Color,
        radius: f64,
        std_dev: f64,
    ) {
        let (r, g, b, a) = color_to_rgba(color);
        if a == 0 {
            return;
        }

        let sigma = (std_dev / 2.0).max(0.0) as f32;
        // Three box passes reach ~3 sigma; past that the tail is rounding noise.
        let pad = (sigma * 3.0).ceil() as i32 + 1;

        // The scratch surface is axis-aligned in device space, so the shape's
        // bounds have to be taken after the transform.
        let shape = kurbo::RoundedRect::from_rect(rect, radius.max(0.0));
        let dev = transform.transform_rect_bbox(rect);
        let x0 = dev.x0.floor() as i32 - pad;
        let y0 = dev.y0.floor() as i32 - pad;
        let w = (dev.width().ceil() as i32 + pad * 2).max(1);
        let h = (dev.height().ceil() as i32 + pad * 2).max(1);

        // Refuse absurd allocations rather than trying to blur them.
        if w > 8192 || h > 8192 {
            return;
        }

        let (Some(mut layer), Some(path)) = (Pixmap::new(w as u32, h as u32), shape_to_path(&shape))
        else {
            return;
        };

        // Filled opaque and composited at the shadow's alpha afterwards, so the
        // blur ramp is not multiplied by that alpha twice.
        layer.fill_path(
            &path,
            &make_paint(r, g, b, 255),
            FillRule::Winding,
            affine_to_transform(transform).post_translate(-x0 as f32, -y0 as f32),
            None,
        );

        box_blur(&mut layer, sigma);

        let mask = self.clip_mask_for(Rect::new(
            x0 as f64,
            y0 as f64,
            (x0 + w) as f64,
            (y0 + h) as f64,
        ));
        let paint = tiny_skia::PixmapPaint {
            opacity: a as f32 / 255.0,
            ..Default::default()
        };
        self.pixmap.draw_pixmap(
            x0,
            y0,
            layer.as_ref(),
            &paint,
            Transform::identity(),
            mask.as_deref(),
        );
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

        // Honour the active clip, exactly as `draw_image` does. Without this a
        // background image ignored `overflow:hidden` and rounded corners and
        // painted its full destination rect — the same class of bug as glyphs
        // passing `None` for the mask. Computed before `data_mut()` takes the
        // mutable borrow.
        let clip_mask = self
            .active_clip
            .as_ref()
            .and_then(|c| c.to_mask(framebuffer_width as u32, framebuffer_height as u32));
        let clip_bbox = self.active_clip.as_ref().map(|c| c.bounding_box());

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

                // Clip test: the rasterized mask when the clip has curves,
                // otherwise a bounding-box rejection.
                if let Some(ref mask) = clip_mask {
                    let mask_index =
                        (pixel_y as u32) * (framebuffer_width as u32) + (pixel_x as u32);
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
