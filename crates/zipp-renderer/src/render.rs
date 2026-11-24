//! Core document painting logic.
//!
//! [`DocumentPainter`] walks the resolved DOM tree emitting draw commands for
//! backgrounds, borders, text, images, and child elements. It coordinates
//! clipping layers, opacity, CSS transforms, and the debug overlay.

mod background;
mod box_shadow;
mod form_controls;

use std::any::Any;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};

use super::kurbo_css::{CssBox, Edge};
use crate::SELECTION_COLOR;
use crate::color::{Color, ToColorColor};
use crate::debug_overlay::render_debug_overlay;
use crate::kurbo_css::NonUniformRoundedRectRadii;
use crate::layers::LayerManager;
use crate::sizing::compute_object_fit;
use anyrender::{CustomPaint, Paint, PaintScene};
use crate::tinyskia::BlurLayer;
use zipp_dom::node::{
    ListItemLayout, ListItemLayoutPosition, Marker, NodeData, RasterImageData, SpecialElementData,
    TextInputData, TextNodeData,
};
use zipp_dom::{BaseDocument, ElementData, Node, local_name};
use zipp_dom::traits_src::devtools::DevtoolSettings;

use style::values::computed::BorderCornerRadius;
use style::{
    computed_values::border_collapse::T as BorderCollapse,
    dom::TElement,
    properties::{
        ComputedValues, generated::longhands::visibility::computed_value::T as StyloVisibility,
        style_structs::Font,
    },
    values::{
        computed::{CSSPixelLength, Overflow},
        specified::{BorderStyle, OutlineStyle, image::ImageRendering},
    },
};

use kurbo::{self, Affine, BezPath, Insets, Point, Rect, Stroke, Vec2};
use peniko::{self, Fill, ImageData, ImageSampler};
use style::values::generics::color::{ColorOrAuto, GenericColor};
use taffy::Layout;

/// Short-lived painter that walks a resolved [`BaseDocument`] and emits draw
/// commands into a [`PaintScene`]. Created once per frame and discarded after
/// [`paint_scene`](Self::paint_scene) returns.
pub struct DocumentPainter<'dom> {
    /// The document whose tree is being painted.
    pub(crate) dom: &'dom BaseDocument,
    pub(crate) scale: f64,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) initial_x: f64,
    pub(crate) initial_y: f64,
    pub(crate) layer_manager: LayerManager,
    /// Cached selection ranges for O(1) lookup: node_id -> (start_offset, end_offset)
    pub(crate) selection_ranges: HashMap<usize, (usize, usize)>,
    /// Cached root background color (resolved from <html> or <body> once at
    /// construction time to avoid DOM traversal on every frame).
    pub(crate) cached_bg_color: Option<Color>,
    /// Optional partial-repaint filter: `(target_nodes, ancestor_nodes)`.
    /// Elements strictly prior to the first target node skip drawing while
    /// ancestors still push clips and scroll offsets.
    pub(crate) skip_targets: Option<(HashSet<usize>, HashSet<usize>)>,
    pub(crate) skip_reached: Cell<bool>,
}

impl<'dom> DocumentPainter<'dom> {
    /// Create a new DocumentPainter for the given document
    pub fn new(
        dom: &'dom BaseDocument,
        scale: f64,
        width: u32,
        height: u32,
        initial_x: f64,
        initial_y: f64,
    ) -> Self {
        let selection_ranges: HashMap<usize, (usize, usize)> = dom
            .get_text_selection_ranges()
            .into_iter()
            .map(|(node_id, start, end)| (node_id, (start, end)))
            .collect();

        let layer_manager = LayerManager::default();

        // Pre-compute root background color once to avoid DOM traversal
        // on every frame. Checks <html> first, falls back to <body>.
        let cached_bg_color = {
            let root_element = dom.as_ref().root_element();
            let html_color = root_element
                .primary_styles()
                .map(|s| s.clone_background_color())
                .unwrap_or(GenericColor::TRANSPARENT_BLACK);
            if html_color == GenericColor::TRANSPARENT_BLACK {
                root_element
                    .children
                    .iter()
                    .find_map(|id| {
                        dom.as_ref()
                            .get_node(*id)
                            .filter(|node| node.data.is_element_with_tag_name(&local_name!("body")))
                    })
                    .and_then(|body| body.primary_styles())
                    .map(|style| {
                        let current_color = style.clone_color();
                        style
                            .clone_background_color()
                            .resolve_to_absolute(&current_color)
                            .as_srgb_color()
                    })
            } else {
                let current_color = root_element.primary_styles().unwrap().clone_color();
                Some(html_color.resolve_to_absolute(&current_color).as_srgb_color())
            }
        };

        // A root whose used colour scheme is dark has a dark canvas, as in
        // browsers. Without it a `color-scheme: light dark` page with no
        // background of its own put its light `CanvasText` on white. The rule
        // matches stylo's `is_dark_color_scheme` (see third_party/stylo).
        let root_is_dark = dom.as_ref().root_element().primary_styles().is_some_and(|style| {
            use style::values::specified::color::ColorSchemeFlags;
            let bits = style.clone_color_scheme().bits;
            match (bits.contains(ColorSchemeFlags::LIGHT), bits.contains(ColorSchemeFlags::DARK)) {
                (false, true) => true,
                (true, true) => {
                    dom.as_ref().viewport().color_scheme
                        == zipp_dom::traits_src::shell::ColorScheme::Dark
                }
                _ => false,
            }
        });
        let cached_bg_color = match cached_bg_color {
            Some(color) if color.components[3] > 0.0 => Some(color),
            // Stylo's dark `Canvas`.
            _ if root_is_dark => Some(Color::from_rgb8(30, 30, 30)),
            other => other,
        };

        Self {
            dom,
            scale,
            width,
            height,
            initial_x,
            initial_y,
            layer_manager,
            selection_ranges,
            cached_bg_color,
            skip_targets: None,
            skip_reached: Cell::new(false),
        }
    }

    pub fn set_skip_until_nodes(&mut self, targets: &[usize]) {
        if targets.is_empty() {
            return;
        }
        let mut target_set = HashSet::with_capacity(targets.len());
        let mut ancestor_set = HashSet::new();
        for &tid in targets {
            target_set.insert(tid);
            let mut cur = self.dom.as_ref().get_node(tid).and_then(|n| n.parent);
            while let Some(pid) = cur {
                if !ancestor_set.insert(pid) {
                    break;
                }
                cur = self.dom.as_ref().get_node(pid).and_then(|n| n.parent);
            }
        }
        self.skip_targets = Some((target_set, ancestor_set));
        self.skip_reached.set(false);
    }

    #[inline]
    fn node_position(&self, node: usize, location: Point) -> (Layout, Point) {
        let layout = self.layout(node);
        let pos = location + Vec2::new(layout.location.x as f64, layout.location.y as f64);
        (layout, pos)
    }

    #[inline]
    fn layout(&self, child: usize) -> Layout {
        self.dom
            .as_ref()
            .tree()
            .get(child)
            .map(|n| n.final_layout)
            .unwrap_or_default()
    }

