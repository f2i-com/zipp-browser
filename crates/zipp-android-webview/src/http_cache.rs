//! Persistent HTTP disk + memory cache, per-app cookie jar, and shared `blob:`
//! URL store for the SoftN Android mobile runtime.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use url::Url;

// ---------------------------------------------------------------------------
// Shared `blob:` URL store (main VM + VfsNetProvider + Canvas 2D + WebGL)
// ---------------------------------------------------------------------------

type BlobMap = HashMap<String, (String, Vec<u8>)>;
static BLOB_STORE: LazyLock<Mutex<BlobMap>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static NEXT_BLOB_ID: AtomicU64 = AtomicU64::new(1);

pub fn store_blob(origin: &str, mime: &str, bytes: Vec<u8>) -> String {
    let id = NEXT_BLOB_ID.fetch_add(1, Ordering::Relaxed);
    let base = if origin.is_empty() || origin == "null" {
        "null"
    } else {
        origin.trim_end_matches('/')
    };
    let url = format!("blob:{base}/softn-{id:016x}");
    BLOB_STORE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(url.clone(), (mime.to_string(), bytes));
    url
}

pub fn lookup_blob(url: &str) -> Option<(String, Vec<u8>)> {
    BLOB_STORE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(url)
        .cloned()
}

pub fn lookup_blob_bytes(url: &str) -> Option<Vec<u8>> {
    lookup_blob(url).map(|(_, bytes)| bytes)
}

pub fn revoke_blob(url: &str) {
    BLOB_STORE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(url);
}

pub fn clear_blobs() {
    BLOB_STORE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
}

// ---------------------------------------------------------------------------
// Per-app Cookie Jar (session cookies in memory + persistent cookies on disk)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoredCookie {
    name: String,
    value: String,
    domain: String,
    host_only: bool,
    path: String,
    secure: bool,
    http_only: bool,
    expires_at: Option<u64>, // None = session cookie
}

pub struct MobileCookieJar {
    cookies: Mutex<Vec<StoredCookie>>,
    path: Option<PathBuf>,
}

