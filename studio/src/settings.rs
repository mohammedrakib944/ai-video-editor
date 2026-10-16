//! Per-user settings: recent projects and the editor layout.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentProject {
    pub name: String,
    pub path: PathBuf,
    pub summary: String,
    pub opened_at: u64,
}

/// Split ratios of the editor panes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub top_bottom: f32,
    pub library: f32,
    pub preview_inspector: f32,
}

impl Default for Layout {
    fn default() -> Self {
        Layout { top_bottom: 0.6, library: 0.2, preview_inspector: 0.74 }
    }
}

/// The AI service the assistant uses. Nothing is used until the user connects one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AiProvider {
    ClaudeCode,
}

impl AiProvider {
    pub fn name(self) -> &'static str {
        match self {
            AiProvider::ClaudeCode => "Claude Code",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub recent: Vec<RecentProject>,
    #[serde(default)]
    pub layout: Layout,
    #[serde(default)]
    pub ai: Option<AiProvider>,
}

const MAX_RECENT: usize = 12;

impl Settings {
    pub fn load() -> Settings {
        std::fs::read_to_string(file()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let path = file();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }

    pub fn touch_recent(&mut self, name: &str, path: &Path, summary: String) {
        self.recent.retain(|r| r.path != path);
        let opened_at = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        self.recent.insert(0, RecentProject { name: name.to_string(), path: path.to_path_buf(), summary, opened_at });
        self.recent.truncate(MAX_RECENT);
    }
}

pub fn config_dir() -> PathBuf {
    if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_default().join("AIVideoEditor")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .unwrap_or_default()
            .join("ai-video-editor")
    }
}

fn file() -> PathBuf {
    config_dir().join("studio.json")
}

pub fn home_dir() -> PathBuf {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_default()
}

/// Folder where new projects are saved by default.
pub fn projects_dir() -> PathBuf {
    let home = home_dir();
    let videos = ["Videos", "Movies"].iter().map(|d| home.join(d)).find(|d| d.is_dir()).unwrap_or(home);
    videos.join("AI Video Editor")
}

/// "just now", "5 min ago", "3 days ago".
pub fn relative_time(unix: u64) -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(unix);
    let s = now.saturating_sub(unix);
    match s {
        0..60 => "Just now".into(),
        60..3600 => format!("{} min ago", s / 60),
        3600..86400 => format!("{} h ago", s / 3600),
        86400..172800 => "Yesterday".into(),
        _ => format!("{} days ago", s / 86400),
    }
}
