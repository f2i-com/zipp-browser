//! Phase 2 Live DOM Bridge: JS event handler dispatch.
//!
//! After SSR, the dom shim's `__eventHandlers` registry contains a mapping
//! from JS node `_id` values to the event types they listen on.  During
//! serialization, elements with handlers get a `data-flid="<_id>"` attribute
//! injected into the HTML.
//!
//! After blitz-dom parses that HTML we walk the node tree, find every
//! `data-flid` attribute, and build a reverse map from the JS node ID
//! (flid) to the blitz-dom slab index.  Combined with the event-map JSON
//! from the JS side, this tells us which blitz-dom node corresponds to
//! which JS event handler, and for which event types.
//!
//! When a live `ScriptState` is provided (Phase 2), the bridge calls
//! `eval_in_context` to invoke the actual JS handler function via the
//! `__eventHandlers[flid][eventType][0](event)` registry.

use std::collections::HashMap;
use std::time::Instant;

use crate::console;

use zipp_vm::embed::{JsValue, ScriptState};
use zipp_dom::traits_events::{DomEvent, DomEventData, EventState, PointerCoords, PointerId, WheelDelta};
use zipp_dom::{BaseDocument, Document, EventHandler, LocalName};

// ---------------------------------------------------------------------------
// flid map construction
// ---------------------------------------------------------------------------

/// Run the page's event loop until it is quiet, as a browser does: one task
/// at a time -- a due timer, a posted message, and up to `frames` animation
/// frames once nothing else is due -- with the VM's microtasks run after each
/// (`__zippRunTask`). Stops after `max_tasks`, or once `budget` has passed,
/// as a page may reschedule forever -- an app animating on a timer chain whose
/// every step was slow kept a page from appearing for three minutes. What is
/// left runs as the window's timers come due. Returns how many tasks ran,
/// frames not counted.
///
/// The page's timers used to run in batches, the VM's microtasks only between
/// batches, so an `await` continuation ran after every timer that was due
/// rather than before the next one -- out of order, and late enough that an
/// IndexedDB transaction would commit under it.
pub(crate) fn run_page_tasks(state: &mut ScriptState, max_tasks: usize, mut frames: u32, budget: std::time::Duration) -> usize {
    let start = Instant::now();
    state.run_microtasks();
    let mut ran = 0;
    while ran < max_tasks && start.elapsed() < budget {
        match state.call_global("__zippRunTask", &[JsValue::Bool(frames > 0)]) {
            Ok(JsValue::Number(n)) if n == 1.0 => ran += 1,
            Ok(JsValue::Number(n)) if n == 2.0 => frames = frames.saturating_sub(1),
            _ => break,
        }
        state.run_microtasks();
    }
    ran
}

/// Walk every node in the parsed blitz-dom document and collect the
/// `data-flid` attribute values.  Returns a map from JS node ID (the flid
/// integer) to the blitz-dom slab node ID.
pub fn build_flid_map(doc: &BaseDocument) -> HashMap<u32, usize> {
    let mut map = HashMap::new();
    let flid_attr = LocalName::from("data-flid");
    // Recursive tree walk starting from the root node (id 0).
    walk_node(doc, 0, &flid_attr, &mut map);
    map
}

/// Recursively visit `node_id` and all descendants, collecting data-flid attrs.
fn walk_node(
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
        // Visit children
        let child_ids: Vec<usize> = node.children.clone();
        for child_id in child_ids {
            walk_node(doc, child_id, flid_attr, map);
        }
    }
}

// ---------------------------------------------------------------------------
// JsEventBridge
// ---------------------------------------------------------------------------

/// Maps blitz-dom node IDs to the JS event types registered on them, and
/// optionally holds a live `ScriptState` for calling handlers at runtime.
pub struct JsEventBridge {
    /// blitz_node_id -> list of event type strings (e.g. "click", "mousedown")
    handler_map: HashMap<usize, Vec<String>>,
    /// Reverse map: blitz_node_id -> JS flid (node _id in the dom shim).
    reverse_flid: HashMap<usize, u32>,
    /// The live Zipp VM state.
    js_state: Option<ScriptState>,
    /// If a click handler caused a DOM update, this holds the new HTML.
    /// The shell should check this after event dispatch and re-render if set.
    pub pending_rerender: Option<String>,
    /// A `focus()` on an element not shown yet (made by the same listener):
    /// given when the re-render that shows it lands.
    pending_focus: Option<u32>,
    /// The page URL, used to re-absolutise relative URLs and rebuild the
    /// browser chrome when a re-render replaces the document body.
    page_url: String,
    /// When the page started, for its animation frames' timestamps.
    started: std::time::Instant,
    /// Which page this is (`worker::new_page`), so its going stops its own
    /// workers and not the next page's.
    page: u64,
    /// Whether the page shows the script's tree, so its listeners are wired
    /// to it. A page showing its own markup keeps its script for timers and
    /// the console, but hears no events: its listeners were never wired, and
    /// a link-intercepting one (a site's own router) could cancel real
    /// navigations it then cannot carry out.
    live_dom: bool,
    /// When the page's next timer is due, as it last said (`__zippNextTimer`).
    next_timer: Option<Instant>,
    /// The page's DOM mutation count when its tree was last rendered, so
    /// background work re-renders only what changed it.
    rendered_mutations: f64,
    /// The scroll containers among the script's nodes; `None` until found
    /// for the tree as it is now.
    scrollers: Option<Vec<usize>>,
    /// What the script was last told of each one (`Node::scroll_metrics`).
    scroll_metrics: HashMap<u32, [f64; 6]>,
    /// A hash of the tree as last rendered: background work (timers, worker
    /// messages) compares against it, which sees every kind of change.
    rendered_hash: Option<u64>,
    /// The console as the panel last showed it: its generation, when, and
    /// the badge's text.
    console_shown: u64,
    console_refreshed: Instant,
    console_badge: String,
}

/// The console panel refreshes at most this often, so a page logging every
/// frame does not lay the window out every frame.
const CONSOLE_REFRESH: std::time::Duration = std::time::Duration::from_millis(250);

