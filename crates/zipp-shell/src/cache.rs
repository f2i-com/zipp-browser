//! Persistent HTTP & asset cache for Zipp Browser (`<profile>/cache/`).
//!
//! Combines a bounded in-memory LRU cache (fast path for hot subresources like
//! ES modules, stylesheets, images, fonts, and 3D `.glb`/`.hdr` assets) with a
//! persistent disk store honoring RFC 9111 HTTP caching semantics:
//!
//! - `Cache-Control: no-store` -> never stored
//! - `Cache-Control: no-cache` / `max-age=0` -> stored for conditional
//!   revalidation (`If-None-Match` / `If-Modified-Since` -> `304 Not Modified`)
//! - `Cache-Control: max-age=<N>` / `immutable` -> served fresh with zero
//!   network round-trips until expiry
//! - Responses carrying `Set-Cookie` or `Vary: *` / `Vary: Cookie` /
//!   `Vary: Authorization` are never stored, keeping auth/session flows safe
//! - Bucketed by request kind (`"doc"` vs `"sub"`) so navigation HTML and
//!   script/API fetches for the same URL never collide
//! - Bounded disk (128 MiB) and memory (32 MiB) budgets with LRU eviction,
//!   plus [`clear`] for instant full cache purges.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use reqwest::header::HeaderMap;

/// Maximum in-memory cache bytes (32 MiB).
const MAX_MEM_BYTES: usize = 32 * 1024 * 1024;
/// Maximum in-memory entry count.
const MAX_MEM_ENTRIES: usize = 512;
/// Maximum size of a single cached response (32 MiB — covers large SoftN game bundles).
const MAX_ENTRY_BYTES: usize = 32 * 1024 * 1024;
/// Default maximum persistent disk cache bytes (128 MiB).
const DEFAULT_MAX_DISK_BYTES: u64 = 128 * 1024 * 1024;
/// Heuristic freshness for static assets when server omits `Cache-Control` (5 min).
const STATIC_HEURISTIC_SECS: u64 = 300;
/// Default freshness when `Cache-Control: immutable` is sent without `max-age` (7 days).
const IMMUTABLE_DEFAULT_SECS: u64 = 7 * 24 * 3600;
/// Run a disk size trim at most every N writes to avoid scanning the directory on every asset.
const TRIM_EVERY_WRITES: u64 = 32;

/// One cached HTTP response.
#[derive(Clone, Debug)]
pub struct CachedEntry {
    pub url: String,
    pub bucket: String,
    pub status: u16,
    pub content_type: String,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub stored_at: u64,
    pub expires_at: u64,
    pub must_revalidate: bool,
    pub body: Arc<Vec<u8>>,
}

/// Result of looking up a URL in the cache.
#[derive(Debug)]
pub enum Lookup {
    /// Entry is still fresh; serve directly without contacting the origin.
    Fresh(CachedEntry),
    /// Entry is stale (or `no-cache`) and has validators (`ETag` / `Last-Modified`);
    /// send a conditional request and call [`HttpCache::revalidate_304`] on 304.
    Revalidate(CachedEntry),
    /// Not cached or expired without validators.
    Miss,
}

/// Snapshot of cache occupancy and hit/miss counters.
#[derive(Clone, Copy, Debug, Default)]
pub struct CacheStats {
    pub mem_entries: usize,
    pub mem_bytes: usize,
    pub disk_entries: usize,
    pub disk_bytes: u64,
    pub hits: u64,
    pub revalidated: u64,
    pub misses: u64,
}

struct MemState {
    entries: HashMap<String, (u64, CachedEntry)>,
    total_bytes: usize,
    tick: u64,
}

impl MemState {
    fn new() -> Self {
        Self { entries: HashMap::new(), total_bytes: 0, tick: 0 }
    }

    fn get(&mut self, key: &str) -> Option<CachedEntry> {
        self.tick = self.tick.wrapping_add(1);
        let tick = self.tick;
        let slot = self.entries.get_mut(key)?;
        slot.0 = tick;
        Some(slot.1.clone())
    }

