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
        // tiny-skia does not validate points: a NaN passes `finish()` (the
        // bounds skip it) and then panics the anti-aliased rasteriser. A
        // shape with a non-finite point cannot be drawn anyway, so skip it.
        if !element.is_finite() {
            return None;
        }
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
    /// If this is an opacity layer (alpha < 1.0), what the surface looked like
    /// before the layer drew anything, plus the alpha.
    opacity: Option<OpacityGroup>,
    /// The enclosing group's dirty region, restored (and re-unioned) on pop so
    /// that the layer's own region can be measured in isolation.
    saved_dirty: Option<DirtyRect>,
    /// How many `filter: blur()` layers were capturing when this layer was
    /// pushed — i.e. which surface it composites against.
    blur_depth: usize,
}

/// The surface underneath an `opacity` group, kept one row at a time.
///
/// Group opacity is applied by re-blending the group's result against what was
/// there before, so the "before" has to survive the group. Snapshotting the
/// WHOLE framebuffer to do it costs several megabytes of memcpy for an element
/// that usually covers a sliver of the page — and the composite only ever reads
/// back the rows the group actually wrote. So rows are saved lazily, the first
/// time something draws into them, from the same place that records the group's
/// dirty region: the set of saved rows and the set of blended rows are
/// therefore the same set by construction, and cannot drift apart.
struct OpacityGroup {
    alpha: f32,
    /// Where in `rows` each surface row was saved, or `-1` for "untouched".
    slot: Vec<i32>,
    /// The saved rows, back to back.
    rows: Vec<u8>,
}

impl OpacityGroup {
    fn new(alpha: f32, height: usize) -> Self {
        Self {
            alpha,
            slot: vec![-1; height],
            rows: Vec::new(),
        }
    }

    #[inline]
    fn save_row(&mut self, y: usize, row: &[u8]) {
        if self.slot[y] >= 0 {
            return;
        }
        self.slot[y] = (self.rows.len() / row.len()) as i32;
        self.rows.extend_from_slice(row);
    }

    #[inline]
    fn row(&self, y: usize, stride: usize) -> Option<&[u8]> {
        let slot = *self.slot.get(y)?;
        if slot < 0 {
            return None;
        }
        let start = slot as usize * stride;
        Some(&self.rows[start..start + stride])
    }
}

/// Offscreen surface captured for `filter: blur()`.
struct BlurEntry {
    /// The surface the element is being painted *over*.
    saved: Pixmap,
    radius: f32,
    saved_dirty: Option<DirtyRect>,
}

/// Device-space `[x0, y0, x1, y1]` with an exclusive maximum, always clamped to
/// the framebuffer.
type DirtyRect = [i32; 4];

pub struct TinySkiaPainter {
    /// The RGBA pixel buffer that accumulates all drawing operations.
    pub pixmap: Pixmap,

    /// Stack of saved layer states, pushed/popped alongside layers.
    layer_stack: Vec<LayerEntry>,
    /// Offscreen surfaces captured for `filter: blur()`, with their radii.
    blur_stack: Vec<BlurEntry>,

