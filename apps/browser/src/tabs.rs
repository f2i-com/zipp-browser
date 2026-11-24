//! Multi-tab state management for Zipp Browser.
//!
//! Tracks open tabs (each with its own URL, title, back/forward history stack,
//! loading status, and cached raw HTML for instant tab switching), recently
//! closed tabs (`Ctrl+Shift+T`), and a monotonic navigation generation counter
//! so late background fetches never clobber a tab the user has already left.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// Built-in New Tab page URL.
pub const NEW_TAB_URL: &str = "zipp:newtab";

/// Maximum recently closed tabs kept for `Ctrl+Shift+T`.
const MAX_CLOSED_TABS: usize = 16;

/// Maximum per-tab cached raw HTML size (4 MiB) to keep memory bounded.
const MAX_TAB_HTML_BYTES: usize = 4 * 1024 * 1024;

/// Back/forward navigation history for a single tab.
#[derive(Clone, Debug)]
pub struct TabHistory {
    entries: Vec<String>,
    index: usize,
}

impl TabHistory {
    pub fn new(first: &str) -> Self {
        Self {
            entries: vec![first.to_string()],
            index: 0,
        }
    }

    /// Push a new URL onto this tab's history, dropping any forward entries.
    pub fn push(&mut self, url: &str) {
        if self.entries[self.index] == url {
            return;
        }
        self.entries.truncate(self.index + 1);
        self.entries.push(url.to_string());
        self.index += 1;
    }

    pub fn back(&mut self) -> Option<String> {
        self.index = self.index.checked_sub(1)?;
        Some(self.entries[self.index].clone())
    }

    pub fn forward(&mut self) -> Option<String> {
        let next = self.index + 1;
        let url = self.entries.get(next)?.clone();
        self.index = next;
        Some(url)
    }

    pub fn can_back(&self) -> bool {
        self.index > 0
    }

    pub fn can_forward(&self) -> bool {
        self.index + 1 < self.entries.len()
    }

    /// Replace the current history entry URL in-place (used for internal page updates).
    pub fn replace_current(&mut self, url: &str) {
        if let Some(slot) = self.entries.get_mut(self.index) {
            *slot = url.to_string();
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[derive(Clone, Debug)]
struct Tab {
    id: u64,
    url: String,
    title: String,
    history: TabHistory,
    cached_html: Option<String>,
    loading: bool,
}

#[derive(Clone, Debug)]
struct ClosedTab {
    url: String,
    title: String,
    history: TabHistory,
}

/// Snapshot of a tab for rendering the browser tab strip chrome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabStripItem {
    pub id: u64,
    pub url: String,
    pub title: String,
    pub active: bool,
    pub loading: bool,
}

/// Navigation target describing which tab & URL to load into the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabTarget {
    pub id: u64,
    pub url: String,
    pub cached_html: Option<String>,
    pub nav_gen: u64,
}

/// Result of closing a tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloseResult {
    /// The closed tab was active (or was the last tab, replaced by a fresh New Tab),
    /// so the window should now display this target.
    Switched(TabTarget),
    /// A background tab was closed; the active tab stays untouched and only the
    /// tab strip needs an in-place refresh.
    ClosedBackground,
    /// No tab matched the requested id.
    NotFound,
}

pub fn is_internal_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    lower.starts_with("zipp:") || lower.starts_with("about:")
}

/// Fallback human-readable tab title derived from a URL before `<title>` is known.
pub fn default_title_for_url(url: &str) -> String {
    let base = url.split(['?', '#']).next().unwrap_or(url).trim();
    match base.to_ascii_lowercase().as_str() {
        "zipp:newtab" | "about:newtab" | "about:blank" | "" => return "New Tab".to_string(),
        "zipp:settings" | "zipp:properties" | "about:settings" | "about:properties" => {
            return "Properties".to_string();
        }
        "zipp:history" | "about:history" => return "History".to_string(),
        _ => {}
    }
    if let Ok(parsed) = url::Url::parse(url) {
        if parsed.scheme() == "file" {
            if let Ok(path) = parsed.to_file_path()
                && let Some(name) = path.file_name().and_then(|n| n.to_str())
                && !name.is_empty()
            {
                return name.to_string();
            }
            return "Local File".to_string();
        }
        if let Some(host) = parsed.host_str() {
            let clean = host.strip_prefix("www.").unwrap_or(host);
            if !clean.is_empty() {
                return clean.to_string();
            }
        }
    }
    url.chars().take(48).collect()
}