impl JsEventBridge {
    /// Build the bridge from:
    ///  - `flid_map`: JS node _id -> blitz-dom node id  (from `build_flid_map`)
    ///  - `event_map_json`: JSON string `{"<js_nid>": ["click", ...], ...}`
    ///     produced by `__getEventMap()` in the dom shim.
    ///  - `js_state`: optional live ScriptState for calling handlers.
    pub fn new(
        flid_map: HashMap<u32, usize>,
        event_map_json: &str,
        js_state: Option<ScriptState>,
        page_url: &str,
    ) -> Self {
        let handler_map = parse_event_map(&flid_map, event_map_json);
        // Build reverse map: blitz_node_id -> JS flid
        let reverse_flid: HashMap<usize, u32> = flid_map
            .iter()
            .map(|(&flid, &blitz_id)| (blitz_id, flid))
            .collect();
        let mut bridge = JsEventBridge {
            handler_map,
            reverse_flid,
            js_state,
            pending_rerender: None,
            pending_focus: None,
            page_url: page_url.to_string(),
            started: std::time::Instant::now(),
            page: crate::worker::current_page(),
            live_dom: !event_map_json.is_empty(),
            next_timer: None,
            rendered_mutations: 0.0,
            scrollers: None,
            scroll_metrics: HashMap::new(),
            rendered_hash: None,
            console_shown: 0,
            console_refreshed: Instant::now(),
            console_badge: String::new(),
        };
        bridge.note_rendered();
        bridge.refresh_next_timer();
        bridge
    }

    /// Number of blitz-dom nodes that have at least one JS event handler.
    pub fn handler_count(&self) -> usize {
        self.handler_map.len()
    }

    /// Whether the bridge has a live JS state for calling handlers.
    pub fn has_js_state(&self) -> bool {
        self.js_state.is_some()
    }

    /// Return the event types registered on a given blitz-dom node, if any.
    #[allow(dead_code)]
    pub fn event_types_for(&self, blitz_node_id: usize) -> Option<&[String]> {
        self.handler_map.get(&blitz_node_id).map(|v| v.as_slice())
    }
}

// ---------------------------------------------------------------------------
// EventHandler implementation
// ---------------------------------------------------------------------------

impl JsEventBridge {
    /// After a click's listeners: settle the page, then see whether they
    /// changed the DOM.
    fn settle_after_click(&mut self) {
        let Some(state) = self.js_state.as_mut() else {
            return;
        };
        // React's setState is async — the handler has only
        // queued the re-render at this point.
        run_page_tasks(state, 400, 2, std::time::Duration::from_secs(1));
        print_console(state);
        if let Ok(JsValue::String(new_html)) =
            state.call_global("__getRenderedHTML", &[])
        {
            if !new_html.is_empty() {
                // `__getRenderedHTML` returns the page's own
                // <body> element; its contents replace the
                // document body's. Relative URLs need
                // re-absolutising: the initial render did it,
                // and without it images broke on the first
                // click. The <html> attributes (the theme)
                // travel separately, through
                // `document_element_attributes`.
                let rebuilt =
                    crate::rerender_body(&new_html, &self.page_url);
                eprintln!(
                    "[EVENT] DOM updated after click ({} bytes)",
                    rebuilt.len()
                );
                if let Ok(path) = std::env::var("ZIPP_DUMP_RERENDER") {
                    let _ = std::fs::write(&path, &rebuilt);
                }
                self.pending_rerender = Some(rebuilt);
            }
        }
        self.note_rendered();
        self.refresh_next_timer();
    }

