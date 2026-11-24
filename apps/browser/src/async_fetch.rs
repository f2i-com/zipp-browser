//! Concurrent background HTTP fetch pool for `window.fetch()`.
//!
//! Previously `window.fetch()` executed `reqwest::blocking::Client` synchronously
//! on the UI/VM thread, so SPAs that issue many parallel requests (e.g.
//! `Promise.all` / `mapConcurrent` across 60+ API calls on FormLogic `/demo`)
//! serialized every network round-trip and froze the UI for tens of seconds.
//!
//! This module runs HTTP/HTTPS `fetch()` calls concurrently on a persistent
//! 8-thread worker pool (sharing `HTTP_CLIENT` keep-alive connections), queues
//! completed responses per document scope, and wakes the shell event loop via
//! `crate::worker::wake()` so `JsEventBridge::run_tasks()` resolves their JS
//! `Promise`s without blocking rendering or user interaction.

use std::cell::Cell;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, LazyLock, Mutex};

const POOL_THREADS: usize = 8;

type JobFn = Box<dyn FnOnce() -> Result<String, String> + Send + 'static>;
pub(crate) type FetchOutcome = (u64, Result<String, String>);

struct Job {
    scope: u64,
    req_id: u64,
    run: JobFn,
}

struct PoolQueue {
    jobs: VecDeque<Job>,
}

struct Results {
    completed: HashMap<u64, Vec<FetchOutcome>>,
    pending: HashMap<u64, usize>,
}

static NEXT_SCOPE: AtomicU64 = AtomicU64::new(1);
static NEXT_REQ_ID: AtomicU64 = AtomicU64::new(1);

static RESULTS: LazyLock<Mutex<Results>> = LazyLock::new(|| {
    Mutex::new(Results {
        completed: HashMap::new(),
        pending: HashMap::new(),
    })
});

static POOL: LazyLock<(Mutex<PoolQueue>, Condvar)> = LazyLock::new(|| {
    let pair = (
        Mutex::new(PoolQueue {
            jobs: VecDeque::new(),
        }),
        Condvar::new(),
    );
    // Spawn persistent worker threads on first use.
    for i in 0..POOL_THREADS {
        let _ = std::thread::Builder::new()
            .name(format!("zipp-fetch-{i}"))
            .spawn(worker_loop);
    }
    pair
});

fn worker_loop() {
    let (lock, cvar) = &*POOL;
    loop {
        let job = {
            let mut q = lock.lock().unwrap_or_else(|e| e.into_inner());
            while q.jobs.is_empty() {
                q = cvar.wait(q).unwrap_or_else(|e| e.into_inner());
            }
            q.jobs.pop_front()
        };
        let Some(job) = job else { continue };
        // Skip jobs whose document scope was already torn down so dead pages do
        // not starve the shared fetch pool.
        let alive = RESULTS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pending
            .contains_key(&job.scope);
        if !alive {
            continue;
        }
        let res = (job.run)();
        {
            let mut state = RESULTS.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(count) = state.pending.get_mut(&job.scope) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    state.pending.remove(&job.scope);
                }
                state
                    .completed
                    .entry(job.scope)
                    .or_default()
                    .push((job.req_id, res));
            }
            // If `job.scope` was already dropped via `drop_scope`, discard silently.
        }
        crate::worker::wake();
    }
}

thread_local! {
    /// Whether this thread may dispatch `window.fetch()` onto the async pool.
    static ALLOWED: Cell<bool> = const { Cell::new(false) };
    /// Active document scope ID on this thread (0 = none / sync fallback).
    static CURRENT_SCOPE: Cell<u64> = const { Cell::new(0) };
    /// Scope created for the most recent `run_zipp_js` call, hand-off to `JsEventBridge`.
    static LAST_SCOPE: Cell<u64> = const { Cell::new(0) };
}

/// Enable concurrent async `fetch()` on this thread (window thread & `--screenshot`).
pub(crate) fn allow_on_this_thread() {
    ALLOWED.set(true);
}

/// Allocate a fresh fetch scope for a document (top-level page or iframe).
pub(crate) fn new_scope() -> u64 {
    let scope = NEXT_SCOPE.fetch_add(1, Ordering::Relaxed);
    LAST_SCOPE.set(scope);
    scope
}

/// Peek at the fetch scope created by the most recent `run_zipp_js` on this thread
/// without consuming it (used by `build_document` / `mount_iframe` before
/// `JsEventBridge::new` takes ownership of the scope).
pub(crate) fn peek_last_scope() -> u64 {
    LAST_SCOPE.get()
}

/// Take the fetch scope created by the most recent `run_zipp_js` on this thread.
pub(crate) fn take_last_scope() -> u64 {
    LAST_SCOPE.replace(0)
}

/// Return the currently active scope on this thread (0 if none).
pub(crate) fn current_scope() -> u64 {
    CURRENT_SCOPE.get()
}

/// Whether async `fetch()` can be queued on this thread right now.
pub(crate) fn is_enabled() -> bool {
    ALLOWED.get() && CURRENT_SCOPE.get() != 0
}

/// RAII guard binding `CURRENT_SCOPE` for the duration of a JS execution slice.
pub(crate) struct ScopeGuard {
    prev: u64,
}

impl ScopeGuard {
    pub(crate) fn enter(scope: u64) -> Self {
        let prev = CURRENT_SCOPE.replace(scope);
        Self { prev }
    }
}

impl Drop for ScopeGuard {
    fn drop(&mut self) {
        CURRENT_SCOPE.set(self.prev);
    }
}

/// Enqueue an HTTP fetch on the background pool for the current thread's scope.
/// Returns `Some(req_id)` if queued, or `None` if async fetch is not active.
pub(crate) fn enqueue<F>(run: F) -> Option<u64>
where
    F: FnOnce() -> Result<String, String> + Send + 'static,
{
    let scope = CURRENT_SCOPE.get();
    if !ALLOWED.get() || scope == 0 {
        return None;
    }
    let req_id = NEXT_REQ_ID.fetch_add(1, Ordering::Relaxed);
    {
        let mut state = RESULTS.lock().unwrap_or_else(|e| e.into_inner());
        *state.pending.entry(scope).or_insert(0) += 1;
    }
    let (lock, cvar) = &*POOL;
    {
        let mut q = lock.lock().unwrap_or_else(|e| e.into_inner());
        q.jobs.push_back(Job {
            scope,
            req_id,
            run: Box::new(run),
        });
    }
    cvar.notify_one();
    Some(req_id)
}

/// Drain all completed fetch results for `scope`.
pub(crate) fn take_completed(scope: u64) -> Vec<FetchOutcome> {
    if scope == 0 {
        return Vec::new();
    }
    let mut state = RESULTS.lock().unwrap_or_else(|e| e.into_inner());
    state.completed.remove(&scope).unwrap_or_default()
}

/// Whether `scope` still has in-flight background fetches.
pub(crate) fn has_pending(scope: u64) -> bool {
    if scope == 0 {
        return false;
    }
    let state = RESULTS.lock().unwrap_or_else(|e| e.into_inner());
    state.pending.get(&scope).copied().unwrap_or(0) > 0
}

/// Discard queued/pending state for a document that is being torn down.
pub(crate) fn drop_scope(scope: u64) {
    if scope == 0 {
        return;
    }
    {
        let mut state = RESULTS.lock().unwrap_or_else(|e| e.into_inner());
        state.pending.remove(&scope);
        state.completed.remove(&scope);
    }
    let (lock, _) = &*POOL;
    let mut q = lock.lock().unwrap_or_else(|e| e.into_inner());
    q.jobs.retain(|j| j.scope != scope);
}