    fn insert(&mut self, key: String, entry: CachedEntry) {
        let size = entry.body.len();
        if size > MAX_MEM_BYTES / 2 {
            return;
        }
        if let Some((_, old)) = self.entries.remove(&key) {
            self.total_bytes = self.total_bytes.saturating_sub(old.body.len());
        }
        while (self.total_bytes + size > MAX_MEM_BYTES || self.entries.len() >= MAX_MEM_ENTRIES)
            && !self.entries.is_empty()
        {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, (t, _))| *t)
                .map(|(k, _)| k.clone());
            if let Some(k) = oldest
                && let Some((_, old)) = self.entries.remove(&k)
            {
                self.total_bytes = self.total_bytes.saturating_sub(old.body.len());
            } else {
                break;
            }
        }
        self.tick = self.tick.wrapping_add(1);
        self.total_bytes += size;
        self.entries.insert(key, (self.tick, entry));
    }

    fn remove(&mut self, key: &str) {
        if let Some((_, old)) = self.entries.remove(key) {
            self.total_bytes = self.total_bytes.saturating_sub(old.body.len());
        }
    }

    fn clear(&mut self) -> (usize, u64) {
        let count = self.entries.len();
        let bytes = self.total_bytes as u64;
        self.entries.clear();
        self.total_bytes = 0;
        (count, bytes)
    }
}

/// Persistent HTTP disk + memory cache.
pub struct HttpCache {
    dir: Option<PathBuf>,
    mem: Mutex<MemState>,
    disabled: AtomicBool,
    max_disk_bytes: AtomicU64,
    writes_since_trim: AtomicU64,
    hits: AtomicU64,
    revalidated: AtomicU64,
    misses: AtomicU64,
}

static GLOBAL_CACHE: OnceLock<HttpCache> = OnceLock::new();

/// The browser's shared cache rooted at `<profile>/cache`.
pub fn global() -> &'static HttpCache {
    GLOBAL_CACHE.get_or_init(|| {
        let dir = crate::profile::dir().map(|d| d.join("cache"));
        HttpCache::with_dir(dir)
    })
}

/// Clear the browser's shared cache (memory + disk).
pub fn clear() -> (usize, u64) {
    global().clear()
}

/// Statistics for the browser's shared cache.
pub fn stats() -> CacheStats {
    global().stats()
}

/// Invalidate cached entries for `url` (both document and subresource buckets).
pub fn invalidate(url: &str) {
    global().invalidate_url(url);
}

/// Enable or disable caching globally (e.g. for `--no-cache` or settings).
pub fn set_disabled(disabled: bool) {
    global().set_disabled(disabled);
}

/// Whether caching is currently disabled.
pub fn is_disabled() -> bool {
    global().is_disabled()
}

/// Configure the disk cache size ceiling in bytes (minimum 8 MiB) and trim if needed.
pub fn set_max_disk_bytes(bytes: u64) {
    global().set_max_disk_bytes(bytes);
}

/// Current disk cache size ceiling in bytes.
pub fn max_disk_bytes() -> u64 {
    global().max_disk_bytes()
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Two independent 64-bit FNV-1a hashes combined into a 32-hex-char filename key.
/// Full URL & bucket are verified inside the file header to eliminate collisions.
fn cache_key(bucket: &str, url: &str) -> String {
    let mut h1: u64 = 0xcbf29ce484222325;
    let mut h2: u64 = 0x84222325cbf29ce4;
    for b in bucket.bytes().chain([0xff]).chain(url.bytes()) {
        h1 = (h1 ^ b as u64).wrapping_mul(0x100000001b3);
        h2 = h2.rotate_left(7) ^ (b as u64).wrapping_mul(0x9e3779b97f4a7c15);
    }
    format!("{h1:016x}{h2:016x}")
}

fn sanitize_header(s: &str) -> String {
    s.chars().filter(|&c| c != '\r' && c != '\n').collect()
}

fn looks_like_static_asset(url: &str, content_type: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url).to_ascii_lowercase();
    let static_ext = [
        ".js", ".mjs", ".css", ".wasm", ".softn", ".png", ".jpg", ".jpeg", ".webp", ".gif", ".svg",
        ".ico", ".woff", ".woff2", ".ttf", ".otf", ".glb", ".gltf", ".hdr", ".mp3", ".m4a", ".mp4",
        ".ogg", ".wav", ".avif", ".bmp",
    ]
    .iter()
    .any(|ext| path.ends_with(ext));
    if static_ext {
        return true;
    }
    let ct = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    matches!(
        ct.as_str(),
        "text/css"
            | "text/javascript"
            | "application/javascript"
            | "application/x-javascript"
            | "application/wasm"
            | "font/woff"
            | "font/woff2"
            | "font/ttf"
            | "font/otf"
            | "application/font-woff"
            | "application/font-woff2"
            | "model/gltf-binary"
            | "model/gltf+json"
            | "image/vnd.radiance"
    ) || ct.starts_with("image/")
}

