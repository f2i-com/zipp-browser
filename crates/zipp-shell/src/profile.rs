//! Where the browser keeps what outlives it: cookies, and what sites store.

use std::path::PathBuf;
use std::sync::OnceLock;

/// The profile folder, made on first use: `ZIPP_PROFILE_DIR` if set (tests
/// keep theirs apart), else `%LOCALAPPDATA%\Zipp` on Windows and
/// `$XDG_DATA_HOME/zipp` (`~/.local/share/zipp`) elsewhere. `None` when there
/// is nowhere to keep it, and then nothing is kept.
pub fn dir() -> Option<PathBuf> {
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
