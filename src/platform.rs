//! The few things that differ between Windows and Linux: home folder, fonts,
//! opening files, and how paths look in messages.

use std::path::{Path, PathBuf};

/// Used in messages and in the AI's instructions.
#[cfg(windows)]
pub const OS_NAME: &str = "Windows";
#[cfg(not(windows))]
pub const OS_NAME: &str = "Linux";

/// An absolute path to show as an example.
#[cfg(windows)]
pub const EXAMPLE_PATH: &str = "C:\\Users\\you\\Videos\\video.mp4";
#[cfg(not(windows))]
pub const EXAMPLE_PATH: &str = "/home/you/Videos/video.mp4";

/// What to tell the user when FFmpeg is missing.
#[cfg(windows)]
pub const FFMPEG_HINT: &str = "Put ffmpeg.exe and ffprobe.exe in an 'ffmpeg' folder next to the app.";
#[cfg(not(windows))]
pub const FFMPEG_HINT: &str = "Install FFmpeg (for example `sudo apt install ffmpeg`) so ffmpeg and ffprobe are on PATH.";

pub fn home_dir() -> PathBuf {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_default()
}

/// Returns the first of `windows_names` found in the Windows font folder.
#[cfg(windows)]
pub fn find_font(windows_names: &[&str], _fontconfig_pattern: &str) -> Option<PathBuf> {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
    windows_names.iter().map(|n| Path::new(&windir).join("Fonts").join(n)).find(|p| p.is_file())
}

/// Asks fontconfig for the best match for `fontconfig_pattern` (e.g. `sans:bold`).
#[cfg(not(windows))]
pub fn find_font(_windows_names: &[&str], fontconfig_pattern: &str) -> Option<PathBuf> {
    let out = std::process::Command::new("fc-match").args(["-f", "%{file}", fontconfig_pattern]).output().ok()?;
    let path = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    (out.status.success() && path.is_file()).then_some(path)
}

/// Opens the file manager with `path` selected (or its folder, where selecting is not supported).
pub fn reveal(path: &Path) {
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn();
    #[cfg(not(windows))]
    open(path.parent().unwrap_or(path));
}

/// Opens a file with its default app.
pub fn open(path: &Path) {
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(not(windows))]
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}
