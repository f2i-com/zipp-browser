//! Configuration for constructing a [`BaseDocument`](crate::BaseDocument).
//!
//! The [`DocumentConfig`] struct collects all optional dependencies and settings needed
//! to initialize a new document, including network, navigation, shell providers, fonts,
//! and user-agent stylesheets.

use crate::HtmlParserProvider;
use crate::traits_src::{
    navigation::NavigationProvider,
    net::NetProvider,
    shell::{ShellProvider, Viewport},
};
use parley::FontContext;
use std::sync::Arc;

/// Options used when constructing a [`BaseDocument`](crate::BaseDocument).
///
/// All fields are optional. Sensible defaults (dummy/no-op providers) are used
/// when a field is left as `None`.
#[derive(Default)]
pub struct DocumentConfig {
    /// The initial viewport dimensions and scale.
    pub viewport: Option<Viewport>,

    /// The base URL against which relative URLs are resolved (e.g. for images, stylesheets).
    pub base_url: Option<String>,

    /// User-agent stylesheets to apply instead of the built-in default.
    /// When `None`, the engine's bundled UA stylesheet is used.
    pub ua_stylesheets: Option<Vec<String>>,

    /// Network provider used to fetch remote resources (images, fonts, stylesheets).
    pub net_provider: Option<Arc<dyn NetProvider>>,

    /// Navigation provider that handles link clicks and form submissions.
    pub navigation_provider: Option<Arc<dyn NavigationProvider>>,

    /// Shell provider for redraw requests, clipboard access, cursor changes, etc.
    pub shell_provider: Option<Arc<dyn ShellProvider>>,

    /// HTML parser provider used to implement `setInnerHTML` and similar operations.
    pub html_parser_provider: Option<Arc<dyn HtmlParserProvider>>,

    /// A pre-configured Parley [`FontContext`] for text shaping and font resolution.
    /// When `None`, a new context is created with system fonts enabled.
    pub font_ctx: Option<FontContext>,
}
