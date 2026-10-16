//! Turns a project (or a time range of it) into an FFmpeg filter graph.
//!
//! The same graph renders the live preview, the frames the AI looks at and the
//! final export, so the preview always matches the exported file.

use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

use anyhow::{bail, Context};

use crate::ffmpeg::{self, secs};
use crate::media::{self, MediaKind};
use crate::project::{Canvas, Fit, Project, Quality};

pub const AUDIO_RATE: u32 = 48_000;

#[derive(Clone, Copy, Debug)]
pub struct RenderSpec {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub start: f64,
    pub end: f64,
    pub video: bool,
    pub audio: bool,
    /// Faster scaling and hardware decoding for interactive preview.
    pub preview: bool,
}

/// Temporary files for one ffmpeg run; removed when dropped.
pub struct JobDir {
    pub rel: String,
    abs: PathBuf,
}

impl Drop for JobDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.abs);
    }
}

fn new_job_dir() -> anyhow::Result<JobDir> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let rel = format!("jobs/{}-{n}", std::process::id());
    let abs = ffmpeg::work_dir().join(&rel);
    std::fs::create_dir_all(&abs).context("could not create a temp folder")?;
    Ok(JobDir { rel, abs })
}

/// Copies a system font into the work dir once so filter graphs can use a short relative path.
fn font_available() -> bool {
    static FONT: OnceLock<bool> = OnceLock::new();
    *FONT.get_or_init(|| {
        let dest = ffmpeg::work_dir().join("font.ttf");
        if dest.is_file() {
            return true;
        }
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
        for name in ["segoeuib.ttf", "arialbd.ttf", "segoeui.ttf", "arial.ttf"] {
            let src = Path::new(&windir).join("Fonts").join(name);
            if src.is_file() && std::fs::copy(&src, &dest).is_ok() {
                return true;
            }
        }
        false
    })
}

fn overlay_font() -> Option<&'static ab_glyph::FontVec> {
    static FONT: OnceLock<Option<ab_glyph::FontVec>> = OnceLock::new();
    FONT.get_or_init(|| {
        if !font_available() {
            return None;
        }
        let data = std::fs::read(ffmpeg::work_dir().join("font.ttf")).ok()?;
        ab_glyph::FontVec::try_from_vec(data).ok()
    })
    .as_ref()
}

/// Largest font size (up to `px`) at which every line fits within the frame width.
pub fn fit_text_size(text: &str, px: u32, frame_w: u32, boxed: bool) -> u32 {
    use ab_glyph::{Font, ScaleFont};
    let Some(font) = overlay_font() else { return px };
    let scaled = font.as_scaled(px as f32);
    let widest = text
        .lines()
        .map(|line| {
            let mut w = 0.0;
            let mut prev = None;
            for c in line.chars() {
                let g = scaled.glyph_id(c);
                if let Some(p) = prev {
                    w += scaled.kern(p, g);
                }
                w += scaled.h_advance(g);
                prev = Some(g);
            }
            w
        })
        .fold(0.0f32, f32::max);
    let border = if boxed { 2.0 * (px / 3).max(2) as f32 } else { 0.0 };
    let available = frame_w as f32 * 0.92 - border;
    if widest <= available || widest <= 0.0 {
        return px;
    }
    ((px as f32 * available / widest).floor() as u32).max(4)
}

/// The preview renders at a reduced size with the canvas's aspect ratio.
pub fn preview_size(canvas: &Canvas, max_long_side: u32) -> (u32, u32) {
    let (w, h) = (canvas.width as f64, canvas.height as f64);
    let scale = (max_long_side as f64 / w.max(h)).min(1.0);
    let even = |v: f64| ((v * scale / 2.0).round() as u32).max(1) * 2;
    (even(w), even(h))
}

fn fit_filter(fit: Fit, w: u32, h: u32, preview: bool) -> String {
    let flags = if preview { "bilinear" } else { "bicubic" };
    match fit {
        Fit::Fit => format!("scale={w}:{h}:force_original_aspect_ratio=decrease:flags={flags},pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color=black"),
        Fit::Fill => format!("scale={w}:{h}:force_original_aspect_ratio=increase:flags={flags},crop={w}:{h}"),
    }
}

const AUDIO_NORM: &str = "aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo";

