//! Zipp Browser -- Stylo CSS + Taffy Layout + tiny-skia rendering + Zipp JS
//!
//! Pipeline: Fetch -> Zipp JS (React/SPA) -> inline CSS/images -> responsive transform
//!           -> Zipp layout -> tiny-skia paint -> winit window
//!
//! Responsive Tailwind classes (md:*, lg:*) are handled by the `responsive` module which
//! converts them to inline styles. CSS `@media` blocks are unwrapped so all rules apply
//! unconditionally at desktop width.

// -- std --
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// -- external --
use linebender_resource_handle::Blob;

// -- internal (workspace) --
use zipp_vm::engine::{ZippEngine, ScriptState};
use zipp_dom::html_parser::{HtmlDocument, HtmlProvider};
use zipp_dom::traits_src::navigation::{NavigationOptions, NavigationProvider};
use zipp_dom::{DocumentConfig, FontContext};
use zipp_renderer::tinyskia::TinySkiaWindowRenderer;
use zipp_shell::{create_default_event_loop, ZippApp, ShellProxy, WindowConfig};

mod event_bridge;
mod responsive;

// ---------------------------------------------------------------------------
// Global caches
// ---------------------------------------------------------------------------

lazy_static::lazy_static! {
    /// Fetched URL content (JS bundles, CSS, HTML).
    static ref FETCH_CACHE: Mutex<HashMap<String, String>> = Mutex::new(HashMap::new());
    /// Inlined images (URL -> base64 data URI).
    static ref IMAGE_CACHE: Mutex<HashMap<String, String>> = Mutex::new(HashMap::new());
    /// Fully rendered HTML keyed by URL (skip JS re-compilation on revisit).
    static ref RENDERED_CACHE: Mutex<HashMap<String, String>> = Mutex::new(HashMap::new());
    /// Shared blocking HTTP client -- reused across all fetch() calls to
    /// take advantage of connection pooling and avoid per-request TLS setup.
    static ref HTTP_CLIENT: reqwest::blocking::Client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .danger_accept_invalid_certs(true)
        .user_agent(USER_AGENT)
        .build()
        .expect("failed to build HTTP client");
}

/// Customisable User-Agent string. Change this to mimic different browsers.
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Zipp/1.0";

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_default();

    // Quick JS test mode: run test suite through VM and exit
    if arg == "--test-js" {
        run_js_tests();
        return;
    }
    if arg == "--test-vm" {
        run_vm_test();
        return;
    }
    if arg == "--test-compat" {
        run_compat_tests();
        return;
    }

    let url = if arg.is_empty() { "https://lance.name".to_string() } else { arg };
    eprintln!("[BROWSER] Loading {url}");

    // 1. Fetch the page
    let html = fetch(&url)
        .unwrap_or_else(|e| format!("<html><body><h1>Error</h1><p>{e}</p></body></html>"));

    // 2. Run Zipp JS if needed (React/SPA) -- run on main thread with
    //    grown stack so that the ScriptState (which contains Rc and is !Send)
    //    stays alive for later event dispatch.
    let (final_html, event_map_json, js_state) = if html.contains("<script") {
        stacker::maybe_grow(64 * 1024 * 1024, 64 * 1024 * 1024, || {
            match run_zipp_js(&html, &url) {
                Some((rendered, emap, state)) => (rendered, Some(emap), Some(state)),
                None => (html.clone(), None, None),
            }
        })
    } else {
        (html, None, None)
    };

    // 3. Post-process HTML
    // Strip <noscript> blocks (our browser has JS support)
    let final_html = strip_noscript(&final_html);
    // Toggle JS detection classes: no-js → js, client-nojs → client-js
    let final_html = final_html
        .replace("class=\"no-js", "class=\"js")
        .replace("class=\"client-nojs", "class=\"client-js");
    let final_html = inline_images(&final_html, &url);
    let final_html = responsive::apply_desktop_classes(&final_html);
    let final_html = wrap_with_toolbar(&final_html, &url);
    eprintln!("[BROWSER] Ready ({} bytes)", final_html.len());

    // Cache the initial page for instant back-navigation
    RENDERED_CACHE
        .lock()
        .unwrap()
        .insert(url.clone(), final_html.clone());

    // 4. Load fonts manually (system font API doesn't work on RDP)
    let font_ctx = load_fonts();

    // 5. Set up async runtime for network fetches (CSS, images)
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let _guard = rt.enter();

    // 6. Create window and net provider
    let event_loop = create_default_event_loop();
    let (proxy, receiver) = ShellProxy::new(event_loop.create_proxy());
    let net_provider = Arc::new(zipp_shell::net::Provider::new(Some(Arc::new(
        proxy.clone(),
    ))));

    // 7. Create Zipp document with fonts + networking + navigation
    let nav_provider = Arc::new(BrowserNavProvider {
        proxy: proxy.clone(),
    });
    let config = DocumentConfig {
        font_ctx: Some(font_ctx),
        base_url: Some(url.clone()),
        net_provider: Some(net_provider as _),
        navigation_provider: Some(nav_provider as _),
        html_parser_provider: Some(Arc::new(HtmlProvider) as _),
        ..Default::default()
    };
    let mut doc = HtmlDocument::from_html(&final_html, config);

    // Build JS-node-ID -> blitz-dom-node-ID mapping from data-flid attributes
    // and wire the event bridge into the document so that DOM events call
    // actual JS handlers via the live ScriptState.
    if let Some(ref emap_json) = event_map_json {
        let flid_map = event_bridge::build_flid_map(&doc);
        if !flid_map.is_empty() {
            eprintln!("[EVENT-BRIDGE] Mapped {} nodes with data-flid attributes", flid_map.len());
        }
        let bridge = event_bridge::JsEventBridge::new(flid_map, emap_json, js_state);
        eprintln!(
            "[EVENT-BRIDGE] Bridge created with {} handler entries (live JS: {})",
            bridge.handler_count(),
            bridge.has_js_state(),
        );
        if bridge.handler_count() > 0 {
            doc.set_event_handler(Box::new(bridge));
            eprintln!("[EVENT-BRIDGE] Event handler installed on document");
        }
    }

    // Pre-render linked pages in background for instant navigation
    let prerender_url = url.clone();
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || prerender_linked_pages(&final_html, &prerender_url))
        .ok();

    // 8. Open window with tiny-skia renderer
    let mut app = ZippApp::new(proxy, receiver);
    let renderer = TinySkiaWindowRenderer::new();
    let window = WindowConfig::new(Box::new(doc) as _, renderer);
    app.add_window(window);

    eprintln!("[BROWSER] Window opened");
    event_loop.run_app(app).unwrap();
}

