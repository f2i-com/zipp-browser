//! Enable the dom to participate in styling by servo
//!

use std::ptr::NonNull;
use std::sync::atomic::Ordering;

use crate::layout::damage::compute_layout_damage;
use crate::node::Node;
use crate::node::NodeData;
use markup5ever::{LocalName, LocalNameStaticSet, Namespace, NamespaceStaticSet, local_name};
use selectors::bloom::BLOOM_HASH_MASK;
use selectors::{
    Element, OpaqueElement,
    attr::{AttrSelectorOperation, AttrSelectorOperator, NamespaceConstraint},
    matching::{ElementSelectorFlags, MatchingContext, VisitedHandlingMode},
    sink::Push,
};
use style::CaseSensitivityExt;
use style::animation::AnimationSetKey;
use style::animation::AnimationState;
use style::applicable_declarations::ApplicableDeclarationBlock;
use style::bloom::each_relevant_element_hash;
use style::color::AbsoluteColor;
use style::data::{ElementDataMut, ElementDataRef};
use style::dom::AttributeProvider;
use style::global_style_data::STYLE_THREAD_POOL;
use style::invalidation::element::restyle_hints::RestyleHint;
use style::properties::ComputedValues;
use style::properties::{Importance, PropertyDeclaration};
use style::rule_tree::CascadeLevel;
use style::selector_parser::PseudoElement;
use style::selector_parser::RestyleDamage;
use style::stylesheets::layer_rule::LayerOrder;
use style::stylesheets::scope_rule::ImplicitScopeRoot;
use style::values::AtomString;
use style::values::computed::Percentage;
use style::{
    Atom,
    context::{
        QuirksMode, RegisteredSpeculativePainter, RegisteredSpeculativePainters,
        SharedStyleContext, StyleContext,
    },
    dom::{LayoutIterator, NodeInfo, OpaqueNode, TDocument, TElement, TNode, TShadowRoot},
    global_style_data::GLOBAL_STYLE_DATA,
    properties::PropertyDeclarationBlock,
    selector_parser::{NonTSPseudoClass, SelectorImpl},
    servo_arc::{Arc, ArcBorrow},
    shared_lock::{Locked, SharedRwLock, StylesheetGuards},
    thread_state::ThreadState,
    traversal::{DomTraversal, PerLevelTraversalData},
    traversal_flags::TraversalFlags,
    values::{AtomIdent, GenericAtomIdent},
};
use style_dom::ElementState;

use style::values::computed::text::TextAlign as StyloTextAlign;

impl crate::document::BaseDocument {
    /// Drop every animation and transition still keyed to a node id.
    ///
    /// Animation state is keyed by OPAQUE NODE ID, and replacing the body frees
    /// that whole subtree and re-parses it — so the ids come straight back and a
    /// brand-new element silently inherits whatever the previous occupant of
    /// that id was running. On lance.name a plain unclassed `<div>` adopted a
    /// neighbour's `rise` keyframes, whose 0% frame is `opacity: 0`, and sat
    /// there permanently: after the third menu toggle the hero vanished while
    /// its layout stayed pixel-identical.
    ///
    /// Everything visible lives under `<body>`, so after a body swap there is no
    /// animation worth keeping. The new elements start their animations fresh,
    /// which is also what they should do — they really are new elements.
    /// Forget every cached node id held as interaction state.
    ///
    /// Replacing the body frees that whole subtree, but the document keeps
    /// pointing at the node the mouse was over, the node with focus, and the
    /// node that received the last mousedown. The very next mouse move walks
    /// the hovered node's ancestors to work out what changed, indexes a freed
    /// id and panics with "invalid key".
    ///
    /// This only became reachable once in-page routing started replacing the
    /// body instead of navigating: a real navigation built a whole new document
    /// and took this state with it.
    pub fn clear_interaction_state(&mut self) {
        self.hover_node_id = None;
        self.hover_node_is_text = false;
        self.focus_node_id = None;
        self.active_node_id = None;
        self.mousedown_node_id = None;
        self.changed_nodes.clear();
    }

    pub fn clear_animation_state(&mut self) {
        self.animations.sets.write().clear();
        self.has_active_animations = false;
    }