fn clean_title(raw: &str) -> String {
    let collapsed: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim();
    // Strip trailing browser suffix so tab labels stay concise.
    let stripped = trimmed
        .strip_suffix(" — Zipp Browser")
        .or_else(|| trimmed.strip_suffix(" - Zipp Browser"))
        .unwrap_or(trimmed)
        .trim();
    stripped.to_string()
}

struct TabManager {
    tabs: Vec<Tab>,
    active_index: usize,
    next_id: u64,
    nav_gen: u64,
    closed: Vec<ClosedTab>,
    last_web_url: Option<String>,
}

impl TabManager {
    fn new() -> Self {
        Self {
            tabs: Vec::new(),
            active_index: 0,
            next_id: 1,
            nav_gen: 1,
            closed: Vec::new(),
            last_web_url: None,
        }
    }

    fn init(&mut self, initial_url: &str) -> u64 {
        self.tabs.clear();
        self.closed.clear();
        self.active_index = 0;
        self.next_id = 1;
        self.nav_gen = self.nav_gen.wrapping_add(1).max(1);
        let id = self.alloc_id();
        self.record_last_web(initial_url);
        self.tabs.push(Tab {
            id,
            url: initial_url.to_string(),
            title: default_title_for_url(initial_url),
            history: TabHistory::new(initial_url),
            cached_html: None,
            loading: false,
        });
        id
    }

