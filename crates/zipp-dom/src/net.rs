//! Network resource loading and handling.
//!
//! This module provides the infrastructure for fetching and processing remote resources
//! (stylesheets, images, fonts) needed by the document. Each resource type has a dedicated
//! [`NetHandler`](crate::traits_src::net::NetHandler) implementation that parses the raw
//! bytes and sends the result back to the document via an MPSC channel.
//!
//! # Resource types
//! - **CSS** -- parsed into a Servo `DocumentStyleSheet` via [`StylesheetHandler`].
//! - **Images** -- decoded via the `image` crate (or `usvg` for SVGs) via [`ImageHandler`].
//! - **Fonts** -- optionally decompressed from WOFF/WOFF2 via [`FontFaceHandler`].

use selectors::context::QuirksMode;
use std::sync::atomic::Ordering;
use std::{
    io::Cursor,
    sync::{Arc, atomic::AtomicUsize, mpsc::Sender},
};
use style::{
    font_face::{FontFaceSourceFormat, FontFaceSourceFormatKeyword, Source},
    media_queries::MediaList,
    servo_arc::Arc as ServoArc,
    shared_lock::SharedRwLock,
    shared_lock::{Locked, SharedRwLockReadGuard},
    stylesheets::{
        AllowImportRules, CssRule, DocumentStyleSheet, ImportRule, Origin, Stylesheet,
        StylesheetInDocument, StylesheetLoader as ServoStylesheetLoader, UrlExtraData,
        import_rule::{ImportLayer, ImportSheet, ImportSupportsCondition},
    },
    values::{CssUrl, SourceLocation},
};

use crate::traits_src::net::{Bytes, NetHandler, NetProvider, Request};
use crate::traits_src::shell::ShellProvider;

use url::Url;

use crate::{document::DocumentEvent, util::ImageType};

// ---------------------------------------------------------------------------
// Resource enum
// ---------------------------------------------------------------------------

/// A loaded resource, ready to be applied to the document.
#[derive(Clone, Debug)]
pub enum Resource {
    /// A decoded raster image with its type, dimensions, and raw RGBA8 pixel data.
    Image(ImageType, u32, u32, Arc<Vec<u8>>),
    /// A parsed SVG tree (requires the `svg` feature).
    #[cfg(feature = "svg")]
    Svg(ImageType, Arc<usvg::Tree>),
    /// A parsed CSS stylesheet.
    Css(DocumentStyleSheet),
    /// Raw font bytes (possibly decompressed from WOFF/WOFF2).
    Font(Bytes),
    /// Placeholder for requests that complete but produce no usable resource
    /// (e.g. nested `@import` rules whose result is applied in-place).
    None,
}

// ---------------------------------------------------------------------------
// ResourceHandler -- generic handler scaffold
// ---------------------------------------------------------------------------

/// Generic scaffolding for resource network handlers.
///
/// `T` carries handler-specific state (e.g. [`StylesheetHandler`], [`ImageHandler`]).
/// The handler sends its result back to the document via the `tx` channel.
pub(crate) struct ResourceHandler<T: Send + Sync + 'static> {
    doc_id: usize,
    request_id: usize,
    node_id: Option<usize>,
    tx: Sender<DocumentEvent>,
    shell_provider: Arc<dyn ShellProvider>,
    data: T,
}