    pub fn resolve_stylist(&mut self, now: f64) {
        style::thread_state::enter(ThreadState::LAYOUT);

        let guard = &self.guard;
        let guards = StylesheetGuards {
            author: &guard.read(),
            ua_or_user: &guard.read(),
        };

        let root = TDocument::as_node(&&self.nodes[0])
            .first_element_child()
            .unwrap()
            .as_element()
            .unwrap();

        self.stylist
            .flush(&guards)
            .process_style(root, Some(&self.snapshots));

        // Mark actively animating nodes as dirty
        let mut sets = self.animations.sets.write();
        for (key, set) in sets.iter_mut() {
            let node_id = key.node.id();
            self.nodes[node_id].set_restyle_hint(RestyleHint::RESTYLE_SELF);

            for animation in set.animations.iter_mut() {
                if animation.state == AnimationState::Pending && animation.started_at <= now {
                    animation.state = AnimationState::Running;
                }
                animation.iterate_if_necessary(now);

                if animation.state == AnimationState::Running && animation.has_ended(now) {
                    animation.state = AnimationState::Finished;
                }
            }

            for transition in set.transitions.iter_mut() {
                if transition.state == AnimationState::Pending && transition.start_time <= now {
                    transition.state = AnimationState::Running;
                }
                if transition.state == AnimationState::Running && transition.has_ended(now) {
                    transition.state = AnimationState::Finished;
                }
            }
        }
        drop(sets);

        // Build the style context used by the style traversal
        let context = SharedStyleContext {
            traversal_flags: TraversalFlags::empty(),
            stylist: &self.stylist,
            options: GLOBAL_STYLE_DATA.options.clone(),
            guards,
            visited_styles_enabled: false,
            animations: self.animations.clone(),
            current_time_for_animations: now,
            snapshot_map: &self.snapshots,
            registered_speculative_painters: &RegisteredPaintersImpl,
        };

        // components/layout_2020/lib.rs:983
        let root = self.root_element();
        // dbg!(root);
        let token = RecalcStyle::pre_traverse(root, &context);

        if token.should_traverse() {
            // `STYLE_THREAD_POOL` is a single process-wide Rayon pool whose worker
            // threads keep a thread-local `AtomicRefCell<ThreadLocalStyleContext>`.
            // Serialize concurrent callers (e.g. parallel tests or multiple embedded
            // documents) so two traversals never work-steal onto the same worker and
            // trip `AtomicRefCell: already mutably borrowed`.
            static STYLO_TRAVERSE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
            let _traverse_guard = STYLO_TRAVERSE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            // Style the elements, resolving their data
            let traverser = RecalcStyle::new(context);
            let rayon_pool = STYLE_THREAD_POOL.pool();
            style::driver::traverse_dom(&traverser, token, rayon_pool.as_ref());
        }

        for opaque in self.snapshots.keys() {
            let id = opaque.id();
            if let Some(node) = self.nodes.get_mut(id) {
                node.has_snapshot = false;
            }
        }
        self.snapshots.clear();

        let mut sets = self.animations.sets.write();
        for set in sets.values_mut() {
            set.clear_canceled_animations();
            for animation in set.animations.iter_mut() {
                animation.is_new = false;
            }
            for transition in set.transitions.iter_mut() {
                transition.is_new = false;
            }
        }
        sets.retain(|_, state| !state.is_empty());
        self.has_active_animations = sets.values().any(|state| state.needs_animation_ticks());

        // Maybe run garbage collection. Stylo has internal to determine whether to run or not.
        self.stylist.rule_tree().maybe_gc();

        style::thread_state::exit(ThreadState::LAYOUT);
    }
}

/// A handle to a node that Servo's style traits are implemented against
///
/// Since these node references are not persistent (IE we don't keep the pointers around between frames), we choose to just implement
/// the tree structure in the nodes themselves, and temporarily give out pointers during the layout phase.
type DomNode<'a> = &'a Node;

impl<'a> TDocument for DomNode<'a> {
    type ConcreteNode = DomNode<'a>;

#[inline]
    fn as_node(&self) -> Self::ConcreteNode {
        self
    }

#[inline]
    fn is_html_document(&self) -> bool {
        true
    }

#[inline]
    fn quirks_mode(&self) -> QuirksMode {
        QuirksMode::NoQuirks
    }

#[inline]
    fn shared_lock(&self) -> &SharedRwLock {
        &self.guard
    }
}

impl NodeInfo for DomNode<'_> {
#[inline]
    fn is_element(&self) -> bool {
        Node::is_element(self)
    }

#[inline]
    fn is_text_node(&self) -> bool {
        Node::is_text_node(self)
    }
}

impl<'a> TShadowRoot for DomNode<'a> {
    type ConcreteNode = DomNode<'a>;

#[inline]
    fn as_node(&self) -> Self::ConcreteNode {
        self
    }

#[inline]
    fn host(&self) -> <Self::ConcreteNode as TNode>::ConcreteElement {
        todo!("Shadow roots not implemented")
    }

#[inline]
    fn style_data<'b>(&self) -> Option<&'b style::stylist::CascadeData>
    where
        Self: 'b,
    {
        todo!("Shadow roots not implemented")
    }
}

// components/styleaapper.rs:
impl<'a> TNode for DomNode<'a> {
    type ConcreteElement = DomNode<'a>;
    type ConcreteDocument = DomNode<'a>;
    type ConcreteShadowRoot = DomNode<'a>;

#[inline]
    fn parent_node(&self) -> Option<Self> {
        self.parent.map(|id| self.with(id))
    }

#[inline]
    fn first_child(&self) -> Option<Self> {
        self.children.first().map(|id| self.with(*id))
    }

#[inline]
    fn last_child(&self) -> Option<Self> {
        self.children.last().map(|id| self.with(*id))
    }

#[inline]
    fn prev_sibling(&self) -> Option<Self> {
        self.backward(1)
    }

#[inline]
    fn next_sibling(&self) -> Option<Self> {
        self.forward(1)
    }

#[inline]
    fn owner_doc(&self) -> Self::ConcreteDocument {
        self.with(1)
    }

#[inline]
    fn is_in_document(&self) -> bool {
        true
    }

