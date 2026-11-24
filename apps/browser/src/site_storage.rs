//! What sites keep in the browser between visits -- their IndexedDB databases
//! and `localStorage` -- a folder per origin in the profile:
//! `<profile>/storage/<origin>/<kind>/<name>.json`.
//!
//! A page's script reads and writes through host calls (`site_*` in main.rs).
//! Which origin's folder a call reaches is decided by the browser, from the
//! document the script belongs to; the page names only the kind and the item.
//!
//! Writes go to a saver thread, which writes each file whole -- to a
//! temporary file, then renamed over the old one -- and turns a burst of
//! writes to one file into one write. A read sees a write still waiting, and
//! `flush` finishes them all before the browser exits.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

/// What a site may keep, each kind in its own folder (`session` stays in memory
/// for the lifetime of this browser process, per origin).
const KINDS: &[&str] = &["idb", "local", "session"];

static SESSION_STORE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
/// In-memory-only store for `localStorage` and `IndexedDB` when running in Incognito mode.
static EPHEMERAL_STORE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

fn session_store() -> &'static Mutex<HashMap<String, String>> {
    SESSION_STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn ephemeral_store() -> &'static Mutex<HashMap<String, String>> {
    EPHEMERAL_STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn ephemeral_key(doc: &url::Url, kind: &str, name: &str) -> Result<String, String> {
    if !KINDS.contains(&kind) {
        return Err(format!("unknown storage kind {kind:?}"));
    }
    let origin = origin_key(doc).ok_or("SecurityError: this document cannot keep storage")?;
    Ok(format!("{origin}::{kind}::{name}"))
}

fn origin_key(doc: &url::Url) -> Option<String> {
    let name = match doc.scheme() {
        scheme @ ("http" | "https") => {
            let host: String = doc
                .host_str()?
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' })
                .collect();
            format!("{scheme}_{host}_{}", doc.port_or_known_default()?)
        }
        "file" => crate::app_storage_name().unwrap_or_else(|| "file".to_string()),
        _ => return None,
    };
    Some(name)
}

struct Saver {
    /// Writes not yet on disk: a file's new content, or `None` to delete it.
    pending: Mutex<HashMap<PathBuf, Option<String>>>,
    changed: Condvar,
    /// Held while files are written and while one is read, so that a read
    /// never falls between a write leaving `pending` and reaching the disk.
    disk: Mutex<()>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn saver() -> &'static Saver {
    static SAVER: OnceLock<Saver> = OnceLock::new();
    SAVER.get_or_init(|| {
        let _ = std::thread::Builder::new().name("zipp-site-storage".into()).spawn(|| loop {
            let s = saver();
            {
                let mut pending = lock(&s.pending);
                while pending.is_empty() {
                    pending = s.changed.wait(pending).unwrap_or_else(|e| e.into_inner());
                }
            }
            // A transaction's writes, and a loop of `setItem`s, arrive
            // together: let them gather.
            std::thread::sleep(Duration::from_millis(300));
            flush();
        });
        Saver { pending: Mutex::new(HashMap::new()), changed: Condvar::new(), disk: Mutex::new(()) }
    })
}

/// The folder an origin's storage lives in: `https_example.com_443`, or
/// `file` for pages opened from disk (which share one, as `localStorage`
/// does in Chrome). Other documents -- `data:`, `about:` -- keep nothing.
fn origin_dir(doc: &url::Url) -> Option<PathBuf> {
    Some(zipp_shell::profile::dir()?.join("storage").join(origin_key(doc)?))
}

/// The file holding `name`: its UTF-8 bytes in hex, so that any name a page
/// gives is a safe file name and can be listed back. A name too long for
/// that is cut and given a hash, and then is not listed.
fn file_for(doc: &url::Url, kind: &str, name: &str) -> Result<PathBuf, String> {
    if !KINDS.contains(&kind) {
        return Err(format!("unknown storage kind {kind:?}"));
    }
    let dir = origin_dir(doc).ok_or("SecurityError: this document cannot keep storage")?;
    let hex: String = name.bytes().map(|b| format!("{b:02x}")).collect();
    let stem = if hex.len() <= 160 {
        format!("n{hex}")
    } else {
        // FNV-1a: stable across builds, unlike the standard hasher.
        let hash = name.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
        format!("h{}-{hash:016x}", &hex[..120])
    };
    Ok(dir.join(kind).join(format!("{stem}.json")))
}