impl<T: Send + Sync + 'static> ResourceHandler<T> {
    /// Create a new `ResourceHandler`, auto-assigning a unique request ID.
    pub(crate) fn new(
        tx: Sender<DocumentEvent>,
        doc_id: usize,
        node_id: Option<usize>,
        shell_provider: Arc<dyn ShellProvider>,
        data: T,
    ) -> Self {
        static REQUEST_ID_COUNTER: AtomicUsize = AtomicUsize::new(0);
        Self {
            request_id: REQUEST_ID_COUNTER.fetch_add(1, Ordering::Relaxed),
            doc_id,
            node_id,
            tx,
            shell_provider,
            data,
        }
    }

    /// Create a new `ResourceHandler` and box-erase it into `Box<dyn NetHandler>`.
    pub(crate) fn boxed(
        tx: Sender<DocumentEvent>,
        doc_id: usize,
        node_id: Option<usize>,
        shell_provider: Arc<dyn ShellProvider>,
        data: T,
    ) -> Box<dyn NetHandler>
    where
        ResourceHandler<T>: NetHandler,
    {
        Box::new(Self::new(tx, doc_id, node_id, shell_provider, data)) as _
    }

    /// The unique ID assigned to this request.
    pub(crate) fn request_id(&self) -> usize {
        self.request_id
    }

    /// Send the resource load result back to the document and request a redraw.
    fn respond(&self, resolved_url: String, result: Result<Resource, String>) {
        let response = ResourceLoadResponse {
            request_id: self.request_id,
            node_id: self.node_id,
            resolved_url: Some(resolved_url),
            result,
        };
        let _ = self.tx.send(DocumentEvent::ResourceLoad(response));
        self.shell_provider.request_redraw();
    }
}

// ---------------------------------------------------------------------------
// ResourceLoadResponse
// ---------------------------------------------------------------------------

/// The result of a resource fetch, delivered to the document via the event channel.
#[allow(unused)]
pub struct ResourceLoadResponse {
    /// The unique request ID (matches [`ResourceHandler::request_id`]).
    pub request_id: usize,
    /// The node that initiated the request (if any).
    pub node_id: Option<usize>,
    /// The final URL after any redirects.
    pub resolved_url: Option<String>,
    /// The loaded resource, or an error message.
    pub result: Result<Resource, String>,
}

// ---------------------------------------------------------------------------
// Stylesheet handling
// ---------------------------------------------------------------------------

/// Handler-specific data for fetching a top-level `<link>` stylesheet.
pub struct StylesheetHandler {
    /// The URL this stylesheet was fetched from (used as the base for relative URLs within the CSS).
    pub source_url: Url,
    /// The Stylo shared lock used for stylesheet parsing.
    pub guard: SharedRwLock,
    /// The network provider, needed for fetching nested `@import` rules.
    pub net_provider: Arc<dyn NetProvider>,
}

impl NetHandler for ResourceHandler<StylesheetHandler> {
    fn bytes(self: Box<Self>, resolved_url: String, bytes: Bytes) {
        let Ok(css) = std::str::from_utf8(&bytes) else {
            return self.respond(resolved_url, Err(String::from("Invalid UTF8")));
        };

        let sheet = Stylesheet::from_str(
            css,
            self.data.source_url.clone().into(),
            Origin::Author,
            ServoArc::new(self.data.guard.wrap(MediaList::empty())),
            self.data.guard.clone(),
            Some(&StylesheetLoader {
                tx: self.tx.clone(),
                doc_id: self.doc_id,
                net_provider: self.data.net_provider.clone(),
                shell_provider: self.shell_provider.clone(),
            }),
            None, // error_reporter
            QuirksMode::NoQuirks,
            AllowImportRules::Yes,
        );

        self.respond(
            resolved_url,
            Ok(Resource::Css(DocumentStyleSheet(ServoArc::new(sheet)))),
        );
    }
}

// ---------------------------------------------------------------------------
// StylesheetLoader -- handles @import rules
// ---------------------------------------------------------------------------