struct CacheDirectives {
    no_store: bool,
    no_cache: bool,
    must_revalidate: bool,
    immutable: bool,
    max_age: Option<u64>,
}

fn parse_cache_control(headers: &HeaderMap) -> CacheDirectives {
    let mut d = CacheDirectives {
        no_store: false,
        no_cache: false,
        must_revalidate: false,
        immutable: false,
        max_age: None,
    };
    for val in headers.get_all(reqwest::header::CACHE_CONTROL) {
        let Ok(raw) = val.to_str() else { continue };
        for token in raw.split(',') {
            let t = token.trim();
            if t.eq_ignore_ascii_case("no-store") {
                d.no_store = true;
            } else if t.eq_ignore_ascii_case("no-cache") {
                d.no_cache = true;
            } else if t.eq_ignore_ascii_case("must-revalidate") {
                d.must_revalidate = true;
            } else if t.eq_ignore_ascii_case("immutable") {
                d.immutable = true;
            } else if let Some((k, v)) = t.split_once('=')
                && k.trim().eq_ignore_ascii_case("max-age")
                && let Ok(secs) = v.trim().trim_matches('"').parse::<u64>()
            {
                d.max_age = Some(secs);
            }
        }
    }
    if let Some(pragma) = headers.get(reqwest::header::PRAGMA).and_then(|v| v.to_str().ok())
        && pragma.to_ascii_lowercase().contains("no-cache")
        && d.max_age.is_none()
    {
        d.no_cache = true;
    }
    d
}

fn header_str(headers: &HeaderMap, name: reqwest::header::HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(sanitize_header)
}

fn vary_forbids_cache(headers: &HeaderMap) -> bool {
    for val in headers.get_all(reqwest::header::VARY) {
        let Ok(s) = val.to_str() else { continue };
        for part in s.split(',') {
            let p = part.trim();
            if p == "*" || p.eq_ignore_ascii_case("cookie") || p.eq_ignore_ascii_case("authorization") {
                return true;
            }
        }
    }
    false
}

/// Compute `(storeable, expires_at_unix, must_revalidate)` from response headers.
fn evaluate_policy(
    bucket: &str,
    url: &str,
    status: u16,
    content_type: &str,
    headers: &HeaderMap,
    has_validators: bool,
    now: u64,
) -> Option<(u64, bool)> {
    // Only cache standard GET success / permanent redirect statuses.
    if !matches!(status, 200 | 203 | 204 | 301) {
        return None;
    }
    // Never cache responses carrying Set-Cookie so auth/session state stays exact.
    if headers.contains_key(reqwest::header::SET_COOKIE) {
        return None;
    }
    if vary_forbids_cache(headers) {
        return None;
    }
    let cc = parse_cache_control(headers);
    if cc.no_store {
        return None;
    }

    if cc.no_cache || cc.max_age == Some(0) {
        // Store only if we can conditionally revalidate later.
        return has_validators.then_some((0, true));
    }

    if let Some(max_age) = cc.max_age {
        return Some((now.saturating_add(max_age), cc.must_revalidate));
    }

    if cc.immutable {
        return Some((now.saturating_add(IMMUTABLE_DEFAULT_SECS), false));
    }

    // Check `Expires: 0` or `-1` as immediate staleness.
    if let Some(exp) = header_str(headers, reqwest::header::EXPIRES)
        && (exp == "0" || exp == "-1")
    {
        return has_validators.then_some((0, true));
    }

    // Heuristic freshness: static subresources get a short fresh window; dynamic
    // documents/API JSON without explicit `max-age` only keep validators for 304.
    if bucket != "doc" && looks_like_static_asset(url, content_type) {
        return Some((now.saturating_add(STATIC_HEURISTIC_SECS), false));
    }
    if has_validators {
        return Some((0, true));
    }
    None
}

