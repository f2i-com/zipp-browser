# Live DOM Bridge Design

## Problem Statement

The current architecture runs JavaScript in the Zipp VM using `dom_shim.js`,
a pure-JS simulation of the DOM. After execution completes, the shim serializes
the virtual DOM tree to an HTML string via `__getRenderedHTML()`. The browser then
parses that HTML string into a blitz-dom tree for layout and rendering.

This is a one-shot SSR-like pipeline:

```
JS execution (dom_shim.js)  -->  HTML string  -->  blitz-dom parse  -->  layout/render
```

The fundamental limitation: **JS cannot interact with the rendered page.** Once the
HTML string is handed off, the JS VM is destroyed. There is no event loop, no event
handling, and no way for React (or any JS framework) to update the DOM in response to
user interaction.

## Target Architecture

Replace the serialization boundary with a live bridge where JS DOM operations
directly manipulate blitz-dom nodes, and blitz-dom events dispatch back to JS:

```
JS execution <---> Native Bridge (Rust) <---> blitz-dom tree --> layout/render
                        ^
                        |
                   Event dispatch
```

The VM stays alive. DOM mutations flow from JS to Rust in real time. User events flow
from blitz-dom back to JS handlers.

---

## Current Architecture (Reference)

### JS Side: dom_shim.js

The shim defines JS classes (`DOMNode`, `Element`, `Text`, `Document`) that simulate
the browser DOM API. Key characteristics:

- **Node IDs**: Each node gets a monotonic `_nid` (JS integer). Used for identity
  comparison since the VM's `indexOf` may not work reliably on object references.
- **Tree structure**: `childNodes` array, `parentNode` reference. All tree ops
  (appendChild, insertBefore, removeChild, replaceChild) are implemented in JS.
- **Attributes**: Stored in a plain `attributes` object on each Element.
- **Style**: Plain JS object (`node.style.color = "red"`).
- **Event listeners**: Stored in `_listeners` map on each node. Never actually
  dispatched by external events -- only used if JS code calls `dispatchEvent()`.
- **Serialization**: `__sn(node)` / `__sc(node)` walk the tree and produce HTML.
  `__getRenderedHTML()` serializes `doc.body` and returns it.
- **Document**: A single `doc` global with `createElement`, `createTextNode`,
  `getElementById`, `querySelector`, etc.

### Rust Side: Zipp VM

Key APIs on `ScriptState` (from `zipp-vm/src/embed.rs`):

- `compile_script(source) -> ScriptState` -- Parse and compile, returns VM ready to run.
- `run_init()` -- Execute top-level code.
- `eval_in_context(source)` -- Evaluate an expression in the script's global context.
- `set_global(name, Object)` -- Write a global variable (creates it if missing).
- `get_global(name) -> Object` -- Read a global variable.
- `call_function(name, &[Object])` -- Call a named global function.
- `call_value(Value, &[Object])` -- Call a Value (closure/function). Used for event handlers.
- `call_compiled_function(&CompiledFunctionObject, &[Object])` -- Call a function object directly.
- `vm_mut()` -- Mutable access to the VM for attaching bridges.

**Bridge pattern**: The VM supports pluggable bridges via trait objects:
- `set_http(Box<dyn HttpBridge>)` -- Already used for fetch.
- `set_local_storage(Box<dyn LocalStorageBridge>)`
- `set_db(Box<dyn DbBridge>)`
- `set_draw(Box<dyn DrawBridge>)`
- `set_layout(Box<dyn LayoutBridge>)`
- `set_input(Box<dyn InputBridge>)`

**Host calls**: Two mechanisms for JS-to-Rust communication:
1. `host.callSync(kind, argsArray)` -- Synchronous. Invokes `config.sync_host_call`
   (a `SyncHostCallFn`). Takes string kind + string args, returns JSON string that
   gets parsed back into a VM value. **This is the primary mechanism for Phase 2.**
2. `host.call(kind, argsArray, callback)` -- Async. Queues a `PendingHostCall` that
   the host drains after execution. Suitable for Phase 3.

