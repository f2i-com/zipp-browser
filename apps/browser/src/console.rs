//! The page's console: what its scripts logged and what went wrong while it
//! ran, kept for the console panel (F12, or the toolbar's button).
//!
//! Only the window's page is recorded -- the thread that runs it, and its
//! workers -- not the pages prerendered in the background. Repeats of the
//! newest line collapse into a count, and the oldest go past a thousand.
//!
//! The panel is part of the browser's chrome, in the band above the page
//! beside the toolbar (`toolbar_markup`), so a re-render of the page leaves
//! it alone; the event bridge opens and closes it and refreshes its lines
//! (`JsEventBridge::refresh_console`).

use std::cell::Cell;
use std::collections::VecDeque;
use std::fmt::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Level {
    Log,
    Warn,
    Error,
    /// Code typed into the console, and what it gave back.
    Input,
    Result,
}

impl Level {
    /// The level of a page `console.*` call, by the name the shim gives it.
    pub(crate) fn from_name(name: &str) -> Level {
        match name {
            "error" => Level::Error,
            "warn" | "warning" => Level::Warn,
            _ => Level::Log,
        }
    }
}

struct Entry {
    level: Level,
    text: String,
    repeats: u32,
}

struct Log {
    entries: VecDeque<Entry>,
    errors: usize,
    warnings: usize,
    /// Bumped by every change, so the panel knows when it is stale.
    generation: u64,
}

const LIMIT: usize = 1000;
/// The height of the open panel, in CSS pixels.
pub(crate) const PANEL_HEIGHT: u32 = 260;

static LOG: Mutex<Log> = Mutex::new(Log { entries: VecDeque::new(), errors: 0, warnings: 0, generation: 0 });
static OPEN: AtomicBool = AtomicBool::new(false);
/// Code typed into the console, waiting for the page's thread to run it.
static PENDING: Mutex<Vec<String>> = Mutex::new(Vec::new());

thread_local! {
    static RECORDS: Cell<bool> = const { Cell::new(false) };
}

/// Record what this thread's page logs: the window's thread, and its
/// workers'.
pub(crate) fn record_this_thread() {
    RECORDS.set(true);
}

pub(crate) fn push(level: Level, text: impl Into<String>) {
    if !RECORDS.get() {
        return;
    }
    let text = text.into();
    let mut log = LOG.lock().unwrap();
    match level {
        Level::Error => log.errors += 1,
        Level::Warn => log.warnings += 1,
        _ => {}
    }
    log.generation += 1;
    if let Some(last) = log.entries.back_mut()
        && last.level == level
        && last.text == text
    {
        last.repeats += 1;
        return;
    }
    if log.entries.len() == LIMIT {
        log.entries.pop_front();
    }
    log.entries.push_back(Entry { level, text, repeats: 1 });
}

/// A new page: an empty console.
pub(crate) fn clear() {
    let mut log = LOG.lock().unwrap();
    log.entries.clear();
    log.errors = 0;
    log.warnings = 0;
    log.generation += 1;
}

pub(crate) fn generation() -> u64 {
    LOG.lock().unwrap().generation
}

pub(crate) fn is_open() -> bool {
    OPEN.load(Ordering::Relaxed)
}

/// Open or close the panel; whether it is now open.
pub(crate) fn toggle() -> bool {
    !OPEN.fetch_xor(true, Ordering::Relaxed)
}

/// Code to run in the page, from the console's input line.
pub(crate) fn submit(code: String) {
    if !code.trim().is_empty() {
        PENDING.lock().unwrap().push(code);
    }
}

pub(crate) fn take_submitted() -> Vec<String> {
    std::mem::take(&mut *PENDING.lock().unwrap())
}

pub(crate) fn has_submitted() -> bool {
    !PENDING.lock().unwrap().is_empty()
}

fn escape(text: &str, out: &mut String) {
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

/// The badge on the toolbar's console button: the error count, if any.
pub(crate) fn badge_text() -> String {
    match LOG.lock().unwrap().errors {
        0 => String::new(),
        n if n > 99 => "99+".into(),
        n => n.to_string(),
    }
}

/// The panel's header counts.
pub(crate) fn counts_text() -> String {
    let log = LOG.lock().unwrap();
    let plural = |n: usize, one: &str| if n == 1 { format!("1 {one}") } else { format!("{n} {one}s") };
    format!("{} · {}", plural(log.errors, "error"), plural(log.warnings, "warning"))
}

/// The panel's lines, newest first, as HTML.
pub(crate) fn lines_html() -> String {
    let log = LOG.lock().unwrap();
    if log.entries.is_empty() {
        return r#"<div style="padding:6px 10px;color:#868e96;font-family:Arial,sans-serif">Nothing logged yet.</div>"#.into();
    }
    let mut out = String::with_capacity(log.entries.len() * 80);
    for entry in log.entries.iter().rev() {
        let (color, background, mark) = match entry.level {
            Level::Error => ("#c92a2a", "#fff5f5", "✖"),
            Level::Warn => ("#8a6100", "#fff9db", "⚠"),
            Level::Input => ("#1c7ed6", "#ffffff", "›"),
            Level::Result => ("#495057", "#ffffff", "‹"),
            Level::Log => ("#212529", "#ffffff", ""),
        };
        let _ = write!(
            out,
            r#"<div style="display:flex;gap:6px;padding:2px 10px;border-bottom:1px solid #f1f3f5;color:{color};background:{background};white-space:pre-wrap;word-break:break-word"><span style="width:12px;flex-shrink:0">{mark}</span><span style="flex:1">"#
        );
        escape(&entry.text, &mut out);
        out.push_str("</span>");
        if entry.repeats > 1 {
            let _ = write!(
                out,
                r#"<span style="flex-shrink:0;padding:0 6px;border-radius:8px;background:#e9ecef;color:#495057">{}</span>"#,
                entry.repeats
            );
        }
        out.push_str("</div>");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_collapses_and_escapes() {
        record_this_thread();
        clear();
        push(Level::Log, "a <b>");
        push(Level::Error, "boom");
        push(Level::Error, "boom");
        let html = lines_html();
        assert!(html.contains("a &lt;b&gt;"), "{html}");
        assert_eq!(html.matches("boom").count(), 1, "repeats collapse: {html}");
        assert!(html.find("boom").unwrap() < html.find("a &lt;b&gt;").unwrap(), "newest first");
        assert_eq!(badge_text(), "2");
        assert_eq!(counts_text(), "2 errors · 0 warnings");
    }
}