/// Loader passed to Servo's CSS parser to handle `@import` rules.
///
/// When the parser encounters an `@import`, it calls [`request_stylesheet`] which
/// fires off a network fetch for the imported stylesheet.
#[derive(Clone)]
pub(crate) struct StylesheetLoader {
    pub(crate) tx: Sender<DocumentEvent>,
    pub(crate) doc_id: usize,
    pub(crate) net_provider: Arc<dyn NetProvider>,
    pub(crate) shell_provider: Arc<dyn ShellProvider>,
}
impl ServoStylesheetLoader for StylesheetLoader {
    fn request_stylesheet(
        &self,
        url: CssUrl,
        location: SourceLocation,
        lock: &SharedRwLock,
        media: ServoArc<Locked<MediaList>>,
        supports: Option<ImportSupportsCondition>,
        layer: ImportLayer,
    ) -> ServoArc<Locked<ImportRule>> {
        if !supports.as_ref().is_none_or(|s| s.enabled) {
            return ServoArc::new(lock.wrap(ImportRule {
                url,
                stylesheet: ImportSheet::new_refused(),
                supports,
                layer,
                source_location: location,
            }));
        }

        let import = ImportRule {
            url,
            stylesheet: ImportSheet::new_pending(),
            supports,
            layer,
            source_location: location,
        };

        let url = import.url.url().unwrap().clone();
        let import = ServoArc::new(lock.wrap(import));
        self.net_provider.fetch(
            self.doc_id,
            Request::get(url.as_ref().clone()),
            ResourceHandler::boxed(
                self.tx.clone(),
                self.doc_id,
                None, // node_id
                self.shell_provider.clone(),
                NestedStylesheetHandler {
                    url: url.clone(),
                    loader: self.clone(),
                    lock: lock.clone(),
                    media,
                    import_rule: import.clone(),
                    net_provider: self.net_provider.clone(),
                },
            ),
        );

        import
    }
}

// ---------------------------------------------------------------------------
// Nested stylesheet handling (@import)
// ---------------------------------------------------------------------------

/// Handler-specific data for a stylesheet loaded via `@import`.
struct NestedStylesheetHandler {
    loader: StylesheetLoader,
    lock: SharedRwLock,
    url: ServoArc<Url>,
    media: ServoArc<Locked<MediaList>>,
    import_rule: ServoArc<Locked<ImportRule>>,
    net_provider: Arc<dyn NetProvider>,
}

impl NetHandler for ResourceHandler<NestedStylesheetHandler> {
    fn bytes(self: Box<Self>, resolved_url: String, bytes: Bytes) {
        let Ok(css) = std::str::from_utf8(&bytes) else {
            return self.respond(resolved_url, Err(String::from("Invalid UTF8")));
        };

        let sheet = ServoArc::new(Stylesheet::from_str(
            css,
            UrlExtraData(self.data.url.clone()),
            Origin::Author,
            self.data.media.clone(),
            self.data.lock.clone(),
            Some(&self.data.loader),
            None, // error_reporter
            QuirksMode::NoQuirks,
            AllowImportRules::Yes,
        ));

        // Fetch @font-face fonts declared in the imported stylesheet
        fetch_font_face(
            self.tx.clone(),
            self.doc_id,
            self.node_id,
            &sheet,
            &self.data.net_provider,
            &self.shell_provider,
            &self.data.lock.read(),
        );

        let mut guard = self.data.lock.write();
        self.data.import_rule.write_with(&mut guard).stylesheet = ImportSheet::Sheet(sheet);
        drop(guard);

        self.respond(resolved_url, Ok(Resource::None))
    }
}

// ---------------------------------------------------------------------------
// Font handling
// ---------------------------------------------------------------------------

/// Handler-specific data for fetching a `@font-face` font.
///
/// Stores the font format hint (if known) so the handler can attempt WOFF
/// decompression when appropriate.
struct FontFaceHandler(FontFaceSourceFormatKeyword);

impl NetHandler for ResourceHandler<FontFaceHandler> {
    fn bytes(mut self: Box<Self>, resolved_url: String, bytes: Bytes) {
        let result = self.data.parse(bytes);
        self.respond(resolved_url, result)
    }
}