impl MobileCookieJar {
    pub fn open(app_data_dir: &Path) -> Self {
        let path = app_data_dir.join("cookies.json");
        let now = unix_now();
        let initial: Vec<StoredCookie> = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Vec<StoredCookie>>(&s).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|c| c.expires_at.is_some_and(|exp| exp > now))
            .collect();
        Self {
            cookies: Mutex::new(initial),
            path: Some(path),
        }
    }

    fn save_locked(&self, list: &[StoredCookie]) {
        let Some(ref path) = self.path else {
            return;
        };
        let now = unix_now();
        let persistent: Vec<&StoredCookie> = list
            .iter()
            .filter(|c| c.expires_at.is_some_and(|exp| exp > now))
            .collect();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string(&persistent) {
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, path);
            }
        }
    }

    /// Insert a single parsed cookie into `list`. Returns `true` if the set of persistent
    /// (non-session) cookies changed and therefore needs to be flushed to disk.
    fn insert_cookie_locked(
        list: &mut Vec<StoredCookie>,
        url: &Url,
        raw: &str,
        from_http: bool,
    ) -> bool {
        let Some(host) = url.host_str().map(|h| h.to_ascii_lowercase()) else {
            return false;
        };
        let mut parts = raw.split(';');
        let Some(nv) = parts.next() else {
            return false;
        };
        let Some((name, value)) = nv.split_once('=') else {
            return false;
        };
        let name = name.trim().to_string();
        if name.is_empty() {
            return false;
        }
        let value = value.trim().to_string();

        let mut domain = host.clone();
        let mut host_only = true;
        let mut path = default_cookie_path(url.path());
        let mut secure = false;
        let mut http_only = false;
        let mut expires_at: Option<u64> = None;
        let mut expired_now = false;

        for attr in parts {
            let attr = attr.trim();
            if attr.is_empty() {
                continue;
            }
            if let Some((k, v)) = attr.split_once('=') {
                let k_low = k.trim().to_ascii_lowercase();
                let v = v.trim();
                match k_low.as_str() {
                    "domain" => {
                        let d = v.trim_start_matches('.').to_ascii_lowercase();
                        if !d.is_empty() && domain_matches(&host, &d) {
                            domain = d;
                            host_only = false;
                        }
                    }
                    "path" if v.starts_with('/') => {
                        path = v.to_string();
                    }
                    "max-age" => {
                        if let Ok(secs) = v.parse::<i64>() {
                            if secs <= 0 {
                                expired_now = true;
                            } else {
                                expires_at = Some(unix_now().saturating_add(secs as u64));
                            }
                        }
                    }
                    "expires" => {
                        if expires_at.is_none() && v.contains("1970") {
                            expired_now = true;
                        }
                    }
                    _ => {}
                }
            } else {
                match attr.to_ascii_lowercase().as_str() {
                    "secure" => secure = true,
                    "httponly" => http_only = true,
                    _ => {}
                }
            }
        }

        if !from_http && http_only {
            return false;
        }

        let now = unix_now();
        let mut persistent_changed = false;
        list.retain(|c| {
            let keep = c.expires_at.is_none_or(|exp| exp > now);
            if !keep && c.expires_at.is_some() {
                persistent_changed = true;
            }
            keep
        });

        if !from_http
            && list.iter().any(|c| {
                c.name == name
                    && c.http_only
                    && c.path == path
                    && matches_domain(&host, &c.domain, c.host_only)
            })
        {
            return persistent_changed;
        }

        list.retain(|c| {
            let same = c.name == name && c.domain == domain && c.path == path;
            if same && c.expires_at.is_some() {
                persistent_changed = true;
            }
            !same
        });
        if !expired_now {
            if expires_at.is_some() {
                persistent_changed = true;
            }
            list.push(StoredCookie {
                name,
                value,
                domain,
                host_only,
                path,
                secure,
                http_only,
                expires_at,
            });
        }
        persistent_changed
    }

    pub fn set_from_response(&self, url: &Url, set_cookie_headers: &[String]) {
        if set_cookie_headers.is_empty() {
            return;
        }
        let mut list = self.cookies.lock().unwrap_or_else(|e| e.into_inner());
        let mut needs_save = false;
        for hdr in set_cookie_headers {
            needs_save |= Self::insert_cookie_locked(&mut list, url, hdr, true);
        }
        if needs_save {
            self.save_locked(&list);
        }
    }

    pub fn set_document_cookie(&self, url: &Url, raw: &str) {
        let mut list = self.cookies.lock().unwrap_or_else(|e| e.into_inner());
        if Self::insert_cookie_locked(&mut list, url, raw, false) {
            self.save_locked(&list);
        }
    }

    pub fn document_cookie(&self, url: &Url) -> String {
        self.collect_for_url(url, false)
    }

    pub fn cookie_header(&self, url: &Url) -> Option<String> {
        let s = self.collect_for_url(url, true);
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }

    fn collect_for_url(&self, url: &Url, include_http_only: bool) -> String {
        let Some(host) = url.host_str().map(|h| h.to_ascii_lowercase()) else {
            return String::new();
        };
        let is_secure = url.scheme() == "https";
        let req_path = url.path();
        let now = unix_now();
        let list = self.cookies.lock().unwrap_or_else(|e| e.into_inner());
        list.iter()
            .filter(|c| {
                c.expires_at.is_none_or(|exp| exp > now)
                    && (include_http_only || !c.http_only)
                    && (!c.secure || is_secure)
                    && matches_domain(&host, &c.domain, c.host_only)
                    && path_matches(req_path, &c.path)
            })
            .map(|c| format!("{}={}", c.name, c.value))
            .collect::<Vec<_>>()
            .join("; ")
    }

    pub fn clear(&self) -> usize {
        let mut list = self.cookies.lock().unwrap_or_else(|e| e.into_inner());
        let count = list.len();
        list.clear();
        if let Some(ref path) = self.path {
            let _ = std::fs::remove_file(path);
        }
        count
    }
}

fn default_cookie_path(req_path: &str) -> String {
    if !req_path.starts_with('/') {
        return "/".to_string();
    }
    match req_path.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(idx) => req_path[..idx].to_string(),
    }
}

