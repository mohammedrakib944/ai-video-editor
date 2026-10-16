//! Probing media files and extracting thumbnails and waveforms.

use std::io::Read;
use std::path::Path;
use std::process::Stdio;

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};

use crate::ffmpeg;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Video,
    Audio,
    Image,
}

#[derive(Clone, Debug)]
pub struct ProbeInfo {
    pub kind: MediaKind,
    /// Seconds; 0 for still images.
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub has_audio: bool,
}

pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "bmp", "webp", "tif", "tiff"];
pub const MEDIA_EXTENSIONS: &[&str] = &[
    "mp4", "mov", "mkv", "webm", "avi", "m4v", "wmv", "flv", "mts", "m2ts", "3gp", "mpg", "mpeg", "mp3", "wav", "aac",
    "m4a", "flac", "ogg", "opus", "wma", "png", "jpg", "jpeg", "bmp", "webp", "tif", "tiff",
];

fn parse_rate(s: &str) -> f64 {
    let mut parts = s.split('/');
    let num: f64 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let den: f64 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(1.0);
    if den > 0.0 {
        num / den
    } else {
        0.0
    }
}

fn json_f64(v: &serde_json::Value) -> Option<f64> {
    match v {
        serde_json::Value::String(s) => s.parse().ok(),
        serde_json::Value::Number(n) => n.as_f64(),
        _ => None,
    }
}

fn stream_rotation(stream: &serde_json::Value) -> i64 {
    if let Some(list) = stream["side_data_list"].as_array() {
        for sd in list {
            if let Some(r) = sd["rotation"].as_f64() {
                return r as i64;
            }
        }
    }
    stream["tags"]["rotate"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0)
}

pub fn probe(path: &Path) -> anyhow::Result<ProbeInfo> {
    if !path.is_file() {
        bail!("File not found: {}", path.display());
    }
    let output = ffmpeg::ffprobe()?
        .args(["-print_format", "json", "-show_format", "-show_streams"])
        .arg(path)
        .output()
        .context("failed to run ffprobe")?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        bail!("Unsupported or damaged media file ({}): {}", path.display(), err.trim());
    }
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).context("invalid ffprobe output")?;
    let streams = json["streams"].as_array().cloned().unwrap_or_default();
    let format_name = json["format"]["format_name"].as_str().unwrap_or_default();

    // Cover art inside audio files shows up as a video stream; ignore it.
    let video = streams.iter().find(|s| {
        s["codec_type"] == "video" && s["disposition"]["attached_pic"].as_i64().unwrap_or(0) == 0
    });
    let has_audio = streams.iter().any(|s| s["codec_type"] == "audio");

    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    let is_image_format = format_name.contains("image2") || format_name.ends_with("_pipe") || IMAGE_EXTENSIONS.contains(&ext.as_str());

    let mut duration = json_f64(&json["format"]["duration"]).unwrap_or(0.0);
    if let Some(v) = video {
        if duration <= 0.0 {
            duration = json_f64(&v["duration"]).unwrap_or(0.0);
        }
        let mut width = v["width"].as_u64().unwrap_or(0) as u32;
        let mut height = v["height"].as_u64().unwrap_or(0) as u32;
        if width == 0 || height == 0 {
            bail!("Video stream has no size: {}", path.display());
        }
        if stream_rotation(v).rem_euclid(180) == 90 {
            std::mem::swap(&mut width, &mut height);
        }
        let mut fps = parse_rate(v["avg_frame_rate"].as_str().unwrap_or("0/1"));
        if !(1.0..=240.0).contains(&fps) {
            fps = parse_rate(v["r_frame_rate"].as_str().unwrap_or("0/1"));
        }
        if !(1.0..=240.0).contains(&fps) {
            fps = 30.0;
        }
        if is_image_format && !has_audio && duration < 0.5 {
            return Ok(ProbeInfo { kind: MediaKind::Image, duration: 0.0, width, height, fps: 0.0, has_audio: false });
        }
        if duration <= 0.0 {
            bail!("Could not read the duration of {}", path.display());
        }
        return Ok(ProbeInfo { kind: MediaKind::Video, duration, width, height, fps, has_audio });
    }
    if has_audio {
        if duration <= 0.0 {
            bail!("Could not read the duration of {}", path.display());
        }
        return Ok(ProbeInfo { kind: MediaKind::Audio, duration, width: 0, height: 0, fps: 0.0, has_audio: true });
    }
    bail!("No video or audio found in {}", path.display())
}

