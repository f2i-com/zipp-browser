//! Style resolution and layout computation.
//!
//! This module implements the top-level resolve pipeline that takes a styled DOM tree
//! and produces a fully laid-out tree. The main entry point is [`BaseDocument::resolve`],
//! which orchestrates restyling, layout-child collection, deferred task processing,
//! style-to-layout flushing, and final layout computation.

use std::{
    cell::RefCell,
    time::{SystemTime, UNIX_EPOCH},
};

use parley::LayoutContext;
use selectors::Element as _;
use style::dom::TDocument;

#[cfg(feature = "parallel-construct")]
use rayon::prelude::*;

// FIXME: static thread_local FontCtx isn't necessarily correct in multi-document context.
// Should use thread_local crate with ThreadLocal value store in the Document.
thread_local! {
    pub(crate) static LAYOUT_CTX: RefCell<Option<Box<LayoutContext<TextBrush>>>> = const { RefCell::new(None) };
}

#[cfg(feature = "incremental")]
use style::selector_parser::RestyleDamage;
use style::properties::generated::longhands::position::computed_value::T as Position;
use taffy::AvailableSpace;

use crate::{
    BaseDocument, NON_INCREMENTAL, NodeData,
    events::ScrollAnimationState,
    layout::{
        construct::{
            ConstructionTask, ConstructionTaskData, ConstructionTaskResult,
            ConstructionTaskResultData, build_inline_layout_into, collect_layout_children,
        },
        damage::{ALL_DAMAGE, CONSTRUCT_BOX, CONSTRUCT_DESCENDENT, CONSTRUCT_FC},
    },
    node::TextBrush,
};

impl BaseDocument {
    /// Restyle the tree and then relayout it.
    ///
    /// This is the main "resolve" entry point called once per frame. It:
    /// 1. Processes pending resource-load messages.
    /// 2. Advances scroll animations.
    /// 3. Resolves the Stylo stylist (CSS cascade).
    /// 4. Propagates damage flags (incremental mode).
    /// 5. Collects layout children (inserting anonymous blocks as needed).
    /// 6. Processes deferred inline-layout tasks.
    /// 7. Flushes computed styles into taffy layout styles.
    /// 8. Runs taffy layout.
    /// 9. Recursively resolves sub-documents (iframes).
    pub fn resolve(&mut self, current_time_for_animations: f64) {
        self.animation_time = current_time_for_animations;
        if TDocument::as_node(&&self.nodes[0])
            .first_element_child()
            .is_none()
        {
            #[cfg(feature = "tracing")]
            tracing::warn!("No DOM - not resolving");
            return;
        }

        // Process messages that have been sent to our message channel (e.g. loaded resource)
        self.handle_messages();

        self.resolve_scroll_animation();

        let root_node_id = self.root_element().id;

        // Resolve stylist first since it drives layout
        self.resolve_stylist(current_time_for_animations);

        // Propagate damage flags (from mutation and restyles) up and down the tree
        #[cfg(feature = "incremental")]
        let accumulated_damage = self.propagate_damage_flags(root_node_id, RestyleDamage::empty());

        // In incremental mode, skip the full layout pass when only paint damage exists
        // (e.g. color or background-color changes that don't affect geometry).
        #[cfg(feature = "incremental")]
        let needs_layout = accumulated_damage.intersects(LAYOUT_AFFECTING_DAMAGE);
        #[cfg(not(feature = "incremental"))]
        let needs_layout = true;

        if needs_layout {
            // Fix up tree for layout (insert anonymous blocks as necessary, etc)
            self.resolve_layout_children();
            self.resolve_deferred_tasks();
        }

        // Merge stylo into taffy (always needed to sync paint properties like background-image)
        self.flush_styles_to_layout(root_node_id);

        if needs_layout {
            // Resolve layout
            self.resolve_layout();
        }

        // Clear all damage and dirty flags
        #[cfg(feature = "incremental")]
        {
            for (_, node) in self.nodes.iter_mut() {
                node.clear_damage_mut();
                node.unset_dirty_descendants();
            }
        }

        // Resolve sub-documents (e.g. iframes)
        let mut subdoc_is_animating = false;
        for &node_id in &self.sub_document_nodes {
            let node = &mut self.nodes[node_id];
            let size = node.final_layout.size;
            if let Some(mut sub_doc) = node.subdoc_mut().map(|doc| doc.inner_mut()) {
                // Set viewport
                // viewport_mut handles change detection. So we just unconditionally set the values;
                let mut sub_viewport = sub_doc.viewport_mut();
                sub_viewport.hidpi_scale = self.viewport.hidpi_scale;
                sub_viewport.zoom = self.viewport.zoom;
                sub_viewport.color_scheme = self.viewport.color_scheme;

                let viewport_scale = self.viewport.scale();
                sub_viewport.window_size = (
                    (size.width * viewport_scale) as u32,
                    (size.height * viewport_scale) as u32,
                );
                drop(sub_viewport);

                sub_doc.resolve(current_time_for_animations);

                subdoc_is_animating |= sub_doc.is_animating();
            }
        }
        self.subdoc_is_animating = subdoc_is_animating;
    }

