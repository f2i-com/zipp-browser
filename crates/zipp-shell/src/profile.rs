//! Where the browser keeps what outlives it: cookies, cache, settings,
//! history, and what sites store.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

static INCOGNITO: AtomicBool = AtomicBool::new(false);

/// Turn Incognito (private browsing) mode on or off. Must be called before
/// caches, cookies, or site storage initialize.
pub fn set_incognito(enabled: bool) {
    INCOGNITO.store(enabled, Ordering::Release);
}

/// Whether this browser session is running in Incognito (ephemeral, zero-disk) mode.
pub fn is_incognito() -> bool {
    INCOGNITO.load(Ordering::Acquire)
}

/// The persistent profile folder on disk, regardless of Incognito mode.
/// Used read-only in Incognito mode solely to load user preferences such as
/// the configured start page; never written to in Incognito mode.
pub fn persistent_dir() -> Option<PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = if let Some(dir) = std::env::var_os("ZIPP_PROFILE_DIR") {
            PathBuf::from(dir)
        } else if cfg!(windows) {
            PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Zipp")
        } else if let Some(data) = std::env::var_os("XDG_DATA_HOME") {
            PathBuf::from(data).join("zipp")
        } else {
            PathBuf::from(std::env::var_os("HOME")?).join(".local").join("share").join("zipp")
        };
        std::fs::create_dir_all(&dir).ok()?;
        Some(dir)
    })
    .clone()
}

/// The active profile folder, made on first use: `ZIPP_PROFILE_DIR` if set
/// (tests keep theirs apart), else `%LOCALAPPDATA%\Zipp` on Windows and
/// `$XDG_DATA_HOME/zipp` (`~/.local/share/zipp`) elsewhere.
///
/// Always returns `None` in Incognito mode so cookies, disk HTTP cache,
/// site storage, and browsing history never read or write the user's profile.
pub fn dir() -> Option<PathBuf> {
    if is_incognito() {
        return None;
    }
    persistent_dir()
}
