//! Core SoftN Mobile Runtime Engine powered by `zipp-browser` (`zipp-dom` + `zipp-renderer`)
//! and `zipp-vm` (frontend JS/TS/React + full-stack Container v2 backend + native SQLite).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use anyrender::PaintScene;
use linebender_resource_handle::Blob;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use zipp_dom::html_parser::{HtmlDocument, HtmlProvider};
use zipp_dom::traits_src::net::{Bytes, NetHandler, NetProvider, Request};
use zipp_dom::traits_src::shell::{ColorScheme, Viewport};
use zipp_dom::{BaseDocument, Document, DocumentConfig, FontContext, LocalName};
use zipp_renderer::tinyskia::TinySkiaPainter;
use zipp_vm::embed::{HostCtx, JsValue, ScriptState};
use zipp_vm::frontend::{compile_source, Frontend};

use crate::backend::{BackendHttpRequest, LocalContainerBackend};
use crate::bundle::{resolve_vfs_url, SoftnBundle, VIRTUAL_ORIGIN};
use crate::http_cache::{CacheLookup, MobileCookieJar, MobileHttpCache};
use crate::{canvas, canvas2d, esm, html_seed, http_cache};
use crate::sql::{RequestSqlBridge, SqlStorage};

const ZIPP_CONSOLE_SHIM: &str =
    include_str!("../../../apps/browser/src/console_shim.js");
const ZIPP_CLONE_JS: &str =
    include_str!("../../../apps/browser/src/clone.js");
const ZIPP_DOM_SHIM: &str =
    include_str!("../../../apps/browser/src/dom_shim.js");
const ZIPP_IDB_SHIM: &str =
    include_str!("../../../apps/browser/src/indexeddb_shim.js");
const ZIPP_WEBGL_SHIM: &str =
    include_str!("../../../apps/browser/src/webgl_shim.js");
const ZIPP_CANVAS2D_SHIM: &str =
    include_str!("../../../apps/browser/src/canvas2d_shim.js");
const SOFTN_PRELUDE_JS: &str = include_str!("softn_prelude.js");