// ---------------------------------------------------------------------------
// Pre-rendering
// ---------------------------------------------------------------------------

/// Run the JS test suite through the Zipp VM with DOM shim and report results.
fn run_js_tests() {
    let dom_shim = include_str!("dom_shim.js");
    let shim = dom_shim
        .replace("__PAGE_URL__", "http://localhost/test")
        .replace("__VIEWPORT_W__", "1280")
        .replace("__VIEWPORT_H__", "800")
        .replace("__BODY_HTML__", "<div id=\\'root\\'></div>");

    let tests = include_str!("js_tests.js");
    let mut full = shim;
    full.push_str(tests);

    eprintln!("[TEST] Compiling {} bytes...", full.len());
    let engine = ZippEngine::default();
    match engine.compile_script(&full) {
        Ok(mut state) => {
            state.set_execution_limits(Some(50_000_000), Some(10_000));
            eprintln!("[TEST] Executing...");
            let _ = state.run_init();
            // Also try eval to get test results
            if let Ok(zipp_vm::object::Object::String(s)) = state.eval_in_context("_p + '/' + (_p+_f) + ' passed'") {
                eprintln!("[TEST] {s}");
            }
            for i in 0..10 {
                if let Ok(zipp_vm::object::Object::String(s)) = state.eval_in_context(&format!("_e[{i}] || ''")) {
                    if !s.is_empty() { eprintln!("[TEST]   {s}"); }
                }
            }
        }
        Err(e) => eprintln!("Compile error: {e:?}"),
    }
}

/// Run minimal VM test without DOM shim to isolate class method issues.
fn run_compat_tests() {
    let dom_shim = include_str!("dom_shim.js");
    let shim = dom_shim
        .replace("__PAGE_URL__", "http://localhost/test")
        .replace("__VIEWPORT_W__", "1280")
        .replace("__VIEWPORT_H__", "800")
        .replace("__BODY_HTML__", "<div id=\\'root\\'></div>");
    let tests = include_str!("js_compat_tests.js");
    let mut full = shim;
    full.push_str(tests);
    eprintln!("[COMPAT] Compiling {} bytes...", full.len());
    let engine = ZippEngine::default();
    match engine.compile_script(&full) {
        Ok(mut state) => {
            state.set_execution_limits(Some(50_000_000), Some(10_000));
            let _ = state.run_init();
            if let Ok(zipp_vm::object::Object::String(s)) = state.eval_in_context("_p + '/' + (_p+_f) + ' passed'") {
                eprintln!("[COMPAT] {s}");
            }
            for i in 0..20 {
                if let Ok(zipp_vm::object::Object::String(s)) = state.eval_in_context(&format!("_e[{i}] || ''")) {
                    if !s.is_empty() { eprintln!("[COMPAT]   {s}"); }
                }
            }
        }
        Err(e) => eprintln!("[COMPAT] Compile error: {e:?}"),
    }
}

