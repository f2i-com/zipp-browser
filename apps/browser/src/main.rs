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

mod async_fetch;
mod crypto;
mod esm;
mod event_bridge;
mod js_opt;
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
mod devtools;
mod history;
mod properties_page;
mod settings;
mod tabs;
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
    /// The URL of the page currently loaded, so a relative `window.open` or
    /// `location.assign` from the page can be resolved against it.
    static ref CURRENT_URL: Mutex<String> = Mutex::new(String::new());
    /// Pages as fetched, before their JS runs, tagged with the cookie jar
    /// generation at fetch time so any login/logout/cookie change automatically
    /// invalidates stale HTML snapshots. Bounded to 24 entries.
    static ref PAGE_CACHE: Mutex<HashMap<String, (u64, String)>> = Mutex::new(HashMap::new());
    /// Shared blocking HTTP client -- reused across all fetch() calls to
    /// take advantage of connection pooling and avoid per-request TLS setup.
    /// For what a page asks of another site: the same, with no cookies, as a
    /// page does not get to act as the person on a site it is not.
    static ref ANONYMOUS_CLIENT: reqwest::blocking::Client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .danger_accept_invalid_certs(std::env::var_os("ZIPP_ACCEPT_INVALID_CERTS").is_some())
        .user_agent(USER_AGENT)
        .build()
        .expect("http client");
    static ref HTTP_CLIENT: reqwest::blocking::Client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        // A proxy on the path that signs what it passes on is told apart by
        // asking for it (`ZIPP_ACCEPT_INVALID_CERTS`), not by trusting anyone.
        .danger_accept_invalid_certs(std::env::var_os("ZIPP_ACCEPT_INVALID_CERTS").is_some())
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
    static ref ANONYMOUS_MOBILE_CLIENT: reqwest::blocking::Client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .danger_accept_invalid_certs(std::env::var_os("ZIPP_ACCEPT_INVALID_CERTS").is_some())
        .user_agent(MOBILE_USER_AGENT)
        .build()
        .expect("mobile http client");
    static ref MOBILE_HTTP_CLIENT: reqwest::blocking::Client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .danger_accept_invalid_certs(std::env::var_os("ZIPP_ACCEPT_INVALID_CERTS").is_some())
        .user_agent(MOBILE_USER_AGENT)
        .cookie_provider(zipp_shell::cookies::jar())
        .build()
        .expect("failed to build mobile HTTP client");
}

/// Active HTTP client matching Desktop vs Mobile View (`--mobile` / toolbar toggle).
pub(crate) fn http_client() -> &'static reqwest::blocking::Client {
    if is_mobile_view() { &MOBILE_HTTP_CLIENT } else { &HTTP_CLIENT }
}

/// Active anonymous (cookie-free) HTTP client matching Desktop vs Mobile View.
pub(crate) fn anonymous_http_client() -> &'static reqwest::blocking::Client {
    if is_mobile_view() { &ANONYMOUS_MOBILE_CLIENT } else { &ANONYMOUS_CLIENT }
}

/// Default desktop viewport (logical CSS pixels).
const INITIAL_WINDOW_SIZE: (u32, u32) = (1280, 800);

/// Default mobile viewport (logical CSS pixels), matching Android Pixel 9a
/// (`1080×2424` @ `2.625×` DPR) used by the SoftN Mobile runtime.
const MOBILE_WINDOW_SIZE: (u32, u32) = (412, 915);

/// Mobile User-Agent string matching the SoftN Mobile Android viewport.
const MOBILE_USER_AGENT: &str = "Mozilla/5.0 (Linux; Android 16; Pixel 9a) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Mobile Safari/537.36 Zipp/1.0";

static MOBILE_VIEW: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static SHELL_PROXY: std::sync::OnceLock<ShellProxy> = std::sync::OnceLock::new();

pub(crate) fn is_mobile_view() -> bool {
    MOBILE_VIEW.load(std::sync::atomic::Ordering::Relaxed)
}

/// Toggle between Desktop (`1280×800`) and Mobile (`412×915`) viewports at runtime.
pub(crate) fn toggle_mobile_view() -> (bool, (u32, u32)) {
    let now_mobile = !MOBILE_VIEW.fetch_xor(true, std::sync::atomic::Ordering::Relaxed);
    let target = if now_mobile { MOBILE_WINDOW_SIZE } else { INITIAL_WINDOW_SIZE };
    if let Some(proxy) = SHELL_PROXY.get() {
        proxy.send_event(zipp_shell::ShellEvent::ResizeWindow {
            width: target.0,
            height: target.1,
        });
    }
    eprintln!(
        "[BROWSER] View mode -> {} ({}x{})",
        if now_mobile { "mobile" } else { "desktop" },
        target.0,
        target.1,
    );
    (now_mobile, target)
}

/// Parse a `WxH` viewport specifier (e.g. `412x915` or `1440x900`).
fn parse_viewport(spec: &str) -> Option<(u32, u32)> {
    let (w, h) = spec.split_once('x').or_else(|| spec.split_once('X'))?;
    let w: u32 = w.trim().parse().ok()?;
    let h: u32 = h.trim().parse().ok()?;
    if w >= 240 && h >= 240 && w <= 7680 && h <= 4320 {
        Some((w, h))
    } else {
        None
    }
}

const MAX_PAGE_CACHE_ENTRIES: usize = 24;

fn page_cache_get(url: &str) -> Option<String> {
    if zipp_shell::cache::global().is_disabled() {
        return None;
    }
    let cookie_gen = zipp_shell::cookies::generation();
    let mut cache = PAGE_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    match cache.get(url) {
        Some((entry_gen, html)) if *entry_gen == cookie_gen => Some(html.clone()),
        Some(_) => {
            cache.remove(url);
            None
        }
        None => None,
    }
}

fn page_cache_put(url: String, html: String) {
    if zipp_shell::cache::global().is_disabled() {
        return;
    }
    let cookie_gen = zipp_shell::cookies::generation();
    let mut cache = PAGE_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if cache.len() >= MAX_PAGE_CACHE_ENTRIES
        && !cache.contains_key(&url)
        && let Some(first) = cache.keys().next().cloned()
    {
        cache.remove(&first);
    }
    cache.insert(url, (cookie_gen, html));
}

fn page_cache_clear() {
    PAGE_CACHE.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// Clear all HTTP/asset/page/decoded-image caches (memory + persistent disk).
pub(crate) fn clear_browser_cache() -> (usize, u64) {
    page_cache_clear();
    esm::clear_cache();
    clear_failed_iframes();
    tabs::clear_cached_html();
    canvas2d::clear_image_cache();
    let (entries, bytes) = zipp_shell::cache::clear();
    let mb = bytes as f64 / (1024.0 * 1024.0);
    let msg = format!("Cleared cache ({entries} entries, {mb:.2} MB)");
    eprintln!("[CACHE] {msg}");
    console::push(console::Level::Log, msg);
    (entries, bytes)
}

/// Clear caches, cookies, and site storage (`localStorage`, `sessionStorage`, IndexedDB).
pub(crate) fn clear_all_site_data() -> (usize, u64, usize) {
    let (entries, bytes) = clear_browser_cache();
    let cookies = zipp_shell::cookies::clear_all();
    site_storage::clear_all();
    BLOB_STORE.lock().unwrap_or_else(|e| e.into_inner()).clear();
    let msg = format!("Cleared site data ({cookies} cookies + localStorage/sessionStorage/IndexedDB)");
    eprintln!("[DATA] {msg}");
    console::push(console::Level::Log, msg);
    (entries, bytes, cookies)
}

#[derive(Debug, PartialEq, Eq)]
enum CliAction {
    Help,
    Version,
    ClearCacheOnly,
    ClearDataOnly,
    TestJs,
    TestVm,
    TestCompat,
    Screenshot {
        out: String,
        url: String,
        width: u32,
        height: u32,
        mobile: bool,
        incognito: bool,
        no_chrome: bool,
        clear_cache: bool,
        clear_data: bool,
        no_cache: bool,
    },
    Reftest { pairs: String, results: String },
    Run {
        url: String,
        mobile: bool,
        incognito: bool,
        viewport: Option<(u32, u32)>,
        kiosk: bool,
        app: bool,
        app_files: Vec<String>,
        clear_cache: bool,
        clear_data: bool,
        no_cache: bool,
    },
}

fn parse_cli_args(args: &[String]) -> Result<CliAction, String> {
    let mut mobile = false;
    let mut incognito = false;
    let mut custom_viewport: Option<(u32, u32)> = None;
    let mut kiosk = false;
    let mut app = false;
    let mut clear_cache = false;
    let mut clear_data = false;
    let mut no_cache = false;
    let mut screenshot_mode = false;
    let mut positional: Vec<String> = Vec::new();
    let mut i = 0;
    let mut end_of_opts = false;

    while i < args.len() {
        let a = args[i].as_str();
        if end_of_opts || (!positional.is_empty() && (app || screenshot_mode)) {
            positional.push(args[i].clone());
            i += 1;
            continue;
        }
        if a == "--" {
            end_of_opts = true;
            i += 1;
            continue;
        }
        match a {
            "-h" | "--help" => return Ok(CliAction::Help),
            "-V" | "--version" => return Ok(CliAction::Version),
            "--test-js" => return Ok(CliAction::TestJs),
            "--test-vm" => return Ok(CliAction::TestVm),
            "--test-compat" => return Ok(CliAction::TestCompat),
            "--reftest" => {
                let pairs = args.get(i + 1).cloned().ok_or("usage: zipp-browser --reftest pairs.tsv results.tsv")?;
                let results = args.get(i + 2).cloned().ok_or("usage: zipp-browser --reftest pairs.tsv results.tsv")?;
                return Ok(CliAction::Reftest { pairs, results });
            }
            "--mobile" => {
                mobile = true;
                i += 1;
            }
            "--incognito" | "--private" => {
                incognito = true;
                i += 1;
            }
            "--portable" => {
                zipp_shell::profile::set_portable(true);
                i += 1;
            }
            "--viewport" => {
                let spec = args.get(i + 1).ok_or("--viewport requires WxH (e.g. --viewport 412x915)")?;
                custom_viewport = Some(parse_viewport(spec).ok_or_else(|| format!("invalid --viewport {spec:?}, expected WxH"))?);
                i += 2;
            }
            "--kiosk" => {
                kiosk = true;
                i += 1;
            }
            "--app" => {
                app = true;
                i += 1;
            }
            "--clear-cache" => {
                clear_cache = true;
                i += 1;
            }
            "--clear-data" => {
                clear_data = true;
                i += 1;
            }
            "--no-cache" => {
                no_cache = true;
                i += 1;
            }
            "--screenshot" => {
                screenshot_mode = true;
                i += 1;
            }
            _ => {
                positional.push(args[i].clone());
                i += 1;
            }
        }
    }

    if screenshot_mode {
        let out = positional.first().cloned().unwrap_or_else(|| "screenshot.png".to_string());
        let url = positional.get(1).cloned().unwrap_or_else(|| settings::DEFAULT_START_PAGE.to_string());
        let (def_w, def_h) = custom_viewport.unwrap_or(if mobile { MOBILE_WINDOW_SIZE } else { (1280, 2000) });
        let width = positional.get(2).and_then(|s| s.parse().ok()).unwrap_or(def_w);
        let height = positional.get(3).and_then(|s| s.parse().ok()).unwrap_or(def_h);
        return Ok(CliAction::Screenshot {
            out,
            url,
            width,
            height,
            mobile,
            incognito,
            no_chrome: app || kiosk,
            clear_cache,
            clear_data,
            no_cache,
        });
    }

    if positional.is_empty() && !mobile && !incognito && !kiosk && !app && custom_viewport.is_none() {
        if clear_data {
            return Ok(CliAction::ClearDataOnly);
        }
        if clear_cache {
            return Ok(CliAction::ClearCacheOnly);
        }
    }

    let url = positional.first().cloned().unwrap_or_default();
    let app_files = if app && positional.len() > 1 {
        positional[1..].to_vec()
    } else {
        Vec::new()
    };
    Ok(CliAction::Run {
        url,
        mobile,
        incognito,
        viewport: custom_viewport,
        kiosk,
        app,
        app_files,
        clear_cache,
        clear_data,
        no_cache,
    })
}

fn print_usage() {
    println!(
        "ZIPP Browser v{} (Part of the ZIPP Project · zipp-vm 0.0.40 + Stylo + Taffy + TinySkia)\n\
         \n\
         USAGE:\n\
         \x20 zipp-browser [OPTIONS] [URL]\n\
         \x20 zipp-browser [--mobile] [--viewport WxH] --app <URL> [FILE ...]\n\
         \x20 zipp-browser [--mobile] [--viewport WxH] --screenshot <OUT.png> [URL] [W] [H]\n\
         \n\
         OPTIONS:\n\
         \x20 --incognito           Private window: in-memory cookies/cache/storage only, zero history saved\n\
         \x20 --portable            Store settings, cookies, history, and cache in ./data next to the executable\n\
         \x20 --mobile              Open in Mobile View (412x915 Pixel 9a logical viewport + mobile UA/touch)\n\
         \x20 --viewport <WxH>      Set initial logical viewport size (e.g. 390x844, 1440x900)\n\
         \x20 --app                 Chromeless application window (restricted local file access)\n\
         \x20 --kiosk               Full-screen DRM/KMS mode (Linux only)\n\
         \x20 --clear-cache         Clear persistent HTTP/asset disk cache (exits if no URL given)\n\
         \x20 --clear-data          Clear cache, cookies, and site storage (exits if no URL given)\n\
         \x20 --no-cache            Bypass the HTTP cache for this run\n\
         \x20 --screenshot          Headless render to PNG and exit\n\
         \x20 --test-js             Run built-in DOM/browser JS test suite\n\
         \x20 --test-compat         Run JS compatibility test suite\n\
         \x20 --test-vm             Run quick zipp-vm sanity check\n\
         \x20 -h, --help            Show this help message\n\
         \x20 -V, --version         Print version information\n\
         \n\
         SHORTCUTS:\n\
         \x20 Ctrl+Shift+N          Open a new Incognito window\n\
         \x20 Ctrl+,                Open Properties panel (Start Page, Cache, History)\n\
         \x20 Ctrl+H                Open Browsing History\n\
         \x20 Ctrl+Shift+M          Toggle Desktop <-> Mobile View live in the browser window\n\
         \x20 Ctrl+Shift+Delete     Clear browser cache and reload\n\
         \x20 Ctrl+Shift+R          Hard reload (bypass cache)\n\
         \x20 F12                   Toggle developer console\n\
         \x20 Ctrl+L / F6           Focus address bar\n\
         \x20 Alt+Home              Go to configured Start Page\n\
         \x20 Alt+Left / Alt+Right  History Back / Forward\n\
         \x20 F5 / Ctrl+R           Reload page",
        env!("CARGO_PKG_VERSION")
    );
}

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
    if is_mobile_view() {
        return js
            .replace(USER_AGENT, MOBILE_USER_AGENT)
            .replace("Windows NT 10.0; Win64; x64", "Linux; Android 16; Pixel 9a")
            .replace("platform:\"Win32\"", "platform:\"Linux armv8l\"")
            .replace("platform: \"Win32\"", "platform: \"Linux armv8l\"")
            .replace("maxTouchPoints:0", "maxTouchPoints:5")
            .replace("orientation:{type:\"landscape-primary\",angle:0}", "orientation:{type:\"portrait-primary\",angle:0}");
    }
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
    // Signals blocked by what started this one (zipp-init blocks its own, and
    // Rust's Command passes a mask on) would be blocked in every thread here
    // and every program started from here: a Ctrl+C in a terminal that never
    // reaches its program, a SIGTERM at shutdown that never arrives.
    #[cfg(target_os = "linux")]
    // SAFETY: an empty set, made and given here.
    unsafe {
        let mut none: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut none);
        libc::pthread_sigmask(libc::SIG_SETMASK, &none, std::ptr::null_mut());
    }

    let raw_args: Vec<String> = std::env::args().skip(1).collect();
    let action = match parse_cli_args(&raw_args) {
        Ok(action) => action,
        Err(msg) => {
            eprintln!("error: {msg}");
            std::process::exit(2);
        }
    };

    zipp_shell::net::set_file_gate(page_may_read);
    zipp_shell::net::set_blob_resolver(lookup_blob_bytes);
    let (raw_url, mobile, custom_viewport, kiosk, app, app_files) = match action {
        CliAction::Help => {
            print_usage();
            return;
        }
        CliAction::Version => {
            println!("zipp-browser {} (ZIPP v0.0.40 · Apache-2.0)", env!("CARGO_PKG_VERSION"));
            return;
        }
        CliAction::ClearCacheOnly => {
            let (entries, bytes) = clear_browser_cache();
            println!("Cleared browser cache: {entries} entries ({:.2} MB freed).", bytes as f64 / (1024.0 * 1024.0));
            return;
        }
        CliAction::ClearDataOnly => {
            let (entries, bytes, cookies) = clear_all_site_data();
            println!(
                "Cleared browser site data: {entries} cache entries ({:.2} MB), {cookies} cookies, and site storage.",
                bytes as f64 / (1024.0 * 1024.0)
            );
            return;
        }
        CliAction::TestJs => {
            let ok = run_js_suite("TEST", include_str!("js_tests.js"), 10);
            std::process::exit(if ok { 0 } else { 1 });
        }
        CliAction::TestVm => {
            run_vm_test();
            return;
        }
        CliAction::TestCompat => {
            let ok = run_js_suite("COMPAT", include_str!("js_compat_tests.js"), 20);
            std::process::exit(if ok { 0 } else { 1 });
        }
        CliAction::Screenshot { out, url, width, height, mobile, incognito, no_chrome, clear_cache, clear_data, no_cache } => {
            if incognito {
                zipp_shell::profile::set_incognito(true);
            }
            if clear_data {
                clear_all_site_data();
            } else if clear_cache {
                clear_browser_cache();
            }
            settings::apply_runtime(no_cache);
            if mobile {
                MOBILE_VIEW.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            if no_chrome {
                NO_CHROME.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            let ok = screenshot(&normalize_url(&url), &out, width, height);
            save_profile();
            std::process::exit(if ok { 0 } else { 1 });
        }
        CliAction::Reftest { pairs, results } => {
            let ok = reftest_batch(&pairs, &results);
            save_profile();
            std::process::exit(if ok { 0 } else { 1 });
        }
        CliAction::Run { url, mobile, incognito, viewport, kiosk, app, app_files, clear_cache, clear_data, no_cache } => {
            if incognito {
                zipp_shell::profile::set_incognito(true);
            }
            if clear_data {
                clear_all_site_data();
            } else if clear_cache {
                clear_browser_cache();
            }
            settings::apply_runtime(no_cache);
            (url, mobile, viewport, kiosk, app, app_files)
        }
    };

    if mobile {
        MOBILE_VIEW.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    #[cfg(not(target_os = "linux"))]
    if kiosk {
        eprintln!("[BROWSER] --kiosk runs on Linux's DRM/KMS only");
        std::process::exit(2);
    }
    #[cfg(target_os = "linux")]
    let kms = kiosk.then(open_kms);
    let default_viewport = custom_viewport.unwrap_or(if mobile { MOBILE_WINDOW_SIZE } else { INITIAL_WINDOW_SIZE });
    #[cfg(target_os = "linux")]
    let viewport = kms.as_ref().map_or(default_viewport, |kms| {
        let (w, h) = kms.size();
        ((w as f64 / kms.scale_factor()) as u32, (h as f64 / kms.scale_factor()) as u32)
    });
    #[cfg(not(target_os = "linux"))]
    let viewport = default_viewport;
    if kiosk {
        NO_CHROME.store(true, std::sync::atomic::Ordering::Relaxed);
        // `--kiosk --app` runs a single isolated app full-screen on DRM/KMS
        // without granting machine APIs (`navigator.zipp`).
        system::KIOSK.store(!app, std::sync::atomic::Ordering::Relaxed);
        zipp_shell::give_keys_to_page();
    }

    let url = if raw_url.is_empty() {
        normalize_url(&settings::start_page())
    } else if raw_url.starts_with("http://")
        || raw_url.starts_with("https://")
        || raw_url.starts_with("file://")
        || raw_url.starts_with("zipp:")
        || raw_url.starts_with("about:")
    {
        normalize_url(&raw_url)
    } else if let Ok(canon) = std::fs::canonicalize(&raw_url) {
        let s = canon.to_string_lossy().replace('\\', "/");
        let s = s.strip_prefix("//?/").unwrap_or(&s);
        format!("file:///{}", s.trim_start_matches('/'))
    } else {
        normalize_url(&raw_url)
    };
    if app {
        NO_CHROME.store(true, std::sync::atomic::Ordering::Relaxed);
        zipp_shell::give_keys_to_page();
        allow_app_files(&url, app_files.into_iter());
    }
    eprintln!(
        "[BROWSER] Loading {url} ({}{} {}x{})",
        if zipp_shell::profile::is_incognito() { "incognito " } else { "" },
        if mobile { "mobile" } else { "desktop" },
        viewport.0,
        viewport.1,
    );
    // Overlap embedded font registration and tokio runtime startup with initial
    // document fetch + JS execution to reduce cold startup latency.
    let font_init = std::thread::Builder::new()
        .name("zipp-fonts".into())
        .spawn(load_fonts)
        .ok();
    let rt_init = std::thread::Builder::new()
        .name("zipp-rt-init".into())
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap()
        })
        .ok();
    // The window's page runs on this thread: its console & DevTools are shown,
    // and it may start workers.
    console::record_this_thread();
    devtools::record_this_thread();
    async_fetch::allow_on_this_thread();
    worker::allow_on_this_thread();

    // 1-3. Fetch, run the page's JS, post-process. The ScriptState is !Send, so
    //      this stays on the main thread to keep it alive for event dispatch.
    let initial_tab_id = tabs::init(&url);
    let t_fetch = std::time::Instant::now();
    let html = if let Some(internal) = internal_page_html(&url, None, None, None) {
        internal
    } else {
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
        match fetched {
            Ok(html) => {
                page_cache_put(url.clone(), html.clone());
                html
            }
            Err(e) => error_page(&url, &e),
        }
    };
    let fetch_ms = t_fetch.elapsed().as_millis();
    tabs::store_tab_html(initial_tab_id, &url, html.clone(), history::extract_html_title(&html).as_deref());
    let (final_html, event_map_json, js_state) =
        render_page(&url, &html, viewport, Some(fetch_ms));
    eprintln!("[BROWSER] Ready ({} bytes)", final_html.len());

    // 4. Join fonts loaded in parallel
    let font_ctx = font_init
        .and_then(|h| h.join().ok())
        .unwrap_or_else(load_fonts);

    // 5. Join async runtime built in parallel
    let rt = rt_init
        .and_then(|h| h.join().ok())
        .unwrap_or_else(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap()
        });
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
    let _ = SHELL_PROXY.set(proxy.clone());
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
    });
    // Published so `browser_host_call` can drive a navigation the PAGE asked
    // for (`window.open`, `location.assign`) -- it is a free function with no
    // other route to the shell.
    let _ = NAV_PROVIDER.set(nav_provider.clone() as Arc<dyn NavigationProvider>);
    // Where the first page asked to go as it loaded (its last request).
    if let Some(to) = EARLY_NAV.lock().unwrap_or_else(|e| e.into_inner()).take() {
        nav_provider.navigate_to(NavigationOptions::new(to, String::from("text/html"), 0));
    }
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
    sync_iframes(doc.as_mut(), &url, 0);
    focus_newtab_address_bar(&mut doc, &url);

    // Pre-render linked pages in background for instant navigation (skip for local kiosk/app shells).
    if !kiosk && !app && (url.starts_with("http://") || url.starts_with("https://")) {
        let prerender_url = url.clone();
        let prerender_html = final_html.clone();
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(move || prerender_linked_pages(&prerender_html, &prerender_url))
            .ok();
    }

    // 8. Open window with tiny-skia renderer
    let mut app = ZippApp::new(proxy, receiver);
    // Navigations come back here, on the main thread, to be built: the page's
    // JS runs and its event bridge is attached exactly as for the first page.
    // The fetching thread can do neither -- the script state is not `Send` --
    // which is why every page reached by a link used to render and then
    // respond to nothing.
    app.set_document_builder(Box::new(|url, html, config, viewport| {
        worker::new_page();
        worker::terminate_stale_pages();
        console::clear();
        system::new_page();
        #[cfg(target_os = "linux")]
        usb::new_page();
        let (final_html, event_map_json, js_state) = render_page(&url, &html, viewport, None);
        let mut doc = build_document(&final_html, config);
        attach_event_bridge(&mut doc, event_map_json, js_state, &url);
        sync_iframes(doc.as_mut(), &url, 0);
        focus_newtab_address_bar(&mut doc, &url);
        Box::new(doc)
    }));
    let renderer = TinySkiaWindowRenderer::new();
    let attributes = winit::window::WindowAttributes::default()
        .with_surface_size(winit::dpi::LogicalSize::new(viewport.0, viewport.1))
        .with_window_icon(if kiosk { None } else { app_icon(false) });
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
#[allow(clippy::large_enum_variant)]
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
    let event_loop = zipp_kms::EventLoop::new().unwrap_or_else(|e| {
        eprintln!("[BROWSER] cannot take the screen: {e}");
        std::process::exit(1);
    });
    // A panel mounted on its side (or one turned by hand): the page is laid
    // out at the picture's size and the renderer turns it onto the panel.
    let (rotation, (width, height)) = event_loop.screen_transform();
    zipp_renderer::tinyskia::set_screen_transform(rotation, width, height);
    zipp_kms::on_rotation(|rotation, (width, height)| zipp_renderer::tinyskia::set_screen_transform(rotation, width, height));
    event_loop
}

