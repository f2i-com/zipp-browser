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
use zipp_vm::embed::{compile_script, HostCtx, JsValue, ScriptState};

// The JS compiler front end allocates heavily -- a HashSet<String> per scope on
// every capture walk, Vec<(String, Reg)> scope chains -- so the allocator is on
// the hot path for page load, not just for running code. ZIPP adopted
// mimalloc for the same workload.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;
use zipp_dom::html_parser::{HtmlDocument, HtmlProvider};
use zipp_dom::traits_src::navigation::{NavigationOptions, NavigationProvider};
use zipp_dom::{BaseDocument, DocumentConfig, FontContext};
use zipp_renderer::tinyskia::TinySkiaWindowRenderer;
use zipp_shell::{create_default_event_loop, ZippApp, ShellProxy, WindowConfig};

mod esm;
mod event_bridge;
mod seed;
mod site_storage;
#[cfg(windows)]
mod media;
mod audio;
mod canvas;
#[cfg(target_os = "linux")]
mod pty;
#[cfg(target_os = "linux")]
mod wayland;
mod system;
#[cfg(target_os = "linux")]
mod usb;
mod canvas2d;
mod console;
mod worker;
#[cfg(any(windows, target_os = "linux"))]
mod webgl;
#[cfg(any(windows, target_os = "linux"))]
mod webgpu;
mod viewers;

// ---------------------------------------------------------------------------
// Global caches
// ---------------------------------------------------------------------------

lazy_static::lazy_static! {
    /// Fetched URL content (JS bundles, CSS, HTML).
    static ref FETCH_CACHE: Mutex<HashMap<String, String>> = Mutex::new(HashMap::new());
    /// Inlined images (URL -> base64 data URI).
    static ref IMAGE_CACHE: Mutex<HashMap<String, String>> = Mutex::new(HashMap::new());
    /// The URL of the page currently loaded, so a relative `window.open` or
    /// `location.assign` from the page can be resolved against it.
    static ref CURRENT_URL: Mutex<String> = Mutex::new(String::new());
    /// Fully rendered HTML keyed by URL (skip JS re-compilation on revisit).
    /// Pages as fetched, before their JS runs: what a navigation to the same URL
    /// starts from instead of the network. It holds the fetched page rather than
    /// the rendered one so that a revisited page still gets a live script and an
    /// event bridge, not a frozen snapshot.
    static ref PAGE_CACHE: Mutex<HashMap<String, String>> = Mutex::new(HashMap::new());
    /// Shared blocking HTTP client -- reused across all fetch() calls to
    /// take advantage of connection pooling and avoid per-request TLS setup.
    static ref HTTP_CLIENT: reqwest::blocking::Client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .danger_accept_invalid_certs(true)
        .user_agent(USER_AGENT)
        // Every `fetch()` the page's JS makes goes through this client. Without
        // a cookie store it discarded every `Set-Cookie` the server sent, so a
        // login POST succeeded and the session it established was thrown away
        // immediately -- the next request went out unauthenticated and the app
        // decided the user was not logged in. Nothing about that looks like a
        // network failure from inside the page, which is why it surfaced as a
        // dashboard 404 rather than an error. It is the browser's one jar,
        // which the page's images and styles, and `document.cookie`, share.
        .cookie_provider(zipp_shell::cookies::jar())
        .build()
        .expect("failed to build HTTP client");
}

/// Viewport the page's JavaScript is told about on the windowed path.
///
/// The window does not exist yet when the page's script runs, so this is the
/// size the shim reports for `window.innerWidth`/`innerHeight` and answers
/// `matchMedia` against — and the window opens at it, so that JS and CSS agree
/// on load. It used to open at the platform default (800x600) while the
/// script was told 1280x800: a page branching on a breakpoint in JS chose one
/// layout while its CSS chose another. CSS is laid out against the real window
/// (the Stylo device is rebuilt from it, and again on every resize).
const INITIAL_WINDOW_SIZE: (u32, u32) = (1280, 800);

/// Seconds to advance the animation clock before a headless screenshot, so CSS
/// entrance animations land on their final keyframe rather than frame zero.
const ANIMATION_SETTLE_SECS: f64 = 5.0;

/// Customisable User-Agent string. Change this to mimic different browsers.
/// It names the platform the browser runs on, as Chrome's does.
#[cfg(not(target_os = "linux"))]
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Zipp/1.0";
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux aarch64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Zipp/1.0";
#[cfg(all(target_os = "linux", not(target_arch = "aarch64")))]
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Zipp/1.0";

/// A shim with the platform it is told about made this one's: the shims say
/// Windows (`navigator.userAgent`, `navigator.platform`), as `USER_AGENT` does
/// there.
pub(crate) fn platform_shim(js: &str) -> String {
    #[cfg(target_os = "linux")]
    {
        let platform = if cfg!(target_arch = "aarch64") { "Linux aarch64" } else { "Linux x86_64" };
        js.replace("Windows NT 10.0; Win64; x64", &format!("X11; {platform}"))
            .replace("platform:\"Win32\"", &format!("platform:\"{platform}\""))
            .replace("platform: \"Win32\"", &format!("platform: \"{platform}\""))
    }
    #[cfg(not(target_os = "linux"))]
    js.to_string()
}

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
        save_profile();
        std::process::exit(if ok { 0 } else { 1 });
    }
    // WPT reftests, many in one process: `tools/wpt_reftest.py` drives it.
    //   zipp-browser --reftest pairs.tsv results.tsv
    if arg == "--reftest" {
        let mut rest = std::env::args().skip(2);
        let (Some(pairs), Some(results)) = (rest.next(), rest.next()) else {
            eprintln!("usage: zipp-browser --reftest pairs.tsv results.tsv");
            std::process::exit(2);
        };
        let ok = reftest_batch(&pairs, &results);
        save_profile();
        std::process::exit(if ok { 0 } else { 1 });
    }

    // `--kiosk <url>`: the page is the whole screen, with no toolbar. On Linux
    // the screen is taken through DRM/KMS with input from evdev (zipp-kms), so
    // no display server is needed: zipp-linux runs the browser this way. The
    // screen is opened first because its size is the viewport the page's
    // script is told about on its first run.
    let kiosk = arg == "--kiosk";
    let arg = if kiosk { std::env::args().nth(2).unwrap_or_default() } else { arg };
    #[cfg(not(target_os = "linux"))]
    if kiosk {
        eprintln!("[BROWSER] --kiosk runs on Linux's DRM/KMS only");
        std::process::exit(2);
    }
    #[cfg(target_os = "linux")]
    let kms = kiosk.then(open_kms);
    #[cfg(target_os = "linux")]
    let viewport = kms.as_ref().map_or(INITIAL_WINDOW_SIZE, |kms| {
        let (w, h) = kms.size();
        ((w as f64 / kms.scale_factor()) as u32, (h as f64 / kms.scale_factor()) as u32)
    });
    #[cfg(not(target_os = "linux"))]
    let viewport = INITIAL_WINDOW_SIZE;
    if kiosk {
        NO_CHROME.store(true, std::sync::atomic::Ordering::Relaxed);
        system::KIOSK.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    let url = if arg.is_empty() { "https://lance.name".to_string() } else { arg };
    eprintln!("[BROWSER] Loading {url}");
    // The window's page runs on this thread: its console is the one shown,
    // and it may start workers.
    console::record_this_thread();
    worker::allow_on_this_thread();

    // 1-3. Fetch, run the page's JS, post-process. The ScriptState is !Send, so
    //      this stays on the main thread to keep it alive for event dispatch.
    let t_fetch = std::time::Instant::now();
    let mut fetched = viewers::fetch_document(&url, true);
    // A kiosk starts with its network: a page on the web may not be
    // reachable yet (no address, or no clock for TLS until NTP), so it is
    // asked again for two minutes before the error is what shows.
    while kiosk && url.starts_with("http") && t_fetch.elapsed() < std::time::Duration::from_secs(120) {
        let Err(e) = &fetched else { break };
        eprintln!("[BROWSER] {url}: {e}; trying again");
        std::thread::sleep(std::time::Duration::from_secs(3));
        fetched = viewers::fetch_document(&url, true);
    }
    let html = match fetched {
        Ok(html) => {
            PAGE_CACHE.lock().unwrap().insert(url.clone(), html.clone());
            html
        }
        Err(e) => error_page(&url, &e),
    };
    let fetch_ms = t_fetch.elapsed().as_millis();
    let (final_html, event_map_json, js_state) =
        render_page(&url, &html, viewport, Some(fetch_ms));
    eprintln!("[BROWSER] Ready ({} bytes)", final_html.len());

    // 4. Load fonts manually (system font API doesn't work on RDP)
    let font_ctx = load_fonts();

    // 5. Set up async runtime for network fetches (CSS, images)
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let _guard = rt.enter();

    // 6. Create window and net provider
    #[cfg(target_os = "linux")]
    let screen = match kms {
        Some(kms) => Screen::Kiosk(kms),
        None => Screen::Window(create_default_event_loop()),
    };
    #[cfg(not(target_os = "linux"))]
    let screen = Screen::Window(create_default_event_loop());
    let (proxy, receiver) = ShellProxy::new(screen.create_proxy());
    // A worker that has sent its page something wakes the window, whose event
    // bridge hands it over (`JsEventBridge::run_tasks`).
    let wake_proxy = std::sync::Mutex::new(proxy.clone());
    worker::set_waker(Box::new(move || {
        if let Ok(proxy) = wake_proxy.lock() {
            proxy.send_event(zipp_shell::ShellEvent::Embedder(Arc::new(())));
        }
    }));
    let net_provider = Arc::new(zipp_shell::net::Provider::new(Some(Arc::new(
        proxy.clone(),
    ))));

    // 7. Create Zipp document with fonts + networking + navigation
    let nav_provider = Arc::new(BrowserNavProvider {
        proxy: proxy.clone(),
        current_url: std::sync::Mutex::new(url.clone()),
        history: std::sync::Mutex::new(History::new(&url)),
    });
    // Published so `browser_host_call` can drive a navigation the PAGE asked
    // for (`window.open`, `location.assign`) -- it is a free function with no
    // other route to the shell.
    let _ = NAV_PROVIDER.set(nav_provider.clone() as Arc<dyn NavigationProvider>);
    let config = DocumentConfig {
        font_ctx: Some(font_ctx),
        base_url: Some(url.clone()),
        net_provider: Some(net_provider as _),
        navigation_provider: Some(nav_provider as _),
        html_parser_provider: Some(Arc::new(HtmlProvider) as _),
        // Videos play in the window only; screenshots and tests show posters.
        // Pages reached by navigation inherit it with the rest of the config.
        #[cfg(windows)]
        media_provider: Some(Arc::new(media::MediaFoundationProvider::new()) as _),
        ..Default::default()
    };
    let mut doc = build_document(&final_html, config);
    attach_event_bridge(&mut doc, event_map_json, js_state, &url);

    // Pre-render linked pages in background for instant navigation
    let prerender_url = url.clone();
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || prerender_linked_pages(&final_html, &prerender_url))
        .ok();

    // 8. Open window with tiny-skia renderer
    let mut app = ZippApp::new(proxy, receiver);
    // Navigations come back here, on the main thread, to be built: the page's
    // JS runs and its event bridge is attached exactly as for the first page.
    // The fetching thread can do neither -- the script state is not `Send` --
    // which is why every page reached by a link used to render and then
    // respond to nothing.
    app.set_document_builder(Box::new(|url, html, config, viewport| {
        console::clear();
        worker::new_page();
        system::new_page();
        #[cfg(target_os = "linux")]
        usb::new_page();
        let (final_html, event_map_json, js_state) = render_page(&url, &html, viewport, None);
        let mut doc = build_document(&final_html, config);
        attach_event_bridge(&mut doc, event_map_json, js_state, &url);
        Box::new(doc)
    }));
    let renderer = TinySkiaWindowRenderer::new();
    let attributes = winit::window::WindowAttributes::default()
        .with_surface_size(winit::dpi::LogicalSize::new(INITIAL_WINDOW_SIZE.0, INITIAL_WINDOW_SIZE.1))
        .with_window_icon(app_icon(false));
    #[cfg(windows)]
    let attributes = attributes.with_platform_attributes(Box::new(
        winit::platform::windows::WindowAttributesWindows::default().with_taskbar_icon(app_icon(true)),
    ));
    let window = WindowConfig::with_attributes(Box::new(doc) as _, renderer, attributes);
    app.add_window(window);

    eprintln!("[BROWSER] Window opened");
    // Run the whole event loop on a grown stack. Layout, hit-testing and paint
    // all recurse over the DOM on the native stack, and a real page's tree is
    // deep enough to overflow the default main-thread stack — a server-rendered
    // React app nests far further than the hand-written test pages did. Growing
    // here covers every callback the loop dispatches, including the re-render
    // that follows a JS event handler.
    stacker::maybe_grow(64 * 1024 * 1024, 256 * 1024 * 1024, || {
        screen.run_app(app);
    });
    save_profile();
}