fn run_vm_test() {
    let js = r#"
class Box {
    constructor() { this.items = []; }
    add(x) { this.items.push(x); }
    has(x) { return this.items.indexOf(x); }
    count() { return this.items.length; }
}
// Test 1: Class method - works because indexOf runs inside method scope
var b1 = new Box();
var i1 = {v:1};
b1.add(i1);
var t1 = "method: count=" + b1.count() + " has=" + b1.has(i1);

// Test 2: Class with MANY constructor properties (like DOM Element)
class BigClass {
    constructor() {
        this.items = [];
        this.a = 0; this.b = 0; this.c = ""; this.d = ""; this.e = null;
        this.f = null; this.g = {}; this.h = false; this.i = ""; this.j = "";
        this.k = 0; this.l = 0; this.m = 0; this.n = 0; this.o = 0;
        this.p = 0; this.q = 0; this.r = 0; this.s = 0; this.t = 0;
    }
}
var b2 = new BigClass();
var i2 = {v:2};
b2.items.push(i2);
var t2 = "big20: len=" + b2.items.length + " indexOf=" + b2.items.indexOf(i2);

// Test 3: Even more properties (40+)
class HugeClass extends BigClass {
    constructor() {
        super();
        this.u = ""; this.v1 = ""; this.w = ""; this.x = ""; this.y = "";
        this.z = ""; this.aa = 0; this.bb = 0; this.cc = ""; this.dd = "";
        this.ee = null; this.ff = null; this.gg = {}; this.hh = false;
        this.ii = ""; this.jj = ""; this.kk = 0; this.ll = 0; this.mm = 0;
        this.nn = 0; this.extra_items = [];
    }
}
var b3 = new HugeClass();
var i3 = {v:3};
b3.items.push(i3);
var t3 = "huge40: len=" + b3.items.length + " indexOf=" + b3.items.indexOf(i3);

// Test 4: Push to extra_items on huge class
var b4 = new HugeClass();
var i4 = {v:4};
b4.extra_items.push(i4);
var t4 = "huge_extra: len=" + b4.extra_items.length + " indexOf=" + b4.extra_items.indexOf(i4);

var result = t1 + " | " + t2 + " | " + t3 + " | " + t4;
"#;
    // Load with DOM shim to test in the same environment as real pages
    let dom_shim = include_str!("dom_shim.js");
    let shim = dom_shim
        .replace("__PAGE_URL__", "http://localhost/test")
        .replace("__VIEWPORT_W__", "1280")
        .replace("__VIEWPORT_H__", "800")
        .replace("__BODY_HTML__", "<div id=\\'root\\'></div>");
    let full = format!("{}\n{}", shim, js);
    eprintln!("[VM-TEST] Running with DOM shim ({} bytes)...", full.len());
    let engine = ZippEngine::default();
    match engine.compile_script(&full) {
        Ok(mut state) => {
            state.set_execution_limits(Some(10_000_000), Some(5_000));
            let _ = state.run_init();
            // Read results
            if let Ok(zipp_vm::object::Object::String(s)) = state.eval_in_context("result") {
                eprintln!("[VM-TEST] {s}");
            } else {
                eprintln!("[VM-TEST] Could not read result");
            }
        }
        Err(e) => eprintln!("[VM-TEST] Compile error: {e:?}"),
    }
}

/// Extract same-origin links from rendered HTML and pre-render them into
/// `RENDERED_CACHE` so that navigation is instant on revisit.
fn prerender_linked_pages(html: &str, base_url: &str) {
    let mut links = Vec::new();
    let mut pos = 0;
    while let Some(idx) = html[pos..].find("href=\"") {
        let start = pos + idx + 6;
        if let Some(end) = html[start..].find('"') {
            let href = &html[start..start + end];
            if href.starts_with("http") && !links.contains(&href.to_string()) {
                links.push(href.to_string());
            }
        }
        pos = start + 1;
    }

    let base = url::Url::parse(base_url)
        .ok()
        .map(|u| format!("{}://{}", u.scheme(), u.host_str().unwrap_or("")))
        .unwrap_or_default();

    for link in &links {
        if !link.starts_with(&base) {
            continue;
        }
        if RENDERED_CACHE.lock().unwrap().contains_key(link) {
            continue;
        }
        eprintln!("[PRERENDER] {link}");
        let html = match fetch(link) {
            Ok(h) => h,
            Err(_) => continue,
        };
        let rendered = if html.contains("<script") {
            // Prerender only needs the HTML, discard the ScriptState.
            run_zipp_js(&html, link)
                .map(|(h, _emap, _state)| h)
                .unwrap_or(html)
        } else {
            html
        };
        let rendered = inline_css(&rendered, link);
        let rendered = inline_images(&rendered, link);
        let final_html = wrap_with_toolbar(&rendered, link);
        RENDERED_CACHE
            .lock()
            .unwrap()
            .insert(link.clone(), final_html);
    }
    eprintln!(
        "[PRERENDER] Done -- {} pages cached",
        RENDERED_CACHE.lock().unwrap().len()
    );
}

// ---------------------------------------------------------------------------
// Font loading
// ---------------------------------------------------------------------------

