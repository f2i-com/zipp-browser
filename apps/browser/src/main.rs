//! Zipp Browser -- Stylo CSS + Taffy Layout + tiny-skia rendering + Zipp JS
//!
//! Pipeline: Fetch -> Zipp JS (React/SPA) -> inline images -> Stylo cascade
//!           -> Taffy layout -> tiny-skia paint -> winit window
//!
//! CSS is handled by the real engine, not rewritten on the way in. There used to
//! be a `responsive` module that unwrapped `@media` blocks, converted Tailwind
//! `md:`/`lg:` classes to inline styles and rewrote CSS Grid containers into
//! flexbox with hardcoded child widths — all on the premise that Stylo could not
//! do media queries and Taffy could not do grid. Both premises were wrong:
//! Stylo is Firefox's style engine and evaluates `@media` against the device
//! built in `zipp_dom::document::make_device`, and Taffy's `grid` feature is on
//! by default with `compute_grid_layout` wired up. Those rewrites only made the
//! output diverge from a real browser, so they are gone.
//!
//! `--screenshot` renders a page headlessly to a PNG, which is how the output
//! gets compared against Chrome.

// -- std --
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// -- external --
use linebender_resource_handle::Blob;

// -- internal (workspace) --
use zipp_vm::embed::{compile_script, JsValue, ScriptState};
use zipp_dom::html_parser::{HtmlDocument, HtmlProvider};
use zipp_dom::traits_src::navigation::{NavigationOptions, NavigationProvider};
use zipp_dom::{DocumentConfig, FontContext};
use zipp_renderer::tinyskia::TinySkiaWindowRenderer;
use zipp_shell::{create_default_event_loop, ZippApp, ShellProxy, WindowConfig};

mod esm;
mod event_bridge;

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

/// Viewport the page's JavaScript is told about on the windowed path.
///
/// The window does not exist yet when the page's script runs, so this is the
/// size the shim reports for `window.innerWidth`/`innerHeight`. CSS is laid out
/// against the real window (the Stylo device is rebuilt from it, and again on
/// every resize), so only JS that branches on the reported size is affected.
const INITIAL_WINDOW_SIZE: (u32, u32) = (1280, 800);

/// Seconds to advance the animation clock before a headless screenshot, so CSS
/// entrance animations land on their final keyframe rather than frame zero.
const ANIMATION_SETTLE_SECS: f64 = 5.0;

/// Customisable User-Agent string. Change this to mimic different browsers.
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Zipp/1.0";

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_default();

    // Quick JS test mode: run a suite through the VM and exit. Exit status
    // reflects the result so these are usable from a script or CI.
    if arg == "--test-js" {
        let ok = run_js_suite("TEST", include_str!("js_tests.js"), 10);
        std::process::exit(if ok { 0 } else { 1 });
    }
    if arg == "--test-vm" {
        run_vm_test();
        return;
    }
    if arg == "--test-compat" {
        let ok = run_js_suite("COMPAT", include_str!("js_compat_tests.js"), 20);
        std::process::exit(if ok { 0 } else { 1 });
    }
    // Headless render: load a page, lay it out and paint it to a PNG without
    // opening a window. This is how the rendering is checked against a real
    // browser — a screenshot is the only honest way to compare CSS output.
    //   zipp-browser --screenshot out.png [url] [width] [height]
    if arg == "--screenshot" {
        let mut rest = std::env::args().skip(2);
        let out = rest.next().unwrap_or_else(|| "screenshot.png".to_string());
        let url = rest.next().unwrap_or_else(|| "https://lance.name".to_string());
        let width: u32 = rest.next().and_then(|s| s.parse().ok()).unwrap_or(1280);
        let height: u32 = rest.next().and_then(|s| s.parse().ok()).unwrap_or(2000);
        let ok = screenshot(&normalize_url(&url), &out, width, height);
        std::process::exit(if ok { 0 } else { 1 });
    }

    let url = if arg.is_empty() { "https://lance.name".to_string() } else { arg };
    eprintln!("[BROWSER] Loading {url}");

    // 1-3. Fetch, run the page's JS, post-process. The ScriptState is !Send, so
    //      this stays on the main thread to keep it alive for event dispatch.
    let (final_html, event_map_json, js_state) = prepare_page(&url, INITIAL_WINDOW_SIZE);
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
        let flid_map = event_bridge::build_flid_map(doc.as_ref());
        if !flid_map.is_empty() {
            eprintln!("[EVENT-BRIDGE] Mapped {} nodes with data-flid attributes", flid_map.len());
        }
        let bridge = event_bridge::JsEventBridge::new(flid_map, emap_json, js_state, &url);
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
    // Run the whole event loop on a grown stack. Layout, hit-testing and paint
    // all recurse over the DOM on the native stack, and a real page's tree is
    // deep enough to overflow the default main-thread stack — a server-rendered
    // React app nests far further than the hand-written test pages did. Growing
    // here covers every callback the loop dispatches, including the re-render
    // that follows a JS event handler.
    stacker::maybe_grow(64 * 1024 * 1024, 256 * 1024 * 1024, || {
        event_loop.run_app(app).unwrap();
    });
}

// ---------------------------------------------------------------------------
// Pre-rendering
// ---------------------------------------------------------------------------

