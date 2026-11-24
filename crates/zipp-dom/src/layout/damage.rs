use std::ops::Range;

use crate::net::ResourceHandler;
use crate::node::NodeFlags;
use crate::{
    BaseDocument, net::ImageHandler, node::BackgroundImageData, node::Status, util::ImageType,
};
use crate::{NON_INCREMENTAL, Node};
use crate::traits_src::net::Request;
use style::properties::ComputedValues;
use style::properties::generated::longhands::position::computed_value::T as Position;
use style::selector_parser::RestyleDamage;
use style::servo::url::ComputedUrl;
use style::values::computed::Float;
use style::values::generics::image::Image as StyloImage;
use style::values::specified::align::AlignFlags;
use style::values::specified::box_::DisplayInside;
use style::values::specified::box_::DisplayOutside;
use taffy::Rect;

pub(crate) const CONSTRUCT_BOX: RestyleDamage =
    RestyleDamage::from_bits_retain(0b_0000_0000_0001_0000);
pub(crate) const CONSTRUCT_FC: RestyleDamage =
    RestyleDamage::from_bits_retain(0b_0000_0000_0010_0000);
pub(crate) const CONSTRUCT_DESCENDENT: RestyleDamage =
    RestyleDamage::from_bits_retain(0b_0000_0000_0100_0000);

pub(crate) const ONLY_RELAYOUT: RestyleDamage =
    RestyleDamage::from_bits_retain(0b_0000_0000_0000_1000);

/// Mask of all damage flags that require a layout recomputation.
/// Paint-only damage (REPAINT, REBUILD_STACKING_CONTEXT, RECALCULATE_OVERFLOW)
/// does not require running the Taffy layout pass.
#[allow(dead_code)]
pub(crate) const LAYOUT_AFFECTING_DAMAGE: RestyleDamage =
    RestyleDamage::from_bits_retain(0b_0000_0000_0111_1000);

pub(crate) const ALL_DAMAGE: RestyleDamage =
    RestyleDamage::from_bits_retain(0b_0000_0000_0111_1111);

impl BaseDocument {
    #[cfg(feature = "incremental")]
    pub(crate) fn propagate_damage_flags(
        &mut self,
        node_id: usize,
        damage_from_parent: RestyleDamage,
    ) -> RestyleDamage {
        let mut damage = if let Some(data) = self.nodes[node_id].stylo_element_data.get_mut() {
            data.damage
        } else {
            return RestyleDamage::empty();
        };
        damage |= damage_from_parent;

        let damage_for_children = RestyleDamage::empty();
        let children = std::mem::take(&mut self.nodes[node_id].children);
        let layout_children = std::mem::take(self.nodes[node_id].layout_children.get_mut());
        let use_layout_children = self.nodes[node_id].should_traverse_layout_children();
        if use_layout_children {
            let layout_children = layout_children.as_ref().unwrap();
            for child in layout_children.iter() {
                damage |= self.propagate_damage_flags(*child, damage_for_children);
            }
        } else {
            for child in children.iter() {
                damage |= self.propagate_damage_flags(*child, damage_for_children);
            }
            if let Some(before_id) = self.nodes[node_id].before {
                damage |= self.propagate_damage_flags(before_id, damage_for_children);
            }
            if let Some(after_id) = self.nodes[node_id].after {
                damage |= self.propagate_damage_flags(after_id, damage_for_children);
            }
        }

        let node = &mut self.nodes[node_id];

        // Put children back
        node.children = children;
        *node.layout_children.get_mut() = layout_children;

        if damage.contains(CONSTRUCT_BOX) {
            damage.insert(RestyleDamage::RELAYOUT);
        }

        // Compute damage to propagate to parent
        let damage_for_parent = damage; // & RestyleDamage::RELAYOUT;

        // If the node or any of it's children have been mutated or their layout styles
        // have changed, then we should clear it's layout cache.
        if damage.intersects(ONLY_RELAYOUT | CONSTRUCT_BOX) {
            node.cache.clear();
            if let Some(inline_layout) = node
                .data
                .downcast_element_mut()
                .and_then(|el| el.inline_layout_data.as_mut())
            {
                inline_layout.content_widths = None;
            }
            damage.remove(ONLY_RELAYOUT);
        }

        // Store damage for current node
        node.set_damage(damage);

        // Propagate damage to parent
        damage_for_parent
    }
}