fn load_fonts() -> FontContext {
    let mut font_ctx = FontContext::default();

    let font_files = [
        r"C:\Windows\Fonts\arial.ttf",
        r"C:\Windows\Fonts\arialbd.ttf",
        r"C:\Windows\Fonts\ariali.ttf",
        r"C:\Windows\Fonts\arialbi.ttf",
        r"C:\Windows\Fonts\segoeui.ttf",
        r"C:\Windows\Fonts\segoeuib.ttf",
        r"C:\Windows\Fonts\segoeuil.ttf",
        r"C:\Windows\Fonts\times.ttf",
        r"C:\Windows\Fonts\timesbd.ttf",
        r"C:\Windows\Fonts\cour.ttf",
        r"C:\Windows\Fonts\courbd.ttf",
        r"C:\Windows\Fonts\verdana.ttf",
        r"C:\Windows\Fonts\verdanab.ttf",
        r"C:\Windows\Fonts\tahoma.ttf",
        r"C:\Windows\Fonts\tahomabd.ttf",
        r"C:\Windows\Fonts\calibri.ttf",
        r"C:\Windows\Fonts\calibrib.ttf",
        r"C:\Windows\Fonts\calibrii.ttf",
        r"C:\Windows\Fonts\georgia.ttf",
        r"C:\Windows\Fonts\georgiab.ttf",
        r"C:\Windows\Fonts\trebuc.ttf",
        r"C:\Windows\Fonts\trebucbd.ttf",
        r"C:\Windows\Fonts\impact.ttf",
        r"C:\Windows\Fonts\comic.ttf",
        r"C:\Windows\Fonts\consola.ttf",
        r"C:\Windows\Fonts\consolab.ttf",
        r"C:\Windows\Fonts\pala.ttf",
        r"C:\Windows\Fonts\Roboto-Regular.ttf",
        r"C:\Windows\Fonts\Roboto-Bold.ttf",
        r"C:\Windows\Fonts\Roboto-Light.ttf",
    ];

    let mut loaded = 0;
    for path in &font_files {
        if let Ok(data) = std::fs::read(path) {
            let blob = Blob::new(Arc::new(data) as _);
            font_ctx.collection.register_fonts(blob, None);
            loaded += 1;
        }
    }
    eprintln!("[FONT] Loaded {loaded} fonts");

    font_ctx
        .collection
        .register_fonts(Blob::new(Arc::new(zipp_dom::BULLET_FONT) as _), None);

    font_ctx
}

// ---------------------------------------------------------------------------
// Networking
// ---------------------------------------------------------------------------

fn fetch(url: &str) -> Result<String, String> {
    let is_static = url.contains("/static/") || url.ends_with(".js") || url.ends_with(".css");
    // Check cache for static assets
    if is_static {
        if let Some(cached) = FETCH_CACHE.lock().unwrap().get(url) {
            return Ok(cached.clone());
        }
    }
    let text = HTTP_CLIENT
        .get(url)
        .send()
        .map_err(|e| e.to_string())?
        .text()
        .map_err(|e| e.to_string())?;
    // Cache static assets
    if is_static {
        FETCH_CACHE
            .lock()
            .unwrap()
            .insert(url.to_string(), text.clone());
    }
    Ok(text)
}

// ---------------------------------------------------------------------------
// HTTP bridge for Zipp JS
// ---------------------------------------------------------------------------

struct BrowserHttpBridge;
impl zipp_vm::http_bridge::HttpBridge for BrowserHttpBridge {
    fn get(&self, url: &str) -> Result<zipp_vm::http_bridge::HttpResponse, String> {
        let resp = HTTP_CLIENT.get(url).send().map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        let ok = resp.status().is_success();
        let body = resp.text().map_err(|e| e.to_string())?;
        Ok(zipp_vm::http_bridge::HttpResponse { status, body, ok })
    }
    fn post(&self, url: &str, body: &str, content_type: &str) -> Result<zipp_vm::http_bridge::HttpResponse, String> {
        let resp = HTTP_CLIENT.post(url).header("Content-Type", content_type).body(body.to_string()).send().map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        let ok = resp.status().is_success();
        let body = resp.text().map_err(|e| e.to_string())?;
        Ok(zipp_vm::http_bridge::HttpResponse { status, body, ok })
    }
    fn put(&self, url: &str, body: &str, content_type: &str) -> Result<zipp_vm::http_bridge::HttpResponse, String> {
        let resp = HTTP_CLIENT.put(url).header("Content-Type", content_type).body(body.to_string()).send().map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        let ok = resp.status().is_success();
        let body = resp.text().map_err(|e| e.to_string())?;
        Ok(zipp_vm::http_bridge::HttpResponse { status, body, ok })
    }
    fn delete(&self, url: &str) -> Result<zipp_vm::http_bridge::HttpResponse, String> {
        let resp = HTTP_CLIENT.delete(url).send().map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        let ok = resp.status().is_success();
        let body = resp.text().map_err(|e| e.to_string())?;
        Ok(zipp_vm::http_bridge::HttpResponse { status, body, ok })
    }
}

// ---------------------------------------------------------------------------
// Zipp JS execution
// ---------------------------------------------------------------------------

