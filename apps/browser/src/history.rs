//! Persistent browsing history (`<profile>/history.json`).
//!
//! Records visited web & local pages with title, timestamp, and visit count so
//! the user can view, search, jump back to, or clear browsing history from the
//! Properties panel (`zipp:settings` / `zipp:history`).
//!
//! In Incognito mode, history is completely disabled: nothing is loaded from
//! disk, nothing is recorded in memory, and nothing is written to disk.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_HISTORY_ENTRIES: usize = 1000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryEntry {
    pub id: u64,
    pub url: String,
    pub title: String,
    pub visited_at: u64,
    pub visits: u32,
}

struct HistoryStore {
    next_id: u64,
    entries: Vec<HistoryEntry>,
}

static STORE: OnceLock<Mutex<HistoryStore>> = OnceLock::new();

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn history_path() -> Option<PathBuf> {
    if zipp_shell::profile::is_incognito() {
        return None;
    }
    zipp_shell::profile::dir().map(|d| d.join("history.json"))
}

fn is_recordable_url(url: &str) -> bool {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return false;
    }
    !(trimmed.starts_with("zipp:")
        || trimmed.starts_with("zipp-nav:")
        || trimmed.starts_with("zipp-settings:")
        || trimmed.starts_with("zipp-console:")
        || trimmed.starts_with("zipp-view:")
        || trimmed.starts_with("zipp-save:")
        || trimmed.starts_with("about:")
        || trimmed.starts_with("data:")
        || trimmed.starts_with("blob:"))
}

fn parse_entries(raw: &str) -> Vec<HistoryEntry> {
    let Ok(serde_json::Value::Array(arr)) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(arr.len());
    for (idx, item) in arr.into_iter().enumerate() {
        let Some(obj) = item.as_object() else { continue };
        let Some(url) = obj.get("url").and_then(|v| v.as_str()).filter(|u| is_recordable_url(u)) else {
            continue;
        };
        let id = obj.get("id").and_then(|v| v.as_u64()).unwrap_or(idx as u64 + 1);
        let title = obj.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let visited_at = obj.get("visited_at").and_then(|v| v.as_u64()).unwrap_or(0);
        let visits = obj.get("visits").and_then(|v| v.as_u64()).unwrap_or(1).max(1) as u32;
        out.push(HistoryEntry { id, url: url.to_string(), title, visited_at, visits });
    }
    out.sort_by(|a, b| b.visited_at.cmp(&a.visited_at).then_with(|| b.id.cmp(&a.id)));
    out.truncate(MAX_HISTORY_ENTRIES);
    out
}

fn serialize_entries(entries: &[HistoryEntry]) -> String {
    let arr: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            serde_json::json!({
                "id": e.id,
                "url": e.url,
                "title": e.title,
                "visited_at": e.visited_at,
                "visits": e.visits,
            })
        })
        .collect();
    serde_json::to_string(&arr).unwrap_or_else(|_| "[]".to_string())
}

fn store() -> &'static Mutex<HistoryStore> {
    STORE.get_or_init(|| {
        let entries = history_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|raw| parse_entries(&raw))
            .unwrap_or_default();
        let next_id = entries.iter().map(|e| e.id).max().unwrap_or(0) + 1;
        Mutex::new(HistoryStore { next_id, entries })
    })
}

/// Record a visit to `url` with optional page `title`.
/// Strictly a no-op in Incognito mode or for internal browser URLs.
pub fn record(url: &str, title: Option<&str>) {
    if zipp_shell::profile::is_incognito() || !is_recordable_url(url) {
        return;
    }
    let clean_title = title.map(str::trim).unwrap_or("");
    let now = unix_now();
    let snapshot = {
        let mut guard = store().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(pos) = guard.entries.iter().position(|e| e.url == url) {
            let mut existing = guard.entries.remove(pos);
            existing.visited_at = now;
            existing.visits = existing.visits.saturating_add(1);
            if !clean_title.is_empty() {
                existing.title = clean_title.to_string();
            }
            guard.entries.insert(0, existing);
        } else {
            let id = guard.next_id;
            guard.next_id += 1;
            guard.entries.insert(
                0,
                HistoryEntry {
                    id,
                    url: url.to_string(),
                    title: clean_title.to_string(),
                    visited_at: now,
                    visits: 1,
                },
            );
            guard.entries.truncate(MAX_HISTORY_ENTRIES);
        }
        guard.entries.clone()
    };
    write_disk(&snapshot);
}

