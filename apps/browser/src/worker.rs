//! Web Workers: each runs on a thread of its own, in a zipp-vm of its own.
//!
//! The page's side is `worker_page.js` (`new Worker(url)`), the worker's
//! global scope `worker_shim.js`. Messages cross as structured clones encoded
//! as JSON (`clone.js`), over channels: the page posts straight to the
//! worker's thread, which sleeps until a message comes or its next timer is
//! due; a worker's messages wait for the page's thread, which the worker wakes
//! (`set_waker`), and the event bridge hands them to the page
//! (`JsEventBridge::run_tasks`).
//!
//! A module worker's graph is fetched and linked on its own thread as it
//! starts, as a page's is (esm.rs). Its console lines go to the page's console,
//! marked as the worker's.
//!
//! `terminate()` stops a worker between tasks. zipp-vm cannot interrupt
//! running script, so a worker stuck in a loop keeps its thread busy until the
//! browser exits, though the page no longer hears from it.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use zipp_vm::embed::{HostCtx, JsValue, ScriptState, compile_script};

use crate::console::{self, Level};

enum ToWorker {
    Message(String),
    Terminate,
}

/// What a worker sends its page: a message (a clone), or an uncaught error.
pub(crate) enum FromWorker {
    Message(String),
    Error(String),
}

struct Handle {
    to: Sender<ToWorker>,
    from: Receiver<FromWorker>,
    /// The page that started it (`new_page`).
    page: u64,
}

thread_local! {
    /// The workers this thread's page started, by id.
    static WORKERS: RefCell<HashMap<u32, Handle>> = RefCell::new(HashMap::new());
    static NEXT_ID: Cell<u32> = const { Cell::new(1) };
    /// Which page this thread is running: a navigation builds the next page,
    /// its workers starting, before the last page's document is dropped, so a
    /// page's going must stop its own workers only.
    static PAGE: Cell<u64> = const { Cell::new(0) };
    /// Whether this thread's page may start workers: the window's may, not a
    /// page prerendered in the background, which would start them for nothing.
    static ALLOWED: Cell<bool> = const { Cell::new(false) };
    /// On a worker's thread: where its messages go, and whether it closed itself.
    static OUTBOX: RefCell<Option<Sender<FromWorker>>> = const { RefCell::new(None) };
    static CLOSED: Cell<bool> = const { Cell::new(false) };
}

static WAKER: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// How a worker wakes the window's thread when it has sent something.
pub(crate) fn set_waker(wake: Box<dyn Fn() + Send + Sync>) {
    let _ = WAKER.set(wake);
}

/// `ZIPP_DEBUG_WORKERS=1` logs the messages that cross.
fn debug() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("ZIPP_DEBUG_WORKERS").is_some())
}

fn wake() {
    if let Some(wake) = WAKER.get() {
        wake();
    }
}

/// A new page on this thread: the workers it starts are its own.
pub(crate) fn new_page() {
    PAGE.set(PAGE.get() + 1);
}

/// The page this thread is running, for its event bridge to remember.
pub(crate) fn current_page() -> u64 {
    PAGE.get()
}

/// Let this thread's page start workers (the window's thread).
pub(crate) fn allow_on_this_thread() {
    ALLOWED.set(true);
}

/// `new Worker(url)`: its id.
pub(crate) fn start(url: &str, module: bool) -> Result<u32, String> {
    let id = NEXT_ID.with(|n| {
        let id = n.get();
        n.set(id + 1);
        id
    });
    if !ALLOWED.get() {
        // A worker that never starts: the page still gets an object to talk to.
        return Ok(id);
    }
    eprintln!("[WORKER] {id}: starting {url} ({})", if module { "module" } else { "classic" });
    let (to_tx, to_rx) = mpsc::channel();
    let (from_tx, from_rx) = mpsc::channel();
    let url = url.to_string();
    std::thread::Builder::new()
        .name(format!("zipp-worker-{id}"))
        // Compiling a module graph recurses over its syntax tree on the native
        // stack, as the page's does (`run_zipp_js`).
        .stack_size(64 * 1024 * 1024)
        .spawn(move || run(&url, module, to_rx, from_tx))
        .map_err(|e| format!("Error: cannot start a worker: {e}"))?;
    WORKERS.with(|w| w.borrow_mut().insert(id, Handle { to: to_tx, from: from_rx, page: PAGE.get() }));
    Ok(id)
}