    // I think this is the same as parent_node only in the cases when the direct parent is not a real element, forcing us
    // to travel upwards
    //
    // For the sake of this demo, we're just going to return the parent node ann
#[inline]
    fn traversal_parent(&self) -> Option<Self::ConcreteElement> {
        self.parent_node().and_then(|node| node.as_element())
    }

#[inline]
    fn opaque(&self) -> OpaqueNode {
        OpaqueNode(self.id)
    }

#[inline]
    fn debug_id(self) -> usize {
        self.id
    }

#[inline]
    fn as_element(&self) -> Option<Self::ConcreteElement> {
        match self.data {
            NodeData::Element { .. } => Some(self),
            _ => None,
        }
    }

#[inline]
    fn as_document(&self) -> Option<Self::ConcreteDocument> {
        match self.data {
            NodeData::Document => Some(self),
            _ => None,
        }
    }

#[inline]
    fn as_shadow_root(&self) -> Option<Self::ConcreteShadowRoot> {
        // TODO: implement shadow DOM
        None
    }
}

impl AttributeProvider for DomNode<'_> {
    fn get_attr(&self, attr: &style::LocalName, _ns: &style::Namespace) -> Option<String> {
        // TODO: filter by namespace
        self.attr(attr.0.clone()).map(|s| s.to_string())
    }
}

impl selectors::Element for DomNode<'_> {
    type Impl = SelectorImpl;

#[inline]
    fn opaque(&self) -> selectors::OpaqueElement {
        // This correctly uses a unique id for the OpaqueElement (unlike using a pointer to the "slot")
        // However, it makes it impossible for us to "rehydrate" the OpaqueElement back into an actual Element
        // which is required to implement the `implicit_scope_for_sheet_in_shadow_root` method below
        //
        // We should see if selectors will accept a PR that allows us to use 128bits for the OpaqueElement. Or
        // find some other solution that will enable "rehydration". This is required to enable and use the
        // Shadow DOM functionality in Stylo.
        let non_null = NonNull::new((self.id + 1) as *mut ()).unwrap();
        OpaqueElement::from_non_null_ptr(non_null)
    }

#[inline]
    fn parent_element(&self) -> Option<Self> {
        TElement::traversal_parent(self)
    }

#[inline]
    fn parent_node_is_shadow_root(&self) -> bool {
        false
    }

#[inline]
    fn containing_shadow_host(&self) -> Option<Self> {
        None
    }

#[inline]
    fn is_pseudo_element(&self) -> bool {
        matches!(self.data, NodeData::AnonymousBlock(_))
    }

    // These methods are implemented naively since we only threaded real nodes and not fake nodes
    // we should try and use `find` instead of this foward/backward stuff since its ugly and slow
#[inline]
    fn prev_sibling_element(&self) -> Option<Self> {
        let mut n = 1;
        while let Some(node) = self.backward(n) {
            if node.is_element() {
                return Some(node);
            }
            n += 1;
        }

        None
    }

#[inline]
    fn next_sibling_element(&self) -> Option<Self> {
        let mut n = 1;
        while let Some(node) = self.forward(n) {
            if node.is_element() {
                return Some(node);
            }
            n += 1;
        }

        None
    }

#[inline]
    fn first_element_child(&self) -> Option<Self> {
        let mut children = self.dom_children();
        children.find(|child| child.is_element())
    }

#[inline]
    fn is_html_element_in_html_document(&self) -> bool {
        true // self.has_namespace(ns!(html))
    }

#[inline]
    fn has_local_name(&self, local_name: &LocalName) -> bool {
        self.data.is_element_with_tag_name(local_name)
    }

#[inline]
    fn has_namespace(&self, ns: &Namespace) -> bool {
        self.element_data().expect("Not an element").name.ns == *ns
    }

#[inline]
    fn is_same_type(&self, other: &Self) -> bool {
        self.local_name() == other.local_name() && self.namespace() == other.namespace()
    }

