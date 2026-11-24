//! The browser's cookies: one jar for everything it fetches -- the pages, what
//! they load, what their scripts `fetch` -- and for `document.cookie`, kept in
//! the profile so that a sign-in outlives the browser.
//!
//! Before, the page fetches, the subresource fetches and `document.cookie` each
//! kept their own, in memory: a cookie a script set never reached the server, a
//! session cookie the server set was not sent with the page's own requests for
//! its images and styles, and nothing survived a restart.

use std::io::BufReader;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use cookie_store::{CookieStore, RawCookie};
use reqwest::header::HeaderValue;
use url::Url;

/// A cookie jar, saved to the profile when it changes.
pub struct Jar {
    store: Mutex<CookieStore>,
    path: Option<PathBuf>,
    /// Changed since it was last saved; the saver waits on `changed`.
    dirty: Mutex<bool>,
    changed: Condvar,
    /// Held while the file is written, so that `flush` waits for a save
    /// already under way rather than letting the process end in the middle.
    saving: Mutex<()>,
    /// Monotonic counter incremented on every cookie mutation, so page caches
    /// can detect login/logout state transitions without inspecting cookie values.
    generation: AtomicU64,
}

static JAR: OnceLock<Arc<Jar>> = OnceLock::new();

/// The browser's jar, read from the profile on first use.
pub fn jar() -> Arc<Jar> {
    JAR.get_or_init(|| {
        let path = crate::profile::dir().map(|dir| dir.join("cookies.json"));
        let store = path
            .as_ref()
            .and_then(|p| std::fs::File::open(p).ok())
            .and_then(|f| cookie_store::serde::json::load(BufReader::new(f)).ok())
            .unwrap_or_default();
        let jar = Arc::new(Jar {
            store: Mutex::new(store),
            path,
            dirty: Mutex::new(false),
            changed: Condvar::new(),
            saving: Mutex::new(()),
            generation: AtomicU64::new(0),
        });
        // Saved off the thread that set the cookie, half a second after a
        // change: a response sets several at once and a page load many, and
        // each would otherwise write the file.
        if jar.path.is_some() {
            let saver = jar.clone();
            let _ = std::thread::Builder::new().name("zipp-cookies".into()).spawn(move || loop {
                {
                    let mut dirty = saver.dirty.lock().unwrap_or_else(|e| e.into_inner());
                    while !*dirty {
                        dirty = saver.changed.wait(dirty).unwrap_or_else(|e| e.into_inner());
                    }
                }
                std::thread::sleep(Duration::from_millis(500));
                saver.flush();
            });
        }
        jar
    })
    .clone()
}

impl Jar {
    fn lock(&self) -> MutexGuard<'_, CookieStore> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn mark_changed(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        *self.dirty.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.changed.notify_one();
    }

    /// Monotonic counter incremented on every cookie insert/delete/clear.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    /// Number of unexpired cookies currently held in the jar.
    pub fn count(&self) -> usize {
        self.lock().iter_unexpired().count()
    }

    /// Remove every cookie from memory and delete the persisted `cookies.json`.
    pub fn clear_all(&self) -> usize {
        let removed = {
            let mut store = self.lock();
            let count = store.iter_any().count();
            store.clear();
            count
        };
        self.generation.fetch_add(1, Ordering::Relaxed);
        let _saving = self.saving.lock().unwrap_or_else(|e| e.into_inner());
        *self.dirty.lock().unwrap_or_else(|e| e.into_inner()) = false;
        if let Some(path) = &self.path {
            let _ = std::fs::remove_file(path);
        }
        removed
    }

    /// Save now if anything changed since the last save.
    fn flush(&self) {
        let _saving = self.saving.lock().unwrap_or_else(|e| e.into_inner());
        if std::mem::take(&mut *self.dirty.lock().unwrap_or_else(|e| e.into_inner())) {
            self.save();
        }
    }

    /// What `document.cookie` reads on a page at `url`: `name=value; ...`,
    /// leaving out the HttpOnly ones, which are the server's.
    pub fn document_cookie(&self, url: &Url) -> String {
        let store = self.lock();
        store
            .matches(url)
            .iter()
            .filter(|c| !c.http_only().unwrap_or(false))
            .map(|c| format!("{}={}", c.name(), c.value()))
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// `document.cookie = value` on a page at `url`: one cookie and its
    /// attributes. A script cannot set or overwrite an HttpOnly cookie.
    pub fn set_document_cookie(&self, url: &Url, value: &str) {
        let Ok(cookie) = RawCookie::parse(value.to_string()) else {
            return;
        };
        if cookie.http_only().unwrap_or(false) {
            return;
        }
        let mut store = self.lock();
        // RFC 6265 §5.3: non-HTTP APIs must not overwrite an existing HttpOnly cookie.
        if store
            .matches(url)
            .iter()
            .any(|c| c.name() == cookie.name() && c.http_only().unwrap_or(false))
        {
            return;
        }
        if store.insert_raw(&cookie, url).is_ok() {
            drop(store);
            self.mark_changed();
        }
    }

    /// Write the cookies that outlive a session to the profile; a session
    /// cookie ends with the browser, as it should.
    fn save(&self) {
        let Some(path) = &self.path else {
            return;
        };
        let mut out = Vec::new();
        if cookie_store::serde::json::save(&self.lock(), &mut out).is_ok() {
            // Whole or not at all: a write cut short would lose every cookie.
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, &out).is_ok() && std::fs::rename(&tmp, path).is_err() {
                let _ = std::fs::remove_file(path);
                let _ = std::fs::rename(&tmp, path);
            }
        }
    }
}

