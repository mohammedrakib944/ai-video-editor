//! Audio during preview playback. FFmpeg mixes the project from the playhead
//! (the same graph as export) into a buffer that the audio device drains; the
//! number of samples played is the master clock the video follows.

use std::collections::VecDeque;
use std::io::Read;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::clip::Content;
use crate::ffmpeg;
use crate::model::Project;
use crate::render::audio;

/// Seconds of decoded audio kept ahead of the device.
const BUFFER_SECONDS: f64 = 1.5;

struct Shared {
    samples: Mutex<VecDeque<f32>>,
    /// Frames (samples per channel) actually played.
    played: AtomicU64,
    muted: AtomicBool,
    finished: AtomicBool,
    stop: AtomicBool,
}

pub struct AudioPlayer {
    _stream: cpal::Stream,
    shared: Arc<Shared>,
    child: Option<Child>,
    rate: u32,
    from: f64,
    opened: Instant,
    graph_file: std::path::PathBuf,
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        let _ = std::fs::remove_file(&self.graph_file);
    }
}

/// The project with everything before `t` cut away and the rest moved to start at 0.
fn shifted(project: &Project, t: f64) -> Project {
    let mut p = project.clone();
    p.clips.retain(|c| c.end() > t + 1e-6);
    for c in &mut p.clips {
        if c.start < t {
            let cut = t - c.start;
            if let Content::Media { src_in, speed, .. } = &mut c.content {
                *src_in += cut * *speed;
            }
            c.duration -= cut;
            c.audio.fade_in = (c.audio.fade_in - cut).max(0.0);
            c.start = 0.0;
        } else {
            c.start -= t;
        }
    }
    p
}