fn encode_disk_entry(entry: &CachedEntry) -> Vec<u8> {
    let mut header = String::with_capacity(256 + entry.url.len());
    header.push_str("ZCACHE1\n");
    header.push_str("url=");
    header.push_str(&sanitize_header(&entry.url));
    header.push('\n');
    header.push_str("bucket=");
    header.push_str(&sanitize_header(&entry.bucket));
    header.push('\n');
    header.push_str(&format!("status={}\n", entry.status));
    header.push_str("content_type=");
    header.push_str(&sanitize_header(&entry.content_type));
    header.push('\n');
    if let Some(etag) = &entry.etag {
        header.push_str("etag=");
        header.push_str(&sanitize_header(etag));
        header.push('\n');
    }
    if let Some(lm) = &entry.last_modified {
        header.push_str("last_modified=");
        header.push_str(&sanitize_header(lm));
        header.push('\n');
    }
    header.push_str(&format!("stored_at={}\n", entry.stored_at));
    header.push_str(&format!("expires_at={}\n", entry.expires_at));
    header.push_str(&format!("must_revalidate={}\n\n", u8::from(entry.must_revalidate)));
    let mut out = Vec::with_capacity(header.len() + entry.body.len());
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(&entry.body);
    out
}

fn decode_disk_entry(raw: &[u8]) -> Option<CachedEntry> {
    let split = raw.windows(2).position(|w| w == b"\n\n")?;
    let head = std::str::from_utf8(&raw[..split]).ok()?;
    let body = raw[split + 2..].to_vec();
    let mut lines = head.lines();
    if lines.next()? != "ZCACHE1" {
        return None;
    }
    let mut url = String::new();
    let mut bucket = String::new();
    let mut status = 200u16;
    let mut content_type = String::new();
    let mut etag = None;
    let mut last_modified = None;
    let mut stored_at = 0u64;
    let mut expires_at = 0u64;
    let mut must_revalidate = false;

    for line in lines {
        let Some((k, v)) = line.split_once('=') else { continue };
        match k {
            "url" => url = v.to_string(),
            "bucket" => bucket = v.to_string(),
            "status" => status = v.parse().ok()?,
            "content_type" => content_type = v.to_string(),
            "etag" => etag = Some(v.to_string()),
            "last_modified" => last_modified = Some(v.to_string()),
            "stored_at" => stored_at = v.parse().unwrap_or(0),
            "expires_at" => expires_at = v.parse().unwrap_or(0),
            "must_revalidate" => must_revalidate = v == "1",
            _ => {}
        }
    }
    if url.is_empty() {
        return None;
    }
    Some(CachedEntry {
        url,
        bucket,
        status,
        content_type,
        etag,
        last_modified,
        stored_at,
        expires_at,
        must_revalidate,
        body: Arc::new(body),
    })
}