#[inline]
    fn attr_matches(
        &self,
        _ns: &NamespaceConstraint<&GenericAtomIdent<NamespaceStaticSet>>,
        local_name: &GenericAtomIdent<LocalNameStaticSet>,
        operation: &AttrSelectorOperation<&AtomString>,
    ) -> bool {
        let Some(attr_value) = self.data.attr(local_name.0.clone()) else {
            return false;
        };

        match operation {
            AttrSelectorOperation::Exists => true,
            AttrSelectorOperation::WithValue {
                operator,
                case_sensitivity: _,
                value,
            } => {
                let value = value.as_ref();

                // TODO: case sensitivity
                match operator {
                    AttrSelectorOperator::Equal => attr_value == value,
                    AttrSelectorOperator::Includes => attr_value
                        .split_ascii_whitespace()
                        .any(|word| word == value),
                    AttrSelectorOperator::DashMatch => {
                        // Represents elements with an attribute name of attr whose value can be exactly value
                        // or can begin with value immediately followed by a hyphen, - (U+002D)
                        attr_value.starts_with(value)
                            && (attr_value.len() == value.len()
                                || attr_value.chars().nth(value.len()) == Some('-'))
                    }
                    AttrSelectorOperator::Prefix => attr_value.starts_with(value),
                    AttrSelectorOperator::Substring => attr_value.contains(value),
                    AttrSelectorOperator::Suffix => attr_value.ends_with(value),
                }
            }
        }
    }

#[inline]
    fn match_non_ts_pseudo_class(
        &self,
        pseudo_class: &<Self::Impl as selectors::SelectorImpl>::NonTSPseudoClass,
        _context: &mut MatchingContext<Self::Impl>,
    ) -> bool {
        match *pseudo_class {
            NonTSPseudoClass::Active => self.element_state.contains(ElementState::ACTIVE),
            NonTSPseudoClass::AnyLink => self
                .data
                .downcast_element()
                .map(|elem| {
                    (elem.name.local == local_name!("a") || elem.name.local == local_name!("area"))
                        && elem.attr(local_name!("href")).is_some()
                })
                .unwrap_or(false),
            NonTSPseudoClass::Checked => self
                .data
                .downcast_element()
                .and_then(|elem| elem.checkbox_input_checked())
                .unwrap_or(false),
            NonTSPseudoClass::Valid => false,
            NonTSPseudoClass::Invalid => false,
            NonTSPseudoClass::Defined => false,
            NonTSPseudoClass::Disabled => self.element_state.contains(ElementState::DISABLED),
            NonTSPseudoClass::Enabled => self.element_state.contains(ElementState::ENABLED),
            NonTSPseudoClass::Focus => self.element_state.contains(ElementState::FOCUS),
            NonTSPseudoClass::FocusWithin => false,
            NonTSPseudoClass::FocusVisible => false,
            NonTSPseudoClass::Fullscreen => false,
            NonTSPseudoClass::Hover => self.element_state.contains(ElementState::HOVER),
            NonTSPseudoClass::Indeterminate => false,
            NonTSPseudoClass::Lang(_) => false,
            NonTSPseudoClass::CustomState(_) => false,
            NonTSPseudoClass::Link => self
                .data
                .downcast_element()
                .map(|elem| {
                    (elem.name.local == local_name!("a") || elem.name.local == local_name!("area"))
                        && elem.attr(local_name!("href")).is_some()
                })
                .unwrap_or(false),
            NonTSPseudoClass::PlaceholderShown => false,
            NonTSPseudoClass::ReadWrite => false,
            NonTSPseudoClass::ReadOnly => false,
            NonTSPseudoClass::ServoNonZeroBorder => false,
            NonTSPseudoClass::Target => false,
            NonTSPseudoClass::Visited => false,
            NonTSPseudoClass::Autofill => false,
            NonTSPseudoClass::Default => false,

            NonTSPseudoClass::InRange => false,
            NonTSPseudoClass::Modal => false,
            NonTSPseudoClass::Open => false,
            NonTSPseudoClass::Optional => false,
            NonTSPseudoClass::OutOfRange => false,
            NonTSPseudoClass::PopoverOpen => false,
            NonTSPseudoClass::Required => false,
            NonTSPseudoClass::UserInvalid => false,
            NonTSPseudoClass::UserValid => false,
            NonTSPseudoClass::MozMeterOptimum => false,
            NonTSPseudoClass::MozMeterSubOptimum => false,
            NonTSPseudoClass::MozMeterSubSubOptimum => false,
        }
    }

#[inline]
    fn match_pseudo_element(
        &self,
        pe: &PseudoElement,
        _context: &mut MatchingContext<Self::Impl>,
    ) -> bool {
        match self.data {
            NodeData::AnonymousBlock(_) => *pe == PseudoElement::ServoAnonymousBox,
            _ => false,
        }
    }

#[inline]
    fn apply_selector_flags(&self, flags: ElementSelectorFlags) {
        // Handle flags that apply to the element.
        let self_flags = flags.for_self();
        if !self_flags.is_empty() {
            self.selector_flags
                .set(self.selector_flags.get() | self_flags);
        }

        // Handle flags that apply to the parent.
        let parent_flags = flags.for_parent();
        if !parent_flags.is_empty() {
            if let Some(parent) = self.parent_node() {
                parent
                    .selector_flags
                    .set(parent.selector_flags.get() | parent_flags);
            }
        }
    }

#[inline]
    fn is_link(&self) -> bool {
        self.data.is_element_with_tag_name(&local_name!("a"))
    }

#[inline]
    fn is_html_slot_element(&self) -> bool {
        false
    }