/// `worker.postMessage(data)`, `data` already encoded.
pub(crate) fn post(id: u32, data: String) {
    if debug() {
        eprintln!("[WORKER] {id} posted {}", data.chars().take(60).collect::<String>());
    }
    WORKERS.with(|w| {
        if let Some(handle) = w.borrow().get(&id) {
            let _ = handle.to.send(ToWorker::Message(data));
        }
    });
}

/// `worker.terminate()`.
pub(crate) fn terminate(id: u32) {
    if let Some(handle) = WORKERS.with(|w| w.borrow_mut().remove(&id)) {
        let _ = handle.to.send(ToWorker::Terminate);
    }
}

/// Page `page` is going: so are its workers.
pub(crate) fn terminate_page(page: u64) {
    let gone: Vec<Handle> = WORKERS.with(|w| {
        let mut w = w.borrow_mut();
        let ids: Vec<u32> = w.iter().filter(|(_, h)| h.page == page).map(|(&id, _)| id).collect();
        ids.into_iter().filter_map(|id| w.remove(&id)).collect()
    });
    for handle in gone {
        let _ = handle.to.send(ToWorker::Terminate);
    }
}

/// What this thread's workers have sent since last asked.
pub(crate) fn take_messages() -> Vec<(u32, FromWorker)> {
    WORKERS.with(|w| {
        let mut out = Vec::new();
        for (&id, handle) in w.borrow().iter() {
            while let Ok(message) = handle.from.try_recv() {
                out.push((id, message));
            }
        }
        out
    })
}

/// A worker's thread: build its scope and script, run it, then answer
/// messages and timers until it is terminated or closes itself.
fn run(url: &str, module: bool, inbox: Receiver<ToWorker>, outbox: Sender<FromWorker>) {
    console::record_this_thread();
    console::label_this_thread("worker");
    OUTBOX.with(|o| *o.borrow_mut() = Some(outbox.clone()));
    let report = |message: String| {
        console::push(Level::Error, message.clone());
        let _ = outbox.send(FromWorker::Error(message));
        wake();
    };
    let program = match program(url, module) {
        Ok(program) => program,
        Err(e) => return report(e),
    };
    eprintln!("[WORKER] {url}: compiling {} bytes", program.len());
    let t0 = std::time::Instant::now();
    let mut state = match compile_script(&program) {
        Ok(state) => state,
        Err(e) => return report(format!("SyntaxError: {e}")),
    };
    state.set_host_call_ctx(Box::new(host_call));
    state.set_unhandled_rejection_sink(Box::new(|reason| {
        console::push(Level::Error, format!("Uncaught (in promise) {reason}"));
    }));
    if let Err(e) = state.run_init() {
        report(format!("Uncaught {e}"));
    }
    settle(&mut state);
    eprintln!("[WORKER] {url}: running ({}ms to start)", t0.elapsed().as_millis());
    while !CLOSED.get() {
        let wait = match state.call_global("__zippWorkerNextTimer", &[]) {
            Ok(JsValue::Number(ms)) if ms >= 0.0 => Duration::from_millis(ms as u64),
            _ => Duration::from_secs(3600),
        };
        let mut received = None;
        match inbox.recv_timeout(wait) {
            Ok(ToWorker::Message(data)) => {
                received = Some(std::time::Instant::now());
                if debug() {
                    eprintln!("[WORKER] {url} <- {}", data.chars().take(200).collect::<String>());
                }
                if let Err(e) = state.call_global("__zippWorkerReceive", &[JsValue::String(data)]) {
                    report(format!("Uncaught {e}"));
                }
            }
            Ok(ToWorker::Terminate) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }
        settle(&mut state);
        // How long the message kept the worker: its handler, and the promises
        // and timers it set going.
        if let Some(received) = received.filter(|_| debug()) {
            eprintln!("[WORKER] {url}: busy {}ms", received.elapsed().as_millis());
        }
    }
}

