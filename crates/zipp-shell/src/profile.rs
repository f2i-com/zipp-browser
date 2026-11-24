//! Where the browser keeps what outlives it: cookies, cache, settings,
//! history, and what sites store.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

static INCOGNITO: AtomicBool = AtomicBool::new(false);
static PORTABLE: AtomicBool = AtomicBool::new(false);

/// Turn Incognito (private browsing) mode on or off. Must be called before
/// caches, cookies, or site storage initialize.
pub fn set_incognito(enabled: bool) {
    INCOGNITO.store(enabled, Ordering::Release);
}

/// Whether this browser session is running in Incognito (ephemeral, zero-disk) mode.
pub fn is_incognito() -> bool {
    INCOGNITO.load(Ordering::Acquire)
}

/// Enable portable mode explicitly (`--portable`): stores all profile data in
/// `<exe_dir>/data/` next to the executable instead of `%LOCALAPPDATA%\Zipp`.
pub fn set_portable(enabled: bool) {
    PORTABLE.store(enabled, Ordering::Release);
}

fn portable_dir_for_exe(exe_path: &Path, force_portable: bool) -> Option<PathBuf> {
    let exe_dir = exe_path.parent()?;
    let data_dir = exe_dir.join("data");
    if force_portable
        || exe_dir.join("portable.txt").exists()
        || exe_dir.join(".portable").exists()
        || data_dir.is_dir()
    {
        Some(data_dir)
    } else {
        None
    }
}

/// Whether the browser is using a portable profile folder next to its executable.
pub fn is_portable() -> bool {
    if PORTABLE.load(Ordering::Acquire) {
        return true;
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| portable_dir_for_exe(&exe, false))
        .is_some()
}

/// The persistent profile folder on disk, regardless of Incognito mode.
/// Used read-only in Incognito mode solely to load user preferences such as
/// the configured start page; never written to in Incognito mode.
///
/// Resolution order:
/// 1. `ZIPP_PROFILE_DIR` environment variable (used by tests and custom scripts)
/// 2. `<exe_dir>/data` when `--portable` is passed or `portable.txt` / `.portable` / `data/` sits beside the binary
/// 3. `%LOCALAPPDATA%\Zipp` on Windows, `$XDG_DATA_HOME/zipp` or `~/.local/share/zipp` elsewhere
pub fn persistent_dir() -> Option<PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = if let Some(dir) = std::env::var_os("ZIPP_PROFILE_DIR") {
            PathBuf::from(dir)
        } else if let Some(portable_dir) = std::env::current_exe()
            .ok()
            .and_then(|exe| portable_dir_for_exe(&exe, PORTABLE.load(Ordering::Acquire)))
        {
            portable_dir
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_mode_detects_marker_or_explicit_flag() {
        let base = std::env::temp_dir().join(format!(
            "zipp-portable-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let fake_exe = base.join("zipp-browser.exe");

        assert_eq!(portable_dir_for_exe(&fake_exe, false), None);
        assert_eq!(
            portable_dir_for_exe(&fake_exe, true),
            Some(base.join("data"))
        );

        std::fs::write(base.join("portable.txt"), "portable\n").unwrap();
        assert_eq!(
            portable_dir_for_exe(&fake_exe, false),
            Some(base.join("data"))
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