fn domain_matches(host: &str, domain: &str) -> bool {
    host == domain
        || (host.len() > domain.len()
            && host.ends_with(domain)
            && host.as_bytes()[host.len() - domain.len() - 1] == b'.')
}

fn matches_domain(host: &str, cookie_domain: &str, host_only: bool) -> bool {
    if host_only {
        host == cookie_domain
    } else {
        domain_matches(host, cookie_domain)
    }
}

fn path_matches(req_path: &str, cookie_path: &str) -> bool {
    if req_path == cookie_path || cookie_path == "/" {
        return true;
    }
    if req_path.starts_with(cookie_path) {
        if cookie_path.ends_with('/') {
            return true;
        }
        return req_path.as_bytes().get(cookie_path.len()) == Some(&b'/');
    }
    false
}

// ---------------------------------------------------------------------------
// Persistent HTTP Disk + Memory Cache (for `net`-permitted external fetches)
// ---------------------------------------------------------------------------

const MAX_MEM_BYTES: usize = 16 * 1024 * 1024;
const MAX_MEM_ENTRIES: usize = 256;
const MAX_ENTRY_BYTES: usize = 8 * 1024 * 1024;
const MAX_DISK_BYTES: u64 = 64 * 1024 * 1024;
const STATIC_HEURISTIC_SECS: u64 = 300;
const IMMUTABLE_DEFAULT_SECS: u64 = 7 * 24 * 3600;

#[derive(Clone, Debug)]
pub struct CachedEntry {
    pub url: String,
    pub status: u16,
    pub content_type: String,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub expires_at: u64,
    pub body: Arc<Vec<u8>>,
}

pub enum CacheLookup {
    Fresh(CachedEntry),
    Revalidate(CachedEntry),
    Miss,
}

struct MemCache {
    entries: HashMap<String, (u64, CachedEntry)>,
    total_bytes: usize,
    tick: u64,
}

impl MemCache {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            total_bytes: 0,
            tick: 0,
        }
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
            if let Some(k) = oldest {
                if let Some((_, old)) = self.entries.remove(&k) {
                    self.total_bytes = self.total_bytes.saturating_sub(old.body.len());
                }
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
        let c = self.entries.len();
        let b = self.total_bytes as u64;
        self.entries.clear();
        self.total_bytes = 0;
        (c, b)
    }
}

pub struct MobileHttpCache {
    dir: PathBuf,
    mem: Mutex<MemCache>,
    writes: AtomicU64,
}

impl MobileHttpCache {
    pub fn open(app_data_dir: &Path) -> Self {
        let dir = app_data_dir.join("http_cache");
        let _ = std::fs::create_dir_all(&dir);
        Self {
            dir,
            mem: Mutex::new(MemCache::new()),
            writes: AtomicU64::new(0),
        }
    }