pub(crate) fn compute_layout_damage(old: &ComputedValues, new: &ComputedValues) -> RestyleDamage {
    let box_tree_needs_rebuild = || {
        let old_box = old.get_box();
        let new_box = new.get_box();

        if old_box.display != new_box.display
            || old_box.float != new_box.float
            || old_box.position != new_box.position
            || old.clone_visibility() != new.clone_visibility()
        {
            return true;
        }

        if old.get_font() != new.get_font() {
            return true;
        }

        if new_box.display.outside() == DisplayOutside::Block
            && new_box.display.inside() == DisplayInside::Flow
        {
            let alignment_establishes_new_block_formatting_context = |style: &ComputedValues| {
                style.get_position().align_content.primary() != AlignFlags::NORMAL
            };

            let old_column = old.get_column();
            let new_column = new.get_column();
            if old_box.overflow_x.is_scrollable() != new_box.overflow_x.is_scrollable()
                || old_column.is_multicol() != new_column.is_multicol()
                || old_column.column_span != new_column.column_span
                || alignment_establishes_new_block_formatting_context(old)
                    != alignment_establishes_new_block_formatting_context(new)
            {
                return true;
            }
        }

        if old_box.display.is_list_item() {
            let old_list = old.get_list();
            let new_list = new.get_list();
            if old_list.list_style_position != new_list.list_style_position
                || old_list.list_style_image != new_list.list_style_image
                || (new_list.list_style_image == StyloImage::None
                    && old_list.list_style_type != new_list.list_style_type)
            {
                return true;
            }
        }

        if new.is_pseudo_style() && old.get_counters().content != new.get_counters().content {
            return true;
        }

        false
    };

    let text_shaping_needs_recollect = || {
        if old.clone_direction() != new.clone_direction()
            || old.clone_unicode_bidi() != new.clone_unicode_bidi()
        {
            return true;
        }

        let old_text = old.get_inherited_text();
        let new_text = new.get_inherited_text();
        if !std::ptr::eq(old_text, new_text)
            && (old_text.white_space_collapse != new_text.white_space_collapse
                || old_text.text_transform != new_text.text_transform
                || old_text.word_break != new_text.word_break
                || old_text.overflow_wrap != new_text.overflow_wrap
                || old_text.letter_spacing != new_text.letter_spacing
                || old_text.word_spacing != new_text.word_spacing
                || old_text.text_rendering != new_text.text_rendering)
        {
            return true;
        }

        false
    };

    #[allow(
        clippy::if_same_then_else,
        reason = "these branches will soon be different"
    )]
    if box_tree_needs_rebuild() {
        ALL_DAMAGE
    } else if text_shaping_needs_recollect() {
        ALL_DAMAGE
    } else {
        // This element needs to be laid out again, but does not have any damage to
        // its box. In the future, we will distinguish between types of damage to the
        // fragment as well.
        RestyleDamage::RELAYOUT
    }
}

/// A child with a z_index that is hoisted up to it's containing Stacking Context for paint purposes
#[derive(Debug, Clone)]
pub struct HoistedPaintChild {
    pub node_id: usize,
    pub z_index: i32,
    pub position: taffy::Point<f32>,
}

#[derive(Debug)]
pub struct HoistedPaintChildren {
    pub children: Vec<HoistedPaintChild>,
    /// The number of hoisted point children with negative z_index
    pub negative_z_count: u32,

    pub content_area: taffy::Rect<f32>,
}

impl HoistedPaintChildren {
    #[inline]
    fn new() -> Self {
        Self {
            children: Vec::new(),
            negative_z_count: 0,
            content_area: taffy::Rect::ZERO,
        }
    }

    #[inline]
    pub fn reset(&mut self) {
        self.children.clear();
        self.negative_z_count = 0;
    }

