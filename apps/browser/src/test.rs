use std::sync::Arc;
use zipp_dom::{DocumentConfig, FontContext};
use zipp_dom::html_parser::{HtmlDocument, HtmlProvider};
use zipp_dom::traits_src::navigation::{NavigationOptions, NavigationProvider};
use zipp_shell::{ZippApp, ShellProxy, WindowConfig, create_default_event_loop};
use zipp_renderer::tinyskia::TinySkiaWindowRenderer;
use linebender_resource_handle::Blob;

struct TestNavProvider;
impl NavigationProvider for TestNavProvider {
    fn navigate_to(&self, options: NavigationOptions) {
        eprintln!("[TEST-NAV] Clicked: {}", options.url);
    }
}

fn main() {
    eprintln!("[TEST] Loading fonts manually...");

    // Create font context and manually register Arial
    let mut font_ctx = FontContext::default();

    // Load Arial font from disk
    let font_path = r"C:\Windows\Fonts\arial.ttf";
    if let Ok(font_data) = std::fs::read(font_path) {
        eprintln!("[FONT] Loaded {} bytes from {}", font_data.len(), font_path);
        let blob = Blob::new(Arc::new(font_data) as _);
        font_ctx.collection.register_fonts(blob, None);
        eprintln!("[FONT] Registered Arial");
    }

    // Also load bold
    if let Ok(font_data) = std::fs::read(r"C:\Windows\Fonts\arialbd.ttf") {
        let blob = Blob::new(Arc::new(font_data) as _);
        font_ctx.collection.register_fonts(blob, None);
        eprintln!("[FONT] Registered Arial Bold");
    }

    // Register bullet font
    font_ctx.collection.register_fonts(
        Blob::new(Arc::new(zipp_dom::BULLET_FONT) as _), None
    );

    // Use inline HTML with links (this WORKS - confirmed earlier)
    let _url = "https://example.com".to_string();
    let html = r#"<!DOCTYPE html><html><head><title>Link Test</title></head>
<body style="font-family:Arial;padding:20px">
<h1>Link Test</h1>
<p><a href="https://example.com">Click to go to example.com</a></p>
<p><a href="https://news.ycombinator.com">Click for HN</a></p>
</body></html>"#.to_string();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let _guard = rt.enter();

    let event_loop = create_default_event_loop();
    let (proxy, receiver) = ShellProxy::new(event_loop.create_proxy());
    let net_provider = Arc::new(zipp_shell::net::Provider::new(Some(Arc::new(proxy.clone()))));
    let nav_provider = Arc::new(TestNavProvider);

    let config = DocumentConfig {
        font_ctx: Some(font_ctx),
        // base_url: Some(url.clone()),  // DISABLED: may affect click handling
        net_provider: Some(net_provider as _),
        navigation_provider: Some(nav_provider as _),
        html_parser_provider: Some(Arc::new(HtmlProvider) as _),
        ..Default::default()
    };

    let mut doc = HtmlDocument::from_html(&html, config);

    // Wait for images to load
    use zipp_dom::Document;
    eprintln!("[TEST] Waiting for images...");
    rt.block_on(async {
        for i in 0..200 {
            let pending = doc.poll(None);
            if !pending && i > 10 { break; }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    });
    eprintln!("[TEST] Images loaded");

    let mut app = ZippApp::new(proxy, receiver);
    let renderer = TinySkiaWindowRenderer::new();
    let window = WindowConfig::new(Box::new(doc) as _, renderer);
    app.add_window(window);

    eprintln!("[TEST] Window opening...");
    event_loop.run_app(app).unwrap();
}
