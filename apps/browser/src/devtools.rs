//! Chrome-style Developer Tools panel for Zipp Browser.
//!
//! Upgrades the `#__zipp_console` drawer into a multi-tab inspector with:
//! - **Elements** (`Ctrl+Shift+I` / `Ctrl+Shift+C`): live element picker + DOM
//!   tree preview, attributes, box dimensions, outer HTML, and Stylo computed styles.
//! - **Source** (`Ctrl+U`): line-numbered Original HTML vs. live Rendered DOM.
//! - **Scripts**: every classic script, ES module (including dynamically loaded
//!   chunks), inline script, import map, Web Worker, and embedded iframe document,
//!   with full source viewer and copy support.
//! - **Console** (`F12`): live logs, errors, warnings, clipboard copy, and JS REPL.

use std::cell::Cell;
use std::fmt::Write;
use std::sync::Mutex;

use zipp_dom::{BaseDocument, LocalName};

use crate::console;

/// Height of the open DevTools drawer in CSS pixels.
pub(crate) const PANEL_HEIGHT: u32 = 320;

const MAX_SCRIPTS: usize = 300;
const MAX_DISPLAY_BYTES: usize = 96 * 1024;
const MAX_DISPLAY_LINES: usize = 1200;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Tab {
    Console,
    Elements,
    Source,
    Scripts,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SourceMode {
    Original,
    Rendered,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ScriptKind {
    Classic,
    Module,
    Inline,
    ImportMap,
    Worker,
    Iframe,
}

impl ScriptKind {
    fn badge(self) -> (&'static str, &'static str, &'static str) {
        match self {
            ScriptKind::Module => ("MOD", "#c4b5fd", "rgba(139,92,246,0.18)"),
            ScriptKind::Classic => ("EXT", "#93c5fd", "rgba(59,130,246,0.18)"),
            ScriptKind::Inline => ("INL", "#fcd34d", "rgba(245,158,11,0.18)"),
            ScriptKind::ImportMap => ("MAP", "#5eead4", "rgba(20,184,166,0.18)"),
            ScriptKind::Worker => ("WRK", "#86efac", "rgba(34,197,94,0.18)"),
            ScriptKind::Iframe => ("IFR", "#f9a8d4", "rgba(236,72,153,0.18)"),
        }
    }

    fn label(self) -> &'static str {
        match self {
            ScriptKind::Module => "ES Module",
            ScriptKind::Classic => "Classic Script",
            ScriptKind::Inline => "Inline Script",
            ScriptKind::ImportMap => "Import Map",
            ScriptKind::Worker => "Web Worker",
            ScriptKind::Iframe => "Embedded Iframe",
        }
    }
}

#[derive(Clone)]
struct LoadedScript {
    kind: ScriptKind,
    url: String,
    label: String,
    source: String,
    bytes: usize,
    error: Option<String>,
}

#[derive(Clone)]
struct InspectedElement {
    node_id: usize,
    tag: String,
    id_attr: String,
    classes: String,
    breadcrumb: Vec<String>,
    rect: (f64, f64, f64, f64),
    attrs: Vec<(String, String)>,
    computed: Vec<(&'static str, String)>,
    outer_html: String,
    child_count: usize,
}

struct State {
    tab: Tab,
    source_mode: SourceMode,
    picking: bool,
    page_url: String,
    raw_html: String,
    rendered_html: String,
    scripts: Vec<LoadedScript>,
    selected_script: usize,
    inspected: Option<InspectedElement>,
    generation: u64,
}

static STATE: Mutex<State> = Mutex::new(State {
    tab: Tab::Console,
    source_mode: SourceMode::Original,
    picking: false,
    page_url: String::new(),
    raw_html: String::new(),
    rendered_html: String::new(),
    scripts: Vec::new(),
    selected_script: 0,
    inspected: None,
    generation: 0,
});

thread_local! {
    /// Only record DevTools page/script state on the window thread (and its workers),
    /// never during background link prerendering.
    static RECORDS: Cell<bool> = const { Cell::new(false) };
    /// Temporarily suppress page-level overrides while rendering an embedded iframe.
    static IN_IFRAME: Cell<bool> = const { Cell::new(false) };
}

/// Enable DevTools recording on this thread (window thread & workers).
pub(crate) fn record_this_thread() {
    RECORDS.set(true);
}

/// RAII guard marking that the current thread is rendering an `<iframe>`
/// sub-document so it doesn't overwrite the parent page's HTML source.
pub(crate) struct IframeScope {
    prev: bool,
}

impl IframeScope {
    pub(crate) fn enter() -> Self {
        let prev = IN_IFRAME.replace(true);
        Self { prev }
    }
}

impl Drop for IframeScope {
    fn drop(&mut self) {
        IN_IFRAME.set(self.prev);
    }
}

/// Reset recorded page data on a new top-level navigation.
pub(crate) fn new_page(url: &str, raw_html: &str) {
    if !RECORDS.get() || IN_IFRAME.get() {
        return;
    }
    let mut st = STATE.lock().unwrap_or_else(|e| e.into_inner());
    st.page_url = url.to_string();
    st.raw_html = raw_html.to_string();
    st.rendered_html.clear();
    st.scripts.clear();
    st.selected_script = 0;
    st.inspected = None;
    st.picking = false;
    st.generation = st.generation.wrapping_add(1);
}

/// Store the post-JS rendered HTML for the Source tab's "Rendered" view.
pub(crate) fn set_rendered_html(html: &str) {
    if !RECORDS.get() || IN_IFRAME.get() {
        return;
    }
    let mut st = STATE.lock().unwrap_or_else(|e| e.into_inner());
    st.rendered_html = html.to_string();
    st.generation = st.generation.wrapping_add(1);
}

/// Record a loaded script, ES module, import map, worker, or iframe document.
pub(crate) fn record_script(kind: ScriptKind, url: &str, source: String, error: Option<String>) {
    if !RECORDS.get() {
        return;
    }
    let mut st = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if st.scripts.len() >= MAX_SCRIPTS {
        return;
    }
    if let Some(existing) = st.scripts.iter_mut().find(|s| s.kind == kind && s.url == url) {
        if existing.source.is_empty() && !source.is_empty() {
            existing.bytes = source.len();
            existing.source = source;
        }
        if error.is_some() {
            existing.error = error;
        }
        st.generation = st.generation.wrapping_add(1);
        return;
    }
    let bytes = source.len();
    let label = short_script_label(kind, url, st.scripts.len() + 1);
    st.scripts.push(LoadedScript {
        kind,
        url: url.to_string(),
        label,
        source,
        bytes,
        error,
    });
    st.generation = st.generation.wrapping_add(1);
}

fn short_script_label(kind: ScriptKind, url: &str, idx: usize) -> String {
    if let Some((_, frag)) = url.split_once('#')
        && (frag.starts_with("inline") || frag.starts_with("importmap"))
    {
        return format!("{} #{idx}", kind.label());
    }
    if let Ok(parsed) = url::Url::parse(url) {
        let seg = parsed
            .path_segments()
            .and_then(|mut s| s.rfind(|p| !p.is_empty()))
            .unwrap_or("");
        if !seg.is_empty() {
            return seg.to_string();
        }
        if let Some(host) = parsed.host_str() {
            return host.to_string();
        }
    }
    let tail = url.rsplit('/').next().unwrap_or(url);
    if tail.is_empty() {
        format!("script #{idx}")
    } else {
        tail.chars().take(40).collect()
    }
}

pub(crate) fn active_tab() -> Tab {
    STATE.lock().unwrap_or_else(|e| e.into_inner()).tab
}

pub(crate) fn is_picking() -> bool {
    STATE.lock().unwrap_or_else(|e| e.into_inner()).picking
}

pub(crate) fn set_picking(on: bool) {
    let mut st = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if st.picking != on {
        st.picking = on;
        st.generation = st.generation.wrapping_add(1);
    }
}

pub(crate) fn toggle_picking() -> bool {
    let mut st = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let next = !st.picking;
    st.picking = next;
    if next {
        st.tab = Tab::Elements;
        console::set_open(true);
    }
    st.generation = st.generation.wrapping_add(1);
    next
}

pub(crate) fn set_tab(tab: Tab) {
    let mut st = STATE.lock().unwrap_or_else(|e| e.into_inner());
    st.tab = tab;
    if tab != Tab::Elements {
        st.picking = false;
    }
    console::set_open(true);
    st.generation = st.generation.wrapping_add(1);
}

/// Clicking a toolbar button (`Inspect` or `Console`) or shortcut (`Ctrl+Shift+I`):
/// - if closed: opens on `tab` (returns `true`)
/// - if open on a different tab: switches to `tab` (returns `true`)
/// - if open on the same tab: closes the drawer (returns `false`)
pub(crate) fn toggle_tab(tab: Tab) -> bool {
    let mut st = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let now_open = if !console::is_open() {
        st.tab = tab;
        console::set_open(true);
        true
    } else if st.tab != tab {
        st.tab = tab;
        if tab != Tab::Elements {
            st.picking = false;
        }
        true
    } else {
        st.picking = false;
        console::set_open(false);
        false
    };
    st.generation = st.generation.wrapping_add(1);
    now_open
}

pub(crate) fn set_source_mode(mode: SourceMode) {
    let mut st = STATE.lock().unwrap_or_else(|e| e.into_inner());
    st.source_mode = mode;
    st.generation = st.generation.wrapping_add(1);
}

pub(crate) fn select_script(index: usize) {
    let mut st = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if index < st.scripts.len() {
        st.selected_script = index;
        st.tab = Tab::Scripts;
        console::set_open(true);
        st.generation = st.generation.wrapping_add(1);
    }
}

pub(crate) fn generation() -> u64 {
    let dt = STATE.lock().unwrap_or_else(|e| e.into_inner()).generation;
    dt.wrapping_add(console::generation().wrapping_mul(31))
}

/// Inspect a concrete DOM node from `doc` (from Element Picker click or tree click).
pub(crate) fn inspect_node(doc: &BaseDocument, mut node_id: usize) {
    // Walk up from text/comment nodes to the enclosing element.
    while let Some(node) = doc.get_node(node_id) {
        if node.element_data().is_some() {
            break;
        }
        match node.parent {
            Some(parent) => node_id = parent,
            None => return,
        }
    }
    let Some(node) = doc.get_node(node_id) else {
        return;
    };
    let Some(el) = node.element_data() else {
        return;
    };

    let tag = el.name.local.to_string();
    let id_attr = el.attr(LocalName::from("id")).unwrap_or("").to_string();
    let classes = el.attr(LocalName::from("class")).unwrap_or("").to_string();

    // Build breadcrumb path from root down to this node.
    let mut breadcrumb = Vec::new();
    let mut cur = Some(node_id);
    while let Some(cid) = cur {
        if let Some(n) = doc.get_node(cid) {
            if let Some(cel) = n.element_data() {
                let mut seg = cel.name.local.to_string();
                if let Some(idv) = cel.attr(LocalName::from("id"))
                    && !idv.is_empty()
                {
                    seg.push('#');
                    seg.push_str(idv);
                } else if let Some(clsv) = cel.attr(LocalName::from("class"))
                    && let Some(first) = clsv.split_whitespace().next()
                {
                    seg.push('.');
                    seg.push_str(first);
                }
                breadcrumb.push(seg);
            }
            cur = n.parent;
        } else {
            break;
        }
    }
    breadcrumb.reverse();

    let (x, y, w, h) = node.transformed_border_rect();
    let attrs: Vec<(String, String)> = el
        .attrs
        .iter()
        .filter(|a| a.name.local.as_ref() != "data-flid" && a.name.local.as_ref() != "data-zsr")
        .map(|a| (a.name.local.to_string(), a.value.clone()))
        .collect();

    const PROPS: &[&str] = &[
        "display",
        "position",
        "width",
        "height",
        "color",
        "background-color",
        "font-family",
        "font-size",
        "font-weight",
        "line-height",
        "margin-top",
        "margin-right",
        "margin-bottom",
        "margin-left",
        "padding-top",
        "padding-right",
        "padding-bottom",
        "padding-left",
        "border-top-width",
        "border-radius",
        "flex-direction",
        "justify-content",
        "align-items",
        "gap",
        "opacity",
        "z-index",
        "overflow-x",
        "overflow-y",
        "visibility",
    ];
    let mut computed = Vec::new();
    for &prop in PROPS {
        if let Some(val) = node.computed_style_value(prop)
            && !val.is_empty()
        {
            computed.push((prop, val));
        }
    }

    let outer_html = node.outer_html();
    let child_count = node.children.len();

    let mut st = STATE.lock().unwrap_or_else(|e| e.into_inner());
    st.inspected = Some(InspectedElement {
        node_id,
        tag,
        id_attr,
        classes,
        breadcrumb,
        rect: (x, y, w, h),
        attrs,
        computed,
        outer_html,
        child_count,
    });
    st.picking = false;
    st.tab = Tab::Elements;
    console::set_open(true);
    st.generation = st.generation.wrapping_add(1);
}

/// Plain text copied when the user clicks the DevTools Copy button.
pub(crate) fn copy_text() -> String {
    let st = STATE.lock().unwrap_or_else(|e| e.into_inner());
    match st.tab {
        Tab::Console => {
            drop(st);
            console::plain_text()
        }
        Tab::Elements => match &st.inspected {
            Some(el) => el.outer_html.clone(),
            None => st.rendered_html.clone().if_empty_then(&st.raw_html),
        },
        Tab::Source => match st.source_mode {
            SourceMode::Original => st.raw_html.clone(),
            SourceMode::Rendered => {
                if st.rendered_html.is_empty() {
                    st.raw_html.clone()
                } else {
                    st.rendered_html.clone()
                }
            }
        },
        Tab::Scripts => st
            .scripts
            .get(st.selected_script)
            .map(|s| s.source.clone())
            .unwrap_or_default(),
    }
}

trait StrFallback {
    fn if_empty_then(self, fallback: &str) -> String;
}
impl StrFallback for String {
    fn if_empty_then(self, fallback: &str) -> String {
        if self.is_empty() {
            fallback.to_string()
        } else {
            self
        }
    }
}

/// Label for the DevTools copy button based on active tab & copied state.
pub(crate) fn copy_button_inner(copied: bool) -> String {
    let tab = active_tab();
    if copied {
        return r##"<svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="#4ade80" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg><span style="color:#4ade80">Copied!</span>"##.into();
    }
    let label = match tab {
        Tab::Console => "Copy",
        Tab::Elements => "Copy HTML",
        Tab::Source => "Copy Source",
        Tab::Scripts => "Copy Script",
    };
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/></svg><span>{label}</span>"##
    )
}

fn escape_html(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
}

fn format_bytes(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Render the unified DevTools top bar (tabs, picker toggle, context actions, Copy, Close).
pub(crate) fn header_html(token: &str, copied: bool) -> String {
    let (tab, picking, source_mode, script_count) = {
        let st = STATE.lock().unwrap_or_else(|e| e.into_inner());
        (st.tab, st.picking, st.source_mode, st.scripts.len())
    };
    let counts = console::counts_text();
    let copy_inner = copy_button_inner(copied);

    let tab_style = |active: bool| -> &'static str {
        if active {
            "display:inline-flex;align-items:center;gap:5px;padding:4px 10px;border-radius:6px;background:rgba(59,130,246,0.2);border:1px solid rgba(96,165,250,0.4);color:#f8fafc;text-decoration:none;font-weight:600"
        } else {
            "display:inline-flex;align-items:center;gap:5px;padding:4px 10px;border-radius:6px;background:transparent;border:1px solid transparent;color:#94a3b8;text-decoration:none;font-weight:500"
        }
    };

    let pick_style = if picking {
        "display:inline-flex;align-items:center;gap:5px;padding:4px 9px;border-radius:6px;background:#2563eb;border:1px solid #60a5fa;color:#ffffff;text-decoration:none;font-weight:600"
    } else {
        "display:inline-flex;align-items:center;gap:5px;padding:4px 9px;border-radius:6px;background:rgba(148,163,184,0.12);border:1px solid rgba(148,163,184,0.24);color:#cbd5e1;text-decoration:none;font-weight:500"
    };

    let mut out = String::with_capacity(2048);
    // Element Picker toggle
    let _ = write!(
        out,
        r##"<a id="__zipp_dt_pick" href="zipp-devtools:pick" title="Pick an element on the page to inspect (Ctrl+Shift+C)" style="{pick_style}"><svg xmlns="http://www.w3.org/2000/svg" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"><path d="M5 3a2 2 0 0 0-2 2"/><path d="M19 3a2 2 0 0 1 2 2"/><path d="M5 21a2 2 0 0 1-2-2"/><path d="M9 3h1"/><path d="M14 3h1"/><path d="M3 9v1"/><path d="M3 14v1"/><path d="m12 12 9 3-4 2-2 4-3-9Z"/></svg><span>{}</span></a>"##,
        if picking { "Picking..." } else { "Pick" }
    );

    // Tab pills
    let _ = write!(
        out,
        r##"<a id="__zipp_dt_tab_elements" href="zipp-devtools:tab/elements" style="{}">Elements</a>"##,
        tab_style(tab == Tab::Elements)
    );
    let _ = write!(
        out,
        r##"<a id="__zipp_dt_tab_source" href="zipp-devtools:tab/source" style="{}">Source</a>"##,
        tab_style(tab == Tab::Source)
    );
    let _ = write!(
        out,
        r##"<a id="__zipp_dt_tab_scripts" href="zipp-devtools:tab/scripts" style="{}">Scripts <span style="padding:0 5px;border-radius:999px;background:rgba(148,163,184,0.18);color:#cbd5e1;font-size:10px">{script_count}</span></a>"##,
        tab_style(tab == Tab::Scripts)
    );
    let _ = write!(
        out,
        r##"<a id="__zipp_dt_tab_console" href="zipp-devtools:tab/console" style="{}">Console</a><span id="__zipp_console_counts" style="color:#94a3b8;font-size:11px">{counts}</span>"##,
        tab_style(tab == Tab::Console)
    );

    out.push_str(r#"<span style="flex:1"></span>"#);

    // Contextual right-hand controls
    match tab {
        Tab::Source => {
            let orig_style = if source_mode == SourceMode::Original {
                "padding:2px 8px;border-radius:5px;background:#1e3a8a;color:#dbeafe;text-decoration:none;font-weight:600;font-size:11px"
            } else {
                "padding:2px 8px;border-radius:5px;background:transparent;color:#94a3b8;text-decoration:none;font-size:11px"
            };
            let live_style = if source_mode == SourceMode::Rendered {
                "padding:2px 8px;border-radius:5px;background:#1e3a8a;color:#dbeafe;text-decoration:none;font-weight:600;font-size:11px"
            } else {
                "padding:2px 8px;border-radius:5px;background:transparent;color:#94a3b8;text-decoration:none;font-size:11px"
            };
            let _ = write!(
                out,
                r##"<a id="__zipp_dt_src_orig" href="zipp-devtools:source/original" style="{orig_style}">Original HTML</a><a id="__zipp_dt_src_live" href="zipp-devtools:source/rendered" style="{live_style}">Rendered DOM</a>"##
            );
        }
        Tab::Console => {
            let _ = write!(
                out,
                r##"<a href="zipp-nav:settings?__zipp_chrome={token}" style="color:#60a5fa;text-decoration:none;font-weight:500">Properties</a><a href="zipp-nav:history?__zipp_chrome={token}" style="color:#60a5fa;text-decoration:none;font-weight:500">History</a><a href="zipp-nav:clear-cache?__zipp_chrome={token}" style="color:#60a5fa;text-decoration:none;font-weight:500">Clear Cache</a><a href="zipp-nav:clear-data?__zipp_chrome={token}" style="color:#f87171;text-decoration:none;font-weight:500">Clear Site Data</a><a href="zipp-console:clear" id="__zipp_console_clear" style="color:#cbd5e1;text-decoration:none;font-weight:500">Clear</a>"##
            );
        }
        Tab::Elements | Tab::Scripts => {}
    }

    let _ = write!(
        out,
        r##"<a href="zipp-console:copy" id="__zipp_console_copy" title="Copy to clipboard" style="display:inline-flex;align-items:center;gap:5px;padding:2px 9px;border-radius:6px;background:rgba(96,165,250,0.14);border:1px solid rgba(96,165,250,0.28);color:#93c5fd;text-decoration:none;font-weight:600">{copy_inner}</a><a href="zipp-console:toggle" id="__zipp_console_close" style="color:#cbd5e1;text-decoration:none;font-weight:500">Close</a>"##
    );
    out
}

/// Render the active tab's body content inside `#__zipp_console_lines`.
pub(crate) fn body_html(doc: Option<&BaseDocument>) -> String {
    let snapshot = STATE.lock().unwrap_or_else(|e| e.into_inner()).clone_snapshot();
    match snapshot.tab {
        Tab::Console => console::lines_html(),
        Tab::Elements => render_elements_tab(&snapshot, doc),
        Tab::Source => render_source_tab(&snapshot),
        Tab::Scripts => render_scripts_tab(&snapshot),
    }
}

struct Snapshot {
    tab: Tab,
    source_mode: SourceMode,
    picking: bool,
    page_url: String,
    raw_html: String,
    rendered_html: String,
    scripts: Vec<LoadedScript>,
    selected_script: usize,
    inspected: Option<InspectedElement>,
}

impl State {
    fn clone_snapshot(&self) -> Snapshot {
        Snapshot {
            tab: self.tab,
            source_mode: self.source_mode,
            picking: self.picking,
            page_url: self.page_url.clone(),
            raw_html: self.raw_html.clone(),
            rendered_html: self.rendered_html.clone(),
            scripts: self.scripts.clone(),
            selected_script: self.selected_script,
            inspected: self.inspected.clone(),
        }
    }
}

fn render_code_lines(code: &str, out: &mut String, max_bytes: usize, max_lines: usize) {
    if code.is_empty() {
        out.push_str(r#"<div style="padding:12px;color:#64748b;font-family:'Segoe UI',Arial,sans-serif">Empty source.</div>"#);
        return;
    }
    let truncated_bytes = code.len() > max_bytes;
    let slice = if truncated_bytes {
        let mut end = max_bytes;
        while end > 0 && !code.is_char_boundary(end) {
            end -= 1;
        }
        &code[..end]
    } else {
        code
    };

    let mut line_count = 0usize;
    let mut truncated_lines = false;
    for (idx, line) in slice.lines().enumerate() {
        if idx >= max_lines {
            truncated_lines = true;
            break;
        }
        line_count = idx + 1;
        let _ = write!(
            out,
            r#"<div style="display:flex;gap:10px;padding:1px 8px;border-bottom:1px solid rgba(30,41,59,0.45)"><span style="width:38px;flex-shrink:0;text-align:right;color:#475569;user-select:none">{}</span><span style="flex:1;min-width:0;white-space:pre-wrap;word-break:break-all;color:#cbd5e1">"#,
            line_count
        );
        escape_html(line, out);
        out.push_str("</span></div>");
    }
    if truncated_bytes || truncated_lines {
        let _ = write!(
            out,
            r#"<div style="padding:8px 12px;background:rgba(59,130,246,0.1);border-top:1px solid #1e293b;color:#93c5fd;font-family:'Segoe UI',Arial,sans-serif;font-size:11px">Showing first {line_count} lines ({} of {}). Click <b>Copy</b> above to copy the full source.</div>"#,
            format_bytes(slice.len()),
            format_bytes(code.len())
        );
    }
}

fn render_elements_tab(snap: &Snapshot, doc: Option<&BaseDocument>) -> String {
    let mut out = String::with_capacity(4096);
    out.push_str(r#"<div style="display:flex;flex-direction:column;height:100%;min-height:0">"#);
    if snap.picking {
        out.push_str(
            r#"<div style="padding:6px 12px;background:rgba(37,99,235,0.22);border-bottom:1px solid #3b82f6;color:#dbeafe;font-family:'Segoe UI',Arial,sans-serif;font-size:12px;flex-shrink:0"><b>Element Picker Active:</b> Click any element on the page to inspect its HTML, layout box, and computed CSS styles (press <b>Esc</b> to cancel).</div>"#,
        );
    }
    out.push_str(r#"<div style="display:flex;flex:1;min-height:0;overflow:hidden;gap:0">"#);

    // Left column: DOM tree outline + inspected element details & outerHTML
    out.push_str(r#"<div style="flex:1;min-width:0;height:100%;border-right:1px solid #1e293b;padding:8px 10px;overflow:auto;box-sizing:border-box">"#);

    if let Some(el) = &snap.inspected {
        // Breadcrumb bar
        out.push_str(r#"<div style="display:flex;flex-wrap:wrap;gap:4px;align-items:center;margin-bottom:8px;font-size:11px;color:#94a3b8">"#);
        for (i, seg) in el.breadcrumb.iter().enumerate() {
            if i > 0 {
                out.push_str(r#"<span style="color:#475569">›</span>"#);
            }
            let color = if i + 1 == el.breadcrumb.len() { "#60a5fa" } else { "#94a3b8" };
            let _ = write!(out, r#"<span style="color:{color}">"#);
            escape_html(seg, &mut out);
            out.push_str("</span>");
        }
        out.push_str("</div>");

        // Element summary badge + dimensions
        let (x, y, w, h) = el.rect;
        let _ = write!(
            out,
            r#"<div style="display:flex;flex-wrap:wrap;align-items:center;gap:8px;padding:6px 9px;background:#131b2c;border:1px solid #1e293b;border-radius:6px;margin-bottom:8px"><span style="color:#f472b6;font-weight:700">&lt;{}</span>"#,
            el.tag
        );
        if !el.id_attr.is_empty() {
            out.push_str(r#"<span style="color:#fbbf24">#"#);
            escape_html(&el.id_attr, &mut out);
            out.push_str("</span>");
        }
        if !el.classes.is_empty() {
            let short_cls: String = el.classes.chars().take(64).collect();
            out.push_str(r#"<span style="color:#38bdf8">."#);
            escape_html(&short_cls.replace(' ', "."), &mut out);
            out.push_str("</span>");
        }
        let _ = write!(
            out,
            r#"<span style="color:#f472b6;font-weight:700">&gt;</span><span style="flex:1"></span><span style="padding:1px 7px;border-radius:4px;background:rgba(59,130,246,0.16);color:#93c5fd;font-size:11px">{w:.0} × {h:.0} px @ ({x:.0}, {y:.0})</span><span style="color:#64748b;font-size:11px">{} children · node #{}</span></div>"#,
            el.child_count, el.node_id
        );

        // Attributes list
        if !el.attrs.is_empty() {
            out.push_str(r#"<div style="margin-bottom:8px;padding:6px 8px;background:#0b1120;border:1px solid #1e293b;border-radius:6px"><div style="color:#64748b;font-size:10px;text-transform:uppercase;letter-spacing:0.06em;margin-bottom:4px;font-family:'Segoe UI',Arial,sans-serif">Attributes</div>"#);
            for (k, v) in &el.attrs {
                out.push_str(r#"<div style="word-break:break-all"><span style="color:#93c5fd">"#);
                escape_html(k, &mut out);
                out.push_str(r#"</span><span style="color:#64748b">="</span><span style="color:#86efac">"#);
                escape_html(v, &mut out);
                out.push_str(r#"</span><span style="color:#64748b">"</span></div>"#);
            }
            out.push_str("</div>");
        }

        // Outer HTML preview
        out.push_str(r#"<div style="border:1px solid #1e293b;border-radius:6px;background:#090e1a;overflow:hidden"><div style="padding:4px 8px;background:#131b2c;border-bottom:1px solid #1e293b;color:#94a3b8;font-family:'Segoe UI',Arial,sans-serif;font-size:11px">Outer HTML</div>"#);
        render_code_lines(&el.outer_html, &mut out, 16 * 1024, 200);
        out.push_str("</div>");
    } else {
        // No element picked yet: show interactive DOM outline of the page's body!
        out.push_str(r#"<div style="padding:8px 10px;background:#131b2c;border:1px solid #1e293b;border-radius:6px;margin-bottom:8px;font-family:'Segoe UI',Arial,sans-serif;color:#cbd5e1">Click <b>Pick</b> (or press <b>Ctrl+Shift+C</b>) to inspect any element on the page, or click a DOM node below:</div>"#);
        if let Some(d) = doc {
            render_dom_outline(d, &mut out);
        } else {
            out.push_str(r#"<div style="color:#64748b;padding:8px">DOM outline available once the page finishes rendering.</div>"#);
        }
    }
    out.push_str("</div>");

    // Right column: Computed Styles
    out.push_str(r#"<div style="width:320px;flex-shrink:0;height:100%;padding:8px 10px;overflow:auto;background:#0b1120;box-sizing:border-box">"#);
    out.push_str(r#"<div style="color:#94a3b8;font-family:'Segoe UI',Arial,sans-serif;font-size:11px;font-weight:700;text-transform:uppercase;letter-spacing:0.05em;margin-bottom:6px">Computed Styles</div>"#);
    if let Some(el) = &snap.inspected {
        if el.computed.is_empty() {
            out.push_str(r#"<div style="color:#64748b">No computed styles available for this node.</div>"#);
        } else {
            for (prop, val) in &el.computed {
                let _ = write!(
                    out,
                    r#"<div style="display:flex;gap:8px;padding:2px 0;border-bottom:1px solid rgba(30,41,59,0.55)"><span style="color:#7dd3fc;min-width:128px;flex-shrink:0">{prop}</span><span style="color:#e2e8f0;word-break:break-word">"#
                );
                escape_html(val, &mut out);
                out.push_str("</span></div>");
            }
        }
    } else {
        // Page summary metrics when no element is selected
        let total_scripts = snap.scripts.len();
        let total_script_bytes: usize = snap.scripts.iter().map(|s| s.bytes).sum();
        let _ = write!(
            out,
            r#"<div style="color:#94a3b8;font-family:'Segoe UI',Arial,sans-serif;font-size:12px;line-height:1.7"><div><b style="color:#e2e8f0">URL:</b> {}</div><div><b style="color:#e2e8f0">Original HTML:</b> {}</div><div><b style="color:#e2e8f0">Rendered DOM:</b> {}</div><div><b style="color:#e2e8f0">Loaded Scripts:</b> {} ({})</div></div>"#,
            escape_inline(&snap.page_url),
            format_bytes(snap.raw_html.len()),
            if snap.rendered_html.is_empty() {
                "Static HTML".to_string()
            } else {
                format_bytes(snap.rendered_html.len())
            },
            total_scripts,
            format_bytes(total_script_bytes)
        );
    }
    out.push_str("</div></div></div>");
    out
}

fn escape_inline(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    escape_html(s, &mut out);
    out
}

fn render_dom_outline(doc: &BaseDocument, out: &mut String) {
    let body_id = match doc.query_selector("body") {
        Ok(Some(id)) => id,
        _ => return,
    };
    let mut emitted = 0usize;
    render_dom_outline_node(doc, body_id, 0, &mut emitted, out);
}

fn render_dom_outline_node(
    doc: &BaseDocument,
    node_id: usize,
    depth: usize,
    emitted: &mut usize,
    out: &mut String,
) {
    if *emitted >= 80 || depth > 4 {
        return;
    }
    let Some(node) = doc.get_node(node_id) else {
        return;
    };
    let Some(el) = node.element_data() else {
        return;
    };
    if let Some(idv) = el.attr(LocalName::from("id"))
        && (idv == "__zipp_chrome" || idv == "__zipp_console")
    {
        return;
    }
    *emitted += 1;
    let indent = depth * 14;
    let tag = el.name.local.as_ref();
    let id_attr = el.attr(LocalName::from("id")).unwrap_or("");
    let class_attr = el.attr(LocalName::from("class")).unwrap_or("");
    let _ = write!(
        out,
        r##"<div style="padding:2px 6px 2px {indent}px;border-bottom:1px solid rgba(30,41,59,0.35)"><a id="__zipp_dt_node_{node_id}" href="zipp-devtools:node/{node_id}" style="text-decoration:none;display:block"><span style="color:#f472b6">&lt;{tag}</span>"##
    );
    if !id_attr.is_empty() {
        out.push_str(r#" <span style="color:#93c5fd">id</span>=<span style="color:#fbbf24">""#);
        escape_html(id_attr, out);
        out.push_str(r#""</span>"#);
    }
    if !class_attr.is_empty() {
        let short: String = class_attr.chars().take(48).collect();
        out.push_str(r#" <span style="color:#93c5fd">class</span>=<span style="color:#86efac">""#);
        escape_html(&short, out);
        if class_attr.len() > 48 {
            out.push('…');
        }
        out.push_str(r#""</span>"#);
    }
    if tag == "iframe"
        && let Some(src) = el.attr(LocalName::from("src"))
    {
        let short: String = src.chars().take(52).collect();
        out.push_str(r#" <span style="color:#93c5fd">src</span>=<span style="color:#f9a8d4">""#);
        escape_html(&short, out);
        out.push_str(r#""</span>"#);
    }
    out.push_str(r#"<span style="color:#f472b6">&gt;</span></a></div>"#);

    for &child in &node.children {
        if *emitted >= 80 {
            break;
        }
        render_dom_outline_node(doc, child, depth + 1, emitted, out);
    }
}

fn render_source_tab(snap: &Snapshot) -> String {
    let (label, code) = match snap.source_mode {
        SourceMode::Original => ("Original Server HTML", snap.raw_html.as_str()),
        SourceMode::Rendered => {
            if snap.rendered_html.is_empty() {
                ("Rendered DOM (same as original HTML)", snap.raw_html.as_str())
            } else {
                ("Live Post-JS Rendered DOM", snap.rendered_html.as_str())
            }
        }
    };
    let line_count = code.lines().count();
    let mut out = String::with_capacity(code.len().min(MAX_DISPLAY_BYTES) + 512);
    let _ = write!(
        out,
        r#"<div style="display:flex;align-items:center;gap:10px;padding:5px 10px;background:#131b2c;border-bottom:1px solid #1e293b;font-family:'Segoe UI',Arial,sans-serif;font-size:11px;color:#94a3b8"><b style="color:#e2e8f0">{label}</b><span>{} · {line_count} lines</span><span style="flex:1"></span><span style="color:#64748b;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;max-width:460px">{}</span></div>"#,
        format_bytes(code.len()),
        escape_inline(&snap.page_url)
    );
    render_code_lines(code, &mut out, MAX_DISPLAY_BYTES, MAX_DISPLAY_LINES);
    out
}

fn render_scripts_tab(snap: &Snapshot) -> String {
    if snap.scripts.is_empty() {
        return r#"<div style="padding:14px 16px;color:#94a3b8;font-family:'Segoe UI',Arial,sans-serif">No scripts, modules, or workers were loaded on this page.</div>"#.into();
    }
    let selected = snap.selected_script.min(snap.scripts.len().saturating_sub(1));
    let mut out = String::with_capacity(8192);
    out.push_str(r#"<div style="display:flex;height:100%;min-height:0;overflow:hidden">"#);

    // Left sidebar: loaded scripts & modules list
    out.push_str(r#"<div style="width:270px;flex-shrink:0;height:100%;border-right:1px solid #1e293b;background:#0b1120;overflow:auto;box-sizing:border-box">"#);
    for (idx, s) in snap.scripts.iter().enumerate() {
        let is_sel = idx == selected;
        let bg = if is_sel { "rgba(59,130,246,0.2)" } else { "transparent" };
        let fg = if is_sel { "#f8fafc" } else { "#cbd5e1" };
        let (badge, b_fg, b_bg) = s.kind.badge();
        let _ = write!(
            out,
            r##"<a id="__zipp_dt_script_{idx}" href="zipp-devtools:script/{idx}" title="{}" style="display:flex;align-items:center;gap:6px;padding:5px 8px;border-bottom:1px solid rgba(30,41,59,0.55);background:{bg};color:{fg};text-decoration:none;font-family:'Segoe UI',Arial,sans-serif;font-size:11px"><span style="padding:1px 5px;border-radius:4px;background:{b_bg};color:{b_fg};font-family:Consolas,monospace;font-size:10px;font-weight:700;flex-shrink:0">{badge}</span><span style="flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">{}</span><span style="color:#64748b;font-size:10px;flex-shrink:0">{}</span></a>"##,
            escape_inline(&s.url),
            escape_inline(&s.label),
            format_bytes(s.bytes)
        );
    }
    out.push_str("</div>");

    // Right pane: selected script details & source
    let script = &snap.scripts[selected];
    let (badge, b_fg, b_bg) = script.kind.badge();
    let lines = script.source.lines().count();
    out.push_str(r#"<div style="flex:1;min-width:0;height:100%;overflow:auto;box-sizing:border-box">"#);
    let _ = write!(
        out,
        r#"<div style="display:flex;align-items:center;gap:8px;padding:5px 10px;background:#131b2c;border-bottom:1px solid #1e293b;font-family:'Segoe UI',Arial,sans-serif;font-size:11px"><span style="padding:1px 6px;border-radius:4px;background:{b_bg};color:{b_fg};font-family:Consolas,monospace;font-size:10px;font-weight:700">{badge}</span><span style="color:#e2e8f0;font-weight:600;word-break:break-all;flex:1">{}</span><span style="color:#94a3b8;flex-shrink:0">{} · {lines} lines</span></div>"#,
        escape_inline(&script.url),
        format_bytes(script.bytes)
    );
    if let Some(err) = &script.error {
        let _ = write!(
            out,
            r#"<div style="padding:6px 10px;background:rgba(239,68,68,0.14);border-bottom:1px solid rgba(239,68,68,0.3);color:#fca5a5;font-size:11px">Load error: {}</div>"#,
            escape_inline(err)
        );
    }
    render_code_lines(&script.source, &mut out, MAX_DISPLAY_BYTES, MAX_DISPLAY_LINES);
    out.push_str("</div></div>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_source_scripts_and_switches_tabs() {
        record_this_thread();
        new_page("https://formlogic.com/docs", "<!doctype html><html><body>hi</body></html>");
        set_rendered_html("<!doctype html><html><body><main>Docs</main></body></html>");
        record_script(
            ScriptKind::Module,
            "https://formlogic.com/assets/index.js",
            "export const x = 1;".into(),
            None,
        );
        record_script(
            ScriptKind::Inline,
            "https://formlogic.com/docs#inline1",
            "console.log('hi');".into(),
            None,
        );

        set_tab(Tab::Source);
        set_source_mode(SourceMode::Original);
        assert!(copy_text().contains("<body>hi</body>"));
        set_source_mode(SourceMode::Rendered);
        assert!(copy_text().contains("<main>Docs</main>"));
        let src_html = body_html(None);
        assert!(src_html.contains("Live Post-JS Rendered DOM"));

        select_script(0);
        assert_eq!(active_tab(), Tab::Scripts);
        assert_eq!(copy_text(), "export const x = 1;");
        let scripts_html = body_html(None);
        assert!(scripts_html.contains("index.js"));
        assert!(scripts_html.contains("MOD"));

        // Reset to Console tab for subsequent tests.
        set_tab(Tab::Console);
        console::set_open(false);
    }
}