    /// Hand the page what was picked in a file dialog -- `value` is zipp-dom's
    /// `[[path, relativePath], ...]` -- with each file's size and time, and let
    /// it read them. Its `change` listener then runs, and the page settles.
    fn files_chosen(&mut self, flid: u32, value: &str) {
        let Ok(picked) = serde_json::from_str::<Vec<(String, String)>>(value) else {
            return;
        };
        let files: Vec<serde_json::Value> = picked
            .iter()
            .map(|(path, relative)| {
                let meta = std::fs::metadata(path).ok();
                let modified = meta
                    .as_ref()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_millis() as u64);
                serde_json::json!([path, relative, meta.map_or(0, |m| m.len()), modified])
            })
            .collect();
        crate::allow_chosen_files(picked.iter().map(|(path, _)| std::path::Path::new(path)));
        let Some(state) = self.js_state.as_mut() else {
            return;
        };
        let json = serde_json::Value::Array(files).to_string();
        if let Err(e) = state.call_global("__zippFilesChosen", &[JsValue::Number(f64::from(flid)), JsValue::String(json)]) {
            eprintln!("[EVENT] handing over the chosen files failed: {e}");
        }
        state.run_microtasks();
        // The page reads the files and shows them: settle as after a click.
        self.settle_after_click();
    }

    /// Bring scrolling into line both ways. Where the script scrolled an
    /// element (`scrollTop = ...`), scroll it -- to where it asked: the layout
    /// clamps it to the element's range. And tell the script where its scroll
    /// containers are and how big, so that `scrollHeight` and `clientHeight`
    /// mean something and a console can keep its newest line in view.
    fn sync_scrolls(&mut self, doc: &mut BaseDocument) {
        let Some(state) = self.js_state.as_mut() else {
            return;
        };
        if let Ok(JsValue::String(json)) = state.call_global("__zippTakeScrolls", &[])
            && let Ok(serde_json::Value::Object(asked)) = serde_json::from_str(&json)
        {
            for (nid, to) in asked {
                let Ok(flid) = nid.parse::<u32>() else { continue };
                let Some((&node, _)) = self.reverse_flid.iter().find(|&(_, &f)| f == flid) else {
                    continue;
                };
                let Some(node) = doc.get_node_mut(node) else { continue };
                if let Some(top) = to.get(0).and_then(serde_json::Value::as_f64) {
                    node.scroll_offset.y = top;
                }
                if let Some(left) = to.get(1).and_then(serde_json::Value::as_f64) {
                    node.scroll_offset.x = left;
                }
            }
            doc.shell_provider.request_redraw();
        }
        // Nothing more until the tree has been styled and laid out: just after
        // a re-render its nodes are new, and every overflow would read as the
        // default and every size as zero -- a console told its height is 0
        // would scroll back to its top.
        let laid_out = self
            .reverse_flid
            .keys()
            .any(|&id| doc.get_node(id).is_some_and(|n| n.primary_styles().is_some()));
        if !laid_out {
            return;
        }
        if self.scrollers.is_none() {
            let found = self
                .reverse_flid
                .keys()
                .copied()
                .filter(|&id| doc.get_node(id).is_some_and(|n| n.is_scroll_container()))
                .collect();
            self.scrollers = Some(found);
        }
        let scrollers = self.scrollers.clone().unwrap_or_default();
        self.tell_scroll_metrics(doc, &scrollers);
    }

    /// Tell the script how these nodes are scrolled, where that has changed.
    fn tell_scroll_metrics(&mut self, doc: &BaseDocument, nodes: &[usize]) {
        let mut changed = serde_json::Map::new();
        for &id in nodes {
            let (Some(node), Some(&flid)) = (doc.get_node(id), self.reverse_flid.get(&id)) else {
                continue;
            };
            let metrics = node.scroll_metrics();
            if self.scroll_metrics.get(&flid) != Some(&metrics) {
                self.scroll_metrics.insert(flid, metrics);
                changed.insert(flid.to_string(), serde_json::json!(metrics));
            }
        }
        if changed.is_empty() {
            return;
        }
        if let Some(state) = self.js_state.as_mut() {
            let json = serde_json::Value::Object(changed).to_string();
            if let Err(e) = state.call_global("__zippScrollMetrics", &[JsValue::String(json)]) {
                eprintln!("[EVENT] telling the page its scroll positions failed: {e}");
            }
        }
    }

    /// Give focus where the page's `focus()` last asked for it, if it did.
    fn apply_focus_request(&mut self, doc: &mut BaseDocument) {
        let Some(state) = self.js_state.as_mut() else {
            return;
        };
        let Ok(JsValue::Number(n)) = state.call_global("__zippTakeFocus", &[]) else {
            return;
        };
        if n < 0.0 {
            return;
        }
        let flid = n as u32;
        match self.reverse_flid.iter().find(|&(_, &f)| f == flid) {
            Some((&node, _)) => {
                doc.set_focus_to(node);
                self.pending_focus = None;
            }
            None => self.pending_focus = Some(flid),
        }
    }

    /// Whether the page changed its tree since it was last shown.
    fn dom_changed(&mut self) -> bool {
        self.js_state.as_mut().is_some_and(|state| {
            matches!(state.call_global("__zippDomMutationCount", &[]), Ok(JsValue::Number(n)) if n != self.rendered_mutations)
        })
    }

    /// The tree as it is now is the one shown.
    fn note_rendered(&mut self) {
        if let Some(state) = self.js_state.as_mut()
            && let Ok(JsValue::Number(n)) = state.call_global("__zippDomMutationCount", &[])
        {
            self.rendered_mutations = n;
        }
    }

    /// Ask the page when its next timer is due.
    fn refresh_next_timer(&mut self) {
        let Some(state) = self.js_state.as_mut() else {
            self.next_timer = None;
            return;
        };
        self.next_timer = match state.call_global("__zippNextTimer", &[]) {
            Ok(JsValue::Number(ms)) if ms >= 0.0 => Some(Instant::now() + std::time::Duration::from_millis(ms as u64)),
            _ => None,
        };
    }

    /// Run what has come due -- timers, then the promises they settled --
    /// and render the tree again if that changed it.
    fn run_due_work(&mut self) {
        let Some(state) = self.js_state.as_mut() else {
            return;
        };
        run_page_tasks(state, 250, 0, std::time::Duration::from_millis(250));
        print_console(state);
        if let Ok(JsValue::String(html)) = state.call_global("__getRenderedHTML", &[])
            && !html.is_empty()
        {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            html.hash(&mut hasher);
            let hash = hasher.finish();
            if self.rendered_hash != Some(hash) {
                self.rendered_hash = Some(hash);
                let rebuilt = crate::rerender_body(&html, &self.page_url);
                if let Ok(path) = std::env::var("ZIPP_DUMP_RERENDER") {
                    let _ = std::fs::write(&path, &rebuilt);
                }
                self.pending_rerender = Some(rebuilt);
            }
        }
        self.note_rendered();
        self.refresh_next_timer();
    }

    /// Run the code typed into the console, logging it and what it gave.
    fn run_console_input(&mut self) {
        let Some(state) = self.js_state.as_mut() else {
            for code in console::take_submitted() {
                console::push(console::Level::Input, code);
                console::push(console::Level::Error, "This page runs no JavaScript.");
            }
            return;
        };
        for code in console::take_submitted() {
            console::push(console::Level::Input, code.clone());
            match state.call_global("__zippConsoleEval", &[JsValue::String(code)]) {
                Ok(JsValue::String(text)) => match text.strip_prefix('\u{1}') {
                    Some(error) => console::push(console::Level::Error, error),
                    None => console::push(console::Level::Result, text),
                },
                Ok(other) => console::push(console::Level::Result, format!("{other:?}")),
                Err(e) => console::push(console::Level::Error, e.to_string()),
            }
        }
        self.run_due_work();
    }

    /// Open or close the console panel as `console::is_open` says, and fill it.
    fn show_console(&mut self, doc: &mut BaseDocument) {
        if let Ok(Some(style)) = doc.query_selector("#__zipp_chrome_css") {
            doc.mutate().set_inner_html(style, &crate::chrome_css(console::is_open()));
        }
        self.console_shown = u64::MAX;
        self.refresh_console(doc, Instant::now(), true);
        doc.shell_provider.request_redraw();
    }

    /// Bring the toolbar's badge, and the panel when open, up to date with
    /// the console; when it next should, if it is behind and too soon.
    fn refresh_console(&mut self, doc: &mut BaseDocument, now: Instant, force: bool) -> Option<Instant> {
        let generation = console::generation();
        if generation == self.console_shown {
            return None;
        }
        let since = now.saturating_duration_since(self.console_refreshed);
        if !force && since < CONSOLE_REFRESH {
            return Some(now + (CONSOLE_REFRESH - since));
        }
        self.console_shown = generation;
        self.console_refreshed = now;
        let badge = console::badge_text();
        let mut changed = false;
        if badge != self.console_badge || force {
            if let Ok(Some(button)) = doc.query_selector("#__zipp_console_button") {
                doc.mutate().set_inner_html(button, &crate::console_button_inner(&badge));
            }
            self.console_badge = badge;
            changed = true;
        }
        if console::is_open() {
            if let Ok(Some(node)) = doc.query_selector("#__zipp_console_counts") {
                doc.mutate().set_inner_html(node, &console::counts_text());
            }
            if let Ok(Some(node)) = doc.query_selector("#__zipp_console_lines") {
                doc.mutate().set_inner_html(node, &console::lines_html());
            }
            changed = true;
        }
        if changed {
            doc.shell_provider.request_redraw();
        }
        None
    }
}