    pub fn lookup(&self, url: &str) -> CacheLookup {
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return CacheLookup::Miss;
        }
        let key = hash_key(url);
        let entry = {
            let mut mem = self.mem.lock().unwrap_or_else(|e| e.into_inner());
            mem.get(&key).filter(|e| e.url == url)
        };
        let entry = match entry {
            Some(e) => e,
            None => match self.read_disk(&key, url) {
                Some(e) => {
                    self.mem
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(key, e.clone());
                    e
                }
                None => return CacheLookup::Miss,
            },
        };
        let now = unix_now();
        if entry.expires_at > now {
            CacheLookup::Fresh(entry)
        } else if entry.etag.is_some() || entry.last_modified.is_some() {
            CacheLookup::Revalidate(entry)
        } else {
            CacheLookup::Miss
        }
    }

    pub fn store(
        &self,
        url: &str,
        status: u16,
        headers: &[(String, String)],
        body: Vec<u8>,
    ) {
        if !(url.starts_with("http://") || url.starts_with("https://"))
            || body.len() > MAX_ENTRY_BYTES
            || !matches!(status, 200 | 203 | 204 | 301)
        {
            return;
        }
        let key = hash_key(url);
        if headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("set-cookie")) {
            self.remove_key(&key);
            return;
        }
        if headers.iter().any(|(k, v)| {
            k.eq_ignore_ascii_case("vary")
                && v.split(',').any(|p| {
                    let t = p.trim();
                    t == "*"
                        || t.eq_ignore_ascii_case("cookie")
                        || t.eq_ignore_ascii_case("authorization")
                })
        }) {
            return;
        }

        let content_type = find_hdr(headers, "content-type").unwrap_or_default();
        let etag = find_hdr(headers, "etag");
        let last_modified = find_hdr(headers, "last-modified");
        let has_validators = etag.is_some() || last_modified.is_some();
        let cc = find_hdr(headers, "cache-control").unwrap_or_default();

        let mut no_store = false;
        let mut no_cache = false;
        let mut immutable = false;
        let mut max_age: Option<u64> = None;
        for token in cc.split(',') {
            let t = token.trim();
            if t.eq_ignore_ascii_case("no-store") {
                no_store = true;
            } else if t.eq_ignore_ascii_case("no-cache") {
                no_cache = true;
            } else if t.eq_ignore_ascii_case("immutable") {
                immutable = true;
            } else if let Some((k, v)) = t.split_once('=') {
                if k.trim().eq_ignore_ascii_case("max-age") {
                    if let Ok(s) = v.trim().trim_matches('"').parse::<u64>() {
                        max_age = Some(s);
                    }
                }
            }
        }
        if no_store {
            self.remove_key(&key);
            return;
        }

        let now = unix_now();
        let expires_at = if no_cache || max_age == Some(0) {
            if !has_validators {
                return;
            }
            0
        } else if let Some(ma) = max_age {
            now.saturating_add(ma)
        } else if immutable {
            now.saturating_add(IMMUTABLE_DEFAULT_SECS)
        } else if is_static_asset(url, &content_type) {
            now.saturating_add(STATIC_HEURISTIC_SECS)
        } else if has_validators {
            0
        } else {
            return;
        };

        let entry = CachedEntry {
            url: url.to_string(),
            status,
            content_type,
            etag,
            last_modified,
            expires_at,
            body: Arc::new(body),
        };
        self.write_disk(&key, &entry);
        self.mem
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, entry);
    }

    pub fn revalidate_304(
        &self,
        url: &str,
        prev: CachedEntry,
        headers: &[(String, String)],
    ) -> CachedEntry {
        let body = prev.body.as_ref().clone();
        let mut merged = headers.to_vec();
        if find_hdr(&merged, "content-type").is_none() && !prev.content_type.is_empty() {
            merged.push(("content-type".into(), prev.content_type.clone()));
        }
        if find_hdr(&merged, "etag").is_none() {
            if let Some(ref e) = prev.etag {
                merged.push(("etag".into(), e.clone()));
            }
        }
        if find_hdr(&merged, "last-modified").is_none() {
            if let Some(ref lm) = prev.last_modified {
                merged.push(("last-modified".into(), lm.clone()));
            }
        }
        self.store(url, prev.status, &merged, body);
        match self.lookup(url) {
            CacheLookup::Fresh(e) | CacheLookup::Revalidate(e) => e,
            CacheLookup::Miss => prev,
        }
    }

    pub fn invalidate(&self, url: &str) {
        self.remove_key(&hash_key(url));
    }

    fn remove_key(&self, key: &str) {
        self.mem.lock().unwrap_or_else(|e| e.into_inner()).remove(key);
        let _ = std::fs::remove_file(self.dir.join(format!("{key}.zcache")));
    }

    pub fn clear(&self) -> (usize, u64) {
        let (mc, mb) = self.mem.lock().unwrap_or_else(|e| e.into_inner()).clear();
        let mut dc = 0usize;
        let mut db = 0u64;
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension()
                    .and_then(|x| x.to_str())
                    .map(|x| x == "zcache" || x == "tmp")
                    .unwrap_or(false)
                {
                    if let Ok(m) = e.metadata() {
                        db += m.len();
                    }
                    if std::fs::remove_file(&p).is_ok() {
                        dc += 1;
                    }
                }
            }
        }
        (dc.max(mc), db.max(mb))
    }

    fn read_disk(&self, key: &str, url: &str) -> Option<CachedEntry> {
        let raw = std::fs::read(self.dir.join(format!("{key}.zcache"))).ok()?;
        let split = raw.windows(2).position(|w| w == b"\n\n")?;
        let head = std::str::from_utf8(&raw[..split]).ok()?;
        let mut lines = head.lines();
        if lines.next()? != "ZCACHE1" {
            return None;
        }
        let mut e_url = String::new();
        let mut status = 200u16;
        let mut content_type = String::new();
        let mut etag = None;
        let mut last_modified = None;
        let mut expires_at = 0u64;
        for line in lines {
            if let Some((k, v)) = line.split_once('=') {
                match k {
                    "url" => e_url = v.to_string(),
                    "status" => status = v.parse().ok()?,
                    "content_type" => content_type = v.to_string(),
                    "etag" => etag = Some(v.to_string()),
                    "last_modified" => last_modified = Some(v.to_string()),
                    "expires_at" => expires_at = v.parse().unwrap_or(0),
                    _ => {}
                }
            }
        }
        if e_url != url {
            return None;
        }
        Some(CachedEntry {
            url: e_url,
            status,
            content_type,
            etag,
            last_modified,
            expires_at,
            body: Arc::new(raw[split + 2..].to_vec()),
        })
    }

    fn write_disk(&self, key: &str, entry: &CachedEntry) {
        let _ = std::fs::create_dir_all(&self.dir);
        let mut hdr = format!(
            "ZCACHE1\nurl={}\nstatus={}\ncontent_type={}\nexpires_at={}\n",
            clean_hdr(&entry.url),
            entry.status,
            clean_hdr(&entry.content_type),
            entry.expires_at
        );
        if let Some(ref e) = entry.etag {
            hdr.push_str(&format!("etag={}\n", clean_hdr(e)));
        }
        if let Some(ref lm) = entry.last_modified {
            hdr.push_str(&format!("last_modified={}\n", clean_hdr(lm)));
        }
        hdr.push('\n');
        let mut out = Vec::with_capacity(hdr.len() + entry.body.len());
        out.extend_from_slice(hdr.as_bytes());
        out.extend_from_slice(&entry.body);
        let path = self.dir.join(format!("{key}.zcache"));
        let tmp = self.dir.join(format!("{key}.tmp"));
        if std::fs::write(&tmp, out).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
        let n = self.writes.fetch_add(1, Ordering::Relaxed) + 1;
        if n.is_multiple_of(32) {
            trim_disk(&self.dir, MAX_DISK_BYTES);
        }
    }
}