    /// Draw the current tree to current render surface
    /// Eventually we'll want the surface itself to be passed into the render function, along with things like the viewport
    ///
    /// This assumes styles are resolved and layout is complete.
    /// Make sure you do those before trying to render
    pub fn paint_scene(&self, scene: &mut (impl PaintScene + BlurLayer)) {
        // Render starting from the root element (not the root node).
        let root_element = self.dom.as_ref().root_element();
        let bg_width = (self.width as f32).max(root_element.final_layout.size.width);
        let bg_height = (self.height as f32).max(root_element.final_layout.size.height);

        // Paint the background using the cached color (computed once in new()).
        if self.skip_targets.is_none() {
            if let Some(bg_color) = self.cached_bg_color {
                let rect = Rect::from_origin_size(
                    (self.initial_x * self.scale, self.initial_y * self.scale),
                    (bg_width as f64, bg_height as f64),
                );
                scene.fill(Fill::NonZero, Affine::IDENTITY, bg_color, None, &rect);
            }
        }

        // When critical resources are still loading, show the background
        // instead of a blank/frozen screen but skip the full element tree.
        if self.dom.has_pending_critical_resources() {
            return;
        }

        let viewport_scroll = self.dom.as_ref().viewport_scroll();
        let root_id = root_element.id;

        self.render_element(
            scene,
            root_id,
            Point {
                x: self.initial_x - viewport_scroll.x,
                y: self.initial_y - viewport_scroll.y,
            },
            Affine::IDENTITY,
        );

        // Render debug overlay
        if self.dom.devtools().highlight_hover {
            if let Some(node_id) = self.dom.as_ref().get_hover_node_id() {
                render_debug_overlay(
                    scene,
                    self.dom,
                    node_id,
                    self.scale,
                    self.initial_x,
                    self.initial_y,
                );
            }
        }
    }

    /// Renders a node, but is guaranteed that the node is an element
    /// This is because the font_size is calculated from layout resolution and all text is rendered directly here, instead
    /// of a separate text stroking phase.
    ///
    /// In this renderer, text styling gets its attributes from its container element/resolved styles
    /// In other libraries, text gets its attributes from a `text` element - this is not how HTML works.
    ///
    /// Approaching rendering this way guarantees we have all the styles we need when rendering text with not having
    /// to traverse back to the parent for its styles, or needing to pass down styles
    fn render_element(
        &self,
        scene: &mut (impl PaintScene + BlurLayer),
        node_id: usize,
        location: Point,
        ancestor_transform: Affine,
    ) {
        let mut ancestor_passthrough = false;
        if let Some((ref targets, ref ancestors)) = self.skip_targets {
            if !self.skip_reached.get() {
                if targets.contains(&node_id) {
                    self.skip_reached.set(true);
                } else if ancestors.contains(&node_id) {
                    ancestor_passthrough = true;
                } else {
                    return;
                }
            }
        }

        let Some(node) = self.dom.as_ref().tree().get(node_id) else {
            return;
        };

        // Early return if the element is hidden
        if matches!(node.style.display, taffy::Display::None) {
            return;
        }

        // Only draw elements with a style
        let Some(styles) = node.primary_styles() else {
            return;
        };

        // Hide inputs with type=hidden
        // Implemented here rather than using the style engine for performance reasons
        if node.local_name() == "input" && node.attr(local_name!("type")) == Some("hidden") {
            return;
        }

        // Hide elements with a visibility style other than visible
        if styles.get_inherited_box().visibility != StyloVisibility::Visible {
            return;
        }

        // Opacity: hide fully transparent elements; partial opacity uses layer compositing
        let opacity = styles.get_effects().opacity;
        // `ZIPP_DUMP_OPACITY=1` lists every translucent element. An element that
        // is laid out correctly but invisible looks exactly like a missing
        // element, and this is the quickest way to tell those apart. Read once:
        // this is per-node, per-frame.
        static DUMP_OPACITY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if opacity < 1.0
            && *DUMP_OPACITY.get_or_init(|| std::env::var_os("ZIPP_DUMP_OPACITY").is_some())
        {
            eprintln!(
                "[OPACITY] {:.3} node={} <{}>",
                opacity,
                node.id,
                node.node_debug_str().chars().take(80).collect::<String>()
            );
        }
        if opacity == 0.0 {
            return;
        }
        let has_opacity = opacity < 1.0;

        // TODO: account for overflow_x vs overflow_y
        let overflow_x = styles.get_box().overflow_x;
        let overflow_y = styles.get_box().overflow_y;
        let el_data = node.element_data();
        let is_image = el_data
            .and_then(|e| e.raster_image_data())
            .is_some();
        let is_sub_doc = el_data
            .and_then(|el| el.sub_doc_data())
            .is_some();
        let is_text_input = el_data
            .and_then(|el| el.text_input_data())
            .is_some();
        // `overflow` on the root element is the viewport's (CSS Overflow 3,
        // "propagation"), which the surface's own edges already clip: clipping
        // `<html>`'s box too hid what is in its border -- the browser's
        // toolbar, in the band its top border reserves -- on every page that
        // sets `html { overflow: hidden }`, as full-screen apps do.
        let is_root = node_id == self.dom.as_ref().root_element().id;
        let should_clip = is_image
            || is_sub_doc
            || is_text_input
            || (!is_root && (!matches!(overflow_x, Overflow::Visible) || !matches!(overflow_y, Overflow::Visible)));

        // Apply padding/border offset to inline root
        let (layout, box_position) = self.node_position(node_id, location);
        let taffy::Layout {
            size,
            border,
            padding,
            content_size,
            ..
        } = node.final_layout;
        let scaled_pb = (padding + border).map(f64::from);
        let content_position = kurbo::Point {
            x: scaled_pb.left,
            y: scaled_pb.top,
        };
        let content_box_size = kurbo::Size {
            width: (size.width as f64 - scaled_pb.left - scaled_pb.right) * self.scale,
            height: (size.height as f64 - scaled_pb.top - scaled_pb.bottom) * self.scale,
        };

        // Don't render things that are out of view (only valid when neither an
        // ancestor nor this element applies a CSS transform, since a transform
        // like `translate(-50%, -50%)` or `translateY(...)` can move an element
        // whose layout position is off-screen right back into the viewport).
        if ancestor_transform == Affine::IDENTITY && !zipp_dom::has_2d_transform(styles.get_box()) {
            let scaled_y = (box_position.y - self.initial_y) * self.scale;
            let scaled_content_height = content_size.height.max(size.height) as f64 * self.scale;
            if scaled_y > self.height as f64 || scaled_y + scaled_content_height < 0.0 {
                return;
            }
        }

        // Optimise zero-area (/very small area) clips by not rendering at all
        let clip_area = content_box_size.width * content_box_size.height;
        if should_clip && clip_area < 0.01 {
            return;
        }

        if ancestor_passthrough {
            let mut cx = self.element_cx(node, layout, box_position, ancestor_transform);
            let clip = if is_text_input {
                &cx.frame.content_box_path()
            } else {
                &cx.frame.padding_box_path()
            };
            self.layer_manager.maybe_with_layer(
                scene,
                should_clip,
                1.0,
                cx.transform,
                clip,
                |scene| {
                    cx.pos = Point {
                        x: cx.pos.x - node.scroll_offset.x,
                        y: cx.pos.y - node.scroll_offset.y,
                    };
                    cx.draw_children(scene);
                },
            );
            return;
        }

        // `filter: blur()`. Painted into an offscreen surface and blurred on
        // pop, so a decorative glow reads as a soft wash instead of the hard
        // opaque disc it used to be.
        let blur_radius: Option<f32> = styles
            .get_effects()
            .filter
            .0
            .iter()
            .find_map(|f| match f {
                style::values::computed::Filter::Blur(len) => {
                    let px = len.0.px();
                    (px >= 0.5).then_some(px * self.scale as f32)
                }
                _ => None,
            });
        // `backdrop-filter: blur()` — blur what is already behind this element
        // before painting it. Done BEFORE `push_blur_layer` so the backdrop is
        // read from the real surface rather than an empty offscreen one.
        let backdrop_blur: Option<f32> = styles
            .get_effects()
            .backdrop_filter
            .0
            .iter()
            .find_map(|f| match f {
                style::values::computed::Filter::Blur(len) => {
                    let px = len.0.px();
                    (px >= 0.5).then_some(px * self.scale as f32)
                }
                _ => None,
            });
        if let Some(r) = backdrop_blur {
            let x0 = (box_position.x * self.scale) as f32;
            let y0 = ((box_position.y - self.initial_y) * self.scale) as f32;
            scene.apply_backdrop_blur(
                r,
                (
                    x0,
                    y0,
                    x0 + (size.width as f64 * self.scale) as f32,
                    y0 + (size.height as f64 * self.scale) as f32,
                ),
            );
        }

        if let Some(r) = blur_radius {
            scene.push_blur_layer(r);
        }

        let mut cx = self.element_cx(node, layout, box_position, ancestor_transform);

        cx.draw_outline(scene);
        cx.draw_outset_box_shadow(scene);

        // Opacity layer if box has opacity.
        //
        // The layer must NOT clip. It used to be given `border_box_path()` as
        // its clip shape, which the backend installs as a real clip region for
        // everything drawn inside -- including `draw_children`. Any descendant
        // that legitimately paints outside the parent's border box (a shadow, a
        // negative margin, an absolutely positioned child, a transform) was cut
        // off the moment the parent had `opacity` on it. `opacity` groups its
        // subtree for compositing; it does not clip it. Overflow clipping is
        // applied separately by the `should_clip` layer just below, which is
        // where it belongs.
        let unclipped = cx.frame.border_box.inflate(1.0e5, 1.0e5);
        self.layer_manager.maybe_with_layer(
            scene,
            has_opacity,
            opacity,
            cx.transform,
            &unclipped,
            |scene| {
                cx.draw_background(scene);
                cx.draw_inset_box_shadow(scene);
                cx.draw_table_row_backgrounds(scene);
                cx.draw_table_borders(scene);
                cx.draw_border(scene);
                cx.stroke_devtools(scene);

                // TODO: allow layers with opacity to be unclipped (overflow: visible)
                let clip = if is_text_input {
                    &cx.frame.content_box_path()
                } else {
                    &cx.frame.padding_box_path()
                };

                // Clip layer if box requires clipping. Opacity set to 1.0
                self.layer_manager.maybe_with_layer(
                    scene,
                    should_clip,
                    1.0, // opacity
                    cx.transform,
                    clip,
                    |scene| {
                        // Now that background has been drawn, offset pos and cx in order to draw our contents scrolled
                        let content_position = Point {
                            x: content_position.x - node.scroll_offset.x,
                            y: content_position.y - node.scroll_offset.y,
                        };
                        cx.pos = Point {
                            x: cx.pos.x - node.scroll_offset.x,
                            y: cx.pos.y - node.scroll_offset.y,
                        };
                        // Not the transform as well: the element's own content
                        // (its text, a text field's) is drawn at a position
                        // already scrolled, and scrolled twice, a <pre>'s lines
                        // moved at double the speed of the text area under it.
                        cx.draw_image(scene);
                        cx.draw_media_controls(scene);
                        #[cfg(feature = "svg")]
                        cx.draw_svg(scene);
                        cx.draw_canvas(scene);
                        cx.draw_sub_document(scene);
                        cx.draw_input(scene);
                        cx.draw_text_input_text(scene, content_position);
                        cx.draw_inline_layout(scene, content_position);
                        cx.draw_select_arrow(scene);
                        cx.draw_marker(scene, content_position);
                        cx.draw_children(scene);
                    },
                );
            },
        );

        if blur_radius.is_some() {
            scene.pop_blur_layer();
        }
    }