impl FontFaceHandler {
    /// Parse (and optionally decompress) raw font bytes into a [`Resource::Font`].
    fn parse(&mut self, bytes: Bytes) -> Result<Resource, String> {
        // Auto-detect format from magic bytes if no format hint was provided
        if self.0 == FontFaceSourceFormatKeyword::None && bytes.len() >= 4 {
            self.0 = match &bytes.as_ref()[0..4] {
                // WOFF (v1) files begin with 0x774F4646 ('wOFF' in ascii)
                // See: <https://w3c.github.io/woff/woff1/spec/Overview.html#WOFFHeader>
                #[cfg(any(feature = "woff-c", feature = "woff-rust"))]
                b"wOFF" => FontFaceSourceFormatKeyword::Woff,
                // WOFF2 files begin with 0x774F4632 ('wOF2' in ascii)
                // See: <https://w3c.github.io/woff/woff2/#woff20Header>
                #[cfg(any(feature = "woff-c", feature = "woff-rust"))]
                b"wOF2" => FontFaceSourceFormatKeyword::Woff2,
                // Opentype fonts with CFF data begin with 0x4F54544F ('OTTO' in ascii)
                // See: <https://learn.microsoft.com/en-us/typography/opentype/spec/otff#organization-of-an-opentype-font>
                b"OTTO" => FontFaceSourceFormatKeyword::Opentype,
                // Opentype fonts with truetype outlines begin with 0x00010000
                // See: <https://learn.microsoft.com/en-us/typography/opentype/spec/otff#organization-of-an-opentype-font>
                &[0x00, 0x01, 0x00, 0x00] => FontFaceSourceFormatKeyword::Truetype,
                // Truetype fonts begin with 0x74727565 ('true' in ascii)
                // See: <https://developer.apple.com/fonts/TrueType-Reference-Manual/RM06/Chap6.html#ScalerTypeNote>
                b"true" => FontFaceSourceFormatKeyword::Truetype,
                _ => FontFaceSourceFormatKeyword::None,
            }
        }

        // Satisfy rustc's mutability linting with woff feature both enabled/disabled
        #[cfg(any(feature = "woff-c", feature = "woff-rust"))]
        let mut bytes = bytes;

        match self.0 {
            #[cfg(any(feature = "woff-c", feature = "woff-rust"))]
            FontFaceSourceFormatKeyword::Woff => {
                #[cfg(feature = "tracing")]
                tracing::info!("Decompressing woff1 font");

                #[cfg(feature = "woff-c")]
                let decompressed = woff::version1::decompress(&bytes);

                #[cfg(feature = "woff-rust")]
                let decompressed = wuff::decompress_woff1(&bytes).ok();

                if let Some(decompressed) = decompressed {
                    bytes = Bytes::from(decompressed);
                } else {
                    #[cfg(feature = "tracing")]
                    tracing::warn!("Failed to decompress woff1 font");
                }
            }
            #[cfg(any(feature = "woff-c", feature = "woff-rust"))]
            FontFaceSourceFormatKeyword::Woff2 => {
                #[cfg(feature = "tracing")]
                tracing::info!("Decompressing woff2 font");

                #[cfg(feature = "woff-c")]
                let decompressed = woff::version2::decompress(&bytes);

                #[cfg(feature = "woff-rust")]
                let decompressed = wuff::decompress_woff2(&bytes).ok();

                if let Some(decompressed) = decompressed {
                    bytes = Bytes::from(decompressed);
                } else {
                    #[cfg(feature = "tracing")]
                    tracing::warn!("Failed to decompress woff2 font");
                }
            }
            FontFaceSourceFormatKeyword::None => {
                // Unrecognized format -- nothing to return
                return Ok(Resource::None);
            }
            _ => {}
        }

        Ok(Resource::Font(bytes))
    }
}

// ---------------------------------------------------------------------------
// Font face fetching from @font-face rules
// ---------------------------------------------------------------------------

