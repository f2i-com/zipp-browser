//! Persistent desktop browser settings (`<profile>/settings.json`).
//!
//! Stores user preferences for Start Page, HTTP cache enablement & disk limit,
//! and default media mute state. In Incognito mode, settings are read-only from
//! the normal profile (so the user's configured start page still opens) and
//! changes apply in memory for the Incognito session without writing to disk.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

pub const DEFAULT_START_PAGE: &str = "zipp:newtab";
pub const DEFAULT_CACHE_MAX_DISK_MB: u64 = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserSettings {
    pub start_page: String,
    pub cache_enabled: bool,
    pub cache_max_disk_mb: u64,
    pub mute_media: bool,
}

impl Default for BrowserSettings {
    fn default() -> Self {
        Self {
            start_page: DEFAULT_START_PAGE.to_string(),
            cache_enabled: true,
            cache_max_disk_mb: DEFAULT_CACHE_MAX_DISK_MB,
            mute_media: false,
        }
    }
}

static STATE: OnceLock<Mutex<BrowserSettings>> = OnceLock::new();

fn read_path() -> Option<PathBuf> {
    zipp_shell::profile::persistent_dir().map(|d| d.join("settings.json"))
}

fn write_path() -> Option<PathBuf> {
    if zipp_shell::profile::is_incognito() {
        return None;
    }
    zipp_shell::profile::dir().map(|d| d.join("settings.json"))
}

fn parse_settings(raw: &str) -> BrowserSettings {
    let mut s = BrowserSettings::default();
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(raw) else {
        return s;
    };
    if let Some(sp) = map.get("start_page").and_then(|v| v.as_str()) {
        let trimmed = sp.trim();
        if !trimmed.is_empty() {
            s.start_page = trimmed.to_string();
        }
    }
    if let Some(ce) = map.get("cache_enabled").and_then(|v| v.as_bool()) {
        s.cache_enabled = ce;
    }
    if let Some(mb) = map.get("cache_max_disk_mb").and_then(|v| v.as_u64()) {
        s.cache_max_disk_mb = mb.clamp(16, 2048);
    }
    if let Some(mm) = map.get("mute_media").and_then(|v| v.as_bool()) {
        s.mute_media = mm;
    }
    s
}

fn serialize_settings(s: &BrowserSettings) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "start_page": s.start_page,
        "cache_enabled": s.cache_enabled,
        "cache_max_disk_mb": s.cache_max_disk_mb,
        "mute_media": s.mute_media,
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

fn state() -> &'static Mutex<BrowserSettings> {
    STATE.get_or_init(|| {
        let loaded = read_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|raw| parse_settings(&raw))
            .unwrap_or_default();
        Mutex::new(loaded)
    })
}

/// Current snapshot of browser settings.
pub fn get() -> BrowserSettings {
    state().lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Configured start page URL.
pub fn start_page() -> String {
    get().start_page
}

/// Whether media/audio is muted in settings (or via `ZIPP_MUTE_MEDIA`).
#[allow(dead_code)]
pub fn is_media_muted() -> bool {
    std::env::var_os("ZIPP_MUTE_MEDIA").is_some() || get().mute_media
}

/// Apply current settings to runtime subsystems (HTTP cache & audio mute).
pub fn apply_runtime(force_no_cache: bool) {
    let s = get();
    zipp_shell::cache::set_disabled(force_no_cache || !s.cache_enabled);
    zipp_shell::cache::set_max_disk_bytes(s.cache_max_disk_mb * 1024 * 1024);
    crate::audio::set_muted(s.mute_media);
}

/// Mutate settings in place, persist to disk (unless Incognito), and apply at runtime.
pub fn update(f: impl FnOnce(&mut BrowserSettings)) -> BrowserSettings {
    let updated = {
        let mut guard = state().lock().unwrap_or_else(|e| e.into_inner());
        f(&mut guard);
        guard.cache_max_disk_mb = guard.cache_max_disk_mb.clamp(16, 2048);
        if guard.start_page.trim().is_empty() {
            guard.start_page = DEFAULT_START_PAGE.to_string();
        }
        guard.clone()
    };
    save_to_disk(&updated);
    zipp_shell::cache::set_disabled(!updated.cache_enabled);
    zipp_shell::cache::set_max_disk_bytes(updated.cache_max_disk_mb * 1024 * 1024);
    crate::audio::set_muted(updated.mute_media);
    updated
}

/// Persist current settings if running in normal (non-Incognito) mode.
pub fn save() {
    let snapshot = get();
    save_to_disk(&snapshot);
}

fn save_to_disk(s: &BrowserSettings) {
    let Some(path) = write_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = path.with_extension("json.tmp");
    let raw = serialize_settings(s);
    if std::fs::write(&tmp, raw).is_ok() && std::fs::rename(&tmp, &path).is_err() {
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::rename(&tmp, &path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_settings_json() {
        let orig = BrowserSettings {
            start_page: "https://softn.com".to_string(),
            cache_enabled: false,
            cache_max_disk_mb: 256,
            mute_media: true,
        };
        let encoded = serialize_settings(&orig);
        let decoded = parse_settings(&encoded);
        assert_eq!(decoded, orig);
    }

    #[test]
    fn clamps_invalid_values() {
        let parsed = parse_settings(r#"{"start_page":"   ","cache_max_disk_mb":999999}"#);
        assert_eq!(parsed.start_page, DEFAULT_START_PAGE);
        assert_eq!(parsed.cache_max_disk_mb, 2048);
    }
}