    #[inline]
    fn render_node(
        &self,
        scene: &mut (impl PaintScene + BlurLayer),
        node_id: usize,
        location: Point,
        ancestor_transform: Affine,
    ) {
        let Some(node) = self.dom.as_ref().tree().get(node_id) else {
            return;
        };

        match &node.data {
            NodeData::Element(_) | NodeData::AnonymousBlock(_) => {
                self.render_element(scene, node_id, location, ancestor_transform)
            }
            NodeData::Text(TextNodeData { .. }) => {
                // Text nodes should never be rendered directly
                // (they should always be rendered as part of an inline layout)
                // unreachable!()
            }
            NodeData::Document => {}
            // NodeData::Doctype => {}
            NodeData::Comment => {} // NodeData::ProcessingInstruction { .. } => {}
        }
    }

    fn element_cx<'w>(
        &'w self,
        node: &'w Node,
        layout: Layout,
        box_position: Point,
        ancestor_transform: Affine,
    ) -> ElementCx<'w> {
        let style = node
            .stylo_element_data
            .primary_styles()
            .as_ref()
            .map(|styles| (*styles).clone())
            .unwrap_or(
                ComputedValues::initial_values_with_font_override(Font::initial_values()).to_arc(),
            );

        let scale = self.scale;

        // todo: maybe cache this so we don't need to constantly be figuring it out
        // It is quite a bit of math to calculate during render/traverse
        // Also! we can cache the bezpaths themselves, saving us a bunch of work
        let frame = create_css_rect(&style, &layout, scale);

        // the bezpaths for every element are (potentially) cached (not yet, tbd)
        // By performing the transform, we prevent the cache from becoming invalid when the page shifts around
        // `box_position` is an ABSOLUTE document coordinate, so the ancestor
        // matrix must be expressed about the document origin too — see
        // `child_transform` below for how it is built.
        let node_local = Affine::translate(box_position.to_vec2() * scale);
        let mut transform = ancestor_transform * node_local;

        // Reference box for resolve percentage transforms
        let reference_box = euclid::Rect::new(
            euclid::Point2D::new(CSSPixelLength::new(0.0), CSSPixelLength::new(0.0)),
            euclid::Size2D::new(
                CSSPixelLength::new(frame.border_box.width() as f32),
                CSSPixelLength::new(frame.border_box.height() as f32),
            ),
        );

        // Apply CSS transform property (where transforms are 2d)
        //
        // TODO: Handle hit testing correctly for transformed nodes
        let style_transform =
            zipp_dom::resolve_2d_transform(style.get_box(), reference_box, scale);
        if let Some(style_transform) = style_transform {
            transform *= style_transform;
        }

        // What descendants inherit. A CSS transform applies about its own
        // element's origin, but children are positioned in ABSOLUTE document
        // coordinates, so the parent's own translate has to be undone or every
        // ancestor offset would be applied twice and nested content would fly
        // off-screen:
        //
        //     child = ancestor * translate(P_parent) * T_parent
        //                      * translate(-P_parent)          <- undo
        //                      * translate(P_child) * T_child
        //
        // With no CSS transform anywhere this collapses to the identity, so
        // ordinary pages are unaffected.
        //
        // Until this existed, `transform: translateY(...)` moved only the
        // element's own background and border: descendants stayed put, so
        // `.rise`/`.reveal` entrance animations faded in without ever rising —
        // and because the opacity layer's CLIP was transformed while its
        // contents were not, the leading edge of the content was erased.
        let child_transform = match style_transform {
            Some(_) => transform * node_local.inverse(),
            // Fast path: nothing to inherit, so do not build a matrix per node.
            None => ancestor_transform,
        };

        let element = node.element_data().unwrap();

        ElementCx {
            child_transform,
            context: self,
            frame,
            scale,
            style,
            pos: box_position,
            node,
            element,
            transform,
            #[cfg(feature = "svg")]
            svg: element.svg_data(),
            text_input: element.text_input_data(),
            list_item: element.list_item_data.as_deref(),
            devtools: self.dom.devtools(),
        }
    }
}