/// What the browser's window runs on: the desktop's windowing system, or
/// (`--kiosk`, Linux) the screen itself.
enum Screen {
    Window(zipp_shell::EventLoop),
    #[cfg(target_os = "linux")]
    Kiosk(zipp_kms::EventLoop),
}

impl Screen {
    fn create_proxy(&self) -> zipp_shell::EventLoopProxy {
        match self {
            Screen::Window(event_loop) => event_loop.create_proxy(),
            #[cfg(target_os = "linux")]
            Screen::Kiosk(event_loop) => event_loop.create_proxy(),
        }
    }

    fn run_app(self, app: ZippApp<TinySkiaWindowRenderer>) {
        match self {
            Screen::Window(event_loop) => event_loop.run_app(app).unwrap(),
            #[cfg(target_os = "linux")]
            Screen::Kiosk(event_loop) => event_loop.run_app(app).unwrap(),
        }
    }
}

/// The screen for `--kiosk`: the first DRM device with a connected output.
#[cfg(target_os = "linux")]
fn open_kms() -> zipp_kms::EventLoop {
    zipp_kms::EventLoop::new().unwrap_or_else(|e| {
        eprintln!("[BROWSER] cannot take the screen: {e}");
        std::process::exit(1);
    })
}

/// Before the browser exits: write out what is waiting to be saved to the
/// profile -- cookies, and what sites keep.
fn save_profile() {
    zipp_shell::cookies::flush();
    site_storage::flush();
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
    let dom_shim = &platform_shim(&format!("{}\n{}", include_str!("console_shim.js"), include_str!("dom_shim.js")));
    let shim = dom_shim
        .replace("__PAGE_URL__", "http://localhost/test")
        .replace("__PAGE_TITLE__", "")
        .replace("__VIEWPORT_W__", "1280")
        .replace("__VIEWPORT_H__", "800")
        .replace("__BODY_HTML__", "<div id=\\'root\\'></div>");

    let full = format!(
        "{HOST_PRELUDE_JS}\n{shim}\n{}\n{}\n{tests}",
        include_str!("clone.js"),
        include_str!("indexeddb_shim.js")
    );
    eprintln!("[{tag}] Compiling {} bytes...", full.len());
    let mut state = match compile_script(&full) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[{tag}] Compile error: {e}");
            return false;
        }
    };
    state.set_host_call_ctx(Box::new(browser_host_call_ctx));
    eprintln!("[{tag}] Executing...");
    if let Err(e) = state.run_init() {
        eprintln!("[{tag}] Uncaught: {e}");
    }
    // Asynchronous tests (`A`) finish in the page's event loop.
    event_bridge::run_page_tasks(&mut state, 100, 0, std::time::Duration::from_secs(10));

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
    let dom_shim = &platform_shim(&format!("{}\n{}", include_str!("console_shim.js"), include_str!("dom_shim.js")));
    let shim = dom_shim
        .replace("__PAGE_URL__", "http://localhost/test")
        .replace("__PAGE_TITLE__", "")
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
    state.set_host_call_ctx(Box::new(browser_host_call_ctx));
    if let Err(e) = state.run_init() {
        eprintln!("[VM-TEST] Uncaught: {e}");
    }
    match state.eval_in_context("result") {
        Ok(JsValue::String(s)) => eprintln!("[VM-TEST] {s}"),
        Ok(other) => eprintln!("[VM-TEST] Unexpected result type: {other:?}"),
        Err(e) => eprintln!("[VM-TEST] Could not read result: {e}"),
    }
}

/// Whether a link plausibly leads to another page rather than a resource.
///
/// Every `href` used to qualify, so prerendering fetched the page's own
/// stylesheet, its favicon and a linked PDF as if each were a document — and
/// ran the "page's" JS over them. A path whose last segment carries a file
/// extension other than a page-like one is a resource.
fn looks_like_page(href: &str) -> bool {
    let Ok(url) = url::Url::parse(href) else {
        return false;
    };
    let last = url.path().rsplit('/').next().unwrap_or("");
    match last.rsplit_once('.') {
        None => true,
        Some((_, ext)) => matches!(
            ext.to_ascii_lowercase().as_str(),
            "html" | "htm" | "xhtml" | "shtml" | "php" | "asp" | "aspx" | "jsp"
        ),
    }
}

/// Prefetch the same-origin pages `html` links to into `PAGE_CACHE`, so that
/// following one starts without a network round trip.
///
/// Only the fetch: the page's JS runs when it is navigated to, on the main
/// thread, so it arrives live. This used to run every linked page's script in
/// the background to cache a rendered snapshot -- a frozen one, which is what a
/// cached page then showed.
fn prerender_linked_pages(html: &str, base_url: &str) {
    let mut links = Vec::new();
    let mut pos = 0;
    while let Some(idx) = html[pos..].find("href=\"") {
        let start = pos + idx + 6;
        if let Some(end) = html[start..].find('"') {
            let href = &html[start..start + end];
            if href.starts_with("http") && looks_like_page(href) && !links.contains(&href.to_string()) {
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
        if !link.starts_with(&base) || PAGE_CACHE.lock().unwrap().contains_key(link) {
            continue;
        }
        eprintln!("[PRERENDER] {link}");
        // Downloads off: a background prefetch must never write a file.
        if let Ok(page) = viewers::fetch_document(link, false) {
            PAGE_CACHE.lock().unwrap().insert(link.clone(), page);
        }
    }
    eprintln!("[PRERENDER] Done -- {} pages cached", PAGE_CACHE.lock().unwrap().len());
}

// ---------------------------------------------------------------------------
// Font loading
// ---------------------------------------------------------------------------

/// The paper airplane (`assets/zipp.ico`, drawn by `assets/make_icon.py`).
///
/// On Windows it comes from the .exe's own resources, where `build.rs` puts
/// it, at the size Windows asks for: small for the title bar, big for the
/// taskbar and Alt+Tab. Elsewhere, from the PNG.
fn app_icon(big: bool) -> Option<winit::icon::Icon> {
    #[cfg(windows)]
    {
        use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXICON, SM_CXSMICON};
        // SAFETY: reads a system metric; no pointers involved.
        let px = unsafe { GetSystemMetrics(if big { SM_CXICON } else { SM_CXSMICON }) }.max(16) as u32;
        winit::platform::windows::WinIcon::from_resource(1, Some(winit::dpi::PhysicalSize::new(px, px)))
            .ok()
            .map(Into::into)
    }
    #[cfg(not(windows))]
    {
        let _ = big;
        let rgba = image::load_from_memory(include_bytes!("../assets/zipp.png")).ok()?.into_rgba8();
        let (width, height) = rgba.dimensions();
        winit::icon::RgbaIcon::new(rgba.into_raw(), width, height).ok().map(Into::into)
    }
}

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
        let path = viewers::local_path(path);
        return std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()));
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

// Bytes from the host. A binary host call (`browser_host_call_ctx`) copies
// into this array, found by name, so it must be a top-level binding; it grows
// by doubling, as the engine keeps every array it has copied into pinned.
var __zippHostBytes = new Uint8Array(0);
function __zippFetchBytes(url) {
    var info = JSON.parse(__zippHostCall("http_bytes", url));
    if (__zippHostBytes.length < info.len) {
        __zippHostBytes = new Uint8Array(Math.max(info.len, __zippHostBytes.length * 2));
    }
    __zippHostCall("take_bytes");
    return __zippHostBytes.slice(0, info.len).buffer;
}

// Run the shim's timer and microtask queues to quiescence, or until `rounds`
// passes have gone by. React's setState is asynchronous, so a click handler
// has typically only QUEUED the re-render when it returns; without this the
// page would be read back before the update landed. Bounded because a page
// can legitimately reschedule forever (a spinner, a poll loop).
var __zippDrainStats = { rounds: 0, timers: 0, micro: 0 };
function __zippDrainQueues(rounds, frames) {
    // Returns how many callbacks ran, so the host can stop settling once the
    // page is genuinely quiet instead of always burning a fixed number of
    // rounds.
    var did = 0;
    // Up to `frames` animation frames run too, once the other queues are
    // empty, so that work a page defers to `requestAnimationFrame` (a first
    // WebGL frame, a menu opening after a click) lands before the page is read
    // back. They do not count as work done: a page that redraws every frame
    // would otherwise never settle. Its later frames come from the window
    // (`__zippAnimationFrame`).
    frames = frames || 0;
    for (var i = 0; i < rounds; i++) {
        __zippDueTimers();
        if (__timerQueue.length === 0 && __mcQueue.length === 0) {
            if (__rafQueue.length === 0 || frames <= 0) break;
            frames--;
            __zippRunAnimationFrame(performance.now());
            continue;
        }
        // Report, never swallow. A page's real work happens in these callbacks
        // — React commits its first render from one — so an empty catch here
        // turns a broken app into a blank page with no explanation.
        __zippDrainStats.rounds++;
        while (__timerQueue.length > 0) {
            __zippDrainStats.timers++;
            did++;
            try { __timerQueue.shift()(); }
            catch (e) { env.log('error', 'timer callback threw: ' + ((e && (e.stack || e.message)) || e)); }
        }
        while (__mcQueue.length > 0) {
            __zippDrainStats.micro++;
            did++;
            try { __mcQueue.shift()(); }
            catch (e) { env.log('error', 'microtask threw: ' + ((e && (e.stack || e.message)) || e)); }
        }
    }
    return did;
}