/// Before the browser exits: write out what is waiting to be saved to the
/// profile -- cookies, and what sites keep.
fn save_profile() {
    if zipp_shell::profile::is_incognito() {
        return;
    }
    zipp_shell::cookies::flush();
    site_storage::flush();
    settings::save();
    history::save();
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
        if let Ok(JsValue::String(s)) = state.eval_in_context(&format!("_e[{i}] || ''"))
            && !s.is_empty() {
                eprintln!("[{tag}]   {s}");
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

/// How many of a page's links are fetched ahead of being followed.
const MAX_PRERENDERED: usize = 8;

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

    let origin = url::Url::parse(base_url).ok().map(|u| u.origin());
    let same_origin = |link: &str| url::Url::parse(link).is_ok_and(|u| Some(u.origin()) == origin);

    // A few, as a request for each link is a request the person did not make
    // (a text match on the address also took example.com.evil.org for
    // example.com).
    for link in links.iter().filter(|l| same_origin(l)).take(MAX_PRERENDERED) {
        if page_cache_get(link).is_some() {
            continue;
        }
        eprintln!("[PRERENDER] {link}");
        // Downloads off: a background prefetch must never write a file.
        if let Ok(page) = viewers::fetch_document(link, false) {
            page_cache_put(link.clone(), page);
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
    if url.starts_with("blob:") {
        let (_, bytes) = lookup_blob(url).ok_or_else(|| format!("blob URL not found: {url}"))?;
        return Ok(String::from_utf8_lossy(&bytes).into_owned());
    }
    if url.starts_with("data:") {
        let (_, bytes) = decode_data_url(url).ok_or_else(|| format!("invalid data URL: {url}"))?;
        return Ok(String::from_utf8_lossy(&bytes).into_owned());
    }
    // `file://` — load from disk. Needed to render a local page at all, and it
    // is what makes a local CSS test corpus possible: comparing against another
    // browser on hand-written test cases is far more precise than only ever
    // loading live sites.
    if let Some(path) = url.strip_prefix("file://") {
        let path = viewers::local_path(path);
        if !page_may_read(&path) {
            return Err(format!("{} is not this page's to read", path.display()));
        }
        return std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()));
    }

    let cache = zipp_shell::cache::global();
    let revalidate = match cache.lookup("sub", url) {
        zipp_shell::cache::Lookup::Fresh(entry) => {
            return Ok(String::from_utf8_lossy(&entry.body).into_owned());
        }
        zipp_shell::cache::Lookup::Revalidate(entry) => Some(entry),
        zipp_shell::cache::Lookup::Miss => None,
    };

    let mut req = http_client().get(url);
    if let Some(ref prev) = revalidate {
        if let Some(ref etag) = prev.etag {
            req = req.header(reqwest::header::IF_NONE_MATCH, etag.as_str());
        }
        if let Some(ref lm) = prev.last_modified {
            req = req.header(reqwest::header::IF_MODIFIED_SINCE, lm.as_str());
        }
    }
    let resp = req.send().map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    if status == 304
        && let Some(prev) = revalidate
    {
        let refreshed = cache.revalidate_304("sub", url, prev, resp.headers());
        return Ok(String::from_utf8_lossy(&refreshed.body).into_owned());
    }
    let resp_headers = resp.headers().clone();
    let bytes = resp.bytes().map_err(|e| e.to_string())?.to_vec();
    cache.store("sub", url, status, &resp_headers, bytes.clone());
    Ok(String::from_utf8_lossy(&bytes).into_owned())
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

// `credentials` ("omit" | "same-origin" | "include") and extra request headers
// JSON for the next request (`fetch` / `XMLHttpRequest`).
var __zippCredentials = "";
var __zippRequestHeaders = "";
var http = (function () {
    function request(method, url, body, contentType) {
        var credentials = __zippCredentials;
        var extraHeaders = __zippRequestHeaders;
        __zippCredentials = "";
        __zippRequestHeaders = "";
        // Host contract: strings in, JSON string out. A transport failure is a
        // JS throw (the host returns Err), which fetch()'s try/catch turns into
        // a rejected promise — same shape as a real browser network error.
        var raw = __zippHostCall(
            "http", method, String(url),
            body == null ? "" : String(body),
            contentType == null ? "" : String(contentType),
            credentials,
            extraHeaders
        );
        var r = JSON.parse(raw);
        return {
            status: r.status,
            ok: r.ok,
            body: r.body,
            contentType: r.contentType || "",
            headers: r.headers || {}
        };
    }
    return {
        request: request,
        get: function (url) { return request("GET", url, "", ""); },
        head: function (url) { return request("HEAD", url, "", ""); },
        post: function (url, body, ct) { return request("POST", url, body, ct); },
        put: function (url, body, ct) { return request("PUT", url, body, ct); },
        patch: function (url, body, ct) { return request("PATCH", url, body, ct); },
        "delete": function (url) { return request("DELETE", url, "", ""); }
    };
})();

// Bytes from the host. A binary host call (`browser_host_call_ctx`) copies
// into this array, found by name, so it must be a top-level binding; it grows
// by doubling, as the engine keeps every array it has copied into pinned.
var __zippHostBytes = new Uint8Array(0);
function __zippFetchBytes(url, credentials) {
    var info = JSON.parse(__zippHostCall("http_bytes", url, credentials || ""));
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
    var host = __nodesById[handlerNid] || null;
    var target = __nodesById[targetNid] || host || null;
    var result = 0;
    // Listeners attached directly on a host's ShadowRoot fire before the host itself.
    if (host && host._shadowRoot) {
        var srByType = __eventHandlers[host._shadowRoot._id];
        var srHandlers = srByType && srByType[type];
        if (srHandlers && srHandlers.length > 0) {
            result = __zippRunListeners(srHandlers, type, target, host._shadowRoot, 3, details);
            if (result & 2) return result;
        }
    }
    var byType = __eventHandlers[handlerNid];
    if (!byType) return result;
    var handlers = byType[type];
    if (!handlers || handlers.length === 0) return result;
    return result | __zippRunListeners(handlers, type, target, host, 2, details);
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
            while (n) { path.push(n); n = n.parentNode || n.host || null; }
            if (path.length && path[path.length - 1] === document && typeof window !== "undefined") path.push(window);
            return path;
        }
    };
    // React reads `nativeEvent` off its synthetic wrapper; point it at itself so
    // either access works.
    ev.nativeEvent = ev;
    ev.view = typeof window !== "undefined" ? window : null;
    // What kind of event it is, for `instanceof MouseEvent` and the like
    // (monaco asks), with the methods that kind has (`getModifierState`).
    if (typeof __nativeEventClass === "function") Object.setPrototypeOf(ev, __nativeEventClass(type).prototype);
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
/// A navigation the first page asked for before NAV_PROVIDER was set.
static EARLY_NAV: std::sync::Mutex<Option<url::Url>> = std::sync::Mutex::new(None);
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
    let base = page_url();
    url::Url::parse(&base)
        .and_then(|b| b.join(raw))
        .map(|u| u.to_string())
        .unwrap_or_else(|_| raw.to_string())
}

/// What an `--app` page may read on disk: its own folder and the files it
/// was given, as they are on disk (links followed). Empty when not in `--app`.
static APP_FILES: Mutex<Vec<std::path::PathBuf>> = Mutex::new(Vec::new());

/// Under what name an `--app` page keeps what it stores, from its file: its
/// own, as what it runs from is its own.
static APP_STORAGE: Mutex<Option<String>> = Mutex::new(None);

pub(crate) fn app_storage_name() -> Option<String> {
    APP_STORAGE.lock().unwrap().clone()
}

/// Set for a run that loads a trusted tree of test pages (`--reftest`), whose
/// tests share support files in other folders.
static ANY_FILE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `--app <url> [file ...]`: the page's folder, when it is on disk, and the files.
fn allow_app_files(url: &str, files: impl Iterator<Item = String>) {
    let mut allowed = APP_FILES.lock().unwrap();
    if let Some(dir) = url::Url::parse(url).ok().and_then(|u| u.to_file_path().ok()).and_then(|p| p.parent().map(Into::into)) {
        allowed.extend(std::fs::canonicalize::<std::path::PathBuf>(dir));
    }
    let files: Vec<std::path::PathBuf> = files
        .filter_map(|f| std::fs::canonicalize(f).ok())
        .filter(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file()))
        .collect();
    // FNV-1a of what it runs: the same file, the same name, in every version.
    let named = files.first().map(|f| {
        let hash = f.to_string_lossy().bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
        format!("app_{hash:016x}")
    });
    *APP_STORAGE.lock().unwrap() = named;
    allowed.extend(files);
    // Nothing that exists: then nothing at all, rather than everything.
    if allowed.is_empty() {
        allowed.push(std::path::PathBuf::from("/nonexistent/zipp-app"));
    }
}

/// Whether `path` is within what `--app` allowed (anything, outside `--app`).
pub(crate) fn is_app() -> bool {
    !APP_FILES.lock().unwrap().is_empty()
}

pub(crate) fn within_app_files(path: &std::path::Path) -> bool {
    let allowed = APP_FILES.lock().unwrap();
    allowed.is_empty() || std::fs::canonicalize(path).is_ok_and(|real| allowed.iter().any(|a| real.starts_with(a)))
}

thread_local! {
    /// The page a worker's thread belongs to: the one that started it.
    static THREAD_PAGE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    /// Whether the current JS execution slice belongs to an embedded `<iframe>` sub-document.
    static IS_SUBDOCUMENT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// RAII guard tracking whether host calls are executing inside an embedded `<iframe>`.
pub(crate) struct SubdocumentGuard {
    prev: bool,
}

impl SubdocumentGuard {
    pub(crate) fn enter(is_subdocument: bool) -> Self {
        let prev = IS_SUBDOCUMENT.with(|c| c.replace(is_subdocument));
        Self { prev }
    }
}

impl Drop for SubdocumentGuard {
    fn drop(&mut self) {
        IS_SUBDOCUMENT.with(|c| c.set(self.prev));
    }
}

pub(crate) fn is_subdocument() -> bool {
    IS_SUBDOCUMENT.with(|c| c.get())
}

/// This thread works for `page`, whatever the window goes on to show.
pub(crate) fn set_thread_page(page: String) {
    THREAD_PAGE.with(|p| *p.borrow_mut() = Some(page));
}

/// RAII guard that temporarily binds `THREAD_PAGE` and `DOC_URL` to a specific
/// document URL (e.g. an embedded `<iframe>` sub-document) and restores both on
/// drop so iframe `fetch()`/origin/cookie checks resolve against the iframe's
/// own URL without clobbering the parent page.
pub(crate) struct ThreadPageGuard {
    prev_page: Option<String>,
    prev_doc: Option<url::Url>,
}

impl ThreadPageGuard {
    pub(crate) fn enter(page_url: &str) -> Self {
        let prev_page = THREAD_PAGE.with(|p| p.borrow_mut().replace(page_url.to_string()));
        let parsed = url::Url::parse(page_url).ok();
        let prev_doc = DOC_URL.with(|d| std::mem::replace(&mut *d.borrow_mut(), parsed));
        Self { prev_page, prev_doc }
    }
}

impl Drop for ThreadPageGuard {
    fn drop(&mut self) {
        THREAD_PAGE.with(|p| *p.borrow_mut() = self.prev_page.take());
        DOC_URL.with(|d| *d.borrow_mut() = self.prev_doc.take());
    }
}

/// The page asking: a worker's own, or the one showing. Rights go by this, not
/// by what is showing now: a worker in a loop outlives its page, and when the
/// window went on to a page on disk it would have had that page's rights.
pub(crate) fn page_url() -> String {
    THREAD_PAGE
        .with(|p| p.borrow().clone())
        .unwrap_or_else(|| CURRENT_URL.lock().unwrap_or_else(|e| e.into_inner()).clone())
}

/// Folders any page on disk may read, besides its own: `ZIPP_FILE_ROOTS`,
/// separated by `:`. zipp-linux names the pages of its image, which load from
/// beside each other.
fn file_roots() -> Vec<std::path::PathBuf> {
    std::env::var("ZIPP_FILE_ROOTS")
        .map(|roots| roots.split(':').filter(|r| !r.is_empty()).filter_map(|r| std::fs::canonicalize(r).ok()).collect())
        .unwrap_or_default()
}

/// Whether the page showing may load the file at `path`, by any means: script,
/// a stylesheet, an image, a module, a worker. This is the one rule, and every
/// way of reading a page's files asks it:
///
/// - A page from the web loads nothing from disk, as in any browser. Only
///   `fetch` used to ask, so `<script src="file:///...">`, a stylesheet, an
///   image or a worker read what the browser could, root's files under
///   zipp-linux.
/// - An `--app` page: what it was given.
/// - Any other page on disk: its own folder and what is below it, and
///   `ZIPP_FILE_ROOTS`. A page saved from the web and opened here does not get
///   the rest of the machine.
/// - The desktop (a kiosk's page on disk) has `navigator.zipp`, so the machine
///   already.
pub(crate) fn page_may_read(path: &std::path::Path) -> bool {
    if ANY_FILE.load(std::sync::atomic::Ordering::Relaxed) {
        return true;
    }
    let page = page_url();
    let Some(page) = url::Url::parse(&page).ok().filter(|u| u.scheme() == "file").and_then(|u| u.to_file_path().ok()) else {
        return false;
    };
    // Files, not what only looks like one: `file:///dev/zero` never ends.
    if !within_app_files(path) || !std::fs::metadata(path).is_ok_and(|m| m.is_file()) {
        return false;
    }
    if system::KIOSK.load(std::sync::atomic::Ordering::Relaxed) || is_app() {
        return true;
    }
    let Ok(real) = std::fs::canonicalize(path) else {
        return false;
    };
    let own = page.parent().and_then(|p| std::fs::canonicalize(p).ok());
    own.is_some_and(|dir| real.starts_with(dir)) || file_roots().iter().any(|root| real.starts_with(root))
}

/// The proof a form is the browser's own: page script cannot read the
/// toolbar or the console's line, so it cannot put this in a URL. Their forms
/// used to be told apart by a field's name alone, which a link could carry
/// too: `?__nav_url=file:///...` went where the address bar goes, and
/// `?__js=...`, on any link on a page, ran as what was typed into the console.
pub(crate) fn chrome_token() -> &'static str {
    static TOKEN: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    TOKEN.get_or_init(|| {
        let mut bytes = [0u8; 16];
        // Without randomness (it cannot fail on a running system) no form
        // passes: the empty value is refused.
        if getrandom::fill(&mut bytes).is_err() {
            return String::new();
        }
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    })
}

/// Whether `url` was submitted by the browser's own toolbar or console.
fn from_chrome(url: &str) -> bool {
    let token = chrome_token();
    !token.is_empty()
        && url::Url::parse(url).is_ok_and(|u| u.query_pairs().any(|(k, v)| k == "__zipp_chrome" && v == token))
}

/// In-memory `blob:` URL store shared across the main page thread, Web Workers,
/// WebGL texture decoding, and `zipp-shell`'s `NetProvider`.
struct BlobEntry {
    mime: String,
    bytes: Vec<u8>,
    origin: String,
    scope: u64,
}
type BlobMap = HashMap<String, BlobEntry>;
static BLOB_STORE: std::sync::LazyLock<std::sync::Mutex<BlobMap>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));
static NEXT_BLOB_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn caller_blob_origin() -> String {
    web_origin(&page_url())
        .map(|o| o.ascii_serialization())
        .unwrap_or_else(|| "null".to_string())
}

pub(crate) fn store_blob(_origin_hint: &str, mime: &str, bytes: Vec<u8>) -> String {
    let origin = caller_blob_origin();
    let scope = async_fetch::current_scope();
    let mut rand = [0u8; 16];
    let token = if getrandom::fill(&mut rand).is_ok() {
        let mut s = String::with_capacity(32);
        for b in rand {
            let _ = std::fmt::Write::write_fmt(&mut s, format_args!("{b:02x}"));
        }
        s
    } else {
        let id = NEXT_BLOB_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        format!("{id:016x}")
    };
    let base = if origin.is_empty() || origin == "null" { "null" } else { origin.trim_end_matches('/') };
    let url = format!("blob:{base}/zipp-{token}");
    BLOB_STORE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(url.clone(), BlobEntry { mime: mime.to_string(), bytes, origin, scope });
    url
}

fn blob_allowed_for_caller(entry: &BlobEntry) -> bool {
    let caller_origin = caller_blob_origin();
    let caller_scope = async_fetch::current_scope();
    if entry.origin != "null" {
        caller_origin == entry.origin
    } else {
        caller_scope == 0 || entry.scope == 0 || entry.scope == caller_scope
    }
}

pub(crate) fn lookup_blob(url: &str) -> Option<(String, Vec<u8>)> {
    BLOB_STORE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(url)
        .map(|e| (e.mime.clone(), e.bytes.clone()))
}

pub(crate) fn lookup_blob_for_caller(url: &str) -> Option<(String, Vec<u8>)> {
    BLOB_STORE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(url)
        .filter(|e| blob_allowed_for_caller(e))
        .map(|e| (e.mime.clone(), e.bytes.clone()))
}

pub(crate) fn lookup_blob_bytes(url: &str) -> Option<Vec<u8>> {
    lookup_blob(url).map(|(_, bytes)| bytes)
}

pub(crate) fn revoke_blob_for_caller(url: &str) {
    let mut store = BLOB_STORE.lock().unwrap_or_else(|e| e.into_inner());
    if store.get(url).is_some_and(blob_allowed_for_caller) {
        store.remove(url);
    }
}

pub(crate) fn drop_blobs_for_scope(scope: u64) {
    if scope == 0 {
        return;
    }
    BLOB_STORE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|_, e| e.scope != scope);
}