/// `,eq=...` for clips with color adjustments, else empty.
fn color_filter(clip: &crate::project::Clip) -> String {
    if !clip.has_color() {
        return String::new();
    }
    format!(",eq=brightness={:.3}:contrast={:.3}:saturation={:.3}", clip.brightness * 0.5, clip.contrast, clip.saturation)
}

/// `,atempo=...` steps for a speed change (each atempo stage handles 0.5..2).
fn atempo_chain(speed: f64) -> String {
    if (speed - 1.0).abs() < 1e-6 {
        return String::new();
    }
    let mut s = speed;
    let mut out = String::new();
    while s > 2.0 {
        out.push_str(",atempo=2.0");
        s /= 2.0;
    }
    while s < 0.5 {
        out.push_str(",atempo=0.5");
        s /= 0.5;
    }
    let _ = write!(out, ",atempo={s:.6}");
    out
}

/// Fade in/out filters for one rendered piece of a clip.
///
/// The piece starts `offset` seconds into a clip lasting `clip_dur`. Timestamps are
/// shifted to clip time for the fade, so a preview that starts mid-fade matches export.
fn fade_filters(audio: bool, offset: f64, piece: f64, clip_dur: f64, fade_in: f64, fade_out: f64) -> String {
    let fi = fade_in.min(clip_dur / 2.0);
    let fo = fade_out.min(clip_dur / 2.0);
    let wants_in = fi > 1e-3 && offset < fi;
    let wants_out = fo > 1e-3 && offset + piece > clip_dur - fo;
    if !wants_in && !wants_out {
        return String::new();
    }
    let (setpts, fade) = if audio { ("asetpts", "afade") } else { ("setpts", "fade") };
    let mut s = format!(",{setpts}=PTS+{}/TB", secs(offset));
    if wants_in {
        let _ = write!(s, ",{fade}=t=in:st=0:d={}", secs(fi));
    }
    if wants_out {
        let _ = write!(s, ",{fade}=t=out:st={}:d={}", secs(clip_dur - fo), secs(fo));
    }
    let _ = write!(s, ",{setpts}=PTS-STARTPTS");
    s
}