    fn ensure_initialized(&mut self, fallback_url: &str) {
        if self.tabs.is_empty() {
            self.init(fallback_url);
        }
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn next_gen(&mut self) -> u64 {
        self.nav_gen = self.nav_gen.wrapping_add(1).max(1);
        self.nav_gen
    }

    fn record_last_web(&mut self, url: &str) {
        if !url.is_empty() && !is_internal_url(url) {
            self.last_web_url = Some(url.to_string());
        }
    }

    fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active_index)
    }

    fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(self.active_index)
    }

    fn target_for_active(&mut self, use_cache: bool) -> TabTarget {
        let nav_gen = self.next_gen();
        let tab = &self.tabs[self.active_index];
        TabTarget {
            id: tab.id,
            url: tab.url.clone(),
            cached_html: if use_cache { tab.cached_html.clone() } else { None },
            nav_gen,
        }
    }

    fn open_tab(&mut self, url: Option<String>, activate: bool, adjacent: bool) -> (u64, Option<TabTarget>) {
        self.ensure_initialized(NEW_TAB_URL);
        let dest = url.unwrap_or_else(|| NEW_TAB_URL.to_string());
        let id = self.alloc_id();
        let tab = Tab {
            id,
            url: dest.clone(),
            title: default_title_for_url(&dest),
            history: TabHistory::new(&dest),
            cached_html: None,
            loading: !is_internal_url(&dest),
        };
        let insert_at = if adjacent {
            (self.active_index + 1).min(self.tabs.len())
        } else {
            self.tabs.len()
        };
        self.tabs.insert(insert_at, tab);
        if activate {
            self.active_index = insert_at;
            self.record_last_web(&dest);
            let target = self.target_for_active(false);
            (id, Some(target))
        } else {
            (id, None)
        }
    }

    fn switch_to_index(&mut self, index: usize) -> Option<TabTarget> {
        if index >= self.tabs.len() || index == self.active_index {
            return None;
        }
        self.active_index = index;
        let url = self.tabs[self.active_index].url.clone();
        self.record_last_web(&url);
        Some(self.target_for_active(true))
    }

    fn switch_to_id(&mut self, id: u64) -> Option<TabTarget> {
        let idx = self.tabs.iter().position(|t| t.id == id)?;
        self.switch_to_index(idx)
    }

    fn switch_by_offset(&mut self, delta: isize) -> Option<TabTarget> {
        let len = self.tabs.len();
        if len <= 1 {
            return None;
        }
        let next = (self.active_index as isize + delta).rem_euclid(len as isize) as usize;
        self.switch_to_index(next)
    }

    fn switch_to_slot(&mut self, slot_1_based: usize) -> Option<TabTarget> {
        if self.tabs.is_empty() || slot_1_based == 0 {
            return None;
        }
        let idx = if slot_1_based >= 9 {
            self.tabs.len() - 1
        } else {
            (slot_1_based - 1).min(self.tabs.len() - 1)
        };
        self.switch_to_index(idx)
    }

    fn close_tab(&mut self, id: Option<u64>) -> CloseResult {
        if self.tabs.is_empty() {
            return CloseResult::NotFound;
        }
        let idx = match id {
            Some(target_id) => match self.tabs.iter().position(|t| t.id == target_id) {
                Some(i) => i,
                None => return CloseResult::NotFound,
            },
            None => self.active_index,
        };
        let removed = self.tabs.remove(idx);
        if removed.url != NEW_TAB_URL || removed.history.len() > 1 {
            if self.closed.len() >= MAX_CLOSED_TABS {
                self.closed.remove(0);
            }
            self.closed.push(ClosedTab {
                url: removed.url,
                title: removed.title,
                history: removed.history,
            });
        }
        if self.tabs.is_empty() {
            let new_id = self.alloc_id();
            self.tabs.push(Tab {
                id: new_id,
                url: NEW_TAB_URL.to_string(),
                title: default_title_for_url(NEW_TAB_URL),
                history: TabHistory::new(NEW_TAB_URL),
                cached_html: None,
                loading: false,
            });
            self.active_index = 0;
            return CloseResult::Switched(self.target_for_active(false));
        }
        if idx == self.active_index {
            if self.active_index >= self.tabs.len() {
                self.active_index = self.tabs.len() - 1;
            }
            let url = self.tabs[self.active_index].url.clone();
            self.record_last_web(&url);
            CloseResult::Switched(self.target_for_active(true))
        } else {
            if idx < self.active_index {
                self.active_index -= 1;
            }
            CloseResult::ClosedBackground
        }
    }

    fn reopen_closed_tab(&mut self) -> Option<TabTarget> {
        let closed = self.closed.pop()?;
        let id = self.alloc_id();
        let insert_at = if self.tabs.is_empty() {
            0
        } else {
            (self.active_index + 1).min(self.tabs.len())
        };
        let url = closed.url.clone();
        self.tabs.insert(
            insert_at,
            Tab {
                id,
                url: closed.url,
                title: closed.title,
                history: closed.history,
                cached_html: None,
                loading: !is_internal_url(&url),
            },
        );
        self.active_index = insert_at;
        self.record_last_web(&url);
        Some(self.target_for_active(false))
    }

    fn navigate_active(&mut self, url: String) -> TabTarget {
        self.ensure_initialized(&url);
        self.record_last_web(&url);
        if let Some(tab) = self.active_tab_mut() {
            tab.history.push(&url);
            tab.url = url.clone();
            tab.title = default_title_for_url(&url);
            tab.cached_html = None;
            tab.loading = !is_internal_url(&url);
        }
        self.target_for_active(false)
    }

    fn active_back(&mut self) -> Option<TabTarget> {
        let dest = self.active_tab_mut()?.history.back()?;
        self.record_last_web(&dest);
        if let Some(tab) = self.active_tab_mut() {
            tab.url = dest.clone();
            tab.title = default_title_for_url(&dest);
            tab.cached_html = None;
            tab.loading = !is_internal_url(&dest);
        }
        Some(self.target_for_active(false))
    }

    fn active_forward(&mut self) -> Option<TabTarget> {
        let dest = self.active_tab_mut()?.history.forward()?;
        self.record_last_web(&dest);
        if let Some(tab) = self.active_tab_mut() {
            tab.url = dest.clone();
            tab.title = default_title_for_url(&dest);
            tab.cached_html = None;
            tab.loading = !is_internal_url(&dest);
        }
        Some(self.target_for_active(false))
    }

    fn active_reload(&mut self) -> TabTarget {
        self.ensure_initialized(NEW_TAB_URL);
        if let Some(tab) = self.active_tab_mut() {
            tab.cached_html = None;
            tab.loading = !is_internal_url(&tab.url);
        }
        self.target_for_active(false)
    }

    fn set_active_internal(&mut self, url: &str, title: &str) -> u64 {
        self.ensure_initialized(url);
        let nav_gen = self.next_gen();
        if let Some(tab) = self.active_tab_mut() {
            tab.url = url.to_string();
            tab.history.replace_current(url);
            tab.title = clean_title(title);
            tab.cached_html = None;
            tab.loading = false;
        }
        nav_gen
    }

    fn update_title_by_id(&mut self, id: u64, title: &str) -> bool {
        let cleaned = clean_title(title);
        if cleaned.is_empty() {
            return false;
        }
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else {
            return false;
        };
        if tab.title == cleaned {
            return false;
        }
        tab.title = cleaned;
        true
    }

    fn sync_active_spa_url(&mut self, url: &str) -> bool {
        if url.is_empty() || is_internal_url(url) {
            return false;
        }
        let changed = if let Some(tab) = self.active_tab_mut() {
            if tab.url == url {
                false
            } else {
                tab.url = url.to_string();
                tab.history.replace_current(url);
                true
            }
        } else {
            false
        };
        if changed {
            self.record_last_web(url);
        }
        changed
    }

    fn update_active_title(&mut self, title: &str) -> bool {
        let cleaned = clean_title(title);
        if cleaned.is_empty() {
            return false;
        }
        let Some(tab) = self.active_tab_mut() else {
            return false;
        };
        let changed = tab.title != cleaned || tab.loading;
        tab.title = cleaned;
        tab.loading = false;
        changed
    }

    fn store_tab_html(&mut self, id: u64, url: &str, html: String, title: Option<&str>) -> bool {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else {
            return false;
        };
        if tab.url != url {
            return false;
        }
        let mut changed = tab.loading;
        tab.loading = false;
        if html.len() <= MAX_TAB_HTML_BYTES {
            tab.cached_html = Some(html);
        } else {
            tab.cached_html = None;
        }
        if let Some(t) = title {
            let cleaned = clean_title(t);
            if !cleaned.is_empty() && tab.title != cleaned {
                tab.title = cleaned;
                changed = true;
            }
        }
        changed
    }

    fn clear_cached_html(&mut self) {
        for tab in &mut self.tabs {
            tab.cached_html = None;
        }
    }

    fn is_active_nav_gen(&self, id: u64, nav_gen: u64) -> bool {
        self.nav_gen == nav_gen && self.active_tab().is_some_and(|t| t.id == id)
    }

    fn chrome_items(&self) -> Vec<TabStripItem> {
        self.tabs
            .iter()
            .enumerate()
            .map(|(i, t)| TabStripItem {
                id: t.id,
                url: t.url.clone(),
                title: t.title.clone(),
                active: i == self.active_index,
                loading: t.loading,
            })
            .collect()
    }
}

