//! The core DOM abstraction in zipp-dom
//!
//! This crate implements a flexible headless DOM ([`BaseDocument`]), which is designed to be embedded in and "driven" by external code. Most users will want
//! to use a wrapper:
//!
//!  - [`HtmlDocument`] from the html crate.
//!    Allows you to parse HTML (or XHTML) into a [`BaseDocument`], and can be combined with a markdown-to-html converter like [comrak](https://docs.rs/comrak)
//!    or [pulldown-cmark](https://docs.rs/pulldown-cmark) to render/process markdown.
//!  - [`DioxusDocument`] from the dioxus-native crate.
//!    Combines a [`BaseDocument`] with a Dioxus `VirtualDom` to enable dynamic rendering and event handling.
//!
//! It includes: A DOM tree representation, CSS parsing and resolution, layout and event handling. Additional functionality is available in
//! separate crates, including html parsing, networking, rendering, and windowing.
//!
//! Most of the functionality in this crate is provided through the [`BaseDocument`] struct.
//!
//! `zipp-dom` has a native Rust API that is designed for higher-level abstractions to be built on top (although it can also be used directly).
//!
//! The goal behind this crate is that any implementor can interact with the DOM and render it out using any renderer
//! they want.

#![allow(clippy::collapsible_if)]

// TODO: Document features
// ## Feature flags
//  - `default`: Enables the features listed below.
//  - `tracing`: Enables tracing support.

// -----------------------------------------------
// Static assets
// -----------------------------------------------

/// The default user-agent CSS stylesheet applied to all documents.
pub const DEFAULT_CSS: &str = include_str!("../assets/default.css");

/// A fallback bullet font bundled with the engine for list markers.
pub const BULLET_FONT: &[u8] = include_bytes!("../assets/moz-bullet-font.otf");

pub mod fonts;
pub use fonts::register_embedded_fonts;

// -----------------------------------------------
// Incremental layout feature flags
// -----------------------------------------------

const INCREMENTAL: bool = cfg!(feature = "incremental");
const NON_INCREMENTAL: bool = !INCREMENTAL;

// -----------------------------------------------
// Module declarations
// -----------------------------------------------

/// The DOM implementation.
///
/// This is the primary entry point for this crate.
mod document;

/// The nodes themselves, and their data.
pub mod node;

mod config;
mod debug;
mod events;
mod font_metrics;
mod form;
mod html;
/// Integration of taffy and the DOM.
mod layout;
mod media;
mod mutator;
mod query_selector;
mod resolve;
mod selection;
/// Implementations that interact with servo's style engine.
mod stylo;
mod stylo_to_cursor_icon;
mod stylo_to_kurbo;
mod stylo_to_parley;
mod traversal;

mod url;

// -----------------------------------------------
// Public module re-exports
// -----------------------------------------------

pub use stylo_to_kurbo::{has_2d_transform, resolve_2d_transform};

pub mod net;
pub mod util;

pub mod traits_src;
pub mod html_parser;
pub mod stylo_bridge;

#[cfg(feature = "accessibility")]
mod accessibility;

// -----------------------------------------------
// Public API re-exports
// -----------------------------------------------

pub use config::DocumentConfig;
pub use cursor_icon::CursorIcon;
pub use document::{BaseDocument, DocGuard, DocGuardMut, Document, PlainDocument};
pub use markup5ever::{
    LocalName, Namespace, NamespaceStaticSet, Prefix, PrefixStaticSet, QualName, local_name,
    namespace_prefix, namespace_url, ns,
};
pub use mutator::DocumentMutator;
pub use node::{Attribute, ElementData, Node, NodeData, TextNodeData};
pub use parley::FontContext;
pub use style::Atom;
pub use style::invalidation::element::restyle_hints::RestyleHint;
pub type SelectorList = selectors::SelectorList<style::selector_parser::SelectorImpl>;
pub use events::{DomPatch, EventDriver, EventHandler, NoopEventHandler};
pub use html::{DummyHtmlParserProvider, HtmlParserProvider};
pub use util::Point;

// Re-exports from traits_src
pub use traits_src::devtools;
pub use traits_src::events as traits_events;
pub use traits_src::navigation;
pub use traits_src::net as traits_net;
pub use traits_src::shell;