    /// Advance the scroll fling animation (if any) by one frame.
    pub fn resolve_scroll_animation(&mut self) {
        match &mut self.scroll_animation {
            ScrollAnimationState::Fling(fling_state) => {
                let time_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64 as f64;

                let time_diff_ms = time_ms - fling_state.last_seen_time;

                // 0.95 @ 60fps normalized to actual frame times
                let deceleration = 1.0 - ((0.05 / 16.66666) * time_diff_ms);

                fling_state.x_velocity *= deceleration;
                fling_state.y_velocity *= deceleration;
                fling_state.last_seen_time = time_ms;

                // Extract only the fields we need to avoid cloning the entire state
                let target = fling_state.target;
                let x_velocity = fling_state.x_velocity;
                let y_velocity = fling_state.y_velocity;

                let dx = x_velocity * time_diff_ms;
                let dy = y_velocity * time_diff_ms;

                self.scroll_by(Some(target), dx, dy, &mut |_| {});
                if x_velocity.abs() < 0.1 && y_velocity.abs() < 0.1 {
                    self.scroll_animation = ScrollAnimationState::None;
                }
            }
            ScrollAnimationState::None => {
                // Do nothing
            }
        }
    }

    /// Ensure that the `layout_children` field is populated for all nodes.
    ///
    /// Walks the tree top-down, collecting layout children for each node. In incremental
    /// mode, only nodes whose damage flags indicate they need reconstruction are visited.
    pub fn resolve_layout_children(&mut self) {
        resolve_layout_children_recursive(self, self.root_node().id);
        reparent_out_of_flow_children(self, self.root_node().id);

        // Hit-testing can land on an anonymous wrapper, so these ids may name
        // one the rebuild above just freed. Indexing a freed slot panics
        // ("invalid key") on the next mouse move; forget the id instead.
        for id in [
            &mut self.hover_node_id,
            &mut self.focus_node_id,
            &mut self.active_node_id,
            &mut self.mousedown_node_id,
        ] {
            if id.is_some_and(|node_id| !self.nodes.contains(node_id)) {
                *id = None;
            }
        }

        fn resolve_layout_children_recursive(doc: &mut BaseDocument, node_id: usize) {
            let mut damage = doc.nodes[node_id].damage().unwrap_or(ALL_DAMAGE);
            let _flags = doc.nodes[node_id].flags;

            if NON_INCREMENTAL || damage.intersects(CONSTRUCT_FC | CONSTRUCT_BOX) {
                // The last collection's anonymous wrappers are replaced by the
                // ones this one creates, so free them first: they live only in
                // `layout_children`, and dropping that list left every one in
                // the slab. Without the `incremental` feature this runs on every
                // resolve -- every animation frame -- so an animated page
                // leaked a few MB a second (7GB after a few minutes on
                // lance.name). Freeing first also lets the new wrappers reuse
                // the same slots.
                let old_children = doc.nodes[node_id].layout_children.borrow_mut().take();
                if let Some(old_children) = old_children {
                    free_anonymous_wrappers(doc, old_children);
                }

                let mut layout_children = Vec::new();
                let mut anonymous_block: Option<usize> = None;
                collect_layout_children(doc, node_id, &mut layout_children, &mut anonymous_block);

                // Recurse into newly collected layout children
                for child_id in layout_children.iter().copied() {
                    resolve_layout_children_recursive(doc, child_id);
                    doc.nodes[child_id].layout_parent.set(Some(node_id));
                    if let Some(mut data) = doc.nodes[child_id].stylo_element_data.get_mut() {
                        data.damage
                            .remove(CONSTRUCT_DESCENDENT | CONSTRUCT_FC | CONSTRUCT_BOX);
                    }
                }

                *doc.nodes[node_id].layout_children.borrow_mut() = Some(layout_children);

                damage.remove(CONSTRUCT_DESCENDENT | CONSTRUCT_FC | CONSTRUCT_BOX);
            } else {
                let layout_children = doc.nodes[node_id].layout_children.borrow_mut().take();
                if let Some(layout_children) = layout_children {
                    // Recurse into previously computed layout children
                    for child_id in layout_children.iter().copied() {
                        resolve_layout_children_recursive(doc, child_id);
                        doc.nodes[child_id].layout_parent.set(Some(node_id));
                    }

                    *doc.nodes[node_id].layout_children.borrow_mut() = Some(layout_children);
                }
            }

            doc.nodes[node_id].set_damage(damage);
        }

        /// Remove the anonymous block wrappers among `children` (and any nested
        /// in them) from the slab. Pseudo-elements are anonymous blocks too, but
        /// their owner keeps them across collections, so they stay.
        fn free_anonymous_wrappers(doc: &mut BaseDocument, children: Vec<usize>) {
            for child_id in children {
                let Some(child) = doc.nodes.get(child_id) else {
                    continue;
                };
                if !matches!(child.data, NodeData::AnonymousBlock(_)) {
                    continue;
                }
                let is_pseudo = child.parent.and_then(|p| doc.nodes.get(p)).is_some_and(|owner| {
                    owner.before == Some(child_id) || owner.after == Some(child_id)
                });
                if is_pseudo {
                    continue;
                }
                let nested = doc.nodes[child_id].layout_children.borrow_mut().take();
                if let Some(nested) = nested {
                    free_anonymous_wrappers(doc, nested);
                }
                doc.nodes.remove(child_id);
                doc.changed_nodes.remove(&child_id);
            }
        }
    }