    pub fn compute_content_size(&mut self, doc: &BaseDocument) {
        fn child_pos(child: &HoistedPaintChild, doc: &BaseDocument) -> Option<Rect<f32>> {
            let node = doc.nodes.get(child.node_id)?;
            let left = child.position.x + node.final_layout.location.x;
            let top = child.position.y + node.final_layout.location.y;
            let right = left + node.final_layout.size.width;
            let bottom = top + node.final_layout.size.height;

            Some(taffy::Rect {
                top,
                left,
                bottom,
                right,
            })
        }

        // Prune any hoisted children whose nodes were removed from the slab so
        // stale stacking contexts can never panic on invalid keys.
        let prev_len = self.children.len();
        self.children.retain(|child| doc.nodes.contains(child.node_id));
        if self.children.len() != prev_len {
            self.negative_z_count =
                self.children.iter().take_while(|c| c.z_index < 0).count() as u32;
        }

        let mut iter = self.children.iter().filter_map(|c| child_pos(c, doc));
        let Some(mut area) = iter.next() else {
            self.content_area = taffy::Rect::ZERO;
            return;
        };
        for pos in iter {
            area.left = area.left.min(pos.left);
            area.top = area.top.min(pos.top);
            area.right = area.right.max(pos.right);
            area.bottom = area.bottom.max(pos.bottom);
        }
        self.content_area = area;
    }

    pub fn sort(&mut self) {
        self.children.sort_by_key(|c| c.z_index);
        self.negative_z_count = self.children.iter().take_while(|c| c.z_index < 0).count() as u32;
    }

    #[inline]
    pub fn neg_z_range(&self) -> Range<usize> {
        0..(self.negative_z_count as usize)
    }

    #[inline]
    pub fn pos_z_range(&self) -> Range<usize> {
        (self.negative_z_count as usize)..self.children.len()
    }

    #[inline]
    pub fn neg_z_hoisted_children(
        &self,
    ) -> impl ExactSizeIterator<Item = &HoistedPaintChild> + DoubleEndedIterator {
        self.children[self.neg_z_range()].iter()
    }

    #[inline]
    pub fn pos_z_hoisted_children(
        &self,
    ) -> impl ExactSizeIterator<Item = &HoistedPaintChild> + DoubleEndedIterator {
        self.children[self.pos_z_range()].iter()
    }
}

impl BaseDocument {
    pub(crate) fn invalidate_inline_contexts(&mut self) {
        let scale = self.viewport.scale();

        let font_ctx = &self.font_ctx;
        let layout_ctx = &mut self.layout_ctx;

        let mut anon_nodes = Vec::new();

        for (_, node) in self.nodes.iter_mut() {
            if !(node.flags.contains(NodeFlags::IS_IN_DOCUMENT)) {
                continue;
            }

            let Some(element) = node.data.downcast_element_mut() else {
                continue;
            };

            if element.inline_layout_data.is_some() {
                if node.is_anonymous() {
                    anon_nodes.push(node.id);
                } else {
                    node.insert_damage(ALL_DAMAGE);
                }
            } else if let Some(input) = element.text_input_data_mut() {
                input.editor.set_scale(scale);
                let mut font_ctx = font_ctx.lock().unwrap();
                input.editor.refresh_layout(&mut font_ctx, layout_ctx);
                if let Some(placeholder) = input.placeholder.as_mut() {
                    placeholder.set_scale(scale);
                    placeholder.refresh_layout(&mut font_ctx, layout_ctx);
                }
                node.insert_damage(ONLY_RELAYOUT);
            }
        }

        for node_id in anon_nodes {
            if let Some(parent_id) = *(self.nodes[node_id].layout_parent.get_mut())
                && let Some(parent) = self.nodes.get_mut(parent_id)
            {
                parent.insert_damage(ALL_DAMAGE);
            }
        }
    }

    pub fn flush_styles_to_layout(&mut self, node_id: usize) {
        // Clear all previous stacking contexts across the slab before rebuilding
        // them for the live layout tree. Nodes that became `display: none`,
        // stopped being stacking-context roots, or were detached from the
        // document would otherwise retain stale `HoistedPaintChildren` pointing
        // to freed slab keys (panicking in `place_hoisted_boxes` / `compute_content_size`)
        // or double-paint hoisted children.
        for (_, node) in self.nodes.iter_mut() {
            node.stacking_context = None;
        }
        self.flush_styles_to_layout_impl(node_id, None);
    }