impl Drop for JsEventBridge {
    /// The page is going, and its workers with it.
    fn drop(&mut self) {
        crate::worker::terminate_page(self.page);
    }
}

impl EventHandler for JsEventBridge {
    /// A frame for a page with a WebGL context: its `requestAnimationFrame`
    /// callbacks, then each context's drawing read back into its canvas. Pages
    /// without one are not driven: their animation frames run while they
    /// settle and after events, as before.
    fn animation_frame(&mut self, doc: &mut BaseDocument) -> bool {
        let _layout = LayoutScope::enter(doc);
        let Some(state) = self.js_state.as_mut() else {
            return false;
        };
        match state.call_global("__zippWebGLContexts", &[]) {
            Ok(JsValue::Number(n)) if n > 0.0 => {}
            _ => return false,
        }
        let now = self.started.elapsed().as_secs_f64() * 1000.0;
        let flags = match state.call_global("__zippAnimationFrame", &[JsValue::Number(now)]) {
            Ok(JsValue::Number(n)) => n as u32,
            Ok(_) => 0,
            Err(e) => {
                eprintln!("[WEBGL] animation frame failed: {e}");
                0
            }
        };
        state.run_microtasks();
        print_console(state);
        if crate::apply_canvas_frames(doc) {
            doc.shell_provider.request_paint();
        }
        // The frame changed the DOM as well: lay the page out again.
        if flags & 2 != 0 {
            if let Ok(JsValue::String(html)) = state.call_global("__getRenderedHTML", &[]) {
                if !html.is_empty() {
                    self.pending_rerender = Some(crate::rerender_body(&html, &self.page_url));
                }
            }
        }
        self.note_rendered();
        self.refresh_next_timer();
        flags & 1 != 0
    }

    fn handle_event(
        &mut self,
        chain: &[usize],
        event: &mut DomEvent,
        _doc: &mut dyn Document,
        event_state: &mut EventState,
    ) {
        let event_name = event.data.name();
        let _layout = LayoutScope::enter(&_doc.inner());
        // The console's button and links, and F12, are the browser's.
        let console_key = matches!(&event.data, DomEventData::KeyDown(k) if k.code == keyboard_types::Code::F12);
        let console_link = matches!(event.data, DomEventData::Click(_))
            .then(|| chain_element_id(_doc, chain, &["__zipp_console_button", "__zipp_console_close", "__zipp_console_clear"]))
            .flatten();
        if console_key || console_link.is_some() {
            event_state.prevent_default();
            if console_link.as_deref() == Some("__zipp_console_clear") {
                console::clear();
            } else {
                console::toggle();
            }
            let mut inner = _doc.inner_mut();
            self.show_console(&mut inner);
            return;
        }
        // A file dialog closed on a choice: the page gets the files.
        if let DomEventData::Input(input) = &event.data
            && is_file_input(_doc, event.target)
        {
            if let Some(&flid) = self.reverse_flid.get(&event.target) {
                self.files_chosen(flid, &input.value);
                self.apply_focus_request(&mut _doc.inner_mut());
            }
            return;
        }
        // An element scrolled: its script reads where to before its `scroll`
        // listeners run -- a code editor moving its highlighting with it.
        let scrolled = matches!(event.data, DomEventData::Scroll(_));
        if scrolled {
            self.tell_scroll_metrics(&_doc.inner(), &[event.target]);
        }
        // Typing in a text field: the script's node has the text before its
        // listeners run, as a browser's `input` event finds it. Before, a code
        // editor reading `value` back saw nothing typed at all.
        if let DomEventData::Input(input) = &event.data
            && is_text_field(_doc, event.target)
            && let (Some(&flid), Some(state)) = (self.reverse_flid.get(&event.target), self.js_state.as_mut())
        {
            let args = [JsValue::Number(f64::from(flid)), JsValue::String(input.value.clone())];
            if let Err(e) = state.call_global("__zippSetValue", &args) {
                eprintln!("[EVENT] passing on what was typed failed: {e}");
            }
        }
        let details = event_details(&event.data);
        let mut ran_listener = false;

        // The element actually under the pointer. `chain` is ordered target
        // first, so this is the click target regardless of which ancestor ends
        // up carrying the handler — and with React it is nearly always a
        // different node, because React delegates every listener to the root.
        // The nearest node in the chain that the JS side knows about — NOT
        // simply `chain[0]`.
        //
        // Hit-testing lands on whatever is actually under the pointer, which is
        // often an element the shim never serialised with a `data-flid`: an
        // anonymous block, a text run, or (the case that broke the hamburger) a
        // `<path>` inside an inline `<svg>` icon. With no flid the target fell
        // back to 0, `__nodesById[0]` was undefined, and React — which routes
        // purely by `event.target` — dropped the click on the floor.
        //
        // Walking up to the nearest identified ancestor is also what the DOM
        // says should happen: an event target is an element, and React expects
        // the element whose fiber owns the handler to be reachable from it.
        let target_flid = chain
            .iter()
            .find_map(|id| self.reverse_flid.get(id))
            .copied()
            .unwrap_or(0);

        // Walk the bubble chain (target first, then ancestors). A scroll is
        // the scrolled element's own: it does not bubble.
        let chain = if scrolled { &chain[..chain.len().min(1)] } else { chain };
        for &node_id in chain {
            if let Some(types) = self.handler_map.get(&node_id) {
                if types.iter().any(|t| t == event_name) {
                    // Look up the JS flid for this blitz-dom node.
                    let flid = self.reverse_flid.get(&node_id).copied();

                    if std::env::var_os("ZIPP_DEBUG_EVENTS").is_some() {
                        eprintln!(
                            "[EVDBG] {event_name}: handler node={node_id} flid={flid:?} target_flid={target_flid} chain={:?}",
                            &chain[..chain.len().min(6)]
                        );
                    }
                    if let (Some(flid), Some(state)) = (flid, &mut self.js_state) {
                        // Call the prelude's dispatcher rather than assembling
                        // JS source per event: `call_global` compiles nothing,
                        // where an `eval_in_context` would compile a fresh
                        // program on every click and intern it for the VM's
                        // lifetime. It also means `event_name` is passed as a
                        // value, so it cannot escape into the code being run.
                        let dispatched = state.call_global(
                            "__zippDispatchEvent",
                            &[
                                JsValue::Number(f64::from(flid)),
                                JsValue::Number(f64::from(target_flid)),
                                JsValue::String(event_name.to_string()),
                                JsValue::String(details.clone()),
                            ],
                        );
                        ran_listener = true;
                        match &dispatched {
                            Err(e) => {
                                eprintln!("[EVENT] handler for \"{event_name}\" failed: {e}")
                            }
                            // The page called preventDefault() (1). Blitz checks
                            // its own event state before running an element's
                            // default action, so this is what stops an <a> the
                            // page handled itself from also navigating for real.
                            // Or stopPropagation() (2): the event goes no
                            // further up, nor to the document.
                            Ok(JsValue::Number(n)) => {
                                let asked = *n as u32;
                                if asked & 1 != 0 {
                                    event_state.prevent_default();
                                }
                                if asked & 2 != 0 {
                                    event_state.stop_propagation();
                                }
                            }
                            Ok(_) => {}
                        }
                        // Flush what the handler scheduled. `call_global` returns
                        // the moment the listener returns, so anything it queued
                        // as a microtask is still pending — and React applies
                        // state updates on a microtask. Without this the first
                        // interaction appeared to work (its update happened to be
                        // flushed synchronously) and every later one silently did
                        // nothing, which is how the menu opened but would not
                        // close.
                        state.run_microtasks();
                        print_console(state);

                    } else {
                        eprintln!(
                            "[EVENT] Would call JS handler for \"{event_name}\" on node {node_id} (no live state)"
                        );
                    }
                }
            }
            if event_state.propagation_is_stopped() {
                break;
            }
        }

        // Then the document's own listeners, and the window's (which the shim
        // keeps on the document): drag handlers that follow the pointer off
        // their element listen there, as do a game's keys. Not for the toolbar,
        // which is the browser's, not the page's.
        if self.live_dom && !scrolled && !event_state.propagation_is_stopped() && !in_chrome(_doc, chain) {
            if let Some(state) = self.js_state.as_mut() {
                let result = state.call_global(
                    "__zippDispatchToDocument",
                    &[
                        JsValue::Number(f64::from(target_flid)),
                        JsValue::String(event_name.to_string()),
                        JsValue::String(details),
                    ],
                );
                match result {
                    Ok(JsValue::Number(n)) if n as u32 & 1 != 0 => {
                        if n as u32 & 2 != 0 {
                            event_state.prevent_default();
                        }
                        ran_listener = true;
                        state.run_microtasks();
                        print_console(state);
                    }
                    Ok(_) => {}
                    Err(e) => eprintln!("[EVENT] document handler for \"{event_name}\" failed: {e}"),
                }
            }
        }
        if ran_listener && event_name == "click" {
            self.settle_after_click();
        } else if ran_listener && self.dom_changed() {
            // A key or move listener that changed the page -- a score, a log
            // line -- is shown now, not whenever a timer next comes due.
            self.run_due_work();
        } else if ran_listener {
            self.refresh_next_timer();
        }
        if ran_listener {
            let mut doc = _doc.inner_mut();
            self.apply_focus_request(&mut doc);
            self.sync_scrolls(&mut doc);
        }
    }