fn to_image_quality(image_rendering: ImageRendering) -> peniko::ImageQuality {
    match image_rendering {
        ImageRendering::Auto => peniko::ImageQuality::Medium,
        ImageRendering::CrispEdges => peniko::ImageQuality::Low,
        ImageRendering::Pixelated => peniko::ImageQuality::Low,
    }
}

/// Ensure that the `resized_image` field has a correctly sized image
fn to_peniko_image(image: &RasterImageData, quality: peniko::ImageQuality) -> peniko::ImageBrush {
    peniko::ImageBrush {
        image: ImageData {
            data: image.data.clone(),
            format: peniko::ImageFormat::Rgba8,
            width: image.width,
            height: image.height,
            alpha_type: peniko::ImageAlphaType::Alpha,
        },
        sampler: ImageSampler {
            x_extend: peniko::Extend::Repeat,
            y_extend: peniko::Extend::Repeat,
            quality,
            alpha: 1.0,
        },
    }
}

/// A context of loaded and hot data to draw the element from
struct ElementCx<'a> {
    context: &'a DocumentPainter<'a>,
    frame: CssBox,
    style: style::servo_arc::Arc<ComputedValues>,
    pos: Point,
    scale: f64,
    node: &'a Node,
    element: &'a ElementData,
    transform: Affine,
    /// The matrix descendants compose with — this element's `transform` with its
    /// own absolute-position translate removed. See `element_cx`.
    child_transform: Affine,
    #[cfg(feature = "svg")]
    svg: Option<&'a usvg::Tree>,
    text_input: Option<&'a TextInputData>,
    list_item: Option<&'a ListItemLayout>,
    devtools: &'a DevtoolSettings,
}

/// Converts parley BoundingBox into peniko Rect
fn convert_rect(rect: &parley::BoundingBox) -> kurbo::Rect {
    peniko::kurbo::Rect::new(rect.x0, rect.y0, rect.x1, rect.y1)
}