    /// Walk the whole tree, converting styles to layout
    fn flush_styles_to_layout_impl(
        &mut self,
        node_id: usize,
        parent_stacking_context: Option<&mut HoistedPaintChildren>,
    ) {
        let doc_id = self.id();

        let mut new_stacking_context: HoistedPaintChildren = HoistedPaintChildren::new();
        let stacking_context = &mut new_stacking_context;

        let display = {
            let Some(node) = self.nodes.get_mut(node_id) else {
                return;
            };
            let _damage = node.damage().unwrap_or(ALL_DAMAGE);
            let stylo_element_data = node.stylo_element_data.get();
            let primary_styles = stylo_element_data
                .as_ref()
                .and_then(|data| data.styles.get_primary());

            let Some(style) = primary_styles else {
                return;
            };

            node.style = crate::stylo_bridge::to_taffy_style(style);
            node.display_constructed_as = style.clone_display();
            // Native `<select>` draws its dropdown chevron in the right 24px gutter
            // (`draw_select_arrow`). Reserve at least 24px of right padding when no
            // custom background-image replaces the native arrow, so narrow author
            // padding (e.g. Tailwind `px-2.5`) never clips option text under the chevron.
            if let Some(elem) = node.data.downcast_element() {
                if elem.name.local.as_ref() == "select"
                    && !style
                        .get_background()
                        .background_image
                        .0
                        .iter()
                        .any(|img| !matches!(img, StyloImage::None))
                {
                    let pr = node.style.padding.right.into_raw();
                    if (pr.tag() == taffy::CompactLength::LENGTH_TAG && pr.value() < 24.0)
                        || pr.is_zero()
                    {
                        node.style.padding.right = taffy::style_helpers::length(24.0);
                    }
                }
            }
            // A table construction built as a block (no rows or cells, see
            // `has_table_parts`) is laid out as one. It keeps `item_is_table`, so
            // a block parent still sizes it to fit rather than stretching it.
            if style.clone_display().inside() == style::values::specified::box_::DisplayInside::Table
                && !node.flags.is_table_root()
            {
                node.style.display = taffy::Display::Block;
            }

            // Flush background image from style to dedicated storage on the node
            // TODO: handle multiple background images
            if let Some(elem) = node.data.downcast_element_mut() {
                let style_bgs = &style.get_background().background_image.0;
                let elem_bgs = &mut elem.background_images;

                let len = style_bgs.len();
                elem_bgs.resize_with(len, || None);

                for idx in 0..len {
                    let background_image = &style_bgs[idx];
                    let new_bg_image = match background_image {
                        StyloImage::Url(ComputedUrl::Valid(new_url)) => {
                            let old_bg_image = elem_bgs[idx].as_ref();
                            let old_bg_image_url = old_bg_image.map(|data| &data.url);
                            if old_bg_image_url.is_some_and(|old_url| **new_url == **old_url) {
                                break;
                            }

                            // Check cache first
                            let url_str = new_url.as_str();
                            if let Some(cached_image) = self.image_cache.get(url_str) {
                                #[cfg(feature = "tracing")]
                                tracing::info!("Loading image {url_str} from cache");
                                Some(BackgroundImageData {
                                    url: new_url.clone(),
                                    status: Status::Ok,
                                    image: cached_image.clone(),
                                })
                            } else if let Some(waiting_list) = self.pending_images.get_mut(url_str)
                            {
                                // Image is already being fetched, queue this node
                                #[cfg(feature = "tracing")]
                                tracing::info!(
                                    "Image {url_str} already pending, queueing node {node_id}"
                                );
                                waiting_list.push((node_id, ImageType::Background(idx)));
                                Some(BackgroundImageData::new(new_url.clone()))
                            } else {
                                // Start fetch and track as pending
                                #[cfg(feature = "tracing")]
                                tracing::info!("Fetching image {url_str}");
                                self.pending_images.insert(
                                    url_str.to_string(),
                                    vec![(node_id, ImageType::Background(idx))],
                                );

                                self.net_provider.fetch(
                                    doc_id,
                                    Request::get((**new_url).clone()),
                                    ResourceHandler::boxed(
                                        self.tx.clone(),
                                        doc_id,
                                        None, // Don't pass node_id, we'll handle via pending_images
                                        self.shell_provider.clone(),
                                        ImageHandler::new(ImageType::Background(idx)),
                                    ),
                                );

                                Some(BackgroundImageData::new(new_url.clone()))
                            }
                        }
                        _ => None,
                    };

                    // Element will always exist due to resize_with above
                    elem_bgs[idx] = new_bg_image;
                }
            }

            // In non-incremental mode we unconditionally clear the Taffy cache.
            // In incremental mode this is handled as part of damage propagation.
            if NON_INCREMENTAL {
                node.cache.clear();
                if let Some(inline_layout) = node
                    .data
                    .downcast_element_mut()
                    .and_then(|el| el.inline_layout_data.as_mut())
                {
                    inline_layout.content_widths = None;
                }
            }

            node.style.display
        };

        // If the node has children, then take those children and...
        let children = self.nodes[node_id].layout_children.borrow_mut().take();
        if let Some(mut children) = children {
            children.retain(|&id| self.nodes.contains(id));
            let is_flex_or_grid = matches!(display, taffy::Display::Flex | taffy::Display::Grid);

            // Recursively call flush_styles_to_layout on each child
            for &child in children.iter() {
                self.flush_styles_to_layout_impl(
                    child,
                    match self.nodes[child].is_stacking_context_root(is_flex_or_grid) {
                        true => None,
                        false => Some(stacking_context),
                    },
                );
            }

            // Sort layout_children
            if is_flex_or_grid {
                children.sort_by(|left, right| {
                    let left_node = self.nodes.get(*left).unwrap();
                    let right_node = self.nodes.get(*right).unwrap();
                    left_node.order().cmp(&right_node.order())
                });
            }

            // Reserve space for paint_children
            let mut paint_children = self.nodes[node_id].paint_children.borrow_mut();
            if paint_children.is_none() {
                *paint_children = Some(Vec::new());
            }
            let paint_children = paint_children.as_mut().unwrap();
            paint_children.clear();
            paint_children.reserve(children.len());

            // Push children to either paint_children or layout_children depending on
            for &child_id in children.iter() {
                let child = &self.nodes[child_id];

                let Some(style) = child.primary_styles() else {
                    paint_children.push(child_id);
                    continue;
                };

                let position = style.clone_position();
                let z_index = style.clone_z_index().integer_or(0);

                // A static flex/grid item with a z-index is a stacking-context
                // root too (see `Node::is_stacking_context_root`), but it was
                // excluded here, so it stayed in `paint_children` -- which for
                // flex/grid is sorted by the CSS `order` property, not z-index.
                // Two such items both scored 0 and the stable sort just kept DOM
                // order, making z-index inert on every flex and grid item.
                //
                // TODO: more complete hoisting detection
                let positioned = position != Position::Static
                    || (is_flex_or_grid && !style.clone_z_index().is_auto());
                if positioned && z_index != 0 {
                    stacking_context.children.push(HoistedPaintChild {
                        node_id: child_id,
                        z_index,
                        position: taffy::Point::ZERO,
                    })
                } else {
                    paint_children.push(child_id);
                }
            }

            // Sort paint_children
            paint_children.sort_by(|left, right| {
                let left_node = self.nodes.get(*left).unwrap();
                let right_node = self.nodes.get(*right).unwrap();
                node_to_paint_order(left_node, is_flex_or_grid)
                    .cmp(&node_to_paint_order(right_node, is_flex_or_grid))
            });

            // Put children back
            *self.nodes[node_id].layout_children.borrow_mut() = Some(children);
        }

        if let Some(parent_stacking_context) = parent_stacking_context {
            let position = self.nodes[node_id].final_layout.location;
            let scroll_offset = self.nodes[node_id].scroll_offset;
            for hoisted in stacking_context.children.iter_mut() {
                hoisted.position.x += position.x - scroll_offset.x as f32;
                hoisted.position.y += position.y - scroll_offset.y as f32;
            }
            parent_stacking_context
                .children
                .extend(stacking_context.children.iter().cloned());
        } else {
            stacking_context.sort();
            stacking_context.compute_content_size(self);
            self.nodes[node_id].stacking_context = Some(Box::new(new_stacking_context));
        }
    }
}