    /// Process deferred inline-layout construction tasks.
    ///
    /// Inline layout building is deferred so that it can be parallelized (when the
    /// `parallel-construct` feature is enabled). This method drains the queue,
    /// deduplicates by node ID, builds layouts, and writes results back into the tree.
    pub fn resolve_deferred_tasks(&mut self) {
        let mut deferred_construction_nodes = std::mem::take(&mut self.deferred_construction_nodes);

        // Deduplicate deferred tasks by node_id to avoid redundant work
        deferred_construction_nodes.sort_unstable_by_key(|task| task.node_id);
        deferred_construction_nodes.dedup_by_key(|task| task.node_id);

        #[cfg(feature = "parallel-construct")]
        let iter = deferred_construction_nodes.into_par_iter();
        #[cfg(not(feature = "parallel-construct"))]
        let iter = deferred_construction_nodes.into_iter();

        let results: Vec<ConstructionTaskResult> = iter
            .map(|task: ConstructionTask| match task.data {
                ConstructionTaskData::InlineLayout(mut layout) => {
                    #[cfg(feature = "parallel-construct")]
                    let mut layout_ctx = LAYOUT_CTX
                        .take()
                        .unwrap_or_else(|| Box::new(LayoutContext::new()));
                    #[cfg(feature = "parallel-construct")]
                    let layout_ctx_mut = &mut layout_ctx;

                    #[cfg(feature = "parallel-construct")]
                    let mut font_ctx = self
                        .thread_font_contexts
                        .get_or(|| RefCell::new(Box::new(self.font_ctx.lock().unwrap().clone())))
                        .borrow_mut();
                    #[cfg(feature = "parallel-construct")]
                    let font_ctx_mut = &mut *font_ctx;

                    #[cfg(not(feature = "parallel-construct"))]
                    let layout_ctx_mut = &mut self.layout_ctx;
                    #[cfg(not(feature = "parallel-construct"))]
                    let font_ctx_mut = &mut *self.font_ctx.lock().unwrap();

                    layout.content_widths = None;
                    build_inline_layout_into(
                        &self.nodes,
                        layout_ctx_mut,
                        font_ctx_mut,
                        &mut layout,
                        self.viewport.scale(),
                        task.node_id,
                    );

                    #[cfg(feature = "parallel-construct")]
                    {
                        LAYOUT_CTX.set(Some(layout_ctx));
                    }

                    ConstructionTaskResult {
                        node_id: task.node_id,
                        data: ConstructionTaskResultData::InlineLayout(layout),
                    }
                }
            })
            .collect();

        for result in results {
            match result.data {
                ConstructionTaskResultData::InlineLayout(layout) => {
                    self.nodes[result.node_id].cache.clear();
                    self.nodes[result.node_id]
                        .element_data_mut()
                        .unwrap()
                        .inline_layout_data = Some(layout);
                }
            }
        }

        self.deferred_construction_nodes.clear();
    }