/// The worker's whole program: its scope, then its script -- a classic one
/// as fetched, a module with its graph linked.
fn program(url: &str, module: bool) -> Result<String, String> {
    let scope = include_str!("worker_shim.js").replace("__WORKER_URL__", &crate::js_quoted(url));
    let prelude = format!("{}\n{}\n{scope}\n", include_str!("console_shim.js"), include_str!("clone.js"));
    if !module {
        let source = crate::fetch(url).map_err(|e| format!("worker script {url} failed to load: {e}"))?;
        return Ok(format!("{prelude}\n;\n{source}"));
    }
    let linked = crate::esm::link_graph(&[url.to_string()], &crate::fetch, &crate::resolve);
    for (failed, e) in &linked.failures {
        console::push(Level::Error, format!("module {failed} failed to load: {e}"));
    }
    if linked.module_count == 0 {
        return Err(format!("worker module {url} failed to load"));
    }
    Ok(format!("{prelude}\n{}\n{}", crate::esm::MODULE_RUNTIME_JS, linked.source))
}

/// Run what is due -- promise jobs, timers -- until the worker is quiet, and
/// pass on what it logged.
fn settle(state: &mut ScriptState) {
    let mut quiet = 0;
    for _ in 0..200 {
        state.run_microtasks();
        let ran = matches!(state.call_global("__zippWorkerRunTimers", &[]), Ok(JsValue::Number(n)) if n > 0.0);
        quiet = if ran { 0 } else { quiet + 1 };
        if quiet >= 2 {
            break;
        }
    }
    for line in state.take_output() {
        console::push(Level::Log, line);
    }
    for line in state.take_errput() {
        console::push(Level::Error, line);
    }
}

/// A worker's host calls: its messages out, then whatever a page may ask.
fn host_call(ctx: &mut dyn HostCtx, kind: &str, args: &[String]) -> Result<String, String> {
    let send = |message: FromWorker| {
        OUTBOX.with(|o| {
            if let Some(outbox) = o.borrow().as_ref() {
                let _ = outbox.send(message);
            }
        });
        wake();
    };
    match kind {
        "worker_reply" => {
            let data = args.first().cloned().unwrap_or_default();
            if debug() {
                eprintln!("[WORKER] -> {}", data.chars().take(200).collect::<String>());
            }
            send(FromWorker::Message(data))
        }
        "worker_error" => send(FromWorker::Error(args.first().cloned().unwrap_or_default())),
        "worker_close" => CLOSED.set(true),
        "worker_start" => return Err("Error: a worker cannot start workers yet".into()),
        _ => return crate::browser_host_call_ctx(ctx, kind, args),
    }
    Ok(String::new())
}

/// `new URL(input, base)`, for a worker's scope: its parts, as JSON.
pub(crate) fn url_parts(input: &str, base: &str) -> Result<String, String> {
    let parsed = if base.is_empty() {
        url::Url::parse(input)
    } else {
        url::Url::parse(base).and_then(|b| b.join(input))
    }
    .map_err(|e| format!("TypeError: Invalid URL: {e}"))?;
    let port = parsed.port().map(|p| p.to_string()).unwrap_or_default();
    let hostname = parsed.host_str().unwrap_or("").to_string();
    let host = if port.is_empty() { hostname.clone() } else { format!("{hostname}:{port}") };
    Ok(serde_json::json!({
        "href": parsed.as_str(),
        "origin": parsed.origin().ascii_serialization(),
        "protocol": format!("{}:", parsed.scheme()),
        "username": parsed.username(),
        "password": parsed.password().unwrap_or(""),
        "host": host,
        "hostname": hostname,
        "port": port,
        "pathname": parsed.path(),
        "search": parsed.query().map(|q| format!("?{q}")).unwrap_or_default(),
        "hash": parsed.fragment().map(|f| format!("#{f}")).unwrap_or_default(),
    })
    .to_string())
}