static MANAGER: std::sync::LazyLock<Mutex<TabManager>> =
    std::sync::LazyLock::new(|| Mutex::new(TabManager::new()));
static GENERATION: AtomicU64 = AtomicU64::new(1);

fn bump_generation() {
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

pub fn init(initial_url: &str) -> u64 {
    let mut mgr = MANAGER.lock().unwrap_or_else(|e| e.into_inner());
    let id = mgr.init(initial_url);
    bump_generation();
    id
}

pub fn ensure_initialized(fallback_url: &str) {
    let mut mgr = MANAGER.lock().unwrap_or_else(|e| e.into_inner());
    if mgr.tabs.is_empty() {
        mgr.init(fallback_url);
        bump_generation();
    }
}

pub fn active_url() -> String {
    let mgr = MANAGER.lock().unwrap_or_else(|e| e.into_inner());
    mgr.active_tab().map(|t| t.url.clone()).unwrap_or_default()
}

pub fn active_title() -> String {
    let mgr = MANAGER.lock().unwrap_or_else(|e| e.into_inner());
    mgr.active_tab()
        .map(|t| t.title.clone())
        .unwrap_or_else(|| "Zipp Browser".to_string())
}

pub fn active_nav_state() -> (bool, bool) {
    let mgr = MANAGER.lock().unwrap_or_else(|e| e.into_inner());
    mgr.active_tab()
        .map(|t| (t.history.can_back(), t.history.can_forward()))
        .unwrap_or((false, false))
}

pub fn last_web_url() -> Option<String> {
    let mgr = MANAGER.lock().unwrap_or_else(|e| e.into_inner());
    mgr.last_web_url.clone()
}

pub fn open_new_tab(url: Option<String>, activate: bool, adjacent: bool) -> (u64, Option<TabTarget>) {
    let mut mgr = MANAGER.lock().unwrap_or_else(|e| e.into_inner());
    let res = mgr.open_tab(url, activate, adjacent);
    bump_generation();
    res
}

pub fn switch_to_id(id: u64) -> Option<TabTarget> {
    let mut mgr = MANAGER.lock().unwrap_or_else(|e| e.into_inner());
    let res = mgr.switch_to_id(id);
    if res.is_some() {
        bump_generation();
    }
    res
}

pub fn switch_by_offset(delta: isize) -> Option<TabTarget> {
    let mut mgr = MANAGER.lock().unwrap_or_else(|e| e.into_inner());
    let res = mgr.switch_by_offset(delta);
    if res.is_some() {
        bump_generation();
    }
    res
}

pub fn switch_to_slot(slot_1_based: usize) -> Option<TabTarget> {
    let mut mgr = MANAGER.lock().unwrap_or_else(|e| e.into_inner());
    let res = mgr.switch_to_slot(slot_1_based);
    if res.is_some() {
        bump_generation();
    }
    res
}

pub fn close_tab(id: Option<u64>) -> CloseResult {
    let mut mgr = MANAGER.lock().unwrap_or_else(|e| e.into_inner());
    let res = mgr.close_tab(id);
    if !matches!(res, CloseResult::NotFound) {
        bump_generation();
    }
    res
}

pub fn reopen_closed_tab() -> Option<TabTarget> {
    let mut mgr = MANAGER.lock().unwrap_or_else(|e| e.into_inner());
    let res = mgr.reopen_closed_tab();
    if res.is_some() {
        bump_generation();
    }
    res
}

pub fn navigate_active(url: String) -> TabTarget {
    let mut mgr = MANAGER.lock().unwrap_or_else(|e| e.into_inner());
    let res = mgr.navigate_active(url);
    bump_generation();
    res
}

pub fn active_back() -> Option<TabTarget> {
    let mut mgr = MANAGER.lock().unwrap_or_else(|e| e.into_inner());
    let res = mgr.active_back();
    if res.is_some() {
        bump_generation();
    }
    res
}

pub fn active_forward() -> Option<TabTarget> {
    let mut mgr = MANAGER.lock().unwrap_or_else(|e| e.into_inner());
    let res = mgr.active_forward();
    if res.is_some() {
        bump_generation();
    }
    res
}

pub fn active_reload() -> TabTarget {
    let mut mgr = MANAGER.lock().unwrap_or_else(|e| e.into_inner());
    let res = mgr.active_reload();
    bump_generation();
    res
}

pub fn set_active_internal(url: &str, title: &str) {
    let mut mgr = MANAGER.lock().unwrap_or_else(|e| e.into_inner());
    mgr.set_active_internal(url, title);
    bump_generation();
}

pub fn sync_active_spa_url(url: &str) {
    let mut mgr = MANAGER.lock().unwrap_or_else(|e| e.into_inner());
    if mgr.sync_active_spa_url(url) {
        bump_generation();
    }
}

pub fn update_active_title(title: &str) {
    let mut mgr = MANAGER.lock().unwrap_or_else(|e| e.into_inner());
    if mgr.update_active_title(title) {
        bump_generation();
    }
}

#[allow(dead_code)]
pub fn update_tab_title(id: u64, title: &str) {
    let mut mgr = MANAGER.lock().unwrap_or_else(|e| e.into_inner());
    if mgr.update_title_by_id(id, title) {
        bump_generation();
    }
}

pub fn store_tab_html(id: u64, url: &str, html: String, title: Option<&str>) {
    let mut mgr = MANAGER.lock().unwrap_or_else(|e| e.into_inner());
    if mgr.store_tab_html(id, url, html, title) {
        bump_generation();
    }
}

pub fn clear_cached_html() {
    let mut mgr = MANAGER.lock().unwrap_or_else(|e| e.into_inner());
    mgr.clear_cached_html();
}

pub fn is_active_nav_gen(id: u64, nav_gen: u64) -> bool {
    let mgr = MANAGER.lock().unwrap_or_else(|e| e.into_inner());
    mgr.is_active_nav_gen(id, nav_gen)
}

pub fn chrome_items(fallback_url: &str) -> Vec<TabStripItem> {
    let mut mgr = MANAGER.lock().unwrap_or_else(|e| e.into_inner());
    mgr.ensure_initialized(fallback_url);
    mgr.chrome_items()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_lifecycle_switching_closing_and_reopening() {
        let mut mgr = TabManager::new();
        let t1 = mgr.init("https://example.com");
        assert_eq!(mgr.chrome_items().len(), 1);
        assert_eq!(mgr.chrome_items()[0].title, "example.com");
        assert!(mgr.chrome_items()[0].active);

        // Open a link in a background adjacent tab.
        let (t2, target2) = mgr.open_tab(Some("https://softn.com".into()), false, true);
        assert!(target2.is_none());
        assert_eq!(mgr.active_tab().unwrap().id, t1);
        assert_eq!(mgr.chrome_items().len(), 2);
        assert!(mgr.chrome_items()[1].loading);

        // Background fetch finishes and populates HTML + title without activating Tab 2.
        mgr.store_tab_html(t2, "https://softn.com", "<h1>SoftN</h1>".into(), Some("SoftN Home"));
        assert_eq!(mgr.chrome_items()[1].title, "SoftN Home");
        assert!(!mgr.chrome_items()[1].loading);

        // Open a fresh New Tab at the end (active).
        let (t3, target3) = mgr.open_tab(None, true, false);
        assert_eq!(target3.as_ref().unwrap().url, NEW_TAB_URL);
        assert_eq!(mgr.active_tab().unwrap().id, t3);
        assert_eq!(mgr.chrome_items().len(), 3);

        // Ctrl+Tab wraps from last tab (index 2) back to index 0 (t1).
        let sw = mgr.switch_by_offset(1).unwrap();
        assert_eq!(sw.id, t1);

        // Ctrl+Shift+Tab wraps from index 0 back to last tab (t3).
        let sw = mgr.switch_by_offset(-1).unwrap();
        assert_eq!(sw.id, t3);

        // Ctrl+2 jumps to t2 with cached HTML intact!
        let sw = mgr.switch_to_slot(2).unwrap();
        assert_eq!(sw.id, t2);
        assert_eq!(sw.cached_html.as_deref(), Some("<h1>SoftN</h1>"));

        // Ctrl+9 jumps to last tab (t3).
        let sw = mgr.switch_to_slot(9).unwrap();
        assert_eq!(sw.id, t3);

        // Closing background tab t2 does NOT disturb active tab t3.
        assert_eq!(mgr.close_tab(Some(t2)), CloseResult::ClosedBackground);
        assert_eq!(mgr.active_tab().unwrap().id, t3);
        assert_eq!(mgr.chrome_items().len(), 2);

        // Reopening closed tab brings SoftN Home back right after active tab and activates it.
        let reopened = mgr.reopen_closed_tab().unwrap();
        assert_eq!(reopened.url, "https://softn.com");
        assert_eq!(mgr.chrome_items().len(), 3);
    }

    #[test]
    fn per_tab_history_and_last_tab_reset() {
        let mut mgr = TabManager::new();
        let t1 = mgr.init("https://a.example");
        mgr.navigate_active("https://b.example".into());
        assert!(mgr.active_tab().unwrap().history.can_back());

        // Open tab 2 and navigate independently.
        let (t2, _) = mgr.open_tab(Some("https://c.example".into()), true, false);
        assert!(!mgr.active_tab().unwrap().history.can_back());
        mgr.navigate_active("https://d.example".into());
        assert_eq!(mgr.active_back().unwrap().url, "https://c.example");
        assert_eq!(mgr.active_forward().unwrap().url, "https://d.example");

        // Tab 1 still has its own back stack (b -> a).
        mgr.switch_to_id(t1).unwrap();
        assert_eq!(mgr.active_back().unwrap().url, "https://a.example");

        // Closing both tabs resets to a clean New Tab rather than leaving 0 tabs.
        assert_eq!(mgr.close_tab(Some(t2)), CloseResult::ClosedBackground);
        match mgr.close_tab(None) {
            CloseResult::Switched(target) => {
                assert_eq!(target.url, NEW_TAB_URL);
            }
            other => panic!("expected Switched to newtab, got {other:?}"),
        }
        assert_eq!(mgr.chrome_items().len(), 1);
    }

    #[test]
    fn stale_nav_gen_guard_rejects_old_tab_loads() {
        let mut mgr = TabManager::new();
        let _t1 = mgr.init("https://slow.example");
        let target1 = mgr.navigate_active("https://slow.example/page".into());
        assert!(mgr.is_active_nav_gen(target1.id, target1.nav_gen));

        // User switches to a new tab while Tab 1 is still fetching.
        let (_t2, target2) = mgr.open_tab(Some("https://fast.example".into()), true, false);
        let target2 = target2.unwrap();
        assert!(!mgr.is_active_nav_gen(target1.id, target1.nav_gen));
        assert!(mgr.is_active_nav_gen(target2.id, target2.nav_gen));
    }
}