/// Builds an ffmpeg command with all inputs and the filter graph. The graph
/// exposes `[vout]` (if `spec.video`) and `[aout]` (if `spec.audio`); the
/// caller adds the output options.
pub fn build(project: &Project, spec: &RenderSpec, rgba: bool) -> anyhow::Result<(Command, JobDir)> {
    let dur = spec.end - spec.start;
    if !(dur > 0.0) {
        bail!("Nothing to render (empty time range)");
    }
    if !spec.video && !spec.audio {
        bail!("Nothing to render");
    }
    let (w, h, fps) = (spec.width, spec.height, spec.fps);
    let job = new_job_dir()?;
    let mut cmd = ffmpeg::ffmpeg()?;
    cmd.current_dir(ffmpeg::work_dir());
    let mut graph = String::new();
    let mut input = 0usize;

    // ----- Main track ----------------------------------------------------------
    let mut vlabels = Vec::new();
    let mut alabels = Vec::new();
    let starts = project.main_starts();
    for (k, clip) in project.main.iter().enumerate() {
        let (cs, ce) = (starts[k], starts[k] + clip.duration());
        let (t0, t1) = (cs.max(spec.start), ce.min(spec.end));
        let d = t1 - t0;
        if d <= 1e-6 {
            continue;
        }
        let asset = project.asset(clip.asset_id).with_context(|| format!("clip {} uses missing media", clip.id))?;
        if !asset.path.is_file() {
            bail!("Media file is missing: {}", asset.path.display());
        }
        let offset = t0 - cs;
        let src = clip.src_in + clip.to_src(offset);
        let ds = secs(d);
        let speed = if asset.kind == MediaKind::Image { 1.0 } else { clip.speed };
        let seg = vlabels.len().max(alabels.len());
        let wants_audio = spec.audio && asset.kind == MediaKind::Video && asset.has_audio;
        let needs_input = spec.video || wants_audio;
        let mut this_input = None;
        if needs_input {
            match asset.kind {
                MediaKind::Image => {
                    if spec.video {
                        cmd.args(["-loop", "1", "-framerate", &fps.to_string(), "-t", &secs(d + 0.1), "-i"]).arg(&asset.path);
                        this_input = Some(input);
                        input += 1;
                    }
                }
                MediaKind::Video => {
                    cmd.args(["-ss", &secs(src), "-t", &secs(d * speed + 0.25), "-i"]).arg(&asset.path);
                    this_input = Some(input);
                    input += 1;
                }
                MediaKind::Audio => bail!("audio file on the main track"),
            }
        }
        if spec.video {
            let i = this_input.unwrap();
            let _ = writeln!(
                graph,
                "[{i}:v]setpts=(PTS-STARTPTS)/{speed:.6},{},setsar=1,fps={fps},format=yuv420p{},tpad=stop_mode=clone:stop_duration={ds},trim=duration={ds},setpts=PTS-STARTPTS{}[v{seg}];",
                fit_filter(clip.fit, w, h, spec.preview),
                color_filter(clip),
                fade_filters(false, offset, d, clip.duration(), clip.fade_in, clip.fade_out)
            );
            vlabels.push(format!("[v{seg}]"));
        }
        if spec.audio {
            if wants_audio {
                let i = this_input.unwrap();
                let _ = writeln!(
                    graph,
                    "[{i}:a]asetpts=PTS-STARTPTS{},{AUDIO_NORM},volume={:.3},apad=whole_dur={ds},atrim=duration={ds},asetpts=PTS-STARTPTS{}[a{seg}];",
                    atempo_chain(speed),
                    clip.volume,
                    fade_filters(true, offset, d, clip.duration(), clip.fade_in, clip.fade_out)
                );
            } else {
                let _ = writeln!(graph, "anullsrc=r=48000:cl=stereo,atrim=duration={ds}[a{seg}];");
            }
            alabels.push(format!("[a{seg}]"));
        }
    }

    let n = vlabels.len().max(alabels.len());
    if n == 0 {
        if spec.video {
            let _ = writeln!(graph, "color=c=black:s={w}x{h}:r={fps},format=yuv420p,trim=duration={}[vc];", secs(dur));
        }
        if spec.audio {
            let _ = writeln!(graph, "anullsrc=r=48000:cl=stereo,atrim=duration={}[ac];", secs(dur));
        }
    } else {
        for s in 0..n {
            if spec.video {
                graph.push_str(&vlabels[s]);
            }
            if spec.audio {
                graph.push_str(&alabels[s]);
            }
        }
        let (vo, ao) = (if spec.video { "[vc]" } else { "" }, if spec.audio { "[ac]" } else { "" });
        let _ = writeln!(graph, "concat=n={n}:v={}:a={}{vo}{ao};", spec.video as u8, spec.audio as u8);
    }

    // ----- Text overlays -------------------------------------------------------
    if spec.video {
        let mut chain = Vec::new();
        let has_font = font_available();
        for (k, t) in project.texts.iter().enumerate() {
            let (t0, t1) = (t.start.max(spec.start), t.end().min(spec.end));
            if t1 - t0 <= 1e-6 {
                continue;
            }
            let file = format!("{}/text{k}.txt", job.rel);
            std::fs::write(ffmpeg::work_dir().join(&file), t.text.trim_end()).context("could not write text overlay")?;
            // Size follows the shorter side so titles look alike in 16:9 and 9:16,
            // then shrinks if the longest line would not fit the frame width.
            let px = ((t.size as f64) * w.min(h) as f64).round().max(4.0) as u32;
            let px = fit_text_size(&t.text, px, w, t.background);
            let [r, g, b] = t.color;
            let mut f = String::from("drawtext=");
            if has_font {
                f.push_str("fontfile=font.ttf:");
            }
            let _ = write!(
                f,
                "textfile={file}:expansion=none:fontsize={px}:fontcolor=0x{r:02x}{g:02x}{b:02x}:line_spacing={}:text_align=C:x=w*{:.4}-text_w/2:y=h*{:.4}-text_h/2:fix_bounds=1",
                px / 5,
                t.x,
                t.y
            );
            if t.background {
                let _ = write!(f, ":box=1:boxcolor=0x000000@0.55:boxborderw={}", (px / 3).max(2));
            } else {
                let sh = (px / 18).max(1);
                let _ = write!(f, ":shadowcolor=0x000000@0.75:shadowx={sh}:shadowy={sh}");
            }
            let _ = write!(f, ":enable='between(t,{},{})'", secs(t0 - spec.start), secs(t1 - spec.start));
            if t.fade {
                // Times relative to this render, unclipped, so a fade is right even mid-way through.
                let (a, b) = (t.start - spec.start, t.end() - spec.start);
                let fd = (t.duration / 3.0).min(0.3);
                let _ = write!(f, ":alpha='if(lt(t,{a:.4}+{fd:.4}),max(0,(t-({a:.4}))/{fd:.4}),if(gt(t,{b:.4}-{fd:.4}),max(0,({b:.4}-t)/{fd:.4}),1))'");
            }
            chain.push(f);
        }
        let fmt = if rgba { "rgba" } else { "yuv420p" };
        if chain.is_empty() {
            let _ = writeln!(graph, "[vc]format={fmt}[vout];");
        } else {
            let _ = writeln!(graph, "[vc]{},format={fmt}[vout];", chain.join(","));
        }
    }

    // ----- Music track ---------------------------------------------------------
    if spec.audio {
        let mut mlabels = Vec::new();
        for (k, m) in project.music.iter().enumerate() {
            let (t0, t1) = (m.start.max(spec.start), m.end().min(spec.end));
            let d = t1 - t0;
            if d <= 1e-6 {
                continue;
            }
            let Some(asset) = project.asset(m.asset_id) else { continue };
            if !asset.path.is_file() {
                bail!("Media file is missing: {}", asset.path.display());
            }
            let offset = t0 - m.start;
            let src = m.src_in + offset;
            cmd.args(["-ss", &secs(src), "-t", &secs(d + 0.25), "-vn", "-i"]).arg(&asset.path);
            let delay_ms = ((t0 - spec.start) * 1000.0).round() as u64;
            let _ = writeln!(
                graph,
                "[{input}:a]asetpts=PTS-STARTPTS,{AUDIO_NORM},volume={:.3},atrim=duration={},asetpts=PTS-STARTPTS{},adelay=delays={delay_ms}:all=1[m{k}];",
                m.volume,
                secs(d),
                fade_filters(true, offset, d, m.duration(), m.fade_in, m.fade_out)
            );
            input += 1;
            mlabels.push(format!("[m{k}]"));
        }
        if mlabels.is_empty() {
            let _ = writeln!(graph, "[ac]anull[aout];");
        } else {
            let _ = writeln!(
                graph,
                "[ac]{}amix=inputs={}:duration=first:dropout_transition=0:normalize=0[aout];",
                mlabels.join(""),
                mlabels.len() + 1
            );
        }
    }

    let graph = graph.trim_end().trim_end_matches(';').to_string();
    let graph_file = format!("{}/graph.txt", job.rel);
    std::fs::write(ffmpeg::work_dir().join(&graph_file), &graph).context("could not write filter graph")?;
    cmd.arg("-/filter_complex").arg(&graph_file);
    Ok((cmd, job))
}