impl HttpCache {
    /// Construct a cache backed by `dir` (or memory-only if `None`).
    pub fn with_dir(dir: Option<PathBuf>) -> Self {
        if let Some(ref d) = dir {
            let _ = std::fs::create_dir_all(d);
        }
        Self {
            dir,
            mem: Mutex::new(MemState::new()),
            disabled: AtomicBool::new(false),
            max_disk_bytes: AtomicU64::new(DEFAULT_MAX_DISK_BYTES),
            writes_since_trim: AtomicU64::new(0),
            hits: AtomicU64::new(0),
            revalidated: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }

    pub fn set_disabled(&self, disabled: bool) {
        self.disabled.store(disabled, Ordering::Relaxed);
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled.load(Ordering::Relaxed)
    }

    pub fn set_max_disk_bytes(&self, bytes: u64) {
        let clamped = bytes.max(8 * 1024 * 1024);
        self.max_disk_bytes.store(clamped, Ordering::Relaxed);
        if let Some(dir) = &self.dir {
            trim_disk_dir(dir, clamped);
        }
    }

    pub fn max_disk_bytes(&self) -> u64 {
        self.max_disk_bytes.load(Ordering::Relaxed)
    }

    fn disk_path(&self, key: &str) -> Option<PathBuf> {
        Some(self.dir.as_ref()?.join(format!("{key}.zcache")))
    }

    /// Look up `url` in `bucket` (`"doc"` for HTML navigations, `"sub"` for assets/fetch).
    pub fn lookup(&self, bucket: &str, url: &str) -> Lookup {
        if self.is_disabled() || !(url.starts_with("http://") || url.starts_with("https://")) {
            return Lookup::Miss;
        }
        let key = cache_key(bucket, url);
        let entry = {
            let mut mem = self.mem.lock().unwrap_or_else(|e| e.into_inner());
            mem.get(&key).filter(|e| e.url == url && e.bucket == bucket)
        };
        let entry = match entry {
            Some(e) => e,
            None => match self.read_disk(&key, bucket, url) {
                Some(e) => {
                    self.mem.lock().unwrap_or_else(|e| e.into_inner()).insert(key, e.clone());
                    e
                }
                None => {
                    self.misses.fetch_add(1, Ordering::Relaxed);
                    return Lookup::Miss;
                }
            },
        };

        let now = unix_now();
        if !entry.must_revalidate && entry.expires_at > now {
            self.hits.fetch_add(1, Ordering::Relaxed);
            Lookup::Fresh(entry)
        } else if entry.expires_at > now {
            // `must-revalidate` still serves fresh while not expired.
            self.hits.fetch_add(1, Ordering::Relaxed);
            Lookup::Fresh(entry)
        } else if entry.etag.is_some() || entry.last_modified.is_some() {
            Lookup::Revalidate(entry)
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            Lookup::Miss
        }
    }

    /// Store a GET response if permitted by HTTP caching headers.
    pub fn store(&self, bucket: &str, url: &str, status: u16, headers: &HeaderMap, body: Vec<u8>) {
        if self.is_disabled()
            || !(url.starts_with("http://") || url.starts_with("https://"))
            || body.len() > MAX_ENTRY_BYTES
        {
            return;
        }
        let content_type = header_str(headers, reqwest::header::CONTENT_TYPE).unwrap_or_default();
        let etag = header_str(headers, reqwest::header::ETAG);
        let last_modified = header_str(headers, reqwest::header::LAST_MODIFIED);
        let has_validators = etag.is_some() || last_modified.is_some();
        let now = unix_now();

        let Some((expires_at, must_revalidate)) =
            evaluate_policy(bucket, url, status, &content_type, headers, has_validators, now)
        else {
            // If the server explicitly returned `no-store` or `Set-Cookie`, remove any older cached copy.
            self.remove_key(&cache_key(bucket, url));
            return;
        };

        let entry = CachedEntry {
            url: url.to_string(),
            bucket: bucket.to_string(),
            status,
            content_type,
            etag,
            last_modified,
            stored_at: now,
            expires_at,
            must_revalidate,
            body: Arc::new(body),
        };
        let key = cache_key(bucket, url);
        self.write_disk(&key, &entry);
        self.mem.lock().unwrap_or_else(|e| e.into_inner()).insert(key, entry);
    }

    /// Update freshness metadata after a `304 Not Modified` response and return the refreshed entry.
    pub fn revalidate_304(
        &self,
        bucket: &str,
        url: &str,
        previous: CachedEntry,
        headers: &HeaderMap,
    ) -> CachedEntry {
        let now = unix_now();
        let content_type = header_str(headers, reqwest::header::CONTENT_TYPE)
            .unwrap_or(previous.content_type);
        let etag = header_str(headers, reqwest::header::ETAG).or(previous.etag);
        let last_modified = header_str(headers, reqwest::header::LAST_MODIFIED)
            .or(previous.last_modified);
        let has_validators = etag.is_some() || last_modified.is_some();
        let (expires_at, must_revalidate) =
            evaluate_policy(bucket, url, 200, &content_type, headers, has_validators, now)
                .unwrap_or((0, true));

        let updated = CachedEntry {
            url: previous.url,
            bucket: previous.bucket,
            status: previous.status,
            content_type,
            etag,
            last_modified,
            stored_at: now,
            expires_at,
            must_revalidate,
            body: previous.body,
        };
        self.revalidated.fetch_add(1, Ordering::Relaxed);
        let key = cache_key(bucket, url);
        self.write_disk(&key, &updated);
        self.mem.lock().unwrap_or_else(|e| e.into_inner()).insert(key, updated.clone());
        updated
    }

    /// Remove cached entries for `url` across both `"doc"` and `"sub"` buckets.
    pub fn invalidate_url(&self, url: &str) {
        for bucket in ["doc", "sub"] {
            self.remove_key(&cache_key(bucket, url));
        }
    }

    fn remove_key(&self, key: &str) {
        self.mem.lock().unwrap_or_else(|e| e.into_inner()).remove(key);
        if let Some(path) = self.disk_path(key) {
            let _ = std::fs::remove_file(path);
        }
    }

    /// Purge all cached entries from memory and disk. Returns `(entries_removed, bytes_freed)`.
    pub fn clear(&self) -> (usize, u64) {
        let (mem_count, mem_bytes) = self.mem.lock().unwrap_or_else(|e| e.into_inner()).clear();
        let mut disk_count = 0usize;
        let mut disk_bytes = 0u64;
        if let Some(dir) = &self.dir
            && let Ok(read) = std::fs::read_dir(dir)
        {
            for entry in read.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()).is_some_and(|e| e == "zcache" || e == "tmp") {
                    if let Ok(meta) = entry.metadata() {
                        disk_bytes += meta.len();
                    }
                    if std::fs::remove_file(&path).is_ok() {
                        disk_count += 1;
                    }
                }
            }
        }
        self.hits.store(0, Ordering::Relaxed);
        self.revalidated.store(0, Ordering::Relaxed);
        self.misses.store(0, Ordering::Relaxed);
        (disk_count.max(mem_count), disk_bytes.max(mem_bytes))
    }

    /// Current occupancy and hit/miss statistics.
    pub fn stats(&self) -> CacheStats {
        let (mem_entries, mem_bytes) = {
            let mem = self.mem.lock().unwrap_or_else(|e| e.into_inner());
            (mem.entries.len(), mem.total_bytes)
        };
        let mut disk_entries = 0usize;
        let mut disk_bytes = 0u64;
        if let Some(dir) = &self.dir
            && let Ok(read) = std::fs::read_dir(dir)
        {
            for entry in read.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("zcache") {
                    disk_entries += 1;
                    if let Ok(meta) = entry.metadata() {
                        disk_bytes += meta.len();
                    }
                }
            }
        }
        CacheStats {
            mem_entries,
            mem_bytes,
            disk_entries,
            disk_bytes,
            hits: self.hits.load(Ordering::Relaxed),
            revalidated: self.revalidated.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
        }
    }

    fn read_disk(&self, key: &str, bucket: &str, url: &str) -> Option<CachedEntry> {
        let path = self.disk_path(key)?;
        let raw = std::fs::read(&path).ok()?;
        let entry = decode_disk_entry(&raw)?;
        if entry.url == url && entry.bucket == bucket {
            Some(entry)
        } else {
            None
        }
    }

    fn write_disk(&self, key: &str, entry: &CachedEntry) {
        let Some(dir) = &self.dir else { return };
        let _ = std::fs::create_dir_all(dir);
        let path = dir.join(format!("{key}.zcache"));
        let tmp = dir.join(format!("{key}.{}.tmp", std::process::id()));
        let bytes = encode_disk_entry(entry);
        if std::fs::write(&tmp, &bytes).is_ok() && std::fs::rename(&tmp, &path).is_err() {
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::rename(&tmp, &path);
        }
        let n = self.writes_since_trim.fetch_add(1, Ordering::Relaxed) + 1;
        if n.is_multiple_of(TRIM_EVERY_WRITES) {
            trim_disk_dir(dir, self.max_disk_bytes());
        }
    }
}