    /// Bounding box of every pixel written since the current group was pushed.
    ///
    /// Group operations — the opacity composite and the `filter: blur()`
    /// composite — are whole-surface by nature: they have to touch every pixel
    /// the group could have changed. Tracking what the group ACTUALLY changed
    /// turns both from O(surface) into O(the element), which is what they cost
    /// on a real page. Every primitive unions its device-space bounds in here,
    /// padded a pixel for anti-aliasing spill and intersected with the clip
    /// (which is what actually bounds the write).
    dirty: Option<DirtyRect>,

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
            dirty: None,
        }
    }

    /// Union `bounds` (device space) into the current group's dirty region,
    /// first intersecting it with the active clip.
    ///
    /// Only sound for primitives that honour the clip — every one of them does
    /// except the group composites, which use [`mark_dirty_device`].
    #[inline]
    fn mark_dirty(&mut self, bounds: Rect) {
        let bounds = match &self.active_clip {
            Some(clip) => bounds.intersect(clip.bounding_box()),
            None => bounds,
        };
        self.mark_dirty_device(bounds);
    }

    /// Union `bounds` into the current group's dirty region without consulting
    /// the clip.
    ///
    /// MUST be called before the pixels are written: it is also where any
    /// enclosing `opacity` group saves the rows about to change.
    fn mark_dirty_device(&mut self, bounds: Rect) {
        let (w, h) = (self.pixmap.width() as i64, self.pixmap.height() as i64);
        // One pixel of slack on every side: a rasterizer writes anti-aliased
        // coverage just outside a shape's geometric bounds, and this region has
        // to be a superset of what was touched or the group composite would
        // leave a sliver of the element uncomposited. Float-to-int casts
        // saturate in Rust, so infinities and NaN land inside the clamp.
        let x0 = ((bounds.x0.floor() as i64) - 1).clamp(0, w) as i32;
        let y0 = ((bounds.y0.floor() as i64) - 1).clamp(0, h) as i32;
        let x1 = ((bounds.x1.ceil() as i64) + 1).clamp(0, w) as i32;
        let y1 = ((bounds.y1.ceil() as i64) + 1).clamp(0, h) as i32;
        if x1 <= x0 || y1 <= y0 {
            return;
        }

        // Preserve what any enclosing opacity group is about to lose.
        //
        // Only groups sitting on the CURRENT surface: `filter: blur()` swaps
        // `pixmap` for an offscreen one, so a group pushed outside that blur
        // composites against a surface these pixels are not part of, and must
        // not sample them. Its turn comes when `pop_blur_layer` records the
        // composite back onto the real surface.
        if !self.layer_stack.is_empty() {
            let depth = self.blur_stack.len();
            let stride = self.pixmap.width() as usize * 4;
            // Disjoint fields: the surface is read while the stack is updated.
            let data = self.pixmap.data();
            for entry in self.layer_stack.iter_mut() {
                if entry.blur_depth != depth {
                    continue;
                }
                let Some(group) = entry.opacity.as_mut() else {
                    continue;
                };
                for y in y0 as usize..y1 as usize {
                    let start = y * stride;
                    group.save_row(y, &data[start..start + stride]);
                }
            }
        }

        self.union_dirty([x0, y0, x1, y1]);
    }

    #[inline]
    fn union_dirty(&mut self, r: DirtyRect) {
        match &mut self.dirty {
            Some(d) => {
                d[0] = d[0].min(r[0]);
                d[1] = d[1].min(r[1]);
                d[2] = d[2].max(r[2]);
                d[3] = d[3].max(r[3]);
            }
            None => self.dirty = Some(r),
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

/// Half-width of the box window used for each of [`box_blur`]'s three passes,
/// for a Gaussian of standard deviation `sigma`.
#[inline]
fn box_blur_half_width(sigma: f32) -> usize {
    let box_w = ((sigma * 3.0 * (2.0 * std::f32::consts::PI).sqrt() / 4.0) + 0.5) as usize;
    (box_w / 2).max(1)
}

/// How far [`box_blur`] can carry a pixel's colour: three passes of half-width
/// `r` each, plus a pixel of slack.
///
/// A blur confined to a region is identical to a blur of the whole surface as
/// long as the region extends this far past the last non-transparent pixel —
/// beyond it the window sees only transparent pixels, and the edge-clamping the
/// passes do at the region border clamps transparent onto transparent.
#[inline]
fn box_blur_reach(sigma: f32) -> i32 {
    if sigma < 0.5 {
        return 0;
    }
    (box_blur_half_width(sigma) * 3) as i32 + 1
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
    let r = box_blur_half_width(radius);
    let recip = Reciprocal::new((2 * r + 1) as u32);

    let data = pixmap.data_mut();
    // A pixel at a time rather than a byte at a time. The window update is four
    // independent lanes, and grouping them lets the compiler keep a pixel in one
    // register and vectorize the pass; addressing every channel separately hid
    // that, and cost a bounds check per channel on top.
    let n = w * h;
    let mut src: Vec<[u8; 4]> = Vec::with_capacity(n);
    src.extend(data.chunks_exact(4).map(|p| [p[0], p[1], p[2], p[3]]));
    let mut dst: Vec<[u8; 4]> = vec![[0u8; 4]; n];

    for _ in 0..3 {
        blur_rows(&src, &mut dst, w, h, r, recip);
        std::mem::swap(&mut src, &mut dst);
        blur_cols(&src, &mut dst, w, h, r, recip);
        std::mem::swap(&mut src, &mut dst);
    }

    for (out, px) in data.chunks_exact_mut(4).zip(src.iter()) {
        out.copy_from_slice(px);
    }
}

/// Division by a fixed box width, as a multiply.
///
/// The box passes divide by the window width once per channel per pixel — four
/// integer divisions per pixel per pass, twenty-four per pixel per blur, and an
/// integer division is an order of magnitude slower than a multiply. This
/// reproduces `n / d` EXACTLY (not approximately) for every `n` the passes can
/// produce, so the blurred output is unchanged bit for bit.
#[derive(Clone, Copy)]
struct Reciprocal(u64);

impl Reciprocal {
    /// `m = floor(2^55 / d) + 1` gives `floor(n*m / 2^55) == floor(n / d)` for
    /// all `0 <= n <= 255*d` provided `256*d^2 < 2^55` — true for any `d` up to
    /// eleven million, where the largest a box window can be here is a few
    /// thousand. `n*m` stays under `255 * 2^55`, well inside `u64`.
    #[inline]
    fn new(d: u32) -> Self {
        Self((1u64 << 55) / d as u64 + 1)
    }

    #[inline(always)]
    fn apply(self, n: u32) -> u8 {
        ((n as u64 * self.0) >> 55) as u8
    }
}

/// One horizontal box pass: `dst[x] = mean(src[x-r ..= x+r])`, with the row's
/// end pixels replicated past its edges.
fn blur_rows(src: &[[u8; 4]], dst: &mut [[u8; 4]], w: usize, h: usize, r: usize, recip: Reciprocal) {
    for y in 0..h {
        let s = &src[y * w..y * w + w];
        let d = &mut dst[y * w..y * w + w];
        let mut acc = [0u32; 4];
        // Prime the window with the clamped left edge.
        for i in 0..=r {
            let p = s[i.min(w - 1)];
            for c in 0..4 {
                acc[c] += p[c] as u32;
            }
        }
        let first = s[0];
        for c in 0..4 {
            acc[c] += first[c] as u32 * r as u32;
        }
        for x in 0..w {
            let mut out = [0u8; 4];
            for c in 0..4 {
                out[c] = recip.apply(acc[c]);
            }
            d[x] = out;
            let add = s[(x + r + 1).min(w - 1)];
            let sub = s[x.saturating_sub(r)];
            for c in 0..4 {
                acc[c] = acc[c] + add[c] as u32 - sub[c] as u32;
            }
        }
    }
}

/// One vertical box pass.
///
/// Walked in strips of columns rather than one column at a time: a single
/// column steps through memory by a whole row per pixel, so every read is a
/// cache miss that fetches 64 bytes to use 4. A strip that wide uses the entire
/// line it pulls in.
fn blur_cols(src: &[[u8; 4]], dst: &mut [[u8; 4]], w: usize, h: usize, r: usize, recip: Reciprocal) {
    const STRIP: usize = 16;
    let mut x0 = 0;
    while x0 < w {
        let x1 = (x0 + STRIP).min(w);
        let bw = x1 - x0;
        let mut acc = [[0u32; 4]; STRIP];

        for i in 0..=r {
            let row = &src[i.min(h - 1) * w + x0..i.min(h - 1) * w + x1];
            for k in 0..bw {
                for c in 0..4 {
                    acc[k][c] += row[k][c] as u32;
                }
            }
        }
        let top = &src[x0..x1];
        for k in 0..bw {
            for c in 0..4 {
                acc[k][c] += top[k][c] as u32 * r as u32;
            }
        }

        for y in 0..h {
            let out = &mut dst[y * w + x0..y * w + x1];
            for k in 0..bw {
                for c in 0..4 {
                    out[k][c] = recip.apply(acc[k][c]);
                }
            }
            let ya = (y + r + 1).min(h - 1);
            let ys = y.saturating_sub(r);
            let add = &src[ya * w + x0..ya * w + x1];
            let sub = &src[ys * w + x0..ys * w + x1];
            for k in 0..bw {
                for c in 0..4 {
                    acc[k][c] = acc[k][c] + add[k][c] as u32 - sub[k][c] as u32;
                }
            }
        }
        x0 = x1;
    }
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
        self.blur_stack.push(BlurEntry {
            saved,
            radius: radius_px,
            saved_dirty: self.dirty.take(),
        });
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
        // Recorded before the write, and without consulting the clip, which
        // this ignores.
        self.mark_dirty_device(Rect::new(
            x0.floor() as f64,
            y0.floor() as f64,
            x1.ceil() as f64,
            y1.ceil() as f64,
        ));
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
        let Some(entry) = self.blur_stack.pop() else {
            return;
        };
        let layer = std::mem::replace(&mut self.pixmap, entry.saved);

        // What the element actually painted into the offscreen surface.
        let content = self.dirty;
        self.dirty = entry.saved_dirty;

        // Nothing was drawn: there is no glow, and no reason to blur or
        // composite a surface that is uniformly transparent.
        let Some(d) = content else {
            return;
        };

        // Blurring and compositing the WHOLE surface for one element is what
        // made a single `filter: blur()` cost as much as the rest of the frame.
        // The element's own region plus the blur's reach is all that can differ
        // from transparent, so it is all that has to be touched.
        let pad = box_blur_reach(entry.radius);
        let (w, h) = (self.pixmap.width() as i32, self.pixmap.height() as i32);
        let x0 = (d[0] - pad).max(0);
        let y0 = (d[1] - pad).max(0);
        let x1 = (d[2] + pad).min(w);
        let y1 = (d[3] + pad).min(h);
        let (Some(rect), true) = (tiny_skia::IntRect::from_ltrb(x0, y0, x1, y1), x1 > x0 && y1 > y0)
        else {
            return;
        };
        let Some(mut region) = layer.clone_rect(rect) else {
            return;
        };
        box_blur(&mut region, entry.radius);
        // Recorded before the write, and unclipped: the composite ignores the
        // clip.
        self.mark_dirty_device(Rect::new(x0 as f64, y0 as f64, x1 as f64, y1 as f64));
        self.pixmap.draw_pixmap(
            x0,
            y0,
            region.as_ref(),
            &tiny_skia::PixmapPaint::default(),
            tiny_skia::Transform::identity(),
            None,
        );
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
        self.blur_stack.clear();
        self.active_clip = None;
        self.clip_mask = None;
        self.dirty = None;
    }

    /// Push a compositing layer with a clip shape and optional opacity.
    ///
    /// When `alpha` < 1.0 the surface underneath is preserved — lazily, a row
    /// at a time as the layer paints — so that on [`pop_layer`] the newly drawn
    /// content can be alpha-blended over the original. This gives correct
    /// group-opacity semantics.
    fn push_layer(
        &mut self,
        _blend: impl Into<BlendMode>,
        alpha: f32,
        transform: Affine,
        clip: &impl Shape,
    ) {
        let shape = transform_clip_shape(transform, clip);
        let (old_clip, old_mask) = self.push_clip(shape);

        let opacity =
            (alpha < 1.0).then(|| OpacityGroup::new(alpha, self.pixmap.height() as usize));

        self.layer_stack.push(LayerEntry {
            saved_clip: old_clip,
            saved_mask: old_mask,
            opacity,
            saved_dirty: self.dirty.take(),
            blur_depth: self.blur_stack.len(),
        });
    }

    /// Push a clip-only layer (no blend mode, full opacity).
    fn push_clip_layer(&mut self, transform: Affine, clip: &impl Shape) {
        let shape = transform_clip_shape(transform, clip);
        let (old_clip, old_mask) = self.push_clip(shape);
        self.layer_stack.push(LayerEntry {
            saved_clip: old_clip,
            saved_mask: old_mask,
            opacity: None,
            saved_dirty: self.dirty.take(),
            blur_depth: self.blur_stack.len(),
        });
    }

    /// Pop the most recent layer, restoring the previous clip region.
    ///
    /// For opacity layers the preserved rows are blended back with the content
    /// drawn during the layer, at the layer's alpha value — over the region the
    /// layer actually painted, and no further.
    fn pop_layer(&mut self) {
        if let Some(entry) = self.layer_stack.pop() {
            self.active_clip = entry.saved_clip;
            self.clip_mask = entry.saved_mask;

            // What this layer painted, measured in isolation; then folded back
            // into the enclosing group, which is equally changed by it.
            let content = self.dirty;
            self.dirty = entry.saved_dirty;
            if let Some(c) = content {
                self.union_dirty(c);
            }

            if let Some(group) = entry.opacity {
                // Only the region the layer actually painted can differ from
                // what was underneath, so it is the only region worth blending
                // — walking the whole framebuffer per opacity group is what
                // made a page with a dozen of them spend more time compositing
                // than drawing.
                let Some([x0, y0, x1, y1]) = content else {
                    return;
                };
                // Blend: for each pixel, result = under + (current - under) * alpha
                //
                // `current - under` is SIGNED — the layer darkens a pixel as
                // often as it lightens one. Computed in u16 the negative case
                // wraps to ~65535 and the following multiply overflows, which
                // panicked on the first frame of any page using opacity. i32 is
                // wide enough that no intermediate can overflow: the difference
                // is within ±255 and the product within ±65025.
                let alpha = (group.alpha * 255.0).round().clamp(0.0, 255.0) as i32;
                let stride = self.pixmap.width() as usize * 4;
                let (lo, hi) = (x0 as usize * 4, x1 as usize * 4);
                // Disjoint fields: the saved rows are read while the surface is
                // written.
                let dst = self.pixmap.data_mut();
                for y in y0 as usize..y1 as usize {
                    // A row inside the group's bounding box that it never
                    // actually wrote is unchanged, and blending it would be a
                    // no-op.
                    let Some(under) = group.row(y, stride) else {
                        continue;
                    };
                    let base = y * stride;
                    for (d, s) in dst[base + lo..base + hi]
                        .iter_mut()
                        .zip(under[lo..hi].iter())
                    {
                        let under = i32::from(*s);
                        let current = i32::from(*d);
                        *d = (under + (current - under) * alpha / 255) as u8;
                    }
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
        style: Fill,
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
                    let bounds = device_bounds(shape, transform);
                    self.mark_dirty(bounds);
                    let mask = self.clip_mask_for(bounds);
                    // The fill rule was ignored -- everything filled as nonzero --
                    // so an `fill-rule="evenodd"` SVG lost its holes: rust-lang.org's
                    // logo drew as a solid disc, and many icon sets are evenodd.
                    let rule = match style {
                        Fill::NonZero => FillRule::Winding,
                        Fill::EvenOdd => FillRule::EvenOdd,
                    };
                    self.pixmap
                        .fill_path(&path, &paint, rule, ts, mask.as_deref());
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
            // The stroke's own caps, join and dashes. These were fixed at round
            // and solid, so an SVG's `stroke-linecap="butt"`, mitred corners and
            // `stroke-dasharray` all drew the same rounded, unbroken line.
            // (kurbo's defaults are round, so a plain `Stroke::new` is unchanged.)
            let cap = |cap: kurbo::Cap| match cap {
                kurbo::Cap::Butt => tiny_skia::LineCap::Butt,
                kurbo::Cap::Square => tiny_skia::LineCap::Square,
                kurbo::Cap::Round => tiny_skia::LineCap::Round,
            };
            let dash = if style.dash_pattern.is_empty() {
                None
            } else {
                tiny_skia::StrokeDash::new(
                    style.dash_pattern.iter().map(|d| *d as f32).collect(),
                    style.dash_offset as f32,
                )
            };
            let stroke = tiny_skia::Stroke {
                width: style.width as f32,
                miter_limit: style.miter_limit as f32,
                line_cap: cap(style.start_cap),
                line_join: match style.join {
                    kurbo::Join::Bevel => tiny_skia::LineJoin::Bevel,
                    kurbo::Join::Miter => tiny_skia::LineJoin::Miter,
                    kurbo::Join::Round => tiny_skia::LineJoin::Round,
                },
                dash,
            };
            let ts = affine_to_transform(transform);
            let paint = make_brush_paint(&paint_ref, brush_space(brush_transform));
            let bounds = device_bounds(shape, transform).inflate(style.width, style.width);
            self.mark_dirty(bounds);
            let mask = self.clip_mask_for(bounds);
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
        // Twice the em size for the dirty region: `run_bounds` is a good enough
        // guess for "does this run cross the clip", but a group composite needs
        // a bound it can never fall outside, and an accent or a swash can reach
        // past one em from the baseline.
        self.mark_dirty(run_bounds.inflate(font_size as f64, font_size as f64));
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

        self.mark_dirty(Rect::new(
            dest_x as f64,
            dest_y as f64,
            (dest_x + dest_width) as f64,
            (dest_y + dest_height) as f64,
        ));

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

        // Cull shadows that fall entirely outside the surface before allocating
        // and blurring a scratch pixmap for them. A long page has far more
        // shadowed cards off screen than on, and each one otherwise costs a full
        // allocation plus three box-blur passes to produce nothing visible.
        let sw = self.pixmap.width() as i32;
        let sh = self.pixmap.height() as i32;
        if x0 >= sw || y0 >= sh || x0 + w <= 0 || y0 + h <= 0 {
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

        let bounds = Rect::new(x0 as f64, y0 as f64, (x0 + w) as f64, (y0 + h) as f64);
        self.mark_dirty(bounds);
        let mask = self.clip_mask_for(bounds);
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

        // The destination is the shape's bounding box AFTER the transform.
        //
        // This used to read only the translation out of the matrix
        // (`coeffs[4]`, `coeffs[5]`) and take the extent from the shape's
        // UNTRANSFORMED bounds. The caller encodes `background-size` as the
        // matrix's scale and expresses the shape in image space to match
        // (`render/background.rs`), so dropping the linear part meant
        // background-size was ignored entirely and one copy of the image was
        // stretched over the whole positioning area.
        let bounds = shape.bounding_box();
        let coeffs = transform.as_coeffs();
        // Backgrounds are axis-aligned; skew/rotation (coeffs 1 and 2) only
        // appear via `transform`, which is handled by the layer machinery.
        let scale_x = coeffs[0] as f32;
        let scale_y = coeffs[3] as f32;
        if scale_x.abs() < 1.0e-6 || scale_y.abs() < 1.0e-6 {
            return;
        }
        let dest_x = (coeffs[0] * bounds.x0 + coeffs[2] * bounds.y0 + coeffs[4]) as f32;
        let dest_y = (coeffs[1] * bounds.x0 + coeffs[3] * bounds.y0 + coeffs[5]) as f32;
        let dest_width = (bounds.width() * coeffs[0]).abs() as f32;
        let dest_height = (bounds.height() * coeffs[3]).abs() as f32;

        let framebuffer_width = self.pixmap.width() as i32;
        let framebuffer_height = self.pixmap.height() as i32;

        // `background-repeat` reaches here as the brush's extend mode. Sampling
        // used to clamp unconditionally, which silently discarded it: the tiled
        // area was covered by one stretched copy instead of repeats.
        let wrap_x = matches!(img.sampler.x_extend, peniko::Extend::Repeat);
        let wrap_y = matches!(img.sampler.y_extend, peniko::Extend::Repeat);

        self.mark_dirty(Rect::new(
            dest_x as f64,
            dest_y as f64,
            (dest_x + dest_width) as f64,
            (dest_y + dest_height) as f64,
        ));

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

                // Device offset back into image space, then wrapped (repeat) or
                // clamped (pad) into the source.
                let u = col as f32 / scale_x.abs();
                let v = row as f32 / scale_y.abs();
                let src_col = if wrap_x {
                    (u as i64).rem_euclid(image_width as i64) as u32
                } else {
                    (u as u32).min(image_width - 1)
                };
                let src_row = if wrap_y {
                    (v as i64).rem_euclid(image_height as i64) as u32
                } else {
                    (v as u32).min(image_height - 1)
                };

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

/// How the picture goes onto the screen: the quarter turns clockwise it is
/// turned by (a panel mounted on its side, as a GPD Win Max's is), and the
/// size of the screen's buffers then -- the picture's turned, or wider where
/// a driver pads its rows. (0, 0, 0) for a picture shown as it is.
static SCREEN_TRANSFORM: std::sync::Mutex<(u8, u32, u32)> = std::sync::Mutex::new((0, 0, 0));

/// Turn what windows draw by `rotation` quarter turns clockwise onto buffers
/// of `width` x `height` (zipp-kms says both). The window's own size, which
/// the page is laid out at, is the picture's.
pub fn set_screen_transform(rotation: u8, width: u32, height: u32) {
    *SCREEN_TRANSFORM.lock().unwrap_or_else(|e| e.into_inner()) = (rotation % 4, width, height);
}

fn screen_transform() -> (u8, u32, u32) {
    *SCREEN_TRANSFORM.lock().unwrap_or_else(|e| e.into_inner())
}

/// The surface's size for a picture of `width` x `height`: the screen's
/// buffers' when the picture is turned.
fn surface_size(width: u32, height: u32) -> (NonZeroU32, NonZeroU32) {
    let (rotation, bw, bh) = screen_transform();
    let (w, h) = if rotation != 0 && bw > 0 && bh > 0 { (bw, bh) } else { (width, height) };
    (NonZeroU32::new(w.max(1)).unwrap(), NonZeroU32::new(h.max(1)).unwrap())
}

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
        let (w, h) = surface_size(width, height);
        let _ = surface.resize(w, h);

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
            let (w, h) = surface_size(width, height);
            let _ = surface.resize(w, h);
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
        let (rotation, buffer_width, _) = screen_transform();

        if rotation == 0 {
            for (i, chunk) in rgba_pixels.chunks_exact(4).enumerate().take(pixel_count) {
                // Pack as 0x00RRGGBB (red in bits 16-23, green in 8-15, blue in 0-7)
                buffer[i] = ((chunk[0] as u32) << 16)
                    | ((chunk[1] as u32) << 8)
                    | (chunk[2] as u32);
            }
        } else {
            // Turned onto the panel as it copies: the picture's (x, y) is the
            // panel's (px, py), the panel's own size being the picture's turned.
            let (lw, lh) = (self.width as usize, self.height as usize);
            let (pw, ph) = if rotation % 2 == 1 { (lh, lw) } else { (lw, lh) };
            let stride = buffer_width as usize;
            let len = buffer.len();
            for (i, chunk) in rgba_pixels.chunks_exact(4).enumerate().take(lw * lh) {
                let (x, y) = (i % lw, i / lw);
                let (px, py) = match rotation {
                    1 => (pw - 1 - y, x),
                    2 => (pw - 1 - x, ph - 1 - y),
                    _ => (y, ph - 1 - x),
                };
                let at = py * stride + px;
                if at < len {
                    buffer[at] = ((chunk[0] as u32) << 16) | ((chunk[1] as u32) << 8) | (chunk[2] as u32);
                }
            }
        }

        let _ = buffer.present();
    }
}
