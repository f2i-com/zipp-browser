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

use zipp_vm::embed::{JsValue, ScriptState};
use zipp_dom::html_parser::HtmlDocument;
use zipp_dom::traits_events::{DomEvent, EventState};
use zipp_dom::{BaseDocument, Document, EventHandler, LocalName};

// ---------------------------------------------------------------------------
// flid map construction
// ---------------------------------------------------------------------------

/// Walk every node in the parsed blitz-dom document and collect the
/// `data-flid` attribute values.  Returns a map from JS node ID (the flid
/// integer) to the blitz-dom slab node ID.
pub fn build_flid_map(doc: &HtmlDocument) -> HashMap<u32, usize> {
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
}

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
    ) -> Self {
        let handler_map = parse_event_map(&flid_map, event_map_json);
        // Build reverse map: blitz_node_id -> JS flid
        let reverse_flid: HashMap<usize, u32> = flid_map
            .iter()
            .map(|(&flid, &blitz_id)| (blitz_id, flid))
            .collect();
        JsEventBridge {
            handler_map,
            reverse_flid,
            js_state,
            pending_rerender: None,
        }
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

impl EventHandler for JsEventBridge {
    fn handle_event(
        &mut self,
        chain: &[usize],
        event: &mut DomEvent,
        _doc: &mut dyn Document,
        event_state: &mut EventState,
    ) {
        let event_name = event.data.name();

        // Walk the bubble chain (target first, then ancestors).
        for &node_id in chain {
            if let Some(types) = self.handler_map.get(&node_id) {
                if types.iter().any(|t| t == event_name) {
                    // Look up the JS flid for this blitz-dom node.
                    let flid = self.reverse_flid.get(&node_id).copied();

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
                                JsValue::String(event_name.to_string()),
                            ],
                        );
                        if let Err(e) = &dispatched {
                            eprintln!("[EVENT] handler for \"{event_name}\" failed: {e}");
                        }

                        // For click events: settle the page, then see whether
                        // the handler changed the DOM.
                        if event_name == "click" {
                            // React's setState is async — the handler has only
                            // queued the re-render at this point.
                            let _ = state.call_global(
                                "__zippDrainQueues",
                                &[JsValue::Number(50.0)],
                            );
                            if let Ok(JsValue::String(new_html)) =
                                state.call_global("__getRenderedHTML", &[])
                            {
                                if !new_html.is_empty() {
                                    eprintln!(
                                        "[EVENT] DOM updated after click ({} bytes)",
                                        new_html.len()
                                    );
                                    self.pending_rerender = Some(new_html);
                                }
                            }
                        }
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
    }

    fn take_pending_rerender(&mut self) -> Option<String> {
        self.pending_rerender.take()
    }
}

// ---------------------------------------------------------------------------
// JSON parsing (minimal, no serde dependency)
// ---------------------------------------------------------------------------

/// Parse the event map JSON and combine it with the flid map to produce a
/// `blitz_node_id -> [event_type]` lookup.
///
/// The JSON format is: `{"42": ["click","mousedown"], "99": ["click"]}`
/// We do minimal hand-parsing to avoid pulling in serde_json.
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
        let bridge = JsEventBridge::new(flid_map, json, None);

        assert_eq!(bridge.handler_count(), 2);
        assert!(bridge.has_js_state() == false);
        assert_eq!(bridge.event_types_for(50), Some(vec!["click".to_string()].as_slice()));
        assert_eq!(bridge.event_types_for(999), None);
    }
}
