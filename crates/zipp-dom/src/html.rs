//! HTML parser provider trait and dummy implementation.
//!
//! This module defines the [`HtmlParserProvider`] trait which abstracts over HTML parsing
//! implementations, allowing the DOM crate to remain decoupled from any specific HTML parser.
//! A [`DummyHtmlParserProvider`] is provided as a no-op fallback.

use crate::DocumentMutator;

/// Trait for providing HTML parsing capabilities to the DOM.
///
/// Implementors of this trait can parse an HTML string and insert the resulting nodes
/// into the document tree under a given element. This is used to support operations
/// like `innerHTML` assignment from JavaScript.
pub trait HtmlParserProvider {
    /// Parse the given HTML string and insert the resulting nodes as children
    /// of the element identified by `element_id`.
    ///
    /// # Arguments
    /// * `mutr` - A mutable reference to the document mutator for tree manipulation.
    /// * `element_id` - The slab key of the target element node.
    /// * `html` - The raw HTML string to parse.
    fn parse_inner_html<'m, 'doc>(
        &self,
        mutr: &'m mut DocumentMutator<'doc>,
        element_id: usize,
        html: &str,
    );
}

/// A no-op HTML parser provider.
///
/// This is used as the default when no real HTML parser is configured.
/// All calls to [`parse_inner_html`](HtmlParserProvider::parse_inner_html) are silently ignored.
pub struct DummyHtmlParserProvider;
impl HtmlParserProvider for DummyHtmlParserProvider {
    fn parse_inner_html<'m, 'doc>(
        &self,
        mutr: &'m mut DocumentMutator<'doc>,
        element_id: usize,
        html: &str,
    ) {
        let _ = mutr;
        let _ = element_id;
        let _ = html;
        // Do nothing for now
        //
        // TODO: do something:
        // - Print warning?
        // - Parse HTML as plain text?
    }
}
