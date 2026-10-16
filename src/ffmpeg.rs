//! Locating and launching the bundled FFmpeg binaries.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

struct Binaries {
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
}

static BINARIES: OnceLock<Option<Binaries>> = OnceLock::new();

fn exe_name(base: &str) -> String {
    if cfg!(windows) {
        format!("{base}.exe")
    } else {
        base.to_string()
    }
}

fn candidate_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(dir) = std::env::var("AIVE_FFMPEG_DIR") {
        dirs.push(PathBuf::from(dir));
    }
    if let Some(exe_dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
        dirs.push(exe_dir.join("ffmpeg"));
        dirs.push(exe_dir.clone());
    }
    // Development fallback: the vendored copy next to Cargo.toml.
    dirs.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vendor").join("ffmpeg"));
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    dirs
}

fn locate() -> Option<Binaries> {
    for dir in candidate_dirs() {
        let ffmpeg = dir.join(exe_name("ffmpeg"));
        let ffprobe = dir.join(exe_name("ffprobe"));
        if ffmpeg.is_file() && ffprobe.is_file() {
            return Some(Binaries { ffmpeg, ffprobe });
        }
    }
    None
}

fn binaries() -> anyhow::Result<&'static Binaries> {
    BINARIES
        .get_or_init(locate)
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("FFmpeg was not found. Put ffmpeg.exe and ffprobe.exe in an 'ffmpeg' folder next to the app."))
}

pub fn is_available() -> bool {
    binaries().is_ok()
}

fn quiet(mut cmd: Command) -> Command {
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// `ffmpeg` with banner and interactive input disabled.
pub fn ffmpeg() -> anyhow::Result<Command> {
    let mut cmd = quiet(Command::new(&binaries()?.ffmpeg));
    cmd.args(["-hide_banner", "-nostdin", "-loglevel", "error"]);
    Ok(cmd)
}

pub fn ffprobe() -> anyhow::Result<Command> {
    let mut cmd = quiet(Command::new(&binaries()?.ffprobe));
    cmd.args(["-hide_banner", "-loglevel", "error"]);
    Ok(cmd)
}

/// Any other console program, launched without flashing a console window.
pub fn hidden_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    quiet(Command::new(program))
}

/// Formats seconds for ffmpeg arguments with microsecond precision.
pub fn secs(t: f64) -> String {
    format!("{:.6}", t.max(0.0))
}

/// Per-user working directory for temp graphs, fonts and caches.
pub fn work_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join("AIVideoEditor");
    let _ = std::fs::create_dir_all(&dir);
    dir
}