/// Run one of the bundled JS suites against the DOM shim and report results.
///
/// Both suites follow the same protocol: run to completion, then read the
/// pass/fail counters `_p`/`_f` and the failure descriptions in `_e`.
/// `--test-js` exercises the DOM surface, `--test-compat` the language surface;
/// only the source file and how many failures to print differ.
fn run_js_suite(tag: &str, tests: &str, max_failures: usize) -> bool {
    let dom_shim = include_str!("dom_shim.js");
    let shim = dom_shim
        .replace("__PAGE_URL__", "http://localhost/test")
        .replace("__VIEWPORT_W__", "1280")
        .replace("__VIEWPORT_H__", "800")
        .replace("__BODY_HTML__", "<div id=\\'root\\'></div>");

    let full = format!("{HOST_PRELUDE_JS}\n{shim}{tests}");
    eprintln!("[{tag}] Compiling {} bytes...", full.len());
    let mut state = match compile_script(&full) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[{tag}] Compile error: {e}");
            return false;
        }
    };
    state.set_host_call(Box::new(browser_host_call));
    eprintln!("[{tag}] Executing...");
    if let Err(e) = state.run_init() {
        eprintln!("[{tag}] Uncaught: {e}");
    }

    let passed = state.eval_in_context("_p").ok();
    let failed = state.eval_in_context("_f").ok();
    let (Some(JsValue::Number(passed)), Some(JsValue::Number(failed))) = (passed, failed) else {
        eprintln!("[{tag}] suite did not report counters (_p/_f)");
        return false;
    };
    eprintln!("[{tag}] {passed}/{} passed", passed + failed);
    for i in 0..max_failures {
        if let Ok(JsValue::String(s)) = state.eval_in_context(&format!("_e[{i}] || ''")) {
            if !s.is_empty() {
                eprintln!("[{tag}]   {s}");
            }
        }
    }
    failed == 0.0
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
    let full = format!("{HOST_PRELUDE_JS}\n{shim}\n{js}");
    eprintln!("[VM-TEST] Running with DOM shim ({} bytes)...", full.len());
    let mut state = match compile_script(&full) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[VM-TEST] Compile error: {e}");
            return;
        }
    };
    state.set_host_call(Box::new(browser_host_call));
    if let Err(e) = state.run_init() {
        eprintln!("[VM-TEST] Uncaught: {e}");
    }
    match state.eval_in_context("result") {
        Ok(JsValue::String(s)) => eprintln!("[VM-TEST] {s}"),
        Ok(other) => eprintln!("[VM-TEST] Unexpected result type: {other:?}"),
        Err(e) => eprintln!("[VM-TEST] Could not read result: {e}"),
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
            run_zipp_js(&html, link, INITIAL_WINDOW_SIZE)
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

    // Fonts are compiled in, not read from disk. This used to be a hardcoded
    // list of `C:\Windows\Fonts\*.ttf` paths, which tied text rendering to
    // whichever fonts a given Windows install happened to ship and rendered
    // nothing at all on any other platform. See `zipp_dom::fonts`.
    zipp_dom::register_embedded_fonts(&mut font_ctx.collection);

    font_ctx
        .collection
        .register_fonts(Blob::new(Arc::new(zipp_dom::BULLET_FONT) as _), None);

    font_ctx
}

// ---------------------------------------------------------------------------
// Networking
// ---------------------------------------------------------------------------