/// Extract `<title>...</title>` text from an HTML page for history labels.
pub fn extract_html_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower
        .match_indices("<title")
        .map(|(i, _)| i)
        .find(|&i| matches!(lower.as_bytes().get(i + 6), Some(b'>' | b' ' | b'\t' | b'\r' | b'\n')))?;
    let open_end = html[start..].find('>')? + start + 1;
    let close = lower[open_end..].find("</title>")? + open_end;
    let raw = html[open_end..close].trim();
    if raw.is_empty() {
        return None;
    }
    let decoded = raw
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'");
    let collapsed: String = decoded.split_whitespace().collect::<Vec<_>>().join(" ");
    (!collapsed.is_empty()).then_some(collapsed)
}

/// Return matching history entries (most recent first). Always empty in Incognito.
pub fn list(filter: Option<&str>) -> Vec<HistoryEntry> {
    if zipp_shell::profile::is_incognito() {
        return Vec::new();
    }
    let guard = store().lock().unwrap_or_else(|e| e.into_inner());
    let q = filter.map(|s| s.trim().to_ascii_lowercase()).unwrap_or_default();
    if q.is_empty() {
        return guard.entries.clone();
    }
    guard
        .entries
        .iter()
        .filter(|e| e.url.to_ascii_lowercase().contains(&q) || e.title.to_ascii_lowercase().contains(&q))
        .cloned()
        .collect()
}

/// Total recorded history entries (always 0 in Incognito).
pub fn count() -> usize {
    if zipp_shell::profile::is_incognito() {
        return 0;
    }
    store().lock().unwrap_or_else(|e| e.into_inner()).entries.len()
}

/// Delete one history entry by ID. Returns true if removed.
pub fn delete(id: u64) -> bool {
    if zipp_shell::profile::is_incognito() {
        return false;
    }
    let (removed, snapshot) = {
        let mut guard = store().lock().unwrap_or_else(|e| e.into_inner());
        let before = guard.entries.len();
        guard.entries.retain(|e| e.id != id);
        (guard.entries.len() < before, guard.entries.clone())
    };
    if removed {
        write_disk(&snapshot);
    }
    removed
}

/// Clear all browsing history. Returns the number of entries cleared.
pub fn clear() -> usize {
    if zipp_shell::profile::is_incognito() {
        return 0;
    }
    let removed = {
        let mut guard = store().lock().unwrap_or_else(|e| e.into_inner());
        let n = guard.entries.len();
        guard.entries.clear();
        n
    };
    write_disk(&[]);
    removed
}

/// Flush history to disk (no-op in Incognito mode).
pub fn save() {
    if zipp_shell::profile::is_incognito() {
        return;
    }
    let snapshot = store().lock().unwrap_or_else(|e| e.into_inner()).entries.clone();
    write_disk(&snapshot);
}

fn write_disk(entries: &[HistoryEntry]) {
    let Some(path) = history_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = path.with_extension("json.tmp");
    let raw = serialize_entries(entries);
    if std::fs::write(&tmp, raw).is_ok() && std::fs::rename(&tmp, &path).is_err() {
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// Format a Unix timestamp relative to `now` for clean display in the Properties panel.
pub fn format_relative_time(visited_at: u64) -> String {
    let now = unix_now();
    let delta = now.saturating_sub(visited_at);
    if visited_at == 0 {
        return "earlier".to_string();
    }
    if delta < 60 {
        "just now".to_string()
    } else if delta < 3600 {
        format!("{}m ago", delta / 60)
    } else if delta < 86_400 {
        format!("{}h ago", delta / 3600)
    } else {
        format!("{}d ago", delta / 86_400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_filter_internal_urls() {
        assert!(is_recordable_url("https://softn.com/play/showcase"));
        assert!(is_recordable_url("file:///C:/demo/index.html"));
        assert!(!is_recordable_url("zipp:settings"));
        assert!(!is_recordable_url("zipp-nav:back"));
        assert!(!is_recordable_url("about:blank"));
        assert!(!is_recordable_url("data:text/html,hi"));

        let sample = vec![HistoryEntry {
            id: 7,
            url: "https://example.com".into(),
            title: "Example Domain".into(),
            visited_at: 1_700_000_000,
            visits: 3,
        }];
        let json = serialize_entries(&sample);
        let parsed = parse_entries(&json);
        assert_eq!(parsed, sample);
    }

    #[test]
    fn extracts_html_title() {
        assert_eq!(
            extract_html_title("<html><head><title> SoftN &amp; Zipp </title></head></html>").as_deref(),
            Some("SoftN & Zipp")
        );
    }
}