    fn run_tasks(&mut self, doc: &mut BaseDocument) -> Option<Instant> {
        let _layout = LayoutScope::enter(doc);
        let now = Instant::now();
        // What the page's workers sent: each message is a task of its own.
        let messages = crate::worker::take_messages();
        if !messages.is_empty() {
            if let Some(state) = self.js_state.as_mut() {
                for (id, message) in messages {
                    let (kind, payload) = match message {
                        crate::worker::FromWorker::Message(data) => ("message", data),
                        crate::worker::FromWorker::Error(error) => ("error", error),
                    };
                    let args = [JsValue::Number(f64::from(id)), JsValue::String(kind.into()), JsValue::String(payload)];
                    if let Err(e) = state.call_global("__zippWorkerDeliver", &args) {
                        eprintln!("[WORKER] delivering a {kind} failed: {e}");
                    }
                    state.run_microtasks();
                }
            }
            self.run_due_work();
        }
        // Programs a page ran that finished (system.rs): each settles its promise.
        let done = crate::system::take_completions();
        if !done.is_empty() {
            if let Some(state) = self.js_state.as_mut() {
                for (id, result) in done {
                    let args = [JsValue::Number(id as f64), JsValue::String(result)];
                    if let Err(e) = state.call_global("__zippSystemComplete", &args) {
                        eprintln!("[ZIPP] delivering a program's result failed: {e}");
                    }
                    state.run_microtasks();
                }
            }
            self.run_due_work();
        }
        // USB transfers that finished (usb.rs): each settles its promise.
        #[cfg(target_os = "linux")]
        {
            let done = crate::usb::take_completions();
            if !done.is_empty() {
                if let Some(state) = self.js_state.as_mut() {
                    for (id, result) in done {
                        let args = [JsValue::Number(id as f64), JsValue::String(result)];
                        if let Err(e) = state.call_global("__zippUsbComplete", &args) {
                            eprintln!("[USB] delivering a transfer failed: {e}");
                        }
                        state.run_microtasks();
                    }
                }
                self.run_due_work();
            }
        }
        if console::has_submitted() {
            self.run_console_input();
            // An empty line again, still focused, for the next.
            if let Ok(Some(form)) = doc.query_selector("#__zipp_console_form") {
                doc.mutate().set_inner_html(form, crate::CONSOLE_INPUT);
                if let Ok(Some(input)) = doc.query_selector("#__zipp_console_input") {
                    doc.set_focus_to(input);
                }
            }
        }
        if self.next_timer.is_some_and(|due| due <= now) {
            self.run_due_work();
        }
        // What canvases drew in that work, shown now: no frame may come.
        if let Some(state) = self.js_state.as_mut()
            && matches!(state.call_global("__zippWebGLContexts", &[]), Ok(JsValue::Number(n)) if n > 0.0)
        {
            let _ = state.call_global("__zippPresentCanvases", &[]);
            if crate::apply_canvas_frames(doc) {
                doc.shell_provider.request_paint();
            }
        }
        self.apply_focus_request(doc);
        self.sync_scrolls(doc);
        let console_due = self.refresh_console(doc, now, false);
        [self.next_timer, console_due].into_iter().flatten().min()
    }