#[inline]
    fn has_id(
        &self,
        id: &<Self::Impl as selectors::SelectorImpl>::Identifier,
        case_sensitivity: selectors::attr::CaseSensitivity,
    ) -> bool {
        self.element_data()
            .and_then(|data| data.id.as_ref())
            .map(|id_attr| case_sensitivity.eq_atom(id_attr, id))
            .unwrap_or(false)
    }

#[inline]
    fn has_class(
        &self,
        search_name: &<Self::Impl as selectors::SelectorImpl>::Identifier,
        case_sensitivity: selectors::attr::CaseSensitivity,
    ) -> bool {
        let class_attr = self.data.attr(local_name!("class"));
        if let Some(class_attr) = class_attr {
            // split the class attribute
            for pheme in class_attr.split_ascii_whitespace() {
                let atom = Atom::from(pheme);
                if case_sensitivity.eq_atom(&atom, search_name) {
                    return true;
                }
            }
        }

        false
    }

#[inline]
    fn imported_part(
        &self,
        _name: &<Self::Impl as selectors::SelectorImpl>::Identifier,
    ) -> Option<<Self::Impl as selectors::SelectorImpl>::Identifier> {
        None
    }

#[inline]
    fn is_part(&self, _name: &<Self::Impl as selectors::SelectorImpl>::Identifier) -> bool {
        false
    }

#[inline]
    fn is_empty(&self) -> bool {
        self.dom_children().next().is_none()
    }

#[inline]
    fn is_root(&self) -> bool {
        self.parent_node()
            .and_then(|parent| parent.parent_node())
            .is_none()
    }

#[inline]
    fn has_custom_state(
        &self,
        _name: &<Self::Impl as selectors::SelectorImpl>::Identifier,
    ) -> bool {
        false
    }

#[inline]
    fn add_element_unique_hashes(&self, filter: &mut selectors::bloom::BloomFilter) -> bool {
        each_relevant_element_hash(*self, |hash| filter.insert_hash(hash & BLOOM_HASH_MASK));
        true
    }
}

impl<'a> TElement for DomNode<'a> {
    type ConcreteNode = DomNode<'a>;

    type TraversalChildrenIterator = Traverser<'a>;

#[inline]
    fn as_node(&self) -> Self::ConcreteNode {
        self
    }

#[inline]
    fn implicit_scope_for_sheet_in_shadow_root(
        _opaque_host: OpaqueElement,
        _sheet_index: usize,
    ) -> Option<ImplicitScopeRoot> {
        // We cannot currently implement this as we are using the NodeId as the OpaqueElement,
        // and need a reference to the Slab to convert it back into an Element
        //
        // Luckily it is only needed for shadow dom.
        todo!();
    }

#[inline]
    fn traversal_children(&self) -> style::dom::LayoutIterator<Self::TraversalChildrenIterator> {
        LayoutIterator(Traverser {
            // dom: self.tree(),
            parent: self,
            child_index: 0,
        })
    }

#[inline]
    fn is_html_element(&self) -> bool {
        self.is_element()
    }

    // not implemented.....
#[inline]
    fn is_mathml_element(&self) -> bool {
        false
    }

    // need to check the namespace
#[inline]
    fn is_svg_element(&self) -> bool {
        false
    }

#[inline]
    fn style_attribute(&self) -> Option<ArcBorrow<'_, Locked<PropertyDeclarationBlock>>> {
        self.element_data()
            .expect("Not an element")
            .style_attribute
            .as_ref()
            .map(|f| f.borrow_arc())
    }

#[inline]
    fn state(&self) -> ElementState {
        self.element_state
    }

#[inline]
    fn has_part_attr(&self) -> bool {
        false
    }

#[inline]
    fn exports_any_part(&self) -> bool {
        false
    }

#[inline]
    fn id(&self) -> Option<&style::Atom> {
        self.element_data().and_then(|data| data.id.as_ref())
    }

#[inline]
    fn each_class<F>(&self, mut callback: F)
    where
        F: FnMut(&style::values::AtomIdent),
    {
        let class_attr = self.data.attr(local_name!("class"));
        if let Some(class_attr) = class_attr {
            // split the class attribute
            for pheme in class_attr.split_ascii_whitespace() {
                let atom = Atom::from(pheme); // interns the string
                callback(AtomIdent::cast(&atom));
            }
        }
    }

#[inline]
    fn each_attr_name<F>(&self, mut callback: F)
    where
        F: FnMut(&style::LocalName),
    {
        if let Some(attrs) = self.data.attrs() {
            for attr in attrs.iter() {
                callback(&GenericAtomIdent(attr.name.local.clone()));
            }
        }
    }

#[inline]
    fn has_dirty_descendants(&self) -> bool {
        Node::has_dirty_descendants(self)
    }

#[inline]
    fn has_snapshot(&self) -> bool {
        self.has_snapshot
    }