/// Run JavaScript through the Zipp VM with DOM shim.
///
/// Returns `Some((html_string, event_map_json, script_state))` on success.
/// The second element is a JSON string mapping JS node IDs to their event
/// types (e.g. `{"42":["click"]}`), extracted from the `__eventHandlers`
/// registry that the dom shim populates during `addEventListener` calls.
/// The third element is the live `ScriptState` so that event handlers can
/// be called later without re-compiling the script.
///
/// Phase 2 of the Live DOM Bridge: the ScriptState is kept alive on the
/// main thread so that click (etc.) events can invoke actual JS handlers
/// via `eval_in_context`.
fn run_zipp_js(html: &str, page_url: &str) -> Option<(String, String, ScriptState)> {
    let mut external_scripts = Vec::new();
    let mut pos = 0;
    while let Some(start) = html[pos..].find("<script") {
        let abs = pos + start;
        let tag_end = abs + html[abs..].find('>')? + 1;
        let tag = &html[abs..tag_end];
        if let Some(src) = attr(tag, "src") {
            let full = resolve(page_url, &src);
            if let Ok(source) = fetch(&full) {
                external_scripts.push((full, source));
            }
        }
        pos = tag_end;
    }
    if external_scripts.is_empty() {
        return None;
    }

    let base = url::Url::parse(page_url)
        .ok()
        .map(|u| format!("{}://{}", u.scheme(), u.host_str().unwrap_or("")))
        .unwrap_or_default();

    let dom_shim = include_str!("dom_shim.js");
    let router_helpers = include_str!("router_helpers.js");
    let shim = dom_shim.replace(
        "__PAGE_URL__",
        &page_url.replace('\\', "\\\\").replace('\'', "\\'"),
    );
    let shim = shim
        .replace("__VIEWPORT_W__", "1024")
        .replace("__VIEWPORT_H__", "768")
        .replace(
            "__BODY_HTML__",
            &body_html(html)
                .replace('\\', "\\\\")
                .replace('\'', "\\'")
                .replace('\n', "\\n")
                .replace('\r', ""),
        );

    let mut full = shim;
    full.push_str("\n;\n");
    for (_url, source) in &external_scripts {
        let patched = source
            .replace(
                "function v(e,t,n,r){let a=D(",
                &format!("function v(e,t,n,r){{{router_helpers}let a=__RD("),
            )
            .replace(
                "let i=null;for(let l=0;null==i&&l<o.length;++l){let e=R(a);i=T(o[l],e,r)}",
                "var i=null;for(var __ml=0;null==i&&__ml<o.length;++__ml){var __me=__RR(a);i=__RT(o[__ml],__me,r)}",
            );
        full.push('\n');
        full.push_str(&patched);
        full.push('\n');
    }
    // Drain timer and microtask queues
    full.push_str("for(var __t=0;__t<100;__t++){if(__timerQueue.length===0&&__mcQueue.length===0)break;while(__timerQueue.length>0){try{__timerQueue.shift()()}catch(e){}}while(__mcQueue.length>0){try{__mcQueue.shift()()}catch(e){}}}\n");
    full.push_str("for(var __r=0;__r<10;__r++){if(__mcQueue.length===0&&__timerQueue.length===0)break;while(__timerQueue.length>0){try{__timerQueue.shift()()}catch(e){}}while(__mcQueue.length>0){try{__mcQueue.shift()()}catch(e){}}}\n");

    eprintln!("[JS] Compiling {} bytes...", full.len());
    let engine = ZippEngine::default();
    let mut state = engine.compile_script(&full).ok()?;
    state.set_execution_limits(Some(200_000_000), Some(30_000));
    state.vm_mut().config.max_heap_objects = Some(2_000_000);
    state.vm_mut().config.max_heap_bytes = Some(256 * 1024 * 1024);
    state.vm_mut().http = Some(Box::new(BrowserHttpBridge));
    let _ = state.run_init();

    if let Ok(zipp_vm::object::Object::String(s)) = state.eval_in_context("__getRenderedHTML()") {
        let rendered = s.to_string();
        if !rendered.is_empty() {
            // Extract the event handler map before we lose the state.
            let event_map_json = state
                .eval_in_context("__getEventMap()")
                .ok()
                .and_then(|v| match v {
                    zipp_vm::object::Object::String(s) => Some(s.to_string()),
                    _ => None,
                })
                .unwrap_or_else(|| "{}".to_string());

            let inner = rendered
                .find("<body>")
                .map(|s| &rendered[s + 6..rendered.rfind("</body>").unwrap_or(rendered.len())])
                .unwrap_or(&rendered);
            let inner = make_urls_absolute(inner, &base);
            let inner = inner
                .replace("\\xa9", "\u{00A9}")
                .replace("\\u2019", "\u{2019}")
                .replace("\\u2018", "\u{2018}")
                .replace("\\u2014", "\u{2014}");
            let head = make_urls_absolute(&head_content(html), &base);
            eprintln!("[JS] Rendered {} bytes", inner.len());
            let html_out = format!(
                "<!DOCTYPE html><html><head>{head}</head><body>{inner}</body></html>"
            );
            return Some((html_out, event_map_json, state));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Navigation provider
// ---------------------------------------------------------------------------

/// Handles link clicks by loading pages internally.
struct BrowserNavProvider {
    proxy: ShellProxy,
}

impl NavigationProvider for BrowserNavProvider {
    fn navigate_to(&self, options: NavigationOptions) {
        let mut url = options.url.to_string();
        eprintln!("[NAV] {url}");

        // Hash-only navigation: scroll to anchor (don't reload page)
        // TODO: implement actual scroll-to-anchor in blitz-dom
        if url.contains('#') {
            let base = url.split('#').next().unwrap_or(&url);
            // If the base URL matches the current page, skip navigation
            // For now, just ignore hash-only links to avoid unnecessary reloads
            if base.is_empty() || RENDERED_CACHE.lock().unwrap().contains_key(base) {
                eprintln!("[NAV] Hash navigation (scroll) - skipping reload");
                return;
            }
            // Strip hash for fetching (server doesn't need it)
            url = base.to_string();
        }

        // Check if this is an address bar form submission
        if url.contains("__nav_url=") {
            if let Some(pos) = url.find("__nav_url=") {
                let typed = &url[pos + 10..];
                let typed = typed.split('&').next().unwrap_or(typed);
                let typed = urlencoding::decode(typed).unwrap_or_else(|_| typed.into());
                url = normalize_url(&typed);
                eprintln!("[NAV] Address bar -> {url}");
            }
        }

        let proxy = self.proxy.clone();

        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(move || {
                // Check rendered cache first (instant for revisited pages)
                if let Some(cached) = RENDERED_CACHE.lock().unwrap().get(&url) {
                    eprintln!("[NAV] Cache hit for {url}");
                    proxy.send_event(zipp_shell::ShellEvent::NavigationLoad {
                        url,
                        contents: cached.clone(),
                        retain_scroll_position: false,
                        is_md: false,
                    });
                    return;
                }

                eprintln!("[NAV] Loading {url}...");
                let html = match fetch(&url) {
                    Ok(h) => h,
                    Err(e) => {
                        eprintln!("[NAV] Fetch error: {e}");
                        format!("<html><body><h1>Error</h1><p>{e}</p></body></html>")
                    }
                };

                let rendered = if html.contains("<script") {
                    // During navigation we extract just the HTML; the
                    // ScriptState is discarded since we can't move it
                    // across threads (!Send due to Rc).
                    run_zipp_js(&html, &url)
                        .map(|(h, _emap, _state)| h)
                        .unwrap_or(html)
                } else {
                    html
                };

                // Post-process: strip noscript, toggle JS classes, make URLs absolute
                let rendered = strip_noscript(&rendered);
                let rendered = rendered
                    .replace("class=\"no-js", "class=\"js")
                    .replace("class=\"client-nojs", "class=\"client-js");
                let rendered = inline_images(&rendered, &url);
                let final_html = wrap_with_toolbar(&rendered, &url);

                // Cache for instant revisits
                RENDERED_CACHE
                    .lock()
                    .unwrap()
                    .insert(url.clone(), final_html.clone());

                proxy.send_event(zipp_shell::ShellEvent::NavigationLoad {
                    url,
                    contents: final_html,
                    retain_scroll_position: false,
                    is_md: false,
                });
            })
            .ok();
    }
}

// ---------------------------------------------------------------------------
// Toolbar CSS + HTML
// ---------------------------------------------------------------------------

/// CSS injected into every page. Resets body constraints for the toolbar layout
/// and provides Tailwind utility class fallbacks for sites with purged CSS.
const TOOLBAR_CSS: &str = r#"
html { max-width:none !important; width:100% !important; margin:0 !important; padding:0 !important; }
body { max-width:none !important; width:100% !important; margin:0 !important; padding:0 !important; }
/* Tailwind utility classes (fallback for purged CSS) */
.h-100 { height: 250px !important; }
.object-cover { object-fit: contain !important; }
.w-full { width: 100% !important; }
.w-48 { width: 12rem !important; }
.h-48 { height: 12rem !important; }
.flex { display: flex !important; }
.flex-col { flex-direction: column !important; }
.flex-grow { flex-grow: 1 !important; }
.items-center { align-items: center !important; }
.justify-between { justify-content: space-between !important; }
.inline-flex { display: inline-flex !important; }
/* Grid -> Flexbox fallback (CSS Grid not fully supported in layout engine) */
.grid { display: flex !important; flex-wrap: wrap !important; }
.grid-cols-1 > * { width: 100% !important; }
.grid-cols-2 > * { width: 48% !important; }
.grid-cols-3 > * { width: 30% !important; }
.grid-cols-4 > * { width: 23% !important; }
.grid-cols-5 > * { width: 18% !important; }
.gap-6 { gap: 1.5rem !important; }
.gap-8 { gap: 2rem !important; }
/* Spacing */
.p-4 { padding: 1rem !important; }
.p-6 { padding: 1.5rem !important; }
.px-4 { padding-left: 1rem !important; padding-right: 1rem !important; }
.py-3 { padding-top: 0.75rem !important; padding-bottom: 0.75rem !important; }
.py-8 { padding-top: 2rem !important; padding-bottom: 2rem !important; }
.py-16 { padding-top: 4rem !important; padding-bottom: 4rem !important; }
.mb-2 { margin-bottom: 0.5rem !important; }
.mb-4 { margin-bottom: 1rem !important; }
.mb-12 { margin-bottom: 3rem !important; }
.mr-1 { margin-right: 0.25rem !important; }
.mt-auto { margin-top: auto !important; }
.mx-auto { margin-left: auto !important; margin-right: auto !important; }
/* Typography */
.text-center { text-align: center !important; }
.text-2xl { font-size: 1.5rem !important; }
.text-4xl { font-size: 2.25rem !important; }
.font-bold { font-weight: 700 !important; }
.font-semibold { font-weight: 600 !important; }
.font-medium { font-weight: 500 !important; }
.text-primary { color: #1e3a8a !important; }
.text-secondary { color: #3b82f6 !important; }
.text-gray-700 { color: #374151 !important; }
.text-white { color: #ffffff !important; }
.text-text { color: #111827 !important; }
/* Card */
.bg-white { background-color: #ffffff !important; }
.bg-background { background-color: #f9fafb !important; }
.border { border-width: 1px !important; border-style: solid !important; }
.border-gray-200 { border-color: #e5e7eb !important; }
.rounded-lg { border-radius: 0.5rem !important; }
.rounded-full { border-radius: 50% !important; }
.shadow { box-shadow: 0 1px 3px rgba(0,0,0,.1), 0 1px 2px rgba(0,0,0,.06) !important; }
.shadow-md { box-shadow: 0 4px 6px -1px rgba(0,0,0,.1), 0 2px 4px -2px rgba(0,0,0,.1) !important; }
.overflow-hidden { overflow: hidden !important; }
.container { max-width: 1280px !important; width: 100% !important; }
.min-h-screen { min-height: 100vh !important; }
/* Layout helpers */
.block { display: block !important; }
.relative { position: relative !important; }
.fixed { position: fixed !important; }
.top-0 { top: 0 !important; }
.left-0 { left: 0 !important; }
.right-0 { right: 0 !important; }
.z-50 { z-index: 50 !important; }
"#;

fn wrap_with_toolbar(html: &str, current_url: &str) -> String {
    let head_end = html.find("</head>").unwrap_or(0);
    let body_start = html
        .find("<body")
        .and_then(|i| html[i..].find('>').map(|e| i + e + 1))
        .unwrap_or(0);
    let body_end = html.find("</body>").unwrap_or(html.len());

    let head_content = if head_end > 0 {
        let head_start = html
            .find("<head")
            .and_then(|i| html[i..].find('>').map(|e| i + e + 1))
            .unwrap_or(0);
        &html[head_start..head_end]
    } else {
        ""
    };

    let body_content = if body_start > 0 {
        &html[body_start..body_end]
    } else {
        html
    };

    format!(
        r#"<!DOCTYPE html><html><head>
<meta charset="utf-8">
<meta name="viewport" content="width=1280">
{head_content}
<style>{TOOLBAR_CSS}</style>
</head><body style="margin:0 !important;padding:0 !important;max-width:none !important;width:100% !important">
<div style="display:flex;flex-direction:column;margin:0;padding:0;width:100%;height:100vh">
  <form action="{current_url}" method="GET" style="height:42px;min-height:42px;background:#f8f9fa;border-bottom:1px solid #dee2e6;display:flex;align-items:center;padding:4px 8px;gap:6px;font-family:Arial,sans-serif;box-sizing:border-box;width:100%;margin:0;flex-shrink:0">
    <span style="font-size:16px;color:#495057;padding:0 4px;cursor:pointer">◀</span>
    <span style="font-size:16px;color:#495057;padding:0 4px;cursor:pointer">▶</span>
    <input type="text" name="__nav_url" value="{current_url}" style="width:90%;height:32px;padding:0 12px;border:1px solid #ced4da;border-radius:8px;font-size:14px;background:white;outline:none;box-sizing:border-box" />
  </form>
  {body_content}
</div>
</body></html>"#
    )
}

// ---------------------------------------------------------------------------
// CSS inlining
// ---------------------------------------------------------------------------

/// Replace external `<link rel="stylesheet">` tags with inline `<style>` blocks.
fn inline_css(html: &str, base_url: &str) -> String {
    let mut result = html.to_string();

    while let Some(link_start) = result.find("<link") {
        let link_end = match result[link_start..].find('>') {
            Some(e) => link_start + e + 1,
            None => break,
        };
        let tag = result[link_start..link_end].to_string();

        if !tag.contains("stylesheet") {
            // Not a stylesheet -- skip by replacing with a comment marker
            let mut replacement = String::with_capacity(tag.len() + 9);
            replacement.push_str("<!--link");
            replacement.push_str(&tag[1..tag.len() - 1]);
            replacement.push_str("-->");
            result.replace_range(link_start..link_end, &replacement);
            continue;
        }

        if let Some(href) = attr(&tag, "href") {
            let full_url = resolve(base_url, &href);
            if let Ok(css) = fetch(&full_url) {
                if !css.is_empty() && !css.starts_with("<!") {
                    // Pass CSS through unchanged - Stylo + Taffy handle it natively
                    let mut replacement = String::with_capacity(css.len() + 15);
                    replacement.push_str("<style>");
                    replacement.push_str(&css);
                    replacement.push_str("</style>");
                    result.replace_range(link_start..link_end, &replacement);
                    continue;
                }
            }
        }
        break; // Avoid infinite loop
    }

    // Restore comment markers back to tags
    result = result.replace("<!--link", "<").replace("-->", ">");
    result
}

// ---------------------------------------------------------------------------
// Image URL resolution
// ---------------------------------------------------------------------------

/// Make relative image `src` attributes absolute so zipp-dom's net_provider
/// can fetch them. Does not inline image data -- zipp-dom handles raster
/// (via image crate) and SVG (via usvg) natively.
/// Remove `<noscript>...</noscript>` blocks from HTML.
/// Our browser supports JS, so noscript fallback content should not render.
fn strip_noscript(html: &str) -> String {
    let mut result = html.to_string();
    while let Some(start) = result.find("<noscript") {
        // Find matching </noscript>
        if let Some(end) = result[start..].find("</noscript>") {
            result.replace_range(start..start + end + 11, "");
        } else {
            break;
        }
    }
    result
}

fn inline_images(html: &str, base_url: &str) -> String {
    let mut result = html.to_string();
    let mut search_pos = 0;

    while let Some(idx) = result[search_pos..].find("src=\"") {
        let abs_idx = search_pos + idx + 5;
        if let Some(end) = result[abs_idx..].find('"') {
            let src = &result[abs_idx..abs_idx + end];
            if src.starts_with("data:") || src.starts_with("http") || src.is_empty() {
                search_pos = abs_idx + end;
                continue;
            }
            let full_url = resolve(base_url, src);
            // Replace just the src value in place instead of rebuilding the whole string
            result.replace_range(abs_idx..abs_idx + end, &full_url);
            search_pos = abs_idx + full_url.len();
        } else {
            break;
        }
    }
    result
}

// ---------------------------------------------------------------------------
// URL helpers
// ---------------------------------------------------------------------------

fn make_urls_absolute(html: &str, base: &str) -> String {
    // Pre-compute replacement strings once
    let src_replacement = {
        let mut s = String::with_capacity(5 + base.len() + 1);
        s.push_str("src=\"");
        s.push_str(base);
        s.push('/');
        s
    };
    let href_replacement = {
        let mut s = String::with_capacity(6 + base.len() + 1);
        s.push_str("href=\"");
        s.push_str(base);
        s.push('/');
        s
    };
    let base_slash = {
        let mut s = String::with_capacity(base.len() + 1);
        s.push_str(base);
        s.push('/');
        s
    };

    let result = html
        .replace("src=\"/", &src_replacement)
        .replace("href=\"/", &href_replacement);

    // Also handle bare relative filenames (e.g. src="image.jpg")
    let mut out = String::with_capacity(result.len());
    let mut pos = 0;
    while pos < result.len() {
        if let Some(idx) = result[pos..].find("src=\"") {
            let abs = pos + idx;
            out.push_str(&result[pos..abs + 5]);
            let val_start = abs + 5;
            if let Some(end) = result[val_start..].find('"') {
                let val = &result[val_start..val_start + end];
                if !val.starts_with("http") && !val.starts_with("data:") && !val.starts_with("//")
                {
                    out.push_str(&base_slash);
                }
                out.push_str(val);
                pos = val_start + end;
            } else {
                pos = val_start;
            }
        } else {
            out.push_str(&result[pos..]);
            break;
        }
    }
    out
}

#[inline]
fn normalize_url(s: &str) -> String {
    let s = s.trim();
    if s.is_empty() {
        return "https://html.duckduckgo.com".to_string();
    }
    if s.starts_with("http://") || s.starts_with("https://") {
        return s.to_string();
    }
    if s.contains('.') && !s.contains(' ') {
        let mut url = String::with_capacity(8 + s.len());
        url.push_str("https://");
        url.push_str(s);
        return url;
    }
    let encoded = s.replace(' ', "+");
    let mut url = String::with_capacity(37 + encoded.len());
    url.push_str("https://html.duckduckgo.com/?q=");
    url.push_str(&encoded);
    url
}

#[inline]
fn resolve(base: &str, rel: &str) -> String {
    if rel.starts_with("http") {
        return rel.to_string();
    }
    url::Url::parse(base)
        .ok()
        .and_then(|b| b.join(rel).ok())
        .map(|u| u.to_string())
        .unwrap_or_else(|| rel.to_string())
}

// ---------------------------------------------------------------------------
// HTML attribute extraction
// ---------------------------------------------------------------------------

/// Extract the value of a `name="value"` attribute from an HTML tag string.
#[inline]
fn attr(tag: &str, name: &str) -> Option<String> {
    let mut s = String::with_capacity(name.len() + 2);
    s.push_str(name);
    s.push_str("=\"");
    let p = tag.find(&s)? + s.len();
    let raw = &tag[p..p + tag[p..].find('"')?];
    // Decode common HTML entities in attribute values
    Some(raw.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\""))
}

#[inline]
fn body_html(html: &str) -> String {
    let start = match html.find("<body") {
        Some(i) => match html[i..].find('>') {
            Some(e) => i + e + 1,
            None => return String::new(),
        },
        None => return String::new(),
    };
    let end = html.find("</body>").unwrap_or(html.len());
    html[start..end].to_string()
}

#[inline]
fn head_content(html: &str) -> String {
    let s = html
        .find("<head")
        .and_then(|i| html[i..].find('>').map(|e| i + e + 1));
    let e = html.find("</head>");
    match (s, e) {
        (Some(s), Some(e)) => {
            let head = &html[s..e];
            let mut result = String::new();
            let mut p = 0;
            while p < head.len() {
                if let Some(ss) = head[p..].find("<script") {
                    result.push_str(&head[p..p + ss]);
                    p += ss
                        + head[p + ss..]
                            .find("</script>")
                            .map(|e| e + 9)
                            .unwrap_or(head.len() - p);
                } else {
                    result.push_str(&head[p..]);
                    break;
                }
            }
            result
        }
        _ => String::new(),
    }
}