    fn take_pending_rerender(&mut self) -> Option<String> {
        self.pending_rerender.take()
    }

    /// The attributes the page currently has on `<html>`.
    ///
    /// Themes live there and a body-only re-render cannot carry them, so the
    /// document applies these to the real document element separately. `class`
    /// is always included, even when EMPTY: clearing the class is how a page
    /// leaves dark mode, and treating "" as "nothing to say" would make the
    /// theme switchable one way only.
    fn document_element_attributes(&mut self) -> Option<Vec<(String, String)>> {
        let state = self.js_state.as_mut()?;
        match state.call_global("__getHtmlAttrList", &[]) {
            // Records separated by U+001E, name from value by U+001F: control
            // characters no attribute name or theme value contains.
            Ok(JsValue::String(list)) => Some(
                list.split('\u{1e}')
                    .filter_map(|rec| rec.split_once('\u{1f}'))
                    .map(|(name, value)| (name.to_string(), value.to_string()))
                    .collect(),
            ),
            _ => None,
        }
    }

    /// Rebuild the node maps after the body was replaced.
    ///
    /// `set_inner_html` drops and re-parses the entire body, so every blitz-dom
    /// node id in `flid_map`/`reverse_flid`/`handler_map` now refers to a node
    /// that no longer exists. Without this the first click on a page worked and
    /// every later one resolved against dead ids and did nothing — which is why
    /// the hamburger opened but its X would not close it.
    ///
    /// The event map is re-read from the VM at the same time: a re-render can
    /// mount new components, and those register their own listeners.
    fn document_rerendered(&mut self, doc: &mut BaseDocument) {
        // The script's tree is showing now, whatever it was before.
        self.live_dom = true;
        // Its scroll containers are new nodes: find them again.
        self.scrollers = None;
        // The canvases are new nodes now: give them back the frames they showed.
        crate::reapply_canvas_frames(doc);
        let flid_map = build_flid_map(doc);
        let event_map_json = self
            .js_state
            .as_mut()
            .and_then(|state| match state.call_global("__getEventMap", &[]) {
                Ok(JsValue::String(s)) => Some(s),
                _ => None,
            })
            .unwrap_or_else(|| "{}".to_string());

        self.handler_map = parse_event_map(&flid_map, &event_map_json);
        self.reverse_flid = flid_map
            .iter()
            .map(|(&flid, &blitz_id)| (blitz_id, flid))
            .collect();
        if let Some(flid) = self.pending_focus.take()
            && let Some(&node) = flid_map.get(&flid)
        {
            doc.set_focus_to(node);
        }
    }
}

thread_local! {
    /// The document, laid out, while the page's script runs on its behalf --
    /// an event, a timer, a frame: what `getBoundingClientRect` reads.
    static LAYOUT_DOC: std::cell::Cell<*const BaseDocument> = const { std::cell::Cell::new(std::ptr::null()) };
}

/// While alive, the page's script can read the document's layout.
struct LayoutScope(*const BaseDocument);

impl LayoutScope {
    fn enter(doc: &BaseDocument) -> Self {
        LayoutScope(LAYOUT_DOC.with(|d| d.replace(doc)))
    }
}

impl Drop for LayoutScope {
    fn drop(&mut self) {
        LAYOUT_DOC.with(|d| d.set(self.0));
    }
}

/// The border box of the script's node `nid` in the viewport, as JSON
/// `[x, y, width, height]`; "" when there is no layout to read. Before, every
/// box was all zeros, and a page mapping a click onto its canvas divided by a
/// width of 0: a game's buttons, drawn on the canvas, could not be pressed.
/// Whether `node` has been through a layout. A re-render builds the body's
/// nodes afresh, and until the next one styles and lays them out their boxes
/// read as 0 by 0: a page's script running in between saw its elements
/// shrink to nothing and grow back, and a WebGL app sizing its canvas to them
/// reallocated it at 1 by 1 and back, frame after frame.
fn laid_out(node: &zipp_dom::Node) -> bool {
    node.primary_styles().is_some()
}

pub(crate) fn layout_rect(nid: &str) -> String {
    let Ok(nid) = nid.parse::<u32>() else {
        return String::new();
    };
    LAYOUT_DOC.with(|d| {
        let ptr = d.get();
        if ptr.is_null() {
            return String::new();
        }
        // SAFETY: set only while a bridge method that has the document runs
        // the page's script, which reads the layout and cannot change it.
        let doc = unsafe { &*ptr };
        let Ok(Some(id)) = doc.query_selector(&format!("[data-flid=\"{nid}\"]")) else {
            return String::new();
        };
        let Some(node) = doc.get_node(id).filter(|n| laid_out(n)) else {
            return String::new();
        };
        // `absolute_position` takes off the node's own scroll too, which moves
        // its content, not the box.
        let at = node.absolute_position(0.0, 0.0);
        let scroll = doc.viewport_scroll();
        let size = node.final_layout.size;
        format!(
            "[{},{},{},{}]",
            f64::from(at.x) + node.scroll_offset.x - scroll.x,
            f64::from(at.y) + node.scroll_offset.y - scroll.y,
            size.width,
            size.height
        )
    })
}