/// Renders one RGBA frame of the timeline at time `t`.
pub fn render_frame(project: &Project, t: f64, width: u32, height: u32) -> anyhow::Result<Vec<u8>> {
    let total = project.duration();
    let frame = project.canvas.frame();
    if total <= 0.0 {
        return Ok(black_frame(width, height));
    }
    let t = t.clamp(0.0, (total - frame).max(0.0));
    let spec = RenderSpec {
        width,
        height,
        fps: project.canvas.fps,
        start: t,
        end: (t + frame * 2.0).min(total),
        video: true,
        audio: false,
        preview: true,
    };
    let (mut cmd, _job) = build(project, &spec, true)?;
    cmd.args(["-map", "[vout]", "-frames:v", "1", "-f", "rawvideo", "pipe:1"]);
    media::read_raw(cmd, (width * height * 4) as usize)
}

pub fn black_frame(width: u32, height: u32) -> Vec<u8> {
    let mut v = vec![0u8; (width * height * 4) as usize];
    for px in v.chunks_exact_mut(4) {
        px[3] = 255;
    }
    v
}

/// Encodes an RGBA frame as JPEG for the AI.
pub fn encode_jpeg(rgba: &[u8], width: u32, height: u32) -> anyhow::Result<Vec<u8>> {
    let img = image::RgbaImage::from_raw(width, height, rgba.to_vec()).context("bad frame size")?;
    let rgb = image::DynamicImage::ImageRgba8(img).to_rgb8();
    let mut out = std::io::Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85).encode_image(&rgb)?;
    Ok(out.into_inner())
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

