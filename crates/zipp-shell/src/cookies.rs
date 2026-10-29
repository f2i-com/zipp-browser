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
        *self.dirty.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.changed.notify_one();
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
    /// attributes. A script cannot set an HttpOnly cookie.
    pub fn set_document_cookie(&self, url: &Url, value: &str) {
        let Ok(cookie) = RawCookie::parse(value.to_string()) else {
            return;
        };
        if cookie.http_only().unwrap_or(false) {
            return;
        }
        if self.lock().insert_raw(&cookie, url).is_ok() {
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
            if std::fs::write(&tmp, &out).is_ok() {
                let _ = std::fs::rename(&tmp, path);
            }
        }
    }
}

/// Save the jar before the browser exits: changes from the last half second
/// are otherwise lost with the process.
pub fn flush() {
    if let Some(jar) = JAR.get() {
        jar.flush();
    }
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
