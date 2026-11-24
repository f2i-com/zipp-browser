//! Abstractions allow embedders to handle link clicks and form submissions

use http::{HeaderMap, Method};
use url::Url;

use super::net::{Body, Request};

/// An abstraction to allow embedders to hook into "navigation events" such as clicking a link
/// or submitting a form.
pub trait NavigationProvider: Send + Sync + 'static {
    fn navigate_to(&self, options: NavigationOptions);
}

pub struct DummyNavigationProvider;

impl NavigationProvider for DummyNavigationProvider {
    fn navigate_to(&self, _options: NavigationOptions) {
        // Default impl: do nothing
    }
}

#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct NavigationOptions {
    /// The URL to navigate to
    pub url: Url,

    pub content_type: String,

    /// Source document for the navigation
    pub source_document: usize,

    pub method: Method,

    pub document_resource: Body,

    /// Whether the navigation should open in a new tab (e.g. `target="_blank"`,
    /// middle-click, or Ctrl/Cmd+click).
    pub new_tab: bool,

    /// When `new_tab` is true, whether the new tab should open in the background
    /// without switching focus away from the active tab.
    pub background: bool,
}

impl NavigationOptions {
    pub fn new(url: Url, content_type: String, source_document: usize) -> Self {
        Self {
            url,
            content_type,
            source_document,
            method: Method::GET,
            document_resource: Body::Empty,
            new_tab: false,
            background: false,
        }
    }
    pub fn set_document_resource(mut self, document_resource: Body) -> Self {
        self.document_resource = document_resource;
        self
    }

    pub fn set_method(mut self, method: Method) -> Self {
        self.method = method;
        self
    }

    pub fn set_new_tab(mut self, new_tab: bool) -> Self {
        self.new_tab = new_tab;
        self
    }

    pub fn set_background(mut self, background: bool) -> Self {
        self.background = background;
        self
    }

    pub fn into_request(self) -> Request {
        Request {
            url: self.url,
            method: self.method,
            content_type: self.content_type,
            headers: HeaderMap::new(),
            body: self.document_resource,
            signal: None,
        }
    }
}