### Rust Side: blitz-dom DocumentMutator

`DocumentMutator` (from `zipp-dom/src/mutator.rs`) wraps `&mut BaseDocument`
and provides the mutation API:

- `create_element(name: QualName, attrs: Vec<Attribute>) -> usize` -- Returns node ID.
- `create_text_node(text: &str) -> usize` -- Returns node ID.
- `create_comment_node() -> usize` -- Returns node ID.
- `append_children(parent_id, &[child_id])` -- Appends children.
- `insert_nodes_before(anchor_id, &[node_id])` -- Insert before anchor.
- `remove_node(node_id)` -- Remove from parent (keep alive).
- `remove_and_drop_node(node_id)` -- Remove and deallocate.
- `set_attribute(node_id, QualName, &str)` -- Set attribute (handles style, id, etc.).
- `clear_attribute(node_id, QualName)` -- Remove attribute.
- `set_node_text(node_id, &str)` -- Update text content of a text node.
- `set_inner_html(node_id, &str)` -- Parse HTML into subtree.
- `set_style_property(node_id, name, value)` -- Set individual CSS property.

On drop, `DocumentMutator` calls `flush()` which processes deferred operations
(stylesheet updates, image loads, title changes, etc.).

### Rust Side: Event System

`EventDriver<Handler>` processes UI events through the DOM tree:

1. `handle_ui_event(UiEvent)` -- Entry point. Hit-tests to find target node.
2. Events bubble up via `node_chain(target)` (list of ancestor node IDs).
3. `EventHandler::handle_event(chain, event, doc, event_state)` -- Called for each
   event. The handler can call `event_state.prevent_default()` or `stop_propagation()`.
4. After handler, `run_default_action(event)` applies built-in behavior (link
   navigation, form submission, text input, etc.).
5. Events: Click, PointerDown/Up/Move, KeyDown/Up, Input, Focus/Blur, Scroll, Wheel.

Currently `NoopEventHandler` is used -- events only trigger built-in DOM behavior
(link clicks, form submissions). There is no JS handler dispatch.

---

## Phase 1: Event Dispatch (Minimal Interactivity)

### Goal

Keep the current SSR pipeline but add event handling. When a user clicks a node that
has a JS event handler, call that handler in the VM.

### Design

#### 1.1 Keep the VM Alive After SSR

Currently `run_zipp_js` creates a `ScriptState`, runs it, extracts the HTML
string, and drops everything. Instead:

```
run_zipp_js(html, page_url) -> (Option<String>, Option<ScriptState>)
```

Return both the rendered HTML and the live ScriptState. Store the ScriptState
alongside the document so it persists for the page's lifetime.

#### 1.2 Track Event Handlers During SSR

The dom_shim already has `addEventListener` on DOMNode, which stores handlers in
`_listeners`. The problem is these are JS-side objects with no connection to
blitz-dom nodes.

**Approach**: During SSR, when React calls `addEventListener("click", handler)` on a
shim node, also record a mapping from `(shim_node_id, event_type)` to the handler
function. After SSR, build an `event_map` that maps blitz-dom node IDs to handler
functions.

**Node ID Mapping (SSR Phase 1)**:

The shim already assigns monotonic `_nid` values. We need to correlate these with
blitz-dom node IDs after parsing. Strategy:

1. During serialization, inject `data-flid="<_nid>"` attributes on elements that
   have event listeners.
2. After blitz-dom parses the HTML, walk the tree and build a map:
   `HashMap<u32, usize>` where key = shim _nid (from data-flid), value = blitz-dom
   node ID.
3. Combine with the JS-side listener map to produce:
   `HashMap<(usize, EventKind), Value>` -- blitz-dom node ID + event type to JS
   handler Value.

#### 1.3 Implement EventHandler for JS Dispatch

Create a `JsEventHandler` that implements the `EventHandler` trait:

