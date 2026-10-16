//! End-to-end tests against real FFmpeg: import every common format, edit,
//! render frames and export, then verify the output with ffprobe.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;

use ai_video_editor::ffmpeg;
use ai_video_editor::media::{self, MediaKind};
use ai_video_editor::project::{Canvas, Fit, Project, Quality, TextClip};
use ai_video_editor::render::{self, RenderSpec};

struct Fixtures {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

fn make(root: &Path, name: &str, args: &[&str]) -> PathBuf {
    let out = root.join(name);
    let status = ffmpeg::ffmpeg().unwrap().args(args).arg("-y").arg(&out).status().unwrap();
    assert!(status.success(), "failed to create fixture {name}");
    out
}

fn fixtures() -> &'static Fixtures {
    static F: OnceLock<Fixtures> = OnceLock::new();
    F.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let av = ["-f", "lavfi", "-i", "testsrc2=size=640x360:rate=30:duration=4", "-f", "lavfi", "-i", "sine=frequency=440:duration=4"];
        let mut a = av.to_vec();
        a.extend(["-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"]);
        make(&root, "clip.mp4", &a);
        make(&root, "clip.mov", &a);
        make(&root, "clip.mkv", &a);
        let mut w = av.to_vec();
        w.extend(["-c:v", "libvpx-vp9", "-deadline", "realtime", "-cpu-used", "8", "-b:v", "300k", "-c:a", "libopus", "-shortest"]);
        make(&root, "clip.webm", &w);
        make(&root, "portrait.mp4", &["-f", "lavfi", "-i", "testsrc2=size=360x640:rate=25:duration=3", "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"]);
        make(&root, "music.mp3", &["-f", "lavfi", "-i", "sine=frequency=220:duration=6", "-c:a", "libmp3lame"]);
        make(&root, "still.png", &["-f", "lavfi", "-i", "testsrc2=size=800x600:duration=1", "-frames:v", "1"]);
        // Path with spaces and non-ASCII characters.
        let odd = root.join("my clips ✓");
        std::fs::create_dir_all(&odd).unwrap();
        make(&odd, "odd name 'quoted'.mp4", &a);
        Fixtures { _dir: dir, root }
    })
}

fn probe_output(path: &Path) -> serde_json::Value {
    let out = ffmpeg::ffprobe().unwrap().args(["-print_format", "json", "-show_format", "-show_streams"]).arg(path).output().unwrap();
    assert!(out.status.success());
    serde_json::from_slice(&out.stdout).unwrap()
}

fn add(p: &mut Project, path: &Path) -> u64 {
    let info = media::probe(path).unwrap();
    p.add_asset(path, &info)
}

#[test]
fn probes_all_formats() {
    let f = fixtures();
    for name in ["clip.mp4", "clip.mov", "clip.mkv", "clip.webm"] {
        let info = media::probe(&f.root.join(name)).unwrap();
        assert_eq!(info.kind, MediaKind::Video, "{name}");
        assert!((info.duration - 4.0).abs() < 0.2, "{name}: {}", info.duration);
        assert_eq!((info.width, info.height), (640, 360), "{name}");
        assert!(info.has_audio, "{name}");
    }
    let p = media::probe(&f.root.join("portrait.mp4")).unwrap();
    assert_eq!((p.width, p.height, p.has_audio), (360, 640, false));
    assert_eq!(media::probe(&f.root.join("music.mp3")).unwrap().kind, MediaKind::Audio);
    let img = media::probe(&f.root.join("still.png")).unwrap();
    assert_eq!(img.kind, MediaKind::Image);
    assert_eq!((img.width, img.height), (800, 600));
    assert!(media::probe(&f.root.join("missing.mp4")).is_err());
    let junk = f.root.join("junk.mp4");
    std::fs::write(&junk, b"definitely not a video").unwrap();
    assert!(media::probe(&junk).is_err());
}

#[test]
fn thumbnails_and_waveforms() {
    let f = fixtures();
    let t = media::thumbnail(&f.root.join("clip.mp4"), MediaKind::Video, 1.0, 96, 54).unwrap();
    assert_eq!(t.len(), 96 * 54 * 4);
    // Past the end falls back to the first frame instead of failing.
    assert!(media::thumbnail(&f.root.join("clip.mp4"), MediaKind::Video, 100.0, 96, 54).is_ok());
    let img = media::thumbnail(&f.root.join("still.png"), MediaKind::Image, 0.0, 96, 54).unwrap();
    assert_eq!(img.len(), 96 * 54 * 4);
    let wf = media::waveform(&f.root.join("music.mp3"), 50).unwrap();
    assert!((wf.len() as i64 - 300).abs() <= 3, "{}", wf.len());
    let peak = wf.iter().cloned().fold(0.0, f32::max);
    assert!(peak > 0.08 && peak < 0.2, "lavfi sine is generated at 1/8 amplitude, got {peak}");
}

