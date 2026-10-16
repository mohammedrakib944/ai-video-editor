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
        .ok_or_else(|| anyhow::anyhow!("FFmpeg was not found. {}", crate::platform::FFMPEG_HINT))
}

pub fn is_available() -> bool {
    binaries().is_ok()
}

#[cfg_attr(not(windows), allow(unused_mut))]
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

/// The option that reads a filter graph from a file: `-/filter_complex` since FFmpeg 7,
/// `-filter_complex_script` before (e.g. the 6.1 shipped by Ubuntu 24.04).
pub fn filter_script_option() -> &'static str {
    static OPTION: OnceLock<&'static str> = OnceLock::new();
    OPTION.get_or_init(|| {
        let major = binaries()
            .ok()
            .and_then(|b| quiet(Command::new(&b.ffmpeg)).arg("-version").output().ok())
            .and_then(|out| parse_major_version(&String::from_utf8_lossy(&out.stdout)));
        match major {
            Some(v) if v < 7 => "-filter_complex_script",
            _ => "-/filter_complex",
        }
    })
}

/// `ffmpeg version 6.1.1-3ubuntu5 ...` -> 6, `n7.1-...` -> 7. Git builds (`N-118000-...`) give None.
fn parse_major_version(version_output: &str) -> Option<u32> {
    let v = version_output.split_whitespace().nth(2)?;
    v.strip_prefix('n').unwrap_or(v).split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
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

#[cfg(test)]
mod tests {
    use super::parse_major_version;

    #[test]
    fn reads_ffmpeg_major_version() {
        assert_eq!(parse_major_version("ffmpeg version 6.1.1-3ubuntu5 Copyright (c) 2000-2023"), Some(6));
        assert_eq!(parse_major_version("ffmpeg version 7.1-full_build-www.gyan.dev Copyright"), Some(7));
        assert_eq!(parse_major_version("ffmpeg version n7.1-12-g1234 Copyright"), Some(7));
        assert_eq!(parse_major_version("ffmpeg version n6.1.2 Copyright"), Some(6));
        assert_eq!(parse_major_version("ffmpeg version N-118000-gabc123-20250101 Copyright"), None);
    }
}