pub struct ExportProgress {
    pub progress: f32,
    pub eta_secs: Option<f64>,
}

/// Exports the whole timeline. `on_progress` returns false to cancel.
/// Writes to a temporary file first so a cancelled or failed export never
/// leaves a broken file behind.
pub fn export(
    project: &Project,
    canvas: Canvas,
    quality: Quality,
    out: &Path,
    mut on_progress: impl FnMut(ExportProgress) -> bool,
) -> anyhow::Result<f64> {
    canvas.validate()?;
    let total = project.duration();
    if total <= 0.0 {
        bail!("The timeline is empty — add a clip before exporting");
    }
    if let Some(m) = project.missing_media().first() {
        bail!("Media file is missing: {}", m.path.display());
    }
    if let Some(dir) = out.parent() {
        if !dir.as_os_str().is_empty() && !dir.is_dir() {
            bail!("Folder does not exist: {}", dir.display());
        }
    }
    let spec = RenderSpec {
        width: canvas.width,
        height: canvas.height,
        fps: canvas.fps,
        start: 0.0,
        end: total,
        video: true,
        audio: true,
        preview: false,
    };
    let (mut cmd, _job) = build(project, &spec, false)?;
    let ext = out.extension().and_then(|e| e.to_str()).unwrap_or("mp4").to_ascii_lowercase();
    let tmp = out.with_extension(format!("partial.{ext}"));
    cmd.args(["-map", "[vout]", "-map", "[aout]"]);
    cmd.args(["-c:v", "libx264", "-preset", "medium", "-crf", &quality.crf().to_string(), "-pix_fmt", "yuv420p"]);
    cmd.args(["-r", &canvas.fps.to_string(), "-c:a", "aac", "-b:a", "192k", "-ar", "48000"]);
    if ext == "mp4" || ext == "mov" || ext == "m4v" {
        cmd.args(["-movflags", "+faststart"]);
    }
    cmd.args(["-progress", "pipe:1", "-nostats", "-y"]).arg(&tmp);
    let started = Instant::now();
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().context("failed to start ffmpeg")?;
    let mut stderr = child.stderr.take().unwrap();
    let err_thread = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    let reader = BufReader::new(child.stdout.take().unwrap());
    let mut cancelled = false;
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<f64>().ok()) {
            let p = ((us / 1e6) / total).clamp(0.0, 1.0) as f32;
            let elapsed = started.elapsed().as_secs_f64();
            let eta = (p > 0.02).then(|| elapsed / p as f64 * (1.0 - p as f64));
            if !on_progress(ExportProgress { progress: p, eta_secs: eta }) {
                cancelled = true;
                let _ = child.kill();
                break;
            }
        }
    }
    let status = child.wait()?;
    let err = err_thread.join().unwrap_or_default();
    if cancelled {
        let _ = std::fs::remove_file(&tmp);
        bail!("cancelled");
    }
    if !status.success() {
        let _ = std::fs::remove_file(&tmp);
        let msg = err.lines().filter(|l| !l.trim().is_empty()).last().unwrap_or("unknown error");
        bail!("Export failed: {msg}");
    }
    if out.exists() {
        std::fs::remove_file(out).with_context(|| format!("could not replace {} (is it open in another app?)", out.display()))?;
    }
    std::fs::rename(&tmp, out).with_context(|| format!("could not write {}", out.display()))?;
    on_progress(ExportProgress { progress: 1.0, eta_secs: Some(0.0) });
    Ok(started.elapsed().as_secs_f64())
}

/// Rough output size in bytes, for the export dialog.
pub fn estimate_size(canvas: &Canvas, quality: Quality, secs: f64) -> f64 {
    let pixels_per_sec = canvas.width as f64 * canvas.height as f64 * canvas.fps as f64;
    let bits_per_pixel = match quality {
        Quality::High => 0.11,
        Quality::Standard => 0.06,
        Quality::Small => 0.03,
    };
    (pixels_per_sec * bits_per_pixel + 192_000.0) * secs / 8.0
}