fn trim_disk_dir(dir: &Path, max_bytes: u64) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
    let mut total = 0u64;
    for entry in read.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("zcache") {
            continue;
        }
        if let Ok(meta) = entry.metadata() {
            let len = meta.len();
            let modified = meta.modified().unwrap_or(UNIX_EPOCH);
            total += len;
            files.push((modified, len, path));
        }
    }
    if total <= max_bytes {
        return;
    }
    files.sort_by_key(|(m, _, _)| *m);
    for (_, len, path) in files {
        if total <= max_bytes {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{CACHE_CONTROL, CONTENT_TYPE, ETAG, HeaderValue, SET_COOKIE, VARY};

    fn temp_dir(label: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "zipp-cache-test-{}-{}-{}",
            label,
            std::process::id(),
            unix_now()
        ));
        let _ = std::fs::remove_dir_all(&p);
        p
    }

    #[test]
    fn stores_persists_to_disk_and_clears() {
        let dir = temp_dir("persist");
        let url = "https://example.com/assets/app.js";
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/javascript"));
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("public, max-age=3600"));

        {
            let cache = HttpCache::with_dir(Some(dir.clone()));
            assert!(matches!(cache.lookup("sub", url), Lookup::Miss));
            cache.store("sub", url, 200, &headers, b"console.log(1);".to_vec());
            match cache.lookup("sub", url) {
                Lookup::Fresh(e) => {
                    assert_eq!(e.body.as_slice(), b"console.log(1);");
                    assert_eq!(e.content_type, "text/javascript");
                }
                other => panic!("expected Fresh, got {other:?}"),
            }
        }

        // Re-open fresh instance from disk only -> still a Fresh hit!
        let cache2 = HttpCache::with_dir(Some(dir.clone()));
        match cache2.lookup("sub", url) {
            Lookup::Fresh(e) => assert_eq!(e.body.as_slice(), b"console.log(1);"),
            other => panic!("expected disk Fresh hit, got {other:?}"),
        }

        // Clear purges both memory and disk.
        let (removed, freed) = cache2.clear();
        assert!(removed >= 1);
        assert!(freed > 0);
        assert!(matches!(cache2.lookup("sub", url), Lookup::Miss));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn respects_no_store_and_set_cookie_and_vary() {
        let dir = temp_dir("nostore");
        let cache = HttpCache::with_dir(Some(dir.clone()));

        let mut no_store = HeaderMap::new();
        no_store.insert(CACHE_CONTROL, HeaderValue::from_static("no-store, max-age=600"));
        cache.store("doc", "https://example.com/secret", 200, &no_store, b"secret".to_vec());
        assert!(matches!(cache.lookup("doc", "https://example.com/secret"), Lookup::Miss));

        let mut with_cookie = HeaderMap::new();
        with_cookie.insert(CACHE_CONTROL, HeaderValue::from_static("max-age=600"));
        with_cookie.insert(SET_COOKIE, HeaderValue::from_static("sid=abc; HttpOnly"));
        cache.store("doc", "https://example.com/login", 200, &with_cookie, b"ok".to_vec());
        assert!(matches!(cache.lookup("doc", "https://example.com/login"), Lookup::Miss));

        let mut vary_cookie = HeaderMap::new();
        vary_cookie.insert(CACHE_CONTROL, HeaderValue::from_static("max-age=600"));
        vary_cookie.insert(VARY, HeaderValue::from_static("Cookie"));
        cache.store("doc", "https://example.com/me", 200, &vary_cookie, b"user".to_vec());
        assert!(matches!(cache.lookup("doc", "https://example.com/me"), Lookup::Miss));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_cache_with_etag_triggers_revalidation_and_304_refreshes() {
        let dir = temp_dir("etag");
        let cache = HttpCache::with_dir(Some(dir.clone()));
        let url = "https://example.com/data.json";

        let mut h = HeaderMap::new();
        h.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        h.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        h.insert(ETAG, HeaderValue::from_static("\"v1\""));
        cache.store("sub", url, 200, &h, b"{\"ok\":true}".to_vec());

        let prev = match cache.lookup("sub", url) {
            Lookup::Revalidate(e) => {
                assert_eq!(e.etag.as_deref(), Some("\"v1\""));
                e
            }
            other => panic!("expected Revalidate, got {other:?}"),
        };

        let mut h304 = HeaderMap::new();
        h304.insert(CACHE_CONTROL, HeaderValue::from_static("max-age=120"));
        h304.insert(ETAG, HeaderValue::from_static("\"v1\""));
        let refreshed = cache.revalidate_304("sub", url, prev, &h304);
        assert_eq!(refreshed.body.as_slice(), b"{\"ok\":true}");
        assert!(matches!(cache.lookup("sub", url), Lookup::Fresh(_)));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn doc_and_sub_buckets_do_not_collide() {
        let dir = temp_dir("buckets");
        let cache = HttpCache::with_dir(Some(dir.clone()));
        let url = "https://softn.com/play/demo";

        let mut h = HeaderMap::new();
        h.insert(CACHE_CONTROL, HeaderValue::from_static("max-age=600"));
        cache.store("doc", url, 200, &h, b"<html>app</html>".to_vec());
        cache.store("sub", url, 200, &h, b"{\"api\":1}".to_vec());

        match cache.lookup("doc", url) {
            Lookup::Fresh(e) => assert_eq!(e.body.as_slice(), b"<html>app</html>"),
            other => panic!("expected doc Fresh, got {other:?}"),
        }
        match cache.lookup("sub", url) {
            Lookup::Fresh(e) => assert_eq!(e.body.as_slice(), b"{\"api\":1}"),
            other => panic!("expected sub Fresh, got {other:?}"),
        }

        cache.invalidate_url(url);
        assert!(matches!(cache.lookup("doc", url), Lookup::Miss));
        assert!(matches!(cache.lookup("sub", url), Lookup::Miss));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