```rust
struct JsEventHandler {
    state: Rc<RefCell<ScriptState>>,
    // blitz_node_id -> [(event_kind, js_handler_value)]
    event_map: HashMap<usize, Vec<(DomEventKind, Value)>>,
}

impl EventHandler for JsEventHandler {
    fn handle_event(
        &mut self,
        chain: &[usize],
        event: &mut DomEvent,
        doc: &mut dyn Document,
        event_state: &mut EventState,
    ) {
        let kind = event.data.kind();
        // Walk the chain (target -> ancestors) for bubbling
        for &node_id in chain {
            if let Some(handlers) = self.event_map.get(&node_id) {
                for (k, handler_val) in handlers {
                    if *k == kind {
                        // Build a JS event object
                        let js_event = build_js_event_object(&event, node_id);
                        // Call the handler
                        let mut state = self.state.borrow_mut();
                        let _ = state.call_value(*handler_val, &[js_event]);
                        // Check if preventDefault was called
                        // (would need to read a flag from the event object)
                    }
                }
            }
            if event_state.propagation_is_stopped() {
                break;
            }
        }
    }
}
```

#### 1.4 Re-render After Event Handling

After a JS handler runs, it may have mutated the virtual DOM (e.g., React state
update triggers re-render). We need to:

1. Run pending microtasks / timer queue in the VM.
2. Extract updated HTML via `eval_in_context("__getRenderedHTML()")`.
3. Re-parse and replace the blitz-dom tree.
4. Request a redraw from the shell.

This is expensive (full re-render) but correct. Phase 2 eliminates this.

#### 1.5 Limitations

- Full re-render on every event (slow for complex pages).
- Event object is a simple JS object, not a full Event with methods.
- No continuous execution (setTimeout, requestAnimationFrame are no-ops).
- Node ID mapping relies on data attributes (fragile if HTML is restructured).

---

## Phase 2: Live DOM Bridge (Native Calls)

### Goal

Replace `dom_shim.js` with a thin JS layer whose methods call into Rust via
`host.callSync`. JS DOM operations directly create/modify/remove blitz-dom nodes.
No HTML serialization step.

### Design

#### 2.1 Bridge Protocol via host.callSync

The VM already supports `host.callSync(kind, argsArray) -> jsonResult`. This is
synchronous and returns a parsed value -- exactly what DOM operations need. Each DOM
operation becomes a host call:

| JS Call | host.callSync Kind | Args | Returns |
|---|---|---|---|
| `document.createElement(tag)` | `"dom.createElement"` | `[tag]` | `nodeId` (integer) |
| `document.createTextNode(text)` | `"dom.createTextNode"` | `[text]` | `nodeId` |
| `document.createComment(data)` | `"dom.createComment"` | `[data]` | `nodeId` |
| `document.createDocumentFragment()` | `"dom.createFragment"` | `[]` | `fragmentId` |
| `node.appendChild(child)` | `"dom.appendChild"` | `[parentId, childId]` | `childId` |
| `node.insertBefore(new, ref)` | `"dom.insertBefore"` | `[parentId, newId, refId]` | `newId` |
| `node.removeChild(child)` | `"dom.removeChild"` | `[parentId, childId]` | `childId` |
| `node.replaceChild(new, old)` | `"dom.replaceChild"` | `[parentId, newId, oldId]` | `oldId` |
| `node.setAttribute(k, v)` | `"dom.setAttribute"` | `[nodeId, name, value]` | `null` |
| `node.removeAttribute(k)` | `"dom.removeAttribute"` | `[nodeId, name]` | `null` |
| `node.textContent = v` | `"dom.setTextContent"` | `[nodeId, text]` | `null` |
| `node.style.setProperty(k,v)` | `"dom.setStyle"` | `[nodeId, prop, value]` | `null` |
| `node.addEventListener(t,fn)` | (JS-side only) | -- | -- |
| `node.removeEventListener(t,fn)` | (JS-side only) | -- | -- |
| `document.getElementById(id)` | `"dom.getElementById"` | `[id]` | `nodeId or null` |
| `document.querySelector(sel)` | `"dom.querySelector"` | `[contextId, sel]` | `nodeId or null` |