const HOST_PRELUDE_JS: &str = r#"
var __zippHostBytes = new Uint8Array(65536);
var __zippCredentials = "";
var __zippRequestHeaders = "";
function __zippFetchBytes(url, credentials) {
    var info = JSON.parse(__zippHostCall("http_bytes", String(url), credentials || ""));
    if (info && info.error && !info.status) {
        throw new TypeError(String(info.error));
    }
    var len = (info && info.len) || 0;
    if (__zippHostBytes.length < len) {
        __zippHostBytes = new Uint8Array(Math.max(len, __zippHostBytes.length * 2));
    }
    __zippHostCall("take_bytes");
    return __zippHostBytes.slice(0, len).buffer;
}
var env = {
    log: function (level, message) {
        __zippHostCall("log", String(level), String(message));
    }
};
var http = (function () {
    function request(method, url, body, contentType) {
        var credentials = __zippCredentials || "same-origin";
        var extraHeaders = __zippRequestHeaders;
        __zippCredentials = "";
        __zippRequestHeaders = "";
        var headers = {};
        if (extraHeaders) {
            try {
                var parsed = JSON.parse(extraHeaders);
                if (parsed && typeof parsed === "object") headers = parsed;
            } catch (e) {}
        }
        if (contentType && !headers["Content-Type"] && !headers["content-type"]) {
            headers["Content-Type"] = String(contentType);
        }
        var raw = __zippHostCall(
            "softn.fetch",
            method,
            String(url),
            JSON.stringify(headers),
            body == null ? "" : String(body),
            credentials
        );
        var r = JSON.parse(raw);
        if (r && r.error && !r.status) {
            throw new TypeError(String(r.error));
        }
        var hdrMap = {};
        if (Array.isArray(r.headers)) {
            for (var i = 0; i < r.headers.length; i++) {
                var p = r.headers[i];
                if (p && p.length >= 2) hdrMap[String(p[0]).toLowerCase()] = String(p[1]);
            }
        }
        var status = r.status || 200;
        return {
            status: status,
            ok: status >= 200 && status < 300,
            body: r.body || "",
            contentType: hdrMap["content-type"] || "",
            headers: hdrMap
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
var __zippDrainStats = { rounds: 0, timers: 0, micro: 0, raf: 0 };
function __zippDrainQueues(rounds) {
    var did = 0;
    for (var i = 0; i < rounds; i++) {
        if (typeof __zippDueTimers === "function") __zippDueTimers();
        if (__timerQueue.length === 0 && __mcQueue.length === 0) break;
        __zippDrainStats.rounds++;
        var tLimit = 64;
        while (__timerQueue.length > 0 && tLimit-- > 0) {
            __zippDrainStats.timers++;
            did++;
            try { __timerQueue.shift()(); }
            catch (e) { env.log("error", "timer callback threw: " + ((e && (e.stack || e.message)) || e)); }
        }
        var mLimit = 128;
        while (__mcQueue.length > 0 && mLimit-- > 0) {
            __zippDrainStats.micro++;
            did++;
            try { __mcQueue.shift()(); }
            catch (e) { env.log("error", "microtask threw: " + ((e && (e.stack || e.message)) || e)); }
        }
    }
    return did;
}
function __zippStepRaf() {
    var now = typeof performance !== "undefined" && performance.now ? performance.now() : Date.now();
    var ran = 0;
    if (typeof __zippRunAnimationFrame === "function" && typeof __rafQueue !== "undefined" && __rafQueue.length > 0) {
        ran = __zippRunAnimationFrame(now);
        __zippDrainStats.raf += ran;
    }
    if (typeof __zippPresentCanvases === "function") {
        __zippPresentCanvases();
    }
    return ran;
}
function __zippHasPendingRaf() {
    return (
        typeof __rafQueue !== "undefined" &&
        __rafQueue.length > 0 &&
        typeof __zippCanvasContexts !== "undefined" &&
        __zippCanvasContexts.length > 0
    );
}
function __getHtmlClassOnly() {
    try { return String((doc.documentElement && doc.documentElement.className) || ""); }
    catch (e) { return ""; }
}
function __getHtmlClass() {
    try {
        var el = doc.documentElement;
        if (!el) return "";
        var out = [];
        if (el.className) out.push('class="' + String(el.className).replace(/"/g, '&quot;') + '"');
        for (var k in el.attributes) {
            if (k === "class" || k === "style") continue;
            out.push(k + '="' + String(el.attributes[k]).replace(/"/g, '&quot;') + '"');
        }
        var sp = [];
        if (el.attributes && el.attributes.style) sp.push(String(el.attributes.style).replace(/;+$/, ""));
        if (el.style && typeof el.style === "object") {
            var sk = Object.keys(el.style);
            for (var i = 0; i < sk.length; i++) {
                var p = sk[i];
                if (p === "cssText" || p === "length") continue;
                var v = el.style[p];
                if (v === undefined || v === null || v === "" || typeof v === "function") continue;
                var cssName = p.indexOf("--") === 0 ? p : p.replace(/[A-Z]/g, function (m) { return "-" + m.toLowerCase(); });
                sp.push(cssName + ":" + String(v));
            }
        }
        if (sp.length > 0) out.push('style="' + sp.join(";").replace(/"/g, '&quot;') + '"');
        return out.join(" ");
    } catch (e) { return ""; }
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
        srcElement: target,
        currentTarget: currentTarget,
        eventPhase: 2,
        bubbles: true,
        cancelable: true,
        isTrusted: true,
        timeStamp: Date.now ? Date.now() : 0,
        detail: 1,
        button: 0,
        buttons: 1,
        clientX: 0, clientY: 0, pageX: 0, pageY: 0, screenX: 0, screenY: 0,
        altKey: false, ctrlKey: false, metaKey: false, shiftKey: false,
        get defaultPrevented() { return defaultPrevented; },
        preventDefault: function () { defaultPrevented = true; },
        stopPropagation: function () { propagationStopped = true; },
        stopImmediatePropagation: function () { propagationStopped = true; immediateStopped = true; },
        composedPath: function () {
            var path = [], n = target;
            while (n) { path.push(n); n = n.parentNode; }
            return path;
        }
    };
    ev.nativeEvent = ev;
    for (var i = 0; i < handlers.length; i++) {
        try { handlers[i](ev); }
        catch (e) { env.log("error", "handler for " + type + " threw: " + ((e && (e.stack || e.message)) || e)); }
        if (immediateStopped) break;
    }
    return defaultPrevented;
}
"#;

const SOFTN_UI_CSS: &str = r#"
:root {
  --softn-bg: #0f172a;
  --softn-surface: #1e293b;
  --softn-border: #334155;
  --softn-fg: #f8fafc;
  --softn-muted: #94a3b8;
  --softn-primary: #3b82f6;
  --softn-success: #10b981;
  --softn-warn: #f59e0b;
  --softn-danger: #ef4444;
  --softn-radius: 12px;
}
* { box-sizing: border-box; }
html, body {
  margin: 0;
  padding: 0;
  font-family: system-ui, -apple-system, sans-serif;
  background: var(--softn-bg);
  color: var(--softn-fg);
  font-size: 15px;
  line-height: 1.45;
}
.stack { display: flex; flex-direction: column; }
.row { display: flex; flex-direction: row; align-items: center; }
.between { justify-content: space-between; }
.wrap { flex-wrap: wrap; }
.gap-xs { gap: 6px; }
.gap-sm { gap: 10px; }
.gap-md { gap: 14px; }
.gap-lg { gap: 20px; }
.card {
  background: var(--softn-surface);
  border: 1px solid var(--softn-border);
  border-radius: var(--softn-radius);
  padding: 14px;
}
.btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  padding: 10px 14px;
  border-radius: 10px;
  border: 1px solid var(--softn-border);
  background: var(--softn-surface);
  color: var(--softn-fg);
  font-weight: 600;
  font-size: 14px;
}
.btn.primary { background: var(--softn-primary); border-color: var(--softn-primary); color: #ffffff; }
.btn.success { background: var(--softn-success); border-color: var(--softn-success); color: #ffffff; }
.btn.danger { background: var(--softn-danger); border-color: var(--softn-danger); color: #ffffff; }
.badge {
  display: inline-block;
  padding: 3px 9px;
  border-radius: 999px;
  font-size: 12px;
  font-weight: 600;
  background: rgba(59, 130, 246, 0.2);
  color: #93c5fd;
}
input, textarea, select {
  width: 100%;
  padding: 10px 12px;
  border-radius: 10px;
  border: 1px solid var(--softn-border);
  background: #0b1220;
  color: var(--softn-fg);
  font-size: 14px;
}
"#;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HostDeviceInfo {
    #[serde(default = "default_platform")]
    pub platform: String,
    #[serde(default = "default_engine")]
    pub engine: String,
    #[serde(default)]
    pub os_version: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub capabilities: HashMap<String, bool>,
}

fn default_platform() -> String {
    "android".to_string()
}
fn default_engine() -> String {
    "zipp-browser-android".to_string()
}

impl Default for HostDeviceInfo {
    fn default() -> Self {
        let mut capabilities = HashMap::new();
        capabilities.insert("camera".into(), true);
        capabilities.insert("bluetooth".into(), true);
        capabilities.insert("location".into(), true);
        capabilities.insert("haptics".into(), true);
        capabilities.insert("notifications".into(), true);
        Self {
            platform: default_platform(),
            engine: default_engine(),
            os_version: "16".into(),
            model: "Android".into(),
            capabilities,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingDeviceRequest {
    pub id: u64,
    pub namespace: String,
    pub method: String,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct SelectPickerRequest {
    pub flid: u32,
    pub selected_index: i32,
    pub options: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InteractionResult {
    pub dirty: bool,
    pub wants_raf: bool,
    pub ime_changed: bool,
    pub wants_ime: bool,
    pub input_text: String,
    pub input_type: String,
    pub pending_calls: Vec<PendingDeviceRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub select_picker: Option<SelectPickerRequest>,
}

/// Snapshot of laid-out CSS box metrics (`clientWidth`/`clientHeight`/`getBoundingClientRect`),
/// computed styles (`getComputedStyle`), and hit-test rects (`elementFromPoint`) exposed
/// synchronously to JS via `__zippHostCall`.
#[derive(Default)]
struct LayoutCache {
    boxes: HashMap<u32, [f32; 6]>,
    rects: HashMap<u32, [f32; 4]>,
    order: Vec<u32>,
    styles: HashMap<u32, HashMap<String, String>>,
    scroll_y: f64,
}

const CACHED_STYLE_PROPS: &[&str] = &[
    "display",
    "visibility",
    "opacity",
    "position",
    "overflow",
    "overflow-x",
    "overflow-y",
    "z-index",
    "transform",
    "width",
    "height",
    "background-color",
    "color",
    "font-size",
];

impl LayoutCache {
    fn update_from_doc(
        &mut self,
        doc: &BaseDocument,
        flid_to_dom: &HashMap<u32, usize>,
        scroll_y: f64,
    ) {
        self.boxes.clear();
        self.rects.clear();
        self.order.clear();
        self.styles.clear();
        self.scroll_y = scroll_y;
        let mut pairs: Vec<(usize, u32)> = flid_to_dom.iter().map(|(&f, &n)| (n, f)).collect();
        pairs.sort_unstable_by_key(|&(node_id, _)| node_id);
        for (node_id, flid) in pairs {
            let Some(node) = doc.get_node(node_id) else {
                continue;
            };
            let l = &node.final_layout;
            let client_w =
                (l.size.width - l.border.left - l.border.right - l.scrollbar_size.width).max(0.0);
            let client_h =
                (l.size.height - l.border.top - l.border.bottom - l.scrollbar_size.height).max(0.0);
            let content_w = (client_w - l.padding.left - l.padding.right).max(0.0);
            let content_h = (client_h - l.padding.top - l.padding.bottom).max(0.0);
            self.boxes.insert(
                flid,
                [l.size.width, l.size.height, client_w, client_h, content_w, content_h],
            );
            let (tx, ty, tw, th) = node.transformed_border_rect();
            self.rects.insert(flid, [tx as f32, ty as f32, tw as f32, th as f32]);
            self.order.push(flid);
            if node.primary_styles().is_some() {
                let mut smap = HashMap::new();
                for &prop in CACHED_STYLE_PROPS {
                    if let Some(val) = node.computed_style_value(prop) {
                        if !val.is_empty() {
                            smap.insert(prop.to_string(), val);
                        }
                    }
                }
                if !smap.is_empty() {
                    self.styles.insert(flid, smap);
                }
            }
        }
    }

    fn format_box(&self, flid_str: &str) -> String {
        let Ok(flid) = flid_str.parse::<u32>() else {
            return String::new();
        };
        let Some(b) = self.boxes.get(&flid) else {
            return String::new();
        };
        format!("[{},{},{},{},{},{}]", b[0], b[1], b[2], b[3], b[4], b[5])
    }

    fn format_rect(&self, flid_str: &str) -> String {
        let Ok(flid) = flid_str.parse::<u32>() else {
            return String::new();
        };
        let Some(r) = self.rects.get(&flid) else {
            return String::new();
        };
        format!("[{},{},{},{}]", r[0], f64::from(r[1]) - self.scroll_y, r[2], r[3])
    }

    fn format_style(&self, flid_str: &str, prop: &str) -> String {
        let Ok(flid) = flid_str.parse::<u32>() else {
            return String::new();
        };
        let key = prop.trim().to_ascii_lowercase();
        self.styles
            .get(&flid)
            .and_then(|m| m.get(&key))
            .cloned()
            .unwrap_or_default()
    }

    fn hit(&self, x_str: &str, y_str: &str) -> String {
        let (Ok(x), Ok(y)) = (x_str.parse::<f32>(), y_str.parse::<f32>()) else {
            return String::new();
        };
        let doc_y = y + self.scroll_y as f32;
        for &flid in self.order.iter().rev() {
            if let Some(&[rx, ry, rw, rh]) = self.rects.get(&flid) {
                if rw > 0.0 && rh > 0.0 && x >= rx && x <= rx + rw && doc_y >= ry && doc_y <= ry + rh {
                    return flid.to_string();
                }
            }
        }
        String::new()
    }
}

/// Zero-latency synchronous `NetProvider` serving `blob:` URIs, `data:` URIs
/// (such as captured camera photos), `.softn` archive assets, and cached/cookie-aware
/// `net`-permitted HTTPS resources straight into `zipp-dom`.
struct VfsNetProvider {
    bundle: Arc<SoftnBundle>,
    http_cache: Arc<MobileHttpCache>,
    cookies: Arc<MobileCookieJar>,
}

impl NetProvider for VfsNetProvider {
    fn fetch(&self, _doc_id: usize, request: Request, handler: Box<dyn NetHandler>) {
        let url_str = request.url.as_str();
        if url_str.starts_with("blob:") {
            if let Some(bytes) = http_cache::lookup_blob_bytes(url_str) {
                handler.bytes(url_str.to_string(), Bytes::from(bytes));
            }
            return;
        }
        if url_str.starts_with("data:") {
            if let Ok(parsed) = data_url::DataUrl::process(url_str) {
                if let Ok((body, _)) = parsed.decode_to_vec() {
                    handler.bytes(url_str.to_string(), Bytes::from(body));
                }
            }
            return;
        }
        if let Some(rel) = url_str.strip_prefix(VIRTUAL_ORIGIN) {
            if let Some((_, _, bytes)) = self.bundle.read_public_asset(rel) {
                handler.bytes(url_str.to_string(), Bytes::from(bytes));
            }
            return;
        }
        if self.bundle.check_net_url(url_str).is_ok() {
            let revalidate = match self.http_cache.lookup(url_str) {
                CacheLookup::Fresh(entry) => {
                    handler.bytes(url_str.to_string(), Bytes::from(entry.body.as_ref().clone()));
                    return;
                }
                CacheLookup::Revalidate(entry) => Some(entry),
                CacheLookup::Miss => None,
            };
            let mut req = ureq::get(url_str);
            if let Some(cookie) = self.cookies.cookie_header(&request.url) {
                req = req.header("Cookie", &cookie);
            }
            if let Some(ref prev) = revalidate {
                if let Some(ref etag) = prev.etag {
                    req = req.header("If-None-Match", etag);
                }
                if let Some(ref lm) = prev.last_modified {
                    req = req.header("If-Modified-Since", lm);
                }
            }
            if let Ok(mut resp) = req.call() {
                let status = resp.status().as_u16();
                let resp_hdrs = collect_ureq_headers(&resp);
                let set_cookies: Vec<String> = resp_hdrs
                    .iter()
                    .filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
                    .map(|(_, v)| v.clone())
                    .collect();
                if !set_cookies.is_empty() {
                    self.cookies.set_from_response(&request.url, &set_cookies);
                }
                if status == 304 {
                    if let Some(prev) = revalidate {
                        let refreshed = self.http_cache.revalidate_304(url_str, prev, &resp_hdrs);
                        handler.bytes(url_str.to_string(), Bytes::from(refreshed.body.as_ref().clone()));
                        return;
                    }
                }
                if let Ok(bytes) = resp.body_mut().read_to_vec() {
                    self.http_cache.store(url_str, status, &resp_hdrs, bytes.clone());
                    handler.bytes(url_str.to_string(), Bytes::from(bytes));
                }
            }
        }
    }
}

pub struct SoftnMobileEngine {
    pub bundle: Arc<SoftnBundle>,
    pub backend: Option<Arc<LocalContainerBackend>>,
    pub storage: SqlStorage,
    app_data_dir: PathBuf,
    http_cache: Arc<MobileHttpCache>,
    cookies: Arc<MobileCookieJar>,
    session_storage: Arc<Mutex<HashMap<String, String>>>,
    js_state: Option<ScriptState>,
    js_abort: Arc<AtomicBool>,
    doc: HtmlDocument,
    painter: TinySkiaPainter,
    head_css: String,
    width: u32,
    height: u32,
    scale: f32,
    scroll_y: f64,
    focused_flid: Option<u32>,
    active_canvas_flid: Option<u32>,
    last_pointer_phys: (f32, f32),
    flid_to_dom: HashMap<u32, usize>,
    dom_to_flid: HashMap<usize, u32>,
    layout_cache: Arc<Mutex<LayoutCache>>,
    pending_calls: Arc<Mutex<Vec<PendingDeviceRequest>>>,
    pub logs: Arc<Mutex<Vec<String>>>,
    last_synced_html: String,
    last_dom_mutations: i64,
    layout_dirty: bool,
    full_repaint: bool,
    dirty_canvas_nodes: Vec<usize>,
    dirty: bool,
}

impl SoftnMobileEngine {
    pub fn load_bundle(
        bundle_bytes: &[u8],
        data_root: &Path,
        width: u32,
        height: u32,
        scale: f32,
        host_info: HostDeviceInfo,
    ) -> Result<Self, String> {
        canvas::forget_shown();
        let bundle = Arc::new(SoftnBundle::from_zip_bytes(bundle_bytes)?);
        let app_data_dir = data_root.join(&bundle.app_id);
        let _ = std::fs::create_dir_all(&app_data_dir);
        let databases = bundle.manifest.databases.clone().unwrap_or_default();
        let storage = SqlStorage::new(&app_data_dir, databases)?;
        let backend = LocalContainerBackend::new(&bundle, storage.clone())?.map(Arc::new);
        let http_cache = Arc::new(MobileHttpCache::open(&app_data_dir));
        let cookies = Arc::new(MobileCookieJar::open(&app_data_dir));
        let session_storage: Arc<Mutex<HashMap<String, String>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let pending_calls: Arc<Mutex<Vec<PendingDeviceRequest>>> =
            Arc::new(Mutex::new(Vec::new()));
        let logs: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let layout_cache: Arc<Mutex<LayoutCache>> = Arc::new(Mutex::new(LayoutCache::default()));

        // Load initial HTML either from static VFS entry or from SSR backend (`GET /`)
        let raw_html = if !bundle.entry_path.is_empty() {
            bundle
                .read_text(&bundle.entry_path)
                .ok_or_else(|| format!("Entry file missing: {}", bundle.entry_path))?
        } else if let Some(ref be) = backend {
            let resp = be.handle_request(BackendHttpRequest {
                method: "GET".into(),
                path: "/".into(),
                query: String::new(),
                headers: vec![("Accept".into(), "text/html".into())],
                body: Vec::new(),
            });
            String::from_utf8_lossy(&resp.body).into_owned()
        } else {
            return Err("Bundle has neither index.html nor SSR backend".into());
        };

        let head_css = collect_head_css(&raw_html, &bundle);
        let initial_body = extract_body_inner_html(&raw_html);
        let initial_html_attrs = extract_html_open_attrs(&raw_html);
        let initial_html_attrs_str = initial_html_attrs
            .iter()
            .map(|(k, v)| format!("{k}=\"{}\"", v.replace('"', "&quot;")))
            .collect::<Vec<_>>()
            .join(" ");

        let logical_w = ((width.max(1) as f32) / scale.max(1.0)).round().max(320.0) as u32;
        let logical_h = ((height.max(1) as f32) / scale.max(1.0)).round().max(480.0) as u32;
        let js_abort = Arc::new(AtomicBool::new(false));

        // Boot frontend JavaScript/TypeScript inside `zipp-vm`
        let js_state = boot_frontend_vm(
            &raw_html,
            &initial_body,
            &initial_html_attrs,
            logical_w,
            logical_h,
            scale,
            &bundle,
            backend.as_ref(),
            &storage,
            &app_data_dir,
            Arc::clone(&http_cache),
            Arc::clone(&cookies),
            Arc::clone(&session_storage),
            host_info,
            Arc::clone(&layout_cache),
            Arc::clone(&pending_calls),
            Arc::clone(&logs),
            Arc::clone(&js_abort),
        )?;

        let mut engine = Self {
            bundle: Arc::clone(&bundle),
            backend,
            storage,
            app_data_dir,
            http_cache: Arc::clone(&http_cache),
            cookies: Arc::clone(&cookies),
            session_storage,
            js_state,
            js_abort,
            doc: build_html_document(
                &compose_document_html(&head_css, &initial_body, &initial_html_attrs_str),
                &bundle,
                &http_cache,
                &cookies,
                width,
                height,
                scale,
            ),
            painter: TinySkiaPainter::new(width.max(1), height.max(1)),
            head_css,
            width: width.max(1),
            height: height.max(1),
            scale: scale.max(1.0),
            scroll_y: 0.0,
            focused_flid: None,
            active_canvas_flid: None,
            last_pointer_phys: (
                (width.max(1) as f32) * 0.5,
                (height.max(1) as f32) * 0.5,
            ),
            flid_to_dom: HashMap::new(),
            dom_to_flid: HashMap::new(),
            layout_cache,
            pending_calls,
            logs,
            last_synced_html: String::new(),
            last_dom_mutations: -1,
            layout_dirty: true,
            full_repaint: true,
            dirty_canvas_nodes: Vec::new(),
            dirty: true,
        };

        // Sync rendered DOM after initial JS execution + populate real LayoutCache bounds
        engine.sync_dom_from_vm();
        // Run initial animation frame so Canvas 2D scenes (e.g. 3D mine maps & charts)
        // measure their real LayoutCache container dimensions and render Frame 1 immediately.
        if let Some(ref mut st) = engine.js_state {
            let _ = st.call_global("__zippStepRaf", &[]);
            drain_js(st);
            let _ = st.call_global("__zippEndSettle", &[]);
        }
        engine.sync_dom_from_vm();
        Ok(engine)
    }

    pub fn resize(&mut self, width: u32, height: u32, scale: f32) {
        let w = width.max(1);
        let h = height.max(1);
        let s = scale.max(1.0);
        if self.width == w && self.height == h && (self.scale - s).abs() < 0.001 {
            return;
        }
        self.width = w;
        self.height = h;
        self.scale = s;
        self.painter.resize(w, h);
        let viewport = Viewport::new(w, h, s, self.color_scheme());
        self.doc.as_mut().set_viewport(viewport);
        self.doc.as_mut().resolve(0.0);
        if let Ok(mut lc) = self.layout_cache.lock() {
            lc.update_from_doc(self.doc.as_ref(), &self.flid_to_dom, self.scroll_y);
        }
        self.layout_dirty = false;
        self.full_repaint = true;
        self.dirty = true;
    }

    fn color_scheme(&self) -> ColorScheme {
        match self.bundle.manifest.theme.as_deref() {
            Some("light") => ColorScheme::Light,
            _ => ColorScheme::Dark,
        }
    }

    /// Paint the laid-out DOM into RGBA8888 pixel bytes (`width * height * 4` bytes).
    pub fn render_rgba(&mut self, out_rgba: &mut [u8]) -> bool {
        let expected = (self.width as usize) * (self.height as usize) * 4;
        if out_rgba.len() < expected {
            #[cfg(target_os = "android")]
            log::error!(
                "render_rgba: out_rgba too small ({} < {}) for {}x{}",
                out_rgba.len(),
                expected,
                self.width,
                self.height
            );
            return false;
        }
        if self.layout_dirty {
            self.doc.as_mut().poll(None);
            self.doc.as_mut().resolve(0.0);
            self.layout_dirty = false;
            self.full_repaint = true;
        }
        if self.full_repaint {
            self.painter.reset();
            zipp_renderer::paint_scene(
                &mut self.painter,
                self.doc.as_ref(),
                self.scale as f64,
                self.width,
                self.height,
                0,
                0,
            );
            self.full_repaint = false;
            self.dirty_canvas_nodes.clear();
        } else if !self.dirty_canvas_nodes.is_empty() {
            match self.compute_canvas_damage_rect() {
                CanvasDamage::Full => {
                    self.painter.reset();
                    zipp_renderer::paint_scene(
                        &mut self.painter,
                        self.doc.as_ref(),
                        self.scale as f64,
                        self.width,
                        self.height,
                        0,
                        0,
                    );
                }
                CanvasDamage::OpaqueRect(x0, y0, x1, y1) => {
                    self.painter.reset_damage_preserve(x0, y0, x1, y1);
                    zipp_renderer::paint_scene_from_nodes(
                        &mut self.painter,
                        self.doc.as_ref(),
                        self.scale as f64,
                        self.width,
                        self.height,
                        0,
                        0,
                        &self.dirty_canvas_nodes,
                    );
                }
                CanvasDamage::Rect(x0, y0, x1, y1) => {
                    self.painter.reset_damage(x0, y0, x1, y1);
                    zipp_renderer::paint_scene(
                        &mut self.painter,
                        self.doc.as_ref(),
                        self.scale as f64,
                        self.width,
                        self.height,
                        0,
                        0,
                    );
                }
                CanvasDamage::Offscreen => {
                    // All updated canvases are currently scrolled out of the viewport;
                    // the cached framebuffer is already bit-for-bit up to date.
                }
            }
            self.dirty_canvas_nodes.clear();
        }
        let src = self.painter.rgba_data();
        if src.len() >= expected {
            out_rgba[..expected].copy_from_slice(&src[..expected]);
            self.dirty = false;
            true
        } else {
            #[cfg(target_os = "android")]
            log::error!(
                "render_rgba: painter rgba_data too small ({} < {}) for {}x{}",
                src.len(),
                expected,
                self.width,
                self.height
            );
            false
        }
    }

    fn compute_canvas_damage_rect(&self) -> CanvasDamage {
        let doc = self.doc.as_ref();
        let scale = self.scale as f64;
        let fw = self.width as f64;
        let fh = self.height as f64;
        let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);

        let mut all_opaque = true;
        for &node_id in &self.dirty_canvas_nodes {
            let Some(node) = doc.get_node(node_id) else {
                continue;
            };
            // Fall back to a full repaint if the canvas or any ancestor has a CSS transform,
            // opacity group, or filter blur so compositing stays 100% exact.
            let mut cur = Some(node_id);
            while let Some(cid) = cur {
                if let Some(n) = doc.get_node(cid) {
                    if let Some(st) = n.primary_styles() {
                        if !st.get_box().transform.0.is_empty()
                            || st.get_effects().opacity < 1.0
                            || !st.get_effects().filter.0.is_empty()
                        {
                            return CanvasDamage::Full;
                        }
                    }
                    cur = n.parent;
                } else {
                    break;
                }
            }
            if all_opaque {
                let is_opaque = node
                    .element_data()
                    .and_then(|e| e.raster_image_data())
                    .map(|img| {
                        let bytes: &[u8] = img.data.as_ref();
                        !bytes.is_empty()
                            && bytes.chunks_exact(4).step_by(64).all(|px| px[3] == 255)
                    })
                    .unwrap_or(false);
                if !is_opaque {
                    all_opaque = false;
                }
            }
            let at = node.absolute_position(0.0, 0.0);
            let lx = (at.x + node.scroll_offset.x as f32) as f64;
            let ly = (at.y + node.scroll_offset.y as f32) as f64 - self.scroll_y;
            let lw = node.final_layout.size.width as f64;
            let lh = node.final_layout.size.height as f64;
            if lw <= 0.0 || lh <= 0.0 {
                continue;
            }
            x0 = x0.min(lx * scale - 2.0);
            y0 = y0.min(ly * scale - 2.0);
            x1 = x1.max((lx + lw) * scale + 2.0);
            y1 = y1.max((ly + lh) * scale + 2.0);
        }

        if !x0.is_finite() {
            return CanvasDamage::Offscreen;
        }
        let cx0 = x0.clamp(0.0, fw);
        let cy0 = y0.clamp(0.0, fh);
        let cx1 = x1.clamp(0.0, fw);
        let cy1 = y1.clamp(0.0, fh);
        if cx1 <= cx0 || cy1 <= cy0 {
            CanvasDamage::Offscreen
        } else if all_opaque && self.dirty_canvas_nodes.len() == 1 {
            CanvasDamage::OpaqueRect(cx0, cy0, cx1, cy1)
        } else {
            CanvasDamage::Rect(cx0, cy0, cx1, cy1)
        }
    }

    fn renew_frontend_budget(&mut self) {
        self.js_abort.store(false, std::sync::atomic::Ordering::Relaxed);
    }

    fn has_pending_raf(&mut self) -> bool {
        let Some(ref mut st) = self.js_state else {
            return false;
        };
        matches!(st.call_global("__zippHasPendingRaf", &[]), Ok(JsValue::Bool(true)))
    }

    /// Handle a tap at physical pixel coordinates `(x, y)` on Android.
    pub fn on_tap(&mut self, phys_x: f32, phys_y: f32) -> InteractionResult {
        self.renew_frontend_budget();
        let logical_x = phys_x / self.scale;
        let logical_y = phys_y / self.scale + self.scroll_y as f32;

        let mut wants_ime = false;
        let mut input_text = String::new();
        let mut input_type = "text".to_string();
        let mut select_picker: Option<SelectPickerRequest> = None;

        if let Some(hit) = self.doc.as_ref().hit(logical_x, logical_y) {
            let chain = self.doc.as_ref().node_chain(hit.node_id);
            let target_flid = chain
                .iter()
                .find_map(|id| self.dom_to_flid.get(id).copied());

            if let Some(flid) = target_flid {
                // Check whether the tapped element is an input/textarea/select/checkbox/submit button
                let meta = self.inspect_js_node(flid);
                let tag = meta
                    .get("tag")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let itype = meta
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("text");

                if tag == "select" {
                    self.focused_flid = None;
                    if let Some(ref mut st) = self.js_state {
                        if let Ok(JsValue::String(raw)) = st.call_global(
                            "__softnInspectSelect",
                            &[JsValue::Number(f64::from(flid))],
                        ) {
                            if let Ok(parsed) = serde_json::from_str::<Value>(&raw) {
                                let opts: Vec<String> = parsed
                                    .get("options")
                                    .and_then(|v| v.as_array())
                                    .map(|arr| {
                                        arr.iter()
                                            .map(|o| {
                                                o.get("label")
                                                    .and_then(|l| l.as_str())
                                                    .unwrap_or("")
                                                    .to_string()
                                            })
                                            .collect()
                                    })
                                    .unwrap_or_default();
                                if !opts.is_empty() {
                                    let selected_index = parsed
                                        .get("selectedIndex")
                                        .and_then(|v| v.as_i64())
                                        .unwrap_or(0) as i32;
                                    select_picker = Some(SelectPickerRequest {
                                        flid,
                                        selected_index,
                                        options: opts,
                                    });
                                }
                            }
                        }
                    }
                } else if tag == "textarea"
                    || (tag == "input"
                        && itype != "checkbox"
                        && itype != "radio"
                        && itype != "submit"
                        && itype != "button")
                {
                    self.focused_flid = Some(flid);
                    wants_ime = true;
                    input_text = meta
                        .get("value")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    input_type = itype.to_string();
                } else {
                    self.focused_flid = None;
                    if tag == "input" && itype == "checkbox" {
                        if let Some(ref mut st) = self.js_state {
                            let _ = st.call_global(
                                "__softnToggleCheckbox",
                                &[JsValue::Number(f64::from(flid))],
                            );
                        }
                    }
                }

                // Dispatch click along the ancestor chain in the JS VM (skip click if opening select picker)
                let mut flid_chain: Vec<u32> = Vec::new();
                for dom_id in &chain {
                    if let Some(&f) = self.dom_to_flid.get(dom_id) {
                        if !flid_chain.contains(&f) {
                            flid_chain.push(f);
                        }
                    }
                }
                if select_picker.is_none() {
                    if let Some(ref mut st) = self.js_state {
                        stacker::maybe_grow(512 * 1024, 16 * 1024 * 1024, || {
                            let mut prevented = false;
                            for handler_flid in &flid_chain {
                                if let Ok(JsValue::Bool(true)) = st.call_global(
                                    "__zippDispatchEvent",
                                    &[
                                        JsValue::Number(f64::from(*handler_flid)),
                                        JsValue::Number(f64::from(flid)),
                                        JsValue::String("click".into()),
                                    ],
                                ) {
                                    prevented = true;
                                }
                            }
                            // If a submit button (or a child inside a submit button) was clicked and click didn't preventDefault, submit ancestor form
                            let is_submit_trigger = (tag == "button" && (itype == "submit" || itype == "text"))
                                || (tag == "input" && itype == "submit");
                            if !prevented {
                                if is_submit_trigger {
                                    let _ = st.call_global(
                                        "__softnSubmitAncestorForm",
                                        &[JsValue::Number(f64::from(flid))],
                                    );
                                } else {
                                    // Check if any ancestor in flid_chain is a submit <button> (e.g., tapping an SVG/span inside a button)
                                    for &anc_flid in &flid_chain {
                                        if anc_flid == flid {
                                            continue;
                                        }
                                        let anc_json = match st.call_global(
                                            "__softnInspectNode",
                                            &[JsValue::Number(f64::from(anc_flid))],
                                        ) {
                                            Ok(JsValue::String(s)) => serde_json::from_str::<Value>(&s).unwrap_or_else(|_| json!({})),
                                            _ => json!({}),
                                        };
                                        let atag = anc_json.get("tag").and_then(|v| v.as_str()).unwrap_or("");
                                        let atype = anc_json.get("type").and_then(|v| v.as_str()).unwrap_or("text");
                                        if atag == "button" && (atype == "submit" || atype == "text") {
                                            let _ = st.call_global(
                                                "__softnSubmitAncestorForm",
                                                &[JsValue::Number(f64::from(anc_flid))],
                                            );
                                            break;
                                        }
                                    }
                                }
                            }
                            drain_js(st);
                            // Also step rAF once after tab/UI clicks so any newly mounted <canvas> paints immediately!
                            let _ = st.call_global("__zippStepRaf", &[]);
                            drain_js(st);
                        });
                    }
                    self.sync_dom_from_vm();
                    // If the click mounted a brand-new <canvas>, LayoutCache now has its real container size;
                    // run one more rAF frame so the new canvas draws immediately on the same tap!
                    if let Some(ref mut st) = self.js_state {
                        stacker::maybe_grow(512 * 1024, 16 * 1024 * 1024, || {
                            let _ = st.call_global("__zippStepRaf", &[]);
                            drain_js(st);
                        });
                    }
                    self.sync_dom_from_vm();
                }
            }
        }

        let wants_raf = self.has_pending_raf();
        InteractionResult {
            dirty: self.dirty,
            wants_raf,
            ime_changed: true,
            wants_ime,
            input_text,
            input_type,
            pending_calls: self.drain_pending_calls(),
            select_picker,
        }
    }

    /// Select an option on a `<select>` element chosen from Android's native picker dialog.
    pub fn on_select_option(&mut self, flid: u32, index: i32) -> InteractionResult {
        self.renew_frontend_budget();
        if let Some(ref mut st) = self.js_state {
            let _ = st.call_global(
                "__softnSetSelectIndex",
                &[JsValue::Number(f64::from(flid)), JsValue::Number(f64::from(index))],
            );
            drain_js(st);
            let _ = st.call_global("__zippStepRaf", &[]);
            drain_js(st);
        }
        self.sync_dom_from_vm();
        if let Some(ref mut st) = self.js_state {
            let _ = st.call_global("__zippStepRaf", &[]);
            drain_js(st);
        }
        self.sync_dom_from_vm();
        let wants_raf = self.has_pending_raf();
        InteractionResult {
            dirty: self.dirty,
            wants_raf,
            ime_changed: false,
            wants_ime: false,
            input_text: String::new(),
            input_type: "text".to_string(),
            pending_calls: self.drain_pending_calls(),
            select_picker: None,
        }
    }

    /// Begin a touch pointer sequence. Returns `true` if the touch landed on an interactive
    /// `<canvas>` (such as the 3D Mine Map), capturing subsequent move/up drags to orbit/pan
    /// instead of scrolling the page.
    pub fn on_pointer_down(&mut self, phys_x: f32, phys_y: f32) -> bool {
        self.active_canvas_flid = None;
        self.last_pointer_phys = (phys_x, phys_y);
        let logical_x = phys_x / self.scale;
        let logical_y = phys_y / self.scale + self.scroll_y as f32;
        let Some(hit) = self.doc.as_ref().hit(logical_x, logical_y) else {
            return false;
        };
        let chain = self.doc.as_ref().node_chain(hit.node_id);
        let canvas_flid = chain.iter().find_map(|&dom_id| {
            let node = self.doc.as_ref().get_node(dom_id)?;
            let is_canvas = node
                .element_data()
                .map(|e| e.name.local.as_ref() == "canvas")
                .unwrap_or(false);
            if is_canvas {
                self.dom_to_flid.get(&dom_id).copied()
            } else {
                None
            }
        });
        let Some(flid) = canvas_flid else {
            return false;
        };
        self.active_canvas_flid = Some(flid);
        self.renew_frontend_budget();
        let client_x = f64::from(phys_x / self.scale);
        let client_y = f64::from(phys_y / self.scale);
        if let Some(ref mut st) = self.js_state {
            let _ = st.call_global(
                "__zippDispatchPointerEvent",
                &[
                    JsValue::Number(f64::from(flid)),
                    JsValue::String("pointerdown".into()),
                    JsValue::Number(client_x),
                    JsValue::Number(client_y),
                    JsValue::Number(0.0),
                    JsValue::Number(1.0),
                    JsValue::Number(1.0),
                ],
            );
            drain_js(st);
        }
        true
    }

    /// Route pointer movement while a `<canvas>` drag is active (3D camera orbit/pan).
    pub fn on_pointer_move(&mut self, phys_x: f32, phys_y: f32) -> InteractionResult {
        if let Some(flid) = self.active_canvas_flid {
            self.renew_frontend_budget();
            let client_x = f64::from(phys_x / self.scale);
            let client_y = f64::from(phys_y / self.scale);
            if let Some(ref mut st) = self.js_state {
                let _ = st.call_global(
                    "__zippDispatchPointerEvent",
                    &[
                        JsValue::Number(f64::from(flid)),
                        JsValue::String("pointermove".into()),
                        JsValue::Number(client_x),
                        JsValue::Number(client_y),
                        JsValue::Number(0.0),
                        JsValue::Number(1.0),
                        JsValue::Number(1.0),
                    ],
                );
                drain_js(st);
                let _ = st.call_global("__zippStepRaf", &[]);
                drain_js(st);
            }
            self.sync_dom_from_vm();
        }
        let wants_raf = self.has_pending_raf();
        InteractionResult {
            dirty: self.dirty,
            wants_raf,
            ime_changed: false,
            wants_ime: false,
            input_text: String::new(),
            input_type: "text".to_string(),
            pending_calls: self.drain_pending_calls(),
            select_picker: None,
        }
    }

    /// Finish a `<canvas>` touch drag (triggers click-picking on short taps inside 3D view).
    pub fn on_pointer_up(&mut self, phys_x: f32, phys_y: f32) -> InteractionResult {
        if let Some(flid) = self.active_canvas_flid.take() {
            self.renew_frontend_budget();
            let client_x = f64::from(phys_x / self.scale);
            let client_y = f64::from(phys_y / self.scale);
            if let Some(ref mut st) = self.js_state {
                let _ = st.call_global(
                    "__zippDispatchPointerEvent",
                    &[
                        JsValue::Number(f64::from(flid)),
                        JsValue::String("pointerup".into()),
                        JsValue::Number(client_x),
                        JsValue::Number(client_y),
                        JsValue::Number(0.0),
                        JsValue::Number(0.0),
                        JsValue::Number(1.0),
                    ],
                );
                drain_js(st);
                let _ = st.call_global("__zippStepRaf", &[]);
                drain_js(st);
            }
            self.sync_dom_from_vm();
        }
        let wants_raf = self.has_pending_raf();
        InteractionResult {
            dirty: self.dirty,
            wants_raf,
            ime_changed: false,
            wants_ime: false,
            input_text: String::new(),
            input_type: "text".to_string(),
            pending_calls: self.drain_pending_calls(),
            select_picker: None,
        }
    }

    /// Scroll vertically by `delta_phys_y` physical pixels (positive = scroll down).
    /// Supports both nested `overflow-y: auto|scroll` containers (such as flex app shells)
    /// and root document viewport scrolling via `zipp-dom`'s bubbling `scroll_by`.
    pub fn on_scroll(&mut self, delta_phys_y: f32) -> bool {
        let delta_logical = (delta_phys_y / self.scale) as f64;
        let (px, py) = self.last_pointer_phys;
        let lx = if px > 0.0 {
            px / self.scale
        } else {
            (self.width as f32 * 0.5) / self.scale
        };
        let ly = if py > 0.0 {
            py / self.scale
        } else {
            (self.height as f32 * 0.5) / self.scale
        };
        let anchor_id = self
            .doc
            .as_ref()
            .hit(lx, ly + self.scroll_y as f32)
            .map(|h| h.node_id);
        let changed = self
            .doc
            .as_mut()
            .scroll_by(anchor_id, 0.0, -delta_logical, &mut |_| {});
        if !changed {
            return false;
        }
        self.scroll_y = self.doc.as_ref().viewport_scroll().y;
        if let Ok(mut lc) = self.layout_cache.lock() {
            lc.update_from_doc(self.doc.as_ref(), &self.flid_to_dom, self.scroll_y);
        }
        self.full_repaint = true;
        self.dirty = true;
        true
    }

    /// Set text value of the currently focused `<input>` / `<textarea>` from Android IME.
    pub fn on_text_input(&mut self, full_text: &str) -> InteractionResult {
        self.renew_frontend_budget();
        if let Some(flid) = self.focused_flid {
            if let Some(ref mut st) = self.js_state {
                let _ = st.call_global(
                    "__softnSetInputValue",
                    &[
                        JsValue::Number(f64::from(flid)),
                        JsValue::String(full_text.to_string()),
                    ],
                );
                drain_js(st);
            }
            self.sync_dom_from_vm();
        }
        let wants_raf = self.has_pending_raf();
        InteractionResult {
            dirty: self.dirty,
            wants_raf,
            ime_changed: false,
            wants_ime: self.focused_flid.is_some(),
            input_text: full_text.to_string(),
            input_type: "text".to_string(),
            pending_calls: self.drain_pending_calls(),
            select_picker: None,
        }
    }

    /// Submit focused form on IME Done/Enter key.
    pub fn on_submit_ime(&mut self) -> InteractionResult {
        self.renew_frontend_budget();
        if let Some(flid) = self.focused_flid {
            if let Some(ref mut st) = self.js_state {
                let _ = st.call_global(
                    "__softnSubmitAncestorForm",
                    &[JsValue::Number(f64::from(flid))],
                );
                drain_js(st);
            }
            self.focused_flid = None;
            self.sync_dom_from_vm();
        }
        let wants_raf = self.has_pending_raf();
        InteractionResult {
            dirty: self.dirty,
            wants_raf,
            ime_changed: true,
            wants_ime: false,
            input_text: String::new(),
            input_type: "text".to_string(),
            pending_calls: self.drain_pending_calls(),
            select_picker: None,
        }
    }

    /// Resolve an asynchronous native Android hardware call back into the live `zipp-vm` Promise.
    pub fn resolve_device_call(
        &mut self,
        request_id: u64,
        ok: bool,
        payload_json: &str,
    ) -> InteractionResult {
        self.renew_frontend_budget();
        if let Some(ref mut st) = self.js_state {
            let _ = st.call_global(
                "__softnResolveDeviceCall",
                &[
                    JsValue::Number(request_id as f64),
                    JsValue::Bool(ok),
                    JsValue::String(payload_json.to_string()),
                ],
            );
            drain_js(st);
        }
        self.sync_dom_from_vm();
        let wants_raf = self.has_pending_raf();
        InteractionResult {
            dirty: self.dirty,
            wants_raf,
            ime_changed: false,
            wants_ime: self.focused_flid.is_some(),
            input_text: String::new(),
            input_type: "text".to_string(),
            pending_calls: self.drain_pending_calls(),
            select_picker: None,
        }
    }

    pub fn tick(&mut self) -> InteractionResult {
        self.renew_frontend_budget();
        let mut changed = false;
        if let Some(ref be) = self.backend {
            let ws_frames = be.poll_rust_timers();
            if !ws_frames.is_empty() {
                if let Some(ref mut st) = self.js_state {
                    let frames_json = Value::Array(ws_frames).to_string();
                    let _ = st.call_global(
                        "__softnDeliverWsFrames",
                        &[JsValue::String(frames_json)],
                    );
                    st.run_microtasks();
                    changed = true;
                }
            }
        }
        if let Some(ref mut st) = self.js_state {
            if let Ok(JsValue::Number(n)) =
                st.call_global("__zippDrainQueues", &[JsValue::Number(8.0)])
            {
                if n > 0.0 {
                    st.run_microtasks();
                    changed = true;
                }
            }
            if let Ok(JsValue::Number(r)) = st.call_global("__zippStepRaf", &[]) {
                if r > 0.0 {
                    st.run_microtasks();
                    changed = true;
                }
            }
        }
        if changed {
            self.sync_dom_from_vm();
        } else {
            let updated = show_canvas_frames(
                self.doc.as_mut(),
                &self.flid_to_dom,
                canvas::take_frames(),
            );
            if !updated.is_empty() {
                self.dirty_canvas_nodes.extend(updated);
                self.dirty = true;
            }
        }
        let wants_raf = self.has_pending_raf();
        InteractionResult {
            dirty: self.dirty,
            wants_raf,
            ime_changed: false,
            wants_ime: self.focused_flid.is_some(),
            input_text: String::new(),
            input_type: "text".to_string(),
            pending_calls: self.drain_pending_calls(),
            select_picker: None,
        }
    }

    pub fn drain_pending_calls(&self) -> Vec<PendingDeviceRequest> {
        self.pending_calls
            .lock()
            .map(|mut q| std::mem::take(&mut *q))
            .unwrap_or_default()
    }

    pub fn bundle_info_json(&self) -> String {
        json!({
            "appId": self.bundle.app_id,
            "name": self.bundle.name,
            "version": self.bundle.version,
            "capabilities": self.bundle.declared_capabilities(),
            "hasBackend": self.backend.is_some(),
            "backendRuntime": self.bundle.manifest.backend.as_ref().map(|b| b.runtime.as_str()),
            "databases": self.bundle.manifest.databases.as_ref().map(|d| d.len()).unwrap_or(0),
        })
        .to_string()
    }

    /// Clear HTTP disk + memory cache and decoded image caches for this app.
    pub fn clear_cache(&mut self) -> String {
        canvas2d::clear_image_cache();
        let (entries, bytes) = self.http_cache.clear();
        json!({ "ok": true, "entries": entries, "bytes": bytes }).to_string()
    }

    /// Clear cache, cookies, sessionStorage, localStorage, IndexedDB, and blobs for this app.
    pub fn clear_site_data(&mut self) -> String {
        canvas2d::clear_image_cache();
        let (entries, bytes) = self.http_cache.clear();
        let cookies = self.cookies.clear();
        if let Ok(mut sess) = self.session_storage.lock() {
            sess.clear();
        }
        self.storage.clear_local_storage();
        let _ = std::fs::remove_dir_all(self.app_data_dir.join("idb"));
        http_cache::clear_blobs();
        json!({ "ok": true, "entries": entries, "bytes": bytes, "cookies": cookies }).to_string()
    }

    pub fn current_rendered_html(&mut self) -> String {
        let Some(ref mut st) = self.js_state else {
            return String::new();
        };
        match st.call_global("__getRenderedHTML", &[]) {
            Ok(JsValue::String(s)) => s,
            _ => String::new(),
        }
    }

    #[cfg(test)]
    pub fn canvas_frame_summary(&self) -> Vec<(u32, u32, usize)> {
        let doc = self.doc.as_ref();
        let mut out = Vec::new();
        for &node_id in self.flid_to_dom.values() {
            if let Some(el) = doc.get_node(node_id).and_then(|n| n.element_data()) {
                if el.name.local.as_ref() == "canvas" {
                    if let Some(img) = el.raster_image_data() {
                        let bytes: &[u8] = img.data.as_ref();
                        let first = bytes.get(0..4).unwrap_or(&[0, 0, 0, 0]);
                        let varied = bytes
                            .chunks_exact(4)
                            .filter(|px| px[3] > 0 && *px != first)
                            .count();
                        out.push((img.width, img.height, varied));
                    } else {
                        out.push((0, 0, 0));
                    }
                }
            }
        }
        out
    }

    #[cfg(test)]
    pub fn find_flid_physical_center(&self, flid: u32) -> Option<(f32, f32)> {
        let doc = self.doc.as_ref();
        let w = (self.width as f32) / self.scale;
        let h = (self.height as f32) / self.scale;
        // Probe with the real Blitz hit-tester to find an exact screen coordinate that hits `flid`
        let mut y = 8.0f32;
        while y < h {
            let mut x = 16.0f32;
            while x < w {
                if let Some(hit) = doc.hit(x, y + self.scroll_y as f32) {
                    let chain = doc.node_chain(hit.node_id);
                    if chain.iter().any(|id| self.dom_to_flid.get(id) == Some(&flid)) {
                        return Some((x * self.scale, y * self.scale));
                    }
                }
                x += 24.0;
            }
            y += 12.0;
        }
        None
    }

    fn inspect_js_node(&mut self, flid: u32) -> Value {
        let Some(ref mut st) = self.js_state else {
            return json!({});
        };
        match st.call_global("__softnInspectNode", &[JsValue::Number(f64::from(flid))]) {
            Ok(JsValue::String(s)) => serde_json::from_str(&s).unwrap_or_else(|_| json!({})),
            _ => json!({}),
        }
    }

    fn sync_dom_from_vm(&mut self) {
        let Some(ref mut st) = self.js_state else {
            return;
        };
        // Fast path for steady 3D / WebGL / Canvas 2D animation frames: if JS made
        // zero DOM mutations since our last sync, skip serializing the entire DOM tree
        // and jump straight to swapping any newly presented canvas frames!
        let cur_mutations = match st.call_global("__zippDomMutationCount", &[]) {
            Ok(JsValue::Number(n)) => n as i64,
            _ => -1,
        };
        if self.last_dom_mutations >= 0 && cur_mutations == self.last_dom_mutations {
            let updated = show_canvas_frames(
                self.doc.as_mut(),
                &self.flid_to_dom,
                canvas::take_frames(),
            );
            if !updated.is_empty() {
                self.dirty_canvas_nodes.extend(updated);
                self.dirty = true;
            }
            return;
        }
        let body_html = match stacker::maybe_grow(512 * 1024, 16 * 1024 * 1024, || {
            st.call_global("__getRenderedHTML", &[])
        }) {
            Ok(JsValue::String(s)) if !s.is_empty() => s,
            _ => return,
        };
        let dyn_css = match st.call_global("__getDynamicHeadStyles", &[]) {
            Ok(JsValue::String(s)) => s,
            _ => String::new(),
        };
        let full_css = if dyn_css.is_empty() {
            self.head_css.clone()
        } else {
            format!("{}\n{}", self.head_css, dyn_css)
        };
        let html_attrs = match st.call_global("__getHtmlClass", &[]) {
            Ok(JsValue::String(s)) => s,
            _ => String::new(),
        };
        self.last_dom_mutations = cur_mutations;
        let full_html = compose_document_html(&full_css, &body_html, &html_attrs);
        if full_html == self.last_synced_html {
            let updated = show_canvas_frames(
                self.doc.as_mut(),
                &self.flid_to_dom,
                canvas::take_frames(),
            );
            if !updated.is_empty() {
                self.dirty_canvas_nodes.extend(updated);
                self.dirty = true;
            }
            return;
        }
        self.last_synced_html = full_html.clone();
        // Preserve per-element nested container scroll offsets across DOM rebuilds
        let saved_scrolls: Vec<(u32, zipp_dom::Point<f64>)> = self
            .flid_to_dom
            .iter()
            .filter_map(|(&flid, &dom_id)| {
                let n = self.doc.as_ref().get_node(dom_id)?;
                if n.scroll_offset.x != 0.0 || n.scroll_offset.y != 0.0 {
                    Some((flid, n.scroll_offset))
                } else {
                    None
                }
            })
            .collect();
        let mut doc = build_html_document(
            &full_html,
            &self.bundle,
            &self.http_cache,
            &self.cookies,
            self.width,
            self.height,
            self.scale,
        );
        let max_scroll = {
            let l = doc.as_ref().root_element().final_layout;
            let content_h = l.size.height.max(l.content_size.height) as f64;
            let view_h = (self.height as f64) / (self.scale as f64);
            (content_h - view_h).max(0.0)
        };
        self.scroll_y = self.scroll_y.clamp(0.0, max_scroll);
        doc.as_mut()
            .set_viewport_scroll(zipp_dom::Point { x: 0.0, y: self.scroll_y });

        self.flid_to_dom = build_flid_map(doc.as_ref());
        self.dom_to_flid = self
            .flid_to_dom
            .iter()
            .map(|(&flid, &dom_id)| (dom_id, flid))
            .collect();
        for (flid, offset) in saved_scrolls {
            if let Some(&dom_id) = self.flid_to_dom.get(&flid) {
                if let Some(node) = doc.as_mut().get_node_mut(dom_id) {
                    let max_y = (node.final_layout.content_size.height - node.final_layout.size.height).max(0.0) as f64;
                    let max_x = (node.final_layout.content_size.width - node.final_layout.size.width).max(0.0) as f64;
                    node.scroll_offset = zipp_dom::Point {
                        x: offset.x.clamp(0.0, max_x),
                        y: offset.y.clamp(0.0, max_y),
                    };
                }
            }
        }
        if let Ok(mut lc) = self.layout_cache.lock() {
            lc.update_from_doc(doc.as_ref(), &self.flid_to_dom, self.scroll_y);
        }
        // Flush any newly presented canvas frames into SHOWN, then attach every canvas's
        // latest rendered picture directly onto its new DOM node.
        let _ = canvas::take_frames();
        let _ = show_canvas_frames(doc.as_mut(), &self.flid_to_dom, canvas::shown_frames());
        self.doc = doc;
        // build_html_document already polled and resolved layout twice; no need to re-resolve in render_rgba
        self.layout_dirty = false;
        self.full_repaint = true;
        self.dirty_canvas_nodes.clear();
        self.dirty = true;
    }
}

enum CanvasDamage {
    Full,
    OpaqueRect(f64, f64, f64, f64),
    Rect(f64, f64, f64, f64),
    Offscreen,
}

fn show_canvas_frames(
    doc: &mut BaseDocument,
    flid_to_dom: &HashMap<u32, usize>,
    frames: Vec<(u32, canvas::Shown)>,
) -> Vec<usize> {
    use zipp_dom::node::{ImageData, RasterImageData, SpecialElementData};
    if frames.is_empty() {
        return Vec::new();
    }
    let mut updated = Vec::new();
    for (flid, (width, height, rgba)) in frames {
        let Some(&node_id) = flid_to_dom.get(&flid) else {
            continue;
        };
        if let Some(element) = doc.get_node_mut(node_id).and_then(|n| n.element_data_mut()) {
            let image = RasterImageData::new(width, height, rgba);
            element.special_data =
                SpecialElementData::Image(Box::new(ImageData::Raster(image)));
            updated.push(node_id);
        }
    }
    updated
}

fn drain_js(st: &mut ScriptState) {
    stacker::maybe_grow(512 * 1024, 16 * 1024 * 1024, || {
        for _ in 0..10 {
            st.run_microtasks();
            match st.call_global("__zippDrainQueues", &[JsValue::Number(50.0)]) {
                Ok(JsValue::Number(n)) if n > 0.0 => continue,
                _ => break,
            }
        }
        st.run_microtasks();
    });
}

fn load_embedded_fonts() -> FontContext {
    let mut font_ctx = FontContext::default();
    zipp_dom::register_embedded_fonts(&mut font_ctx.collection);
    font_ctx
        .collection
        .register_fonts(Blob::new(Arc::new(zipp_dom::BULLET_FONT) as _), None);
    font_ctx
}

fn build_html_document(
    html: &str,
    bundle: &Arc<SoftnBundle>,
    http_cache: &Arc<MobileHttpCache>,
    cookies: &Arc<MobileCookieJar>,
    width: u32,
    height: u32,
    scale: f32,
) -> HtmlDocument {
    stacker::maybe_grow(512 * 1024, 16 * 1024 * 1024, || {
        let font_ctx = load_embedded_fonts();
        let net_provider = Arc::new(VfsNetProvider {
            bundle: Arc::clone(bundle),
            http_cache: Arc::clone(http_cache),
            cookies: Arc::clone(cookies),
        });
        let config = DocumentConfig {
            font_ctx: Some(font_ctx),
            base_url: Some(format!("{VIRTUAL_ORIGIN}/")),
            net_provider: Some(net_provider),
            html_parser_provider: Some(Arc::new(HtmlProvider)),
            ..Default::default()
        };
        let mut doc = HtmlDocument::from_html(html, config);
        let color_scheme = match bundle.manifest.theme.as_deref() {
            Some("light") => ColorScheme::Light,
            _ => ColorScheme::Dark,
        };
        doc.as_mut().set_viewport(Viewport::new(
            width.max(1),
            height.max(1),
            scale.max(1.0),
            color_scheme,
        ));
        // Drain synchronous VfsNetProvider image/stylesheet loads and resolve layout
        doc.as_mut().poll(None);
        doc.as_mut().resolve(0.0);
        doc.as_mut().poll(None);
        doc.as_mut().resolve(0.0);
        doc
    })
}

fn compose_document_html(head_css: &str, body_html: &str, html_attrs: &str) -> String {
    let attrs = if html_attrs.trim().is_empty() {
        String::new()
    } else {
        format!(" {html_attrs}")
    };
    format!(
        "<!DOCTYPE html><html{attrs}><head><meta charset=\"utf-8\"><style>\n*{{box-sizing:border-box;}}html,body{{margin:0;padding:0;min-height:100%;}}\n{head_css}\n</style></head><body>{body_html}</body></html>"
    )
}

fn build_flid_map(doc: &BaseDocument) -> HashMap<u32, usize> {
    let mut map = HashMap::new();
    let flid_attr = LocalName::from("data-flid");
    walk_flid(doc, 0, &flid_attr, &mut map);
    map
}

fn walk_flid(
    doc: &BaseDocument,
    node_id: usize,
    flid_attr: &LocalName,
    map: &mut HashMap<u32, usize>,
) {
    if let Some(node) = doc.get_node(node_id) {
        if let Some(val) = node.attr(flid_attr.clone()) {
            if let Ok(flid) = val.parse::<u32>() {
                map.insert(flid, node_id);
            }
        }
        for &kid in &node.children {
            walk_flid(doc, kid, flid_attr, map);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn boot_frontend_vm(
    raw_html: &str,
    initial_body: &str,
    initial_html_attrs: &[(String, String)],
    logical_w: u32,
    logical_h: u32,
    scale: f32,
    bundle: &Arc<SoftnBundle>,
    backend: Option<&Arc<LocalContainerBackend>>,
    storage: &SqlStorage,
    app_data_dir: &Path,
    http_cache: Arc<MobileHttpCache>,
    cookies: Arc<MobileCookieJar>,
    session_storage: Arc<Mutex<HashMap<String, String>>>,
    host_info: HostDeviceInfo,
    layout_cache: Arc<Mutex<LayoutCache>>,
    pending_calls: Arc<Mutex<Vec<PendingDeviceRequest>>>,
    logs: Arc<Mutex<Vec<String>>>,
    abort: Arc<AtomicBool>,
) -> Result<Option<ScriptState>, String> {
    let (classic_scripts, module_entries) = extract_scripts(raw_html, bundle);
    if classic_scripts.is_empty() && module_entries.is_empty() {
        return Ok(None);
    }

    let escaped_body = initial_body
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\'', "\\'")
        .replace('\n', "\\n")
        .replace('\r', "");

    let html_attrs_map: serde_json::Map<String, Value> = initial_html_attrs
        .iter()
        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
        .collect();
    let escaped_html_attrs = Value::Object(html_attrs_map)
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");

    let prefers_dark = !matches!(bundle.manifest.theme.as_deref(), Some("light"));
    let escaped_title = bundle
        .name
        .replace('\\', "\\\\")
        .replace('\'', "\\'");

    let shim = format!(
        "{HOST_PRELUDE_JS}\n{ZIPP_CONSOLE_SHIM}\n{ZIPP_CLONE_JS}\n{ZIPP_DOM_SHIM}\n{ZIPP_IDB_SHIM}\n{ZIPP_WEBGL_SHIM}\n{ZIPP_CANVAS2D_SHIM}\n{SOFTN_PRELUDE_JS}"
    )
    .replace("__PAGE_URL__", &format!("{VIRTUAL_ORIGIN}/"))
    .replace("__PAGE_TITLE__", &escaped_title)
    .replace("__PREFERS_DARK__", if prefers_dark { "true" } else { "false" })
    .replace("__VIEWPORT_W__", &logical_w.to_string())
    .replace("__VIEWPORT_H__", &logical_h.to_string())
    .replace("__BODY_HTML__", &escaped_body)
    .replace("__SOFTN_BODY_HTML__", &escaped_body)
    .replace("__SOFTN_HTML_ATTRS_JSON__", &escaped_html_attrs);

    let mut full = shim;
    full.push_str(&format!("\nwindow.devicePixelRatio = {scale}; devicePixelRatio = {scale};\n"));

    for src in classic_scripts {
        full.push('\n');
        full.push_str(&src);
        full.push_str("\n;\n");
    }

    if !module_entries.is_empty() {
        let bundle_ref = Arc::clone(bundle);
        let bundle_ref2 = Arc::clone(bundle);
        let linked = esm::link_modules(
            &module_entries,
            &move |importer, spec| {
                if spec.starts_with("softn:") {
                    return spec.to_string();
                }
                let imp_path = importer
                    .strip_prefix(&format!("{VIRTUAL_ORIGIN}/"))
                    .unwrap_or(importer);
                if let Some(resolved) =
                    resolve_vfs_url(spec, imp_path, &bundle_ref.frontend_root)
                {
                    let mut final_path = resolved.clone();
                    if !bundle_ref.vfs.contains_key(&final_path) {
                        for ext in [".js", ".mjs", ".ts", ".tsx", ".jsx", "/index.js", "/index.ts"] {
                            let cand = format!("{resolved}{ext}");
                            if bundle_ref.vfs.contains_key(&cand) {
                                final_path = cand;
                                break;
                            }
                        }
                    }
                    format!("{VIRTUAL_ORIGIN}/{final_path}")
                } else {
                    spec.to_string()
                }
            },
            &move |url| {
                let rel = url
                    .strip_prefix(&format!("{VIRTUAL_ORIGIN}/"))
                    .unwrap_or(url);
                bundle_ref2
                    .read_text(rel)
                    .ok_or_else(|| format!("Module not found in .softn archive: {rel}"))
            },
        );
        full.push('\n');
        full.push_str(&linked.source);
        full.push('\n');
    }

    // Grow the stack in-place (ScriptState is !Send) so large React/Three.js
    // bundles never overflow Android's 1 MiB JNI thread stack or debug test stacks.
    let mut st = stacker::grow(16 * 1024 * 1024, || {
        compile_source(&full, Frontend::TypeScript)
    })
    .map_err(|e| format!("Frontend JS/TS compile error: {e}"))?
    .into_state();

    // Match `zipp-browser`'s interactive page VM: leave instrumentation limits unset
    // on the trusted local frontend VM so both x86-64 and ARM64 native JIT stay enabled
    // at full speed (attaching an instrumentation recorder disables the ARM64 JIT in
    // zipp-vm and counts the x86-64 JIT's pinned 256 MiB register reservation against
    // `heap_limit`). The backend Container v2 VMs in `backend.rs` remain fully sandboxed.

    let bundle_hc = Arc::clone(bundle);
    let backend_hc = backend.cloned();
    let storage_hc = storage.clone();
    let idb_dir = app_data_dir.join("idb");
    let http_cache_hc = Arc::clone(&http_cache);
    let cookies_hc = Arc::clone(&cookies);
    let session_hc = Arc::clone(&session_storage);
    let layout_hc = Arc::clone(&layout_cache);
    let pending_hc = Arc::clone(&pending_calls);
    let logs_hc = Arc::clone(&logs);
    let mut pending_bytes: Vec<u8> = Vec::new();
    let mut frontend_sql = RequestSqlBridge::new(storage.clone(), abort);

    st.set_host_call_ctx(Box::new(move |ctx: &mut dyn HostCtx, kind: &str, args: &[String]| {
        let arg = |i: usize| args.get(i).map(String::as_str).unwrap_or("");
        match kind {
            "c2d" => canvas2d::host_call(ctx, &bundle_hc, args),
            #[cfg(target_os = "android")]
            "gl" => crate::webgl::host_call(ctx, Arc::clone(&bundle_hc), args),
            #[cfg(not(target_os = "android"))]
            "gl" => Err("Error: WebGL is available on Android EGL builds".into()),
            "image_info" => Ok(probe_image_info(&bundle_hc, arg(0))),
            "url" => url_parts(arg(0), arg(1)),
            "url_set" => url_set(arg(0), arg(1), arg(2)),
            "random" => {
                let n: usize = arg(0).parse().unwrap_or(0).min(65536);
                Ok(random_hex(n))
            }
            "parse_html" => Ok(html_seed::parse_html_json(arg(0))
                .unwrap_or_else(|| "[[],[],[],[]]".to_string())),
            "blob_create" => {
                let mode = arg(0);
                let mime = arg(1);
                let origin = arg(2);
                let bytes = if mode == "text" {
                    arg(3).as_bytes().to_vec()
                } else {
                    let len: usize = mode.parse().unwrap_or(0);
                    if len == 0 {
                        Vec::new()
                    } else {
                        let (addr, count, ty) = ctx.typed_array_region("__zippHostBytes")?;
                        if ty != 1 || len > count {
                            return Err("RangeError: blob_create buffer mismatch".into());
                        }
                        unsafe { std::slice::from_raw_parts(addr as *const u8, len).to_vec() }
                    }
                };
                Ok(http_cache::store_blob(origin, mime, bytes))
            }
            "blob_revoke" => {
                http_cache::revoke_blob(arg(0));
                Ok(String::new())
            }
            "blob_read" => {
                let url = arg(0);
                let Some((mime, bytes)) = http_cache::lookup_blob(url) else {
                    return Err(format!("TypeError: blob URL not found: {url}"));
                };
                let len = bytes.len();
                pending_bytes = bytes;
                Ok(json!({ "len": len, "type": mime }).to_string())
            }
            "http_bytes" => {
                let raw_url = arg(0);
                let credentials = arg(1);
                let (status, mime, bytes, err) = handle_softn_fetch_bytes(
                    &bundle_hc,
                    backend_hc.as_deref(),
                    &http_cache_hc,
                    &cookies_hc,
                    raw_url,
                    credentials,
                );
                if let Some(error) = err {
                    return Ok(json!({ "error": error }).to_string());
                }
                let len = bytes.len();
                pending_bytes = bytes;
                Ok(json!({
                    "status": status,
                    "ok": (200..300).contains(&status),
                    "len": len,
                    "contentType": mime,
                })
                .to_string())
            }
            "take_bytes" => {
                let (addr, count, ty) = ctx.typed_array_region("__zippHostBytes")?;
                if ty != 1 {
                    return Err("TypeError: __zippHostBytes must be a Uint8Array".into());
                }
                let bytes = std::mem::take(&mut pending_bytes);
                if bytes.len() > count {
                    return Err("RangeError: __zippHostBytes is too small".into());
                }
                unsafe {
                    std::ptr::copy_nonoverlapping(bytes.as_ptr(), addr as *mut u8, bytes.len());
                }
                Ok(bytes.len().to_string())
            }
            "site_read" => {
                let store_kind = arg(0);
                let name = arg(1);
                match store_kind {
                    "local" => Ok(json!(storage_hc.load_local_storage()).to_string()),
                    "session" => Ok(session_hc
                        .lock()
                        .ok()
                        .and_then(|s| s.get(name).cloned())
                        .unwrap_or_default()),
                    "idb" => Ok(read_idb_file(&idb_dir, name).unwrap_or_default()),
                    _ => Ok(String::new()),
                }
            }
            "site_write" => {
                let store_kind = arg(0);
                let name = arg(1);
                let data = arg(2);
                match store_kind {
                    "local" => {
                        if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(data) {
                            storage_hc.clear_local_storage();
                            for (k, v) in map {
                                if let Some(s) = v.as_str() {
                                    storage_hc.set_local_storage(&k, s);
                                }
                            }
                        }
                        Ok(String::new())
                    }
                    "session" => {
                        if let Ok(mut s) = session_hc.lock() {
                            s.insert(name.to_string(), data.to_string());
                        }
                        Ok(String::new())
                    }
                    "idb" => {
                        write_idb_file(&idb_dir, name, Some(data));
                        Ok(String::new())
                    }
                    _ => Ok(String::new()),
                }
            }
            "site_delete" => {
                let store_kind = arg(0);
                let name = arg(1);
                match store_kind {
                    "session" => {
                        if let Ok(mut s) = session_hc.lock() {
                            s.remove(name);
                        }
                    }
                    "idb" => write_idb_file(&idb_dir, name, None),
                    _ => {}
                }
                Ok(String::new())
            }
            "site_names" => {
                let store_kind = arg(0);
                match store_kind {
                    "idb" => Ok(list_idb_names(&idb_dir)),
                    "session" => {
                        let names: Vec<String> = session_hc
                            .lock()
                            .map(|s| s.keys().cloned().collect())
                            .unwrap_or_default();
                        Ok(json!(names).to_string())
                    }
                    _ => Ok("[]".into()),
                }
            }
            "cookie_get" => {
                let target = url::Url::parse(arg(0))
                    .ok()
                    .or_else(|| url::Url::parse(&format!("{VIRTUAL_ORIGIN}/")).ok());
                Ok(target
                    .map(|u| cookies_hc.document_cookie(&u))
                    .unwrap_or_default())
            }
            "cookie_set" => {
                let target = url::Url::parse(arg(0))
                    .ok()
                    .or_else(|| url::Url::parse(&format!("{VIRTUAL_ORIGIN}/")).ok());
                if let Some(u) = target {
                    cookies_hc.set_document_cookie(&u, arg(1));
                }
                Ok(String::new())
            }
            "computed_style" => Ok(layout_hc
                .lock()
                .map(|lc| lc.format_style(arg(0), arg(1)))
                .unwrap_or_default()),
            "hit" => Ok(layout_hc
                .lock()
                .map(|lc| lc.hit(arg(0), arg(1)))
                .unwrap_or_default()),
            "seed" | "window_title" | "fullscreen" => Ok(String::new()),
            "box" => Ok(layout_hc
                .lock()
                .map(|lc| lc.format_box(arg(0)))
                .unwrap_or_default()),
            "rect" => Ok(layout_hc
                .lock()
                .map(|lc| lc.format_rect(arg(0)))
                .unwrap_or_default()),
            "log" => {
                let line = format!("[{}] {}", arg(0), arg(1));
                #[cfg(target_os = "android")]
                log::info!("{line}");
                if let Ok(mut buf) = logs_hc.lock() {
                    if buf.len() < 500 {
                        buf.push(line);
                    }
                }
                Ok(String::new())
            }
            "softn.storage" => match arg(0) {
                "load_all" => Ok(json!(storage_hc.load_local_storage()).to_string()),
                "set" => {
                    storage_hc.set_local_storage(arg(1), arg(2));
                    Ok("true".into())
                }
                "remove" => {
                    storage_hc.remove_local_storage(arg(1));
                    Ok("true".into())
                }
                "clear" => {
                    storage_hc.clear_local_storage();
                    Ok("true".into())
                }
                _ => Ok("null".into()),
            },
            "softn.xdb" => match arg(0) {
                "load_all" => Ok(json!(storage_hc.load_xdb_collections()).to_string()),
                "save" => {
                    storage_hc.save_xdb_collection(arg(1), arg(2));
                    Ok("true".into())
                }
                _ => Ok("null".into()),
            },
            k if k.starts_with("app.sql.") => {
                if backend_hc.is_some() {
                    return Ok(json!({ "error": "Direct frontend SQLite disabled when backend is configured" }).to_string());
                }
                Ok(frontend_sql.call(k, args))
            }
            "softn.fetch" => {
                let method = arg(0);
                let raw_url = arg(1);
                let headers_json = arg(2);
                let body_text = arg(3);
                let credentials = arg(4);
                Ok(handle_softn_fetch(
                    &bundle_hc,
                    backend_hc.as_deref(),
                    &http_cache_hc,
                    &cookies_hc,
                    method,
                    raw_url,
                    headers_json,
                    body_text,
                    credentials,
                ))
            }
            "softn.device" => {
                let req_id: u64 = arg(0).parse().unwrap_or(0);
                let namespace = arg(1);
                let method = arg(2);
                let params: Value =
                    serde_json::from_str(arg(3)).unwrap_or_else(|_| json!({}));
                Ok(handle_softn_device(
                    &bundle_hc,
                    &host_info,
                    logical_w,
                    logical_h,
                    scale,
                    &pending_hc,
                    req_id,
                    namespace,
                    method,
                    params,
                ))
            }
            "softn.ws" => {
                let op = arg(0);
                let ws_id = arg(1);
                let a1 = arg(2);
                let a2 = arg(3);
                if let Some(ref be) = backend_hc {
                    let payload = match op {
                        "open" => {
                            let uri = extract_ws_uri(a1);
                            json!({ "__control": "ws_open", "wsId": ws_id, "uri": uri, "headers": [] })
                        }
                        "send" => json!({ "__control": "ws_send", "wsId": ws_id, "data": a1, "binary": false }),
                        "close" => json!({ "__control": "ws_close", "wsId": ws_id, "code": a1.parse::<u16>().unwrap_or(1000), "reason": a2 }),
                        _ => return Ok(json!({ "ok": false }).to_string()),
                    };
                    Ok(be.rust_control(payload).to_string())
                } else {
                    Ok(json!({ "ok": false, "accepted": false, "body": "No backend WebSocket handler" }).to_string())
                }
            }
            "navigate" => Ok(String::new()),
            _ => Ok(String::new()),
        }
    }));

    st.run_init()
        .map_err(|e| format!("Frontend runtime init error: {e}"))?;
    drain_js(&mut st);

    Ok(Some(st))
}

fn idb_file_path(dir: &Path, name: &str) -> PathBuf {
    let hex: String = name.bytes().map(|b| format!("{b:02x}")).collect();
    dir.join(format!("n{hex}.json"))
}

fn read_idb_file(dir: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(idb_file_path(dir, name)).ok()
}

fn write_idb_file(dir: &Path, name: &str, data: Option<&str>) {
    let path = idb_file_path(dir, name);
    match data {
        Some(text) => {
            let _ = std::fs::create_dir_all(dir);
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
        None => {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn list_idb_names(dir: &Path) -> String {
    let names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let fname = e.file_name();
            let hex = fname.to_str()?.strip_prefix('n')?.strip_suffix(".json")?;
            let bytes: Option<Vec<u8>> = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
                .collect();
            String::from_utf8(bytes?).ok()
        })
        .collect();
    json!(names).to_string()
}

fn probe_image_info(bundle: &SoftnBundle, url: &str) -> String {
    let raw = if url.starts_with("blob:") {
        http_cache::lookup_blob_bytes(url)
    } else if url.starts_with("data:") {
        data_url::DataUrl::process(url)
            .ok()
            .and_then(|p| p.decode_to_vec().ok())
            .map(|(b, _)| b)
    } else {
        let rel = url.strip_prefix(VIRTUAL_ORIGIN).unwrap_or(url);
        bundle.read_public_asset(rel).map(|(_, _, b)| b)
    };
    if let Some(bytes) = raw {
        if let Ok(reader) = image::ImageReader::new(std::io::Cursor::new(&bytes)).with_guessed_format() {
            if let Ok((w, h)) = reader.into_dimensions() {
                return json!({ "width": w, "height": h }).to_string();
            }
        }
        if let Some((w, h)) = zipp_renderer::svg_size(&bytes) {
            return json!({ "width": w, "height": h }).to_string();
        }
    }
    r#"{"width":0,"height":0}"#.to_string()
}

fn url_parts(input: &str, base: &str) -> Result<String, String> {
    let parsed = if base.is_empty() {
        url::Url::parse(input)
    } else {
        url::Url::parse(base).and_then(|b| b.join(input))
    }
    .map_err(|e| format!("TypeError: Invalid URL: {e}"))?;
    Ok(url_parts_json(&parsed))
}

fn url_parts_json(u: &url::Url) -> String {
    use url::quirks as q;
    json!({
        "href": q::href(u),
        "origin": q::origin(u),
        "protocol": q::protocol(u),
        "username": q::username(u),
        "password": q::password(u),
        "host": q::host(u),
        "hostname": q::hostname(u),
        "port": q::port(u),
        "pathname": q::pathname(u),
        "search": q::search(u),
        "hash": q::hash(u),
    })
    .to_string()
}

fn url_set(href: &str, part: &str, value: &str) -> Result<String, String> {
    use url::quirks as q;
    let mut u = url::Url::parse(href).map_err(|e| format!("TypeError: Invalid URL: {e}"))?;
    let _ = match part {
        "protocol" => q::set_protocol(&mut u, value),
        "username" => q::set_username(&mut u, value),
        "password" => q::set_password(&mut u, value),
        "host" => q::set_host(&mut u, value),
        "hostname" => q::set_hostname(&mut u, value),
        "port" => q::set_port(&mut u, value),
        "pathname" => Ok(q::set_pathname(&mut u, value)),
        "search" => Ok(q::set_search(&mut u, value)),
        "hash" => Ok(q::set_hash(&mut u, value)),
        _ => return Err(format!("unknown URL part {part:?}")),
    };
    Ok(url_parts_json(&u))
}

fn random_hex(n: usize) -> String {
    use sha2::{Digest, Sha256};
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let mut out = String::with_capacity(n * 2);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut produced = 0usize;
    while produced < n {
        let seq = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let mut h = Sha256::new();
        h.update(seq.to_le_bytes());
        h.update(nanos.to_le_bytes());
        h.update(std::process::id().to_le_bytes());
        let digest = h.finalize();
        for &b in digest.iter() {
            if produced >= n {
                break;
            }
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 15) as usize] as char);
            produced += 1;
        }
    }
    out
}

fn collect_ureq_headers<B>(resp: &ureq::http::Response<B>) -> Vec<(String, String)> {
    resp.headers()
        .iter()
        .filter_map(|(k, v)| Some((k.as_str().to_string(), v.to_str().ok()?.to_string())))
        .collect()
}

fn store_set_cookies(cookies: &MobileCookieJar, url_str: &str, headers: &[(String, String)]) {
    let set_cookies: Vec<String> = headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
        .map(|(_, v)| v.clone())
        .collect();
    if set_cookies.is_empty() {
        return;
    }
    let resolved = if url_str.starts_with("http://") || url_str.starts_with("https://") {
        url::Url::parse(url_str).ok()
    } else {
        let path = if url_str.starts_with('/') {
            url_str.to_string()
        } else {
            format!("/{url_str}")
        };
        url::Url::parse(&format!("{VIRTUAL_ORIGIN}{path}")).ok()
    };
    if let Some(u) = resolved {
        cookies.set_from_response(&u, &set_cookies);
    }
}

fn handle_softn_fetch_bytes(
    bundle: &SoftnBundle,
    backend: Option<&LocalContainerBackend>,
    http_cache: &MobileHttpCache,
    cookies: &MobileCookieJar,
    raw_url: &str,
    credentials: &str,
) -> (u16, String, Vec<u8>, Option<String>) {
    if raw_url.starts_with("blob:") {
        return match http_cache::lookup_blob(raw_url) {
            Some((mime, bytes)) => (200, mime, bytes, None),
            None => (404, String::new(), Vec::new(), Some(format!("Blob URL not found: {raw_url}"))),
        };
    }
    if raw_url.starts_with("data:") {
        return match data_url::DataUrl::process(raw_url) {
            Ok(parsed) => {
                let mime = parsed.mime_type().to_string();
                match parsed.decode_to_vec() {
                    Ok((bytes, _)) => (200, mime, bytes, None),
                    Err(e) => (400, String::new(), Vec::new(), Some(format!("Invalid data URL: {e:?}"))),
                }
            }
            Err(e) => (400, String::new(), Vec::new(), Some(format!("Invalid data URL: {e:?}"))),
        };
    }

    let send_cookies = credentials != "omit";
    let local_part = if let Some(rest) = raw_url.strip_prefix(VIRTUAL_ORIGIN) {
        Some(rest)
    } else if !raw_url.starts_with("http://") && !raw_url.starts_with("https://") {
        Some(raw_url)
    } else {
        None
    };

    if let Some(rel) = local_part {
        let (path_raw, query) = match rel.split_once('?') {
            Some((p, q)) => (p, q),
            None => (rel, ""),
        };
        let norm_path = if path_raw.starts_with('/') {
            path_raw.to_string()
        } else {
            format!("/{path_raw}")
        };
        if let Some(be) = backend {
            if be.should_route(&norm_path) {
                let mut hdrs = Vec::new();
                if send_cookies {
                    if let Ok(u) = url::Url::parse(&format!("{VIRTUAL_ORIGIN}{norm_path}")) {
                        if let Some(c) = cookies.cookie_header(&u) {
                            hdrs.push(("Cookie".into(), c));
                        }
                    }
                }
                let resp = be.handle_request_opts(
                    BackendHttpRequest {
                        method: "GET".into(),
                        path: norm_path.clone(),
                        query: query.to_string(),
                        headers: hdrs,
                        body: Vec::new(),
                    },
                    false,
                );
                store_set_cookies(cookies, &norm_path, &resp.headers);
                let ct = resp
                    .headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                    .map(|(_, v)| v.clone())
                    .unwrap_or_else(|| "application/octet-stream".into());
                return (resp.status, ct, resp.body, None);
            }
        }
        if let Some((_, mime, bytes)) = bundle.read_public_asset(&norm_path) {
            return (200, mime.to_string(), bytes, None);
        }
        return (404, "text/plain".into(), b"Not Found".to_vec(), None);
    }

    if let Err(err) = bundle.check_net_url(raw_url) {
        return (0, String::new(), Vec::new(), Some(err));
    }

    let revalidate = if send_cookies {
        match http_cache.lookup(raw_url) {
            CacheLookup::Fresh(e) => return (e.status, e.content_type, e.body.as_ref().clone(), None),
            CacheLookup::Revalidate(e) => Some(e),
            CacheLookup::Miss => None,
        }
    } else {
        None
    };

    let mut req = ureq::get(raw_url);
    if send_cookies {
        if let Ok(u) = url::Url::parse(raw_url) {
            if let Some(c) = cookies.cookie_header(&u) {
                req = req.header("Cookie", &c);
            }
        }
    }
    if let Some(ref prev) = revalidate {
        if let Some(ref etag) = prev.etag {
            req = req.header("If-None-Match", etag);
        }
        if let Some(ref lm) = prev.last_modified {
            req = req.header("If-Modified-Since", lm);
        }
    }
    match req.call() {
        Ok(mut resp) => {
            let status = resp.status().as_u16();
            let resp_hdrs = collect_ureq_headers(&resp);
            store_set_cookies(cookies, raw_url, &resp_hdrs);
            if status == 304 {
                if let Some(prev) = revalidate {
                    let refreshed = http_cache.revalidate_304(raw_url, prev, &resp_hdrs);
                    return (
                        refreshed.status,
                        refreshed.content_type,
                        refreshed.body.as_ref().clone(),
                        None,
                    );
                }
            }
            let ct = resp_hdrs
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| "application/octet-stream".into());
            let body = resp.body_mut().read_to_vec().unwrap_or_default();
            if send_cookies {
                http_cache.store(raw_url, status, &resp_hdrs, body.clone());
            }
            (status, ct, body, None)
        }
        Err(e) => (0, String::new(), Vec::new(), Some(format!("Network error: {e}"))),
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_softn_fetch(
    bundle: &SoftnBundle,
    backend: Option<&LocalContainerBackend>,
    http_cache: &MobileHttpCache,
    cookies: &MobileCookieJar,
    method: &str,
    raw_url: &str,
    headers_json: &str,
    body_text: &str,
    credentials: &str,
) -> String {
    if method.eq_ignore_ascii_case("GET") && (raw_url.starts_with("blob:") || raw_url.starts_with("data:")) {
        let (status, ct, bytes, err) = handle_softn_fetch_bytes(
            bundle,
            backend,
            http_cache,
            cookies,
            raw_url,
            credentials,
        );
        if let Some(error) = err {
            return json!({ "error": error }).to_string();
        }
        return format_fetch_response(raw_url, status, vec![("Content-Type".into(), ct)], &bytes);
    }

    let mut headers: Vec<(String, String)> = Vec::new();
    if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(headers_json) {
        for (k, v) in map {
            if let Some(s) = v.as_str() {
                if !k.eq_ignore_ascii_case("cookie") {
                    headers.push((k, s.to_string()));
                }
            }
        }
    }
    let send_cookies = credentials != "omit";
    let has_auth_hdr = headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("authorization"));

    // 1. Check backend interceptOrigins first
    if let Some(be) = backend {
        if let Some((path, query)) = be.matches_intercept_origin(raw_url) {
            if send_cookies {
                if let Ok(u) = url::Url::parse(raw_url) {
                    if let Some(c) = cookies.cookie_header(&u) {
                        headers.push(("Cookie".into(), c));
                    }
                }
            }
            let resp = be.handle_request_opts(
                BackendHttpRequest {
                    method: method.to_string(),
                    path,
                    query,
                    headers,
                    body: body_text.as_bytes().to_vec(),
                },
                false,
            );
            store_set_cookies(cookies, raw_url, &resp.headers);
            return format_fetch_response(raw_url, resp.status, resp.headers, &resp.body);
        }
    }

    // 2. Relative or virtual origin URL
    let local_part = if let Some(rest) = raw_url.strip_prefix(VIRTUAL_ORIGIN) {
        Some(rest)
    } else if !raw_url.starts_with("http://") && !raw_url.starts_with("https://") {
        Some(raw_url)
    } else {
        None
    };

    if let Some(rel) = local_part {
        let (path_raw, query) = match rel.split_once('?') {
            Some((p, q)) => (p, q),
            None => (rel, ""),
        };
        let norm_path = if path_raw.starts_with('/') {
            path_raw.to_string()
        } else {
            format!("/{path_raw}")
        };

        if let Some(be) = backend {
            if be.should_route(&norm_path) {
                if send_cookies {
                    if let Ok(u) = url::Url::parse(&format!("{VIRTUAL_ORIGIN}{norm_path}")) {
                        if let Some(c) = cookies.cookie_header(&u) {
                            headers.push(("Cookie".into(), c));
                        }
                    }
                }
                let resp = be.handle_request_opts(
                    BackendHttpRequest {
                        method: method.to_string(),
                        path: norm_path.clone(),
                        query: query.to_string(),
                        headers,
                        body: body_text.as_bytes().to_vec(),
                    },
                    false,
                );
                store_set_cookies(cookies, &norm_path, &resp.headers);
                return format_fetch_response(raw_url, resp.status, resp.headers, &resp.body);
            }
        }

        if let Some((_, mime, bytes)) = bundle.read_public_asset(&norm_path) {
            return format_fetch_response(
                raw_url,
                200,
                vec![("Content-Type".into(), mime.into())],
                &bytes,
            );
        }

        return format_fetch_response(
            raw_url,
            404,
            vec![("Content-Type".into(), "text/plain".into())],
            b"Not Found",
        );
    }

    // 3. External HTTP/HTTPS — strictly gated by permission.json `net`
    if let Err(err) = bundle.check_net_url(raw_url) {
        return json!({ "error": err }).to_string();
    }

    let upper_method = method.to_ascii_uppercase();
    let cacheable_get = upper_method == "GET" && !has_auth_hdr && send_cookies;
    let revalidate = if cacheable_get {
        match http_cache.lookup(raw_url) {
            CacheLookup::Fresh(e) => {
                return format_fetch_response(
                    raw_url,
                    e.status,
                    vec![("Content-Type".into(), e.content_type)],
                    &e.body,
                );
            }
            CacheLookup::Revalidate(e) => Some(e),
            CacheLookup::Miss => None,
        }
    } else {
        None
    };

    let cookie_val = if send_cookies {
        url::Url::parse(raw_url)
            .ok()
            .and_then(|u| cookies.cookie_header(&u))
    } else {
        None
    };

    macro_rules! apply_headers {
        ($req:expr) => {{
            let mut r = $req;
            for (k, v) in &headers {
                r = r.header(k.as_str(), v.as_str());
            }
            if let Some(ref c) = cookie_val {
                r = r.header("Cookie", c.as_str());
            }
            if let Some(ref prev) = revalidate {
                if let Some(ref etag) = prev.etag {
                    r = r.header("If-None-Match", etag.as_str());
                }
                if let Some(ref lm) = prev.last_modified {
                    r = r.header("If-Modified-Since", lm.as_str());
                }
            }
            r
        }};
    }

    let req = match upper_method.as_str() {
        "POST" => apply_headers!(ureq::post(raw_url)).send(body_text),
        "PUT" => apply_headers!(ureq::put(raw_url)).send(body_text),
        "PATCH" => apply_headers!(ureq::patch(raw_url)).send(body_text),
        "DELETE" => apply_headers!(ureq::delete(raw_url)).call(),
        "HEAD" => apply_headers!(ureq::head(raw_url)).call(),
        _ => apply_headers!(ureq::get(raw_url)).call(),
    };
    match req {
        Ok(mut resp) => {
            let status = resp.status().as_u16();
            let resp_hdrs = collect_ureq_headers(&resp);
            store_set_cookies(cookies, raw_url, &resp_hdrs);
            if status == 304 {
                if let Some(prev) = revalidate {
                    let refreshed = http_cache.revalidate_304(raw_url, prev, &resp_hdrs);
                    return format_fetch_response(
                        raw_url,
                        refreshed.status,
                        resp_hdrs,
                        &refreshed.body,
                    );
                }
            }
            let body = resp.body_mut().read_to_vec().unwrap_or_default();
            if cacheable_get {
                http_cache.store(raw_url, status, &resp_hdrs, body.clone());
            } else if matches!(upper_method.as_str(), "POST" | "PUT" | "PATCH" | "DELETE") {
                http_cache.invalidate(raw_url);
            }
            format_fetch_response(raw_url, status, resp_hdrs, &body)
        }
        Err(e) => json!({ "error": format!("Network error: {e}") }).to_string(),
    }
}

fn format_fetch_response(
    url: &str,
    status: u16,
    headers: Vec<(String, String)>,
    body: &[u8],
) -> String {
    let hdr_pairs: Vec<[String; 2]> = headers
        .into_iter()
        .filter(|(k, _)| !k.eq_ignore_ascii_case("set-cookie"))
        .map(|(k, v)| [k, v])
        .collect();
    json!({
        "url": url,
        "status": status,
        "headers": hdr_pairs,
        "body": String::from_utf8_lossy(body),
    })
    .to_string()
}

#[allow(clippy::too_many_arguments)]
fn handle_softn_device(
    bundle: &SoftnBundle,
    host_info: &HostDeviceInfo,
    logical_w: u32,
    logical_h: u32,
    scale: f32,
    pending_calls: &Arc<Mutex<Vec<PendingDeviceRequest>>>,
    req_id: u64,
    namespace: &str,
    method: &str,
    params: Value,
) -> String {
    // 1. Non-sensitive device metadata (always permitted)
    if namespace == "device" && method == "getInfo" {
        return json!({
            "ok": true,
            "result": {
                "platform": host_info.platform,
                "engine": host_info.engine,
                "osVersion": host_info.os_version,
                "model": host_info.model,
                "screenWidth": logical_w,
                "screenHeight": logical_h,
                "devicePixelRatio": scale,
                "capabilities": host_info.capabilities,
            }
        })
        .to_string();
    }

    // 2. Feature detection (`isAvailable`) never touches user hardware
    if method == "isAvailable" && namespace != "device" {
        let cap_key = match namespace {
            "sensors" => "accel",
            other => other,
        };
        let available = host_info
            .capabilities
            .get(namespace)
            .or_else(|| host_info.capabilities.get(cap_key))
            .copied()
            .unwrap_or(false);
        let permitted = bundle.has_capability(cap_key);
        return json!({
            "ok": true,
            "result": { "available": available, "permitted": permitted }
        })
        .to_string();
    }

    // 3. Enforce capability declaration in permission.json natively in Rust
    let required_cap = match namespace {
        "device" | "permissions" => None,
        "camera" => Some("camera"),
        "bluetooth" => Some("bluetooth"),
        "location" => Some("location"),
        "haptics" => Some("haptics"),
        "notifications" => Some("notifications"),
        "files" => Some("files"),
        "sensors" => Some("accel"),
        "mic" => Some("mic"),
        other => {
            return json!({ "ok": false, "error": format!("Unknown device namespace: {other}") })
                .to_string();
        }
    };

    // Permission query checks bundle permission.json FIRST (denied immediately if undeclared),
    // then delegates to Android OS permission state check via DeviceBridge.
    if namespace == "permissions" && method == "query" {
        let cap = params
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        let mapped = match cap.as_str() {
            "geolocation" => "location",
            "microphone" => "mic",
            "accelerometer" | "gyroscope" | "magnetometer" => "accel",
            other => other,
        };
        if !mapped.is_empty() && !matches!(mapped, "clipboard-read" | "clipboard-write") && !bundle.has_capability(mapped) {
            return json!({
                "ok": true,
                "result": { "state": "denied", "reason": "not_declared_in_permission_json" }
            })
            .to_string();
        }
    }

    if let Some(cap) = required_cap {
        if !bundle.has_capability(cap) {
            return json!({
                "ok": false,
                "error": format!(
                    "{cap} access is not permitted. Declare \"{cap}\": {{ \"enabled\": true }} in permission.json."
                )
            })
            .to_string();
        }
    }

    // 4. Enforce BLE service UUID allowlist natively in Rust
    if namespace == "bluetooth" {
        if let Some(allowed) = bundle.allowed_ble_services() {
            if method == "scan" {
                if let Some(arr) = params.get("services").and_then(|v| v.as_array()) {
                    for s in arr {
                        if let Some(u) = s.as_str() {
                            if !allowed.contains(&u.trim().to_ascii_lowercase()) {
                                return json!({
                                    "ok": false,
                                    "error": format!("Bluetooth service '{u}' is not in permission.json bluetooth.services allowlist")
                                })
                                .to_string();
                            }
                        }
                    }
                }
            } else if method == "read" || method == "write" {
                let svc = params
                    .get("serviceUuid")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase();
                if !allowed.contains(&svc) {
                    return json!({
                        "ok": false,
                        "error": format!("Bluetooth service '{svc}' is not allowed by permission.json")
                    })
                    .to_string();
                }
            }
        }
    }

    // 5. Queue native hardware call for Android Kotlin bridge
    if let Ok(mut queue) = pending_calls.lock() {
        queue.push(PendingDeviceRequest {
            id: req_id,
            namespace: namespace.to_string(),
            method: method.to_string(),
            params,
        });
    }
    json!({ "pending": true }).to_string()
}

fn collect_head_css(raw_html: &str, bundle: &SoftnBundle) -> String {
    let mut css = String::new();
    let lower = raw_html.to_ascii_lowercase();

    // 1. Inline <link rel="stylesheet" href="...">
    let mut pos = 0usize;
    while let Some(rel_idx) = lower[pos..].find("<link") {
        let abs = pos + rel_idx;
        let Some(end_rel) = raw_html[abs..].find('>') else {
            break;
        };
        let tag = &raw_html[abs..abs + end_rel + 1];
        let tag_lower = tag.to_ascii_lowercase();
        if tag_lower.contains("stylesheet") {
            if let Some(href) = extract_attr(tag, "href") {
                if href == "softn:ui.css" || href == "/runtime/ui.css" {
                    css.push_str(SOFTN_UI_CSS);
                    css.push('\n');
                } else if let Some(vfs_path) =
                    resolve_vfs_url(&href, &bundle.entry_path, &bundle.frontend_root)
                {
                    if let Some(text) = bundle.read_text(&vfs_path) {
                        css.push_str(&text);
                        css.push('\n');
                    }
                } else if let Some((_, _, bytes)) = bundle.read_public_asset(&href) {
                    css.push_str(&String::from_utf8_lossy(&bytes));
                    css.push('\n');
                }
            }
        }
        pos = abs + end_rel + 1;
    }

    // 2. Inline <style>...</style> blocks
    let mut spos = 0usize;
    while let Some(rel_idx) = lower[spos..].find("<style") {
        let abs = spos + rel_idx;
        let Some(open_end) = raw_html[abs..].find('>') else {
            break;
        };
        let content_start = abs + open_end + 1;
        let Some(close_rel) = lower[content_start..].find("</style") else {
            break;
        };
        css.push_str(&raw_html[content_start..content_start + close_rel]);
        css.push('\n');
        spos = content_start + close_rel + 8;
    }

    css
}

fn extract_ws_uri(raw_url: &str) -> String {
    if let Some(rest) = raw_url
        .strip_prefix("wss://")
        .or_else(|| raw_url.strip_prefix("ws://"))
        .or_else(|| raw_url.strip_prefix("https://"))
        .or_else(|| raw_url.strip_prefix("http://"))
    {
        if let Some(idx) = rest.find('/') {
            return rest[idx..].to_string();
        }
        return "/".to_string();
    }
    if raw_url.starts_with('/') {
        raw_url.to_string()
    } else {
        format!("/{raw_url}")
    }
}

fn extract_html_open_attrs(raw_html: &str) -> Vec<(String, String)> {
    let lower = raw_html.to_ascii_lowercase();
    let Some(idx) = lower.find("<html") else {
        return Vec::new();
    };
    let Some(end) = raw_html[idx..].find('>') else {
        return Vec::new();
    };
    let tag = &raw_html[idx + 5..idx + end];
    let bytes = tag.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] == b'/' {
            break;
        }
        let ns = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'=' {
            i += 1;
        }
        let name = tag[ns..i].trim().to_ascii_lowercase();
        if name.is_empty() {
            break;
        }
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut val = String::new();
        if i < bytes.len() && bytes[i] == b'=' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
                let q = bytes[i];
                i += 1;
                let vs = i;
                while i < bytes.len() && bytes[i] != q {
                    i += 1;
                }
                val = tag[vs..i].to_string();
                if i < bytes.len() {
                    i += 1;
                }
            } else {
                let vs = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                val = tag[vs..i].to_string();
            }
        }
        out.push((name, val));
    }
    out
}

fn extract_body_inner_html(raw_html: &str) -> String {
    let lower = raw_html.to_ascii_lowercase();
    let Some(body_idx) = lower.find("<body") else {
        return strip_script_tags(raw_html);
    };
    let Some(open_end) = raw_html[body_idx..].find('>') else {
        return String::new();
    };
    let start = body_idx + open_end + 1;
    let end = lower[start..]
        .rfind("</body>")
        .map(|i| start + i)
        .unwrap_or(raw_html.len());
    strip_script_tags(&raw_html[start..end])
}

fn strip_script_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let lower = html.to_ascii_lowercase();
    let mut pos = 0usize;
    while let Some(idx) = lower[pos..].find("<script") {
        let abs = pos + idx;
        out.push_str(&html[pos..abs]);
        if let Some(close) = lower[abs..].find("</script>") {
            pos = abs + close + 9;
        } else {
            pos = html.len();
            break;
        }
    }
    if pos < html.len() {
        out.push_str(&html[pos..]);
    }
    out
}

fn extract_scripts(raw_html: &str, bundle: &SoftnBundle) -> (Vec<String>, Vec<(String, String)>) {
    let mut classic = Vec::new();
    let mut modules = Vec::new();
    let lower = raw_html.to_ascii_lowercase();
    let mut pos = 0usize;
    let mut inline_seq = 0usize;

    while let Some(rel_idx) = lower[pos..].find("<script") {
        let abs = pos + rel_idx;
        let Some(open_end) = raw_html[abs..].find('>') else {
            break;
        };
        let tag = &raw_html[abs..abs + open_end + 1];
        let body_start = abs + open_end + 1;
        let close_rel = lower[body_start..].find("</script");
        let ty = extract_attr(tag, "type")
            .unwrap_or_default()
            .to_ascii_lowercase();
        let is_js = ty.is_empty()
            || ty == "module"
            || ty == "text/javascript"
            || ty == "application/javascript"
            || ty == "text/typescript";

        if is_js {
            if let Some(src) = extract_attr(tag, "src") {
                if let Some(vfs_path) =
                    resolve_vfs_url(&src, &bundle.entry_path, &bundle.frontend_root)
                {
                    if let Some(code) = bundle.read_text(&vfs_path) {
                        if ty == "module" {
                            modules.push((format!("{VIRTUAL_ORIGIN}/{vfs_path}"), code));
                        } else {
                            classic.push(code);
                        }
                    }
                } else if let Some((path, _, bytes)) = bundle.read_public_asset(&src) {
                    let code = String::from_utf8_lossy(&bytes).into_owned();
                    if ty == "module" {
                        modules.push((format!("{VIRTUAL_ORIGIN}/{path}"), code));
                    } else {
                        classic.push(code);
                    }
                }
            } else if let Some(cr) = close_rel {
                let code = raw_html[body_start..body_start + cr].trim();
                if !code.is_empty() {
                    inline_seq += 1;
                    if ty == "module" {
                        let base_name = if bundle.entry_path.is_empty() {
                            format!("inline_{inline_seq}.js")
                        } else {
                            format!("{}#inline_{inline_seq}.js", bundle.entry_path)
                        };
                        modules.push((format!("{VIRTUAL_ORIGIN}/{base_name}"), code.to_string()));
                    } else {
                        classic.push(code.to_string());
                    }
                }
            }
        }

        if let Some(cr) = close_rel {
            pos = body_start + cr + 9;
        } else {
            pos = body_start;
        }
    }

    (classic, modules)
}

fn extract_attr(tag: &str, attr_name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let needle = format!("{attr_name}=");
    let mut search_from = 0usize;
    while let Some(idx) = lower[search_from..].find(&needle) {
        let abs = search_from + idx;
        // Verify attribute boundary (preceded by whitespace)
        if abs > 0 {
            let prev = tag.as_bytes()[abs - 1];
            if !prev.is_ascii_whitespace() {
                search_from = abs + needle.len();
                continue;
            }
        }
        let after = &tag[abs + needle.len()..];
        let trimmed = after.trim_start();
        if let Some(rest) = trimmed.strip_prefix('"') {
            return rest.split('"').next().map(|s| s.to_string());
        } else if let Some(rest) = trimmed.strip_prefix('\'') {
            return rest.split('\'').next().map(|s| s.to_string());
        } else {
            return trimmed
                .split(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/')
                .next()
                .map(|s| s.to_string());
        }
    }
    None
}