fn sample_project() -> Project {
    let f = fixtures();
    let mut p = Project::new("test", Canvas::from_preset(16, 9, 360, 30));
    let a = add(&mut p, &f.root.join("clip.mp4"));
    let b = add(&mut p, &f.root.join("portrait.mp4"));
    let c = add(&mut p, &f.root.join("still.png"));
    let d = add(&mut p, &f.root.join("clip.webm"));
    let m = add(&mut p, &f.root.join("music.mp3"));
    let odd = add(&mut p, &f.root.join("my clips ✓").join("odd name 'quoted'.mp4"));
    p.insert_main(a, None, Some((0.5, 3.0))).unwrap();
    let pid = p.insert_main(b, None, None).unwrap();
    p.set_fit(pid, Fit::Fill).unwrap();
    p.insert_main(c, None, Some((0.0, 1.5))).unwrap();
    p.insert_main(d, None, None).unwrap();
    p.insert_main(odd, None, Some((0.0, 1.0))).unwrap();
    p.add_music(m, 1.0, Some((0.0, 4.0))).unwrap();
    p.add_text(TextClip::new("Hello: it's 100% \"working\", ok?\nline two", 0.2, 2.0)).unwrap();
    p.split_at(5.0).unwrap();
    p.delete_range(6.0, 6.5).unwrap();
    p
}

fn mean_luma(rgba: &[u8]) -> f64 {
    let sum: u64 = rgba.chunks_exact(4).map(|p| (p[0] as u64 + p[1] as u64 + p[2] as u64) / 3).sum();
    sum as f64 / (rgba.len() / 4) as f64
}

#[test]
fn renders_frames_across_the_timeline() {
    let p = sample_project();
    let total = p.duration();
    let (w, h) = render::preview_size(&p.canvas, 480);
    for t in [0.0, 1.0, 2.6, 4.0, 5.5, total - 0.01, total + 5.0] {
        let f = render::render_frame(&p, t, w, h).unwrap_or_else(|e| panic!("frame at {t}: {e}"));
        assert_eq!(f.len(), (w * h * 4) as usize);
        assert!(mean_luma(&f) > 10.0, "frame at {t} is black");
    }
    let empty = Project::new("e", Canvas::default());
    assert_eq!(render::render_frame(&empty, 0.0, 64, 36).unwrap().len(), 64 * 36 * 4);
}

#[test]
fn preview_stream_produces_exact_frame_count() {
    let p = sample_project();
    let from = 2.0;
    let (w, h) = (192, 108);
    let spec = RenderSpec { width: w, height: h, fps: 30, start: from, end: p.duration(), video: true, audio: false, preview: true };
    let (mut cmd, _job) = render::build(&p, &spec, true).unwrap();
    cmd.args(["-map", "[vout]", "-f", "rawvideo", "pipe:1"]);
    let out = cmd.stdout(Stdio::piped()).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let frames = out.stdout.len() / (w * h * 4) as usize;
    let expected = ((p.duration() - from) * 30.0).round() as usize;
    assert!((frames as i64 - expected as i64).abs() <= 1, "got {frames} frames, expected {expected}");

    let aspec = RenderSpec { video: false, audio: true, ..spec };
    let (mut acmd, _ajob) = render::build(&p, &aspec, false).unwrap();
    acmd.args(["-map", "[aout]", "-f", "f32le", "-ac", "2", "-ar", "48000", "pipe:1"]);
    let out = acmd.stdout(Stdio::piped()).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let secs = out.stdout.len() as f64 / 8.0 / 48000.0;
    assert!((secs - (p.duration() - from)).abs() < 0.05, "audio length {secs}");
}

fn export_and_check(p: &Project, canvas: Canvas, name: &str) {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join(name);
    let mut last = 0.0f32;
    render::export(p, canvas, Quality::Small, &out, |pr| {
        assert!(pr.progress >= last - 1e-6, "progress went backwards");
        last = pr.progress;
        true
    })
    .unwrap();
    assert_eq!(last, 1.0);
    let v = probe_output(&out);
    let streams = v["streams"].as_array().unwrap();
    let video = streams.iter().find(|s| s["codec_type"] == "video").expect("video stream");
    assert!(streams.iter().any(|s| s["codec_type"] == "audio"), "audio stream");
    assert_eq!(video["width"].as_u64().unwrap() as u32, canvas.width);
    assert_eq!(video["height"].as_u64().unwrap() as u32, canvas.height);
    let dur: f64 = v["format"]["duration"].as_str().unwrap().parse().unwrap();
    assert!((dur - p.duration()).abs() < 0.15, "duration {dur} vs {}", p.duration());
    assert!(!dir.path().join(name.replace(".mp4", ".partial.mp4")).exists());
}

#[test]
fn exports_match_the_timeline() {
    let p = sample_project();
    export_and_check(&p, p.canvas, "landscape.mp4");
    export_and_check(&p, Canvas::from_preset(9, 16, 360, 24), "portrait.mp4");
    export_and_check(&p, Canvas::from_preset(1, 1, 240, 60), "square.mp4");
}