/// `name`'s content, `None` if the site keeps nothing under it.
pub fn read(doc: &url::Url, kind: &str, name: &str) -> Result<Option<String>, String> {
    if kind == "session" {
        let key = format!("{}::{name}", origin_key(doc).ok_or("SecurityError: this document cannot keep storage")?);
        return Ok(lock(session_store()).get(&key).cloned());
    }
    if zipp_shell::profile::is_incognito() {
        let key = ephemeral_key(doc, kind, name)?;
        return Ok(lock(ephemeral_store()).get(&key).cloned());
    }
    let path = file_for(doc, kind, name)?;
    let s = saver();
    let _disk = lock(&s.disk);
    if let Some(waiting) = lock(&s.pending).get(&path) {
        return Ok(waiting.clone());
    }
    Ok(std::fs::read_to_string(&path).ok())
}

/// Keep `data` under `name`, or with `None` remove it.
pub fn write(doc: &url::Url, kind: &str, name: &str, data: Option<String>) -> Result<(), String> {
    if kind == "session" {
        let key = format!("{}::{name}", origin_key(doc).ok_or("SecurityError: this document cannot keep storage")?);
        let mut store = lock(session_store());
        match data {
            Some(v) => { store.insert(key, v); }
            None => { store.remove(&key); }
        }
        return Ok(());
    }
    if zipp_shell::profile::is_incognito() {
        let key = ephemeral_key(doc, kind, name)?;
        let mut store = lock(ephemeral_store());
        match data {
            Some(v) => { store.insert(key, v); }
            None => { store.remove(&key); }
        }
        return Ok(());
    }
    let path = file_for(doc, kind, name)?;
    let s = saver();
    lock(&s.pending).insert(path, data);
    s.changed.notify_one();
    Ok(())
}

/// The names the site keeps something under, as a JSON array.
pub fn names(doc: &url::Url, kind: &str) -> Result<String, String> {
    if kind == "session" {
        let prefix = format!("{}::", origin_key(doc).ok_or("SecurityError: this document cannot keep storage")?);
        let names: Vec<String> = lock(session_store())
            .keys()
            .filter_map(|k| k.strip_prefix(&prefix).map(str::to_string))
            .collect();
        return Ok(serde_json::to_string(&names).unwrap_or_else(|_| "[]".into()));
    }
    if zipp_shell::profile::is_incognito() {
        if !KINDS.contains(&kind) {
            return Err(format!("unknown storage kind {kind:?}"));
        }
        let origin = origin_key(doc).ok_or("SecurityError: this document cannot keep storage")?;
        let prefix = format!("{origin}::{kind}::");
        let names: Vec<String> = lock(ephemeral_store())
            .keys()
            .filter_map(|k| k.strip_prefix(&prefix).map(str::to_string))
            .collect();
        return Ok(serde_json::to_string(&names).unwrap_or_else(|_| "[]".into()));
    }
    let dir = file_for(doc, kind, "")?.parent().map(PathBuf::from).unwrap_or_default();
    let s = saver();
    let _disk = lock(&s.disk);
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .collect();
    for (path, data) in lock(&s.pending).iter() {
        if path.parent() == Some(dir.as_path()) {
            paths.retain(|p| p != path);
            if data.is_some() {
                paths.push(path.clone());
            }
        }
    }
    let names: Vec<String> = paths
        .iter()
        .filter_map(|p| {
            let hex = p.file_name()?.to_str()?.strip_prefix('n')?.strip_suffix(".json")?;
            let bytes: Option<Vec<u8>> =
                (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok()).collect();
            String::from_utf8(bytes?).ok()
        })
        .collect();
    Ok(serde_json::to_string(&names).unwrap_or_else(|_| "[]".into()))
}

/// Clear every origin's `localStorage`, IndexedDB, and in-memory `sessionStorage`.
pub fn clear_all() {
    lock(session_store()).clear();
    lock(ephemeral_store()).clear();
    if zipp_shell::profile::is_incognito() {
        return;
    }
    let s = saver();
    let _disk = lock(&s.disk);
    lock(&s.pending).clear();
    if let Some(dir) = zipp_shell::profile::dir().map(|d| d.join("storage")) {
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Clear in-memory `sessionStorage` only (e.g., for testing or starting a clean session).
#[cfg(test)]
pub fn clear_sessions() {
    lock(session_store()).clear();
}

/// Write everything waiting, before the browser exits.
pub fn flush() {
    if zipp_shell::profile::is_incognito() {
        return;
    }
    let s = saver();
    let _disk = lock(&s.disk);
    let batch = std::mem::take(&mut *lock(&s.pending));
    for (path, data) in batch {
        match data {
            Some(data) => {
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                // Whole or not at all: a write cut short would lose the lot.
                let tmp = path.with_extension("json.tmp");
                if std::fs::write(&tmp, data).is_ok() {
                    let _ = std::fs::rename(&tmp, &path);
                }
            }
            None => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}