impl ElementCx<'_> {
    fn draw_inline_layout(&self, scene: &mut (impl PaintScene + BlurLayer), pos: Point) {
        if self.node.flags.is_inline_root() {
            let text_layout = self.element
                .inline_layout_data
                .as_ref()
                .unwrap_or_else(|| {
                    panic!("Tried to render node marked as inline root that does not have an inline layout: {:?}", self.node);
                });

            // Down its box by its offset (a button's text, centred in it).
            let y = pos.y + f64::from(text_layout.v_offset);
            let transform =
                Affine::translate((pos.x * self.scale, y * self.scale)) * self.transform;

            // Render text selection highlight (if any) using cached selection ranges
            if let Some(&(sel_start, sel_end)) = self.context.selection_ranges.get(&self.node.id) {
                crate::text::draw_text_selection(
                    scene,
                    &text_layout.layout,
                    transform,
                    sel_start,
                    sel_end,
                );
            }

            // Backgrounds of inline elements, behind the glyphs. Inline
            // elements are parley style spans rather than Taffy nodes, so the
            // ordinary box-painting path never reaches them.
            crate::text::draw_inline_backgrounds(
                scene,
                text_layout.layout.lines(),
                self.context.dom,
                transform,
            );

            // Render text
            crate::text::draw_text_runs(
                scene,
                text_layout.layout.lines(),
                self.context.dom,
                transform,
            );
        }
    }

    fn draw_text_input_text(&self, scene: &mut (impl PaintScene + BlurLayer), pos: Point) {
        // Render the text in text inputs
        if let Some(input_data) = self.text_input {
            // For single-line inputs, add an offset to vertically center the text input layout
            // within the content box of it's node.
            let y_offset = self.node.text_input_v_centering_offset(self.scale);
            let pos = Point {
                x: pos.x,
                y: pos.y + y_offset,
            };

            let transform =
                Affine::translate((pos.x * self.scale, pos.y * self.scale)) * self.transform;

            // A field never given a value holds the single space its editor
            // starts with: empty, as far as what was typed goes.
            let text = input_data.editor.text().to_string();
            let empty = text.is_empty() || (text == " " && self.node.attr(zipp_dom::local_name!("value")).is_none());
            // A password: evenly spaced dots, the caret and selection placed
            // by how many characters come before them.
            let secret = self
                .node
                .attr(zipp_dom::local_name!("type"))
                .is_some_and(|t| t.eq_ignore_ascii_case("password"));
            let mask = if secret && !empty {
                crate::text::PasswordMask::new(input_data.editor.try_layout().unwrap().lines(), self.context.dom)
            } else {
                None
            };
            let before = |index: usize| text.get(..index).map_or_else(|| text.chars().count(), |s| s.chars().count());

            if self.node.is_focussed() {
                // Render selection/caret
                if let Some(mask) = &mask {
                    let selection = input_data.editor.raw_selection();
                    let (a, b) = (before(selection.anchor().index()), before(selection.focus().index()));
                    if a != b {
                        let rect = kurbo::Rect::new(mask.x(a.min(b)), mask.top, mask.x(a.max(b)), mask.bottom);
                        scene.fill(Fill::NonZero, transform, SELECTION_COLOR, None, &rect);
                    }
                    let color = self.style.get_inherited_text().color;
                    let caret_color = match &self.style.get_inherited_ui().caret_color.0 {
                        ColorOrAuto::Auto => color,
                        ColorOrAuto::Color(caret_color) => caret_color.resolve_to_absolute(&color),
                    };
                    let caret = kurbo::Rect::new(mask.x(b), mask.top, mask.x(b) + 1.5, mask.bottom);
                    scene.fill(Fill::NonZero, transform, caret_color.as_srgb_color(), None, &caret);
                }
                for (rect, _line_idx) in input_data.editor.selection_geometry().iter().filter(|_| mask.is_none()) {
                    scene.fill(
                        Fill::NonZero,
                        transform,
                        SELECTION_COLOR,
                        None,
                        &convert_rect(rect),
                    );
                }
                if let Some(cursor) = input_data.editor.cursor_geometry(1.5).filter(|_| mask.is_none()) {
                    let color = self.style.get_inherited_text().color;
                    let caret_color = match &self.style.get_inherited_ui().caret_color.0 {
                        ColorOrAuto::Auto => color,
                        ColorOrAuto::Color(caret_color) => caret_color.resolve_to_absolute(&color),
                    };

                    scene.fill(
                        Fill::NonZero,
                        transform,
                        caret_color.as_srgb_color(),
                        None,
                        &convert_rect(&cursor),
                    );
                };
            }

            // Its placeholder, while it is empty.
            if empty
                && let Some(layout) = input_data.placeholder.as_ref().and_then(|p| p.try_layout())
            {
                crate::text::draw_text_runs_faded(scene, layout.lines(), self.context.dom, transform, 0.55);
            }

            // Render text: a password as its dots.
            if let Some(mask) = &mask {
                mask.draw(scene, transform, text.chars().count());
            } else if !secret {
                crate::text::draw_text_runs(
                    scene,
                    input_data.editor.try_layout().unwrap().lines(),
                    self.context.dom,
                    transform,
                );
            }
        }
    }

    fn draw_marker(&self, scene: &mut (impl PaintScene + BlurLayer), pos: Point) {
        if let Some(ListItemLayout {
            marker,
            position: ListItemLayoutPosition::Outside(layout),
        }) = self.list_item
        {
            // Right align and pad the bullet when rendering outside
            let x_padding = match marker {
                Marker::Char(_) => 8.0,
                Marker::String(_) => 0.0,
            };
            let x_offset = -(layout.full_width() / layout.scale() + x_padding);

            // Align the marker with the baseline of the first line of text in the list item
            let y_offset = if let Some(first_text_line) = &self
                .element
                .inline_layout_data
                .as_ref()
                .and_then(|text_layout| text_layout.layout.lines().next())
            {
                (first_text_line.metrics().baseline
                    - layout.lines().next().unwrap().metrics().baseline)
                    / layout.scale()
            } else {
                0.0
            };

            let pos = Point {
                x: pos.x + x_offset as f64,
                y: pos.y + y_offset as f64,
            };

            let transform =
                Affine::translate((pos.x * self.scale, pos.y * self.scale)) * self.transform;

            crate::text::draw_text_runs(scene, layout.lines(), self.context.dom, transform);
        }
    }

    fn draw_children(&self, scene: &mut (impl PaintScene + BlurLayer)) {
        // Negative z_index hoisted nodes
        if let Some(hoisted) = &self.node.stacking_context {
            for hoisted_child in hoisted.neg_z_hoisted_children() {
                let pos = kurbo::Point {
                    x: self.pos.x + hoisted_child.position.x as f64,
                    y: self.pos.y + hoisted_child.position.y as f64,
                };
                self.render_node(scene, hoisted_child.node_id, pos, self.child_transform);
            }
        }

        // Regular children — render_node/render_element handles position via node_position()
        if let Some(children) = &*self.node.paint_children.borrow() {
            for child_id in children {
                self.render_node(scene, *child_id, self.pos, self.child_transform);
            }
        }

        // Positive z_index hoisted nodes
        if let Some(hoisted) = &self.node.stacking_context {
            for hoisted_child in hoisted.pos_z_hoisted_children() {
                let pos = kurbo::Point {
                    x: self.pos.x + hoisted_child.position.x as f64,
                    y: self.pos.y + hoisted_child.position.y as f64,
                };
                self.render_node(scene, hoisted_child.node_id, pos, self.child_transform);
            }
        }
    }

    #[cfg(feature = "svg")]
    fn draw_svg(&self, scene: &mut (impl PaintScene + BlurLayer)) {
        use style::properties::generated::longhands::object_fit::computed_value::T as ObjectFit;

        let Some(svg) = self.svg else {
            return;
        };

        let width = self.frame.content_box.width() as u32;
        let height = self.frame.content_box.height() as u32;
        let svg_size = svg.size();

        let x = self.frame.content_box.origin().x;
        let y = self.frame.content_box.origin().y;

        let object_position = self.style.clone_object_position();

        // Apply object-fit algorithm
        let container_size = taffy::Size {
            width: width as f32,
            height: height as f32,
        };
        let object_size = taffy::Size {
            width: svg_size.width(),
            height: svg_size.height(),
        };
        let paint_size = compute_object_fit(container_size, Some(object_size), ObjectFit::Contain);

        // Compute object-position
        let x_offset = object_position.horizontal.resolve(
            CSSPixelLength::new(container_size.width - paint_size.width) / self.scale as f32,
        ) * self.scale as f32;
        let y_offset = object_position.vertical.resolve(
            CSSPixelLength::new(container_size.height - paint_size.height) / self.scale as f32,
        ) * self.scale as f32;
        let x = x + x_offset.px() as f64;
        let y = y + y_offset.px() as f64;

        let x_scale = paint_size.width as f64 / object_size.width as f64;
        let y_scale = paint_size.height as f64 / object_size.height as f64;

        let transform = self
            .transform
            .pre_scale_non_uniform(x_scale, y_scale)
            .then_translate(Vec2 { x, y });

        anyrender_svg::render_svg_tree(scene, svg, transform);
    }

    fn draw_image(&self, scene: &mut (impl PaintScene + BlurLayer)) {
        // A playing video draws its latest frame; before the first one, and
        // when nothing plays, the element's own image (a video's poster).
        let frame = self.element.media.as_ref().and_then(|media| {
            media.0.set_display_size(
                self.frame.content_box.width().round() as u32,
                self.frame.content_box.height().round() as u32,
            );
            media.0.frame()
        });
        if let Some(image) = frame.as_ref().or(self.element.raster_image_data()) {
            let width = self.frame.content_box.width() as u32;
            let height = self.frame.content_box.height() as u32;
            let x = self.frame.content_box.origin().x;
            let y = self.frame.content_box.origin().y;

            let object_fit = self.style.clone_object_fit();
            let object_position = self.style.clone_object_position();
            let image_rendering = self.style.clone_image_rendering();
            let quality = to_image_quality(image_rendering);

            // Apply object-fit algorithm
            let container_size = taffy::Size {
                width: width as f32,
                height: height as f32,
            };
            let object_size = taffy::Size {
                width: image.width as f32,
                height: image.height as f32,
            };
            let paint_size = compute_object_fit(container_size, Some(object_size), object_fit);

            // Compute object-position
            let x_offset = object_position.horizontal.resolve(
                CSSPixelLength::new(container_size.width - paint_size.width) / self.scale as f32,
            ) * self.scale as f32;
            let y_offset = object_position.vertical.resolve(
                CSSPixelLength::new(container_size.height - paint_size.height) / self.scale as f32,
            ) * self.scale as f32;
            let x = x + x_offset.px() as f64;
            let y = y + y_offset.px() as f64;

            let x_scale = paint_size.width as f64 / object_size.width as f64;
            let y_scale = paint_size.height as f64 / object_size.height as f64;
            let transform = self
                .transform
                .pre_translate(Vec2 { x, y })
                .pre_scale_non_uniform(x_scale, y_scale);

            scene.draw_image(to_peniko_image(image, quality).as_ref(), transform);
        }
    }

    /// A video's built-in controls: a play button over it while it is not
    /// playing, and with `controls` a bar along the bottom -- play/pause, the
    /// progress through it, mute. Laid out by `MediaControlsLayout`, which the
    /// click handling shares.
    fn draw_media_controls(&self, scene: &mut (impl PaintScene + BlurLayer)) {
        use kurbo::{Circle, Shape};
        use zipp_dom::traits_src::media::{ControlRect, MediaControlsLayout};

        if self.element.name.local != local_name!("video") {
            return;
        }
        let has_controls = self.element.attr(local_name!("controls")).is_some();
        let status = self.element.media.as_ref().map(|media| media.0.status());
        if !has_controls && status.is_some() {
            // A video without controls that is already under way (a muted
            // autoplaying background, say) is drawn bare.
            return;
        }
        let status = status.unwrap_or_default();

        let scale = self.scale;
        let origin = self.frame.border_box.origin();
        let width = self.frame.border_box.width() / scale;
        let height = self.frame.border_box.height() / scale;
        if width < 8.0 || height < 8.0 {
            return;
        }
        let to_device = |r: ControlRect| {
            Rect::new(
                origin.x + r.x as f64 * scale,
                origin.y + r.y as f64 * scale,
                origin.x + (r.x + r.width) as f64 * scale,
                origin.y + (r.y + r.height) as f64 * scale,
            )
        };
        let shade = peniko::Color::from_rgba8(0, 0, 0, 140);
        let white = peniko::Color::from_rgba8(255, 255, 255, 235);
        let faint = peniko::Color::from_rgba8(255, 255, 255, 90);
        // A play triangle centred in `r`, `size` device pixels tall.
        let triangle = |center: Point, size: f64| {
            let mut path = BezPath::new();
            path.move_to((center.x - size * 0.35, center.y - size * 0.5));
            path.line_to((center.x + size * 0.55, center.y));
            path.line_to((center.x - size * 0.35, center.y + size * 0.5));
            path.close_path();
            path
        };

        let playing = !status.paused && !status.ended;
        if !playing {
            let center = Point::new(origin.x + width * scale / 2.0, origin.y + height * scale / 2.0);
            let radius = (width.min(height) / 6.0).clamp(14.0, 36.0) * scale;
            scene.fill(Fill::NonZero, self.transform, shade, None, &Circle::new(center, radius));
            let glyph = triangle(Point::new(center.x + radius * 0.08, center.y), radius * 0.9);
            scene.fill(Fill::NonZero, self.transform, white, None, &glyph);
        }
        if !has_controls {
            return;
        }

        let layout = MediaControlsLayout::new(width as f32, height as f32);
        scene.fill(Fill::NonZero, self.transform, shade, None, &to_device(layout.bar));

        // Play/pause.
        let play = to_device(layout.play);
        let glyph_size = play.height() * 0.45;
        if playing {
            let bar_w = glyph_size * 0.3;
            for dx in [-glyph_size * 0.3, glyph_size * 0.3] {
                let cx = play.center().x + dx;
                let r = Rect::new(cx - bar_w / 2.0, play.center().y - glyph_size / 2.0, cx + bar_w / 2.0, play.center().y + glyph_size / 2.0);
                scene.fill(Fill::NonZero, self.transform, white, None, &r);
            }
        } else {
            scene.fill(Fill::NonZero, self.transform, white, None, &triangle(play.center(), glyph_size));
        }

        // Progress.
        let track = to_device(layout.track);
        if track.width() > 0.0 {
            scene.fill(Fill::NonZero, self.transform, faint, None, &track.to_rounded_rect(track.height() / 2.0));
            if let Some(duration) = status.duration.filter(|d| d.is_finite() && *d > 0.0) {
                let fraction = (status.position / duration).clamp(0.0, 1.0);
                let done = Rect::new(track.x0, track.y0, track.x0 + track.width() * fraction, track.y1);
                scene.fill(Fill::NonZero, self.transform, white, None, &done.to_rounded_rect(track.height() / 2.0));
                let knob = Circle::new(Point::new(done.x1, track.center().y), track.height() * 1.5);
                scene.fill(Fill::NonZero, self.transform, white, None, &knob);
            }
        }

        // Mute: a speaker, struck through when muted.
        let mute = to_device(layout.mute);
        let c = mute.center();
        let s = mute.height() * 0.22;
        let mut speaker = BezPath::new();
        speaker.move_to((c.x - s * 1.3, c.y - s * 0.5));
        speaker.line_to((c.x - s * 0.5, c.y - s * 0.5));
        speaker.line_to((c.x + s * 0.4, c.y - s * 1.2));
        speaker.line_to((c.x + s * 0.4, c.y + s * 1.2));
        speaker.line_to((c.x - s * 0.5, c.y + s * 0.5));
        speaker.line_to((c.x - s * 1.3, c.y + s * 0.5));
        speaker.close_path();
        scene.fill(Fill::NonZero, self.transform, white, None, &speaker);
        if status.muted {
            let mut cross = BezPath::new();
            cross.move_to((c.x - s * 1.5, c.y - s * 1.5));
            cross.line_to((c.x + s * 1.5, c.y + s * 1.5));
            scene.stroke(&Stroke::new(1.6 * scale), self.transform, white, None, &cross);
        } else {
            let wave = kurbo::Arc::new((c.x + s * 0.4, c.y), (s * 0.9, s * 0.9), -0.9, 1.8, 0.0);
            scene.stroke(&Stroke::new(1.4 * scale), self.transform, white, None, &wave.into_path(0.1));
        }
    }

    fn draw_canvas(&self, scene: &mut (impl PaintScene + BlurLayer)) {
        if let Some(custom_paint_source) = self.element.canvas_data() {
            let width = self.frame.content_box.width() as u32;
            let height = self.frame.content_box.height() as u32;
            let x = self.frame.content_box.origin().x;
            let y = self.frame.content_box.origin().y;

            let transform = self.transform.then_translate(Vec2 { x, y });

            scene.fill(
                Fill::NonZero,
                transform,
                // TODO: replace `Arc<dyn Any>` with `CustomPaint` in API?
                Paint::Custom(&CustomPaint {
                    source_id: custom_paint_source.custom_paint_source_id,
                    width,
                    height,
                    scale: self.scale,
                } as &(dyn Any + Send + Sync)),
                None,
                &Rect::from_origin_size((0.0, 0.0), (width as f64, height as f64)),
            );
        }
    }

    fn draw_sub_document(&self, scene: &mut (impl PaintScene + BlurLayer)) {
        if let Some(sub_doc) = self.element.sub_doc_data().map(|doc| doc.inner()) {
            let scale = self.scale;
            let width = self.frame.content_box.width() as u32;
            let height = self.frame.content_box.height() as u32;
            let initial_x = self.pos.x + self.frame.content_box.origin().x / scale;
            let initial_y = self.pos.y + self.frame.content_box.origin().y / scale;

            let painter =
                DocumentPainter::new(&sub_doc, scale, width, height, initial_x, initial_y);
            painter.paint_scene(scene);
        }
    }

    fn stroke_devtools(&self, scene: &mut (impl PaintScene + BlurLayer)) {
        if self.devtools.show_layout {
            let shape = &self.frame.border_box;
            let stroke = Stroke::new(self.scale);

            let stroke_color = match self.node.style.display {
                taffy::Display::Block => Color::new([1.0, 0.0, 0.0, 1.0]),
                taffy::Display::Flex => Color::new([0.0, 1.0, 0.0, 1.0]),
                taffy::Display::Grid => Color::new([0.0, 0.0, 1.0, 1.0]),
                taffy::Display::None => Color::new([0.0, 0.0, 1.0, 1.0]),
            };

            scene.stroke(&stroke, self.transform, stroke_color, None, &shape);
        }
    }

    /// Draw all borders for a node
    fn draw_border(&self, scene: &mut (impl PaintScene + BlurLayer)) {
        let style = &*self.style;
        let border = style.get_border();
        let current_color = style.clone_color();

        // Fast path: uniform 4-sided dashed or dotted border — stroke the closed
        // rounded border centerline directly with a real dash pattern so rounded
        // corners curve smoothly without clip-mask corner wedges.
        if matches!(border.border_top_style, BorderStyle::Dashed | BorderStyle::Dotted)
            && border.border_top_style == border.border_right_style
            && border.border_top_style == border.border_bottom_style
            && border.border_top_style == border.border_left_style
        {
            let bw = &self.frame.border_width;
            let w = bw.y0;
            if w > 0.0
                && (bw.x1 - w).abs() < 1e-3
                && (bw.y1 - w).abs() < 1e-3
                && (bw.x0 - w).abs() < 1e-3
            {
                let c_top = border.border_top_color.resolve_to_absolute(&current_color).as_srgb_color();
                let c_right = border.border_right_color.resolve_to_absolute(&current_color).as_srgb_color();
                let c_bottom = border.border_bottom_color.resolve_to_absolute(&current_color).as_srgb_color();
                let c_left = border.border_left_color.resolve_to_absolute(&current_color).as_srgb_color();
                if c_top.components[3] > 0.0 && c_top == c_right && c_top == c_bottom && c_top == c_left {
                    let centerline = self.frame.border_centerline_path();
                    let (dash, gap, cap) = match border.border_top_style {
                        BorderStyle::Dotted => (w, w * 1.5, kurbo::Cap::Round),
                        _ => (w * 3.0, w * 2.0, kurbo::Cap::Butt),
                    };
                    let mut stroke = Stroke::new(w).with_caps(cap);
                    stroke.dash_pattern = [dash, gap].as_slice().into();
                    scene.stroke(&stroke, self.transform, c_top, None, &centerline);
                    return;
                }
            }
        }

        let mut borders: [(Color, Option<BezPath>); 4] = [
            (Color::TRANSPARENT, None),
            (Color::TRANSPARENT, None),
            (Color::TRANSPARENT, None),
            (Color::TRANSPARENT, None),
        ];
        let mut count = 0;

        for &edge in &[Edge::Top, Edge::Right, Edge::Bottom, Edge::Left] {
            let color = match edge {
                Edge::Top => &border.border_top_color,
                Edge::Right => &border.border_right_color,
                Edge::Bottom => &border.border_bottom_color,
                Edge::Left => &border.border_left_color,
            }
            .resolve_to_absolute(&current_color)
            .as_srgb_color();

            let edge_style = match edge {
                Edge::Top => border.border_top_style,
                Edge::Right => border.border_right_style,
                Edge::Bottom => border.border_bottom_style,
                Edge::Left => border.border_left_style,
            };

            // A zero-width edge paints nothing. Tailwind's preflight gives every
            // element `border-width: 0` in an opaque colour, so without this
            // nearly every box built four empty edge paths.
            let edge_width = match edge {
                Edge::Top => self.frame.border_width.y0,
                Edge::Right => self.frame.border_width.x1,
                Edge::Bottom => self.frame.border_width.y1,
                Edge::Left => self.frame.border_width.x0,
            };

            if edge_width > 0.0 && color.components[3] > 0.0 {
                // `dashed` and `dotted` used to fall through to a solid fill.
                // The pattern is drawn clipped to the edge's own shape, so
                // mitred and rounded corners stay correct without the dash
                // geometry having to know about them.
                if matches!(edge_style, BorderStyle::Dashed | BorderStyle::Dotted) {
                    self.draw_dashed_edge(scene, edge, edge_style, color);
                    continue;
                }
                borders[count] = (color, Some(self.frame.border_edge_shape(edge)));
                count += 1;
            }
        }

        if count == 0 {
            return;
        }

        // Fast path: single border side, no need to sort or group
        if count == 1 {
            scene.fill(
                Fill::NonZero,
                self.transform,
                borders[0].0,
                None,
                borders[0].1.as_ref().unwrap(),
            );
            return;
        }

        // Group together identical colors by sorting.
        let active_slice = &mut borders[0..count];
        active_slice.sort_unstable_by(|a, b| {
            a.0.components
                .partial_cmp(&b.0.components)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut start_border_index = 0;
        while start_border_index < count {
            let color = borders[start_border_index].0;
            let mut next_border_index = start_border_index + 1;
            let has_multiple_edges =
                next_border_index < count && borders[next_border_index].0 == color;
            if has_multiple_edges {
                let mut border_path = borders[start_border_index].1.take().unwrap();
                while next_border_index < count && borders[next_border_index].0 == color {
                    border_path.extend(&borders[next_border_index].1.take().unwrap());
                    next_border_index += 1;
                }
                scene.fill(Fill::NonZero, self.transform, color, None, &border_path);
            } else {
                scene.fill(
                    Fill::NonZero,
                    self.transform,
                    color,
                    None,
                    borders[start_border_index].1.as_ref().unwrap(),
                );
            }
            start_border_index = next_border_index;
        }
    }

    fn draw_table_borders(&self, scene: &mut (impl PaintScene + BlurLayer)) {
        let SpecialElementData::TableRoot(table) = &self.element.special_data else {
            return;
        };
        // Borders are only handled at the table level when BorderCollapse::Collapse
        if table.border_collapse != BorderCollapse::Collapse {
            return;
        }

        let Some(grid_info) = &mut *table.computed_grid_info.borrow_mut() else {
            return;
        };
        let Some(border_style) = table.border_style.as_deref() else {
            return;
        };

        let outer_border_style = self.style.get_border();

        let cols = &grid_info.columns;
        let rows = &grid_info.rows;

        let inner_width =
            (cols.sizes.iter().sum::<f32>() + cols.gutters.iter().sum::<f32>()) as f64;
        let inner_height =
            (rows.sizes.iter().sum::<f32>() + rows.gutters.iter().sum::<f32>()) as f64;

        // TODO: support different colors for different borders
        let current_color = self.style.clone_color();
        let border_color = border_style
            .border_top_color
            .resolve_to_absolute(&current_color)
            .as_srgb_color();

        // No need to draw transparent borders (as they won't be visible anyway)
        if border_color == Color::TRANSPARENT {
            return;
        }

        let border_width = border_style.border_top_width.0.to_f64_px();

        // Draw horizontal inner borders
        let mut y = 0.0;
        for (&height, &gutter) in rows.sizes.iter().zip(rows.gutters.iter()) {
            let shape =
                Rect::new(0.0, y, inner_width, y + gutter as f64).scale_from_origin(self.scale);
            scene.fill(Fill::NonZero, self.transform, border_color, None, &shape);

            y += (height + gutter) as f64;
        }

        // Draw horizontal outer borders
        // Top border
        if outer_border_style.border_top_style != BorderStyle::Hidden {
            let shape =
                Rect::new(0.0, 0.0, inner_width, border_width).scale_from_origin(self.scale);
            scene.fill(Fill::NonZero, self.transform, border_color, None, &shape);
        }
        // Bottom border
        if outer_border_style.border_bottom_style != BorderStyle::Hidden {
            let shape = Rect::new(0.0, inner_height, inner_width, inner_height + border_width)
                .scale_from_origin(self.scale);
            scene.fill(Fill::NonZero, self.transform, border_color, None, &shape);
        }

        // Draw vertical inner borders
        let mut x = 0.0;
        for (&width, &gutter) in cols.sizes.iter().zip(cols.gutters.iter()) {
            let shape =
                Rect::new(x, 0.0, x + gutter as f64, inner_height).scale_from_origin(self.scale);
            scene.fill(Fill::NonZero, self.transform, border_color, None, &shape);

            x += (width + gutter) as f64;
        }

        // Draw vertical outer borders
        // Left border
        if outer_border_style.border_left_style != BorderStyle::Hidden {
            let shape =
                Rect::new(0.0, 0.0, border_width, inner_height).scale_from_origin(self.scale);
            scene.fill(Fill::NonZero, self.transform, border_color, None, &shape);
        }
        // Right border
        if outer_border_style.border_right_style != BorderStyle::Hidden {
            let shape = Rect::new(inner_width, 0.0, inner_width + border_width, inner_height)
                .scale_from_origin(self.scale);
            scene.fill(Fill::NonZero, self.transform, border_color, None, &shape);
        }
    }

    /// Paint one `dashed` or `dotted` edge.
    ///
    /// The dashes are plain rectangles laid along the edge and clipped to the
    /// edge's own shape, which is what keeps mitred joins and rounded corners
    /// right — the pattern itself stays a simple 1-D walk.
    ///
    /// Lengths follow what browsers actually draw rather than a specific spec
    /// requirement (CSS leaves the pattern up to the UA): dashes three times
    /// the border width separated by gaps of the same width, dots a single
    /// width square. The period is then stretched slightly so a whole number of
    /// them spans the edge, which is why a dashed box has symmetric corners
    /// instead of a truncated dash on one side.
    fn draw_dashed_edge(
        &self,
        scene: &mut (impl PaintScene + BlurLayer),
        edge: Edge,
        style: BorderStyle,
        color: Color,
    ) {
        let bb = self.frame.border_box;
        let bw = &self.frame.border_width;

        // Thickness of this edge, and the span the dashes run along.
        let (thickness, span): (f64, f64) = match edge {
            Edge::Top => (bw.y0, bb.width()),
            Edge::Bottom => (bw.y1, bb.width()),
            Edge::Left => (bw.x0, bb.height()),
            Edge::Right => (bw.x1, bb.height()),
        };
        if thickness <= 0.0 || span <= 0.0 {
            return;
        }

        let (dash, gap) = match style {
            BorderStyle::Dotted => (thickness, thickness),
            _ => (thickness * 3.0, thickness),
        };
        let period = dash + gap;
        // Fit a whole number of periods so both ends land on a dash.
        let count = ((span + gap) / period).round().max(1.0);
        let period = (span + gap) / count;
        let dash = (period - gap).max(thickness * 0.5);

        // How far each dash reaches INTO the box. A rounded corner curves away
        // from the straight edge, so a rect only `thickness` deep leaves the
        // arcs bare and a rounded dashed box comes out with four blank corners.
        // Over-reaching costs nothing because the edge shape clips it back --
        // and that shape is mitred, so a dash can never bleed into the
        // neighbouring edge. Half the shorter side always covers any legal
        // corner radius.
        let depth = thickness.max(bb.width().min(bb.height()) * 0.5);

        let mut pattern = BezPath::new();
        let mut offset = 0.0;
        while offset < span - 0.01 {
            let end = (offset + dash).min(span);
            let r = match edge {
                Edge::Top => Rect::new(bb.x0 + offset, bb.y0, bb.x0 + end, bb.y0 + depth),
                Edge::Bottom => Rect::new(bb.x0 + offset, bb.y1 - depth, bb.x0 + end, bb.y1),
                Edge::Left => Rect::new(bb.x0, bb.y0 + offset, bb.x0 + depth, bb.y0 + end),
                Edge::Right => Rect::new(bb.x1 - depth, bb.y0 + offset, bb.x1, bb.y0 + end),
            };
            pattern.extend(kurbo::Shape::path_elements(&r, 0.1));
            offset += period;
        }

        let mut clip = self.frame.border_edge_shape(edge);
        clip.close_path();
        scene.push_layer(peniko::Mix::Normal, 1.0, self.transform, &clip);
        scene.fill(Fill::NonZero, self.transform, color, None, &pattern);
        scene.pop_layer();
    }

    /// ✅ dotted - Defines a dotted border
    /// ✅ dashed - Defines a dashed border
    /// ✅ solid - Defines a solid border
    /// ❌ double - Defines a double border
    /// ❌ groove - Defines a 3D grooved border. The effect depends on the border-color value
    /// ❌ ridge - Defines a 3D ridged border. The effect depends on the border-color value
    /// ❌ inset - Defines a 3D inset border. The effect depends on the border-color value
    /// ❌ outset - Defines a 3D outset border. The effect depends on the border-color value
    /// ✅ none - Defines no border
    /// ✅ hidden - Defines a hidden border
    fn draw_outline(&self, scene: &mut (impl PaintScene + BlurLayer)) {
        let outline = self.style.get_outline();

        let current_color = self.style.clone_color();
        let color = outline
            .outline_color
            .resolve_to_absolute(&current_color)
            .as_srgb_color();

        let style = match outline.outline_style {
            OutlineStyle::Auto => return,
            OutlineStyle::BorderStyle(style) => style,
        };

        let path = match style {
            BorderStyle::None | BorderStyle::Hidden => return,
            BorderStyle::Solid => self.frame.outline(),

            // TODO: Implement other border styles
            BorderStyle::Inset
            | BorderStyle::Groove
            | BorderStyle::Outset
            | BorderStyle::Ridge
            | BorderStyle::Dotted
            | BorderStyle::Dashed
            | BorderStyle::Double => self.frame.outline(),
        };

        scene.fill(Fill::NonZero, self.transform, color, None, &path);
    }
}
impl<'a> std::ops::Deref for ElementCx<'a> {
    type Target = DocumentPainter<'a>;
    fn deref(&self) -> &Self::Target {
        self.context
    }
}

fn insets_from_taffy_rect(input: taffy::Rect<f64>) -> Insets {
    Insets {
        x0: input.left,
        y0: input.top,
        x1: input.right,
        y1: input.bottom,
    }
}

/// Convert Stylo and Taffy types into Kurbo types
fn create_css_rect(style: &ComputedValues, layout: &Layout, scale: f64) -> CssBox {
    // Resolve and rescale
    // We have to scale since document pixels are not same same as rendered pixels
    let width: f64 = layout.size.width as f64;
    let height: f64 = layout.size.height as f64;
    let border_box = Rect::new(0.0, 0.0, width * scale, height * scale);
    let border = insets_from_taffy_rect(layout.border.map(|p| p as f64 * scale));
    let padding = insets_from_taffy_rect(layout.padding.map(|p| p as f64 * scale));
    let outline_width = style.get_outline().outline_width.0.to_f64_px() * scale;

    // Resolve the radii to a length. need to downscale since the radii are in document pixels
    let resolve_w = CSSPixelLength::new(width as _);
    let resolve_h = CSSPixelLength::new(height as _);
    let resolve_radii = |radius: &BorderCornerRadius| -> Vec2 {
        Vec2 {
            x: scale * radius.0.width.0.resolve(resolve_w).px() as f64,
            y: scale * radius.0.height.0.resolve(resolve_h).px() as f64,
        }
    };
    let s_border = style.get_border();
    let border_radii = NonUniformRoundedRectRadii {
        top_left: resolve_radii(&s_border.border_top_left_radius),
        top_right: resolve_radii(&s_border.border_top_right_radius),
        bottom_right: resolve_radii(&s_border.border_bottom_right_radius),
        bottom_left: resolve_radii(&s_border.border_bottom_left_radius),
    };

    CssBox::new(border_box, border, padding, outline_width, border_radii)
}