/// Decode a `data:[<mediatype>][;base64],<data>` URI into `(mime_type, bytes)`.
pub(crate) fn decode_data_url(url: &str) -> Option<(String, Vec<u8>)> {
    use base64::Engine as _;
    let rest = url.strip_prefix("data:")?;
    let (meta, payload) = rest.split_once(',')?;
    let mut is_base64 = false;
    let mut mime = String::new();
    for part in meta.split(';') {
        let p = part.trim();
        if p.eq_ignore_ascii_case("base64") {
            is_base64 = true;
        } else if mime.is_empty() && !p.is_empty() && !p.contains('=') {
            mime = p.to_string();
        }
    }
    if mime.is_empty() {
        mime = "text/plain;charset=US-ASCII".to_string();
    }
    let bytes = if is_base64 {
        let cleaned: String = payload.chars().filter(|c| !c.is_ascii_whitespace()).collect();
        let decoded = urlencoding::decode(&cleaned).unwrap_or(std::borrow::Cow::Borrowed(&cleaned));
        base64::engine::general_purpose::STANDARD
            .decode(decoded.as_bytes())
            .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(decoded.as_bytes()))
            .ok()?
    } else {
        urlencoding::decode_binary(payload.as_bytes()).into_owned()
    };
    Some((mime, bytes))
}

/// Guess a basic Content-Type for local `file:` fetches (Wasm, JS, JSON, images, glTF).
fn guess_mime_from_url(url: &str) -> &'static str {
    let path = url.split(['?', '#']).next().unwrap_or(url).to_ascii_lowercase();
    if path.ends_with(".wasm") { "application/wasm" }
    else if path.ends_with(".js") || path.ends_with(".mjs") { "text/javascript" }
    else if path.ends_with(".json") { "application/json" }
    else if path.ends_with(".css") { "text/css" }
    else if path.ends_with(".html") || path.ends_with(".htm") { "text/html" }
    else if path.ends_with(".svg") { "image/svg+xml" }
    else if path.ends_with(".png") { "image/png" }
    else if path.ends_with(".jpg") || path.ends_with(".jpeg") { "image/jpeg" }
    else if path.ends_with(".webp") { "image/webp" }
    else if path.ends_with(".glb") { "model/gltf-binary" }
    else if path.ends_with(".gltf") { "model/gltf+json" }
    else if path.ends_with(".hdr") { "image/vnd.radiance" }
    else { "application/octet-stream" }
}

/// GET a URL for page script, as bytes: `blob:`, `data:`, `file:`, or http(s).
/// Fetch an `http://` or `https://` `<video>` resource through the browser's
/// own HTTP client (cookies, HTTP cache, Incognito-isolated with zero WinINet
/// disk leaks) up to 64 MiB. Larger/streaming resources return `None` so Media
/// Foundation can stream them directly.
#[cfg(windows)]
pub(crate) fn fetch_media_bytes(url: &str) -> Option<(String, Vec<u8>)> {
    use std::io::Read as _;
    const MAX_INMEM_VIDEO_BYTES: u64 = 64 * 1024 * 1024;
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return None;
    }
    let cache = zipp_shell::cache::global();
    let revalidate = match cache.lookup("sub", url) {
        zipp_shell::cache::Lookup::Fresh(entry) if (200..300).contains(&entry.status) => {
            return Some((entry.content_type, entry.body.as_ref().clone()));
        }
        zipp_shell::cache::Lookup::Revalidate(entry) => Some(entry),
        _ => None,
    };
    let mut req = http_client().get(url);
    if let Some(ref prev) = revalidate {
        if let Some(ref etag) = prev.etag {
            req = req.header(reqwest::header::IF_NONE_MATCH, etag.as_str());
        }
        if let Some(ref lm) = prev.last_modified {
            req = req.header(reqwest::header::IF_MODIFIED_SINCE, lm.as_str());
        }
    }
    let resp = req.send().ok()?;
    let status = resp.status().as_u16();
    if status == 304
        && let Some(prev) = revalidate
    {
        let refreshed = cache.revalidate_304("sub", url, prev, resp.headers());
        return Some((refreshed.content_type, refreshed.body.as_ref().clone()));
    }
    if !(200..300).contains(&status) {
        return None;
    }
    if resp.content_length().is_some_and(|len| len > MAX_INMEM_VIDEO_BYTES) {
        return None;
    }
    let resp_headers = resp.headers().clone();
    let mime = resp_headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let mut bytes = Vec::new();
    let mut limited = resp.take(MAX_INMEM_VIDEO_BYTES + 1);
    limited.read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_INMEM_VIDEO_BYTES || bytes.is_empty() {
        return None;
    }
    cache.store("sub", url, status, &resp_headers, bytes.clone());
    Some((mime, bytes))
}

fn fetch_bytes_for_page(url: &str) -> Result<(u16, Vec<u8>), String> {
    let (status, _, bytes) = fetch_bytes_with_mime(url, CredentialsMode::SameOrigin)?;
    Ok((status, bytes))
}

fn fetch_bytes_with_mime(url: &str, creds: CredentialsMode) -> Result<(u16, String, Vec<u8>), String> {
    if url.starts_with("blob:") {
        return match lookup_blob_for_caller(url) {
            Some((mime, bytes)) => Ok((200, mime, bytes)),
            None => Err(format!("TypeError: Failed to fetch: blob URL not found: {url}")),
        };
    }
    if url.starts_with("data:") {
        return match decode_data_url(url) {
            Some((mime, bytes)) => Ok((200, mime, bytes)),
            None => Err(format!("TypeError: Failed to fetch: invalid data URL: {url}")),
        };
    }
    if let Ok(parsed) = url::Url::parse(url)
        && parsed.scheme() == "file" {
            let path = parsed.to_file_path().map_err(|_| format!("Error: GET {url}: not a file path"))?;
            if !page_may_read(&path) {
                return Ok((403, String::new(), Vec::new()));
            }
            return match std::fs::read(&path) {
                Ok(bytes) => Ok((200, guess_mime_from_url(url).to_string(), bytes)),
                Err(_) => Ok((404, String::new(), Vec::new())),
            };
        }
    let cache = zipp_shell::cache::global();
    let revalidate = if creds != CredentialsMode::Omit {
        match cache.lookup("sub", url) {
            zipp_shell::cache::Lookup::Fresh(entry) => {
                return Ok((entry.status, entry.content_type, entry.body.as_ref().clone()));
            }
            zipp_shell::cache::Lookup::Revalidate(entry) => Some(entry),
            zipp_shell::cache::Lookup::Miss => None,
        }
    } else {
        None
    };

    let mut req = client_for_page(url, creds).get(url);
    if let Some(ref prev) = revalidate {
        if let Some(ref etag) = prev.etag {
            req = req.header(reqwest::header::IF_NONE_MATCH, etag.as_str());
        }
        if let Some(ref lm) = prev.last_modified {
            req = req.header(reqwest::header::IF_MODIFIED_SINCE, lm.as_str());
        }
    }
    let resp = req.send().map_err(|e| format!("Error: GET {url}: {e}"))?;
    let status = resp.status().as_u16();
    if status == 304
        && let Some(prev) = revalidate
    {
        let refreshed = cache.revalidate_304("sub", url, prev, resp.headers());
        return Ok((refreshed.status, refreshed.content_type, refreshed.body.as_ref().clone()));
    }
    let resp_headers = resp.headers().clone();
    let mime = resp_headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = resp.bytes().map_err(|e| format!("Error: reading {url}: {e}"))?.to_vec();
    if creds != CredentialsMode::Omit {
        cache.store("sub", url, status, &resp_headers, bytes.clone());
    }
    Ok((status, mime, bytes))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CredentialsMode {
    Omit,
    SameOrigin,
    Include,
}

impl CredentialsMode {
    fn from_arg(arg: &str) -> Self {
        match arg {
            "omit" => Self::Omit,
            "include" => Self::Include,
            _ => Self::SameOrigin,
        }
    }

    fn sends_cookies(self, url: &str) -> bool {
        match self {
            Self::Omit => false,
            Self::SameOrigin => same_origin(url),
            Self::Include => true,
        }
    }

    fn is_include(self) -> bool {
        matches!(self, Self::Include)
    }
}

/// The origin of the page asking, when it is on the web: what the same-origin
/// rule compares. None for a page on disk, which is this machine's own.
fn web_origin(page: &str) -> Option<url::Origin> {
    url::Url::parse(page).ok().filter(|u| matches!(u.scheme(), "http" | "https")).map(|u| u.origin())
}

/// Whether `url` is the page's own site.
fn same_origin(url: &str) -> bool {
    let Some(mine) = web_origin(&page_url()) else {
        return false;
    };
    url::Url::parse(url).is_ok_and(|u| u.origin() == mine)
}

/// The client for a request by the page: honours `omit`, `same-origin`, and `include`.
fn client_for_page(url: &str, creds: CredentialsMode) -> &'static reqwest::blocking::Client {
    if creds.sends_cookies(url) { http_client() } else { anonymous_http_client() }
}

/// The same-origin rule, for a page on the web reading another site: the
/// response is the page's to see only if that site says so. Without this, any
/// page read any site it could reach: the person's mail if logged in, the
/// router, `127.0.0.1`.
fn cors_allows(resp: &reqwest::blocking::Response, origin: &str, creds: CredentialsMode) -> bool {
    let header = |name: &str| resp.headers().get(name).and_then(|v| v.to_str().ok()).unwrap_or("");
    let allowed = header("access-control-allow-origin").trim();
    if creds.is_include() {
        allowed == origin && header("access-control-allow-credentials").trim() == "true"
    } else {
        allowed == "*" || allowed == origin
    }
}

/// A request a form could not make (a PUT, a JSON POST) asks first, so that
/// a site that never heard of CORS is not sent one by a page elsewhere.
fn cors_preflight(
    client: &'static reqwest::blocking::Client,
    method: &str,
    url: &str,
    origin: &str,
    content_type: &str,
    extra_header_names: &[String],
    creds: CredentialsMode,
) -> Result<(), String> {
    let mut req_headers = vec!["content-type".to_string()];
    for h in extra_header_names {
        let low = h.to_ascii_lowercase();
        if !req_headers.contains(&low) {
            req_headers.push(low);
        }
    }
    let resp = client
        .request(reqwest::Method::OPTIONS, url)
        .header("Origin", origin)
        .header("Access-Control-Request-Method", method)
        .header("Access-Control-Request-Headers", req_headers.join(", "))
        .send()
        .map_err(|e| format!("TypeError: Failed to fetch: {method} {url}: {e}"))?;
    let methods = resp.headers().get("access-control-allow-methods").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let allowed = resp.status().is_success()
        && cors_allows(&resp, origin, creds)
        && (matches!(method, "GET" | "HEAD" | "POST")
            || methods.split(',').any(|m| m.trim() == "*" || m.trim().eq_ignore_ascii_case(method)));
    if allowed {
        Ok(())
    } else {
        Err(format!("TypeError: Failed to fetch: {url} does not allow {method} from {origin} (content type {content_type})"))
    }
}

/// Whether page script is forbidden from setting a request header per Fetch spec.
fn is_forbidden_request_header(name: &str) -> bool {
    let low = name.to_ascii_lowercase();
    low.starts_with("proxy-")
        || low.starts_with("sec-")
        || matches!(
            low.as_str(),
            "accept-charset"
                | "accept-encoding"
                | "access-control-request-headers"
                | "access-control-request-method"
                | "connection"
                | "content-length"
                | "cookie"
                | "cookie2"
                | "date"
                | "dnt"
                | "expect"
                | "host"
                | "keep-alive"
                | "origin"
                | "referer"
                | "te"
                | "trailer"
                | "transfer-encoding"
                | "upgrade"
                | "via"
        )
}

/// Serialize non-forbidden response headers (filtering out `set-cookie`) to JSON.
fn safe_response_headers_json(headers: &reqwest::header::HeaderMap) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for (name, val) in headers {
        let key = name.as_str().to_ascii_lowercase();
        if key == "set-cookie" || key == "set-cookie2" {
            continue;
        }
        let Ok(v) = val.to_str() else { continue };
        match map.get_mut(&key) {
            Some(serde_json::Value::String(existing)) => {
                existing.push_str(", ");
                existing.push_str(v);
            }
            _ => {
                map.insert(key, serde_json::Value::String(v.to_string()));
            }
        }
    }
    serde_json::Value::Object(map)
}

thread_local! {
    /// The files picked in file dialogs, scoped per document so other origins
    /// and subsequent navigations cannot read previously picked files.
    static CHOSEN_FILES: std::cell::RefCell<std::collections::HashSet<(u64, std::path::PathBuf)>> = std::cell::RefCell::default();
}

/// Let the active document scope read these files: the user picked them in a file dialog.
pub(crate) fn allow_chosen_files<'a>(paths: impl IntoIterator<Item = &'a std::path::Path>) {
    let scope = async_fetch::current_scope();
    CHOSEN_FILES.with(|c| {
        c.borrow_mut()
            .extend(paths.into_iter().map(|p| (scope, p.to_path_buf())));
    });
}

/// Drop chosen-file permissions when a document navigates away or unmounts.
pub(crate) fn drop_chosen_files_for_scope(scope: u64) {
    if scope == 0 {
        return;
    }
    CHOSEN_FILES.with(|c| c.borrow_mut().retain(|(s, _)| *s != scope));
}

/// The host channel with the VM at hand, for the calls that move bytes: a
/// binary body goes into the shim's `__zippHostBytes` scratch array through
/// the engine's typed-array region, not through a string. Every other call is
/// `browser_host_call`'s.
fn browser_host_call_ctx(ctx: &mut dyn HostCtx, kind: &str, args: &[String]) -> Result<String, String> {
    match kind {
        // The page's body, once, as the shim's document starts.
        "seed" => Ok(SEED.with(|s| std::mem::take(&mut *s.borrow_mut()))),
        // `crypto.getRandomValues` (a page's or a worker's): this many bytes of
        // the machine's randomness, as hex; WebCrypto's limit, 65536, at most.
        "random" => {
            let n = args.first().and_then(|a| a.parse::<usize>().ok()).unwrap_or(0).min(65536);
            let mut bytes = vec![0u8; n];
            getrandom::fill(&mut bytes).map_err(|e| format!("OperationError: no randomness: {e}"))?;
            const HEX: &[u8; 16] = b"0123456789abcdef";
            Ok(bytes.iter().flat_map(|b| [HEX[(b >> 4) as usize] as char, HEX[(b & 15) as usize] as char]).collect())
        }
        // `crypto.subtle.digest(algorithm, data)` (SHA-1, SHA-256, SHA-384, SHA-512).
        "crypto_digest" => {
            let alg = args.first().map(String::as_str).unwrap_or_default();
            let len: usize = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(0);
            if len == 0 {
                crypto::digest_hex(alg, &[])
            } else {
                let (addr, count, kind) = ctx.typed_array_region("__zippHostBytes")?;
                if kind != 1 || len > count {
                    return Err("RangeError: crypto_digest buffer mismatch".into());
                }
                // SAFETY: engine pinned `__zippHostBytes` (`count` u8s at `addr`).
                let slice = unsafe { std::slice::from_raw_parts(addr as *const u8, len) };
                crypto::digest_hex(alg, slice)
            }
        }
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
            let resolve_module = |base: &str, spec: &str| map.resolve_specifier(base, spec, &resolve);
            let out = esm::link_more_lazy(&[url], &linked, false, map.fingerprint(), &fetch, &resolve_module);
            for (mod_url, mod_src) in &out.loaded_modules {
                devtools::record_script(devtools::ScriptKind::Module, mod_url, mod_src.clone(), None);
            }
            for (failed_url, err) in &out.failures {
                console::push(console::Level::Error, format!("module {failed_url} failed to load: {err}"));
                devtools::record_script(
                    devtools::ScriptKind::Module,
                    failed_url,
                    String::new(),
                    Some(err.clone()),
                );
            }
            Ok(js_opt::prepare_script(&out.source))
        }
        // `new URL(input, base)`, for a page or a worker.
        "url" => worker::url_parts(
            args.first().map(String::as_str).unwrap_or_default(),
            args.get(1).map(String::as_str).unwrap_or_default(),
        ),
        // A file picked in a file dialog, as bytes for `take_bytes`; its length.
        // Only those: a page reads no other file of yours this way.
        "chosen_bytes" => {
            let scope = async_fetch::current_scope();
            let path = std::path::PathBuf::from(args.first().map(String::as_str).unwrap_or_default());
            if !CHOSEN_FILES.with(|c| c.borrow().contains(&(scope, path.clone()))) {
                return Err("NotAllowedError: only a file picked in a file dialog can be read".into());
            }
            let bytes = std::fs::read(&path).map_err(|e| format!("NotReadableError: {}: {e}", path.display()))?;
            let len = bytes.len();
            PENDING_BYTES.with(|p| *p.borrow_mut() = bytes);
            Ok(len.to_string())
        }
        // Store a Blob's raw bytes from `__zippHostBytes` (or inline text) into
        // `BLOB_STORE` and return its `blob:` URL.
        "blob_create" => {
            let mode = args.first().map(String::as_str).unwrap_or("bytes");
            let mime = args.get(1).map(String::as_str).unwrap_or("");
            let origin = args.get(2).map(String::as_str).unwrap_or("null");
            let bytes = if mode == "text" {
                args.get(3).cloned().unwrap_or_default().into_bytes()
            } else {
                let len: usize = mode.parse().unwrap_or(0);
                if len == 0 {
                    Vec::new()
                } else {
                    let (addr, count, kind) = ctx.typed_array_region("__zippHostBytes")?;
                    if kind != 1 || len > count {
                        return Err("RangeError: blob_create buffer mismatch".into());
                    }
                    // SAFETY: engine pinned `__zippHostBytes` (`count` u8s at `addr`).
                    unsafe { std::slice::from_raw_parts(addr as *const u8, len).to_vec() }
                }
            };
            Ok(store_blob(origin, mime, bytes))
        }
        "blob_revoke" => {
            revoke_blob_for_caller(args.first().map(String::as_str).unwrap_or_default());
            Ok(String::new())
        }
        "blob_read" => {
            let url = args.first().map(String::as_str).unwrap_or_default();
            let Some((mime, bytes)) = lookup_blob_for_caller(url) else {
                return Err(format!("TypeError: blob URL not found: {url}"));
            };
            let len = bytes.len();
            PENDING_BYTES.with(|p| *p.borrow_mut() = bytes);
            Ok(format!("{{\"len\":{len},\"type\":{}}}", json_quote(&mime)))
        }
        // `new DOMParser().parseFromString(html, "text/html")`: real html5ever parse.
        "parse_html" => {
            let html = args.first().map(String::as_str).unwrap_or_default();
            Ok(seed::parse_html_json(html).unwrap_or_else(|| "[[],[],[],[]]".to_string()))
        }
        // `fetch(url).arrayBuffer()`: fetch the body as bytes and keep it for
        // `take_bytes`; report its status, contentType and length, so the shim
        // can make the scratch array big enough first.
        "http_bytes" => {
            let url = resolve_page_url(args.first().map(String::as_str).unwrap_or_default());
            let creds = CredentialsMode::from_arg(args.get(1).map(String::as_str).unwrap_or_default());
            let origin = web_origin(&page_url()).map(|o| o.ascii_serialization());
            let foreign = origin.filter(|_| url.starts_with("http") && !same_origin(&url));
            let (status, mime, bytes) = match &foreign {
                None => fetch_bytes_with_mime(&url, creds)?,
                Some(origin) => {
                    let resp = client_for_page(&url, creds)
                        .get(&url)
                        .header("Origin", origin.as_str())
                        .send()
                        .map_err(|e| format!("TypeError: Failed to fetch: GET {url}: {e}"))?;
                    if !cors_allows(&resp, origin, creds) {
                        return Err(format!("TypeError: Failed to fetch: {url} does not allow {origin} to read it"));
                    }
                    let status = resp.status().as_u16();
                    let mime = resp
                        .headers()
                        .get(reqwest::header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    (status, mime, resp.bytes().map_err(|e| format!("Error: reading {url}: {e}"))?.to_vec())
                }
            };
            let len = bytes.len();
            PENDING_BYTES.with(|p| *p.borrow_mut() = bytes);
            Ok(format!(
                "{{\"status\":{status},\"ok\":{},\"len\":{len},\"contentType\":{}}}",
                (200..300).contains(&status),
                json_quote(&mime)
            ))
        }
        // Copy the pending bytes into `__zippHostBytes`.
        // navigator.clipboard: as the person just did something, as a browser
        // has it, so a page cannot read what was copied elsewhere by itself.
        "clipboard_read" => {
            if !user_just_acted() {
                return Err("NotAllowedError: reading the clipboard needs a click or a key press".into());
            }
            Ok(zipp_shell::clipboard_text())
        }
        "clipboard_write" => {
            if !user_just_acted() {
                return Err("NotAllowedError: writing the clipboard needs a click or a key press".into());
            }
            zipp_shell::set_clipboard_text(args.first().cloned().unwrap_or_default());
            Ok(String::new())
        }
        // `document.title = ...`: update both the active tab label and OS window title
        // (top-level documents only — embedded iframes must not hijack the tab title).
        "window_title" => {
            if is_subdocument() {
                return Ok(String::new());
            }
            let title = args.first().cloned().unwrap_or_default();
            tabs::update_active_title(&title);
            push_window_request(WindowRequest::Title(title));
            Ok(String::new())
        }
        // `requestFullscreen()` ("1") and `exitFullscreen()` ("0").
        "fullscreen" => {
            let on = args.first().is_some_and(|a| a == "1");
            // Going full screen takes the person's doing (leaving does not for
            // the top-level document; embedded iframes always need a gesture).
            if (on || is_subdocument()) && !user_just_acted() {
                return Ok("0".into());
            }
            push_window_request(WindowRequest::Fullscreen(on));
            Ok("1".into())
        }
        // An element's box in the viewport (`getBoundingClientRect`).
        "rect" => Ok(event_bridge::layout_rect(args.first().map(String::as_str).unwrap_or_default())),
        // `document.elementFromPoint(x, y)`: the script's node there.
        "hit" => Ok(event_bridge::layout_hit(
            args.first().map(String::as_str).unwrap_or_default(),
            args.get(1).map(String::as_str).unwrap_or_default(),
        )),
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
        // HTML5 `<video>` playback control & live status (`event_bridge.rs` + `media.rs`).
        "video" => event_bridge::video_host_call(args),
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
        "http" => match prepare_http_request(args)? {
            PreparedHttp::Immediate(json) => Ok(json),
            PreparedHttp::Network(req) => execute_network_http(*req),
        },
        "http_async" => match prepare_http_request(args)? {
            PreparedHttp::Immediate(json) => Ok(format!("SYNC:{json}")),
            PreparedHttp::Network(req) => {
                if async_fetch::is_enabled() {
                    let req_id = async_fetch::enqueue(move || execute_network_http(*req))
                        .ok_or_else(|| "Error: async fetch queue unavailable".to_string())?;
                    Ok(format!("ASYNC:{req_id}"))
                } else {
                    let json = execute_network_http(*req)?;
                    Ok(format!("SYNC:{json}"))
                }
            }
        },
        // `window.open` and `location.assign`/`replace`. Both were inert: `open`
        // returned null and did nothing, and `assign` only wrote to a plain
        // `href` property that nothing observed. A page whose call to action is
        // "opens in a new tab" therefore had a button that did nothing at all.
        //
        // There are no tabs yet, so this navigates the CURRENT window. That is
        // the honest single-window behaviour and beats silently dropping it.
        "navigate" => {
            if is_subdocument() {
                eprintln!("[NAV] blocked top-level navigation from subdocument");
                return Ok(String::new());
            }
            let raw = args.first().cloned().unwrap_or_default();
            if raw.is_empty() {
                return Ok(String::new());
            }
            let base = page_url();
            let full = resolve(&base, &raw);
            match full.parse::<url::Url>() {
                Ok(parsed) => {
                    // Untrusted page scripts may only navigate to normal web/document schemes,
                    // never internal browser chrome/settings/action URLs (and `file:` only from `file:`).
                    let scheme_ok = match parsed.scheme() {
                        "http" | "https" | "about" => true,
                        "file" => base.starts_with("file:"),
                        _ => false,
                    };
                    if !scheme_ok {
                        eprintln!("[NAV] refused script navigation to disallowed scheme: {full}");
                        return Ok(String::new());
                    }
                    eprintln!("[NAV] page requested {full}");
                    match NAV_PROVIDER.get() {
                        Some(provider) => provider.navigate_to(NavigationOptions::new(
                            parsed,
                            String::from("text/html"),
                            0,
                        )),
                        // The first page's script runs before there is a
                        // window to navigate: kept until there is (a page
                        // that sends the reader on at once was left where it
                        // was, saying nothing).
                        None => *EARLY_NAV.lock().unwrap_or_else(|e| e.into_inner()) = Some(parsed),
                    }
                    Ok(String::new())
                }
                Err(e) => Err(format!("Error: bad navigate url {full:?}: {e}")),
            }
        }
        // `window.open(url, target)` when opening a new tab (requires a recent user gesture).
        "open_tab" => {
            let raw = args.first().cloned().unwrap_or_default();
            if raw.is_empty() {
                return Ok(String::new());
            }
            if !user_just_acted() {
                eprintln!("[NAV] blocked unprompted window.open popup: {raw}");
                return Ok(String::new());
            }
            let base = page_url();
            let full = resolve(&base, &raw);
            match full.parse::<url::Url>() {
                Ok(parsed) => {
                    let scheme_ok = match parsed.scheme() {
                        "http" | "https" | "about" => true,
                        "file" => base.starts_with("file:"),
                        _ => false,
                    };
                    if !scheme_ok {
                        eprintln!("[NAV] refused script window.open to disallowed scheme: {full}");
                        return Ok(String::new());
                    }
                    eprintln!("[NAV] page opened new tab {full}");
                    if let Some(provider) = NAV_PROVIDER.get() {
                        provider.navigate_to(
                            NavigationOptions::new(parsed, String::from("text/html"), 0)
                                .set_new_tab(true)
                                .set_background(false),
                        );
                    }
                    Ok(String::new())
                }
                Err(e) => Err(format!("Error: bad window.open url {full:?}: {e}")),
            }
        }
        other => Err(format!("Error: unknown host call {other:?}")),
    }
}

