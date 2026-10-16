//! Recently opened projects, remembered across sessions.

use std::path::{Path, PathBuf};

use crate::ffmpeg;

const MAX: usize = 8;

fn file() -> PathBuf {
    ffmpeg::work_dir().join("recent.json")
}

/// Recent project paths that still exist, newest first.
pub fn load() -> Vec<PathBuf> {
    let list: Vec<PathBuf> = std::fs::read_to_string(file()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    list.into_iter().filter(|p| p.is_file()).take(MAX).collect()
}

/// Moves `path` to the front of the list and saves it. Returns the new list.
pub fn push(path: &Path) -> Vec<PathBuf> {
    let mut list = load();
    list.retain(|p| p != path);
    list.insert(0, path.to_path_buf());
    list.truncate(MAX);
    if let Ok(json) = serde_json::to_string_pretty(&list) {
        let _ = std::fs::write(file(), json);
    }
    list
}