/// The boxes of the script's node `nid` from the last layout, as JSON
/// `[borderW, borderH, clientW, clientH, contentW, contentH]` -- what
/// `offsetWidth`, `clientWidth` and a ResizeObserver's content box read; ""
/// when there is no layout to read.
pub(crate) fn layout_box(nid: &str) -> String {
    let Ok(nid) = nid.parse::<u32>() else {
        return String::new();
    };
    LAYOUT_DOC.with(|d| {
        let ptr = d.get();
        if ptr.is_null() {
            return String::new();
        }
        // SAFETY: as in `layout_rect`.
        let doc = unsafe { &*ptr };
        let Ok(Some(id)) = doc.query_selector(&format!("[data-flid=\"{nid}\"]")) else {
            return String::new();
        };
        let Some(node) = doc.get_node(id).filter(|n| laid_out(n)) else {
            return String::new();
        };
        let l = &node.final_layout;
        let client_w = (l.size.width - l.border.left - l.border.right - l.scrollbar_size.width).max(0.0);
        let client_h = (l.size.height - l.border.top - l.border.bottom - l.scrollbar_size.height).max(0.0);
        let content_w = (client_w - l.padding.left - l.padding.right).max(0.0);
        let content_h = (client_h - l.padding.top - l.padding.bottom).max(0.0);
        format!("[{},{},{client_w},{client_h},{content_w},{content_h}]", l.size.width, l.size.height)
    })
}

/// The computed value of CSS property `name` on the script's node `nid`, from
/// the last layout, as `getComputedStyle` reads it; "" when there is no
/// layout to read or no such longhand.
pub(crate) fn layout_style(nid: &str, name: &str) -> String {
    let Ok(nid) = nid.parse::<u32>() else {
        return String::new();
    };
    LAYOUT_DOC.with(|d| {
        let ptr = d.get();
        if ptr.is_null() {
            return String::new();
        }
        // SAFETY: as in `layout_rect`.
        let doc = unsafe { &*ptr };
        let Ok(Some(id)) = doc.query_selector(&format!("[data-flid=\"{nid}\"]")) else {
            return String::new();
        };
        doc.get_node(id).and_then(|node| node.computed_style_value(name)).unwrap_or_default()
    })
}

/// Whether `node` is a field text is typed into: an `<input>` or `<textarea>`.
fn is_text_field(doc: &dyn Document, node: usize) -> bool {
    doc.inner()
        .get_node(node)
        .and_then(|n| n.element_data())
        .is_some_and(|el| el.text_input_data().is_some())
}

/// Whether `node` is an `<input type=file>`.
fn is_file_input(doc: &dyn Document, node: usize) -> bool {
    doc.inner()
        .get_node(node)
        .and_then(|n| n.element_data())
        .is_some_and(|el| {
            &*el.name.local == "input" && el.attr(LocalName::from("type")) == Some("file")
        })
}

/// Whether an event happened in the browser's toolbar rather than the page.
fn in_chrome(doc: &dyn Document, chain: &[usize]) -> bool {
    let doc = doc.inner();
    chain.iter().any(|&id| {
        doc.get_node(id)
            .and_then(|n| n.element_data())
            .is_some_and(|el| el.id.as_ref().is_some_and(|id| &**id == "__zipp_chrome"))
    })
}

/// What a listener reads off an event besides its type and target, as a JSON
/// object for `__zippDispatchEvent`: where the pointer is, its buttons, the
/// modifier keys, how far the wheel turned, which key.
fn event_details(data: &DomEventData) -> String {
    use keyboard_types::{Location, Modifiers};
    use std::fmt::Write;
    let mut out = String::from("{");
    let coords = |out: &mut String, c: &PointerCoords| {
        let _ = write!(
            out,
            "\"clientX\":{},\"clientY\":{},\"x\":{},\"y\":{},\"pageX\":{},\"pageY\":{},\"screenX\":{},\"screenY\":{},",
            c.client_x, c.client_y, c.client_x, c.client_y, c.page_x, c.page_y, c.screen_x, c.screen_y
        );
    };
    let mods = |out: &mut String, m: &Modifiers| {
        let _ = write!(
            out,
            "\"altKey\":{},\"ctrlKey\":{},\"shiftKey\":{},\"metaKey\":{},",
            m.contains(Modifiers::ALT),
            m.contains(Modifiers::CONTROL),
            m.contains(Modifiers::SHIFT),
            m.contains(Modifiers::META)
        );
    };
    match data {
        DomEventData::PointerMove(p)
        | DomEventData::PointerDown(p)
        | DomEventData::PointerUp(p)
        | DomEventData::PointerEnter(p)
        | DomEventData::PointerLeave(p)
        | DomEventData::PointerOver(p)
        | DomEventData::PointerOut(p)
        | DomEventData::MouseMove(p)
        | DomEventData::MouseDown(p)
        | DomEventData::MouseUp(p)
        | DomEventData::MouseEnter(p)
        | DomEventData::MouseLeave(p)
        | DomEventData::MouseOver(p)
        | DomEventData::MouseOut(p)
        | DomEventData::Click(p)
        | DomEventData::ContextMenu(p)
        | DomEventData::DoubleClick(p) => {
            coords(&mut out, &p.coords);
            mods(&mut out, &p.mods);
            let (id, kind) = match p.id {
                PointerId::Mouse => (1, "mouse"),
                PointerId::Pen => (2, "pen"),
                PointerId::Finger(n) => (n.saturating_add(3), "touch"),
            };
            let _ = write!(
                out,
                "\"button\":{},\"buttons\":{},\"pointerId\":{id},\"pointerType\":\"{kind}\",\"isPrimary\":{},\"pressure\":{}",
                p.button as u8,
                p.buttons.bits(),
                p.is_primary,
                p.details.pressure
            );
        }
        DomEventData::Wheel(w) => {
            coords(&mut out, &w.coords);
            mods(&mut out, &w.mods);
            // The OS turns the wheel away from you as positive; the DOM calls
            // that scrolling up, a negative delta, and Chrome counts a notch
            // as 100 pixels.
            let (dx, dy) = match w.delta {
                WheelDelta::Lines(x, y) => (-x * 100.0, -y * 100.0),
                WheelDelta::Pixels(x, y) => (-x, -y),
            };
            let _ = write!(out, "\"deltaX\":{dx},\"deltaY\":{dy},\"deltaZ\":0,\"deltaMode\":0,\"buttons\":{}", w.buttons.bits());
        }
        DomEventData::KeyDown(k) | DomEventData::KeyUp(k) | DomEventData::KeyPress(k) => {
            mods(&mut out, &k.modifiers);
            let location = match k.location {
                Location::Standard => 0,
                Location::Left => 1,
                Location::Right => 2,
                Location::Numpad => 3,
            };
            let key = serde_json::Value::String(k.key.to_string());
            let code = serde_json::Value::String(k.code.to_string());
            let _ = write!(
                out,
                "\"key\":{key},\"code\":{code},\"location\":{location},\"repeat\":{},\"isComposing\":{}",
                k.is_auto_repeating, k.is_composing
            );
        }
        _ => {}
    }
    if out.ends_with(',') {
        out.pop();
    }
    out.push('}');
    out
}