/// Reads exactly `len` bytes of raw output from an ffmpeg command.
pub fn read_raw(mut cmd: std::process::Command, len: usize) -> anyhow::Result<Vec<u8>> {
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().context("failed to start ffmpeg")?;
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let err_thread = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    let mut buf = vec![0u8; len];
    let mut filled = 0;
    while filled < len {
        match stdout.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) => {
                let _ = child.kill();
                return Err(e.into());
            }
        }
    }
    drop(stdout);
    let _ = child.kill();
    let _ = child.wait();
    let err = err_thread.join().unwrap_or_default();
    if filled < len {
        bail!("ffmpeg produced no frame: {}", err.trim());
    }
    Ok(buf)
}

/// A single RGBA thumbnail of exactly `w`x`h`, cropped to fill.
pub fn thumbnail(path: &Path, kind: MediaKind, t: f64, w: u32, h: u32) -> anyhow::Result<Vec<u8>> {
    let mut cmd = ffmpeg::ffmpeg()?;
    if kind == MediaKind::Video {
        cmd.args(["-ss", &ffmpeg::secs(t)]);
    }
    cmd.arg("-i").arg(path);
    cmd.args([
        "-frames:v",
        "1",
        "-an",
        "-vf",
        &format!("scale={w}:{h}:force_original_aspect_ratio=increase:flags=bilinear,crop={w}:{h},format=rgba"),
        "-f",
        "rawvideo",
        "pipe:1",
    ]);
    match read_raw(cmd, (w * h * 4) as usize) {
        Ok(frame) => Ok(frame),
        // Seeking right at the end of a file can come back empty; retry from the start.
        Err(_) if kind == MediaKind::Video && t > 0.0 => thumbnail(path, kind, 0.0, w, h),
        Err(e) => Err(e),
    }
}

/// Peak levels (0..1) at `per_second` buckets per second of audio.
pub fn waveform(path: &Path, per_second: u32) -> anyhow::Result<Vec<f32>> {
    const RATE: u32 = 8000;
    let mut child = ffmpeg::ffmpeg()?
        .arg("-i")
        .arg(path)
        .args(["-vn", "-ac", "1", "-ar", &RATE.to_string(), "-f", "s16le", "pipe:1"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to start ffmpeg")?;
    let mut stdout = child.stdout.take().unwrap();
    let bucket = (RATE / per_second).max(1) as usize;
    let mut peaks = Vec::new();
    let mut current = 0i32;
    let mut count = 0usize;
    let mut buf = vec![0u8; 64 * 1024];
    let mut carry: Option<u8> = None;
    loop {
        let n = stdout.read(&mut buf)?;
        if n == 0 {
            break;
        }
        let mut bytes = &buf[..n];
        let mut push = |s: i16| {
            current = current.max((s as i32).abs());
            count += 1;
            if count == bucket {
                peaks.push(current as f32 / 32768.0);
                current = 0;
                count = 0;
            }
        };
        if let Some(lo) = carry.take() {
            push(i16::from_le_bytes([lo, bytes[0]]));
            bytes = &bytes[1..];
        }
        let mut chunks = bytes.chunks_exact(2);
        for c in &mut chunks {
            push(i16::from_le_bytes([c[0], c[1]]));
        }
        if let [b] = chunks.remainder() {
            carry = Some(*b);
        }
    }
    if count > 0 {
        peaks.push(current as f32 / 32768.0);
    }
    let _ = child.wait();
    Ok(peaks)
}
