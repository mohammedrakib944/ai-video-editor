//! Locating and launching FFmpeg.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

struct Binaries {
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
}

static BINARIES: OnceLock<Option<Binaries>> = OnceLock::new();

fn exe(base: &str) -> String {
    if cfg!(windows) { format!("{base}.exe") } else { base.to_string() }
}

fn candidate_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(dir) = std::env::var("AIVE_FFMPEG_DIR") {
        dirs.push(PathBuf::from(dir));
    }
    if let Some(exe_dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
        dirs.push(exe_dir.join("ffmpeg"));
        dirs.push(exe_dir);
    }
    // Development: the vendored copy next to the workspace root.
    dirs.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("vendor").join("ffmpeg"));
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    dirs
}

fn binaries() -> anyhow::Result<&'static Binaries> {
    BINARIES
        .get_or_init(|| {
            candidate_dirs().into_iter().find_map(|dir| {
                let (ffmpeg, ffprobe) = (dir.join(exe("ffmpeg")), dir.join(exe("ffprobe")));
                (ffmpeg.is_file() && ffprobe.is_file()).then_some(Binaries { ffmpeg, ffprobe })
            })
        })
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("FFmpeg was not found. {}", missing_hint()))
}

pub fn missing_hint() -> &'static str {
    if cfg!(windows) {
        "Put ffmpeg.exe and ffprobe.exe in an 'ffmpeg' folder next to the app."
    } else {
        "Install FFmpeg (for example `sudo apt install ffmpeg`) so ffmpeg and ffprobe are on PATH."
    }
}

pub fn is_available() -> bool {
    binaries().is_ok()
}

#[cfg_attr(not(windows), allow(unused_mut))]
fn quiet(mut cmd: Command) -> Command {
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    cmd
}

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

/// Seconds with microsecond precision, for ffmpeg arguments.
pub fn secs(t: f64) -> String {
    format!("{:.6}", t.max(0.0))
}