    /// Compute the final layout of the document tree using taffy.
    ///
    /// Converts the viewport size into taffy `AvailableSpace` and runs
    /// `compute_root_layout` followed by `round_layout` on the root element.
    pub fn resolve_layout(&mut self) {
        let size = self.stylist.device().au_viewport_size();

        let available_space = taffy::Size {
            width: AvailableSpace::Definite(size.width.to_f32_px()),
            height: AvailableSpace::Definite(size.height.to_f32_px()),
        };

        let root_element_id = taffy::NodeId::from(self.root_element().id);

        taffy::compute_root_layout(self, root_element_id, available_space);
        taffy::round_layout(self, root_element_id);

        // Content sizes are only correct now, so this is the one place a scroll
        // offset can be clamped against the truth. Anything that shortens the
        // page — a window resize, a zoom change, a JS re-render — would
        // otherwise leave the content parked past its end with nothing to pull
        // it back; the pre-existing clamps all run BEFORE the relayout their own
        // change triggers, so they measure the previous frame.
        self.clamp_scroll_offsets();
        self.place_hoisted_boxes();
    }

    /// Place each box painted from its stacking context (one with a z-index)
    /// by this layout. Its offset from the stacking context is the sum of its
    /// ancestors' positions between them, gathered when the stacking contexts
    /// are built -- before layout, from the last one. A box that had only just
    /// appeared (a menu opening) was painted for a frame at its stacking
    /// context's corner, and jumped into place on the next.
    fn place_hoisted_boxes(&mut self) {
        let roots: Vec<usize> = self
            .nodes
            .iter()
            .filter(|(_, node)| node.stacking_context.is_some())
            .map(|(id, _)| id)
            .collect();
        for root in roots {
            let Some(mut context) = self.nodes[root].stacking_context.take() else {
                continue;
            };
            for hoisted in context.children.iter_mut() {
                let mut position = taffy::Point::ZERO;
                let mut ancestor = self.nodes.get(hoisted.node_id).and_then(|n| n.layout_parent.get());
                while let Some(id) = ancestor.filter(|&id| id != root) {
                    let node = &self.nodes[id];
                    position.x += node.final_layout.location.x - node.scroll_offset.x as f32;
                    position.y += node.final_layout.location.y - node.scroll_offset.y as f32;
                    ancestor = node.layout_parent.get();
                }
                hoisted.position = position;
            }
            context.compute_content_size(self);
            self.nodes[root].stacking_context = Some(context);
        }
    }