#[inline]
    fn handled_snapshot(&self) -> bool {
        self.snapshot_handled.load(Ordering::SeqCst)
    }

    unsafe fn set_handled_snapshot(&self) {
        self.snapshot_handled.store(true, Ordering::SeqCst);
    }

    unsafe fn set_dirty_descendants(&self) {
        Node::set_dirty_descendants(self);
        Node::mark_ancestors_dirty(self);
    }

    unsafe fn unset_dirty_descendants(&self) {
        Node::unset_dirty_descendants(self);
    }

#[inline]
    fn store_children_to_process(&self, _n: isize) {
        unimplemented!()
    }

#[inline]
    fn did_process_child(&self) -> isize {
        unimplemented!()
    }

    unsafe fn ensure_data(&self) -> ElementDataMut<'_> {
        // SAFETY: stylo traversal has exclusive access to nodes
        unsafe { self.stylo_element_data.ensure_init() }
    }

    unsafe fn clear_data(&self) {
        // SAFETY: stylo traversal has exclusive access to nodes
        unsafe { self.stylo_element_data.clear() }
    }

#[inline]
    fn has_data(&self) -> bool {
        self.stylo_element_data.has_data()
    }

#[inline]
    fn borrow_data(&self) -> Option<ElementDataRef<'_>> {
        self.stylo_element_data.get()
    }

#[inline]
    fn mutate_data(&self) -> Option<ElementDataMut<'_>> {
        unsafe { self.stylo_element_data.unsafe_stylo_only_mut() }
    }

#[inline]
    fn skip_item_display_fixup(&self) -> bool {
        false
    }

#[inline]
    fn may_have_animations(&self) -> bool {
        true
    }

#[inline]
    fn has_animations(&self, context: &SharedStyleContext) -> bool {
        self.has_css_animations(context, None) || self.has_css_transitions(context, None)
    }

#[inline]
    fn has_css_animations(
        &self,
        context: &SharedStyleContext,
        pseudo_element: Option<PseudoElement>,
    ) -> bool {
        let key = AnimationSetKey::new(TNode::opaque(&TElement::as_node(self)), pseudo_element);
        context.animations.has_active_animations(&key)
    }

#[inline]
    fn has_css_transitions(
        &self,
        context: &SharedStyleContext,
        pseudo_element: Option<PseudoElement>,
    ) -> bool {
        let key = AnimationSetKey::new(TNode::opaque(&TElement::as_node(self)), pseudo_element);
        context.animations.has_active_transitions(&key)
    }

#[inline]
    fn animation_rule(
        &self,
        context: &SharedStyleContext,
    ) -> Option<Arc<Locked<PropertyDeclarationBlock>>> {
        let opaque = TNode::opaque(&TElement::as_node(self));
        context.animations.get_animation_declarations(
            &AnimationSetKey::new_for_non_pseudo(opaque),
            context.current_time_for_animations,
            &self.guard,
        )
    }

#[inline]
    fn transition_rule(
        &self,
        context: &SharedStyleContext,
    ) -> Option<Arc<Locked<PropertyDeclarationBlock>>> {
        let opaque = TNode::opaque(&TElement::as_node(self));
        context.animations.get_transition_declarations(
            &AnimationSetKey::new_for_non_pseudo(opaque),
            context.current_time_for_animations,
            &self.guard,
        )
    }

#[inline]
    fn shadow_root(&self) -> Option<<Self::ConcreteNode as TNode>::ConcreteShadowRoot> {
        None
    }

#[inline]
    fn containing_shadow(&self) -> Option<<Self::ConcreteNode as TNode>::ConcreteShadowRoot> {
        None
    }

#[inline]
    fn lang_attr(&self) -> Option<style::selector_parser::AttrValue> {
        None
    }

#[inline]
    fn match_element_lang(
        &self,
        _override_lang: Option<Option<style::selector_parser::AttrValue>>,
        _value: &style::selector_parser::Lang,
    ) -> bool {
        false
    }

#[inline]
    fn is_html_document_body_element(&self) -> bool {
        // Check node is a <body> element
        let is_body_element = self.data.is_element_with_tag_name(&local_name!("body"));

        // If it isn't then return early
        if !is_body_element {
            return false;
        }

        // If it is then check if it is a child of the root (<html>) element
        let root_node = &self.tree()[0];
        let root_element = TDocument::as_node(&root_node)
            .first_element_child()
            .unwrap();
        root_element.children.contains(&self.id)
    }