Note: `addEventListener`/`removeEventListener` are JS-side only -- they store
handler references in the JS wrapper object. The bridge does not need to know about
them until an event fires (Phase 1's event map approach).

#### 2.2 Node ID as the Sole Handle

blitz-dom uses `usize` node IDs (slab indices). These become the canonical handle:

- JS sees node IDs as integers.
- Each JS wrapper object (Element, Text, etc.) stores its `_blitzId`.
- All bridge calls use these integer IDs.
- No separate mapping needed -- the blitz-dom slab index IS the ID.

```javascript
class Element {
    constructor(blitzId, tag) {
        this._blitzId = blitzId;
        this.tagName = tag.toUpperCase();
        this._listeners = {};
        // ... minimal property cache
    }
    appendChild(child) {
        host.callSync("dom.appendChild", [this._blitzId, child._blitzId]);
        return child;
    }
    setAttribute(name, value) {
        host.callSync("dom.setAttribute", [this._blitzId, name, String(value)]);
    }
    // ...
}
```

#### 2.3 Rust-Side Bridge Handler

The `SyncHostCallFn` handler dispatches DOM operations to a `DocumentMutator`:

```rust
// Shared state between the VM and the bridge handler
struct DomBridgeState {
    document: Rc<RefCell<BaseDocument>>,
    // Fragment storage (fragments are not in the document tree)
    fragments: HashMap<u32, Vec<usize>>,
    next_fragment_id: u32,
}

fn create_dom_bridge(state: Arc<Mutex<DomBridgeState>>) -> SyncHostCallFn {
    Arc::new(move |kind: &str, args: &[String]| -> Result<String, String> {
        let mut state = state.lock().unwrap();
        match kind {
            "dom.createElement" => {
                let tag = args.get(0).ok_or("missing tag")?;
                let name = QualName::new(None, ns!(html), LocalName::from(tag.as_str()));
                let mut mutator = DocumentMutator::new(&mut state.document.borrow_mut());
                let id = mutator.create_element(name, vec![]);
                Ok(id.to_string())
            }
            "dom.appendChild" => {
                let parent_id: usize = args[0].parse().map_err(|e| format!("{e}"))?;
                let child_id: usize = args[1].parse().map_err(|e| format!("{e}"))?;
                let mut doc = state.document.borrow_mut();
                let mut mutator = DocumentMutator::new(&mut doc);
                mutator.append_children(parent_id, &[child_id]);
                Ok(child_id.to_string())
            }
            "dom.setAttribute" => {
                let node_id: usize = args[0].parse().map_err(|e| format!("{e}"))?;
                let name = &args[1];
                let value = &args[2];
                let qname = QualName::new(None, ns!(), LocalName::from(name.as_str()));
                let mut doc = state.document.borrow_mut();
                let mut mutator = DocumentMutator::new(&mut doc);
                mutator.set_attribute(node_id, qname, value);
                Ok("null".into())
            }
            // ... other operations
            _ => Err(format!("unknown DOM bridge call: {kind}")),
        }
    })
}
```

#### 2.4 Borrow Conflict: VM Execution vs Document Mutation

**Critical challenge**: The `SyncHostCallFn` is called from within VM execution.
At that point, the VM owns a mutable reference to its own state. If the bridge
handler also needs to borrow the document, and the document is stored in the same
structure, we get a borrow conflict.

**Solution**: The document and the VM must be in separate `Rc<RefCell<_>>` or
`Arc<Mutex<_>>` containers. The `SyncHostCallFn` captures a reference to the
document container. The VM is separate. This works because `SyncHostCallFn` is
`Arc<dyn Fn>` and is called from inside the VM's execution loop, which does not
hold a borrow on the document.

```
+------------------+          +---------------------+
|  ScriptState     |          |  BaseDocument       |
|  (Zipp VM)  |---sync-->|  (via Rc<RefCell>)  |
|                  |  host    |                     |
|  config.sync_    |  call    |  DocumentMutator    |
|  host_call ------+--------->|  created per call   |
+------------------+          +---------------------+
```

#### 2.5 Document Fragment Handling

`DocumentFragment` is a virtual container that is not part of the blitz-dom tree.
When children are appended to a fragment, they are held in a Vec. When the fragment
is appended to a real node, its children are moved into the tree.

Option A: Emulate entirely in JS (keep fragment as JS-only container with child
`_blitzId` list, flatten to individual `appendChild` calls when attached).

Option B: Track in bridge state with a separate `fragments` map.

**Recommendation**: Option A is simpler and matches React's usage pattern. React
uses fragments mainly for batching, and the individual DOM ops are cheap since
they go directly to blitz-dom.

#### 2.6 JS-Side Bridge Shim (dom_bridge_shim.js)

A slimmed-down replacement for `dom_shim.js`. Instead of maintaining a virtual DOM
tree, each object is a thin wrapper around a blitz-dom node ID:

```javascript
class Element {
    constructor(blitzId, tag) {
        this._blitzId = blitzId;
        this.nodeType = 1;
        this.tagName = tag.toUpperCase();
        this.localName = tag.toLowerCase();
        this._listeners = {};
        this._childWrappers = []; // track JS wrapper refs for childNodes
    }

    appendChild(child) {
        if (child.nodeType === 11) {
            // Fragment: append each child
            var kids = child._childWrappers.slice();
            for (var i = 0; i < kids.length; i++) this.appendChild(kids[i]);
            child._childWrappers = [];
            return child;
        }
        if (child._parent) child._parent.removeChild(child);
        host.callSync("dom.appendChild", [this._blitzId, child._blitzId]);
        child._parent = this;
        this._childWrappers.push(child);
        return child;
    }

    get textContent() {
        return host.callSync("dom.getTextContent", [this._blitzId]);
    }
    set textContent(v) {
        host.callSync("dom.setTextContent", [this._blitzId, String(v)]);
    }

    setAttribute(name, value) {
        host.callSync("dom.setAttribute", [this._blitzId, name, String(value)]);
    }

    // ... etc
}
```

#### 2.7 Read-Back Operations

Some DOM operations read state (getElementById, querySelector, textContent getter,
childNodes, getBoundingClientRect). These also go through the bridge:

- `getElementById` / `querySelector`: Return a node ID or null. JS side creates
  a wrapper object for the returned ID (or returns a cached one).
- `textContent` (getter): Bridge reads text content from blitz-dom tree.
- `getBoundingClientRect`: Returns layout-computed bounds from blitz-dom. Requires
  that layout has been computed. May need to trigger a synchronous layout pass.

**Wrapper cache**: To ensure `node.parentNode === otherRef` identity comparisons
work, maintain a `Map<blitzId, wrapper>` in JS. When the bridge returns a node ID,
look up or create the wrapper.

#### 2.8 Event Handling in Phase 2

With the live bridge, event handling becomes cleaner:

1. JS wrappers store listeners in `_listeners` (same as dom_shim.js).
2. The Rust `EventHandler` receives a click event on blitz-dom node X.
3. Bridge looks up the JS wrapper for node X (via the wrapper cache / node ID map).
4. Calls the listener function via `state.call_value(handler, &[event_obj])`.
5. Any DOM mutations inside the handler go through the bridge immediately.
6. After the handler returns, request a re-layout/re-render.

**No full re-parse needed** -- mutations are already applied to blitz-dom.

#### 2.9 Batching and Performance

`host.callSync` has overhead per call (string serialization, mutex acquisition,
function dispatch). React can generate hundreds of DOM ops per render.

**Mitigation strategies**:

1. **Batch protocol**: Add a `"dom.batch"` call kind that accepts a JSON array of
   operations and executes them all in one bridge crossing:
   ```javascript
   host.callSync("dom.batch", [JSON.stringify([
       ["createElement", "div"],
       ["setAttribute", 0, "class", "foo"],
       ["appendChild", 5, 0]
   ])]);
   ```
   Returns an array of results. Node IDs in the batch can reference earlier results
   by index (e.g., "result of op 0" = the created div's ID).

2. **Write-only fast path**: Most DOM ops (setAttribute, appendChild, etc.) don't
   need a return value. A `"dom.mutate"` batch call that returns nothing could skip
   the JSON result serialization.

3. **Property caching**: Cache frequently-read properties (tagName, id, className)
   on the JS wrapper to avoid bridge round-trips for reads.

---

## Phase 3: Full Integration (Event Loop + Async)

### Goal

Continuous JS execution with a real event loop, enabling:
- `setTimeout` / `setInterval` that actually fire
- `requestAnimationFrame` for animation
- React concurrent mode with proper scheduling
- Async operations (fetch with callbacks/promises)

### Design

#### 3.1 Persistent VM Thread

The Zipp VM runs on a dedicated thread. The main thread (blitz-dom + rendering)
communicates with it via channels:

```
Main Thread                          VM Thread
+-----------+    events             +------------+
|  blitz-dom| ---- channel -------> | Zipp  |
|  + render |                       | VM + event |
|           | <--- dom mutations -- | loop       |
+-----------+    (channel or        +------------+
                  shared state)
```

**Option A: Shared Document** -- Both threads access the BaseDocument through
Arc<Mutex<BaseDocument>>. The VM thread locks it during DOM mutations. The main
thread locks it during layout/render. Simple but limits parallelism.

**Option B: Mutation Queue** -- The VM thread builds a queue of DOM mutations
(create, append, setAttribute, etc.). The main thread drains and applies them
between frames. Better parallelism but adds latency (mutations are not visible
until next frame).

**Recommendation**: Start with Option A (shared mutex). It matches the browser's
single-threaded DOM model and avoids consistency issues. Optimize to Option B later
if the mutex becomes a bottleneck.

#### 3.2 Event Loop Implementation

The VM thread runs an event loop:

```rust
loop {
    // 1. Process pending events from the main thread
    while let Ok(event) = event_rx.try_recv() {
        dispatch_event_to_js(&mut state, &event_map, event);
    }

    // 2. Run expired timers
    let now = Instant::now();
    while let Some(timer) = timer_queue.peek() {
        if timer.deadline <= now {
            let timer = timer_queue.pop().unwrap();
            state.call_value(timer.callback, &[]);
        } else {
            break;
        }
    }

    // 3. Drain microtask queue
    drain_microtasks(&mut state);

    // 4. requestAnimationFrame callbacks
    if raf_pending {
        for cb in raf_callbacks.drain(..) {
            state.call_value(cb, &[Object::Float(now_ms)]);
        }
        raf_pending = false;
    }

    // 5. If no work pending, sleep until next timer or event
    let sleep_until = timer_queue.peek().map(|t| t.deadline);
    match sleep_until {
        Some(deadline) => {
            let timeout = deadline.saturating_duration_since(Instant::now());
            event_rx.recv_timeout(timeout);
        }
        None => {
            event_rx.recv(); // block until event
        }
    }
}
```

#### 3.3 Timer Implementation

Replace the current timer stubs with real implementations backed by the event loop:

```javascript
// In the bridge shim:
var __nextTimerId = 1;
function setTimeout(fn, delay) {
    var id = __nextTimerId++;
    host.callSync("timer.set", [id, delay || 0, "timeout"]);
    __timerCallbacks[id] = fn;
    return id;
}
function setInterval(fn, delay) {
    var id = __nextTimerId++;
    host.callSync("timer.set", [id, delay || 0, "interval"]);
    __timerCallbacks[id] = fn;
    return id;
}
function clearTimeout(id) {
    host.callSync("timer.clear", [id]);
    delete __timerCallbacks[id];
}
function requestAnimationFrame(fn) {
    var id = __nextTimerId++;
    host.callSync("timer.raf", [id]);
    __timerCallbacks[id] = fn;
    return id;
}
```

When the event loop fires a timer, it calls back into JS to invoke the stored
callback.

#### 3.4 React Reconciliation

With Phases 1-3 complete, React's reconciler should work naturally:

1. React calls `createElement`, `appendChild`, etc. -- these go through the bridge.
2. React's scheduler uses `MessageChannel` (or setTimeout fallback) for async work
   batching -- the event loop handles these.
3. State updates trigger re-renders, which produce DOM mutations via the bridge.
4. Events from user interaction trigger React's event system, which may schedule
   state updates.

**React-specific considerations**:
- React expects `requestAnimationFrame` for scheduling. Our event loop provides it.
- React uses `performance.now()` for time-slicing. We provide a real implementation.
- React's synthetic event system attaches listeners to the root. Our event bubbling
  delivers events to the root, where React's handler dispatches them.

#### 3.5 Layout Queries

`getBoundingClientRect()` and other layout queries require layout to be computed.
In Phase 3, layout runs on the main thread asynchronously from JS execution.

**Strategy**: When JS calls `getBoundingClientRect`, the bridge:
1. Triggers a synchronous layout pass (if layout is dirty).
2. Reads the computed layout rect from the node.
3. Returns the result.

This is expensive but necessary for correctness. Browsers have the same challenge
(forced reflow).

---

## Implementation Order

### Phase 1 (estimated complexity: medium)

1. Modify `run_zipp_js` to return `Option<ScriptState>` alongside the HTML.
2. Store the ScriptState in the page's state (alongside the document).
3. Add `data-flid` attributes during serialization for nodes with listeners.
4. After parse, build the shim-nid to blitz-dom-id mapping.
5. Extract listener map from the VM via `eval_in_context`.
6. Implement `JsEventHandler` and wire it into `EventDriver`.
7. After event handler runs, re-run `__getRenderedHTML()` and re-parse.

### Phase 2 (estimated complexity: high)

1. Write `dom_bridge_shim.js` -- thin wrapper classes around bridge calls.
2. Implement the `SyncHostCallFn` bridge handler for DOM operations.
3. Solve the borrow architecture (separate Rc<RefCell> for document vs VM).
4. Implement node wrapper cache in JS (blitzId -> wrapper).
5. Port event handler dispatch to use the new wrapper system.
6. Implement read-back operations (getElementById, querySelector, textContent).
7. Add batching protocol for performance.
8. Test with a simple React app (counter, todo list).

### Phase 3 (estimated complexity: high)

1. Move VM to dedicated thread with event channels.
2. Implement the event loop (timers, microtasks, rAF).
3. Implement real setTimeout/setInterval/requestAnimationFrame.
4. Wire up event dispatch across the thread boundary.
5. Add layout query support (forced reflow).
6. Test with React concurrent mode and animations.

---

## Key Risks and Open Questions

1. **host.callSync performance**: Each DOM op requires string serialization +
   deserialization. For 500+ ops per React render, this could be slow. Batching
   mitigates but adds complexity. Need to benchmark.

2. **Borrow architecture**: The SyncHostCallFn is called from inside the VM's
   execution. The handler must not re-enter the VM. It must access the document
   through a separate borrow path. The Rc<RefCell> approach works but adds
   runtime borrow-checking overhead.

3. **DocumentFragment**: React relies heavily on fragments. The JS-only emulation
   (Option A in 2.5) means fragment operations don't go through the bridge, which
   is probably fine since fragments are virtual containers. But it means
   `fragment.childNodes` and similar queries only see the JS-side state.

4. **GC interaction**: JS wrapper objects hold blitz-dom node IDs. If a wrapper is
   GC'd in the Zipp VM, we need to know whether to drop the blitz-dom node.
   For nodes in the document tree, the tree owns them. For detached nodes, the
   wrapper is the only reference -- if GC'd, the blitz-dom node leaks. May need
   a weak-reference or explicit dispose pattern.

5. **Thread safety in Phase 3**: BaseDocument is not Send/Sync. Moving to
   Arc<Mutex> requires either making it thread-safe or using a different
   synchronization primitive. Alternatively, keep everything on one thread and
   use cooperative multitasking (run the event loop in the same thread as
   rendering, yielding between frames).

6. **React event delegation**: React 17+ attaches event listeners to the root
   container, not to individual elements. This means the event bridge only needs
   to handle events at the root level, which simplifies the handler lookup
   significantly. But we need to construct React-compatible event objects.