/// Current cookie jar generation counter (0 if jar not yet initialized).
pub fn generation() -> u64 {
    JAR.get().map_or(0, |j| j.generation())
}

/// Remove every cookie in the browser's jar and delete `cookies.json` on disk.
pub fn clear_all() -> usize {
    if let Some(jar) = JAR.get() {
        return jar.clear_all();
    }
    if let Some(path) = crate::profile::dir().map(|d| d.join("cookies.json")) {
        let _ = std::fs::remove_file(path);
    }
    0
}

/// Save the jar before the browser exits: changes from the last half second
/// are otherwise lost with the process.
pub fn flush() {
    if let Some(jar) = JAR.get() {
        jar.flush();
    }
}

/// Number of cookies currently stored in the active cookie jar.
pub fn count() -> usize {
    jar().count()
}

impl reqwest::cookie::CookieStore for Jar {
    fn set_cookies(&self, headers: &mut dyn Iterator<Item = &HeaderValue>, url: &Url) {
        let mut store = self.lock();
        let mut changed = false;
        for header in headers {
            if let Some(cookie) = header.to_str().ok().and_then(|s| RawCookie::parse(s.to_string()).ok()) {
                changed |= store.insert_raw(&cookie, url).is_ok();
            }
        }
        if changed {
            drop(store);
            self.mark_changed();
        }
    }

    fn cookies(&self, url: &Url) -> Option<HeaderValue> {
        let store = self.lock();
        let header = store
            .get_request_values(url)
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ");
        if header.is_empty() { None } else { HeaderValue::from_str(&header).ok() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::cookie::CookieStore as _;

    fn test_jar(path: Option<PathBuf>) -> Jar {
        let store = path
            .as_ref()
            .and_then(|p| std::fs::File::open(p).ok())
            .and_then(|f| cookie_store::serde::json::load(BufReader::new(f)).ok())
            .unwrap_or_default();
        Jar {
            store: Mutex::new(store),
            path,
            dirty: Mutex::new(false),
            changed: Condvar::new(),
            saving: Mutex::new(()),
            generation: AtomicU64::new(0),
        }
    }

    #[test]
    fn session_and_persistent_cookies_and_httponly_protection() {
        let path = std::env::temp_dir().join(format!("zipp-cookies-test-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let url = Url::parse("http://127.0.0.1:8080/app/dashboard").unwrap();

        let jar = test_jar(Some(path.clone()));
        let h1 = HeaderValue::from_static("sid=secret123; Path=/; HttpOnly");
        let h2 = HeaderValue::from_static("theme=dark; Path=/; Max-Age=3600");
        jar.set_cookies(&mut [&h1, &h2].into_iter(), &url);

        // Request Cookie header includes both HttpOnly session and persistent cookies.
        let req_hdr = jar.cookies(&url).unwrap();
        let req_str = req_hdr.to_str().unwrap();
        assert!(req_str.contains("sid=secret123"), "{req_str}");
        assert!(req_str.contains("theme=dark"), "{req_str}");

        // document.cookie hides HttpOnly cookie, exposes non-HttpOnly cookie.
        let doc_str = jar.document_cookie(&url);
        assert!(!doc_str.contains("sid="), "{doc_str}");
        assert!(doc_str.contains("theme=dark"), "{doc_str}");

        // Script cannot overwrite existing HttpOnly cookie.
        jar.set_document_cookie(&url, "sid=hijacked; Path=/");
        assert!(jar.cookies(&url).unwrap().to_str().unwrap().contains("sid=secret123"));

        // Script can set its own cookie and delete it via Max-Age=0.
        jar.set_document_cookie(&url, "prefs=compact; Path=/");
        assert!(jar.document_cookie(&url).contains("prefs=compact"));
        assert!(jar.cookies(&url).unwrap().to_str().unwrap().contains("prefs=compact"));
        jar.set_document_cookie(&url, "prefs=; Path=/; Max-Age=0");
        assert!(!jar.document_cookie(&url).contains("prefs="));

        // Saving persists Max-Age cookies, drops session cookies.
        jar.flush();
        let jar2 = test_jar(Some(path.clone()));
        let persisted = jar2.cookies(&url).unwrap();
        let persisted_str = persisted.to_str().unwrap();
        assert!(persisted_str.contains("theme=dark"), "{persisted_str}");
        assert!(!persisted_str.contains("sid=secret123"), "session cookie must not persist: {persisted_str}");

        // clear_all removes everything from memory and disk.
        assert!(jar2.clear_all() >= 1);
        assert!(jar2.cookies(&url).is_none());
        assert!(!path.exists());
    }
}