#[inline]
    fn synthesize_presentational_hints_for_legacy_attributes<V>(
        &self,
        _visited_handling: VisitedHandlingMode,
        hints: &mut V,
    ) where
        V: Push<style::applicable_declarations::ApplicableDeclarationBlock>,
    {
        let Some(elem) = self.data.downcast_element() else {
            return;
        };

        let tag = &elem.name.local;

        let mut push_style = |decl: PropertyDeclaration| {
            hints.push(ApplicableDeclarationBlock::from_declarations(
                Arc::new(
                    self.guard
                        .wrap(PropertyDeclarationBlock::with_one(decl, Importance::Normal)),
                ),
                CascadeLevel::PresHints,
                LayerOrder::root(),
            ));
        };

        fn parse_color_attr(value: &str) -> Option<(u8, u8, u8, f32)> {
            if !value.starts_with('#') {
                return None;
            }

            let value = &value[1..];
            if value.len() == 3 {
                let r = u8::from_str_radix(&value[0..1], 16).ok()?;
                let g = u8::from_str_radix(&value[1..2], 16).ok()?;
                let b = u8::from_str_radix(&value[2..3], 16).ok()?;
                return Some((r, g, b, 1.0));
            }

            if value.len() == 6 {
                let r = u8::from_str_radix(&value[0..2], 16).ok()?;
                let g = u8::from_str_radix(&value[2..4], 16).ok()?;
                let b = u8::from_str_radix(&value[4..6], 16).ok()?;
                return Some((r, g, b, 1.0));
            }

            None
        }

        fn parse_size_attr(
            value: &str,
            filter_fn: impl FnOnce(&f32) -> bool,
        ) -> Option<style::values::specified::LengthPercentage> {
            use style::values::specified::{AbsoluteLength, LengthPercentage, NoCalcLength};
            if let Some(value) = value.strip_suffix("px") {
                let val: f32 = value.parse().ok()?;
                return Some(LengthPercentage::Length(NoCalcLength::Absolute(
                    AbsoluteLength::Px(val),
                )));
            }

            if let Some(value) = value.strip_suffix("%") {
                let val: f32 = value.parse().ok()?;
                return Some(LengthPercentage::Percentage(Percentage(val / 100.0)));
            }

            let val: f32 = value.parse().ok().filter(filter_fn)?;
            Some(LengthPercentage::Length(NoCalcLength::Absolute(
                AbsoluteLength::Px(val),
            )))
        }

        for attr in elem.attrs() {
            let name = &attr.name.local;
            let value = attr.value.as_str();

            if *name == local_name!("align") {
                use style::values::specified::TextAlign;
                let keyword = match value {
                    "left" => Some(StyloTextAlign::MozLeft),
                    "right" => Some(StyloTextAlign::MozRight),
                    "center" => Some(StyloTextAlign::MozCenter),
                    _ => None,
                };

                if let Some(keyword) = keyword {
                    push_style(PropertyDeclaration::TextAlign(TextAlign::Keyword(keyword)));
                }
            }

            // https://html.spec.whatwg.org/multipage/rendering.html#dimRendering
            if *name == local_name!("width")
                && (*tag == local_name!("table")
                    || *tag == local_name!("col")
                    || *tag == local_name!("tr")
                    || *tag == local_name!("td")
                    || *tag == local_name!("th")
                    || *tag == local_name!("hr"))
            {
                let is_table = *tag == local_name!("table");
                if let Some(width) = parse_size_attr(value, |v| !is_table || *v != 0.0) {
                    use style::values::generics::{NonNegative, length::Size};

                    push_style(PropertyDeclaration::Width(Size::LengthPercentage(
                        NonNegative(width),
                    )));
                }
            }

            if *name == local_name!("height")
                && (*tag == local_name!("table")
                    || *tag == local_name!("thead")
                    || *tag == local_name!("tbody")
                    || *tag == local_name!("tfoot"))
            {
                if let Some(height) = parse_size_attr(value, |_| true) {
                    use style::values::generics::{NonNegative, length::Size};
                    push_style(PropertyDeclaration::Height(Size::LengthPercentage(
                        NonNegative(height),
                    )));
                }
            }

            if *name == local_name!("bgcolor") {
                use style::values::specified::Color;
                if let Some((r, g, b, a)) = parse_color_attr(value) {
                    push_style(PropertyDeclaration::BackgroundColor(
                        Color::from_absolute_color(AbsoluteColor::srgb_legacy(r, g, b, a)),
                    ));
                }
            }

            if *name == local_name!("hidden") {
                use style::values::specified::Display;
                push_style(PropertyDeclaration::Display(Display::None));
            }
        }
    }

#[inline]
    fn local_name(&self) -> &LocalName {
        &self.element_data().expect("Not an element").name.local
    }

#[inline]
    fn namespace(&self) -> &Namespace {
        &self.element_data().expect("Not an element").name.ns
    }

#[inline]
    fn query_container_size(
        &self,
        display: &style::values::specified::Display,
    ) -> euclid::default::Size2D<Option<app_units::Au>> {
        // Size containment does not apply to an inline box, nor to one that
        // generates no box: such an element is no size container.
        if display.is_none() || display.is_contents() || display.outside() == style::values::specified::box_::DisplayOutside::Inline {
            return Default::default();
        }
        // Recorded after each layout (`update_container_sizes`).
        let (width, height) = self.query_container_size;
        euclid::default::Size2D::new(
            width.map(app_units::Au::from_f32_px),
            height.map(app_units::Au::from_f32_px),
        )
    }

