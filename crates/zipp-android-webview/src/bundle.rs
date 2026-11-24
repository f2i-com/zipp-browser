//! `.softn` v2 Web App Container bundle loader, manifest parser, and capability gatekeeper.
//!
//! Reads a `.softn` ZIP archive into an in-memory VFS, discovers the frontend entry
//! (`manifest.json` or zero-config `index.html` / `public/index.html` / `dist/index.html`),
//! parses `permission.json`, and enforces capability & egress rules natively in Rust.

use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const VIRTUAL_ORIGIN: &str = "https://app.softn.local";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BackendConfig {
    pub runtime: String,
    pub entry: String,
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub routes: Option<Vec<String>>,
    #[serde(default, rename = "interceptOrigins")]
    pub intercept_origins: Option<Vec<String>>,
    #[serde(default)]
    pub env: Option<HashMap<String, String>>,
    #[serde(default, rename = "phpMode")]
    pub php_mode: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DatabaseConfig {
    pub name: String,
    #[serde(default)]
    pub engine: Option<String>,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default, rename = "seedSql")]
    pub seed_sql: Option<String>,
    #[serde(default, rename = "seedJson")]
    pub seed_json: Option<Value>,
    #[serde(default)]
    pub persist: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ContainerManifest {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub main: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default, rename = "frontendRoot")]
    pub frontend_root: Option<String>,
    #[serde(default)]
    pub backend: Option<BackendConfig>,
    #[serde(default)]
    pub databases: Option<Vec<DatabaseConfig>>,
    #[serde(default, rename = "ssrEntry")]
    pub ssr_entry: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct NetPermission {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub allowed_hosts: Option<Vec<String>>,
    #[serde(default)]
    pub allow_http: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct BluetoothPermission {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub services: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct LocationPermission {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub precision: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct BoolPermission {
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct PermissionsMap {
    #[serde(default)]
    pub net: Option<NetPermission>,
    #[serde(default)]
    pub camera: Option<BoolPermission>,
    #[serde(default)]
    pub mic: Option<BoolPermission>,
    #[serde(default)]
    pub files: Option<BoolPermission>,
    #[serde(default)]
    pub qr: Option<BoolPermission>,
    #[serde(default)]
    pub ai: Option<BoolPermission>,
    #[serde(default)]
    pub gpu: Option<BoolPermission>,
    #[serde(default)]
    pub sync: Option<BoolPermission>,
    #[serde(default)]
    pub storage: Option<BoolPermission>,
    #[serde(default)]
    pub accel: Option<BoolPermission>,
    #[serde(default)]
    pub bluetooth: Option<BluetoothPermission>,
    #[serde(default)]
    pub location: Option<LocationPermission>,
    #[serde(default)]
    pub haptics: Option<BoolPermission>,
    #[serde(default)]
    pub notifications: Option<BoolPermission>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct PermissionFile {
    #[serde(default)]
    pub permissions: PermissionsMap,
}

#[derive(Debug, Clone)]
pub struct SoftnBundle {
    pub app_id: String,
    pub digest: String,
    pub name: String,
    pub version: String,
    pub manifest: ContainerManifest,
    pub permissions: PermissionFile,
    pub entry_path: String,
    pub frontend_root: String,
    pub vfs: HashMap<String, Vec<u8>>,
}

/// Normalize a virtual filesystem path (strip leading `/`, `./`, collapse `..`).
pub fn normalize_vfs_path(raw: &str) -> String {
    let normalized = raw.replace('\\', "/");
    let mut parts: Vec<&str> = Vec::new();
    for seg in normalized.split('/') {
        match seg {
            "" | "." => continue,
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

/// Resolve a relative or root-relative URL against `importer_path` and optional `frontend_root`.
pub fn resolve_vfs_url(raw: &str, importer_path: &str, frontend_root: &str) -> Option<String> {
    let stripped = raw
        .split('#')
        .next()
        .unwrap_or(raw)
        .split('?')
        .next()
        .unwrap_or(raw)
        .trim();

    if stripped.is_empty() {
        return None;
    }

    let rel = if let Some(rest) = stripped.strip_prefix(VIRTUAL_ORIGIN) {
        rest
    } else if stripped.starts_with("http://")
        || stripped.starts_with("https://")
        || stripped.starts_with("data:")
        || stripped.starts_with("blob:")
        || stripped.starts_with("softn:")
    {
        return None;
    } else {
        stripped
    };

    if rel.starts_with('/') {
        let clean = normalize_vfs_path(rel);
        if !frontend_root.is_empty() && !clean.starts_with(&format!("{frontend_root}/")) && clean != frontend_root {
            Some(normalize_vfs_path(&format!("{frontend_root}/{clean}")))
        } else {
            Some(clean)
        }
    } else {
        let base_dir = importer_path
            .rsplit_once('/')
            .map(|(dir, _)| dir)
            .unwrap_or("");
        if base_dir.is_empty() {
            Some(normalize_vfs_path(rel))
        } else {
            Some(normalize_vfs_path(&format!("{base_dir}/{rel}")))
        }
    }
}

pub fn mime_type_for(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".html") || lower.ends_with(".htm") {
        "text/html; charset=utf-8"
    } else if lower.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if lower.ends_with(".js") || lower.ends_with(".mjs") {
        "application/javascript; charset=utf-8"
    } else if lower.ends_with(".ts") || lower.ends_with(".tsx") || lower.ends_with(".jsx") {
        "text/typescript; charset=utf-8"
    } else if lower.ends_with(".json") {
        "application/json; charset=utf-8"
    } else if lower.ends_with(".svg") {
        "image/svg+xml"
    } else if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else if lower.ends_with(".ico") {
        "image/x-icon"
    } else if lower.ends_with(".woff2") {
        "font/woff2"
    } else if lower.ends_with(".woff") {
        "font/woff"
    } else if lower.ends_with(".ttf") {
        "font/ttf"
    } else if lower.ends_with(".wasm") {
        "application/wasm"
    } else if lower.ends_with(".txt") || lower.ends_with(".md") {
        "text/plain; charset=utf-8"
    } else {
        "application/octet-stream"
    }
}

fn is_private_asset_path(rel: &str) -> bool {
    let lower = rel.to_ascii_lowercase();
    lower.starts_with("backend/")
        || lower.starts_with("server/")
        || lower.starts_with("private/")
        || lower.starts_with("db/")
        || lower.starts_with("database/")
        || lower.ends_with(".php")
        || lower.ends_with(".sql")
        || lower.ends_with(".db")
        || lower.ends_with(".sqlite")
        || lower.ends_with(".sqlite3")
        || lower == "permission.json"
        || lower == ".htaccess"
        || lower.contains("/.htaccess")
}

impl SoftnBundle {
    pub fn from_zip_bytes(bytes: &[u8]) -> Result<Self, String> {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let digest = format!("{:x}", hasher.finalize());

        let reader = Cursor::new(bytes);
        let mut archive = zip::ZipArchive::new(reader)
            .map_err(|e| format!("Invalid .softn ZIP archive: {e}"))?;

        if archive.len() > 10_000 {
            return Err("Archive exceeds maximum entry count (10000)".into());
        }

        let mut vfs = HashMap::with_capacity(archive.len());
        let mut total_bytes: usize = 0;

        for i in 0..archive.len() {
            let mut file = archive
                .by_index(i)
                .map_err(|e| format!("Failed to read archive entry #{i}: {e}"))?;
            if file.is_dir() {
                continue;
            }
            let raw_name = file
                .name()
                .map_err(|e| format!("Invalid archive entry name #{i}: {e}"))?
                .into_owned();
            let name = normalize_vfs_path(&raw_name);
            if name.is_empty() {
                continue;
            }
            let mut buf = Vec::new();
            file.read_to_end(&mut buf)
                .map_err(|e| format!("Failed to decompress '{name}': {e}"))?;
            total_bytes = total_bytes.saturating_add(buf.len());
            if total_bytes > 256 * 1024 * 1024 {
                return Err("Archive uncompressed size exceeds 256 MiB limit".into());
            }
            vfs.insert(name, buf);
        }

        let manifest: ContainerManifest = match vfs.get("manifest.json") {
            Some(raw) => serde_json::from_slice(raw)
                .map_err(|e| format!("Invalid manifest.json: {e}"))?,
            None => ContainerManifest::default(),
        };

        let permissions: PermissionFile = match vfs.get("permission.json") {
            Some(raw) => serde_json::from_slice(raw)
                .map_err(|e| format!("Invalid permission.json: {e}"))?,
            None => PermissionFile::default(),
        };

        // Discover entry HTML and frontendRoot
        let (entry_path, frontend_root) = Self::discover_entry(&manifest, &vfs)?;

        let inferred_title = vfs
            .get(&entry_path)
            .and_then(|b| extract_html_title(&String::from_utf8_lossy(b)));

        let name = manifest
            .name
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or(inferred_title)
            .unwrap_or_else(|| "SoftN App".to_string());

        let version = manifest
            .version
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "1.0.0".to_string());

        let app_id = manifest
            .id
            .clone()
            .filter(|s| !s.trim().is_empty())
            .map(|id| sanitize_app_id(&id))
            .unwrap_or_else(|| format!("sha256_{}", &digest[..16]));

        Ok(Self {
            app_id,
            digest,
            name,
            version,
            manifest,
            permissions,
            entry_path,
            frontend_root,
            vfs,
        })
    }

    fn discover_entry(
        manifest: &ContainerManifest,
        vfs: &HashMap<String, Vec<u8>>,
    ) -> Result<(String, String), String> {
        let declared_root = manifest
            .frontend_root
            .as_deref()
            .map(normalize_vfs_path)
            .unwrap_or_default();

        if let Some(main) = &manifest.main {
            let norm = normalize_vfs_path(main);
            if vfs.contains_key(&norm) {
                let root = if !declared_root.is_empty() {
                    declared_root
                } else {
                    norm.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default()
                };
                return Ok((norm, root));
            }
        }

        for candidate in ["index.html", "public/index.html", "dist/index.html"] {
            if vfs.contains_key(candidate) {
                let root = if !declared_root.is_empty() {
                    declared_root
                } else {
                    candidate.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default()
                };
                return Ok((candidate.to_string(), root));
            }
        }

        // SSR-only backend apps may not ship a static index.html
        if manifest.backend.is_some() {
            return Ok((String::new(), declared_root));
        }

        // Last resort: first .html file in archive
        let mut htmls: Vec<&String> = vfs
            .keys()
            .filter(|k| k.ends_with(".html") || k.ends_with(".htm"))
            .collect();
        htmls.sort();
        if let Some(first) = htmls.first() {
            let root = first.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
            return Ok(((*first).clone(), root));
        }

        Err("No index.html or backend entry found in .softn archive".into())
    }

    pub fn read_text(&self, path: &str) -> Option<String> {
        let norm = normalize_vfs_path(path);
        self.vfs
            .get(&norm)
            .map(|b| String::from_utf8_lossy(b).into_owned())
    }

    pub fn read_public_asset(&self, url_path: &str) -> Option<(String, &'static str, Vec<u8>)> {
        let clean = normalize_vfs_path(url_path.split('?').next().unwrap_or(url_path));
        let candidates = if !self.frontend_root.is_empty() {
            vec![
                normalize_vfs_path(&format!("{}/{clean}", self.frontend_root)),
                clean.clone(),
            ]
        } else {
            vec![clean.clone()]
        };

        for cand in candidates {
            if cand.is_empty() || is_private_asset_path(&cand) {
                continue;
            }
            if let Some(bytes) = self.vfs.get(&cand) {
                return Some((cand.clone(), mime_type_for(&cand), bytes.clone()));
            }
        }
        None
    }

    pub fn declared_capabilities(&self) -> Vec<&'static str> {
        let p = &self.permissions.permissions;
        let mut out = Vec::new();
        if p.net.as_ref().and_then(|c| c.enabled) == Some(true) {
            out.push("net");
        }
        if p.camera.as_ref().and_then(|c| c.enabled) == Some(true) {
            out.push("camera");
        }
        if p.mic.as_ref().and_then(|c| c.enabled) == Some(true) {
            out.push("mic");
        }
        if p.files.as_ref().and_then(|c| c.enabled) == Some(true) {
            out.push("files");
        }
        if p.qr.as_ref().and_then(|c| c.enabled) == Some(true) {
            out.push("qr");
        }
        if p.ai.as_ref().and_then(|c| c.enabled) == Some(true) {
            out.push("ai");
        }
        if p.gpu.as_ref().and_then(|c| c.enabled) == Some(true) {
            out.push("gpu");
        }
        if p.sync.as_ref().and_then(|c| c.enabled) == Some(true) {
            out.push("sync");
        }
        if p.storage.as_ref().and_then(|c| c.enabled) == Some(true) {
            out.push("storage");
        }
        if p.accel.as_ref().and_then(|c| c.enabled) == Some(true) {
            out.push("accel");
        }
        if p.bluetooth.as_ref().and_then(|c| c.enabled) == Some(true) {
            out.push("bluetooth");
        }
        if p.location.as_ref().and_then(|c| c.enabled) == Some(true) {
            out.push("location");
        }
        if p.haptics.as_ref().and_then(|c| c.enabled) == Some(true) {
            out.push("haptics");
        }
        if p.notifications.as_ref().and_then(|c| c.enabled) == Some(true) {
            out.push("notifications");
        }
        out
    }

    pub fn has_capability(&self, cap: &str) -> bool {
        self.declared_capabilities().contains(&cap)
    }

    pub fn allowed_ble_services(&self) -> Option<HashSet<String>> {
        let services = self
            .permissions
            .permissions
            .bluetooth
            .as_ref()?
            .services
            .as_ref()?;
        if services.is_empty() {
            return None;
        }
        Some(services.iter().map(|s| s.trim().to_ascii_lowercase()).collect())
    }

    pub fn check_net_url(&self, raw_url: &str) -> Result<(), String> {
        let Some(net) = &self.permissions.permissions.net else {
            return Err("Network access not permitted by permission.json".into());
        };
        if net.enabled != Some(true) {
            return Err("Network access not permitted by permission.json".into());
        }
        let parsed = url::Url::parse(raw_url).map_err(|e| format!("Invalid URL: {e}"))?;
        let scheme = parsed.scheme();
        if scheme != "https" && !(scheme == "http" && net.allow_http == Some(true)) {
            return Err(format!("Blocked URL scheme '{scheme}': HTTPS required"));
        }
        let Some(host) = parsed.host_str() else {
            return Err("URL missing host".into());
        };
        if let Some(allowed) = &net.allowed_hosts {
            if !allowed.is_empty() {
                let lower_host = host.to_ascii_lowercase();
                let ok = allowed.iter().any(|rule| {
                    let r = rule.trim().to_ascii_lowercase();
                    if let Some(suffix) = r.strip_prefix("*.") {
                        lower_host == suffix || lower_host.ends_with(&format!(".{suffix}"))
                    } else {
                        lower_host == r
                    }
                });
                if !ok {
                    return Err(format!("Host '{host}' is not in permission.json net.allowed_hosts"));
                }
            }
        }
        Ok(())
    }
}

fn sanitize_app_id(raw: &str) -> String {
    let out: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() {
        "softn_app".to_string()
    } else {
        out
    }
}

fn extract_html_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let after_open = html[start..].find('>')? + start + 1;
    let end = lower[after_open..].find("</title>")? + after_open;
    let title = html[after_open..end].trim();
    if title.is_empty() {
        None
    } else {
        Some(title.to_string())
    }
}