// One task of the page's event loop: a due timer or a posted message
// (MessageChannel, an IntersectionObserver report) -- the two taking turns --
// else, when `allowFrame`, an animation frame. The host (`run_page_tasks`) runs
// the VM's microtasks after each one, as a browser does after every task, so an
// `await` continuation or a `.then` reaction runs before the next timer rather
// than after the whole batch. Returns 0 when nothing was due, 1 after a task, 2
// after a frame.
//
// Turns, because timers always went first: an app animating on a timer chain
// whose every step outlasts its delay always has one due, and React, which
// renders from a posted message, never rendered (softn.com's apps stayed
// hidden behind their "loading" state).
var __zippTimerTurn = true;
function __zippRunTask(allowFrame) {
    __zippDueTimers();
    var q = __timerQueue.length > 0 && (__zippTimerTurn || __mcQueue.length === 0) ? __timerQueue
        : __mcQueue.length > 0 ? __mcQueue : null;
    if (q) __zippTimerTurn = q !== __timerQueue;
    if (q) {
        __zippDrainStats.rounds++;
        try { q.shift()(); }
        catch (e) { env.log('error', (q === __timerQueue ? 'timer callback' : 'microtask') + ' threw: ' + ((e && (e.stack || e.message)) || e)); }
        return 1;
    }
    if (allowFrame && __rafQueue.length > 0) {
        __zippRunAnimationFrame(performance.now());
        return 2;
    }
    return 0;
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
// Every attribute on <html>, for applying to the real document element:
// `name U+001F value` records joined by U+001E. `class` always comes first,
// even when empty, because clearing it is how a page leaves dark mode. `style`
// stays out, as in __getHtmlClass.
function __getHtmlAttrList() {
    try {
        var el = doc.documentElement;
        if (!el) return '';
        var out = ['class\u001f' + String(el.className || '')];
        for (var k in el.attributes) {
            if (k === 'class' || k === 'style') continue;
            out.push(k + '\u001f' + String(el.attributes[k]));
        }
        return out.join('\u001e');
    } catch (e) { return ''; }
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

function __zippDispatchEvent(handlerNid, targetNid, type, details) {
    var byType = __eventHandlers[handlerNid];
    if (!byType) return 0;
    var handlers = byType[type];
    if (!handlers || handlers.length === 0) return 0;
    var target = __nodesById[targetNid] || __nodesById[handlerNid] || null;
    return __zippRunListeners(handlers, type, target, __nodesById[handlerNid] || null, 2, details);
}

// The document's listeners (and the window's, which the shim keeps there),
// after the nodes'. 1: a listener ran; 2: it called preventDefault().
function __zippDispatchToDocument(targetNid, type, details) {
    var byType = __eventHandlers[document._id];
    var handlers = byType && byType[type];
    if (!handlers || handlers.length === 0) return 0;
    var target = __nodesById[targetNid] || document.body || document;
    return (__zippRunListeners(handlers, type, target, document, 3, details) & 1) ? 3 : 1;
}

// Call `handlers` with one event, and say what they asked for: 1 when one
// called preventDefault(), 2 when one stopped propagation (both: 3).
// `details` is the JSON the bridge sends (`event_details`): coordinates,
// buttons, modifier keys, wheel deltas, the key.
function __zippRunListeners(handlers, type, target, currentTarget, phase, details) {
    var defaultPrevented = false;
    var propagationStopped = false;
    var immediateStopped = false;
    var ev = {
        type: type,
        target: target,
        // Legacy alias some libraries still read.
        srcElement: target,
        currentTarget: currentTarget,
        eventPhase: phase,
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
    if (details) {
        var more = JSON.parse(details);
        for (var key in more) ev[key] = more[key];
        ev.offsetX = ev.clientX; ev.offsetY = ev.clientY;
        ev.movementX = 0; ev.movementY = 0;
    }
    // The list as it was when the event arrived: a listener that adds or
    // removes others (a drag starting or ending) changes the next event's.
    handlers = handlers.slice();

    for (var i = 0; i < handlers.length; i++) {
        // Never swallow this silently. A listener that throws is the difference
        // between "the click did nothing" and "the click did nothing BECAUSE
        // React tore its own state down", and those need very different fixes.
        // Swallowing it cost real debugging time on the hamburger menu.
        try {
            var h = handlers[i];
            if (typeof h === "function") h.call(currentTarget, ev);
            else if (h && typeof h.handleEvent === "function") h.handleEvent(ev);
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
    //
    // And whether one stopped propagation, so that the event goes no further:
    // a menu button that stops its click, so the document's "click anywhere
    // closes the menu" listener never hears it, opened its menu and had it
    // shut again in the same click.
    return (defaultPrevented ? 1 : 0) | (propagationStopped ? 2 : 0);
}

// What the user typed into a text field, its node's value before its `input`
// listeners run.
function __zippSetValue(nid, value) {
    var el = __nodesById[nid];
    if (el) el.value = value;
}

// Where script scrolled elements since the host last asked: JSON
// `{"nid": [top or null, left or null]}`, or "" for nowhere.
function __zippTakeScrolls() {
    var asked = __zippScrollRequests;
    for (var nid in asked) {
        __zippScrollRequests = {};
        return JSON.stringify(asked);
    }
    return "";
}

// Where scroll containers are scrolled to and how big, from the layout: JSON
// `{"nid": [scrollTop, scrollLeft, scrollWidth, scrollHeight, clientWidth,
// clientHeight]}`.
function __zippScrollMetrics(json) {
    var metrics = JSON.parse(json);
    for (var nid in metrics) {
        var el = __nodesById[nid], m = metrics[nid];
        if (!el) continue;
        el._scrollTop = m[0]; el._scrollLeft = m[1];
        el._scrollWidth = m[2]; el._scrollHeight = m[3];
        el._clientWidth = m[4]; el._clientHeight = m[5];
    }
}

// The element `focus()` was last called on, since the host last asked, by its
// number; -1 when nothing asked for focus.
function __zippTakeFocus() {
    var node = document._focusRequest;
    document._focusRequest = null;
    return node && typeof node._id === "number" ? node._id : -1;
}

// An event on `target` and up through its ancestors to the document (which
// holds the window's listeners too), as the page's own code would see it.
function __zippFire(target, type) {
    for (var n = target; n; n = n.parentNode) {
        var byType = __eventHandlers[n._id];
        var handlers = byType && byType[type];
        if (handlers && handlers.length &&
            (__zippRunListeners(handlers, type, target, n, n === target ? 2 : 3, null) & 2)) break;
    }
}

// What was picked in an <input type=file>'s dialog, as JSON
// `[[path, relativePath, size, modifiedMs], ...]`: the input's `files` now,
// and its `input` and `change` events, as a browser fires them.
function __zippFilesChosen(nid, json) {
    var input = __nodesById[nid];
    if (!input) return 0;
    var files = JSON.parse(json).map(function (f) { return __zippChosenFile(f[0], f[1], f[2], f[3]); });
    files.item = function (i) { return this[i] || null; };
    input._files = files;
    __zippFire(input, "input");
    __zippFire(input, "change");
    return files.length;
}

// A `File` picked in a dialog: its bytes are read from disk when asked for.
function __zippChosenFile(path, relative, size, modified) {
    var file = Object.create(File.prototype);
    file._path = path;
    file._parts = [];
    file.name = path.split(/[\\/]/).pop();
    file.size = size;
    file.lastModified = modified;
    file.webkitRelativePath = relative || "";
    var ext = (file.name.match(/\.([^.]+)$/) || ["", ""])[1].toLowerCase();
    file.type = ({
        py: "text/x-python", js: "text/javascript", mjs: "text/javascript", json: "application/json",
        txt: "text/plain", md: "text/markdown", csv: "text/csv", html: "text/html", htm: "text/html",
        css: "text/css", svg: "image/svg+xml", png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg",
        gif: "image/gif", webp: "image/webp", wasm: "application/wasm", pdf: "application/pdf"
    })[ext] || "";
    return file;
}

// A picked file's bytes. The host reads only what the user picked.
function __zippChosenBytes(path) {
    var len = Number(__zippHostCall("chosen_bytes", path));
    if (__zippHostBytes.length < len) {
        __zippHostBytes = new Uint8Array(Math.max(len, __zippHostBytes.length * 2));
    }
    __zippHostCall("take_bytes");
    return __zippHostBytes.slice(0, len).buffer;
}
"#;

/// The Rust half of the host channel: blocking HTTP on behalf of page JS.
///
/// Blocking is deliberate and matches the previous engine — the DOM shim's
/// `fetch` resolves its promise synchronously from this result, and the whole
/// SSR pass runs to completion before anything is rendered, so there is no
/// event loop to yield to yet.
/// Set once the window's navigation provider exists; see `browser_host_call`.
static NAV_PROVIDER: std::sync::OnceLock<Arc<dyn NavigationProvider>> =
    std::sync::OnceLock::new();

thread_local! {
    /// A binary body fetched by `http_bytes`, waiting for `take_bytes`.
    static PENDING_BYTES: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
    /// The page's body for the shim to start from (`seed.rs`), until it asks.
    static SEED: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    /// The document whose script runs on this thread. It decides whose cookies
    /// a host call reaches, never an argument from the page: a page could
    /// otherwise read and set another site's.
    static DOC_URL: std::cell::RefCell<Option<url::Url>> = const { std::cell::RefCell::new(None) };
    /// The page's import map, for modules it links as it runs (`esm_link`).
    static IMPORT_MAP: std::cell::RefCell<esm::ImportMap> = std::cell::RefCell::new(esm::ImportMap::default());
}

/// The document a host call comes from: the URL its script started on, or the
/// page's own `location.href` (which `pushState` moves) when it is on that
/// same origin. `None` off a page's thread -- a worker has no cookies to read.
fn host_doc_url(claimed: Option<&str>) -> Option<url::Url> {
    let doc = DOC_URL.with(|d| d.borrow().clone())?;
    match claimed.and_then(|c| url::Url::parse(c).ok()) {
        Some(c) if c.origin() == doc.origin() => Some(c),
        _ => Some(doc),
    }
}

/// A URL from page script, resolved against the page as the browser would.
fn resolve_page_url(raw: &str) -> String {
    let base = CURRENT_URL.lock().unwrap().clone();
    url::Url::parse(&base)
        .and_then(|b| b.join(raw))
        .map(|u| u.to_string())
        .unwrap_or_else(|_| raw.to_string())
}

/// GET a URL for page script, as bytes: http(s) through the shared client,
/// `file:` from disk (a page opened from disk fetches its neighbours).
fn fetch_bytes_for_page(url: &str) -> Result<(u16, Vec<u8>), String> {
    if let Ok(parsed) = url::Url::parse(url) {
        if parsed.scheme() == "file" {
            let path = parsed.to_file_path().map_err(|_| format!("Error: GET {url}: not a file path"))?;
            return match std::fs::read(&path) {
                Ok(bytes) => Ok((200, bytes)),
                Err(_) => Ok((404, Vec::new())),
            };
        }
    }
    let resp = HTTP_CLIENT.get(url).send().map_err(|e| format!("Error: GET {url}: {e}"))?;
    let status = resp.status().as_u16();
    let bytes = resp.bytes().map_err(|e| format!("Error: reading {url}: {e}"))?;
    Ok((status, bytes.to_vec()))
}

thread_local! {
    /// The files picked in this window's file dialogs: the only ones a page
    /// may read (`chosen_bytes`).
    static CHOSEN_FILES: std::cell::RefCell<std::collections::HashSet<std::path::PathBuf>> = std::cell::RefCell::default();
}

/// Let the page read these files: the user picked them in a file dialog.
pub(crate) fn allow_chosen_files<'a>(paths: impl IntoIterator<Item = &'a std::path::Path>) {
    CHOSEN_FILES.with(|c| c.borrow_mut().extend(paths.into_iter().map(std::path::Path::to_path_buf)));
}

/// The host channel with the VM at hand, for the calls that move bytes: a
/// binary body goes into the shim's `__zippHostBytes` scratch array through
/// the engine's typed-array region, not through a string. Every other call is
/// `browser_host_call`'s.
fn browser_host_call_ctx(ctx: &mut dyn HostCtx, kind: &str, args: &[String]) -> Result<String, String> {
    match kind {
        // The page's body, once, as the shim's document starts.
        "seed" => Ok(SEED.with(|s| std::mem::take(&mut *s.borrow_mut()))),
        // Web Workers (worker.rs): start one, post to one, stop one.
        "worker_start" => {
            let url = args.first().map(String::as_str).unwrap_or_default();
            let module = args.get(1).is_some_and(|t| t == "module");
            worker::start(url, module).map(|id| id.to_string())
        }
        "worker_post" => {
            let id = args.first().and_then(|a| a.parse().ok()).unwrap_or(0);
            worker::post(id, args.get(1).cloned().unwrap_or_default());
            Ok(String::new())
        }
        "worker_terminate" => {
            worker::terminate(args.first().and_then(|a| a.parse().ok()).unwrap_or(0));
            Ok(String::new())
        }
        // `url.<part> = value` (`URL`'s setters).
        "url_set" => worker::url_set(
            args.first().map(String::as_str).unwrap_or_default(),
            args.get(1).map(String::as_str).unwrap_or_default(),
            args.get(2).map(String::as_str).unwrap_or_default(),
        ),
        // A module the page imports as it runs that was not linked up front:
        // it, and what it imports that the page lacks, as script to evaluate.
        "esm_link" => {
            let url = args.first().cloned().unwrap_or_default();
            let linked: std::collections::HashSet<String> =
                serde_json::from_str(args.get(1).map(String::as_str).unwrap_or("[]")).unwrap_or_default();
            let map = IMPORT_MAP.with(|m| m.borrow().clone());
            let resolve_module = |base: &str, spec: &str| map.resolve(base, spec, &resolve).unwrap_or_else(|| resolve(base, spec));
            let out = esm::link_more(&[url], &linked, false, &fetch, &resolve_module);
            for (url, err) in &out.failures {
                console::push(console::Level::Error, format!("module {url} failed to load: {err}"));
            }
            Ok(out.source)
        }
        // `new URL(input, base)`, for a page or a worker.
        "url" => worker::url_parts(
            args.first().map(String::as_str).unwrap_or_default(),
            args.get(1).map(String::as_str).unwrap_or_default(),
        ),
        // A file picked in a file dialog, as bytes for `take_bytes`; its length.
        // Only those: a page reads no other file of yours this way.
        "chosen_bytes" => {
            let path = std::path::PathBuf::from(args.first().map(String::as_str).unwrap_or_default());
            if !CHOSEN_FILES.with(|c| c.borrow().contains(&path)) {
                return Err("NotAllowedError: only a file picked in a file dialog can be read".into());
            }
            let bytes = std::fs::read(&path).map_err(|e| format!("NotReadableError: {}: {e}", path.display()))?;
            let len = bytes.len();
            PENDING_BYTES.with(|p| *p.borrow_mut() = bytes);
            Ok(len.to_string())
        }
        // `fetch(url).arrayBuffer()`: fetch the body as bytes and keep it for
        // `take_bytes`; report its status and length, so the shim can make the
        // scratch array big enough first.
        "http_bytes" => {
            let url = resolve_page_url(args.first().map(String::as_str).unwrap_or_default());
            let (status, bytes) = fetch_bytes_for_page(&url)?;
            let len = bytes.len();
            PENDING_BYTES.with(|p| *p.borrow_mut() = bytes);
            Ok(format!(
                "{{\"status\":{status},\"ok\":{},\"len\":{len}}}",
                (200..300).contains(&status)
            ))
        }
        // Copy the pending bytes into `__zippHostBytes`.
        // An element's box in the viewport (`getBoundingClientRect`).
        "rect" => Ok(event_bridge::layout_rect(args.first().map(String::as_str).unwrap_or_default())),
        // `offsetWidth`, `clientWidth`, a ResizeObserver's boxes.
        "box" => Ok(event_bridge::layout_box(args.first().map(String::as_str).unwrap_or_default())),
        // `getComputedStyle(el)[name]`, from the last layout.
        "computed_style" => Ok(event_bridge::layout_style(
            args.first().map(String::as_str).unwrap_or_default(),
            args.get(1).map(String::as_str).unwrap_or_default(),
        )),
        "take_bytes" => {
            let (addr, count, kind) = ctx.typed_array_region("__zippHostBytes")?;
            if kind != 1 {
                return Err("TypeError: __zippHostBytes must be a Uint8Array".into());
            }
            PENDING_BYTES.with(|p| {
                let bytes = std::mem::take(&mut *p.borrow_mut());
                if bytes.len() > count {
                    return Err("RangeError: __zippHostBytes is too small".to_string());
                }
                // SAFETY: the engine pinned the array: `count` bytes at `addr`,
                // alive and in place for the rest of this call.
                unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), addr as *mut u8, bytes.len()) };
                Ok(bytes.len().to_string())
            })
        }
        // A WebGL context's batch of calls (webgl.rs).
        #[cfg(any(windows, target_os = "linux"))]
        "gl" => webgl::host_call(ctx, args),
        #[cfg(not(any(windows, target_os = "linux")))]
        "gl" => Err("Error: WebGL is not available on this platform".into()),
        // A Canvas 2D batch (canvas2d.rs).
        "c2d" => canvas2d::host_call(ctx, args),
        // A WebGPU batch (webgpu.rs).
        #[cfg(any(windows, target_os = "linux"))]
        "gpu" => webgpu::host_call(ctx, args),
        // Web Audio's output, and decoding (audio.rs).
        "audio" => audio::host_call(ctx, args),
        // navigator.zipp: files, programs, the machine (system.rs).
        "zipp" => system::host_call(ctx, args),
        // WebUSB (usb.rs).
        #[cfg(target_os = "linux")]
        "usb" => usb::host_call(args),
        #[cfg(not(target_os = "linux"))]
        "usb" => Err("NotSupportedError: WebUSB is not available on this platform".into()),
        #[cfg(not(any(windows, target_os = "linux")))]
        "gpu" => Err("Error: WebGPU is not available on this platform".into()),
        // An image's size, for `naturalWidth` and `load`.
        "image_info" => viewers::image_info(&resolve_page_url(args.first().map(String::as_str).unwrap_or_default())),
        _ => browser_host_call(kind, args),
    }
}

fn browser_host_call(kind: &str, args: &[String]) -> Result<String, String> {
    match kind {
        // What the site keeps -- IndexedDB, `localStorage` -- in its own
        // folder: which site's is decided here, from the document.
        "site_read" | "site_write" | "site_delete" | "site_names" => {
            let doc = host_doc_url(None).ok_or("SecurityError: no document")?;
            let arg = |i: usize| args.get(i).map(String::as_str).unwrap_or_default();
            match kind {
                "site_read" => site_storage::read(&doc, arg(0), arg(1)).map(Option::unwrap_or_default),
                "site_write" => site_storage::write(&doc, arg(0), arg(1), Some(arg(2).to_string())).map(|_| String::new()),
                "site_delete" => site_storage::write(&doc, arg(0), arg(1), None).map(|_| String::new()),
                _ => site_storage::names(&doc, arg(0)),
            }
        }
        // `document.cookie`, read and written: the browser's jar, for this
        // document only.
        "cookie_get" => Ok(host_doc_url(args.first().map(String::as_str))
            .map(|u| zipp_shell::cookies::jar().document_cookie(&u))
            .unwrap_or_default()),
        "cookie_set" => {
            if let (Some(url), Some(value)) = (host_doc_url(args.first().map(String::as_str)), args.get(1)) {
                zipp_shell::cookies::jar().set_document_cookie(&url, value);
            }
            Ok(String::new())
        }
        // Page `console.*`, via the shim's `env.log`.
        "log" => {
            let level = args.first().map(String::as_str).unwrap_or("info");
            let message = args.get(1).map(String::as_str).unwrap_or("");
            eprintln!("[JS:{level}] {message}");
            console::push(console::Level::from_name(level), message);
            Ok(String::new())
        }
        "http" => {
            let method = args.first().map(String::as_str).unwrap_or("GET");
            let url = args.get(1).map(String::as_str).unwrap_or_default();
            let body = args.get(2).cloned().unwrap_or_default();
            let content_type = args.get(3).map(String::as_str).unwrap_or("");
            let content_type =
                if content_type.is_empty() { "application/json" } else { content_type };

            // Relative to the page, as fetch resolves it; a page opened from
            // disk reads its neighbours.
            let resolved = resolve_page_url(url);
            let url = resolved.as_str();
            if method == "GET" && url.starts_with("file:") {
                let (status, bytes) = fetch_bytes_for_page(url)?;
                return Ok(format!(
                    "{{\"status\":{status},\"ok\":{},\"body\":{}}}",
                    status == 200,
                    json_quote(&String::from_utf8_lossy(&bytes))
                ));
            }
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
        // `window.open` and `location.assign`/`replace`. Both were inert: `open`
        // returned null and did nothing, and `assign` only wrote to a plain
        // `href` property that nothing observed. A page whose call to action is
        // "opens in a new tab" therefore had a button that did nothing at all.
        //
        // There are no tabs yet, so this navigates the CURRENT window. That is
        // the honest single-window behaviour and beats silently dropping it.
        "navigate" => {
            let raw = args.first().cloned().unwrap_or_default();
            if raw.is_empty() {
                return Ok(String::new());
            }
            let base = CURRENT_URL.lock().unwrap().clone();
            let full = resolve(&base, &raw);
            match full.parse::<url::Url>() {
                Ok(parsed) => {
                    eprintln!("[NAV] page requested {full}");
                    if let Some(provider) = NAV_PROVIDER.get() {
                        provider.navigate_to(NavigationOptions::new(
                            parsed,
                            String::from("text/html"),
                            0,
                        ));
                    }
                    Ok(String::new())
                }
                Err(e) => Err(format!("Error: bad navigate url {full:?}: {e}")),
            }
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
    // A PDF, image, text file or download comes back as a viewer page.
    let html = viewers::fetch_document(url, true).unwrap_or_else(|e| error_page(url, &e));
    let fetch_ms = t_fetch.elapsed().as_millis();
    render_page(url, &html, viewport, Some(fetch_ms))
}

/// Run a fetched page's JS and post-process it into the HTML the renderer sees:
/// the second half of [`prepare_page`], and all of a navigation's work on the
/// main thread. `fetch_ms` is `None` when the fetch happened elsewhere.
fn render_page(
    url: &str,
    html: &str,
    viewport: (u32, u32),
    fetch_ms: Option<u128>,
) -> (String, Option<String>, Option<ScriptState>) {
    *CURRENT_URL.lock().unwrap() = url.to_string();
    let html = html.to_string();
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

    let fetch = fetch_ms.map(|ms| format!("{ms}ms")).unwrap_or_else(|| "-".to_string());
    eprintln!(
        "[PERF] Page: fetch={fetch} js={js_ms}ms post={post_ms}ms ({} bytes)",
        out.len()
    );
    (out, event_map_json, js_state)
}

/// The page shown when a load fails.
fn error_page(url: &str, error: &str) -> String {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    format!(
        r#"<!DOCTYPE html><html><head><meta charset="utf-8"><title>Can't load page</title><style>
body{{margin:0;background:#f8f9fa;font:15px Arial,sans-serif;color:#202124}}
.err{{max-width:640px;margin:72px auto;padding:0 24px}} h1{{font-size:24px;font-weight:600;margin:0 0 12px}}
p{{line-height:1.5;overflow-wrap:anywhere}} .k{{color:#5f6368}}
</style></head><body><div class="err"><h1>This page can't be loaded</h1><p>{}</p><p class="k">{}</p></div></body></html>"#,
        esc(url),
        esc(error)
    )
}

/// Map the page's JS nodes onto the document (by `data-flid`) and install the
/// event bridge, so DOM events reach the page's handlers through its live
/// script. A page that ran no script, or registered no handlers, gets none.
fn attach_event_bridge(
    doc: &mut HtmlDocument,
    event_map_json: Option<String>,
    js_state: Option<ScriptState>,
    url: &str,
) {
    // A page without script gets a bridge too: it is what runs the console.
    let emap_json = event_map_json.unwrap_or_else(|| "{}".to_string());
    let flid_map = event_bridge::build_flid_map(doc.as_ref());
    if !flid_map.is_empty() {
        eprintln!("[EVENT-BRIDGE] Mapped {} nodes with data-flid attributes", flid_map.len());
    }
    let bridge = event_bridge::JsEventBridge::new(flid_map, &emap_json, js_state, url);
    eprintln!(
        "[EVENT-BRIDGE] Bridge created with {} handler entries (live JS: {})",
        bridge.handler_count(),
        bridge.has_js_state(),
    );
    // Every page has one: it runs the page's animation frames and timers, and
    // the console's controls, whether or not the page listens for anything.
    doc.set_event_handler(Box::new(bridge));
    eprintln!("[EVENT-BRIDGE] Event handler installed on document");
}

// ---------------------------------------------------------------------------
// Headless rendering
// ---------------------------------------------------------------------------

/// Load `url`, lay it out at `width`x`height` and paint it to a PNG.
///
/// No window, no event loop — just the same document pipeline the window uses,
/// pointed at a software framebuffer. Returns whether it succeeded.
/// The viewport WPT reftests are written for.
const REFTEST_SIZE: (u32, u32) = (800, 600);

/// Pages are rendered without the browser's toolbar (`--reftest`).
static NO_CHROME: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `--reftest pairs.tsv results.tsv`: run Web Platform Tests reftests. Each
/// line of `pairs` is `test URL \t reference URL \t largest channel difference
/// allowed \t differing pixels allowed \t match|mismatch` (WPT's `fuzzy`
/// annotation gives the allowances); both pages are rendered at 800x600, and
/// one line is appended to `results` per test:
/// `test URL \t pass|fail|error|crash \t differing pixels \t largest difference`.
///
/// Tests already in `results` are skipped, so the driver can restart a run that
/// a crash ended where it stopped. A panic is caught and reported as `crash`;
/// an abort (a stack overflow) ends the process, and the driver records it.
fn reftest_batch(pairs: &str, results: &str) -> bool {
    use std::io::Write;
    let done: std::collections::HashSet<String> = std::fs::read_to_string(results)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split('\t').next().map(str::to_string))
        .collect();
    let Ok(list) = std::fs::read_to_string(pairs) else {
        eprintln!("[REFTEST] cannot read {pairs}");
        return false;
    };
    let Ok(mut out) = std::fs::OpenOptions::new().create(true).append(true).open(results) else {
        eprintln!("[REFTEST] cannot write {results}");
        return false;
    };
    NO_CHROME.store(true, std::sync::atomic::Ordering::Relaxed);
    // Stylesheets, fonts and images land asynchronously: a runtime for the run.
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio runtime");
    // One network client for the run, as a window has one for its pages: a
    // client per page opened thousands of connections a minute, and the ones
    // closing still held their ports, so new ones failed now and then.
    let net = Arc::new(zipp_shell::net::Provider::new(None));
    for line in list.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 2 || done.contains(f[0]) {
            continue;
        }
        let max_diff: u8 = f.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);
        let max_pixels: usize = f.get(3).and_then(|v| v.parse().ok()).unwrap_or(0);
        let mismatch = f.get(4) == Some(&"mismatch");
        let rendered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            (render_rgba(f[0], &rt, &net), render_rgba(f[1], &rt, &net))
        }));
        let (status, differing, largest) = match rendered {
            Err(_) => ("crash", 0, 0),
            Ok((Some(test), Some(reference))) => {
                let (mut differing, mut largest) = (0usize, 0u8);
                for (a, b) in test.data().chunks_exact(4).zip(reference.data().chunks_exact(4)) {
                    let d = a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
                    if d > max_diff {
                        differing += 1;
                    }
                    largest = largest.max(d);
                }
                let same = differing <= max_pixels;
                let status = if same != mismatch { "pass" } else { "fail" };
                // `ZIPP_REFTEST_DUMP=<dir>`: both renders of each failure, to look at.
                if status == "fail"
                    && let Some(dir) = std::env::var_os("ZIPP_REFTEST_DUMP")
                {
                    let name: String = f[0].rsplit('/').take(2).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("_");
                    let base = std::path::Path::new(&dir).join(name);
                    let _ = test.save_png(base.with_extension("test.png"));
                    let _ = reference.save_png(base.with_extension("ref.png"));
                }
                (status, differing, largest)
            }
            Ok(_) => ("error", 0, 0),
        };
        let _ = writeln!(out, "{}\t{status}\t{differing}\t{largest}", f[0]);
        let _ = out.flush();
        eprintln!("[REFTEST] {status} {}", f[0]);
    }
    true
}

/// A page rendered at the reftest size, with fonts of its own: a clone of a
/// font context shares its collection, so the web fonts one test registered
/// were there for every test after it in the process, and results depended
/// on the order tests ran in.
fn render_rgba(url: &str, rt: &tokio::runtime::Runtime, net_provider: &Arc<zipp_shell::net::Provider>) -> Option<tiny_skia::Pixmap> {
    use zipp_dom::traits_src::shell::{ColorScheme, Viewport};
    let (width, height) = REFTEST_SIZE;
    // The page itself comes through the blocking client, which must not run
    // inside the runtime: in it, fetches failed now and then ("error sending
    // request"), and a page that did not load failed its test.
    let (final_html, _, _js_state) = prepare_page(url, (width, height));
    let _guard = rt.enter();
    let config = DocumentConfig {
        font_ctx: Some(load_fonts()),
        base_url: Some(url.to_string()),
        net_provider: Some(net_provider.clone() as _),
        html_parser_provider: Some(Arc::new(HtmlProvider) as _),
        navigation_provider: Some(Arc::new(RecordingNavProvider) as _),
        // Light, whatever the machine prefers: the same pixels every run.
        viewport: Some(Viewport::new(width, height, 1.0, ColorScheme::Light)),
        ..Default::default()
    };
    let mut doc = build_document(&final_html, config);
    // Stylesheets, webfonts (Ahem, mostly) and images first, as `screenshot`.
    let t = std::time::Instant::now();
    loop {
        while (!net_provider.is_empty() || doc.as_ref().has_pending_critical_resources()) && t.elapsed().as_secs() < 10 {
            std::thread::sleep(std::time::Duration::from_millis(5));
            doc.as_mut().resolve(0.0);
        }
        doc.as_mut().handle_messages();
        doc.as_mut().resolve(0.0);
        if t.elapsed().as_secs() >= 10 || (net_provider.is_empty() && !doc.as_ref().has_pending_critical_resources()) {
            break;
        }
    }
    doc.as_mut().resolve(0.0);
    doc.as_mut().resolve(ANIMATION_SETTLE_SECS);
    let mut painter = zipp_renderer::tinyskia::TinySkiaPainter::new(width, height);
    // The canvas a browser shows under a page with no background of its own.
    painter.pixmap.fill(tiny_skia::Color::WHITE);
    zipp_renderer::paint_scene(&mut painter, doc.as_ref(), 1.0, width, height, 0, 0);
    Some(painter.pixmap)
}

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
            event_bridge::run_page_tasks(state, 400, 2, std::time::Duration::from_secs(1));
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
                    let body = rerender_body(&updated, url) + &toolbar_markup(url);
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
        viewport: Some(Viewport::new(
            width,
            height,
            1.0,
            if system_prefers_dark() { ColorScheme::Dark } else { ColorScheme::Light },
        )),
        ..Default::default()
    };

    let t_parse = std::time::Instant::now();
    let mut doc = build_document(&final_html, config);

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
    //
    // The whole thing runs in an OUTER loop because layout is what discovers
    // background images: a `background-image` URL is only requested once styles
    // resolve (`zipp-dom/src/layout/damage.rs`), which happens during the
    // resolve below, not during parsing. A page whose only images are
    // backgrounds therefore had an empty network queue here, exited the wait
    // immediately, and was painted before a single tile arrived — every
    // background image came out blank. Keep going until a full resolve leaves
    // the queue still empty.
    let t_net = std::time::Instant::now();
    loop {
        while (!net_provider.is_empty() || doc.as_ref().has_pending_critical_resources())
            && t_net.elapsed().as_secs() < 20
        {
            std::thread::sleep(std::time::Duration::from_millis(50));
            doc.as_mut().resolve(0.0);
        }
        // One more drain so a font blob that landed after the last poll is
        // registered (which invalidates inline layout) before the final resolve.
        doc.as_mut().handle_messages();
        doc.as_mut().resolve(0.0);
        if t_net.elapsed().as_secs() >= 20
            || (net_provider.is_empty() && !doc.as_ref().has_pending_critical_resources())
        {
            break;
        }
    }
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

    // `ZIPP_RESIZE=w,h[;w,h...]` lays the page out at the requested size first,
    // then resizes the viewport through each size in turn, the way dragging the
    // window does, and screenshots at the last. A fresh load at a width and a
    // resize TO that width take different paths (the second restyles and
    // relayouts a live tree), and only the second is what a user resizing the
    // window sees.
    let (width, height) = {
        let (mut width, mut height) = (width, height);
        let sizes = std::env::var("ZIPP_RESIZE").unwrap_or_default();
        for (i, (w, h)) in sizes
            .split(';')
            .filter_map(|s| s.split_once(','))
            .filter_map(|(w, h)| Some((w.trim().parse::<u32>().ok()?, h.trim().parse::<u32>().ok()?)))
            .enumerate()
        {
            eprintln!("[RESIZE] {width}x{height} -> {w}x{h}");
            doc.as_mut().viewport_mut().window_size = (w, h);
            doc.as_mut().resolve(ANIMATION_SETTLE_SECS * (2 + 2 * i) as f64);
            doc.as_mut().resolve(ANIMATION_SETTLE_SECS * (3 + 2 * i) as f64);
            (width, height) = (w, h);
        }
        (width, height)
    };

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
    DOC_URL.with(|d| *d.borrow_mut() = url::Url::parse(page_url).ok());
    let mut external_scripts = Vec::new();
    // `<script type="module">` entries are linked separately: their source is ES
    // module syntax, which is a syntax error in the classic program the rest of
    // the pipeline builds. Before this, the whole bundle was concatenated in as
    // a script and died at `import.meta is only valid in modules`.
    let mut module_entries: Vec<String> = Vec::new();
    // Inline modules' source, by the URL each is linked under (the page's, with
    // a fragment of its own), so they join the graph in document order.
    let mut inline_modules: HashMap<String, String> = HashMap::new();
    let mut import_map = esm::ImportMap::default();
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
        } else if is_js || ty == "importmap" {
            // INLINE script. These were skipped entirely, so a page whose
            // behaviour lives in a `<script>` block ran none of it — the tag was
            // parsed for `src`, found none, and moved on.
            let body_start = tag_end;
            if let Some(close) = html[body_start..].find("</script") {
                let code = &html[body_start..body_start + close];
                if ty == "importmap" {
                    import_map.add(code, page_url, &resolve);
                } else if !code.trim().is_empty() {
                    if ty == "module" {
                        // An inline module still needs linking; give it a URL of
                        // its own so relative specifiers resolve against the page.
                        let url = format!("{page_url}#inline-module-{}", inline_modules.len());
                        inline_modules.insert(url.clone(), code.to_string());
                        module_entries.push(url);
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
    if external_scripts.is_empty() && module_entries.is_empty() {
        return None;
    }
    let seed = seed::body_json(html).unwrap_or_default();
    SEED.with(|s| *s.borrow_mut() = seed);

    let dom_shim = &platform_shim(&format!("{}\n{}", include_str!("console_shim.js"), include_str!("dom_shim.js")));
    let router_helpers = include_str!("router_helpers.js");
    let webgl_shim = include_str!("webgl_shim.js");
    let webgpu_shim = include_str!("webgpu_shim.js");
    let webusb_shim = include_str!("webusb_shim.js");
    let webaudio_shim = include_str!("webaudio_shim.js");
    let zipp_shim = include_str!("zipp_shim.js");
    let dom_shim = &format!(
        "{HOST_PRELUDE_JS}\n{dom_shim}\n{}\n{}\n{}\n{webgl_shim}\n{}\n{webgpu_shim}\n{webusb_shim}\n{webaudio_shim}\n{zipp_shim}",
        include_str!("clone.js"),
        include_str!("indexeddb_shim.js"),
        include_str!("worker_page.js"),
        include_str!("canvas2d_shim.js")
    );
    let shim = dom_shim.replace(
        "__PAGE_URL__",
        &page_url.replace('\\', "\\\\").replace('\'', "\\'"),
    );
    let shim = shim.replace("__PAGE_TITLE__", &js_quoted(&page_title(html)));
    // The page's JS must see the SAME viewport the CSS is laid out against, or
    // a responsive app that branches on `window.innerWidth` renders for one
    // width while the cascade uses another.
    // Likewise the colour scheme: `matchMedia('(prefers-color-scheme: dark)')`
    // has to give the answer Stylo uses for the same query in CSS.
    let shim = shim
        .replace("__VIEWPORT_W__", &viewport.0.to_string())
        .replace("__VIEWPORT_H__", &viewport.1.to_string())
        .replace("__PREFERS_DARK__", if system_prefers_dark() { "true" } else { "false" })
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
        let fetch_module = |url: &str| match inline_modules.get(url) {
            Some(code) => Ok(code.clone()),
            None => fetch(url),
        };
        let resolve_module =
            |base: &str, spec: &str| import_map.resolve(base, spec, &resolve).unwrap_or_else(|| resolve(base, spec));
        let linked = esm::link_graph(&module_entries, &fetch_module, &resolve_module);
        IMPORT_MAP.with(|m| *m.borrow_mut() = import_map.clone());
        eprintln!(
            "[ESM] Linked {} modules from {} entr{} in {}ms",
            linked.module_count,
            module_entries.len(),
            if module_entries.len() == 1 { "y" } else { "ies" },
            t_link.elapsed().as_millis(),
        );
        for (url, err) in &linked.failures {
            eprintln!("[ESM] FAILED {url}: {err}");
            console::push(console::Level::Error, format!("module {url} failed to load: {err}"));
        }
        if std::env::var_os("ZIPP_DEBUG_ESM").is_some() {
            full.push_str("
var __zippEsmDebug=true;
");
        }
        full.push_str(&format!("\nvar __zippImportMap = {};\n", import_map.to_js()));
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

    // The page settles after this program, in the host's event loop
    // (`run_page_tasks`). It used to run its timers here, at the end of the
    // program and before the program's own microtasks, so every timer saw the
    // promises the scripts had settled as still pending.

    // `ZIPP_DUMP_PROGRAM=<path>` writes the whole program out -- shim, page
    // scripts, linked modules -- so that a stack frame's `<anonymous>:line:col`
    // can be read off it directly.
    if let Ok(path) = std::env::var("ZIPP_DUMP_PROGRAM") {
        let _ = std::fs::write(&path, &full);
        eprintln!("[JS] wrote the program to {path}");
    }
    eprintln!("[JS] Compiling {} bytes...", full.len());
    let t_compile = std::time::Instant::now();
    let mut state = match compile_script(&full) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[JS] Compile error: {e}");
            // The offending line of the program, from the error's position:
            // the program is the shim and every script run together, so the
            // number alone says little.
            let line = e
                .to_string()
                .split("<anonymous>:")
                .nth(1)
                .and_then(|at| at.split(':').next()?.trim().parse::<usize>().ok())
                .and_then(|n| full.lines().nth(n.saturating_sub(1)))
                .map(|l| l.chars().take(240).collect::<String>());
            if let Some(line) = &line {
                eprintln!("[JS]   at: {line}");
            }
            console::push(
                console::Level::Error,
                match line {
                    Some(line) => format!("SyntaxError: {e}\n  at: {line}"),
                    None => format!("SyntaxError: {e}"),
                },
            );
            return None;
        }
    };
    let compile_ms = t_compile.elapsed().as_millis();
    let t_init = std::time::Instant::now();
    state.set_host_call_ctx(Box::new(browser_host_call_ctx));
    // A rejection nothing handled is an error the page should hear of, as a
    // browser's console reports one.
    state.set_unhandled_rejection_sink(Box::new(|reason| {
        eprintln!("[JS] Uncaught (in promise) {reason}");
        console::push(console::Level::Error, format!("Uncaught (in promise) {reason}"));
    }));
    if let Err(e) = state.run_init() {
        // An uncaught throw is not fatal: the shim may still have built enough
        // of a tree to render, so fall through and check. This matches how a
        // real browser treats a script error — the page is what it is.
        eprintln!("[JS] Uncaught: {e}");
    }
    // Settle: run the page's event loop until it is quiet -- each due timer
    // and posted message as a task, the VM's microtasks after each (an awaited
    // fetch's continuation, a `.then`), and up to two animation frames once
    // nothing else is due. A page can reschedule forever (a spinner, a poll),
    // so it stops after a fixed number of tasks.
    //
    // Without settling, a client-rendered app is serialised mid-boot:
    // formlogic.local fetched its session, got its 401, and would have
    // rendered the logged-out UI, but the tree was read before the
    // continuation that sets `isInitialized` ever ran.
    let init_ms = t_init.elapsed().as_millis();
    let t_settle = std::time::Instant::now();
    event_bridge::run_page_tasks(&mut state, 600, 2, std::time::Duration::from_secs(3));
    // Settled: from here on the page's timers keep real time.
    let _ = state.call_global("__zippEndSettle", &[]);
    // A page drawing with WebGL shows its first frame from the start: present
    // it now, for the document `build_document` makes from this render.
    if matches!(state.call_global("__zippWebGLContexts", &[]), Ok(JsValue::Number(n)) if n > 0.0) {
        let _ = state.call_global("__zippAnimationFrame", &[JsValue::Number(0.0)]);
        state.run_microtasks();
    }
    // Split so it is obvious whether time goes into compiling the script or
    // running it -- they need completely different fixes.
    eprintln!(
        "[PERF] JS: compile={compile_ms}ms init={init_ms}ms settle={}ms",
        t_settle.elapsed().as_millis()
    );
    for line in state.take_output() {
        eprintln!("[JS] {line}");
        console::push(console::Level::Log, line);
    }
    for line in state.take_errput() {
        eprintln!("[JS] {line}");
        console::push(console::Level::Error, line);
    }

    if let Ok(JsValue::String(rendered)) = state.eval_in_context("__getRenderedHTML()") {
        if !rendered.is_empty() {
            // Extract the event handler map before we lose the state.
            let event_map_json = match state.eval_in_context("__getEventMap()") {
                Ok(JsValue::String(s)) => s,
                _ => "{}".to_string(),
            };

            let (body_attrs, inner) = split_body_element(&rendered);
            let inner = make_urls_absolute(inner, page_url);
            let inner = inner
                .replace("\\xa9", "\u{00A9}")
                .replace("\\u2019", "\u{2019}")
                .replace("\\u2018", "\u{2018}")
                .replace("\\u2014", "\u{2014}");
            let head = make_urls_absolute(&head_content(html), page_url);
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
                "<!DOCTYPE html><html{html_attrs}><head>{head}</head><body{body_attrs}>{inner}</body></html>"
            );
            return Some((html_out, event_map_json, state));
        }
    }
    // The page shows its own markup, not the script's tree (see
    // `__getRenderedHTML`), but its script lives on: its timers run and the
    // console can reach it. An empty event map says so (`JsEventBridge::new`).
    Some((html.to_string(), String::new(), state))
}

// ---------------------------------------------------------------------------
// Navigation provider
// ---------------------------------------------------------------------------

/// Handles link clicks by loading pages internally.
struct BrowserNavProvider {
    proxy: ShellProxy,
    /// The page the window is showing. Every navigation passes through this
    /// provider, so it is the one place that always knows.
    current_url: std::sync::Mutex<String>,
    /// Back/forward history, for the same reason.
    history: std::sync::Mutex<History>,
}

/// The pages visited in this window, and which one is showing.
struct History {
    entries: Vec<String>,
    index: usize,
}

impl History {
    fn new(first: &str) -> Self {
        History { entries: vec![first.to_string()], index: 0 }
    }

    /// A new page: anything ahead of the current one is dropped, as a browser
    /// does when you go back and then follow a different link.
    fn push(&mut self, url: &str) {
        if self.entries[self.index] == url {
            return;
        }
        self.entries.truncate(self.index + 1);
        self.entries.push(url.to_string());
        self.index += 1;
    }

    fn back(&mut self) -> Option<String> {
        self.index = self.index.checked_sub(1)?;
        Some(self.entries[self.index].clone())
    }

    fn forward(&mut self) -> Option<String> {
        let next = self.index + 1;
        let url = self.entries.get(next)?.clone();
        self.index = next;
        Some(url)
    }

    fn current(&self) -> String {
        self.entries[self.index].clone()
    }
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

        // Back, forward and reload -- the toolbar's buttons, and the shell's
        // keyboard and mouse shortcuts, ask for these by URL.
        if let Some(command) = url.strip_prefix("zipp-nav:") {
            let target = {
                let mut history = self.history.lock().unwrap();
                match command {
                    "back" => history.back(),
                    "forward" => history.forward(),
                    "reload" => {
                        let current = history.current();
                        PAGE_CACHE.lock().unwrap().remove(&current);
                        Some(current)
                    }
                    _ => None,
                }
            };
            match target {
                Some(target) => self.load(target),
                None => eprintln!("[NAV] nothing to go {command} to"),
            }
            return;
        }

        // The console's own links: the event bridge opens, closes and clears it
        // itself.
        if url.starts_with("zipp-console:") {
            return;
        }
        // Code typed into the console. Its line is a form posting back to the
        // page, like the address bar's; the code waits here for the page's
        // thread (`JsEventBridge::run_tasks`).
        if url.contains("__js=") {
            if let Ok(parsed) = url::Url::parse(&url) {
                if let Some((_, code)) = parsed.query_pairs().find(|(k, _)| k == "__js") {
                    console::submit(code.into_owned());
                    return;
                }
            }
        }

        // The PDF viewer's Save button: write the file to Downloads and show the
        // viewer again with a note of where it went.
        if let Some(target) = url.strip_prefix("zipp-save:") {
            let target = target.to_string();
            let proxy = self.proxy.clone();
            std::thread::spawn(move || match viewers::save_pdf(&target) {
                Ok(html) => proxy.send_event(zipp_shell::ShellEvent::NavigationLoad {
                    contents: html,
                    url: target,
                    retain_scroll_position: true,
                    is_md: false,
                }),
                Err(e) => eprintln!("[DOWNLOAD] saving {target} failed: {e}"),
            });
            return;
        }

        // A link to a fragment of the page already showing is a scroll, not a
        // navigation. This compared the fragment-less URL against the render
        // cache, keyed `https://lance.name` while the link resolved to
        // `https://lance.name/#about`, so it never matched: every in-page nav
        // link reloaded the page, which left it with no live JS and nothing
        // on it responding. Parsed URLs compare equal across that slash.
        if let Ok(target) = url::Url::parse(&url) {
            if let Some(fragment) = target.fragment() {
                let current = self.current_url.lock().unwrap().clone();
                let same_document = url::Url::parse(&current).is_ok_and(|mut current| {
                    let mut target = target.clone();
                    current.set_fragment(None);
                    target.set_fragment(None);
                    current == target
                });
                if same_document {
                    let fragment = urlencoding::decode(fragment)
                        .map(|f| f.into_owned())
                        .unwrap_or_else(|_| fragment.to_string());
                    eprintln!("[NAV] Same-document link -> scroll to #{fragment}");
                    self.proxy
                        .send_event(zipp_shell::ShellEvent::ScrollToFragment { fragment });
                    return;
                }
            }
        }
        // Another document: the server never sees the fragment.
        if let Some((base, _)) = url.split_once('#') {
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

        self.history.lock().unwrap().push(&url);
        self.load(url);
    }
}

impl BrowserNavProvider {
    /// Show `url`: fetch it on a worker thread (or take it from the cache), then
    /// hand it to the window, whose document builder runs its JS.
    fn load(&self, url: String) {
        *self.current_url.lock().unwrap() = url.clone();
        let proxy = self.proxy.clone();

        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(move || {
                // Only the fetch happens here. The page's JS runs on the main
                // thread, in the document builder, so that its live state can
                // stay attached to the document it rendered.
                let cached = PAGE_CACHE.lock().unwrap().get(&url).cloned();
                let contents = match cached {
                    Some(html) => {
                        eprintln!("[NAV] Cache hit for {url}");
                        html
                    }
                    None => {
                        eprintln!("[NAV] Loading {url}...");
                        match viewers::fetch_document(&url, true) {
                            Ok(html) => {
                                PAGE_CACHE.lock().unwrap().insert(url.clone(), html.clone());
                                html
                            }
                            Err(e) => {
                                eprintln!("[NAV] Fetch error: {e}");
                                error_page(&url, &e)
                            }
                        }
                    }
                };
                proxy.send_event(zipp_shell::ShellEvent::NavigationLoad {
                    url,
                    contents,
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

/// Styles for the injected browser chrome.
///
/// Deliberately TINY. The toolbar's own markup is styled entirely with inline
/// `style` attributes; this is only the room the root makes for it. Anything
/// broader leaks into the page, because the chrome is part of the page's own
/// document and shares its CSS namespace.
///
/// This used to carry ~60 `!important` rules redefining generic Tailwind utility
/// names (`.grid`, `.flex`, `.w-full`, ...), which overrode the design system of
/// every Tailwind site -- `.grid{display:flex}` alone turned every CSS Grid into
/// a flex row. Rule of thumb for anything added here: if a page could plausibly
/// use the same selector, it does not belong.
///
/// The toolbar sits in a band across the top of the root: `<html>` reserves it
/// with a transparent top border, and the toolbar -- `<html>`'s own child,
/// beside `<body>` (see `mount_chrome`) -- is placed absolutely into it, from
/// the root's padding box, hence `top:-42px`. A border rather than padding, so
/// a page's own `html` padding still applies. With the console open the band
/// grows to hold it too, under the toolbar; opening and closing it rewrites
/// this stylesheet (`JsEventBridge::show_console`).
pub(crate) fn chrome_css(console_open: bool) -> String {
    let panel = console::PANEL_HEIGHT;
    let band = if console_open { 42 + panel } else { 42 };
    let (display, button) = if console_open { ("flex", "#dbe4ff") } else { ("none", "transparent") };
    format!(
        "html {{ border-top:{band}px solid transparent !important; margin:0 !important; max-width:none !important; }}\n\
         #__zipp_chrome {{ top:-{band}px; }}\n\
         #__zipp_console {{ display:{display}; top:-{panel}px; height:{panel}px; }}\n\
         #__zipp_console_button {{ background:{button}; }}\n"
    )
}

/// Add the browser chrome to a page.
///
/// The page's own document is kept whole -- its `<html>` and `<body>` with
/// their attributes, the page's `body` rules, the default body margin -- and
/// the toolbar goes in at the end of the body, to be moved up beside it by
/// `mount_chrome` once the document is parsed.
///
/// It used to be the other way round: the document was rebuilt around the
/// chrome, a full-height flex column in a `<body>` reset inline to
/// `margin:0;padding:0`, with the page poured into a `<div>` inside it. No
/// `body` rule could reach the page -- example.com's
/// `max-width:26em;margin:auto;padding:25vh 2em 2em`, `body{display:grid}`,
/// `body > header` -- and the page's body attributes were dropped, taking
/// `body.home .hero`-style selectors with them.
fn wrap_with_toolbar(html: &str, current_url: &str) -> String {
    // A reftest's page as written: no toolbar (whose address differs between a
    // test and its reference), no band above it, no doctype it did not have.
    if NO_CHROME.load(std::sync::atomic::Ordering::Relaxed) {
        return html.to_string();
    }
    let toolbar = toolbar_markup(current_url);
    // Lowercasing only ASCII keeps every byte offset valid in `html`.
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len() + toolbar.len() + 16);
    // Every page used to be rebuilt under `<!DOCTYPE html>`; keep pages without
    // one in standards mode still.
    if !lower
        .trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
        .starts_with("<!doctype")
    {
        out.push_str("<!DOCTYPE html>");
    }
    match lower.rfind("</body>") {
        Some(end) => {
            out.push_str(&html[..end]);
            out.push_str(&toolbar);
            out.push_str(&html[end..]);
        }
        // No closing tag: the parser puts trailing content in the body anyway.
        None => {
            out.push_str(html);
            out.push_str(&toolbar);
        }
    }
    out
}

/// Move the toolbar out of the page's `<body>` to be `<html>`'s own child, in
/// the band the chrome CSS reserves at the top of the root.
///
/// The parser cannot put it there -- content after `</body>` goes back into the
/// body -- so it is moved once the document is built. There it is placed
/// against the root, whatever the page does to its body (Taffy positions an
/// absolute box against its parent, with no containing-block search, so inside
/// a `max-width` body the toolbar would be as narrow as the body). And it is
/// outside every JS re-render, which replaces only the body's contents.
fn mount_chrome(doc: &mut HtmlDocument) {
    let root = doc.root_element().id;
    for id in ["#__zipp_chrome", "#__zipp_console"] {
        if let Ok(Some(node)) = doc.query_selector(id) {
            doc.mutate().append_children(root, &[node]);
        }
    }
}

/// Parse a page built by `render_page` and put its chrome in place, with any
/// WebGL frame the page drew while it settled.
fn build_document(final_html: &str, config: DocumentConfig) -> HtmlDocument {
    let mut doc = HtmlDocument::from_html(final_html, config);
    mount_chrome(&mut doc);
    canvas::forget_shown();
    apply_canvas_frames(&mut doc);
    doc
}

/// Show the WebGL and WebGPU frames presented since the last call: each becomes the
/// image of its canvas element (found by `data-flid`), which is painted, and
/// sized when the canvas has no size of its own, like any image. True when
/// there were any.
pub(crate) fn apply_canvas_frames(doc: &mut BaseDocument) -> bool {
    show_canvas_frames(doc, canvas::take_frames())
}

/// After a re-render rebuilt the body: each canvas shows its frame again.
pub(crate) fn reapply_canvas_frames(doc: &mut BaseDocument) {
    show_canvas_frames(doc, canvas::shown_frames());
}

fn show_canvas_frames(doc: &mut BaseDocument, frames: Vec<(u32, canvas::Shown)>) -> bool {
    use zipp_dom::node::{ImageData, RasterImageData, SpecialElementData};
    if frames.is_empty() {
        return false;
    }
    let flids = event_bridge::build_flid_map(doc);
    for (flid, (width, height, rgba)) in frames {
        let Some(&node) = flids.get(&flid) else {
            continue;
        };
        if let Some(element) = doc.get_node_mut(node).and_then(|n| n.element_data_mut()) {
            let image = RasterImageData::new(width, height, rgba);
            element.special_data = SpecialElementData::Image(Box::new(ImageData::Raster(image)));
        }
    }
    true
}

/// Split a serialized `<body ...>...</body>` element into its attribute text
/// (with its leading space, or empty) and its contents. Anything else is all
/// contents.
fn split_body_element(html: &str) -> (&str, &str) {
    let start = html
        .match_indices("<body")
        .map(|(i, _)| i)
        .find(|&i| matches!(html.as_bytes().get(i + 5), Some(b'>' | b' ' | b'\t' | b'\n' | b'\r' | b'/')));
    match (start, html.rfind("</body>")) {
        (Some(start), Some(end)) => {
            let Some(open_end) = html[start..].find('>').map(|e| start + e) else {
                return ("", html);
            };
            if open_end >= end {
                return ("", html);
            }
            (html[start + 5..open_end].trim_end_matches('/'), &html[open_end + 1..end])
        }
        _ => ("", html),
    }
}

/// The page's new body contents after a JS re-render.
///
/// `__getRenderedHTML()` hands back the page's own `<body>` element. Strip that
/// wrapper and re-absolutise its URLs against the page (the initial load does
/// this; skipping it broke every relative image on the first click). The chrome
/// is not part of it: it lives beside the body, and a re-render replaces only
/// the body's contents.
pub(crate) fn rerender_body(rendered_body: &str, page_url: &str) -> String {
    let (_, inner) = split_body_element(rendered_body);
    make_urls_absolute(inner, page_url)
}

/// The toolbar: back, forward, reload and the address bar.
///
/// The address bar is a GET form back to the current page; the navigation
/// provider picks `__nav_url` out of the submitted URL. It is in normal flow of
/// nothing -- absolutely placed in the root's band -- so it scrolls away with
/// the page (`position:sticky` is inert here: the Stylo->Taffy conversion maps
/// Sticky to Relative). Real chrome belongs outside the document; until then a
/// correctly-rendering page is worth more than a pinned URL bar.
///
/// It is the top stacking layer. Anything a page parks just above its content
/// -- a skip link at `translateY(-150%)`, a header slid out of view -- lands in
/// the band; a real browser clips that at the viewport edge, here the chrome
/// has to cover it, or a `z-index:100` skip link paints over the URL bar.
/// The console's input line, inside its form (put back empty after each run).
pub(crate) const CONSOLE_INPUT: &str = r#"<span style="color:#1c7ed6">›</span><input id="__zipp_console_input" type="text" name="__js" placeholder="Run JavaScript in the page, then Enter" style="flex:1;min-width:0;height:22px;border:0;outline:none;font-family:Consolas,'Liberation Mono',monospace;font-size:12px;background:white;color:#212529" />"#;

/// The console button's icon, and its badge counting the page's errors.
pub(crate) fn console_button_inner(badge: &str) -> String {
    let display = if badge.is_empty() { "none" } else { "inline-block" };
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m4 17 6-6-6-6"/><path d="M12 19h8"/></svg><span id="__zipp_console_badge" style="display:{display};min-width:16px;padding:0 4px;border-radius:8px;background:#e03131;color:white;font-size:11px;line-height:16px;text-align:center">{badge}</span>"#
    )
}

fn toolbar_markup(current_url: &str) -> String {
    let css = chrome_css(console::is_open());
    let button = console_button_inner(&console::badge_text());
    let counts = console::counts_text();
    let lines = console::lines_html();
    format!(
        r#"<form id="__zipp_chrome" action="{current_url}" method="GET" style="position:absolute;left:0;right:0;z-index:2147483647;color:#212529;height:42px;background:#f8f9fa;border-bottom:1px solid #dee2e6;display:flex;align-items:center;padding:4px 8px;gap:6px;font-family:Arial,sans-serif;font-size:14px;line-height:normal;text-align:left;box-sizing:border-box;margin:0">
    <style id="__zipp_chrome_css">{css}</style>
    <a href="zipp-nav:back" title="Back (Alt+Left)" style="display:flex;align-items:center;padding:3px;border-radius:4px;text-decoration:none;color:#495057"><svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m15 18-6-6 6-6"/></svg></a>
    <a href="zipp-nav:forward" title="Forward (Alt+Right)" style="display:flex;align-items:center;padding:3px;border-radius:4px;text-decoration:none;color:#495057"><svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg></a>
    <a href="zipp-nav:reload" title="Reload (F5)" style="display:flex;align-items:center;padding:3px;border-radius:4px;text-decoration:none;color:#495057"><svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/></svg></a>
    <input type="text" name="__nav_url" value="{current_url}" style="flex:1;min-width:0;height:32px;padding:0 12px;border:1px solid #ced4da;border-radius:8px;font-family:Arial,sans-serif;font-size:14px;font-weight:normal;font-style:normal;letter-spacing:normal;background:white;color:#212529;outline:none;box-sizing:border-box" />
    <a id="__zipp_console_button" href="zipp-console:toggle" title="Console (F12)" style="display:flex;align-items:center;gap:4px;padding:4px 6px;border-radius:4px;text-decoration:none;color:#495057">{button}</a>
  </form>
  <div id="__zipp_console" style="position:absolute;left:0;right:0;z-index:2147483646;flex-direction:column;background:white;border-bottom:2px solid #ced4da;font-family:Consolas,'Liberation Mono',monospace;font-size:12px;line-height:16px;color:#212529;text-align:left;box-sizing:border-box;margin:0">
    <div style="display:flex;align-items:center;gap:14px;padding:4px 10px;background:#f1f3f5;border-bottom:1px solid #dee2e6;font-family:Arial,sans-serif;font-size:12px;flex-shrink:0">
      <b>Console</b><span id="__zipp_console_counts" style="color:#495057">{counts}</span><span style="flex:1"></span>
      <a href="zipp-console:clear" id="__zipp_console_clear" style="color:#1c7ed6;text-decoration:none">Clear</a>
      <a href="zipp-console:toggle" id="__zipp_console_close" style="color:#1c7ed6;text-decoration:none">Close</a>
    </div>
    <div id="__zipp_console_lines" style="flex:1;overflow:auto;min-height:0">{lines}</div>
    <form id="__zipp_console_form" action="{current_url}" method="GET" style="display:flex;align-items:center;gap:6px;margin:0;padding:3px 10px;border-top:1px solid #dee2e6;flex-shrink:0">{CONSOLE_INPUT}</form>
  </div>"#
    )
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

/// Resolve every `src` and `href` attribute in `html` against `page_url`, as
/// the document itself would: `//cdn.example/x.png`, `/x.png`, `x.png` and
/// `../x.png` alike. Fragment-only (`#top`) and already-absolute values
/// (`https:`, `data:`, `mailto:`, ...) are left as they are.
///
/// This used to splice the page's origin in front of anything starting with
/// `/`, so a protocol-relative `//upload.wikimedia.org/...` became
/// `https://en.wikipedia.org//upload.wikimedia.org/...` -- why Wikipedia's
/// article images never loaded. It also matched inside `data-src="`, and put
/// bare relative paths at the site root rather than beside the page.
fn make_urls_absolute(html: &str, page_url: &str) -> String {
    let Ok(base) = url::Url::parse(page_url) else {
        return html.to_string();
    };
    let mut out = String::with_capacity(html.len() + 256);
    let mut copied = 0;
    let mut pos = 0;
    while let Some(found) = html[pos..].find("=\"") {
        let eq = pos + found;
        let value_start = eq + 2;
        let Some(len) = html[value_start..].find('"') else {
            break;
        };
        let value_end = value_start + len;
        // The attribute name runs back from `=` to the whitespace before it, so
        // `data-src` is not mistaken for `src`.
        let name_start = html[..eq]
            .rfind(|c: char| c.is_ascii_whitespace())
            .map_or(eq, |ws| ws + 1);
        let name = &html[name_start..eq];
        if name.eq_ignore_ascii_case("src") || name.eq_ignore_ascii_case("href") {
            if let Some(absolute) = absolutize(&base, &html[value_start..value_end]) {
                out.push_str(&html[copied..value_start]);
                out.push_str(&absolute);
                copied = value_end;
            }
        }
        pos = value_end + 1;
    }
    out.push_str(&html[copied..]);
    out
}

/// `value` resolved against `base`, or `None` to leave it untouched.
fn absolutize(base: &url::Url, value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.starts_with('#') {
        return None;
    }
    // Already absolute: a scheme before any `/`, `?` or `#`.
    let scheme_end = value.find(':');
    let path_start = value.find(['/', '?', '#']);
    if scheme_end.is_some_and(|colon| path_start.is_none_or(|p| colon < p)) {
        return None;
    }
    base.join(value).ok().map(|u| u.to_string())
}

/// Whether the OS asks apps for a dark colour scheme.
///
/// The window hands Stylo the OS theme for `prefers-color-scheme`, but the
/// page's JS runs before there is a window to ask, so read the same setting
/// directly. `ZIPP_COLOR_SCHEME=light|dark` overrides it, which keeps
/// screenshots comparable across machines.
fn system_prefers_dark() -> bool {
    if let Ok(scheme) = std::env::var("ZIPP_COLOR_SCHEME") {
        return scheme.eq_ignore_ascii_case("dark");
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
        let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
        let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
        let value = wide("AppsUseLightTheme");
        let mut data: u32 = 1;
        let mut size = std::mem::size_of::<u32>() as u32;
        // SAFETY: both strings are NUL-terminated and outlive the call; `data`
        // and `size` describe a writable DWORD, which RRF_RT_REG_DWORD restricts
        // the result to.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                (&mut data as *mut u32).cast(),
                &mut size,
            )
        };
        return status == 0 && data == 0;
    }
    #[allow(unreachable_code)]
    false
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

/// The value of attribute `name` in an HTML start tag (`<script src=...>`),
/// read as the HTML parser reads it: double-, single- or un-quoted, the name
/// matched whole and without regard to case. It used to find only
/// `name="value"`, by substring -- so a minified `<script type=module
/// src=/assets/app.js>` was never loaded, and asking for `src` found
/// `data-src="..."` first.
fn attr(tag: &str, name: &str) -> Option<String> {
    let b = tag.as_bytes();
    let space = |c: u8| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c');
    // Past `<` and the tag name.
    let mut i = 1;
    while i < b.len() && !space(b[i]) && b[i] != b'>' && b[i] != b'/' {
        i += 1;
    }
    loop {
        while i < b.len() && (space(b[i]) || b[i] == b'/') {
            i += 1;
        }
        if i >= b.len() || b[i] == b'>' {
            return None;
        }
        let name_start = i;
        // The first character may be `=`; after that it ends the name.
        i += 1;
        while i < b.len() && !space(b[i]) && !matches!(b[i], b'=' | b'>' | b'/') {
            i += 1;
        }
        let found = &tag[name_start..i];
        while i < b.len() && space(b[i]) {
            i += 1;
        }
        let mut value = "";
        if i < b.len() && b[i] == b'=' {
            i += 1;
            while i < b.len() && space(b[i]) {
                i += 1;
            }
            if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                let quote = b[i];
                let start = i + 1;
                let end = tag[start..].bytes().position(|c| c == quote).map_or(b.len(), |e| start + e);
                value = &tag[start..end];
                i = end + 1;
            } else {
                let start = i;
                while i < b.len() && !space(b[i]) && b[i] != b'>' {
                    i += 1;
                }
                value = &tag[start..i];
            }
        }
        if found.eq_ignore_ascii_case(name) {
            // Decode common HTML entities in attribute values; `&amp;` last, so
            // `&amp;lt;` stays the text `&lt;`.
            return Some(
                value
                    .replace("&lt;", "<")
                    .replace("&gt;", ">")
                    .replace("&quot;", "\"")
                    .replace("&#39;", "'")
                    .replace("&amp;", "&"),
            );
        }
    }
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
/// The text of the page's `<title>`, its common character references decoded.
fn page_title(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let Some(open) = lower.find("<title") else {
        return String::new();
    };
    let Some(start) = lower[open..].find('>').map(|i| open + i + 1) else {
        return String::new();
    };
    let end = lower[start..].find("</title").map_or(start, |i| start + i);
    let text = html[start..end].split_whitespace().collect::<Vec<_>>().join(" ");
    text.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&")
}

/// `s` inside a single-quoted JS string literal.
fn js_quoted(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'").replace('\n', "\\n").replace('\r', "")
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_keeps_its_own_body() {
        let page = r#"<!doctype html><html lang=en><head><style>body{margin:auto}</style></head><body class="home" style="padding:20px"><p>Hi</p></body></html>"#;
        let out = wrap_with_toolbar(page, "https://example.com/");
        assert!(out.starts_with("<!doctype html><html lang=en>"));
        assert!(out.contains(r#"<body class="home" style="padding:20px"><p>Hi</p><form id="__zipp_chrome""#));
        // The console panel follows the toolbar, closed.
        assert!(out.contains(r#"<div id="__zipp_console""#));
        assert!(out.ends_with("</div></body></html>"));

        // A page with no doctype still gets standards mode, and one with no
        // closing body tag still gets its toolbar.
        let out = wrap_with_toolbar("<p>bare", "https://example.com/");
        assert!(out.starts_with("<!DOCTYPE html><p>bare<form id=\"__zipp_chrome\""));
    }

    #[test]
    fn body_element_splits_into_attributes_and_contents() {
        assert_eq!(
            split_body_element(r#"<body data-flid="4" class="a"><div>x</div></body>"#),
            (r#" data-flid="4" class="a""#, "<div>x</div>")
        );
        assert_eq!(split_body_element("<body><b>y</b></body>"), ("", "<b>y</b>"));
        // Not a body element: all contents.
        assert_eq!(split_body_element("<bodyguard>z</bodyguard>"), ("", "<bodyguard>z</bodyguard>"));
        assert_eq!(split_body_element("<p>w</p>"), ("", "<p>w</p>"));
    }

    #[test]
    fn attributes_read_as_the_parser_reads_them() {
        let tag = r#"<script type=module data-src="no.js" SRC='/a.js' defer>"#;
        assert_eq!(attr(tag, "src").as_deref(), Some("/a.js"));
        assert_eq!(attr(tag, "type").as_deref(), Some("module"));
        assert_eq!(attr(tag, "defer").as_deref(), Some(""));
        assert_eq!(attr(tag, "async"), None);
        assert_eq!(attr("<script src=/s.js>", "src").as_deref(), Some("/s.js"));
        assert_eq!(attr(r#"<script src="/x.js?a=1&amp;b=2">"#, "src").as_deref(), Some("/x.js?a=1&b=2"));
        assert_eq!(attr("<script>", "src"), None);
        // Unterminated quote: the rest of the tag.
        assert_eq!(attr(r#"<script src="/y.js"#, "src").as_deref(), Some("/y.js"));
    }

    #[test]
    fn urls_resolve_against_the_page() {
        let page = "https://en.wikipedia.org/wiki/Web_browser";
        let html = r##"<img src="//upload.wikimedia.org/a.png"><img data-src="/lazy.png" src="/b.png"><a href="#top">t</a><a href="mailto:x@y.z">m</a><img src="c.png"><a href="../d">d</a><img src="data:image/png;base64,AA">"##;
        assert_eq!(
            make_urls_absolute(html, page),
            r##"<img src="https://upload.wikimedia.org/a.png"><img data-src="/lazy.png" src="https://en.wikipedia.org/b.png"><a href="#top">t</a><a href="mailto:x@y.z">m</a><img src="https://en.wikipedia.org/wiki/c.png"><a href="https://en.wikipedia.org/d">d</a><img src="data:image/png;base64,AA">"##
        );
    }

    #[test]
    fn prerender_only_follows_page_links() {
        assert!(looks_like_page("https://lance.name"));
        assert!(looks_like_page("https://lance.name/about/"));
        assert!(looks_like_page("https://example.com/post.html"));
        assert!(looks_like_page("https://example.com/v1.2/notes"));
        assert!(!looks_like_page("https://lance.name/assets/index-Car7W7dH.css"));
        assert!(!looks_like_page("https://lance.name/favicon.svg"));
        assert!(!looks_like_page("https://lance.name/downloads/Lance-Baker-Resume.pdf"));
        assert!(!looks_like_page("not a url"));
    }
}