struct PreparedHttpRequest {
    client: &'static reqwest::blocking::Client,
    method: String,
    url: String,
    body: String,
    content_type: String,
    creds: CredentialsMode,
    extra_headers: Vec<(String, String)>,
    page_href: String,
    page_origin: Option<String>,
    foreign: Option<String>,
    cacheable_get: bool,
    revalidate: Option<zipp_shell::cache::CachedEntry>,
}

enum PreparedHttp {
    Immediate(String),
    Network(Box<PreparedHttpRequest>),
}

fn prepare_http_request(args: &[String]) -> Result<PreparedHttp, String> {
    let method = args.first().map(String::as_str).unwrap_or("GET").to_string();
    let url = args.get(1).map(String::as_str).unwrap_or_default();
    let body = args.get(2).cloned().unwrap_or_default();
    let content_type = args.get(3).map(String::as_str).unwrap_or("");
    let content_type = if content_type.is_empty() {
        "application/json".to_string()
    } else {
        content_type.to_string()
    };

    let resolved = resolve_page_url(url);
    let creds = CredentialsMode::from_arg(args.get(4).map(String::as_str).unwrap_or_default());
    if method == "GET"
        && (resolved.starts_with("file:")
            || resolved.starts_with("blob:")
            || resolved.starts_with("data:"))
    {
        let (status, mime, bytes) = fetch_bytes_with_mime(&resolved, creds)?;
        return Ok(PreparedHttp::Immediate(format!(
            "{{\"status\":{status},\"ok\":{},\"contentType\":{},\"headers\":{{\"content-type\":{}}},\"body\":{}}}",
            (200..300).contains(&status),
            json_quote(&mime),
            json_quote(&mime),
            json_quote(&String::from_utf8_lossy(&bytes))
        )));
    }

    let extra_headers: Vec<(String, String)> = args
        .get(5)
        .filter(|s| !s.is_empty())
        .and_then(|s| serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(s).ok())
        .map(|m| {
            m.into_iter()
                .filter_map(|(k, v)| {
                    if is_forbidden_request_header(&k) {
                        return None;
                    }
                    Some((k, v.as_str()?.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    let has_auth_header = extra_headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("authorization"));

    let page_href = page_url();
    let page_origin = web_origin(&page_href).map(|o| o.ascii_serialization());
    let foreign = page_origin
        .as_ref()
        .filter(|_| !same_origin(&resolved))
        .cloned();

    let cache = zipp_shell::cache::global();
    let cacheable_get =
        method == "GET" && foreign.is_none() && !has_auth_header && creds != CredentialsMode::Omit;
    let revalidate = if cacheable_get {
        match cache.lookup("sub", &resolved) {
            zipp_shell::cache::Lookup::Fresh(entry) => {
                let body = String::from_utf8_lossy(&entry.body).into_owned();
                return Ok(PreparedHttp::Immediate(format!(
                    "{{\"status\":{},\"ok\":{},\"contentType\":{},\"headers\":{{\"content-type\":{}}},\"body\":{}}}",
                    entry.status,
                    (200..300).contains(&entry.status),
                    json_quote(&entry.content_type),
                    json_quote(&entry.content_type),
                    json_quote(&body)
                )));
            }
            zipp_shell::cache::Lookup::Revalidate(entry) => Some(entry),
            zipp_shell::cache::Lookup::Miss => None,
        }
    } else {
        None
    };

    let client = client_for_page(&resolved, creds);
    Ok(PreparedHttp::Network(Box::new(PreparedHttpRequest {
        client,
        method,
        url: resolved,
        body,
        content_type,
        creds,
        extra_headers,
        page_href,
        page_origin,
        foreign,
        cacheable_get,
        revalidate,
    })))
}

fn execute_network_http(p: PreparedHttpRequest) -> Result<String, String> {
    let PreparedHttpRequest {
        client,
        method,
        url,
        body,
        content_type,
        creds,
        extra_headers,
        page_href,
        page_origin,
        foreign,
        cacheable_get,
        revalidate,
    } = p;
    let method_str = method.as_str();
    let url_str = url.as_str();
    let ct_str = content_type.as_str();

    if let Some(origin) = &foreign {
        let has_non_simple_header = extra_headers.iter().any(|(k, _)| {
            !matches!(
                k.to_ascii_lowercase().as_str(),
                "accept" | "accept-language" | "content-language" | "content-type"
            )
        });
        let simple = !has_non_simple_header
            && match method_str {
                "GET" | "HEAD" => true,
                "POST" => matches!(
                    ct_str.split(';').next().unwrap_or("").trim(),
                    "text/plain" | "application/x-www-form-urlencoded" | "multipart/form-data"
                ),
                _ => false,
            };
        if !simple {
            let names: Vec<String> = extra_headers.iter().map(|(k, _)| k.clone()).collect();
            cors_preflight(client, method_str, url_str, origin, ct_str, &names, creds)?;
        }
    }

    let mut req = match method_str {
        "POST" => client.post(url_str).header("Content-Type", ct_str).body(body),
        "PUT" => client.put(url_str).header("Content-Type", ct_str).body(body),
        "PATCH" => client.patch(url_str).header("Content-Type", ct_str).body(body),
        "DELETE" => {
            if body.is_empty() {
                client.delete(url_str)
            } else {
                client.delete(url_str).header("Content-Type", ct_str).body(body)
            }
        }
        "HEAD" => client.head(url_str),
        _ => client.get(url_str),
    };
    for (k, v) in &extra_headers {
        if !k.eq_ignore_ascii_case("content-type") {
            req = req.header(k.as_str(), v.as_str());
        }
    }
    if let Some(origin) = &foreign {
        req = req.header("Origin", origin.as_str());
    } else if matches!(method_str, "POST" | "PUT" | "PATCH" | "DELETE")
        && let Some(origin) = &page_origin
    {
        req = req.header("Origin", origin.as_str());
    }
    if page_href.starts_with("http://") || page_href.starts_with("https://") {
        req = req.header("Referer", page_href.as_str());
    }
    req = req
        .header("Sec-Fetch-Mode", "cors")
        .header("Sec-Fetch-Dest", "empty")
        .header(
            "Sec-Fetch-Site",
            if foreign.is_none() {
                "same-origin"
            } else {
                "cross-site"
            },
        );
    if let Some(ref prev) = revalidate {
        if let Some(ref etag) = prev.etag {
            req = req.header(reqwest::header::IF_NONE_MATCH, etag.as_str());
        }
        if let Some(ref lm) = prev.last_modified {
            req = req.header(reqwest::header::IF_MODIFIED_SINCE, lm.as_str());
        }
    }

    let resp = req
        .send()
        .map_err(|e| format!("Error: {method_str} {url_str}: {e}"))?;
    if let Some(origin) = &foreign
        && !cors_allows(&resp, origin, creds)
    {
        return Err(format!(
            "TypeError: Failed to fetch: {url_str} does not allow {origin} to read it"
        ));
    }
    let cache = zipp_shell::cache::global();
    let status = resp.status().as_u16();
    if status == 304
        && let Some(prev) = revalidate
    {
        let refreshed = cache.revalidate_304("sub", url_str, prev, resp.headers());
        let body = String::from_utf8_lossy(&refreshed.body).into_owned();
        let headers_json = safe_response_headers_json(resp.headers());
        return Ok(format!(
            "{{\"status\":{},\"ok\":{},\"contentType\":{},\"headers\":{},\"body\":{}}}",
            refreshed.status,
            (200..300).contains(&refreshed.status),
            json_quote(&refreshed.content_type),
            headers_json,
            json_quote(&body)
        ));
    }
    let ok = resp.status().is_success();
    let resp_headers = resp.headers().clone();
    let mime = resp_headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let headers_json = safe_response_headers_json(&resp_headers);
    let raw_bytes = resp
        .bytes()
        .map_err(|e| format!("Error: reading {url_str}: {e}"))?
        .to_vec();
    if cacheable_get {
        cache.store("sub", url_str, status, &resp_headers, raw_bytes.clone());
    } else if matches!(method_str, "POST" | "PUT" | "PATCH" | "DELETE") {
        cache.invalidate_url(url_str);
        page_cache_clear();
    }
    let body = String::from_utf8_lossy(&raw_bytes);
    Ok(format!(
        "{{\"status\":{status},\"ok\":{ok},\"contentType\":{},\"headers\":{},\"body\":{}}}",
        json_quote(&mime),
        headers_json,
        json_quote(&body)
    ))
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
/// Built-in internal browser pages (`zipp:settings`, `zipp:properties`, `zipp:history`).
fn internal_page_html(
    url: &str,
    notice: Option<&str>,
    history_filter: Option<&str>,
    last_web_url: Option<&str>,
) -> Option<String> {
    let base = url.split(['?', '#']).next().unwrap_or(url).trim().to_ascii_lowercase();
    match base.as_str() {
        "zipp:newtab" | "about:newtab" | "about:blank" => Some(
            properties_page::render_newtab(chrome_token()),
        ),
        "zipp:settings" | "zipp:properties" | "about:settings" | "about:properties" => Some(
            properties_page::render(chrome_token(), notice, history_filter, last_web_url, false),
        ),
        "zipp:history" | "about:history" => Some(properties_page::render(
            chrome_token(),
            notice,
            history_filter,
            last_web_url,
            true,
        )),
        _ => None,
    }
}

/// Spawn a brand-new isolated Incognito browser window (`--incognito`).
fn launch_incognito_window() {
    let Ok(exe) = std::env::current_exe() else {
        eprintln!("[BROWSER] could not resolve current executable for Incognito window");
        return;
    };
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--incognito");
    if is_mobile_view() {
        cmd.arg("--mobile");
    }
    match cmd.spawn() {
        Ok(_) => eprintln!("[BROWSER] Launched new Incognito window"),
        Err(e) => eprintln!("[BROWSER] Failed to launch Incognito window: {e}"),
    }
}

fn prepare_page(url: &str, viewport: (u32, u32)) -> (String, Option<String>, Option<ScriptState>) {
    let t_fetch = std::time::Instant::now();
    let html = if let Some(internal) = internal_page_html(url, None, None, None) {
        internal
    } else {
        // A PDF, image, text file or download comes back as a viewer page.
        viewers::fetch_document(url, true).unwrap_or_else(|e| error_page(url, &e))
    };
    let fetch_ms = t_fetch.elapsed().as_millis();
    render_page(url, &html, viewport, Some(fetch_ms))
}

/// Run a fetched page's JS and post-process it into the HTML the renderer sees:
/// the second half of [`prepare_page`], and all of a navigation's work on the
/// main thread. `fetch_ms` is `None` when the fetch happened elsewhere.
fn has_script_tag(html: &str) -> bool {
    html.as_bytes()
        .windows(7)
        .any(|w| w.eq_ignore_ascii_case(b"<script"))
}

fn render_page(
    url: &str,
    html: &str,
    viewport: (u32, u32),
    fetch_ms: Option<u128>,
) -> (String, Option<String>, Option<ScriptState>) {
    *CURRENT_URL.lock().unwrap() = url.to_string();
    clear_failed_iframes();
    devtools::new_page(url, html);
    let html = html.to_string();
    let title_count_before = pending_window_title_count();
    let t_js = std::time::Instant::now();
    let (rendered, event_map_json, js_state) = if has_script_tag(&html) {
        match run_zipp_js(&html, url, viewport) {
            Some((r, emap, state)) => (r, Some(emap), Some(state)),
            None => (html.clone(), None, None),
        }
    } else {
        (html, None, None)
    };
    devtools::set_rendered_html(&rendered);
    let js_ms = t_js.elapsed().as_millis();

    let t_post = std::time::Instant::now();
    // Strip <noscript> blocks (our browser has JS support)
    let out = strip_noscript(&rendered);
    // Toggle JS detection classes: no-js → js, client-nojs → client-js
    let out = out
        .replace("class=\"no-js", "class=\"js")
        .replace("class=\"client-nojs", "class=\"client-js");
    let out = inline_images(&out, url);
    let js_title = last_window_title_since(title_count_before);
    let page_title = js_title.clone().or_else(|| history::extract_html_title(&out));
    history::record(url, page_title.as_deref());
    tabs::ensure_initialized(url);
    if let Some(ref title) = page_title {
        tabs::update_active_title(title);
    } else {
        tabs::update_active_title(&tabs::default_title_for_url(url));
    }
    // Preserve `document.title = ...` set during initial script execution rather
    // than clobbering it with the static `<title>` element.
    if js_title.is_none() {
        let active_title = tabs::active_title();
        let window_title = if zipp_shell::profile::is_incognito() {
            format!("{active_title} — Zipp Browser (Incognito)")
        } else {
            format!("{active_title} — Zipp Browser")
        };
        push_window_request(WindowRequest::Title(window_title));
    }
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
    let esc = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    };
    let token = chrome_token();
    format!(
        r#"<!DOCTYPE html><html lang="en"><head><meta charset="utf-8"><title>Can't load page — Zipp Browser</title><style>
*{{box-sizing:border-box}}
body{{margin:0;min-height:100vh;background:#0a0e17;color:#e2e8f0;font-family:'Segoe UI',-apple-system,Arial,sans-serif;display:flex;align-items:center;justify-content:center;padding:24px}}
.card{{width:100%;max-width:560px;background:#131b2c;border:1px solid #1f2a40;border-radius:16px;padding:28px 30px;box-shadow:0 8px 30px rgba(0,0,0,0.35)}}
.ico{{width:44px;height:44px;border-radius:12px;background:rgba(239,68,68,0.14);border:1px solid rgba(239,68,68,0.3);color:#f87171;display:flex;align-items:center;justify-content:center;margin-bottom:16px}}
h1{{margin:0 0 8px;font-size:21px;font-weight:700;color:#f8fafc;letter-spacing:-0.01em}}
.url{{margin:0 0 14px;padding:8px 12px;background:#0c111d;border:1px solid #1f2a40;border-radius:8px;font-family:Consolas,'Liberation Mono',monospace;font-size:12px;color:#93c5fd;overflow-wrap:anywhere}}
.detail{{margin:0 0 22px;font-size:13px;line-height:1.55;color:#94a3b8;overflow-wrap:anywhere}}
.row{{display:flex;flex-wrap:wrap;gap:9px}}
.btn{{display:inline-flex;align-items:center;height:36px;padding:0 15px;border-radius:9px;font-size:13px;font-weight:600;text-decoration:none;border:1px solid transparent}}
.primary{{background:linear-gradient(180deg,#3b82f6 0%,#2563eb 100%);color:#ffffff}}
.ghost{{background:#0c111d;border-color:#2d3d5c;color:#e2e8f0}}
</style></head><body><div class="card">
<div class="ico"><svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/></svg></div>
<h1>This page can't be loaded</h1>
<div class="url">{url}</div>
<p class="detail">{err}</p>
<div class="row">
  <a class="btn primary" href="zipp-nav:reload?__zipp_chrome={token}">Try Again</a>
  <a class="btn ghost" href="zipp-nav:home?__zipp_chrome={token}">Open Home</a>
  <a class="btn ghost" href="zipp-nav:settings?__zipp_chrome={token}">Properties</a>
</div>
</div></body></html>"#,
        url = esc(url),
        err = esc(error),
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

/// Pages are rendered without the browser's toolbar (`--reftest`, `--kiosk`,
/// `--app`).
static NO_CHROME: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether pages are shown with the browser's toolbar (and its console).
pub(crate) fn chrome_shown() -> bool {
    !NO_CHROME.load(std::sync::atomic::Ordering::Relaxed)
}

/// When the person last pressed a key or a button `(scope, ms since start)`.
static LAST_GESTURE: Mutex<(u64, u64)> = Mutex::new((0, u64::MAX));

fn since_start() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64
}

/// The person pressed something in `scope` (the event bridge says; a page cannot).
pub(crate) fn note_user_gesture(scope: u64) {
    *LAST_GESTURE.lock().unwrap_or_else(|e| e.into_inner()) = (scope, since_start());
}

fn user_just_acted() -> bool {
    let (g_scope, last) = *LAST_GESTURE.lock().unwrap_or_else(|e| e.into_inner());
    if last == u64::MAX || since_start().saturating_sub(last) >= 5000 {
        return false;
    }
    let cur = async_fetch::current_scope();
    cur == 0 || g_scope == 0 || g_scope == cur
}

/// What page script asks of its window, for the event bridge to carry out
/// where it has the document (and its shell).
pub(crate) enum WindowRequest {
    Title(String),
    Fullscreen(bool),
}

static WINDOW_REQUESTS: Mutex<Vec<WindowRequest>> = Mutex::new(Vec::new());

fn push_window_request(request: WindowRequest) {
    WINDOW_REQUESTS.lock().unwrap_or_else(|e| e.into_inner()).push(request);
}

fn pending_window_title_count() -> usize {
    WINDOW_REQUESTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter(|r| matches!(r, WindowRequest::Title(_)))
        .count()
}

fn last_window_title_since(before: usize) -> Option<String> {
    let reqs = WINDOW_REQUESTS.lock().unwrap_or_else(|e| e.into_inner());
    let mut seen = 0usize;
    let mut last = None;
    for r in reqs.iter() {
        if let WindowRequest::Title(t) = r {
            seen += 1;
            if seen > before {
                last = Some(t.clone());
            }
        }
    }
    last
}

/// Carry out what the page asked of its window since last time.
pub(crate) fn apply_window_requests(doc: &zipp_dom::BaseDocument) {
    let requests = std::mem::take(&mut *WINDOW_REQUESTS.lock().unwrap_or_else(|e| e.into_inner()));
    for request in requests {
        match request {
            WindowRequest::Title(title) => doc.shell_provider.set_window_title(title),
            WindowRequest::Fullscreen(on) => doc.shell_provider.set_fullscreen(on),
        }
    }
}

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
    ANY_FILE.store(true, std::sync::atomic::Ordering::Relaxed);
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

    console::record_this_thread();
    devtools::record_this_thread();
    async_fetch::allow_on_this_thread();
    worker::allow_on_this_thread();
    worker::new_page();
    let shot_page = worker::current_page();
    tabs::init(url);
    if let Ok(extra) = std::env::var("ZIPP_EXTRA_TABS") {
        for raw in extra.split('|').filter(|s| !s.trim().is_empty()) {
            let extra_url = normalize_url(raw.trim());
            tabs::open_new_tab(Some(extra_url), false, false);
        }
    }
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
            if let Ok(JsValue::String(updated)) = state.call_global("__getRenderedHTML", &[])
                && !updated.is_empty() {
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
        #[cfg(windows)]
        media_provider: Some(Arc::new(media::MediaFoundationProvider::muted()) as _),
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
        if bridge.handler_count() > 0 || bridge.has_js_state() {
            doc.set_event_handler(Box::new(bridge));
        }
    }
    sync_iframes(doc.as_mut(), url, 0);
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
    // Pump the live document loop so async mount effects, dynamic imports,
    // Web Workers (e.g. SoftN container/archive workers), JS-driven `<video>`,
    // and initial WebGL frames all settle before clicks or capture.
    {
        use zipp_dom::Document;
        let wait_secs: u64 = std::env::var("ZIPP_SHOT_WAIT_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10);
        let t_w = std::time::Instant::now();
        let max_wait = std::time::Duration::from_secs(wait_secs);
        let mut quiet = 0u32;
        let mut anim_ticks = 0u32;
        while t_w.elapsed() < max_wait {
            let next_timer = doc.run_tasks();
            let anim = doc.animation_frame();
            let net_busy = doc.poll(None) || !net_provider.is_empty();
            doc.as_mut().resolve(ANIMATION_SETTLE_SECS * 1.5);
            if worker::has_active() || net_busy {
                quiet = 0;
            } else if anim {
                // Allow finite rAF animations (e.g. 1.1s count-ups) to settle,
                // while capping continuous loops (WebGL) at ~1.5s.
                anim_ticks += 1;
                if anim_ticks >= 30 {
                    break;
                }
            } else if next_timer.is_some() {
                quiet += 1;
                if quiet >= 8 {
                    break;
                }
            } else {
                quiet += 1;
                if quiet >= 3 {
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        for _ in 0..3 {
            let _ = doc.run_tasks();
            let _ = doc.animation_frame();
            let _ = doc.poll(None);
            doc.as_mut().resolve(ANIMATION_SETTLE_SECS * 2.0);
        }
    }
    // Wait briefly for any active `<video>` players (both static markup and
    // JS-created `blob:`/`data:` videos) to decode Frame 0 before painting.
    #[cfg(windows)]
    {
        let video_ids: Vec<usize> = doc
            .as_ref()
            .query_selector_all("video")
            .map(|s| s.into_iter().collect())
            .unwrap_or_default();
        if !video_ids.is_empty() {
            let vt = std::time::Instant::now();
            while vt.elapsed() < std::time::Duration::from_millis(2500) {
                let mut any_player = false;
                let mut all_have_frame_or_err = true;
                for &id in &video_ids {
                    if let Some(player) = doc.as_ref().media_player(id) {
                        any_player = true;
                        let st = player.status();
                        if player.frame().is_none() && !st.error {
                            all_have_frame_or_err = false;
                        }
                    }
                }
                if !any_player || all_have_frame_or_err {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(30));
            }
            let _ = zipp_dom::Document::run_tasks(&mut doc);
            doc.as_mut().resolve(ANIMATION_SETTLE_SECS * 2.0);
        }
    }
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
    if let Ok(spec) = std::env::var("ZIPP_UICLICK") {
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
                // Pump post-click async work (e.g. dynamic ESM imports, GLB/HDR
                // model loads, and WebGL animation frames) before painting.
                let t_click = std::time::Instant::now();
                let click_budget = std::time::Duration::from_secs(5);
                let mut frames_drawn = 0u32;
                while t_click.elapsed() < click_budget && frames_drawn < 12 {
                    let _ = doc.run_tasks();
                    let animating = doc.animation_frame();
                    let _ = doc.poll(None);
                    doc.as_mut().resolve(ANIMATION_SETTLE_SECS * (2 + total * 2) as f64);
                    if animating {
                        frames_drawn += 1;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(80));
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
                if let Some(state) = js_state.as_mut()
                    && let Ok(JsValue::String(after)) =
                        state.call_global("__getRenderedHTML", &[])
                    {
                        let _ = std::fs::write(format!("{out_path}.click.html"), &after);
                        eprintln!("[UICLICK] wrote {out_path}.click.html");
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

    // `ZIPP_DEVTOOLS=elements|source|scripts|console` opens the Chrome-style
    // DevTools drawer before painting so headless screenshots can verify every tab.
    if let Ok(tab_name) = std::env::var("ZIPP_DEVTOOLS") {
        let tab = match tab_name.trim().to_ascii_lowercase().as_str() {
            "elements" | "inspect" => devtools::Tab::Elements,
            "source" => devtools::Tab::Source,
            "scripts" => devtools::Tab::Scripts,
            _ => devtools::Tab::Console,
        };
        if let Ok(src_mode) = std::env::var("ZIPP_DEVTOOLS_SOURCE")
            && src_mode.eq_ignore_ascii_case("rendered")
        {
            devtools::set_source_mode(devtools::SourceMode::Rendered);
        }
        if let Ok(sel) = std::env::var("ZIPP_DEVTOOLS_INSPECT")
            && let Ok(Some(node_id)) = doc.as_ref().query_selector(sel.trim())
        {
            devtools::inspect_node(doc.as_ref(), node_id);
        }
        devtools::set_tab(tab);
        let token = chrome_token();
        if let Ok(Some(style)) = doc.as_ref().query_selector("#__zipp_chrome_css") {
            doc.as_mut().mutate().set_inner_html(
                style,
                &chrome_css(console::is_open(), is_mobile_view()),
            );
        }
        if let Ok(Some(hdr)) = doc.as_ref().query_selector("#__zipp_dt_header") {
            doc.as_mut()
                .mutate()
                .set_inner_html(hdr, &devtools::header_html(token, false));
        }
        if let Ok(Some(body)) = doc.as_ref().query_selector("#__zipp_console_lines") {
            let html = devtools::body_html(Some(doc.as_ref()));
            doc.as_mut().mutate().set_inner_html(body, &html);
        }
        doc.as_mut().resolve(ANIMATION_SETTLE_SECS * 2.0);
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

    worker::terminate_page(shot_page);

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
    let fetch_scope = async_fetch::new_scope();
    let _fetch_guard = async_fetch::ScopeGuard::enter(fetch_scope);
    let base_url = extract_base_url(html, page_url);
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
            let full = resolve(&base_url, &src);
            if ty == "module" {
                module_entries.push(full);
            } else if is_js {
                match fetch(&full) {
                    Ok(source) => {
                        devtools::record_script(
                            devtools::ScriptKind::Classic,
                            &full,
                            source.clone(),
                            None,
                        );
                        external_scripts.push((full, source));
                    }
                    Err(err) => {
                        devtools::record_script(
                            devtools::ScriptKind::Classic,
                            &full,
                            String::new(),
                            Some(err),
                        );
                    }
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
                    import_map.add(code, &base_url, &resolve);
                    devtools::record_script(
                        devtools::ScriptKind::ImportMap,
                        &format!("{page_url}#importmap{abs}"),
                        code.to_string(),
                        None,
                    );
                } else if !code.trim().is_empty() {
                    if ty == "module" {
                        // An inline module still needs linking; give it a URL of
                        // its own so relative specifiers resolve against the page.
                        let url = format!("{base_url}#inline-module-{}", inline_modules.len());
                        inline_modules.insert(url.clone(), code.to_string());
                        module_entries.push(url);
                    } else {
                        let inline_url = format!("{page_url}#inline{abs}");
                        devtools::record_script(
                            devtools::ScriptKind::Inline,
                            &inline_url,
                            code.to_string(),
                            None,
                        );
                        external_scripts.push((inline_url, code.to_string()));
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

    static BASE_SHIM: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let base_shim = BASE_SHIM.get_or_init(|| {
        let dom_shim = platform_shim(&format!("{}\n{}", include_str!("console_shim.js"), include_str!("dom_shim.js")));
        format!(
            "{HOST_PRELUDE_JS}\n{dom_shim}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
            include_str!("clone.js"),
            include_str!("indexeddb_shim.js"),
            include_str!("worker_page.js"),
            include_str!("webgl_shim.js"),
            include_str!("canvas2d_shim.js"),
            include_str!("webgpu_shim.js"),
            include_str!("webusb_shim.js"),
            include_str!("webaudio_shim.js"),
        )
    });
    let router_helpers = include_str!("router_helpers.js");
    // Substitute the 5 small page placeholders in one pass (avoiding redundant full-shim
    // copies and the previously dead `__BODY_HTML__` escaping of the entire document).
    let escaped_url = page_url.replace('\\', "\\\\").replace('\'', "\\'");
    let escaped_title = js_quoted(&page_title(html));
    let vw_str = viewport.0.to_string();
    let vh_str = viewport.1.to_string();
    let dark_str = if system_prefers_dark() { "true" } else { "false" };
    let mut shim_prog = base_shim
        .replace("__PAGE_URL__", &escaped_url)
        .replace("__PAGE_TITLE__", &escaped_title)
        .replace("__VIEWPORT_W__", &vw_str)
        .replace("__VIEWPORT_H__", &vh_str)
        .replace("__PREFERS_DARK__", dark_str);
    if system::allowed() {
        shim_prog.push('\n');
        shim_prog.push_str(include_str!("zipp_shim.js"));
    }
    if is_mobile_view() {
        shim_prog.push_str("\nif (typeof __zippSetMobileMode === \"function\") __zippSetMobileMode(true);\n");
    }

    let mut prepared_scripts: Vec<(String, String)> = Vec::with_capacity(external_scripts.len());
    for (script_url, source) in &external_scripts {
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
        let prepared = js_opt::prepare_script(&patched);
        prepared_scripts.push((script_url.clone(), prepared));
    }

    // Link ES module graph if present.
    let mut module_bundle: Option<String> = None;
    if !module_entries.is_empty() {
        let t_link = std::time::Instant::now();
        let fetch_module = |url: &str| match inline_modules.get(url) {
            Some(code) => Ok(code.clone()),
            None => fetch(url),
        };
        let resolve_module = |base: &str, spec: &str| import_map.resolve_specifier(base, spec, &resolve);
        let linked = esm::link_graph_lazy(
            &module_entries,
            import_map.fingerprint(),
            &fetch_module,
            &resolve_module,
        );
        IMPORT_MAP.with(|m| *m.borrow_mut() = import_map.clone());
        eprintln!(
            "[ESM] Linked {} modules from {} entr{} in {}ms (lazy dynamic imports)",
            linked.module_count,
            module_entries.len(),
            if module_entries.len() == 1 { "y" } else { "ies" },
            t_link.elapsed().as_millis(),
        );
        for (mod_url, mod_src) in &linked.loaded_modules {
            let kind = if mod_url.contains("#inline-module-") {
                devtools::ScriptKind::Inline
            } else {
                devtools::ScriptKind::Module
            };
            devtools::record_script(kind, mod_url, mod_src.clone(), None);
        }
        for (url, err) in &linked.failures {
            eprintln!("[ESM] FAILED {url}: {err}");
            console::push(console::Level::Error, format!("module {url} failed to load: {err}"));
            devtools::record_script(devtools::ScriptKind::Module, url, String::new(), Some(err.clone()));
        }
        let mut mod_src = String::with_capacity(linked.source.len() + esm::MODULE_RUNTIME_JS.len() + 256);
        if std::env::var_os("ZIPP_DEBUG_ESM").is_some() {
            mod_src.push_str("\nvar __zippEsmDebug=true;\n");
        }
        mod_src.push_str(&format!("\nvar __zippImportMap = {};\n", import_map.to_js()));
        mod_src.push_str(esm::MODULE_RUNTIME_JS);
        mod_src.push_str(&linked.source);
        if let Ok(path) = std::env::var("ZIPP_DUMP_LINKED") {
            let _ = std::fs::write(&path, &linked.source);
            eprintln!("[ESM] wrote linked graph to {path}");
        }
        module_bundle = Some(mod_src);
    }

    let total_bytes: usize = shim_prog.len()
        + prepared_scripts.iter().map(|(_, s)| s.len()).sum::<usize>()
        + module_bundle.as_ref().map_or(0, String::len);
    if let Ok(path) = std::env::var("ZIPP_DUMP_PROGRAM") {
        let mut dump = shim_prog.clone();
        for (u, s) in &prepared_scripts {
            dump.push_str(&format!("\n/* === {u} === */\n"));
            dump.push_str(s);
            dump.push('\n');
        }
        if let Some(m) = &module_bundle {
            dump.push_str("\n/* === ESM === */\n");
            dump.push_str(m);
        }
        let _ = std::fs::write(&path, &dump);
        eprintln!("[JS] wrote the program to {path}");
    }
    eprintln!(
        "[JS] Compiling {} bytes ({} script{}, {})...",
        total_bytes,
        prepared_scripts.len(),
        if prepared_scripts.len() == 1 { "" } else { "s" },
        if module_bundle.is_some() { "+esm" } else { "classic" }
    );
    let t_compile = std::time::Instant::now();
    let mut state = match compile_script(&shim_prog) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[JS] Shim compile error: {e}");
            console::push(console::Level::Error, format!("SyntaxError: {e}"));
            return None;
        }
    };
    let compile_ms = t_compile.elapsed().as_millis();
    let t_init = std::time::Instant::now();
    state.set_host_call_ctx(Box::new(browser_host_call_ctx));
    state.set_unhandled_rejection_sink(Box::new(|reason| {
        eprintln!("[JS] Uncaught (in promise) {reason}");
        console::push(console::Level::Error, format!("Uncaught (in promise) {reason}"));
    }));
    if let Err(e) = state.run_init() {
        eprintln!("[JS] Shim init error: {e}");
        return None;
    }

    // Run each classic `<script>` tag independently via `run_script`, matching
    // HTML's ScriptEvaluation model: each tag gets its own 32,768-register
    // frame, global declarations persist across tags, microtasks drain after
    // each tag, and an error in one tag does not abort subsequent tags.
    let run_one = |state: &mut ScriptState, label: &str, code: &str| {
        match state.run_script(code) {
            Ok(_) => {}
            Err(e) if js_opt::is_register_limit_error(&e) => {
                eprintln!("[JS] {label}: hit 32768-register limit, retrying with aggressive AST split...");
                let agg = js_opt::prepare_script_aggressive(code);
                if let Err(e2) = state.run_script(&agg) {
                    eprintln!("[JS] Script error ({label}): {e2}");
                    console::push(console::Level::Error, e2);
                }
            }
            Err(e) => {
                let line = e
                    .split("<anonymous>:")
                    .nth(1)
                    .and_then(|at| at.split(':').next()?.trim().parse::<usize>().ok())
                    .and_then(|n| code.lines().nth(n.saturating_sub(1)))
                    .map(|l| l.chars().take(240).collect::<String>());
                eprintln!("[JS] Uncaught ({label}): {e}");
                if let Some(line) = &line {
                    eprintln!("[JS]   at: {line}");
                }
                console::push(
                    console::Level::Error,
                    match line {
                        Some(line) => format!("{e}\n  at: {line}"),
                        None => e,
                    },
                );
            }
        }
        state.run_microtasks();
    };

    for (script_url, code) in &prepared_scripts {
        run_one(&mut state, script_url, code);
    }
    if let Some(mod_src) = &module_bundle {
        run_one(&mut state, "esm-graph", mod_src);
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
    // Give any worker started during init a brief moment for an immediate reply
    // without blocking interactive window startup.
    if worker::has_active_for(fetch_scope) {
        let brief = std::time::Duration::from_millis(300);
        let t_w = std::time::Instant::now();
        while t_w.elapsed() < brief {
            if event_bridge::run_page_tasks(&mut state, 100, 1, std::time::Duration::from_millis(100)) == 0 {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    }
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

    if let Ok(JsValue::String(rendered)) = state.eval_in_context("__getRenderedHTML()")
        && !rendered.is_empty() {
            // Extract the event handler map before we lose the state.
            let event_map_json = match state.eval_in_context("__getEventMap()") {
                Ok(JsValue::String(s)) => s,
                _ => "{}".to_string(),
            };

            let (body_attrs, inner) = split_body_element(&rendered);
            let inner = make_urls_absolute(inner, &base_url);
            let inner = inner
                .replace("\\xa9", "\u{00A9}")
                .replace("\\u2019", "\u{2019}")
                .replace("\\u2018", "\u{2018}")
                .replace("\\u2014", "\u{2014}");
            let static_head = make_urls_absolute(&head_content(html), &base_url);
            let head_extras = match state.eval_in_context("__getHeadExtras()") {
                Ok(JsValue::String(s)) => s,
                _ => "[]".to_string(),
            };
            let head = merge_head_extras(&static_head, &head_extras, &base_url);
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
    // The page shows its own markup, not the script's tree (see
    // `__getRenderedHTML`), but its script lives on: its timers run and the
    // console can reach it. An empty event map says so (`JsEventBridge::new`).
    Some((html.to_string(), String::new(), state))
}

// ---------------------------------------------------------------------------
// Navigation provider
// ---------------------------------------------------------------------------

/// Handles link clicks, tab commands, and form submissions internally.
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

        // Back, forward, reload, home, settings, history, incognito, and tab controls.
        // Every `zipp-nav:` action requires either a native keyboard shortcut
        // (`application/x-zipp-shortcut`) or our unforgeable `__zipp_chrome`
        // token so untrusted web pages cannot hijack tabs, spawn windows, or clear data.
        if let Some(rest) = url.strip_prefix("zipp-nav:") {
            let trusted = options.content_type == "application/x-zipp-shortcut" || from_chrome(&url);
            if !trusted {
                eprintln!("[NAV] refused unauthenticated zipp-nav action: {url}");
                return;
            }
            let (command, query) = rest.split_once('?').unwrap_or((rest, ""));
            let param_u64 = |key: &str| -> Option<u64> {
                url::form_urlencoded::parse(query.as_bytes())
                    .find(|(k, _)| k == key)
                    .and_then(|(_, v)| v.parse::<u64>().ok())
            };
            match command {
                "incognito" => {
                    launch_incognito_window();
                    return;
                }
                "new-tab" => {
                    let (_, target) = tabs::open_new_tab(None, true, false);
                    if let Some(target) = target {
                        self.load_target(target);
                    }
                    return;
                }
                "close-tab" => {
                    let id = param_u64("id");
                    match tabs::close_tab(id) {
                        tabs::CloseResult::Switched(target) => self.load_target(target),
                        tabs::CloseResult::ClosedBackground => self.wake_tabbar(),
                        tabs::CloseResult::NotFound => {}
                    }
                    return;
                }
                "reopen-tab" => {
                    if let Some(target) = tabs::reopen_closed_tab() {
                        self.load_target(target);
                    }
                    return;
                }
                "switch-tab" => {
                    let Some(id) = param_u64("id") else {
                        return;
                    };
                    // Middle-clicking a tab in the tab strip closes that tab, just like Chrome/Firefox.
                    if options.new_tab {
                        match tabs::close_tab(Some(id)) {
                            tabs::CloseResult::Switched(target) => self.load_target(target),
                            tabs::CloseResult::ClosedBackground => self.wake_tabbar(),
                            tabs::CloseResult::NotFound => {}
                        }
                        return;
                    }
                    if let Some(target) = tabs::switch_to_id(id) {
                        self.load_target(target);
                    }
                    return;
                }
                "next-tab" => {
                    if let Some(target) = tabs::switch_by_offset(1) {
                        self.load_target(target);
                    }
                    return;
                }
                "prev-tab" => {
                    if let Some(target) = tabs::switch_by_offset(-1) {
                        self.load_target(target);
                    }
                    return;
                }
                cmd if cmd.starts_with("tab-") => {
                    if let Some(slot) = cmd.strip_prefix("tab-").and_then(|s| s.parse::<usize>().ok())
                        && let Some(target) = tabs::switch_to_slot(slot)
                    {
                        self.load_target(target);
                    }
                    return;
                }
                "back" => {
                    match tabs::active_back() {
                        Some(target) => self.load_target(target),
                        None => eprintln!("[NAV] nothing to go back to"),
                    }
                    return;
                }
                "forward" => {
                    match tabs::active_forward() {
                        Some(target) => self.load_target(target),
                        None => eprintln!("[NAV] nothing to go forward to"),
                    }
                    return;
                }
                "home" => {
                    let home = normalize_url(&settings::start_page());
                    let target = tabs::navigate_active(home);
                    self.load_target(target);
                    return;
                }
                "settings" => {
                    let target = tabs::navigate_active("zipp:settings".to_string());
                    self.load_target(target);
                    return;
                }
                "history" => {
                    let target = tabs::navigate_active("zipp:history".to_string());
                    self.load_target(target);
                    return;
                }
                "reload" => {
                    let current = tabs::active_url();
                    PAGE_CACHE.lock().unwrap_or_else(|e| e.into_inner()).remove(&current);
                    zipp_shell::cache::invalidate(&current);
                    let target = tabs::active_reload();
                    self.load_target(target);
                    return;
                }
                "hard-reload" | "clear-cache" => {
                    clear_browser_cache();
                    let target = tabs::active_reload();
                    self.load_target(target);
                    return;
                }
                "clear-data" => {
                    clear_all_site_data();
                    let target = tabs::active_reload();
                    self.load_target(target);
                    return;
                }
                _ => {
                    eprintln!("[NAV] unknown zipp-nav command: {command}");
                    return;
                }
            }
        }

        // Properties panel actions (`zipp-settings:<action>?__zipp_chrome=<token>&...`).
        // Strictly authenticated with the per-run chrome token so untrusted web
        // pages can never alter settings, cache, or history.
        if let Some(rest) = url.strip_prefix("zipp-settings:") {
            if !from_chrome(&url) {
                eprintln!("[NAV] refused unauthenticated zipp-settings action: {url}");
                return;
            }
            let (action, query) = rest.split_once('?').unwrap_or((rest, ""));
            let params: HashMap<String, String> = url::form_urlencoded::parse(query.as_bytes())
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            let last_web = tabs::last_web_url();
            let (page_url, notice, filter): (&str, Option<String>, Option<String>) = match action {
                "save-start" | "set-start" => {
                    let raw = params
                        .get("start_page")
                        .or_else(|| params.get("url"))
                        .map(String::as_str)
                        .unwrap_or("")
                        .trim();
                    let next = if raw.is_empty() {
                        settings::DEFAULT_START_PAGE.to_string()
                    } else {
                        normalize_url(raw)
                    };
                    settings::update(|s| s.start_page = next.clone());
                    let suffix = if zipp_shell::profile::is_incognito() {
                        " (for this Incognito session only)"
                    } else {
                        ""
                    };
                    ("zipp:settings", Some(format!("Start page set to {next}{suffix}.")), None)
                }
                "toggle-cache" => {
                    let updated = settings::update(|s| s.cache_enabled = !s.cache_enabled);
                    let msg = if updated.cache_enabled {
                        "HTTP cache enabled."
                    } else {
                        "HTTP cache disabled."
                    };
                    ("zipp:settings", Some(msg.to_string()), None)
                }
                "cache-limit" => {
                    let mb = params
                        .get("mb")
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(settings::DEFAULT_CACHE_MAX_DISK_MB);
                    let updated = settings::update(|s| s.cache_max_disk_mb = mb);
                    (
                        "zipp:settings",
                        Some(format!("Disk cache limit updated to {} MB.", updated.cache_max_disk_mb)),
                        None,
                    )
                }
                "clear-cache" => {
                    let (entries, bytes) = clear_browser_cache();
                    let mb = bytes as f64 / (1024.0 * 1024.0);
                    (
                        "zipp:settings",
                        Some(format!("Cleared HTTP cache ({entries} entries, {mb:.2} MB freed).")),
                        None,
                    )
                }
                "clear-data" => {
                    let (entries, bytes, cookies) = clear_all_site_data();
                    let mb = bytes as f64 / (1024.0 * 1024.0);
                    (
                        "zipp:settings",
                        Some(format!(
                            "Cleared all site data ({entries} cache entries / {mb:.2} MB, {cookies} cookies, and site storage)."
                        )),
                        None,
                    )
                }
                "toggle-mute" => {
                    let updated = settings::update(|s| s.mute_media = !s.mute_media);
                    let msg = if updated.mute_media {
                        "Browser audio & video muted."
                    } else {
                        "Browser audio & video unmuted."
                    };
                    ("zipp:settings", Some(msg.to_string()), None)
                }
                "clear-history" => {
                    let n = history::clear();
                    (
                        "zipp:history",
                        Some(format!("Cleared browsing history ({n} entries removed).")),
                        None,
                    )
                }
                "delete-history" => {
                    let id = params.get("id").and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
                    let removed = history::delete(id);
                    let msg = if removed {
                        Some("Removed entry from browsing history.".to_string())
                    } else {
                        None
                    };
                    ("zipp:history", msg, None)
                }
                "filter-history" => {
                    let q = params.get("q").cloned().filter(|s| !s.trim().is_empty());
                    ("zipp:history", None, q)
                }
                _ => ("zipp:settings", None, None),
            };
            let html = properties_page::render(
                chrome_token(),
                notice.as_deref(),
                filter.as_deref(),
                last_web.as_deref(),
                page_url == "zipp:history",
            );
            let title = if page_url == "zipp:history" { "History" } else { "Properties" };
            tabs::set_active_internal(page_url, title);
            self.proxy.send_event(zipp_shell::ShellEvent::NavigationLoad {
                url: page_url.to_string(),
                contents: html,
                retain_scroll_position: false,
                is_md: false,
            });
            return;
        }

        // The console's and viewport toggle's own links: handled directly by the event bridge.
        if url.starts_with("zipp-console:")
            || url.starts_with("zipp-devtools:")
            || url.starts_with("zipp-view:")
        {
            return;
        }
        // Code typed into the console. Its line is a form posting back to the
        // page, like the address bar's; the code waits here for the page's
        // thread (`JsEventBridge::run_tasks`).
        let chrome = from_chrome(&url);
        if chrome && url.contains("__js=")
            && let Ok(parsed) = url::Url::parse(&url)
                && let Some((_, code)) = parsed.query_pairs().find(|(k, _)| k == "__js") {
                    console::submit(code.into_owned());
                    return;
                }

        // The PDF viewer's Save button: write the file to Downloads and show the
        // viewer again with a note of where it went.
        if let Some(target) = url.strip_prefix("zipp-save:") {
            let target = target.to_string();
            // The file showing, and no other: a page's link could otherwise
            // copy any file to Downloads.
            if target != tabs::active_url() {
                eprintln!("[NAV] refused to save {target}: it is not the page showing");
                return;
            }
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
        // navigation (unless the user asked to open it in a new tab).
        if !options.new_tab
            && let Ok(target) = url::Url::parse(&url)
            && let Some(fragment) = target.fragment() {
                let current = tabs::active_url();
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
        // Another document: the server never sees the fragment.
        if let Some((base, _)) = url.split_once('#') {
            url = base.to_string();
        }

        // Check if this is an address bar or New Tab search form submission.
        if chrome && url.contains("__nav_url=") {
            if let Ok(parsed) = url::Url::parse(&url)
                && let Some((_, typed)) = parsed.query_pairs().find(|(k, _)| k == "__nav_url")
            {
                if typed.trim().is_empty() {
                    return;
                }
                url = normalize_url(&typed);
                eprintln!("[NAV] Address bar -> {url}");
            }
        } else if url.starts_with("file:") && !tabs::active_url().starts_with("file:") {
            // A page on the web does not open the machine's files, by a link
            // or by script. The person can, by typing the address.
            eprintln!("[NAV] refused: a page on the web may not open {url}");
            console::push(console::Level::Error, format!("Not allowed to load local resource: {url}"));
            return;
        }

        if options.new_tab {
            if options.background {
                let (tab_id, _) = tabs::open_new_tab(Some(url.clone()), false, true);
                self.wake_tabbar();
                self.prefetch_background_tab(tab_id, url);
            } else {
                let (_, target) = tabs::open_new_tab(Some(url), true, true);
                if let Some(target) = target {
                    self.load_target(target);
                }
            }
            return;
        }

        let target = tabs::navigate_active(url);
        self.load_target(target);
    }
}

impl BrowserNavProvider {
    /// Wake the window event loop so `JsEventBridge::run_tasks` updates the tab strip
    /// in-place without reloading the active document.
    fn wake_tabbar(&self) {
        self.proxy
            .send_event(zipp_shell::ShellEvent::Embedder(Arc::new(())));
    }

    /// Fetch a background tab asynchronously to populate its title & cached HTML
    /// without disturbing the active tab.
    fn prefetch_background_tab(&self, tab_id: u64, url: String) {
        if tabs::is_internal_url(&url) {
            return;
        }
        let proxy = self.proxy.clone();
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(move || {
                let contents = match page_cache_get(&url) {
                    Some(html) => html,
                    None => match viewers::fetch_document(&url, true) {
                        Ok(html) => {
                            page_cache_put(url.clone(), html.clone());
                            html
                        }
                        Err(e) => error_page(&url, &e),
                    },
                };
                let title = history::extract_html_title(&contents);
                tabs::store_tab_html(tab_id, &url, contents, title.as_deref());
                proxy.send_event(zipp_shell::ShellEvent::Embedder(Arc::new(())));
            })
            .ok();
    }

    /// Show `target`: serve internal pages and cached tab switches immediately,
    /// or fetch on a worker thread guarded by `(tab_id, nav_gen)` so late
    /// responses never clobber a tab the user has already switched away from.
    fn load_target(&self, target: tabs::TabTarget) {
        let last_web = tabs::last_web_url();
        if let Some(html) = internal_page_html(&target.url, None, None, last_web.as_deref()) {
            self.proxy.send_event(zipp_shell::ShellEvent::NavigationLoad {
                url: target.url,
                contents: html,
                retain_scroll_position: false,
                is_md: false,
            });
            return;
        }
        if let Some(cached_html) = target.cached_html {
            eprintln!("[NAV] Instant tab switch -> {}", target.url);
            self.proxy.send_event(zipp_shell::ShellEvent::NavigationLoad {
                url: target.url,
                contents: cached_html,
                retain_scroll_position: false,
                is_md: false,
            });
            return;
        }
        let proxy = self.proxy.clone();
        let tab_id = target.id;
        let nav_gen = target.nav_gen;
        let url = target.url;

        // Immediately reflect loading indicator on the tab bar if it takes a moment.
        self.wake_tabbar();

        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(move || {
                // Only the fetch happens here. The page's JS runs on the main
                // thread, in the document builder, so that its live state can
                // stay attached to the document it rendered.
                let cached = page_cache_get(&url);
                let contents = match cached {
                    Some(html) => {
                        eprintln!("[NAV] Cache hit for {url}");
                        html
                    }
                    None => {
                        eprintln!("[NAV] Loading {url}...");
                        match viewers::fetch_document(&url, true) {
                            Ok(html) => {
                                page_cache_put(url.clone(), html.clone());
                                html
                            }
                            Err(e) => {
                                eprintln!("[NAV] Fetch error: {e}");
                                error_page(&url, &e)
                            }
                        }
                    }
                };
                let title = history::extract_html_title(&contents);
                tabs::store_tab_html(tab_id, &url, contents.clone(), title.as_deref());
                if tabs::is_active_nav_gen(tab_id, nav_gen) {
                    proxy.send_event(zipp_shell::ShellEvent::NavigationLoad {
                        url,
                        contents,
                        retain_scroll_position: false,
                        is_md: false,
                    });
                } else {
                    // User moved on to another tab while this fetch was in flight:
                    // update the background tab's title without clobbering the active tab.
                    proxy.send_event(zipp_shell::ShellEvent::Embedder(Arc::new(())));
                }
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
const TAB_BAR_HEIGHT: u32 = 36;
const NAV_BAR_HEIGHT: u32 = 44;
const TOOLBAR_HEIGHT: u32 = TAB_BAR_HEIGHT + NAV_BAR_HEIGHT;

pub(crate) fn chrome_css(console_open: bool, mobile_open: bool) -> String {
    let panel = console::PANEL_HEIGHT;
    let band = if console_open { TOOLBAR_HEIGHT + panel } else { TOOLBAR_HEIGHT };
    let incognito = zipp_shell::profile::is_incognito();
    let active_bg = if incognito { "rgba(167, 139, 250, 0.22)" } else { "#dbeafe" };
    let hover_bg = if incognito { "rgba(167, 139, 250, 0.14)" } else { "rgba(15, 23, 42, 0.07)" };
    let tab_hover_bg = if incognito { "rgba(139, 92, 246, 0.16)" } else { "rgba(255, 255, 255, 0.62)" };
    let close_hover_bg = if incognito { "rgba(167, 139, 250, 0.28)" } else { "rgba(15, 23, 42, 0.12)" };
    let close_hover_fg = if incognito { "#ffffff" } else { "#0f172a" };
    let (display, console_bg, inspect_bg, form_display) = if console_open {
        let on_console = devtools::active_tab() == devtools::Tab::Console;
        (
            "flex",
            if on_console { active_bg } else { "transparent" },
            if on_console { "transparent" } else { active_bg },
            if on_console { "flex" } else { "none" },
        )
    } else {
        ("none", "transparent", "transparent", "none")
    };
    let mobile_bg = if mobile_open { active_bg } else { "transparent" };
    format!(
        "html {{ border-top:{band}px solid transparent !important; box-sizing:border-box !important; margin:0 !important; max-width:none !important; }}\n\
         #__zipp_chrome {{ top:-{band}px; }}\n\
         #__zipp_navbar a:hover {{ background:{hover_bg}; }}\n\
         #__zipp_tabbar .__zt_inactive:hover {{ background:{tab_hover_bg} !important; }}\n\
         #__zipp_tabbar .__zt_close:hover {{ background:{close_hover_bg} !important; color:{close_hover_fg} !important; }}\n\
         #__zipp_tabbar #__zipp_new_tab_btn:hover {{ background:{hover_bg} !important; }}\n\
         #__zipp_console {{ display:{display}; top:-{panel}px; height:{panel}px; }}\n\
         #__zipp_console_form {{ display:{form_display} !important; }}\n\
         #__zipp_console_copy:hover {{ background:rgba(96,165,250,0.24) !important; }}\n\
         #__zipp_dt_header a:hover {{ background:rgba(148,163,184,0.16); }}\n\
         #__zipp_console_lines a:hover {{ background:rgba(148,163,184,0.12); }}\n\
         #__zipp_inspect_button {{ background:{inspect_bg}; }}\n\
         #__zipp_console_button {{ background:{console_bg}; }}\n\
         #__zipp_mobile_button {{ background:{mobile_bg}; }}\n\
         @media (max-width: 520px) {{ #__zipp_tabbar {{ padding:4px 4px 0 4px !important; gap:3px !important; }} #__zipp_navbar {{ padding:0 6px !important; gap:2px !important; }} #__zipp_navbar a {{ width:26px !important; height:26px !important; }} #__zipp_navbar > span {{ display:none !important; }} }}\n"
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
    // Always select the LAST match: `wrap_with_toolbar` appends real chrome at the
    // end of `<body>`, so a page-authored element with the same `id` cannot hijack it.
    for id in ["#__zipp_chrome", "#__zipp_console"] {
        if let Ok(nodes) = doc.query_selector_all(id)
            && let Some(&node) = nodes.last()
        {
            doc.mutate().append_children(root, &[node]);
        }
    }
}

/// Parse a page built by `render_page` and put its chrome in place, with any
/// WebGL frame the page drew while it settled.
fn build_document(final_html: &str, config: DocumentConfig) -> HtmlDocument {
    let mut doc = HtmlDocument::from_html(final_html, config);
    mount_chrome(&mut doc);
    let scope = async_fetch::peek_last_scope();
    canvas::forget_shown();
    canvas::retain_scope(scope);
    apply_canvas_frames_for(&mut doc, scope);
    doc
}

/// Auto-focus the address bar on the New Tab page so the user can type a URL immediately.
fn focus_newtab_address_bar(doc: &mut HtmlDocument, url: &str) {
    if url == tabs::NEW_TAB_URL
        && chrome_shown()
        && let Ok(Some(input_id)) = doc.as_ref().query_selector("#__zipp_url_input")
    {
        doc.as_mut().set_focus_to(input_id);
    }
}

// ---------------------------------------------------------------------------
// Dynamic <head> merging + live <iframe> sub-document mounting
// ---------------------------------------------------------------------------

/// Resolve `<base href="...">` inside `<head>` if present, otherwise `page_url`.
fn extract_base_url(html: &str, page_url: &str) -> String {
    let head = head_content(html);
    let lower = head.to_ascii_lowercase();
    let mut pos = 0;
    while let Some(rel) = lower[pos..].find("<base") {
        let abs = pos + rel;
        let next = lower.as_bytes().get(abs + 5).copied();
        if matches!(next, Some(b' ' | b'\t' | b'\n' | b'\r' | b'>' | b'/'))
            && let Some(end_rel) = head[abs..].find('>')
        {
            let tag = &head[abs..abs + end_rel + 1];
            if let Some(href) = attr(tag, "href")
                && !href.trim().is_empty()
            {
                return resolve(page_url, href.trim());
            }
            pos = abs + end_rel + 1;
            continue;
        }
        pos = abs + 5;
    }
    page_url.to_string()
}

/// Check whether `head` already contains a `<link rel="...stylesheet..." href="<url>">`.
fn head_has_stylesheet_link(head: &str, abs_href: &str) -> bool {
    let lower = head.to_ascii_lowercase();
    let mut pos = 0;
    while let Some(rel) = lower[pos..].find("<link") {
        let abs = pos + rel;
        let Some(end_rel) = head[abs..].find('>') else {
            break;
        };
        let tag = &head[abs..abs + end_rel + 1];
        if let Some(r) = attr(tag, "rel")
            && r.split_ascii_whitespace().any(|t| t.eq_ignore_ascii_case("stylesheet"))
            && let Some(h) = attr(tag, "href")
            && h == abs_href
        {
            return true;
        }
        pos = abs + end_rel + 1;
    }
    false
}

/// Merge dynamically promoted/injected `<link rel="stylesheet">` and `<style>`
/// elements reported by `__getHeadExtras()` into the static `<head>` markup.
pub(crate) fn merge_head_extras(static_head: &str, extras_json: &str, base_url: &str) -> String {
    let Ok(items) = serde_json::from_str::<Vec<(String, String)>>(extras_json) else {
        return static_head.to_string();
    };
    if items.is_empty() {
        return static_head.to_string();
    }
    let mut out = static_head.to_string();
    let mut added_links = std::collections::HashSet::new();
    for (kind, val) in items {
        if kind == "link" {
            let abs = resolve(base_url, val.trim());
            if abs.is_empty() || !added_links.insert(abs.clone()) {
                continue;
            }
            if !head_has_stylesheet_link(&out, &abs) {
                let esc = escape_html_attr(&abs);
                out.push_str(&format!(r#"<link rel="stylesheet" href="{esc}" />"#));
            }
        } else if kind == "style" {
            let trimmed = val.trim();
            if !trimmed.is_empty() && !out.contains(trimmed) {
                out.push_str("<style>");
                out.push_str(trimmed);
                out.push_str("</style>");
            }
        }
    }
    out
}

/// Failed iframe URLs on the current page so we don't retry broken frames every tick.
static FAILED_IFRAMES: std::sync::LazyLock<Mutex<std::collections::HashSet<String>>> =
    std::sync::LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

fn clear_failed_iframes() {
    if let Ok(mut set) = FAILED_IFRAMES.lock() {
        set.clear();
    }
}

/// Fetch an embedded `<iframe>` document with standard browser navigation headers
/// (`Sec-Fetch-Dest: iframe`, `Sec-Fetch-Mode: navigate`, `Sec-Fetch-Site`).
fn fetch_iframe_document(url: &str, parent_url: &str, no_referrer: bool) -> Result<String, String> {
    if url.starts_with("data:") || url.starts_with("blob:") || url.starts_with("file:") {
        return fetch(url);
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(format!("unsupported iframe scheme: {url}"));
    }
    let client = client_for_page(url, CredentialsMode::Include);
    let same = web_origin(parent_url).is_some()
        && web_origin(parent_url) == web_origin(url);
    let mut req = client
        .get(url)
        .header(
            "Accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .header("Sec-Fetch-Dest", "iframe")
        .header("Sec-Fetch-Mode", "navigate")
        .header("Sec-Fetch-Site", if same { "same-origin" } else { "cross-site" });
    if !no_referrer && (parent_url.starts_with("http://") || parent_url.starts_with("https://")) {
        req = req.header("Referer", parent_url);
    }
    let resp = req.send().map_err(|e| format!("iframe fetch {url}: {e}"))?;
    let status = resp.status().as_u16();
    let bytes = resp
        .bytes()
        .map_err(|e| format!("iframe body {url}: {e}"))?
        .to_vec();
    eprintln!("[IFRAME] GET {url} -> {status} ({} bytes)", bytes.len());
    if status >= 400 && bytes.is_empty() {
        return Err(format!("HTTP {status}"));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Build a live, interactive `HtmlDocument` for an `<iframe>` without browser
/// chrome or mutating top-level page state (`CURRENT_URL`, history, tabs, etc.).
fn build_iframe_document(
    parent: &BaseDocument,
    iframe_url: &str,
    html: &str,
    iframe_size: (u32, u32),
    depth: u8,
    allow_scripts: bool,
    allow_same_origin: bool,
) -> HtmlDocument {
    use zipp_dom::traits_src::shell::Viewport;

    let origin_url = if allow_same_origin { iframe_url } else { "about:blank" };
    let _page_guard = ThreadPageGuard::enter(origin_url);
    let _subdoc_guard = SubdocumentGuard::enter(true);
    let _iframe_guard = devtools::IframeScope::enter();
    let saved_import_map = IMPORT_MAP.with(|m| m.borrow().clone());

    devtools::record_script(
        devtools::ScriptKind::Iframe,
        iframe_url,
        html.to_string(),
        None,
    );

    let (rendered, event_map_json, js_state) = if allow_scripts && has_script_tag(html) {
        match run_zipp_js(html, origin_url, iframe_size) {
            Some((r, emap, state)) => (r, Some(emap), Some(state)),
            None => (html.to_string(), None, None),
        }
    } else {
        (html.to_string(), None, None)
    };

    // Restore parent page's import map immediately after iframe script init.
    IMPORT_MAP.with(|m| *m.borrow_mut() = saved_import_map);

    let out = strip_noscript(&rendered);
    let out = out
        .replace("class=\"no-js", "class=\"js")
        .replace("class=\"client-nojs", "class=\"client-js");
    let out = inline_images(&out, iframe_url);

    let pvp = parent.viewport();
    let scale = pvp.scale();
    let vp = Viewport::new(
        ((iframe_size.0 as f32) * scale).round().max(1.0) as u32,
        ((iframe_size.1 as f32) * scale).round().max(1.0) as u32,
        scale,
        pvp.color_scheme,
    );
    let config = DocumentConfig {
        font_ctx: Some(parent.clone_font_ctx()),
        preloaded_fonts: parent.clone_registered_fonts(),
        base_url: Some(iframe_url.to_string()),
        net_provider: Some(parent.net_provider.clone()),
        navigation_provider: Some(parent.navigation_provider.clone()),
        shell_provider: Some(parent.shell_provider.clone()),
        html_parser_provider: Some(parent.html_parser_provider.clone()),
        media_provider: parent.media_provider.clone(),
        viewport: Some(vp),
        ..Default::default()
    };

    let mut sub_doc = HtmlDocument::from_html(&out, config);
    let iframe_scope = async_fetch::peek_last_scope();
    apply_canvas_frames_for(&mut sub_doc, iframe_scope);

    let emap_json = event_map_json.unwrap_or_else(|| "{}".to_string());
    let flid_map = event_bridge::build_flid_map(sub_doc.as_ref());
    let bridge = event_bridge::JsEventBridge::new_subdocument(
        flid_map,
        &emap_json,
        js_state,
        origin_url,
        depth + 1,
    );
    sub_doc.set_event_handler(Box::new(bridge));
    sync_iframes(sub_doc.as_mut(), iframe_url, depth + 1);
    sub_doc.as_mut().resolve(0.0);
    sub_doc
}

/// Mount or update live `SubDocument`s for all `<iframe>` elements in `doc`.
///
/// Safe to call repeatedly (after initial parse, after JS re-renders, and in
/// `run_tasks`): already-mounted iframes whose `src`/`srcdoc` matches are
/// skipped in O(1), while newly-added or `src`-changed iframes (e.g. clicking
/// "Reset" on a live sandbox demo) are fetched and mounted immediately.
pub(crate) fn sync_iframes(doc: &mut BaseDocument, parent_url: &str, depth: u8) -> bool {
    const MAX_IFRAME_DEPTH: u8 = 2;
    if depth >= MAX_IFRAME_DEPTH {
        return false;
    }
    let Ok(iframe_ids) = doc.query_selector_all("iframe") else {
        return false;
    };
    if iframe_ids.is_empty() {
        return false;
    }

    let mut mounted_any = false;
    let src_attr = zipp_dom::LocalName::from("src");
    let srcdoc_attr = zipp_dom::LocalName::from("srcdoc");
    let refpol_attr = zipp_dom::LocalName::from("referrerpolicy");
    let sandbox_attr = zipp_dom::LocalName::from("sandbox");

    for iframe_id in iframe_ids {
        let Some(node) = doc.get_node(iframe_id) else {
            continue;
        };
        let Some(el) = node.element_data() else {
            continue;
        };
        let raw_src = el.attr(src_attr.clone()).map(str::trim).unwrap_or("");
        let raw_srcdoc = el.attr(srcdoc_attr.clone()).map(str::to_string);
        let no_referrer = el
            .attr(refpol_attr.clone())
            .is_some_and(|v| v.eq_ignore_ascii_case("no-referrer"));
        let (allow_scripts, allow_same_origin) = match el.attr(sandbox_attr.clone()) {
            None => (true, true),
            Some(tokens) => {
                let mut scripts = false;
                let mut same_org = false;
                for tok in tokens.split_ascii_whitespace() {
                    if tok.eq_ignore_ascii_case("allow-scripts") {
                        scripts = true;
                    } else if tok.eq_ignore_ascii_case("allow-same-origin") {
                        same_org = true;
                    }
                }
                (scripts, same_org)
            }
        };

        let target_url = if let Some(ref sd) = raw_srcdoc {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            sd.hash(&mut h);
            format!("about:srcdoc#{}", h.finish())
        } else if !raw_src.is_empty()
            && !raw_src.eq_ignore_ascii_case("about:blank")
            && !raw_src.to_ascii_lowercase().starts_with("javascript:")
        {
            resolve(parent_url, raw_src)
        } else {
            // Unmount any previously mounted sub-document when `src`/`srcdoc` is cleared.
            if matches!(el.special_data, zipp_dom::node::SpecialElementData::SubDocument(_))
                && let Some(el_mut) = doc.get_node_mut(iframe_id).and_then(|n| n.element_data_mut())
            {
                el_mut.special_data = zipp_dom::node::SpecialElementData::None;
                mounted_any = true;
            }
            continue;
        };

        // Guard against recursive self-embedding loops.
        if target_url == parent_url {
            continue;
        }

        // Skip if already mounted for this exact target URL.
        if let zipp_dom::node::SpecialElementData::SubDocument(ref existing) = el.special_data
            && existing.inner().base_url() == target_url
        {
            continue;
        }

        // Skip previously failed iframe fetches on this page so we don't spin.
        if FAILED_IFRAMES
            .lock()
            .is_ok_and(|set| set.contains(&target_url))
        {
            continue;
        }

        // Estimate the iframe's CSS viewport size from its layout box (or sensible default).
        let (_, _, bw, bh) = node.transformed_border_rect();
        let iframe_w = if bw >= 64.0 { bw.round() as u32 } else { 800 };
        let iframe_h = if bh >= 64.0 { bh.round() as u32 } else { 560 };

        let html = match raw_srcdoc {
            Some(sd) => sd,
            None => match fetch_iframe_document(&target_url, parent_url, no_referrer) {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("[IFRAME] failed to load {target_url}: {e}");
                    console::push(console::Level::Warn, format!("iframe {target_url}: {e}"));
                    if let Ok(mut set) = FAILED_IFRAMES.lock() {
                        set.insert(target_url);
                    }
                    continue;
                }
            },
        };

        let sub_doc = build_iframe_document(
            doc,
            &target_url,
            &html,
            (iframe_w, iframe_h),
            depth,
            allow_scripts,
            allow_same_origin,
        );
        doc.set_sub_document(iframe_id, Box::new(sub_doc));
        mounted_any = true;
    }

    if mounted_any {
        doc.shell_provider.request_redraw();
    }
    mounted_any
}

/// Show the WebGL, WebGPU, and Canvas 2D frames presented for `scope` since
/// the last call: each becomes the image of its canvas element (found by
/// `data-flid`), which is painted, and sized when the canvas has no size of
/// its own, like any image. True when there were any.
pub(crate) fn apply_canvas_frames_for(doc: &mut BaseDocument, scope: u64) -> bool {
    show_canvas_frames(doc, canvas::take_frames_for(scope), None)
}

/// Faster variant using the event bridge's cached `flid -> node` map (O(1) per frame),
/// scoped to `scope` so parent and iframe canvases never collide.
pub(crate) fn apply_canvas_frames_with_map(doc: &mut BaseDocument, scope: u64, flids: &HashMap<u32, usize>) -> bool {
    show_canvas_frames(doc, canvas::take_frames_for(scope), Some(flids))
}

/// After a re-render rebuilt the body: each canvas in `scope` shows its frame again.
pub(crate) fn reapply_canvas_frames(doc: &mut BaseDocument, scope: u64) {
    show_canvas_frames(doc, canvas::shown_frames_for(scope), None);
}

fn show_canvas_frames(
    doc: &mut BaseDocument,
    frames: Vec<(u32, canvas::Shown)>,
    cached_flids: Option<&HashMap<u32, usize>>,
) -> bool {
    use zipp_dom::node::{ImageData, RasterImageData, SpecialElementData};
    if frames.is_empty() {
        return false;
    }
    let owned_flids;
    let flids = match cached_flids {
        Some(map) => map,
        None => {
            owned_flids = event_bridge::build_flid_map(doc);
            &owned_flids
        }
    };
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
pub(crate) fn console_input() -> String {
    format!(r#"<input type="hidden" name="__zipp_chrome" value="{}" />{CONSOLE_INPUT}"#, chrome_token())
}

const CONSOLE_INPUT: &str = r#"<span style="color:#60a5fa;font-weight:700">›</span><input id="__zipp_console_input" type="text" name="__js" placeholder="Run JavaScript in the page, then Enter" style="flex:1;min-width:0;height:24px;border:0;outline:none;font-family:Consolas,'Liberation Mono',monospace;font-size:12px;background:transparent;color:#e2e8f0" />"#;

/// The console button's icon, and its badge counting the page's errors.
pub(crate) fn console_button_inner(badge: &str) -> String {
    let display = if badge.is_empty() { "none" } else { "inline-block" };
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m4 17 6-6-6-6"/><path d="M12 19h8"/></svg><span id="__zipp_console_badge" style="display:{display};min-width:16px;padding:0 5px;border-radius:999px;background:#ef4444;color:white;font-size:10px;font-weight:700;line-height:16px;text-align:center">{badge}</span>"#
    )
}

/// Inner markup for the DevTools panel's Copy button (`Copy` / `Copied!`).
pub(crate) fn console_copy_button_inner(copied: bool) -> String {
    devtools::copy_button_inner(copied)
}

/// Render the inner HTML of `#__zipp_tabbar` (all open tabs + the `+` New Tab button).
/// Called both during initial chrome injection and in-place by `JsEventBridge`
/// whenever background tabs open/close or `document.title` changes.
pub(crate) fn tabbar_inner_html(fallback_url: &str) -> String {
    let token = chrome_token();
    let incognito = zipp_shell::profile::is_incognito();
    let items = tabs::chrome_items(fallback_url);

    let (
        active_bg,
        active_fg,
        active_border,
        active_bottom,
        active_top_accent,
        inactive_fg,
        inactive_divider,
        close_color,
        new_btn_color,
        mono_bg_active,
        mono_fg_active,
        mono_bg_inactive,
        mono_fg_inactive,
    ) = if incognito {
        (
            "#1b162e",
            "#f8fafc",
            "#35285c",
            "#1b162e",
            "2px solid #8b5cf6",
            "#a799cc",
            "rgba(167, 139, 250, 0.18)",
            "#9b8ec4",
            "#c4b5fd",
            "rgba(139, 92, 246, 0.28)",
            "#ddd6fe",
            "rgba(139, 92, 246, 0.14)",
            "#a78bfa",
        )
    } else {
        (
            "#ffffff",
            "#0f172a",
            "#cbd5e1",
            "#ffffff",
            "2px solid #7c3aed",
            "#475569",
            "rgba(100, 116, 139, 0.24)",
            "#64748b",
            "#475569",
            "#ede9fe",
            "#6d28d9",
            "rgba(15, 23, 42, 0.08)",
            "#475569",
        )
    };

    let mut html = String::with_capacity(items.len() * 640 + 512);
    for item in &items {
        let title_esc = escape_html_attr(&item.title);
        let url_esc = escape_html_attr(&item.url);
        let icon_bg = if item.active { mono_bg_active } else { mono_bg_inactive };
        let icon_fg = if item.active { mono_fg_active } else { mono_fg_inactive };

        let tab_icon = if item.loading {
            format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="{icon_fg}" stroke-width="2.4" stroke-linecap="round" style="flex-shrink:0"><path d="M21 12a9 9 0 1 1-6.219-8.56"/></svg>"#
            )
        } else if item.url.starts_with("zipp:newtab") || item.url.starts_with("about:newtab") {
            format!(
                r#"<span style="width:16px;height:16px;border-radius:5px;background:{icon_bg};color:{icon_fg};display:inline-flex;align-items:center;justify-content:center;flex-shrink:0"><svg xmlns="http://www.w3.org/2000/svg" width="11" height="11" viewBox="0 0 256 256"><path d="M92 0 0 122h58l-25 86L142 69h-62z" transform="translate(57 24)" fill="currentColor"/></svg></span>"#
            )
        } else if item.url.starts_with("zipp:settings") || item.url.starts_with("zipp:properties") {
            format!(
                r#"<span style="width:16px;height:16px;border-radius:5px;background:{icon_bg};color:{icon_fg};display:inline-flex;align-items:center;justify-content:center;flex-shrink:0"><svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M12 1v4"/><path d="M12 19v4"/><path d="M4.22 4.22l2.83 2.83"/><path d="M16.95 16.95l2.83 2.83"/><path d="M1 12h4"/><path d="M19 12h4"/></svg></span>"#
            )
        } else if item.url.starts_with("zipp:history") {
            format!(
                r#"<span style="width:16px;height:16px;border-radius:5px;background:{icon_bg};color:{icon_fg};display:inline-flex;align-items:center;justify-content:center;flex-shrink:0"><svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.3" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><polyline points="12 7 12 12 15 14"/></svg></span>"#
            )
        } else {
            let letter = url::Url::parse(&item.url)
                .ok()
                .and_then(|u| u.host_str().map(|h| h.strip_prefix("www.").unwrap_or(h).to_string()))
                .and_then(|h| h.chars().find(|c| c.is_ascii_alphanumeric()))
                .or_else(|| item.title.chars().find(|c| c.is_ascii_alphanumeric()))
                .unwrap_or('W')
                .to_ascii_uppercase();
            format!(
                r#"<span style="width:16px;height:16px;border-radius:5px;background:{icon_bg};color:{icon_fg};display:inline-flex;align-items:center;justify-content:center;font-size:10px;font-weight:700;flex-shrink:0">{letter}</span>"#
            )
        };

        let (cls, box_style, weight) = if item.active {
            (
                "__zt_tab __zt_active",
                format!(
                    "flex:1 1 170px;max-width:228px;min-width:74px;height:31px;margin-bottom:-1px;display:flex;align-items:center;border-radius:9px 9px 0 0;background:{active_bg};color:{active_fg};border-left:1px solid {active_border};border-right:1px solid {active_border};border-top:{active_top_accent};border-bottom:1px solid {active_bottom};box-sizing:border-box;overflow:hidden"
                ),
                "600",
            )
        } else {
            (
                "__zt_tab __zt_inactive",
                format!(
                    "flex:1 1 160px;max-width:216px;min-width:68px;height:28px;margin-bottom:2px;display:flex;align-items:center;border-radius:8px;background:transparent;color:{inactive_fg};border-right:1px solid {inactive_divider};box-sizing:border-box;overflow:hidden"
                ),
                "500",
            )
        };

        html.push_str(&format!(
            r#"<div class="{cls}" data-tab-id="{id}" style="{box_style}"><a class="__zt_switch" href="zipp-nav:switch-tab?id={id}&amp;__zipp_chrome={token}" title="{url_esc}" style="flex:1;min-width:0;height:100%;display:flex;align-items:center;gap:7px;padding:0 4px 0 9px;text-decoration:none;color:inherit;overflow:hidden">{tab_icon}<span style="flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:12px;font-weight:{weight}">{title_esc}</span></a><a class="__zt_close" href="zipp-nav:close-tab?id={id}&amp;__zipp_chrome={token}" title="Close Tab (Ctrl+W)" style="width:18px;height:18px;margin-right:5px;border-radius:999px;display:flex;align-items:center;justify-content:center;text-decoration:none;color:{close_color};font-size:14px;line-height:1;flex-shrink:0">&times;</a></div>"#,
            id = item.id,
        ));
    }

    html.push_str(&format!(
        r#"<a id="__zipp_new_tab_btn" href="zipp-nav:new-tab?__zipp_chrome={token}" title="New Tab (Ctrl+T)" style="width:28px;height:28px;margin:0 0 2px 2px;border-radius:8px;display:flex;align-items:center;justify-content:center;text-decoration:none;color:{new_btn_color};flex-shrink:0"><svg xmlns="http://www.w3.org/2000/svg" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg></a>"#
    ));
    html
}

fn toolbar_markup(current_url: &str) -> String {
    let css = chrome_css(console::is_open(), is_mobile_view());
    let token = chrome_token();
    let console_line = console_input();
    let button = console_button_inner(&console::badge_text());
    let dt_header = devtools::header_html(token, false);
    let lines = devtools::body_html(None);
    let tabs_html = tabbar_inner_html(current_url);
    let (can_back, can_forward) = tabs::active_nav_state();
    let incognito = zipp_shell::profile::is_incognito();
    let on_settings = current_url == "zipp:settings" || current_url == "zipp:properties";
    let on_history = current_url == "zipp:history";
    let (
        strip_bg,
        strip_border,
        bar_bg,
        bar_border,
        icon_color,
        divider_color,
        omni_bg,
        omni_border,
        input_fg,
        active_pill,
        incog_pill,
    ) = if incognito {
        (
            "linear-gradient(180deg, #100c1e 0%, #0d0a18 100%)",
            "#2d234a",
            "linear-gradient(180deg, #1b162e 0%, #151026 100%)",
            "#2d234a",
            "#c4b5fd",
            "#2f254f",
            "#241d3d",
            "#3e3168",
            "#f8fafc",
            "rgba(167, 139, 250, 0.24)",
            r#"<span id="__zipp_incognito_badge" style="display:inline-flex;align-items:center;gap:5px;background:linear-gradient(180deg,#8b5cf6 0%,#6d28d9 100%);color:#ffffff;font-size:11px;font-weight:700;padding:3px 9px;border-radius:999px;white-space:nowrap;letter-spacing:0.01em"><svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7Z"/><circle cx="12" cy="12" r="3"/><path d="m3 3 18 18"/></svg>Incognito</span>"#,
        )
    } else {
        (
            "linear-gradient(180deg, #e2e8f0 0%, #dbe3ee 100%)",
            "#cbd5e1",
            "linear-gradient(180deg, #ffffff 0%, #f4f6fa 100%)",
            "#dde3ec",
            "#475569",
            "#e2e8f0",
            "#eef2f7",
            "#d5dde8",
            "#0f172a",
            "#ede9fe",
            "",
        )
    };
    let proto_icon = if current_url.starts_with("https://") {
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="#10b981" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" style="flex-shrink:0"><rect width="18" height="11" x="3" y="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/></svg>"##
    } else if current_url.starts_with("zipp:") || current_url.starts_with("about:") {
        if incognito {
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 256 256" style="flex-shrink:0"><path d="M92 0 0 122h58l-25 86L142 69h-62z" transform="translate(57 24)" fill="#c4b5fd"/></svg>"##
        } else {
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 256 256" style="flex-shrink:0"><path d="M92 0 0 122h58l-25 86L142 69h-62z" transform="translate(57 24)" fill="#7c3aed"/></svg>"##
        }
    } else {
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="#64748b" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="flex-shrink:0"><circle cx="12" cy="12" r="10"/><path d="M2 12h20"/><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/></svg>"##
    };
    let settings_bg = if on_settings { active_pill } else { "transparent" };
    let history_bg = if on_history { active_pill } else { "transparent" };
    let back_extra = if can_back { "opacity:1" } else { "opacity:0.38;cursor:default" };
    let fwd_extra = if can_forward { "opacity:1" } else { "opacity:0.38;cursor:default" };
    let is_newtab = current_url == tabs::NEW_TAB_URL
        || current_url.eq_ignore_ascii_case("about:newtab")
        || current_url.eq_ignore_ascii_case("about:blank");
    let form_action = if current_url.starts_with("zipp:") || current_url.starts_with("about:") {
        tabs::NEW_TAB_URL
    } else {
        current_url
    };
    let form_action_esc = escape_html_attr(form_action);
    // Keep the omnibox empty on New Tab so the user can type a URL immediately
    // without clearing "zipp:newtab" first.
    let omnibox_value_esc = if is_newtab {
        String::new()
    } else {
        escape_html_attr(current_url)
    };
    let btn_style = format!(
        "display:flex;align-items:center;justify-content:center;width:30px;height:30px;border-radius:8px;text-decoration:none;color:{icon_color};flex-shrink:0"
    );
    format!(
        r#"<div id="__zipp_chrome" style="position:absolute;left:0;right:0;z-index:2147483647;height:{TOOLBAR_HEIGHT}px;display:flex;flex-direction:column;font-family:'Segoe UI',Arial,sans-serif;font-size:13px;line-height:normal;text-align:left;box-sizing:border-box;margin:0">
    <style id="__zipp_chrome_css">{css}</style>
    <div id="__zipp_tabbar" style="height:{TAB_BAR_HEIGHT}px;background:{strip_bg};border-bottom:1px solid {strip_border};display:flex;align-items:flex-end;gap:4px;padding:4px 8px 0 8px;box-sizing:border-box;overflow:hidden;flex-shrink:0">{tabs_html}</div>
    <form id="__zipp_navbar" action="{form_action_esc}" method="GET" style="height:{NAV_BAR_HEIGHT}px;color:{input_fg};background:{bar_bg};border-bottom:1px solid {bar_border};display:flex;align-items:center;padding:0 10px;gap:4px;box-sizing:border-box;margin:0;flex-shrink:0">
      <input type="hidden" name="__zipp_chrome" value="{token}" />
      <a href="zipp-nav:back?__zipp_chrome={token}" title="Back (Alt+Left)" style="{btn_style};{back_extra}"><svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"><path d="m15 18-6-6 6-6"/></svg></a>
      <a href="zipp-nav:forward?__zipp_chrome={token}" title="Forward (Alt+Right)" style="{btn_style};{fwd_extra}"><svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg></a>
      <a href="zipp-nav:reload?__zipp_chrome={token}" title="Reload (F5)" style="{btn_style}"><svg xmlns="http://www.w3.org/2000/svg" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/></svg></a>
      <a id="__zipp_home_button" href="zipp-nav:home?__zipp_chrome={token}" title="Home / Start Page (Alt+Home)" style="{btn_style}"><svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m3 9 9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/><polyline points="9 22 9 12 15 12 15 22"/></svg></a>
      <span style="width:1px;height:18px;background:{divider_color};margin:0 3px;flex-shrink:0"></span>
      {incog_pill}
      <div style="flex:1;min-width:0;height:31px;display:flex;align-items:center;gap:8px;padding:0 12px;background:{omni_bg};border:1px solid {omni_border};border-radius:10px;box-sizing:border-box;cursor:text">
        {proto_icon}
        <input id="__zipp_url_input" type="text" name="__nav_url" value="{omnibox_value_esc}" placeholder="Search or enter URL..." style="flex:1;min-width:0;height:27px;padding:0;border:0;font-family:'Segoe UI',Arial,sans-serif;font-size:13px;font-weight:normal;font-style:normal;letter-spacing:normal;background:transparent;color:{input_fg};outline:none;box-sizing:border-box" />
      </div>
      <span style="width:1px;height:18px;background:{divider_color};margin:0 3px;flex-shrink:0"></span>
      <a id="__zipp_incognito_button" href="zipp-nav:incognito?__zipp_chrome={token}" title="New Incognito Window (Ctrl+Shift+N)" style="{btn_style}"><svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7Z"/><circle cx="12" cy="12" r="3"/><path d="m3 3 18 18"/></svg></a>
      <a id="__zipp_history_button" href="zipp-nav:history?__zipp_chrome={token}" title="Browsing History (Ctrl+H)" style="{btn_style};background:{history_bg}"><svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/></svg></a>
      <a id="__zipp_cache_button" href="zipp-nav:clear-cache?__zipp_chrome={token}" title="Clear Cache &amp; Reload (Ctrl+Shift+Delete)" style="{btn_style}"><svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18"/><path d="M8 6V4h8v2"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/><path d="M10 11v6"/><path d="M14 11v6"/></svg></a>
      <a id="__zipp_settings_button" href="zipp-nav:settings?__zipp_chrome={token}" title="Properties, Cache &amp; History (Ctrl+,)" style="{btn_style};background:{settings_bg}"><svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9c.26.604.852.997 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg></a>
      <a id="__zipp_mobile_button" href="zipp-view:mobile" title="Toggle Mobile View (Ctrl+Shift+M)" style="{btn_style}"><svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="14" height="20" x="5" y="2" rx="2" ry="2"/><path d="M12 18h.01"/></svg></a>
      <a id="__zipp_inspect_button" href="zipp-devtools:inspect" title="Inspect Page, Source &amp; Scripts (Ctrl+Shift+I)" style="{btn_style}"><svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="16 18 22 12 16 6"/><polyline points="8 6 2 12 8 18"/></svg></a>
      <a id="__zipp_console_button" href="zipp-console:toggle" title="Console (F12)" style="display:flex;align-items:center;justify-content:center;gap:4px;min-width:30px;height:30px;padding:0 6px;border-radius:8px;text-decoration:none;color:{icon_color};flex-shrink:0">{button}</a>
    </form>
  </div>
  <div id="__zipp_console" style="position:absolute;left:0;right:0;z-index:2147483646;flex-direction:column;background:#0f172a;border-bottom:1px solid #1e293b;font-family:Consolas,'Liberation Mono',monospace;font-size:12px;line-height:17px;color:#e2e8f0;text-align:left;box-sizing:border-box;margin:0">
    <div id="__zipp_dt_header" style="display:flex;align-items:center;gap:8px;padding:5px 12px;background:#131b2c;border-bottom:1px solid #1f2a40;font-family:'Segoe UI',Arial,sans-serif;font-size:12px;flex-shrink:0;overflow:hidden">{dt_header}</div>
    <div id="__zipp_console_lines" style="flex:1;overflow:auto;min-height:0;padding:0">{lines}</div>
    <form id="__zipp_console_form" action="{form_action_esc}" method="GET" style="display:flex;align-items:center;gap:8px;margin:0;padding:4px 12px;background:#0a0e17;border-top:1px solid #1f2a40;flex-shrink:0">{console_line}</form>
  </div>"#
    )
}

fn escape_html_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
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
    if !html.contains("<noscript") {
        return html.to_string();
    }
    let mut out = String::with_capacity(html.len());
    let mut pos = 0;
    while let Some(rel_start) = html[pos..].find("<noscript") {
        let start = pos + rel_start;
        out.push_str(&html[pos..start]);
        if let Some(rel_end) = html[start..].find("</noscript>") {
            pos = start + rel_end + 11;
        } else {
            out.push_str(&html[start..]);
            return out;
        }
    }
    out.push_str(&html[pos..]);
    out
}

fn inline_images(html: &str, base_url: &str) -> String {
    if !html.contains("src=\"") {
        return html.to_string();
    }
    let mut out = String::with_capacity(html.len() + 256);
    let mut copied = 0;
    let mut search_pos = 0;

    while let Some(idx) = html[search_pos..].find("src=\"") {
        let abs_idx = search_pos + idx + 5;
        let Some(end) = html[abs_idx..].find('"') else {
            break;
        };
        let src = &html[abs_idx..abs_idx + end];
        if !src.is_empty() && !src.starts_with("data:") && !src.starts_with("http") && !src.starts_with("blob:") {
            let full_url = resolve(base_url, src);
            out.push_str(&html[copied..abs_idx]);
            out.push_str(&full_url);
            copied = abs_idx + end;
        }
        search_pos = abs_idx + end + 1;
    }
    out.push_str(&html[copied..]);
    out
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
pub(crate) fn make_urls_absolute(html: &str, page_url: &str) -> String {
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
        if (name.eq_ignore_ascii_case("src") || name.eq_ignore_ascii_case("href"))
            && let Some(absolute) = absolutize(&base, &html[value_start..value_end]) {
                out.push_str(&html[copied..value_start]);
                out.push_str(&absolute);
                copied = value_end;
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
    let lower = s.to_ascii_lowercase();
    if matches!(lower.as_str(), "zipp:newtab" | "about:newtab" | "about:blank") {
        return tabs::NEW_TAB_URL.to_string();
    }
    if matches!(
        lower.as_str(),
        "zipp:settings" | "zipp:properties" | "about:settings" | "about:properties"
    ) {
        return "zipp:settings".to_string();
    }
    if matches!(lower.as_str(), "zipp:history" | "about:history") {
        return "zipp:history".to_string();
    }
    if s.starts_with("http://") || s.starts_with("https://") || s.starts_with("file://") || s.starts_with("zipp:") {
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
    let encoded = urlencoding::encode(s);
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
        assert!(out.contains(r#"<body class="home" style="padding:20px"><p>Hi</p><div id="__zipp_chrome""#));
        assert!(out.contains(r#"<div id="__zipp_tabbar""#));
        assert!(out.contains(r#"<form id="__zipp_navbar""#));
        // The console panel follows the toolbar, closed.
        assert!(out.contains(r#"<div id="__zipp_console""#));
        assert!(out.ends_with("</div></body></html>"));

        // A page with no doctype still gets standards mode, and one with no
        // closing body tag still gets its toolbar.
        let out = wrap_with_toolbar("<p>bare", "https://example.com/");
        assert!(out.starts_with("<!DOCTYPE html><p>bare<div id=\"__zipp_chrome\""));
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
        assert!(looks_like_page("https://example.com"));
        assert!(looks_like_page("https://example.com/about/"));
        assert!(looks_like_page("https://example.com/post.html"));
        assert!(looks_like_page("https://example.com/v1.2/notes"));
        assert!(!looks_like_page("https://example.com/assets/index-Car7W7dH.css"));
        assert!(!looks_like_page("https://example.com/favicon.svg"));
        assert!(!looks_like_page("https://example.com/downloads/guide.pdf"));
        assert!(!looks_like_page("not a url"));
    }

    #[test]
    fn cli_parses_mobile_viewport_and_app_modes() {
        let s = |xs: &[&str]| xs.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(parse_cli_args(&s(&["--help"])), Ok(CliAction::Help));
        assert_eq!(parse_cli_args(&s(&["-V"])), Ok(CliAction::Version));
        assert_eq!(parse_cli_args(&s(&["--clear-cache"])), Ok(CliAction::ClearCacheOnly));
        assert_eq!(parse_cli_args(&s(&["--clear-data"])), Ok(CliAction::ClearDataOnly));
        assert_eq!(
            parse_cli_args(&s(&["--incognito", "https://example.com"])),
            Ok(CliAction::Run {
                url: "https://example.com".into(),
                mobile: false,
                incognito: true,
                viewport: None,
                kiosk: false,
                app: false,
                app_files: vec![],
                clear_cache: false,
                clear_data: false,
                no_cache: false,
            })
        );
        assert_eq!(
            parse_cli_args(&s(&["--mobile", "--no-cache", "https://example.com"])),
            Ok(CliAction::Run {
                url: "https://example.com".into(),
                mobile: true,
                incognito: false,
                viewport: None,
                kiosk: false,
                app: false,
                app_files: vec![],
                clear_cache: false,
                clear_data: false,
                no_cache: true,
            })
        );
        assert_eq!(
            parse_cli_args(&s(&["--mobile", "--app", "index.html", "app.softn"])),
            Ok(CliAction::Run {
                url: "index.html".into(),
                mobile: true,
                incognito: false,
                viewport: None,
                kiosk: false,
                app: true,
                app_files: vec!["app.softn".into()],
                clear_cache: false,
                clear_data: false,
                no_cache: false,
            })
        );
        assert_eq!(
            parse_cli_args(&s(&["--viewport", "390x844", "--clear-cache", "https://example.com"])),
            Ok(CliAction::Run {
                url: "https://example.com".into(),
                mobile: false,
                incognito: false,
                viewport: Some((390, 844)),
                kiosk: false,
                app: false,
                app_files: vec![],
                clear_cache: true,
                clear_data: false,
                no_cache: false,
            })
        );
        assert_eq!(
            parse_cli_args(&s(&["--mobile", "--incognito", "--screenshot", "out.png", "https://example.com"])),
            Ok(CliAction::Screenshot {
                out: "out.png".into(),
                url: "https://example.com".into(),
                width: MOBILE_WINDOW_SIZE.0,
                height: MOBILE_WINDOW_SIZE.1,
                mobile: true,
                incognito: true,
                no_chrome: false,
                clear_cache: false,
                clear_data: false,
                no_cache: false,
            })
        );
        assert_eq!(
            parse_cli_args(&s(&["--mobile", "--app", "--screenshot", "out.png", "index.html"])),
            Ok(CliAction::Screenshot {
                out: "out.png".into(),
                url: "index.html".into(),
                width: MOBILE_WINDOW_SIZE.0,
                height: MOBILE_WINDOW_SIZE.1,
                mobile: true,
                incognito: false,
                no_chrome: true,
                clear_cache: false,
                clear_data: false,
                no_cache: false,
            })
        );
        assert_eq!(
            parse_cli_args(&s(&["--kiosk", "--app", "file:///app/softn/index.html", "/app/softn/examples/Fieldnotes.softn"])),
            Ok(CliAction::Run {
                url: "file:///app/softn/index.html".into(),
                mobile: false,
                incognito: false,
                viewport: None,
                kiosk: true,
                app: true,
                app_files: vec!["/app/softn/examples/Fieldnotes.softn".into()],
                clear_cache: false,
                clear_data: false,
                no_cache: false,
            })
        );
        assert!(parse_cli_args(&s(&["--viewport", "bad"])).is_err());
    }

    #[test]
    fn toolbar_includes_mobile_view_and_clear_cache_controls() {
        tabs::init("https://example.com/");
        let out = wrap_with_toolbar("<p>hi</p>", "https://example.com/");
        assert!(out.contains(r#"id="__zipp_tabbar""#));
        assert!(out.contains(r#"id="__zipp_new_tab_btn""#));
        assert!(out.contains(r#"href="zipp-nav:new-tab?__zipp_chrome="#));
        assert!(out.contains(r#"href="zipp-nav:switch-tab?id="#));
        assert!(out.contains(r#"href="zipp-nav:close-tab?id="#));
        assert!(out.contains(r#"id="__zipp_home_button""#));
        assert!(out.contains(r#"id="__zipp_incognito_button""#));
        assert!(out.contains(r#"id="__zipp_settings_button""#));
        assert!(out.contains(r#"id="__zipp_mobile_button""#));
        assert!(out.contains(r#"href="zipp-view:mobile""#));
        assert!(out.contains(r#"id="__zipp_cache_button""#));
        assert!(out.contains(r#"href="zipp-nav:clear-cache?__zipp_chrome="#));
        assert!(out.contains(r#"href="zipp-nav:clear-data?__zipp_chrome="#));
        assert!(out.contains(r#"href="zipp-nav:settings?__zipp_chrome="#));
        assert!(out.contains(r#"id="__zipp_console_copy""#));
        assert!(out.contains(r#"href="zipp-console:copy""#));
        assert!(out.contains(r#"href="zipp-nav:incognito?__zipp_chrome="#));
        assert!(out.contains(r#"id="__zipp_url_input""#));
        assert!(out.contains(r#"value="https://example.com/""#));

        // New Tab keeps the address bar blank (no "zipp:newtab" prefilled) with a placeholder.
        let newtab_bar = toolbar_markup(tabs::NEW_TAB_URL);
        assert!(newtab_bar.contains(r#"id="__zipp_url_input" type="text" name="__nav_url" value="" placeholder="Search or enter URL...""#));
        assert!(!newtab_bar.contains(r#"value="zipp:newtab""#));
    }

    #[test]
    fn properties_page_and_internal_url_normalization() {
        assert_eq!(normalize_url("about:newtab"), "zipp:newtab");
        assert_eq!(normalize_url("about:blank"), "zipp:newtab");
        assert_eq!(normalize_url("about:settings"), "zipp:settings");
        assert_eq!(normalize_url("about:history"), "zipp:history");
        assert_eq!(normalize_url("zipp:properties"), "zipp:settings");
        let newtab = internal_page_html("zipp:newtab", None, None, None).expect("newtab page html");
        assert!(newtab.contains("ZIPP Browser"));
        assert!(newtab.contains("Part of the ZIPP Project"));
        assert!(newtab.contains("Quick Access"));
        assert!(newtab.contains(r#"name="__nav_url""#));
        assert!(newtab.contains("Ctrl+T"));
        assert!(newtab.contains(r#"action="zipp:newtab""#));
        assert!(!newtab.contains(r#"action="https://lance.name""#));
        assert_eq!(settings::DEFAULT_START_PAGE, "zipp:newtab");
        let html = internal_page_html("zipp:settings", Some("Start page saved"), None, Some("https://softn.com"))
            .expect("properties page html");
        assert!(html.contains("Start Page"));
        assert!(html.contains("Cache &amp; Storage Management"));
        assert!(html.contains("Browsing History"));
        assert!(html.contains("New Incognito Window"));
        assert!(html.contains("Start page saved"));
        assert!(html.contains("Use Current Page"));
        assert!(!html.contains("Default (lance.name)"));
    }

    #[test]
    fn end_to_end_cookies_sessions_headers_and_http_cache() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let port = listener.local_addr().unwrap().port();
        let asset_hits = Arc::new(AtomicUsize::new(0));
        let asset_hits_srv = Arc::clone(&asset_hits);

        let handle = std::thread::spawn(move || {
            for _ in 0..16 {
                let Ok((mut stream, _)) = listener.accept() else { break };
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                if n == 0 {
                    continue;
                }
                let req = String::from_utf8_lossy(&buf[..n]);
                let first_line = req.lines().next().unwrap_or("");
                let mut parts = first_line.split_whitespace();
                let method = parts.next().unwrap_or("GET");
                let path = parts.next().unwrap_or("/");
                let cookie_hdr = req
                    .lines()
                    .find(|l| l.to_ascii_lowercase().starts_with("cookie:"))
                    .map(|l| l[7..].trim().to_string())
                    .unwrap_or_default();
                let auth_hdr = req
                    .lines()
                    .find(|l| l.to_ascii_lowercase().starts_with("x-token:"))
                    .map(|l| l[8..].trim().to_string())
                    .unwrap_or_default();

                let resp = match (method, path) {
                    ("POST", "/login") => {
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nSet-Cookie: sid=sess_42; Path=/; HttpOnly\r\nSet-Cookie: user=alice; Path=/; Max-Age=3600\r\nX-Session-State: active\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}".to_string()
                    }
                    ("GET", "/me") => {
                        let body = format!(
                            "{{\"cookie\":{},\"token\":{}}}",
                            json_quote(&cookie_hdr),
                            json_quote(&auth_hdr)
                        );
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                    }
                    ("GET", "/cached.js") => {
                        let hits = asset_hits_srv.fetch_add(1, Ordering::SeqCst) + 1;
                        let body = format!("// hit {hits}");
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nCache-Control: public, max-age=3600\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                    }
                    ("POST", "/logout") => {
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nSet-Cookie: sid=; Path=/; Max-Age=0; HttpOnly\r\nSet-Cookie: user=; Path=/; Max-Age=0\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}".to_string()
                    }
                    ("GET", "/binary.softn") => {
                        let bin: [u8; 8] = [0x50, 0x4b, 0x03, 0x04, 0x00, 0xff, 0x80, 0xfe];
                        let hdr = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nCache-Control: public, max-age=600\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            bin.len()
                        );
                        let _ = stream.write_all(hdr.as_bytes());
                        let _ = stream.write_all(&bin);
                        continue;
                    }
                    ("GET", "/done") => {
                        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
                        break;
                    }
                    _ => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string(),
                };
                let _ = stream.write_all(resp.as_bytes());
            }
        });

        let base = format!("http://127.0.0.1:{port}");
        let page = format!("{base}/app");
        set_thread_page(page.clone());
        DOC_URL.with(|d| *d.borrow_mut() = url::Url::parse(&page).ok());
        zipp_shell::cache::clear();

        // 1. Login sets HttpOnly session cookie + persistent user cookie + custom response header.
        let login_res = browser_host_call(
            "http",
            &[
                "POST".into(),
                format!("{base}/login"),
                "{}".into(),
                "application/json".into(),
                "same-origin".into(),
                "{}".into(),
            ],
        )
        .expect("login");
        let login_json: serde_json::Value = serde_json::from_str(&login_res).unwrap();
        assert_eq!(login_json["status"], 200);
        assert_eq!(login_json["headers"]["x-session-state"], "active");
        // Set-Cookie must never be exposed in Response.headers.
        assert!(login_json["headers"].get("set-cookie").is_none());

        // 2. Same-origin GET sends BOTH HttpOnly and non-HttpOnly cookies + custom request header.
        let me_res = browser_host_call(
            "http",
            &[
                "GET".into(),
                format!("{base}/me"),
                String::new(),
                String::new(),
                "same-origin".into(),
                r#"{"X-Token":"jwt-99"}"#.into(),
            ],
        )
        .expect("me");
        let me_json: serde_json::Value = serde_json::from_str(&me_res).unwrap();
        let me_body: serde_json::Value = serde_json::from_str(me_json["body"].as_str().unwrap()).unwrap();
        let sent_cookie = me_body["cookie"].as_str().unwrap();
        assert!(sent_cookie.contains("sid=sess_42"), "missing HttpOnly session cookie: {sent_cookie}");
        assert!(sent_cookie.contains("user=alice"), "missing user cookie: {sent_cookie}");
        assert_eq!(me_body["token"], "jwt-99");

        // 3. `credentials: "omit"` sends NO cookies even on same-origin.
        let omit_res = browser_host_call(
            "http",
            &[
                "GET".into(),
                format!("{base}/me"),
                String::new(),
                String::new(),
                "omit".into(),
                "{}".into(),
            ],
        )
        .expect("omit");
        let omit_json: serde_json::Value = serde_json::from_str(&omit_res).unwrap();
        let omit_body: serde_json::Value = serde_json::from_str(omit_json["body"].as_str().unwrap()).unwrap();
        assert_eq!(omit_body["cookie"], "");

        // 4. `document.cookie` hides HttpOnly `sid` and lets script add/read its own cookie.
        let doc_cookie = browser_host_call("cookie_get", std::slice::from_ref(&page)).unwrap();
        assert!(!doc_cookie.contains("sid="), "HttpOnly leaked to document.cookie: {doc_cookie}");
        assert!(doc_cookie.contains("user=alice"), "{doc_cookie}");
        let _ = browser_host_call("cookie_set", &[page.clone(), "ui=compact; Path=/".into()]);
        assert!(browser_host_call("cookie_get", std::slice::from_ref(&page)).unwrap().contains("ui=compact"));

        // 5. Persistent HTTP cache serves second fetch without hitting the server, and clear_browser_cache forces fresh fetch.
        let asset_url = format!("{base}/cached.js");
        let first = fetch(&asset_url).expect("first fetch");
        let second = fetch(&asset_url).expect("cached fetch");
        assert_eq!(first, "// hit 1");
        assert_eq!(second, "// hit 1");
        assert_eq!(asset_hits.load(Ordering::SeqCst), 1);

        clear_browser_cache();
        let third = fetch(&asset_url).expect("post-clear fetch");
        assert_eq!(third, "// hit 2");
        assert_eq!(asset_hits.load(Ordering::SeqCst), 2);

        // 5b. Caching a binary fetch via `http` must preserve exact raw bytes for `arrayBuffer()`/`fetch_bytes_with_mime`.
        let bin_url = format!("{base}/binary.softn");
        let _ = browser_host_call(
            "http",
            &[
                "GET".into(),
                bin_url.clone(),
                String::new(),
                String::new(),
                "same-origin".into(),
                "{}".into(),
            ],
        )
        .expect("binary http fetch");
        let (bstatus, _, bbytes) =
            fetch_bytes_with_mime(&bin_url, CredentialsMode::SameOrigin).expect("cached binary fetch");
        assert_eq!(bstatus, 200);
        assert_eq!(bbytes, vec![0x50, 0x4b, 0x03, 0x04, 0x00, 0xff, 0x80, 0xfe]);

        // 6. Per-origin sessionStorage survives across reads and clears on clear_sessions().
        let _ = browser_host_call("site_write", &["session".into(), "".into(), r#"{"step":"2"}"#.into()]);
        let sess = browser_host_call("site_read", &["session".into(), "".into()]).unwrap();
        assert_eq!(sess, r#"{"step":"2"}"#);
        site_storage::clear_sessions();
        assert_eq!(browser_host_call("site_read", &["session".into(), "".into()]).unwrap(), "");

        // 7. Logout clears session cookies via Max-Age=0.
        let _ = browser_host_call("http", &["POST".into(), format!("{base}/logout"), String::new(), String::new(), "same-origin".into(), "{}".into()]);
        let after_logout = browser_host_call("http", &["GET".into(), format!("{base}/me"), String::new(), String::new(), "same-origin".into(), "{}".into()]).unwrap();
        let after_json: serde_json::Value = serde_json::from_str(&after_logout).unwrap();
        let after_body: serde_json::Value = serde_json::from_str(after_json["body"].as_str().unwrap()).unwrap();
        assert!(!after_body["cookie"].as_str().unwrap().contains("sid=sess_42"));

        let _ = fetch(&format!("{base}/done"));
        let _ = handle.join();
    }
}