#[test]
fn export_can_be_cancelled_and_cleans_up() {
    let p = sample_project();
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("cancel.mp4");
    let err = render::export(&p, p.canvas, Quality::High, &out, |pr| pr.progress < 0.01).unwrap_err();
    assert_eq!(err.to_string(), "cancelled");
    assert!(!out.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0, "partial file left behind");
}

#[test]
fn export_rejects_bad_input() {
    let empty = Project::new("e", Canvas::default());
    let dir = tempfile::tempdir().unwrap();
    assert!(render::export(&empty, empty.canvas, Quality::Small, &dir.path().join("x.mp4"), |_| true).is_err());
    let p = sample_project();
    assert!(render::export(&p, p.canvas, Quality::Small, Path::new("Z:/no/such/folder/x.mp4"), |_| true).is_err());
}

/// Timing check for scrubbing responsiveness: `cargo test --release --test pipeline still_frame_latency -- --ignored --nocapture`
#[test]
#[ignore]
fn still_frame_latency() {
    let p = sample_project();
    let (w, h) = render::preview_size(&Canvas::from_preset(9, 16, 1080, 30), 960);
    for t in [0.5, 2.0, 3.5, 5.0, 6.0] {
        let start = std::time::Instant::now();
        render::render_frame(&p, t, w, h).unwrap();
        println!("frame at {t}: {} ms", start.elapsed().as_millis());
    }
}

#[test]
fn long_titles_shrink_to_fit_the_frame() {
    let long = "This title is far too long for a vertical frame";
    // 9:16 at 1080 wide: a 0.09 size title would be ~97px; it must shrink to fit.
    let px = render::fit_text_size(long, 97, 1080, true);
    assert!(px < 97 && px >= 20, "got {px}");
    // Short text keeps its size.
    assert_eq!(render::fit_text_size("Hi", 97, 1080, true), 97);
    // Multi-line text is measured by its longest line.
    assert_eq!(render::fit_text_size("Hi\nthere", 60, 1920, false), 60);
}

#[test]
fn speed_fades_color_and_text_fade_render_correctly() {
    let f = fixtures();
    let mut p = Project::new("fx", Canvas::from_preset(16, 9, 360, 30));
    let a = add(&mut p, &f.root.join("clip.mp4"));
    let fast = p.insert_main(a, None, None).unwrap();
    p.set_speed(fast, 2.0).unwrap(); // 4s of source in 2s
    let faded = p.insert_main(a, None, None).unwrap();
    p.set_fades(faded, Some(1.0), Some(1.0)).unwrap();
    p.set_color(faded, None, None, Some(0.0)).unwrap(); // black and white
    p.add_text(TextClip::new("Fading title", 0.0, 2.0)).unwrap();
    assert!((p.duration() - 6.0).abs() < 1e-9);

    let (w, h) = (192, 108);
    let first = render::render_frame(&p, 2.0, w, h).unwrap();
    let middle = render::render_frame(&p, 4.0, w, h).unwrap();
    assert!(mean_luma(&first) < 8.0, "fade-in should start from black, luma {}", mean_luma(&first));
    assert!(mean_luma(&middle) > 30.0, "middle of clip should be visible");
    let gray = middle.chunks_exact(4).all(|px| px[0].abs_diff(px[1]) < 12 && px[1].abs_diff(px[2]) < 12);
    assert!(gray, "saturation 0 should give a grayscale picture");
    // Half-way through the fade, partly visible: the preview from mid-fade matches.
    let half = mean_luma(&render::render_frame(&p, 2.5, w, h).unwrap());
    assert!(half > 8.0 && half < mean_luma(&middle), "mid-fade luma {half}");

    // Preview stream starting mid-clip at 2x speed yields the right frame count and audio length.
    let spec = RenderSpec { width: w, height: h, fps: 30, start: 1.0, end: 6.0, video: true, audio: false, preview: true };
    let (mut cmd, _j) = render::build(&p, &spec, true).unwrap();
    cmd.args(["-map", "[vout]", "-f", "rawvideo", "pipe:1"]);
    let out = cmd.stdout(Stdio::piped()).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let frames = out.stdout.len() / (w * h * 4) as usize;
    assert!((frames as i64 - 150).abs() <= 1, "frames {frames}");
    let aspec = RenderSpec { video: false, audio: true, ..spec };
    let (mut acmd, _aj) = render::build(&p, &aspec, false).unwrap();
    acmd.args(["-map", "[aout]", "-f", "f32le", "-ac", "2", "-ar", "48000", "pipe:1"]);
    let out = acmd.stdout(Stdio::piped()).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let secs = out.stdout.len() as f64 / 8.0 / 48000.0;
    assert!((secs - 5.0).abs() < 0.05, "audio {secs}");

    export_and_check(&p, p.canvas, "fx.mp4");
}