// ---------------------------------------------------------------------------
// JSON parsing (minimal, no serde dependency)
// ---------------------------------------------------------------------------

/// Parse the event map JSON and combine it with the flid map to produce a
/// `blitz_node_id -> [event_type]` lookup.
///
/// The JSON format is: `{"42": ["click","mousedown"], "99": ["click"]}`
/// We do minimal hand-parsing to avoid pulling in serde_json.
/// What the page logged since the last call (its `console`, the shim's
/// reports of callbacks that threw). The first render prints its own; this is
/// everything after, which would otherwise pile up unseen.
fn print_console(state: &mut ScriptState) {
    for line in state.take_output() {
        eprintln!("[JS] {line}");
        console::push(console::Level::Log, line);
    }
    for line in state.take_errput() {
        eprintln!("[JS] {line}");
        console::push(console::Level::Error, line);
    }
}

/// The first of `ids` that an element on the event's path has.
fn chain_element_id(doc: &dyn Document, chain: &[usize], ids: &[&str]) -> Option<String> {
    let doc = doc.inner();
    chain.iter().find_map(|&id| {
        let el = doc.get_node(id)?.element_data()?;
        let found = el.id.as_ref()?;
        ids.iter().find(|&&want| &**found == want).map(|s| s.to_string())
    })
}

fn parse_event_map(
    flid_map: &HashMap<u32, usize>,
    json: &str,
) -> HashMap<usize, Vec<String>> {
    let mut result = HashMap::new();
    let json = json.trim();
    if !json.starts_with('{') || !json.ends_with('}') {
        return result;
    }
    // Strip outer braces
    let inner = &json[1..json.len() - 1];

    // Walk through key-value pairs: "nid": ["type1","type2"]
    let mut pos = 0;
    let bytes = inner.as_bytes();
    while pos < bytes.len() {
        // Skip whitespace and commas
        while pos < bytes.len() && (bytes[pos] == b' ' || bytes[pos] == b',' || bytes[pos] == b'\n' || bytes[pos] == b'\r' || bytes[pos] == b'\t') {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }

        // Expect a quoted key: "nid"
        if bytes[pos] != b'"' {
            break;
        }
        pos += 1; // skip opening quote
        let key_start = pos;
        while pos < bytes.len() && bytes[pos] != b'"' {
            pos += 1;
        }
        let key = &inner[key_start..pos];
        pos += 1; // skip closing quote

        // Skip colon and whitespace
        while pos < bytes.len() && (bytes[pos] == b':' || bytes[pos] == b' ') {
            pos += 1;
        }

        // Parse the JS node ID
        let js_nid: u32 = match key.parse() {
            Ok(n) => n,
            Err(_) => {
                // Skip to next pair
                if let Some(next_comma) = inner[pos..].find(',') {
                    pos += next_comma + 1;
                } else {
                    break;
                }
                continue;
            }
        };

        // Look up the blitz-dom node ID
        let blitz_id = flid_map.get(&js_nid).copied();

        // Parse the array of event type strings: ["click","mousedown"]
        if pos >= bytes.len() || bytes[pos] != b'[' {
            break;
        }
        pos += 1; // skip '['
        let mut types = Vec::new();
        loop {
            // Skip whitespace and commas
            while pos < bytes.len() && (bytes[pos] == b' ' || bytes[pos] == b',' || bytes[pos] == b'\n') {
                pos += 1;
            }
            if pos >= bytes.len() || bytes[pos] == b']' {
                pos += 1; // skip ']'
                break;
            }
            if bytes[pos] == b'"' {
                pos += 1;
                let str_start = pos;
                while pos < bytes.len() && bytes[pos] != b'"' {
                    pos += 1;
                }
                types.push(inner[str_start..pos].to_string());
                pos += 1; // skip closing quote
            } else {
                break;
            }
        }

        if let Some(bid) = blitz_id {
            if !types.is_empty() {
                result.entry(bid).or_insert_with(Vec::new).extend(types);
            }
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_event_map() {
        let flid_map = HashMap::new();
        let result = parse_event_map(&flid_map, "{}");
        assert!(result.is_empty());
    }

    #[test]
    fn parse_event_map_basic() {
        let mut flid_map = HashMap::new();
        flid_map.insert(42, 100); // JS nid 42 -> blitz node 100
        flid_map.insert(99, 200); // JS nid 99 -> blitz node 200

        let json = r#"{"42":["click","mousedown"],"99":["click"]}"#;
        let result = parse_event_map(&flid_map, json);

        assert_eq!(result.len(), 2);
        assert_eq!(result[&100], vec!["click", "mousedown"]);
        assert_eq!(result[&200], vec!["click"]);
    }

    #[test]
    fn parse_event_map_missing_flid() {
        let mut flid_map = HashMap::new();
        flid_map.insert(42, 100);
        // nid 99 is NOT in the flid_map

        let json = r#"{"42":["click"],"99":["mousedown"]}"#;
        let result = parse_event_map(&flid_map, json);

        assert_eq!(result.len(), 1);
        assert_eq!(result[&100], vec!["click"]);
    }

    #[test]
    fn bridge_handler_count() {
        let mut flid_map = HashMap::new();
        flid_map.insert(10, 50);
        flid_map.insert(20, 60);

        let json = r#"{"10":["click"],"20":["mousedown","click"]}"#;
        let bridge = JsEventBridge::new(flid_map, json, None, "http://example.com/");

        assert_eq!(bridge.handler_count(), 2);
        assert!(bridge.has_js_state() == false);
        assert_eq!(bridge.event_types_for(50), Some(vec!["click".to_string()].as_slice()));
        assert_eq!(bridge.event_types_for(999), None);
    }
}