fn fetch(url: &str) -> Result<String, String> {
    // `file://` — load from disk. Needed to render a local page at all, and it
    // is what makes a local CSS test corpus possible: comparing against another
    // browser on hand-written test cases is far more precise than only ever
    // loading live sites.
    if let Some(path) = url.strip_prefix("file://") {
        // Accept file:///C:/x, file://C:/x and file:///c%3A/x alike.
        let path = path.trim_start_matches('/');
        let path = urlencoding::decode(path).unwrap_or(std::borrow::Cow::Borrowed(path));
        return std::fs::read_to_string(path.as_ref())
            .map_err(|e| format!("cannot read {path}: {e}"));
    }

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

/// The JS prelude prepended to every page's script bundle.
///
/// Two jobs. First, it turns the raw `__zippHostCall` trampoline into the
/// `http` object the DOM shim's `fetch` is written against — the old engine
/// exposed `http` as a native global, whereas zipp-vm deliberately has no host
/// surface of its own, so the browser supplies one.
///
/// Second, it defines the entry points the Rust side calls back into
/// (`__zippDispatchEvent`, `__zippDrainQueues`). Declaring them ONCE here,
/// rather than assembling an equivalent source string per event, matters:
/// `eval_in_context` compiles fresh each call and the compiled functions are
/// interned for the VM's lifetime, so a per-click eval would grow memory for
/// as long as the page is open. `call_global` compiles nothing.
///
/// These reference `__eventHandlers`, `__timerQueue` and `__mcQueue`, which
/// `dom_shim.js` defines. That is fine even though the prelude comes first —
/// the names are resolved when the functions run, not when they are declared.
const HOST_PRELUDE_JS: &str = r#"
// `env.log(level, message)` — the shim builds its `console` on top of this.
// The old engine provided it as a native global; here it goes out through the
// host channel to the browser's own stderr log, so page `console.log` shows up
// next to [BROWSER]/[JS] diagnostics.
//
// It cannot be written in terms of `console`: dom_shim.js declares
// `var console = {...}`, and because the shim and this prelude are concatenated
// into ONE script that `var` hoists to the top of the whole program — so
// `console` refers to the shim's object from the first line onward, and routing
// env.log through it would be infinite recursion.
var env = {
    log: function (level, message) {
        __zippHostCall("log", String(level), String(message));
    }
};

var http = (function () {
    function request(method, url, body, contentType) {
        // Host contract: strings in, JSON string out. A transport failure is a
        // JS throw (the host returns Err), which fetch()'s try/catch turns into
        // a rejected promise — same shape as a real browser network error.
        var raw = __zippHostCall(
            "http", method, String(url),
            body == null ? "" : String(body),
            contentType == null ? "" : String(contentType)
        );
        var r = JSON.parse(raw);
        return { status: r.status, ok: r.ok, body: r.body };
    }
    return {
        get: function (url) { return request("GET", url, "", ""); },
        post: function (url, body, ct) { return request("POST", url, body, ct); },
        put: function (url, body, ct) { return request("PUT", url, body, ct); },
        "delete": function (url) { return request("DELETE", url, "", ""); }
    };
})();

// Run the shim's timer and microtask queues to quiescence, or until `rounds`
// passes have gone by. React's setState is asynchronous, so a click handler
// has typically only QUEUED the re-render when it returns; without this the
// page would be read back before the update landed. Bounded because a page
// can legitimately reschedule forever (a spinner, a poll loop).
var __zippDrainStats = { rounds: 0, timers: 0, micro: 0 };
function __zippDrainQueues(rounds) {
    for (var i = 0; i < rounds; i++) {
        if (__timerQueue.length === 0 && __mcQueue.length === 0) break;
        // Report, never swallow. A page's real work happens in these callbacks
        // — React commits its first render from one — so an empty catch here
        // turns a broken app into a blank page with no explanation.
        __zippDrainStats.rounds++;
        while (__timerQueue.length > 0) {
            __zippDrainStats.timers++;
            try { __timerQueue.shift()(); }
            catch (e) { env.log('error', 'timer callback threw: ' + ((e && (e.stack || e.message)) || e)); }
        }
        while (__mcQueue.length > 0) {
            __zippDrainStats.micro++;
            try { __mcQueue.shift()(); }
            catch (e) { env.log('error', 'microtask threw: ' + ((e && (e.stack || e.message)) || e)); }
        }
    }
}

// Invoke the handlers registered on `handlerNid` for `type`, with the event
// targeted at `targetNid`.
//
// The two ids are usually DIFFERENT, and that is the whole point. React 18 does
// not put listeners on the elements you click — it attaches ONE delegated
// listener per event type to the root container and works out which component
// was hit by reading `event.target`. So `handlerNid` is nearly always the root,
// and `targetNid` is the element actually under the pointer.
//
// Passing `target: null` (as this once did) meant React received every click as
// "something in the root was clicked, but nothing in particular" and routed it
// nowhere: buttons appeared dead. The hamburger menu was the visible symptom.
//
// `target` must be the live node object, not an id — React finds its fiber via
// an expando it set on that very object.
// The class list the page put on <html>.
//
// Tailwind's dark mode is a `.dark` class on the document element, and the app
// toggles it from an effect. Our re-render replaces only the BODY, so that
// change never reached the real DOM and the theme toggle did nothing.
// Just the class list on <html>, for applying to the real document element.
function __getHtmlClassOnly() {
    try { return String((doc.documentElement && doc.documentElement.className) || ''); }
    catch (e) { return ''; }
}

function __getHtmlClass() {
    try {
        var el = doc.documentElement;
        if (!el) return '';
        var out = [];
        if (el.className) out.push('class="' + String(el.className).replace(/"/g, '&quot;') + '"');
        // Themes are marked with a class OR an attribute (`data-theme`,
        // `data-mode`, `color-scheme`); carry whatever is actually there rather
        // than guessing which convention this page uses.
        for (var k in el.attributes) {
            if (k === 'class' || k === 'style') continue;
            out.push(k + '="' + String(el.attributes[k]).replace(/"/g, '&quot;') + '"');
        }
        return out.join(' ');
    } catch (e) { return ''; }
}

function __zippDispatchEvent(handlerNid, targetNid, type) {
    var byType = __eventHandlers[handlerNid];
    if (!byType) return false;
    var handlers = byType[type];
    if (!handlers || handlers.length === 0) return false;

    var target = __nodesById[targetNid] || __nodesById[handlerNid] || null;
    var currentTarget = __nodesById[handlerNid] || null;
    var defaultPrevented = false;
    var propagationStopped = false;
    var immediateStopped = false;
    var ev = {
        type: type,
        target: target,
        // Legacy alias some libraries still read.
        srcElement: target,
        currentTarget: currentTarget,
        eventPhase: 2,
        bubbles: true,
        cancelable: true,
        isTrusted: true,
        timeStamp: 0,
        detail: 1,
        button: 0,
        buttons: 1,
        clientX: 0, clientY: 0, pageX: 0, pageY: 0, screenX: 0, screenY: 0,
        altKey: false, ctrlKey: false, metaKey: false, shiftKey: false,
        get defaultPrevented() { return defaultPrevented; },
        preventDefault: function () { defaultPrevented = true; },
        stopPropagation: function () { propagationStopped = true; },
        stopImmediatePropagation: function () {
            propagationStopped = true;
            immediateStopped = true;
        },
        composedPath: function () {
            var path = [], n = target;
            while (n) { path.push(n); n = n.parentNode; }
            return path;
        }
    };
    // React reads `nativeEvent` off its synthetic wrapper; point it at itself so
    // either access works.
    ev.nativeEvent = ev;

    for (var i = 0; i < handlers.length; i++) {
        // Never swallow this silently. A listener that throws is the difference
        // between "the click did nothing" and "the click did nothing BECAUSE
        // React tore its own state down", and those need very different fixes.
        // Swallowing it cost real debugging time on the hamburger menu.
        try {
            handlers[i](ev);
        } catch (e) {
            env.log('error', 'listener for "' + type + '" threw: ' +
                ((e && (e.stack || e.message)) || e));
        }
        // Only stopImmediatePropagation may cut short the OTHER listeners on
        // this same node. stopPropagation stops the event travelling to other
        // nodes, which is a different thing entirely.
        //
        // React registers TWO listeners per event type on the root — one for
        // the capture phase, one for bubble — and `onClick` lives on the bubble
        // one. Treating stopPropagation as "stop calling listeners" meant that
        // as soon as anything stopped propagation during capture (a
        // click-outside overlay, for instance), onClick was never reached and
        // the control became inert after its first use.
        if (immediateStopped) break;
    }
    // Report whether a listener called preventDefault(). The caller turns this
    // into blitz's own event state, which is what suppresses the element's
    // default action — for an <a>, a real navigation. React Router cancels the
    // click and routes in-page; without this the navigation ALSO ran, throwing
    // away the live JS state and leaving every control on the next page dead.
    return defaultPrevented;
}
"#;

/// The Rust half of the host channel: blocking HTTP on behalf of page JS.
///
/// Blocking is deliberate and matches the previous engine — the DOM shim's
/// `fetch` resolves its promise synchronously from this result, and the whole
/// SSR pass runs to completion before anything is rendered, so there is no
/// event loop to yield to yet.
fn browser_host_call(kind: &str, args: &[String]) -> Result<String, String> {
    match kind {
        // Page `console.*`, via the shim's `env.log`.
        "log" => {
            let level = args.first().map(String::as_str).unwrap_or("info");
            let message = args.get(1).map(String::as_str).unwrap_or("");
            eprintln!("[JS:{level}] {message}");
            Ok(String::new())
        }
        "http" => {
            let method = args.first().map(String::as_str).unwrap_or("GET");
            let url = args.get(1).map(String::as_str).unwrap_or_default();
            let body = args.get(2).cloned().unwrap_or_default();
            let content_type = args.get(3).map(String::as_str).unwrap_or("");
            let content_type =
                if content_type.is_empty() { "application/json" } else { content_type };

            let req = match method {
                "POST" => HTTP_CLIENT.post(url).header("Content-Type", content_type).body(body),
                "PUT" => HTTP_CLIENT.put(url).header("Content-Type", content_type).body(body),
                "DELETE" => HTTP_CLIENT.delete(url),
                _ => HTTP_CLIENT.get(url),
            };
            // Errors carry the `Error: ` prefix so the script sees an ordinary
            // JS error rather than a bare string.
            let resp = req.send().map_err(|e| format!("Error: {method} {url}: {e}"))?;
            let status = resp.status().as_u16();
            let ok = resp.status().is_success();
            let body = resp.text().map_err(|e| format!("Error: reading {url}: {e}"))?;
            Ok(format!(
                "{{\"status\":{status},\"ok\":{ok},\"body\":{}}}",
                json_quote(&body)
            ))
        }
        other => Err(format!("Error: unknown host call {other:?}")),
    }
}

/// Minimal JSON string literal encoder — enough to carry an arbitrary response
/// body through `JSON.parse` on the JS side. Hand-rolled for the same reason
/// `event_bridge` hand-rolls its parser: one string field does not justify a
/// serde dependency.
fn json_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // Control characters (and U+2028/29, which are newlines to a JS
            // parser) must be escaped or the JSON is invalid.
            c if (c as u32) < 0x20 || c == '\u{2028}' || c == '\u{2029}' => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// ---------------------------------------------------------------------------
// Page pipeline
// ---------------------------------------------------------------------------

/// Fetch `url`, run its JS, and post-process into the HTML the renderer sees.
///
/// Shared by the window path and `--screenshot` so the two cannot drift — a
/// screenshot that went through a different pipeline would be worthless for
/// comparing against a real browser.
///
/// Returns the final HTML, the JS event map (when the page ran script), and the
/// live `ScriptState` that keeps event handlers callable.
fn prepare_page(url: &str, viewport: (u32, u32)) -> (String, Option<String>, Option<ScriptState>) {
    let t_fetch = std::time::Instant::now();
    let html = fetch(url)
        .unwrap_or_else(|e| format!("<html><body><h1>Error</h1><p>{e}</p></body></html>"));
    let fetch_ms = t_fetch.elapsed().as_millis();

    let t_js = std::time::Instant::now();
    let (rendered, event_map_json, js_state) = if html.contains("<script") {
        match run_zipp_js(&html, url, viewport) {
            Some((r, emap, state)) => (r, Some(emap), Some(state)),
            None => (html.clone(), None, None),
        }
    } else {
        (html, None, None)
    };
    let js_ms = t_js.elapsed().as_millis();

    let t_post = std::time::Instant::now();
    // Strip <noscript> blocks (our browser has JS support)
    let out = strip_noscript(&rendered);
    // Toggle JS detection classes: no-js → js, client-nojs → client-js
    let out = out
        .replace("class=\"no-js", "class=\"js")
        .replace("class=\"client-nojs", "class=\"client-js");
    let out = inline_images(&out, url);
    let out = wrap_with_toolbar(&out, url);
    let post_ms = t_post.elapsed().as_millis();

    eprintln!(
        "[PERF] Page: fetch={fetch_ms}ms js={js_ms}ms post={post_ms}ms ({} bytes)",
        out.len()
    );
    (out, event_map_json, js_state)
}

// ---------------------------------------------------------------------------
// Headless rendering
// ---------------------------------------------------------------------------

/// Load `url`, lay it out at `width`x`height` and paint it to a PNG.
///
/// No window, no event loop — just the same document pipeline the window uses,
/// pointed at a software framebuffer. Returns whether it succeeded.
fn screenshot(url: &str, out_path: &str, width: u32, height: u32) -> bool {
    use zipp_dom::traits_src::shell::{ColorScheme, Viewport};

    eprintln!("[SHOT] {url} -> {out_path} ({width}x{height})");
    let (mut final_html, event_map, mut js_state) = prepare_page(url, (width, height));

    // `ZIPP_CLICK=<data-flid>` dispatches a click at that element after the page
    // has settled, then re-serialises. Interaction otherwise needs a window and a
    // human, which makes it untestable here; this exercises the same JS path the
    // event bridge uses (`__zippDispatchEvent` -> drain -> `__getRenderedHTML`).
    if let Some(flid) = std::env::var("ZIPP_CLICK").ok().and_then(|v| v.parse::<u32>().ok()) {
        if let Some(state) = js_state.as_mut() {
            // React delegates to the root, so the handler node and the clicked
            // node differ: find the root's flid the same way the bridge does.
            let root_flid = match state.eval_in_context(
                "(function(){for(var k in __eventHandlers){if(__eventHandlers[k]['click'])return k}return ''})()",
            ) {
                Ok(JsValue::String(s)) => s.parse::<u32>().unwrap_or(flid),
                _ => flid,
            };
            let repeats: u32 = std::env::var("ZIPP_CLICK_N")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1);
            for n in 0..repeats {
            if n == 0 {
            }
            eprintln!("[CLICK] dispatch {} handler=#{root_flid} target=#{flid}", n + 1);
            match state.call_global(
                "__zippDispatchEvent",
                &[
                    JsValue::Number(f64::from(root_flid)),
                    JsValue::Number(f64::from(flid)),
                    JsValue::String("click".to_string()),
                ],
            ) {
                Ok(v) => eprintln!("[CLICK] dispatched -> {v:?}"),
                Err(e) => eprintln!("[CLICK] dispatch failed: {e}"),
            }
            for _ in 0..8 {
                state.run_microtasks();
                let _ = state.call_global("__zippDrainQueues", &[JsValue::Number(50.0)]);
            }
            if let Ok(JsValue::String(updated)) = state.call_global("__getRenderedHTML", &[]) {
                if !updated.is_empty() {
                    eprintln!(
                        "[CLICK] after dispatch {}: {} bytes, menu_open={}",
                        n + 1,
                        updated.len(),
                        updated.contains("Close menu")
                    );
                    // Same rebuild the event bridge performs, so this harness
                    // exercises the real path rather than an approximation.
                    let body = rerender_body(&updated, url);
                    let _ = &body;
                    final_html = final_html
                        .split_once("<body")
                        .and_then(|(head, rest)| {
                            rest.find('>').map(|e| {
                                let close = rest.rfind("</body>").unwrap_or(rest.len());
                                format!("{head}<body{}>{body}</body></html>", &rest[..e])
                                    .replace("</body></html></html>", "</body></html>")
                                    + &rest[close..close]
                            })
                        })
                        .unwrap_or(final_html);
                }
            }
        }
    }
    // Write the HTML the renderer sees alongside the image. When a screenshot
    // looks wrong the first question is always "is this a CSS/layout problem or
    // did the page's JS produce the wrong DOM?", and this answers it directly.
    let html_path = format!("{out_path}.html");
    if let Err(e) = std::fs::write(&html_path, &final_html) {
        eprintln!("[SHOT] could not write {html_path}: {e}");
    } else {
        eprintln!("[SHOT] wrote {html_path}");
    }

            }

    let font_ctx = load_fonts();
    // The net provider fetches stylesheets and images. Those land asynchronously,
    // so a tokio runtime has to be alive while the document resolves.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let _guard = rt.enter();

    let net_provider = Arc::new(zipp_shell::net::Provider::new(None));
    let config = DocumentConfig {
        font_ctx: Some(font_ctx),
        base_url: Some(url.to_string()),
        net_provider: Some(net_provider.clone() as _),
        html_parser_provider: Some(Arc::new(HtmlProvider) as _),
        // Present so link clicks take the SAME path they take in the window:
        // it records rather than navigates, so `[NAV-WOULD]` in the log means
        // the page did NOT cancel the click and the real browser would have
        // reloaded and dropped its JS state.
        navigation_provider: Some(Arc::new(RecordingNavProvider) as _),
        viewport: Some(Viewport::new(width, height, 1.0, ColorScheme::Light)),
        ..Default::default()
    };

    let t_parse = std::time::Instant::now();
    let mut doc = HtmlDocument::from_html(&final_html, config);

    // Install the JS event bridge here too, not just on the windowed path.
    // Without it `--screenshot` has no event handler at all, so a simulated
    // click silently does nothing — which is exactly how a broken click path
    // could look "verified" while the real window stayed dead.
    if let Some(ref emap) = event_map {
        let flid_map = event_bridge::build_flid_map(doc.as_ref());
        let bridge =
            event_bridge::JsEventBridge::new(flid_map, emap, js_state.take(), url);
        if bridge.handler_count() > 0 {
            doc.set_event_handler(Box::new(bridge));
        }
    }
    let parse_ms = t_parse.elapsed().as_millis();

    // Let linked stylesheets, webfonts and images arrive before layout.
    //
    // Each pass must `resolve`, not just `resolve_deferred_tasks`: only
    // `resolve` calls `handle_messages`, and a fetched stylesheet is not read
    // until its message is handled. Fonts are a SECOND round-trip — the
    // `@font-face` rules only exist once the CSS message is processed, and each
    // one then queues its own woff2 fetch. Draining `resolve_deferred_tasks`
    // alone left the CSS messages unread until after this loop, so the font
    // fetches were still in flight at paint time and every page was screenshotted
    // in fallback fonts. That silently invalidates any comparison against a real
    // browser, since substituted fonts change every metric and line break.
    //
    // Hence: keep going while the network is busy OR the document still knows it
    // is waiting on something.
    let t_net = std::time::Instant::now();
    while (!net_provider.is_empty() || doc.as_ref().has_pending_critical_resources())
        && t_net.elapsed().as_secs() < 20
    {
        std::thread::sleep(std::time::Duration::from_millis(50));
        doc.as_mut().resolve(0.0);
    }
    // One more drain so a font blob that landed after the last poll is registered
    // (which invalidates inline layout) before the final resolve + paint.
    doc.as_mut().handle_messages();
    let net_ms = t_net.elapsed().as_millis();

    let t_layout = std::time::Instant::now();
    // Resolve twice. The first pass builds the CSS animation sets (they are
    // created pending, stamped with this `now`); the second advances the clock
    // well past any plausible entrance animation so they land on their final
    // keyframe.
    //
    // Without it a screenshot captures frame zero, and the very common
    // `animation: fade-in .6s both` pattern renders as a blank page — with
    // `fill-mode: both` an element takes the 0% keyframe (`opacity: 0`) until
    // the animation actually runs. A real browser is a second past that by the
    // time anyone looks at it, so settling is the honest comparison point.
    doc.as_mut().resolve(0.0);
    doc.as_mut().resolve(ANIMATION_SETTLE_SECS);
    let layout_ms = t_layout.elapsed().as_millis();

    // `ZIPP_UICLICK=x,y` sends a real pointer down+up at those client
    // coordinates, so hit-testing, the event driver and the JS bridge all run —
    // the same path a mouse takes. `ZIPP_CLICK` bypasses all of that and only
    // proves the JS half, which is why it once reported success while clicking
    // in the window still did nothing.
    if let Some(spec) = std::env::var("ZIPP_UICLICK").ok() {
        // Semicolon-separated points, clicked in order: `x,y;x,y;...`. A single
        // point is the common case and behaves exactly as before. A SEQUENCE is
        // what you need to reproduce anything that depends on getting somewhere
        // first — following a link and then using a control on the page you
        // landed on cannot be expressed by repeating one coordinate.
        let points: Vec<(f32, f32)> = spec
            .split(';')
            .filter_map(|p| p.split_once(','))
            .filter_map(|(sx, sy)| {
                match (sx.trim().parse::<f32>(), sy.trim().parse::<f32>()) {
                    (Ok(x), Ok(y)) => Some((x, y)),
                    _ => None,
                }
            })
            .collect();
        if !points.is_empty() {
            {
                use zipp_dom::Document;
                use zipp_dom::traits_src::events::{
                    MouseEventButton, MouseEventButtons, PointerCoords, PointerDetails,
                    PointerEvent, PointerId, UiEvent,
                };
                let mk = |x: f32, y: f32| PointerEvent {
                    id: PointerId::Mouse,
                    is_primary: true,
                    coords: PointerCoords {
                        screen_x: x, screen_y: y,
                        client_x: x, client_y: y,
                        page_x: x, page_y: y,
                    },
                    button: MouseEventButton::Main,
                    buttons: MouseEventButtons::Primary,
                    mods: Default::default(),
                    details: PointerDetails::default(),
                };
                let clicks: u32 = std::env::var("ZIPP_UICLICK_N")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1);
                let total = clicks as usize * points.len();
                for i in 0..total {
                    let (x, y) = points[i % points.len()];
                    eprintln!("[UICLICK] click {} at ({x}, {y})", i + 1);
                    doc.handle_ui_event(UiEvent::PointerMove(mk(x, y)));
                    doc.handle_ui_event(UiEvent::PointerDown(mk(x, y)));
                    doc.handle_ui_event(UiEvent::PointerUp(mk(x, y)));
                    // Settle layout between clicks so the next hit-test runs
                    // against the tree the previous click produced.
                    // Two resolves: the first creates the animations the
                    // re-render introduced, the second advances past them, so
                    // the next hit-test runs against settled geometry.
                    doc.as_mut().resolve(ANIMATION_SETTLE_SECS * (2 + i * 2) as f64);
                    doc.as_mut().resolve(ANIMATION_SETTLE_SECS * (3 + i * 2) as f64);
                }
                // Advance PAST the settle time: nodes inserted by the click
                // start their entrance animations at "now", so resolving at the
                // same instant would capture them at opacity 0 and the page
                // would look blank for reasons that have nothing to do with the
                // click.
                // Two frames. The re-render builds BRAND NEW nodes, so their
                // entrance animations are created on the next resolve and
                // stamped with that moment — sampling at the same instant shows
                // every one of them at its 0% keyframe (opacity:0), i.e. a blank
                // page. The second resolve advances past their duration.
                // Past the LAST click's clock, not a fixed multiple: with N
                // clicks the loop above has already advanced to
                // SETTLE*(3+2(N-1)), so a hardcoded 4/8 lands mid-animation for
                // N>=3 and screenshots a half-faded page that looks like a
                // rendering bug.
                let after = ANIMATION_SETTLE_SECS * (2.0 * total as f64 + 2.0);
                // Several frames, spread out. Each re-render builds BRAND NEW
                // nodes whose entrance animations are created on the next
                // resolve and stamped with that moment, and the site staggers
                // them with per-item delays. One extra frame is not enough to
                // clear the longest delay, which is why a late screenshot
                // showed a blank hero and a missing nav item.
                for step in [0.0, 4.0, 12.0, 30.0] {
                    doc.as_mut().resolve(after + ANIMATION_SETTLE_SECS * step);
                }
                if let Some(state) = js_state.as_mut() {
                    if let Ok(JsValue::String(after)) =
                        state.call_global("__getRenderedHTML", &[])
                    {
                        let _ = std::fs::write(format!("{out_path}.click.html"), &after);
                        eprintln!("[UICLICK] wrote {out_path}.click.html");
                    }
                }
            }
        }
    }

    // `ZIPP_SCROLL=<pixels>` scrolls the page before painting, so the scrolled
    // frame can be inspected without a window.
    if let Some(dy) = std::env::var("ZIPP_SCROLL").ok().and_then(|v| v.parse::<f64>().ok()) {
        use zipp_dom::Document;
        use zipp_dom::traits_src::events::{
            MouseEventButtons, PointerCoords, UiEvent, WheelDelta, WheelEvent,
        };
        // Move the pointer first: the wheel is delivered to the hovered node,
        // and without a prior move there is no hover target.
        {
            use zipp_dom::traits_src::events::{
                MouseEventButton, PointerDetails, PointerEvent, PointerId,
            };
            let c = PointerCoords {
                screen_x: (width / 2) as f32, screen_y: (height / 2) as f32,
                client_x: (width / 2) as f32, client_y: (height / 2) as f32,
                page_x: (width / 2) as f32, page_y: (height / 2) as f32,
            };
            doc.handle_ui_event(UiEvent::PointerMove(PointerEvent {
                id: PointerId::Mouse,
                is_primary: true,
                coords: c,
                button: MouseEventButton::Main,
                buttons: MouseEventButtons::empty(),
                mods: Default::default(),
                details: PointerDetails::default(),
            }));
        }
        doc.handle_ui_event(UiEvent::Wheel(WheelEvent {
            delta: WheelDelta::Pixels(0.0, -dy),
            coords: PointerCoords {
                screen_x: (width / 2) as f32, screen_y: (height / 2) as f32,
                client_x: (width / 2) as f32, client_y: (height / 2) as f32,
                page_x: (width / 2) as f32, page_y: (height / 2) as f32,
            },
            buttons: MouseEventButtons::empty(),
            mods: Default::default(),
        }));
        doc.as_mut().resolve(ANIMATION_SETTLE_SECS);
        let vs = doc.as_ref().viewport_scroll();
        eprintln!("[SCROLL] requested {dy}px -> viewport_scroll.y={}", vs.y);
    }

    // `ZIPP_DUMP_TREE=1` prints the resolved Taffy layout tree — the fastest way
    // to tell a cascade problem (wrong computed style) from a layout problem
    // (right style, wrong box).
    if std::env::var_os("ZIPP_DUMP_TREE").is_some() {
        doc.as_ref().print_taffy_tree();
    }

    let t_paint = std::time::Instant::now();
    let mut painter = zipp_renderer::tinyskia::TinySkiaPainter::new(width, height);
    zipp_renderer::paint_scene(&mut painter, doc.as_ref(), 1.0, width, height, 0, 0);
    let paint_ms = t_paint.elapsed().as_millis();

    eprintln!(
        "[PERF] Render: parse={parse_ms}ms net-wait={net_ms}ms layout={layout_ms}ms paint={paint_ms}ms"
    );

    match painter.pixmap.save_png(out_path) {
        Ok(()) => {
            eprintln!("[SHOT] wrote {out_path}");
            true
        }
        Err(e) => {
            eprintln!("[SHOT] failed to write {out_path}: {e}");
            false
        }
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
fn run_zipp_js(
    html: &str,
    page_url: &str,
    viewport: (u32, u32),
) -> Option<(String, String, ScriptState)> {
    // Grow the stack here rather than at each call site. JS recursion itself is
    // safe — zipp-vm keeps JS frames in an explicit Vec, not the native stack —
    // but PARSING and COMPILING a bundle is recursive over the AST on the native
    // stack, and a real page's minified bundle nests deeply enough to blow a
    // default thread stack. The prerender worker used to skip this and crashed
    // the process a few pages in.
    stacker::maybe_grow(64 * 1024 * 1024, 64 * 1024 * 1024, || {
        run_zipp_js_inner(html, page_url, viewport)
    })
}

fn run_zipp_js_inner(
    html: &str,
    page_url: &str,
    viewport: (u32, u32),
) -> Option<(String, String, ScriptState)> {
    let mut external_scripts = Vec::new();
    // `<script type="module">` entries are linked separately: their source is ES
    // module syntax, which is a syntax error in the classic program the rest of
    // the pipeline builds. Before this, the whole bundle was concatenated in as
    // a script and died at `import.meta is only valid in modules`.
    let mut module_entries: Vec<String> = Vec::new();
    let mut inline_modules: Vec<String> = Vec::new();
    let mut pos = 0;
    while let Some(start) = html[pos..].find("<script") {
        let abs = pos + start;
        let tag_end = abs + html[abs..].find('>')? + 1;
        let tag = &html[abs..tag_end];
        let ty = attr(tag, "type").unwrap_or_default();
        let ty = ty.trim().to_ascii_lowercase();
        // A `type` that names a non-JS format (importmap, application/json,
        // text/template) is data, not code, and must not be executed.
        let is_js = ty.is_empty()
            || ty == "module"
            || ty == "text/javascript"
            || ty == "application/javascript";
        if let Some(src) = attr(tag, "src") {
            let full = resolve(page_url, &src);
            if ty == "module" {
                module_entries.push(full);
            } else if is_js {
                if let Ok(source) = fetch(&full) {
                    external_scripts.push((full, source));
                }
            }
        } else if is_js {
            // INLINE script. These were skipped entirely, so a page whose
            // behaviour lives in a `<script>` block ran none of it — the tag was
            // parsed for `src`, found none, and moved on.
            let body_start = tag_end;
            if let Some(close) = html[body_start..].find("</script") {
                let code = &html[body_start..body_start + close];
                if !code.trim().is_empty() {
                    if ty == "module" {
                        // An inline module still needs linking; give it a URL of
                        // its own so relative specifiers resolve against the page.
                        inline_modules.push(code.to_string());
                    } else {
                        external_scripts
                            .push((format!("{page_url}#inline{abs}"), code.to_string()));
                    }
                }
                pos = body_start + close;
                continue;
            }
        }
        pos = tag_end;
    }
    // An inline module is rare and needs the linker; run it as a classic script
    // when it has no import/export, which is the common case for a bootstrap
    // snippet, and report it otherwise rather than silently dropping it.
    for code in inline_modules {
        if code.contains("import ") || code.contains("export ") || code.contains("import(") {
            eprintln!("[JS] inline <script type=module> with imports is not linked yet");
        } else {
            external_scripts.push((format!("{page_url}#inline-module"), code));
        }
    }
    if external_scripts.is_empty() && module_entries.is_empty() {
        return None;
    }

    let base = url::Url::parse(page_url)
        .ok()
        .map(|u| format!("{}://{}", u.scheme(), u.host_str().unwrap_or("")))
        .unwrap_or_default();

    let dom_shim = include_str!("dom_shim.js");
    let router_helpers = include_str!("router_helpers.js");
    let dom_shim = &format!("{HOST_PRELUDE_JS}\n{dom_shim}");
    let shim = dom_shim.replace(
        "__PAGE_URL__",
        &page_url.replace('\\', "\\\\").replace('\'', "\\'"),
    );
    // The page's JS must see the SAME viewport the CSS is laid out against, or
    // a responsive app that branches on `window.innerWidth` renders for one
    // width while the cascade uses another.
    let shim = shim
        .replace("__VIEWPORT_W__", &viewport.0.to_string())
        .replace("__VIEWPORT_H__", &viewport.1.to_string())
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
        // `ZIPP_JS_FROM` / `ZIPP_JS_TO` rewrite the page's own source before it
        // is compiled. Page bundles are minified with no source map, so when a
        // handler misbehaves the only way to see inside it is to edit it.
        // Harmless when unset.
        let patched = match (
            std::env::var("ZIPP_JS_FROM").ok(),
            std::env::var("ZIPP_JS_TO").ok(),
        ) {
            (Some(from), Some(to)) if patched.contains(&from) => {
                eprintln!("[JS] substituted {} bytes of page source", from.len());
                patched.replace(&from, &to)
            }
            _ => patched,
        };
        full.push('\n');
        full.push_str(&patched);
        full.push('\n');
    }
    // Link and append the ES module graph.
    //
    // Every reachable module is fetched and rewritten into calls on a registry,
    // then emitted as one classic script. It has to happen up front: our host
    // HTTP is synchronous and there is no way to fetch from inside the VM part
    // way through execution, so a chunk that is not linked here could never be
    // loaded at all — including the lazily `import()`ed route chunks.
    if !module_entries.is_empty() {
        let t_link = std::time::Instant::now();
        let linked = esm::link_graph(&module_entries, &fetch, &resolve);
        eprintln!(
            "[ESM] Linked {} modules from {} entr{} in {}ms",
            linked.module_count,
            module_entries.len(),
            if module_entries.len() == 1 { "y" } else { "ies" },
            t_link.elapsed().as_millis(),
        );
        for (url, err) in &linked.failures {
            eprintln!("[ESM] FAILED {url}: {err}");
        }
        if std::env::var_os("ZIPP_DEBUG_ESM").is_some() {
            full.push_str("
var __zippEsmDebug=true;
");
        }
        full.push_str(esm::MODULE_RUNTIME_JS);
        full.push_str(&linked.source);
        // `ZIPP_DUMP_LINKED=<path>` writes the linked graph out. A rewrite bug
        // shows up as a runtime error deep inside minified code, and the only
        // way to read it is to look at what was actually emitted.
        if let Ok(path) = std::env::var("ZIPP_DUMP_LINKED") {
            let _ = std::fs::write(&path, &linked.source);
            eprintln!("[ESM] wrote linked graph to {path}");
        }
    }

    // Settle the page: run timers and microtasks until nothing is left to do,
    // so the tree we serialise below is the one the page actually intended.
    full.push_str("\n__zippDrainQueues(100);\n");
    full.push_str(
        "\nif (typeof __zippEsmDebug !== 'undefined' && __zippEsmDebug) { \
         env.log('info','drain rounds='+__zippDrainStats.rounds+' timers='+__zippDrainStats.timers+\
         ' micro='+__zippDrainStats.micro); }\n",
    );

    eprintln!("[JS] Compiling {} bytes...", full.len());
    let mut state = match compile_script(&full) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[JS] Compile error: {e}");
            return None;
        }
    };
    state.set_host_call(Box::new(browser_host_call));
    if let Err(e) = state.run_init() {
        // An uncaught throw is not fatal: the shim may still have built enough
        // of a tree to render, so fall through and check. This matches how a
        // real browser treats a script error — the page is what it is.
        eprintln!("[JS] Uncaught: {e}");
    }
    // Settle the VM's microtasks against the shim's queues.
    //
    // These are two SEPARATE queues. `__zippDrainQueues` runs the shim's timers
    // and MessageChannel callbacks; the continuation of an `await` lives in the
    // VM's own microtask queue and is only drained from here. Draining one feeds
    // the other — an awaited fetch resolves in the VM, its continuation sets
    // state, React schedules a re-render through MessageChannel — so they have
    // to alternate until both are quiet.
    //
    // Without this a client-rendered app is serialised mid-boot: formlogic.local
    // fetched its session, got its 401, and would have rendered the logged-out
    // UI, but the tree was read before the continuation that sets
    // `isInitialized` ever ran, so the page was captured on its loading screen.
    for _ in 0..32 {
        state.run_microtasks();
        let _ = state.call_global("__zippDrainQueues", &[JsValue::Number(50.0)]);
    }
    for line in state.take_output() {
        eprintln!("[JS] {line}");
    }
    for line in state.take_errput() {
        eprintln!("[JS] {line}");
    }

    if let Ok(JsValue::String(rendered)) = state.eval_in_context("__getRenderedHTML()") {
        if !rendered.is_empty() {
            // Extract the event handler map before we lose the state.
            let event_map_json = match state.eval_in_context("__getEventMap()") {
                Ok(JsValue::String(s)) => s,
                _ => "{}".to_string(),
            };

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
            // `ZIPP_DUMP_ROOT=<path>` writes what the page's JS actually built.
            // For a client-rendered app this IS the page; without it a short
            // render is indistinguishable from a blank one.
            if let Ok(path) = std::env::var("ZIPP_DUMP_ROOT") {
                let _ = std::fs::write(&path, &inner);
            }
            // Whatever the page put on <html> — the theme class above all.
            // Its CSS is built on `:root.dark`, so this has to land on the real
            // document element, not merely somewhere inside the body.
            let html_attrs = match state.eval_in_context("__getHtmlClass()") {
                Ok(JsValue::String(a)) if !a.trim().is_empty() => format!(" {a}"),
                _ => String::new(),
            };
            let html_out = format!(
                "<!DOCTYPE html><html{html_attrs}><head>{head}</head><body>{inner}</body></html>"
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

/// Records navigations instead of performing them, for `--screenshot`.
///
/// The screenshot path had NO navigation provider at all, so a link click could
/// never trigger a real navigation there — which meant the harness happily
/// reported the SPA working while the windowed browser threw its JS state away
/// on the first link click and left the next page dead. A harness that cannot
/// reach the broken path is worse than none: it reports success.
struct RecordingNavProvider;

impl NavigationProvider for RecordingNavProvider {
    fn navigate_to(&self, options: NavigationOptions) {
        eprintln!("[NAV-WOULD] {}", options.url);
    }
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
                    run_zipp_js(&html, &url, INITIAL_WINDOW_SIZE)
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
/// Styles for the injected browser chrome.
///
/// Deliberately TINY. The toolbar's own markup is styled entirely with inline
/// `style` attributes, so nothing here is needed to draw it — and anything here
/// leaks into the page, because the chrome is injected into the page's own
/// document and therefore shares its CSS namespace.
///
/// This used to carry ~60 `!important` rules redefining generic Tailwind utility
/// names (`.grid`, `.flex`, `.w-full`, `.text-4xl`, `.rounded-lg`, `.shadow`, …)
/// under the heading "CSS Grid not fully supported in layout engine". That was
/// the same false premise as the old `responsive` module: Taffy implements grid
/// and it works. What the block actually did was override the design system of
/// every Tailwind site with `!important` — `.grid{display:flex}` alone turned
/// every CSS Grid on the page into a flex row, which is why lance.name's project
/// cards stacked instead of sitting three across.
///
/// Rule of thumb for anything added here: if a page could plausibly use the same
/// selector, it does not belong.
const TOOLBAR_CSS: &str = r#"
/* The chrome wrapper is a full-height flex column, so the root element must not
   impose a width or centre itself. `body` is reset inline on the element. */
html { max-width:none !important; width:100% !important; margin:0 !important; padding:0 !important; }
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

    // Keep the source document's <html> attributes. This function rebuilds the
    // document from its head and body, so emitting a bare <html> silently threw
    // away the theme class that was just resolved.
    let html_attrs = html
        .find("<html")
        .and_then(|i| html[i + 5..].find('>').map(|e| html[i + 5..i + 5 + e].trim().to_string()))
        .filter(|a| !a.is_empty())
        .map(|a| format!(" {a}"))
        .unwrap_or_default();

    format!(
        r#"<!DOCTYPE html><html{html_attrs}><head>
<meta charset="utf-8">
{head_content}
<style>{TOOLBAR_CSS}</style>
</head><body style="margin:0 !important;padding:0 !important;max-width:none !important;width:100% !important">{body}</body></html>"#,
        html_attrs = html_attrs,
        head_content = head_content,
        body = toolbar_body(body_content, current_url),
    )
}

/// Rebuild the document body after a JS re-render.
///
/// `__getRenderedHTML()` hands back the page's own `<body>` element. Strip that
/// wrapper, re-absolutise its URLs against the page (the initial load does this;
/// skipping it broke every relative image on the first click), and put the
/// browser chrome back around it.
pub(crate) fn rerender_body(rendered_body: &str, page_url: &str) -> String {
    rerender_body_themed(rendered_body, page_url, "")
}

/// `rerender_body`, carrying the class the page put on `<html>`.
pub(crate) fn rerender_body_themed(
    rendered_body: &str,
    page_url: &str,
    html_class: &str,
) -> String {
    let inner = match (rendered_body.find("<body"), rendered_body.rfind("</body>")) {
        (Some(start), Some(end)) => {
            let open_end = rendered_body[start..]
                .find('>')
                .map(|e| start + e + 1)
                .unwrap_or(start);
            &rendered_body[open_end..end]
        }
        _ => rendered_body,
    };
    let base = url::Url::parse(page_url)
        .ok()
        .map(|u| format!("{}://{}", u.scheme(), u.host_str().unwrap_or("")))
        .unwrap_or_default();
    toolbar_body_themed(&make_urls_absolute(inner, &base), page_url, html_class)
}

/// The `<body>` contents: the browser chrome followed by the page.
///
/// The wrapper uses `min-height:100vh`, NOT `height:100vh`. A fixed height pins
/// `<html>` and `<body>` to the window while the page overflows them, which is
/// not how any real document behaves — the root grows to its content. Two things
/// broke because of it:
///
///  - the document had no scrollable range of its own (`content_size` lived on a
///    descendant), so scrolling had to be faked by an element scroller; and
///  - a page that sets `overflow-x:hidden` on `body` (common, and what
///    lance.name does) turned `body` into a clipper whose box is only one
///    viewport tall. Once that box scrolled up, it clipped everything below —
///    the page appeared to stop rendering partway down.
///
/// The trade-off: the toolbar is in normal flow, so it scrolls away with the
/// page (`position:sticky` is inert here — the Stylo→Taffy conversion maps
/// Sticky to Relative). Real chrome belongs outside the document rather than
/// injected into it; until that happens, a correctly-rendering page is worth
/// more than a pinned URL bar.
///
/// Factored out because a JS re-render replaces the WHOLE body — it is the only
/// element the event handler can address — so the chrome has to be rebuilt with
/// it. Without this a single click removed the toolbar and dropped the page out
/// of the flex column that gives it its height.
fn toolbar_body(page_html: &str, current_url: &str) -> String {
    toolbar_body_themed(page_html, current_url, "")
}

/// `toolbar_body`, carrying the class the page put on `<html>`.
///
/// The class goes on the injected wrapper rather than the real `<html>`: the
/// wrapper is an ancestor of every page node, which is all a `.dark .x`
/// selector needs, and it survives the body swap that a re-render performs.
fn toolbar_body_themed(page_html: &str, current_url: &str, html_class: &str) -> String {
    // Already a rendered attribute list (`class="dark" data-theme="dark"`).
    let class_attr = if html_class.trim().is_empty() {
        String::new()
    } else {
        format!(" {html_class}")
    };
    // The page goes inside its own block container rather than straight into the
    // flex column. As a direct child of the column, every TOP-LEVEL page element
    // became a flex item, which silently changed the page's layout semantics:
    // `align-items:stretch` overrode the intrinsic and attribute-derived sizes of
    // replaced elements (an `<img width=64>` came out viewport-wide), each one
    // established its own BFC, and adjacent margins stopped collapsing. None of
    // that is how the element would behave in the document the author wrote.
    format!(
        r#"<div{class_attr} style="display:flex;flex-direction:column;margin:0;padding:0;width:100%;min-height:100vh">
  <form action="{current_url}" method="GET" style="color:#212529;height:42px;min-height:42px;background:#f8f9fa;border-bottom:1px solid #dee2e6;display:flex;align-items:center;padding:4px 8px;gap:6px;font-family:Arial,sans-serif;box-sizing:border-box;width:100%;margin:0;flex-shrink:0">
    <span style="font-size:16px;color:#495057;padding:0 4px;cursor:pointer">◀</span>
    <span style="font-size:16px;color:#495057;padding:0 4px;cursor:pointer">▶</span>
    <input type="text" name="__nav_url" value="{current_url}" style="width:90%;height:32px;padding:0 12px;border:1px solid #ced4da;border-radius:8px;font-size:14px;background:white;color:#212529;outline:none;box-sizing:border-box" />
  </form>
  <div style="display:block;flex:1 1 auto;width:100%;margin:0;padding:0;min-width:0">
{page_html}
  </div>
</div>"#
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
    if s.starts_with("http://") || s.starts_with("https://") || s.starts_with("file://") {
        return s.to_string();
    }
    // A local path, with or without a drive letter: treat it as a file URL
    // rather than guessing it is a search term.
    if std::path::Path::new(s).is_absolute() && std::path::Path::new(s).exists() {
        return format!("file:///{}", s.replace('\\', "/"));
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