fn trim_disk(dir: &Path, max_bytes: u64) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut files = Vec::new();
    let mut total = 0u64;
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("zcache") {
            continue;
        }
        if let Ok(m) = e.metadata() {
            total += m.len();
            files.push((m.modified().unwrap_or(UNIX_EPOCH), m.len(), p));
        }
    }
    if total <= max_bytes {
        return;
    }
    files.sort_by_key(|(m, _, _)| *m);
    for (_, len, p) in files {
        if total <= max_bytes {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

fn find_hdr(headers: &[(String, String)], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn clean_hdr(s: &str) -> String {
    s.chars().filter(|&c| c != '\r' && c != '\n').collect()
}

fn hash_key(url: &str) -> String {
    let mut h1: u64 = 0xcbf29ce484222325;
    let mut h2: u64 = 0x84222325cbf29ce4;
    for b in url.bytes() {
        h1 = (h1 ^ b as u64).wrapping_mul(0x100000001b3);
        h2 = h2.rotate_left(7) ^ (b as u64).wrapping_mul(0x9e3779b97f4a7c15);
    }
    format!("{h1:016x}{h2:016x}")
}

fn is_static_asset(url: &str, ct: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url).to_ascii_lowercase();
    [
        ".js", ".mjs", ".css", ".wasm", ".png", ".jpg", ".jpeg", ".webp", ".gif", ".svg", ".woff",
        ".woff2", ".ttf", ".glb", ".gltf", ".hdr", ".ico",
    ]
    .iter()
    .any(|ext| path.ends_with(ext))
        || ct.to_ascii_lowercase().starts_with("image/")
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