impl AudioPlayer {
    pub fn start(project: &Project, from: f64, muted: bool) -> anyhow::Result<AudioPlayer> {
        let host = cpal::default_host();
        let device = host.default_output_device().context("No audio output device")?;
        let config = device.default_output_config().context("The audio device has no usable format")?;
        let (rate, channels) = (config.sample_rate().0, config.channels() as usize);
        let total = project.duration() - from;
        if total <= 0.0 {
            bail!("Nothing to play");
        }

        let part = shifted(project, from);
        let (inputs, graph) = audio::mix_graph(&part, 0, total);
        if inputs.is_empty() {
            bail!("silent");
        }
        // FFmpeg reads this file after it starts, so it stays until playback stops.
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let graph_file = ffmpeg::work_dir().join(format!("play-{}-{}.txt", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::write(&graph_file, &graph)?;
        let mut cmd = ffmpeg::ffmpeg()?;
        for input in &inputs {
            cmd.args(["-ss", &ffmpeg::secs(input.seek), "-t", &ffmpeg::secs(input.length), "-i"]).arg(&input.path);
        }
        cmd.arg(ffmpeg::filter_script_option()).arg(&graph_file);
        cmd.args(["-map", "[aout]", "-f", "f32le", "-ac", &channels.to_string(), "-ar", &rate.to_string(), "pipe:1"]);
        let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::null()).spawn().context("could not start FFmpeg")?;
        let mut stdout = child.stdout.take().unwrap();

        let shared = Arc::new(Shared { samples: Mutex::new(VecDeque::new()), played: AtomicU64::new(0), muted: AtomicBool::new(muted), finished: AtomicBool::new(false), stop: AtomicBool::new(false) });
        let reader = shared.clone();
        let cap = (rate as f64 * BUFFER_SECONDS) as usize * channels;
        std::thread::Builder::new().name("audio-reader".into()).spawn(move || {
            let mut buf = vec![0u8; 4096 * 4 * channels];
            loop {
                if reader.stop.load(Ordering::Relaxed) {
                    break;
                }
                if reader.samples.lock().unwrap().len() > cap {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
                match stdout.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let n = n - n % 4;
                        let mut q = reader.samples.lock().unwrap();
                        q.extend(buf[..n].chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])));
                    }
                }
            }
            reader.finished.store(true, Ordering::Relaxed);
        })?;

        let out = shared.clone();
        let err = |e| eprintln!("audio stream error: {e}");
        let fill = move |data: &mut [f32]| {
            let mut q = out.samples.lock().unwrap();
            let muted = out.muted.load(Ordering::Relaxed);
            let available = q.len().min(data.len()) / channels * channels;
            for (i, s) in data.iter_mut().enumerate() {
                *s = if i < available { q.pop_front().map(|v| if muted { 0.0 } else { v }).unwrap_or(0.0) } else { 0.0 };
            }
            out.played.fetch_add((available / channels) as u64, Ordering::Relaxed);
        };
        let stream_config: cpal::StreamConfig = config.clone().into();
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_output_stream(&stream_config, move |d: &mut [f32], _| fill(d), err, None)?,
            cpal::SampleFormat::I16 => {
                let mut tmp = Vec::new();
                device.build_output_stream(
                    &stream_config,
                    move |d: &mut [i16], _| {
                        tmp.resize(d.len(), 0.0);
                        fill(&mut tmp);
                        for (o, v) in d.iter_mut().zip(&tmp) {
                            *o = (v.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                        }
                    },
                    err,
                    None,
                )?
            }
            cpal::SampleFormat::U16 => {
                let mut tmp = Vec::new();
                device.build_output_stream(
                    &stream_config,
                    move |d: &mut [u16], _| {
                        tmp.resize(d.len(), 0.0);
                        fill(&mut tmp);
                        for (o, v) in d.iter_mut().zip(&tmp) {
                            *o = ((v.clamp(-1.0, 1.0) * 0.5 + 0.5) * u16::MAX as f32) as u16;
                        }
                    },
                    err,
                    None,
                )?
            }
            other => bail!("Unsupported audio sample format {other:?}"),
        };
        stream.play().context("could not start audio playback")?;
        Ok(AudioPlayer { _stream: stream, shared, child: Some(child), rate, from, opened: Instant::now(), graph_file })
    }

    /// Timeline position by the audio clock, once sound has started.
    pub fn position(&self) -> Option<f64> {
        let played = self.shared.played.load(Ordering::Relaxed);
        (played > 0).then(|| self.from + played as f64 / self.rate as f64)
    }

    /// True if audio hasn't started for a while (then the caller should use a wall clock).
    pub fn stalled(&self) -> bool {
        self.shared.played.load(Ordering::Relaxed) == 0 && (self.opened.elapsed() > Duration::from_millis(600) || self.shared.finished.load(Ordering::Relaxed))
    }

    pub fn set_muted(&self, muted: bool) {
        self.shared.muted.store(muted, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::ProbeInfo;
    use crate::model::{Canvas, MediaKind};
    use std::path::PathBuf;

    #[test]
    fn shifting_trims_clips_that_started_earlier() {
        let mut p = Project::new("t", Canvas::default());
        let info = ProbeInfo { kind: MediaKind::Audio, duration: 10.0, width: 0, height: 0, fps: 0.0, has_audio: true };
        let a = p.add_asset(&PathBuf::from("/a.wav"), &info);
        let c1 = p.add_media_clip(a, None, 0.0).unwrap();
        let c2 = p.add_media_clip(a, None, 12.0).unwrap();
        let s = shifted(&p, 4.0);
        let (s1, s2) = (s.clip(c1).unwrap(), s.clip(c2).unwrap());
        assert_eq!((s1.start, s1.duration), (0.0, 6.0));
        assert!(matches!(s1.content, Content::Media { src_in, .. } if (src_in - 4.0).abs() < 1e-9));
        assert_eq!(s2.start, 8.0);
        assert!(shifted(&p, 11.0).clip(c1).is_none());
    }

    #[test]
    fn plays_muted_and_the_clock_advances() {
        let dir = std::env::temp_dir().join(format!("studio-play-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tone.wav");
        assert!(ffmpeg::ffmpeg().unwrap().args(["-f", "lavfi", "-i", "sine=frequency=440:duration=3", "-y"]).arg(&path).status().unwrap().success());
        let mut p = Project::new("t", Canvas::default());
        let a = p.add_asset(&path, &crate::media::probe(&path).unwrap());
        p.add_media_clip(a, None, 0.0).unwrap();
        let Ok(player) = AudioPlayer::start(&p, 1.0, true) else {
            eprintln!("no audio device; skipping");
            return;
        };
        std::thread::sleep(Duration::from_millis(900));
        let pos = player.position().expect("audio started");
        assert!(pos > 1.2 && pos < 2.0, "position {pos}");
        drop(player);
        std::fs::remove_dir_all(dir).ok();
    }
}