#[inline(always)]
fn float_to_order(pos: Float) -> i32 {
    match pos {
        Float::None => 0,
        _ => 1,
    }
}

/// Paint bucket within a stacking context, following CSS 2.1 Appendix E.
///
/// `Relative` and `Sticky` used to share bucket 0 with `Static`, so a later
/// static sibling painted OVER a relatively positioned element. Appendix E puts
/// in-flow non-positioned boxes in step 4, floats in step 5, and every
/// positioned box with `z-index: auto`/`0` in step 7 — above both. A positioned
/// box is never also a float for paint purposes, so the float bucket only
/// applies to in-flow content.
///
/// A transform likewise lifts an element into the positioned bucket (and makes
/// it a stacking context; see `Node::is_stacking_context_root`), which is what
/// Chrome does — without it a later static sibling painted over any transformed
/// element.
#[inline(always)]
fn node_to_paint_order(node: &Node, is_flex_or_grid: bool) -> i32 {
    let Some(style) = node.primary_styles() else {
        return 0;
    };
    if is_flex_or_grid {
        match style.clone_position() {
            Position::Static | Position::Relative | Position::Sticky => style.clone_order(),
            Position::Absolute | Position::Fixed => 0,
        }
    } else {
        match style.clone_position() {
            Position::Static if !crate::stylo_to_kurbo::has_2d_transform(style.get_box()) => {
                float_to_order(style.clone_float())
            }
            _ => 2,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::DocumentConfig;
    use crate::html_parser::HtmlDocument;
    use crate::traits_src::shell::Viewport;

    #[test]
    fn stale_stacking_context_after_dom_mutation_does_not_panic() {
        // Case A: a stacking-context root becomes `display: none` while its
        // previously hoisted child is removed/dropped from the slab.
        let html = r#"<!doctype html><html><body style="margin:0">
            <div id="wrap" style="position:relative;z-index:10;width:200px;height:200px">
                <div id="inner">
                    <div id="badge" style="position:absolute;z-index:20;width:40px;height:20px">hi</div>
                </div>
            </div>
        </body></html>"#;
        let mut doc = HtmlDocument::from_html(
            html,
            DocumentConfig {
                viewport: Some(Viewport::new(800, 600, 1.0, Default::default())),
                ..Default::default()
            },
        );
        doc.resolve(0.0);

        let wrap_id = doc.query_selector("#wrap").unwrap().unwrap();
        let inner_id = doc.query_selector("#inner").unwrap().unwrap();
        let badge_id = doc.query_selector("#badge").unwrap().unwrap();
        let sc_children: Vec<usize> = doc
            .get_node(wrap_id)
            .unwrap()
            .stacking_context
            .as_ref()
            .expect("wrap should establish a stacking context on initial resolve")
            .children
            .iter()
            .map(|c| c.node_id)
            .collect();
        assert!(sc_children.contains(&badge_id), "badge should be hoisted into wrap");

        let style_attr = markup5ever::QualName::new(
            None,
            markup5ever::ns!(),
            markup5ever::LocalName::from("style"),
        );
        let mut mutator = doc.mutate();
        mutator.set_attribute(wrap_id, style_attr.clone(), "display:none");
        mutator.set_inner_html(inner_id, "");
        drop(mutator);

        doc.resolve(1.0);
        assert!(
            doc.get_node(wrap_id).unwrap().stacking_context.is_none(),
            "display:none node should not keep a stale stacking context"
        );

        // Case B: a stacking-context root stops being one (z-index/position reset)
        // while its previously hoisted child is dropped.
        let mut doc2 = HtmlDocument::from_html(
            html,
            DocumentConfig {
                viewport: Some(Viewport::new(800, 600, 1.0, Default::default())),
                ..Default::default()
            },
        );
        doc2.resolve(0.0);
        let wrap2 = doc2.query_selector("#wrap").unwrap().unwrap();
        let inner2 = doc2.query_selector("#inner").unwrap().unwrap();
        let mut mutator = doc2.mutate();
        mutator.set_attribute(wrap2, style_attr, "position:static;z-index:auto");
        mutator.set_inner_html(inner2, "");
        drop(mutator);
        doc2.resolve(1.0);
        assert!(
            doc2.get_node(wrap2).unwrap().stacking_context.is_none(),
            "stale stacking context should be cleared once node stops being a root"
        );

        // Case C: a stacking-context root is detached via remove_node while its
        // previously hoisted child is dropped.
        let mut doc3 = HtmlDocument::from_html(
            html,
            DocumentConfig {
                viewport: Some(Viewport::new(800, 600, 1.0, Default::default())),
                ..Default::default()
            },
        );
        doc3.resolve(0.0);
        let wrap3 = doc3.query_selector("#wrap").unwrap().unwrap();
        let badge3 = doc3.query_selector("#badge").unwrap().unwrap();
        let mut mutator = doc3.mutate();
        mutator.remove_node(wrap3);
        mutator.remove_and_drop_node(badge3);
        drop(mutator);
        doc3.resolve(1.0);
    }

    #[test]
    fn inline_block_with_floated_child_computes_shrink_to_fit_width() {
        let html = r#"<!doctype html><html><body style="margin:0">
            <div style="width:400px">
                <div id="wrap" style="display:inline-block;padding:4px">
                    <a id="btn" style="float:right;width:40px;height:40px"></a>
                </div>
            </div>
        </body></html>"#;
        let mut doc = HtmlDocument::from_html(
            html,
            DocumentConfig {
                viewport: Some(Viewport::new(800, 600, 1.0, Default::default())),
                ..Default::default()
            },
        );
        doc.resolve(0.0);

        let wrap = doc.get_node(doc.query_selector("#wrap").unwrap().unwrap()).unwrap();
        let btn = doc.get_node(doc.query_selector("#btn").unwrap().unwrap()).unwrap();
        assert!(
            (wrap.final_layout.size.width - 48.0).abs() < 0.5,
            "inline-block shrink-to-fit width should include floated child (expected ~48, got {})",
            wrap.final_layout.size.width
        );
        assert!(
            (btn.final_layout.location.x - 4.0).abs() < 0.5,
            "floated child should stay inside padding box at x=4 (got {})",
            btn.final_layout.location.x
        );
    }

    #[test]
    fn fixed_element_inside_relative_parent_attaches_to_root() {
        let html = r#"<!doctype html><html><body style="margin:0">
            <div id="rel" style="position:relative;margin-top:120px;margin-left:80px;width:200px;height:200px">
                <div id="fix" style="position:fixed;top:10px;left:14px;width:50px;height:24px"></div>
            </div>
        </body></html>"#;
        let mut doc = HtmlDocument::from_html(
            html,
            DocumentConfig {
                viewport: Some(Viewport::new(800, 600, 1.0, Default::default())),
                ..Default::default()
            },
        );
        doc.resolve(0.0);

        let fix_id = doc.query_selector("#fix").unwrap().unwrap();
        let pos = doc.get_node(fix_id).unwrap().absolute_position(0.0, 0.0);
        assert!(
            (pos.x - 14.0).abs() < 0.5 && (pos.y - 10.0).abs() < 0.5,
            "position:fixed inside position:relative should position relative to viewport root (got {:?})",
            pos
        );
    }

    #[test]
    fn submit_input_uses_inline_block_and_border_box_defaults() {
        let html = r#"<!doctype html><html><body style="margin:0">
            <input id="go" type="submit" value="Search" style="height:36px;padding:0 16px;border:1px solid #ccc">
        </body></html>"#;
        let mut doc = HtmlDocument::from_html(
            html,
            DocumentConfig {
                viewport: Some(Viewport::new(800, 600, 1.0, Default::default())),
                ..Default::default()
            },
        );
        doc.resolve(0.0);

        let go = doc.get_node(doc.query_selector("#go").unwrap().unwrap()).unwrap();
        assert!(
            (go.final_layout.size.height - 36.0).abs() < 0.5,
            "submit input should default to border-box so height:36px stays 36px (got {})",
            go.final_layout.size.height
        );
    }
}