#[inline]
    fn each_custom_state<F>(&self, _callback: F)
    where
        F: FnMut(&AtomIdent),
    {
        todo!()
    }

#[inline]
    fn has_selector_flags(&self, flags: ElementSelectorFlags) -> bool {
        self.selector_flags.get().contains(flags)
    }

#[inline]
    fn relative_selector_search_direction(&self) -> ElementSelectorFlags {
        let flags = self.selector_flags.get();
        if flags.contains(ElementSelectorFlags::RELATIVE_SELECTOR_SEARCH_DIRECTION_ANCESTOR_SIBLING)
        {
            ElementSelectorFlags::RELATIVE_SELECTOR_SEARCH_DIRECTION_ANCESTOR_SIBLING
        } else if flags.contains(ElementSelectorFlags::RELATIVE_SELECTOR_SEARCH_DIRECTION_ANCESTOR)
        {
            ElementSelectorFlags::RELATIVE_SELECTOR_SEARCH_DIRECTION_ANCESTOR
        } else if flags.contains(ElementSelectorFlags::RELATIVE_SELECTOR_SEARCH_DIRECTION_SIBLING) {
            ElementSelectorFlags::RELATIVE_SELECTOR_SEARCH_DIRECTION_SIBLING
        } else {
            ElementSelectorFlags::empty()
        }
    }

#[inline]
    fn compute_layout_damage(old: &ComputedValues, new: &ComputedValues) -> RestyleDamage {
        compute_layout_damage(old, new)
        // ALL_DAMAGE
    }

    // fn update_animations(
    //     &self,
    //     before_change_style: Option<Arc<ComputedValues>>,
    //     tasks: style::context::UpdateAnimationsTasks,
    // ) {
    //     todo!()
    // }

    // fn process_post_animation(&self, tasks: style::context::PostAnimationTasks) {
    //     todo!()
    // }

    // fn needs_transitions_update(
    //     &self,
    //     before_change_style: &ComputedValues,
    //     after_change_style: &ComputedValues,
    // ) -> bool {
    //     todo!()
    // }
}

pub struct Traverser<'a> {
    // dom: &'a Slab<Node>,
    parent: DomNode<'a>,
    child_index: usize,
}

impl<'a> Iterator for Traverser<'a> {
    type Item = DomNode<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let node_id = self.parent.children.get(self.child_index)?;
        let node = self.parent.with(*node_id);

        self.child_index += 1;

        Some(node)
    }
}

impl std::hash::Hash for DomNode<'_> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        state.write_usize(self.id)
    }
}

/// Handle custom painters like images for layouting
///
/// todo: actually implement this
pub struct RegisteredPaintersImpl;
impl RegisteredSpeculativePainters for RegisteredPaintersImpl {
    fn get(&self, _name: &Atom) -> Option<&dyn RegisteredSpeculativePainter> {
        None
    }
}

use style::traversal::recalc_style_at;

pub struct RecalcStyle<'a> {
    context: SharedStyleContext<'a>,
}

impl<'a> RecalcStyle<'a> {
    pub fn new(context: SharedStyleContext<'a>) -> Self {
        RecalcStyle { context }
    }
}

#[allow(unsafe_code)]
impl<E> DomTraversal<E> for RecalcStyle<'_>
where
    E: TElement,
{
    fn process_preorder<F: FnMut(E::ConcreteNode)>(
        &self,
        traversal_data: &PerLevelTraversalData,
        context: &mut StyleContext<E>,
        node: E::ConcreteNode,
        note_child: F,
    ) {
        if let Some(el) = node.as_element() {
            // let mut data = el.mutate_data().unwrap();
            let mut data = unsafe { el.ensure_data() };
            recalc_style_at(self, traversal_data, context, el, &mut data, note_child);

            // Gets set later on
            unsafe { el.unset_dirty_descendants() }
        }
    }

    #[inline]
    fn needs_postorder_traversal() -> bool {
        false
    }

    fn process_postorder(&self, _style_context: &mut StyleContext<E>, _node: E::ConcreteNode) {
        panic!("this should never be called")
    }

    #[inline]
    fn shared_context(&self) -> &SharedStyleContext<'_> {
        &self.context
    }
}

#[test]
fn assert_size_of_equals() {
    // use std::mem;

    // fn assert_layout<E>() {
    //     assert_eq!(
    //         mem::size_of::<SharingCache<E>>(),
    //         mem::size_of::<TypelessSharingCache>()
    //     );
    //     assert_eq!(
    //         mem::align_of::<SharingCache<E>>(),
    //         mem::align_of::<TypelessSharingCache>()
    //     );
    // }

    // let size = mem::size_of::<StyleSharingCandidate<DomNode>>();
    // dbg!(size);
}

#[test]
fn parse_inline() {
    // let attrs = style::attr::AttrValue::from_serialized_tokenlist(
    //     r#"visibility: hidden; left: 1306.5px; top: 50px; display: none;"#.to_string(),
    // );

    // let val = CSSInlineStyleDeclaration();
}