    /// Clamp every scroll offset to its element's current scrollable range.
    ///
    /// Cheap in practice: almost every node sits at offset zero and is skipped
    /// without touching its layout.
    fn clamp_scroll_offsets(&mut self) {
        for (_, node) in self.nodes.iter_mut() {
            if node.scroll_offset.x == 0.0 && node.scroll_offset.y == 0.0 {
                continue;
            }
            let (max_x, max_y) = node.text_input_scroll_range().unwrap_or((
                node.final_layout.scroll_width() as f64,
                node.final_layout.scroll_height() as f64,
            ));
            node.scroll_offset.x = node.scroll_offset.x.clamp(0.0, max_x.max(0.0));
            node.scroll_offset.y = node.scroll_offset.y.clamp(0.0, max_y.max(0.0));
        }
        // And the document scroller, now that the root's content size is fresh.
        self.scroll_viewport_by(0.0, 0.0);
    }
}

/// Move absolutely positioned boxes onto their real containing block.
///
/// Taffy lays every `Position::Absolute` child out against its IMMEDIATE
/// parent's box, unconditionally — see `perform_absolute_layout_on_absolute_children`.
/// CSS says an absolutely positioned box is instead laid out against the padding
/// box of its nearest *positioned* ancestor. Nothing re-parented them, so
/// `top`/`left`/`right`/`bottom`, percentage sizes and `auto` sizing were all
/// resolved against whatever `<div>` happened to wrap the element. That is the
/// single most common positioning idiom on the web — a `position:relative` card
/// with an absolutely placed badge or overlay two or three levels down — and it
/// landed in the wrong place every time.
///
/// The layout tree is a separate structure from the DOM (`layout_children`
/// already differs from `children`: it carries anonymous blocks and pseudo
/// elements), so the fix is simply to build it with the out-of-flow boxes
/// attached where CSS says they belong. Taffy then resolves everything against
/// the right box with no further special-casing, and paint and hit-testing
/// follow because both walk the layout tree.
///
/// Returns the boxes that could not be placed at `node_id` and must be adopted
/// by an ancestor.
fn reparent_out_of_flow_children(doc: &mut BaseDocument, node_id: usize) -> Vec<usize> {
    let is_root = doc.nodes[node_id].parent.is_none()
        || doc.nodes[node_id].id == doc.root_element().id;

    // A box establishes a containing block for absolute descendants when it is
    // itself positioned, or when it has a transform. The root always does.
    //
    // Inline roots are treated as containing blocks too, but for a different
    // reason: inline layout positions the out-of-flow boxes embedded in its own
    // line boxes (`layout/inline.rs`), so lifting them out from under it would
    // strand geometry that code still expects to own. That leaves abspos inside
    // a text-carrying block resolved against that block — the pre-existing
    // behaviour, not a regression.
    let is_containing_block = is_root
        || doc.nodes[node_id].flags.is_inline_root()
        || doc.nodes[node_id]
            .primary_styles()
            .map(|s| {
                !matches!(s.clone_position(), Position::Static)
                    || crate::stylo_to_kurbo::has_2d_transform(s.get_box())
            })
            .unwrap_or(false);

    let children = doc.nodes[node_id].layout_children.borrow_mut().take();
    let Some(children) = children else {
        return Vec::new();
    };

    let mut kept = Vec::with_capacity(children.len());
    let mut escaping = Vec::new();

    for child_id in children {
        // Depth first: anything the child could not contain surfaces here.
        let from_child = reparent_out_of_flow_children(doc, child_id);

        let child_is_abs = doc.nodes[child_id]
            .primary_styles()
            .map(|s| matches!(s.clone_position(), Position::Absolute | Position::Fixed))
            .unwrap_or(false);

        if child_is_abs {
            escaping.push(child_id);
        } else {
            kept.push(child_id);
        }
        escaping.extend(from_child);
    }

    if is_containing_block {
        for &adopted in escaping.iter() {
            doc.nodes[adopted].layout_parent.set(Some(node_id));
        }
        kept.append(&mut escaping);
    }

    *doc.nodes[node_id].layout_children.borrow_mut() = Some(kept);
    escaping
}