/// Scans a stylesheet for `@font-face` rules and fetches the fonts they reference.
///
/// For each `@font-face` rule, the first source with a supported format is selected
/// and a network fetch is initiated.
pub(crate) fn fetch_font_face(
    tx: Sender<DocumentEvent>,
    doc_id: usize,
    node_id: Option<usize>,
    sheet: &Stylesheet,
    network_provider: &Arc<dyn NetProvider>,
    shell_provider: &Arc<dyn ShellProvider>,
    read_guard: &SharedRwLockReadGuard,
) {
    sheet
        .contents(read_guard)
        .rules(read_guard)
        .iter()
        .filter_map(|rule| match rule {
            CssRule::FontFace(font_face) => font_face.read_with(read_guard).sources.as_ref(),
            _ => None,
        })
        .for_each(|source_list| {
            // Find the first font source in the source list that specifies a font of a type
            // that we support.
            let preferred_source = source_list
                .0
                .iter()
                .filter_map(|source| match source {
                    Source::Url(url_source) => Some(url_source),
                    // TODO: support local fonts in @font-face
                    Source::Local(_) => None,
                })
                .find_map(|url_source| {
                    let mut format = match &url_source.format_hint {
                        Some(FontFaceSourceFormat::Keyword(fmt)) => *fmt,
                        Some(FontFaceSourceFormat::String(str)) => match str.as_str() {
                            "woff2" => FontFaceSourceFormatKeyword::Woff2,
                            "ttf" => FontFaceSourceFormatKeyword::Truetype,
                            "otf" => FontFaceSourceFormatKeyword::Opentype,
                            _ => FontFaceSourceFormatKeyword::None,
                        },
                        _ => FontFaceSourceFormatKeyword::None,
                    };
                    if format == FontFaceSourceFormatKeyword::None {
                        let (_, end) = url_source.url.as_str().rsplit_once('.')?;
                        format = match end {
                            "woff2" => FontFaceSourceFormatKeyword::Woff2,
                            "woff" => FontFaceSourceFormatKeyword::Woff,
                            "ttf" => FontFaceSourceFormatKeyword::Truetype,
                            "otf" => FontFaceSourceFormatKeyword::Opentype,
                            "svg" => FontFaceSourceFormatKeyword::Svg,
                            "eot" => FontFaceSourceFormatKeyword::EmbeddedOpentype,
                            _ => FontFaceSourceFormatKeyword::None,
                        }
                    }

                    if matches!(
                        format,
                        FontFaceSourceFormatKeyword::Svg
                            | FontFaceSourceFormatKeyword::EmbeddedOpentype
                    ) {
                        #[cfg(feature = "tracing")]
                        tracing::warn!("Skipping unsupported font of type {:?}", format);
                        return None;
                    }

                    #[cfg(all(not(feature = "woff-c"), not(feature = "woff-rust")))]
                    if matches!(
                        format,
                        FontFaceSourceFormatKeyword::Woff | FontFaceSourceFormatKeyword::Woff2
                    ) {
                        #[cfg(feature = "tracing")]
                        tracing::warn!("Skipping unsupported font of type {:?}", format);
                        return None;
                    }

                    let url = url_source.url.url().unwrap().as_ref().clone();
                    Some((url, format))
                });

            if let Some((url, format)) = preferred_source {
                network_provider.fetch(
                    doc_id,
                    Request::get(url),
                    ResourceHandler::boxed(
                        tx.clone(),
                        doc_id,
                        node_id,
                        shell_provider.clone(),
                        FontFaceHandler(format),
                    ),
                );
            }
        })
}

// ---------------------------------------------------------------------------
// Image handling
// ---------------------------------------------------------------------------

/// Handler-specific data for fetching an image (raster or SVG).
pub struct ImageHandler {
    kind: ImageType,
}

impl ImageHandler {
    /// Create a new image handler for the given image type.
    pub fn new(kind: ImageType) -> Self {
        Self { kind }
    }
}

impl NetHandler for ResourceHandler<ImageHandler> {
    fn bytes(self: Box<Self>, resolved_url: String, bytes: Bytes) {
        let result = self.data.parse(bytes);
        self.respond(resolved_url, result)
    }
}

impl ImageHandler {
    /// Attempt to decode the raw bytes as a raster image, falling back to SVG
    /// if the `svg` feature is enabled.
    fn parse(&self, bytes: Bytes) -> Result<Resource, String> {
        // Try parse as raster image
        if let Ok(image) = image::ImageReader::new(Cursor::new(&bytes))
            .with_guessed_format()
            .expect("IO errors impossible with Cursor")
            .decode()
        {
            let raw_rgba8_data = image.clone().into_rgba8().into_raw();
            return Ok(Resource::Image(
                self.kind,
                image.width(),
                image.height(),
                Arc::new(raw_rgba8_data),
            ));
        };

        #[cfg(feature = "svg")]
        {
            use crate::util::parse_svg;
            if let Ok(tree) = parse_svg(&bytes) {
                return Ok(Resource::Svg(self.kind, Arc::new(tree)));
            }
        }

        Err(String::from("Could not parse image"))
    }
}
