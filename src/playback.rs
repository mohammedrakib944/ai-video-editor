//! Live preview: real-time playback with audio, and still frames for scrubbing.
//!
//! Playback runs two ffmpeg processes over the same timeline range: one
//! produces RGBA frames, the other PCM audio. The audio device is the master
//! clock, so picture and sound stay in sync even if decoding hiccups.

use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{bounded, unbounded, Receiver, Sender, TryRecvError};
use parking_lot::Mutex;

use crate::project::Project;
use crate::render::{self, RenderSpec};

pub const PREVIEW_LONG_SIDE: u32 = 960;

// ---------------------------------------------------------------------------
// Audio output
// ---------------------------------------------------------------------------

struct AudioShared {
    /// Interleaved stereo samples waiting to be played.
    buf: Mutex<VecDeque<f32>>,
    played_frames: AtomicU64,
    running: AtomicBool,
    muted: AtomicBool,
}

struct AudioOut {
    shared: Arc<AudioShared>,
    rate: u32,
    _stream: cpal::Stream,
}

impl AudioOut {
    fn open() -> anyhow::Result<AudioOut> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or_else(|| anyhow::anyhow!("no audio output device"))?;
        let config = device.default_output_config()?;
        let rate = config.sample_rate().0;
        let channels = config.channels() as usize;
        let shared = Arc::new(AudioShared {
            buf: Mutex::new(VecDeque::with_capacity(rate as usize * 4)),
            played_frames: AtomicU64::new(0),
            running: AtomicBool::new(false),
            muted: AtomicBool::new(false),
        });
        let s = shared.clone();
        let fill = move |out: &mut [f32]| {
            out.fill(0.0);
            if !s.running.load(Ordering::Acquire) {
                return;
            }
            let gain = if s.muted.load(Ordering::Relaxed) { 0.0 } else { 1.0 };
            let mut buf = s.buf.lock();
            let mut played = 0u64;
            for frame in out.chunks_mut(channels) {
                if buf.len() < 2 {
                    break; // Underrun: the clock waits for the decoder.
                }
                let l = buf.pop_front().unwrap() * gain;
                let r = buf.pop_front().unwrap() * gain;
                match frame.len() {
                    1 => frame[0] = (l + r) * 0.5,
                    _ => {
                        frame[0] = l;
                        frame[1] = r;
                    }
                }
                played += 1;
            }
            s.played_frames.fetch_add(played, Ordering::Release);
        };
        let err_fn = |e| eprintln!("audio stream error: {e}");
        let stream_config: cpal::StreamConfig = config.clone().into();
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_output_stream(&stream_config, move |d: &mut [f32], _| fill(d), err_fn, None)?,
            cpal::SampleFormat::I16 => {
                let mut tmp = Vec::new();
                device.build_output_stream(
                    &stream_config,
                    move |d: &mut [i16], _| {
                        tmp.resize(d.len(), 0.0);
                        fill(&mut tmp);
                        for (o, v) in d.iter_mut().zip(&tmp) {
                            *o = (v.clamp(-1.0, 1.0) * 32767.0) as i16;
                        }
                    },
                    err_fn,
                    None,
                )?
            }
            other => anyhow::bail!("unsupported audio format {other:?}"),
        };
        stream.play()?;
        Ok(AudioOut { shared, rate, _stream: stream })
    }

    fn reset(&self) {
        self.shared.running.store(false, Ordering::Release);
        self.shared.buf.lock().clear();
        self.shared.played_frames.store(0, Ordering::Release);
    }

    fn buffered_secs(&self) -> f64 {
        self.shared.buf.lock().len() as f64 / 2.0 / self.rate as f64
    }
}

// ---------------------------------------------------------------------------
// Playback session
// ---------------------------------------------------------------------------

struct Session {
    start_t: f64,
    end_t: f64,
    fps: f64,
    size: (u32, u32),
    children: Arc<Mutex<Vec<Child>>>,
    frames: Receiver<Vec<u8>>,
    next_index: u64,
    started: Option<Instant>,
    audio_done: Arc<AtomicBool>,
    stop_flag: Arc<AtomicBool>,
    video_done: bool,
}

impl Session {
    fn kill(&self) {
        self.stop_flag.store(true, Ordering::Release);
        for c in self.children.lock().iter_mut() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

pub struct Tick {
    pub time: f64,
    pub frame: Option<(Vec<u8>, (u32, u32))>,
    pub ended: bool,
}

/// A request for one still frame.
#[derive(Clone)]
struct StillJob {
    key: StillKey,
    t: f64,
    project: Arc<Project>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct StillKey {
    pub revision: u64,
    pub frame: i64,
    pub size: (u32, u32),
}

pub struct Still {
    pub key: StillKey,
    pub rgba: Result<Arc<Vec<u8>>, String>,
}

pub struct Preview {
    audio: Option<AudioOut>,
    pub audio_error: Option<String>,
    session: Option<Session>,
    still_tx: Sender<StillJob>,
    still_rx: Receiver<Still>,
    cache: HashMap<StillKey, Arc<Vec<u8>>>,
    cache_order: VecDeque<StillKey>,
    pending: Option<StillKey>,
    pub last_error: Option<String>,
}

impl Preview {
    pub fn new(repaint: impl Fn() + Send + Sync + 'static) -> Preview {
        let (audio, audio_error) = match AudioOut::open() {
            Ok(a) => (Some(a), None),
            Err(e) => (None, Some(e.to_string())),
        };
        let (job_tx, job_rx) = unbounded::<StillJob>();
        let (res_tx, res_rx) = unbounded::<Still>();
        std::thread::Builder::new()
            .name("still-frames".into())
            .spawn(move || {
                while let Ok(mut job) = job_rx.recv() {
                    // Latest request wins: skip anything superseded while scrubbing.
                    while let Ok(newer) = job_rx.try_recv() {
                        job = newer;
                    }
                    let (w, h) = job.key.size;
                    let rgba = render::render_frame(&job.project, job.t, w, h).map(Arc::new).map_err(|e| e.to_string());
                    if res_tx.send(Still { key: job.key, rgba }).is_err() {
                        break;
                    }
                    repaint();
                }
            })
            .expect("spawn still-frame thread");
        Preview {
            audio,
            audio_error,
            session: None,
            still_tx: job_tx,
            still_rx: res_rx,
            cache: HashMap::new(),
            cache_order: VecDeque::new(),
            pending: None,
            last_error: None,
        }
    }

    pub fn is_playing(&self) -> bool {
        self.session.is_some()
    }

    /// Asks for the frame at `t`; returns it right away if cached.
    pub fn request_still(&mut self, project: &Arc<Project>, revision: u64, t: f64, size: (u32, u32)) -> Option<Arc<Vec<u8>>> {
        let frame = (t * project.canvas.fps as f64).floor() as i64;
        let key = StillKey { revision, frame, size };
        if let Some(f) = self.cache.get(&key) {
            return Some(f.clone());
        }
        if self.pending != Some(key) {
            self.pending = Some(key);
            let _ = self.still_tx.send(StillJob { key, t: frame as f64 / project.canvas.fps as f64, project: project.clone() });
        }
        None
    }

    /// Collects finished still frames. Returns the newest one.
    pub fn poll_stills(&mut self) -> Option<(StillKey, Arc<Vec<u8>>)> {
        let mut newest = None;
        while let Ok(still) = self.still_rx.try_recv() {
            if self.pending == Some(still.key) {
                self.pending = None;
            }
            match still.rgba {
                Ok(rgba) => {
                    self.last_error = None;
                    self.cache.insert(still.key, rgba.clone());
                    self.cache_order.push_back(still.key);
                    while self.cache_order.len() > 90 {
                        if let Some(old) = self.cache_order.pop_front() {
                            self.cache.remove(&old);
                        }
                    }
                    newest = Some((still.key, rgba));
                }
                Err(e) => self.last_error = Some(e),
            }
        }
        newest
    }

    pub fn is_rendering_still(&self) -> bool {
        self.pending.is_some()
    }

    pub fn set_muted(&self, muted: bool) {
        if let Some(a) = &self.audio {
            a.shared.muted.store(muted, Ordering::Relaxed);
        }
    }

    pub fn play(&mut self, project: &Project, from: f64, long_side: u32, repaint: Arc<dyn Fn() + Send + Sync>) -> anyhow::Result<()> {
        self.stop();
        let total = project.duration();
        if total <= 0.0 {
            anyhow::bail!("Add a clip to the timeline first");
        }
        let from = if from >= total - project.canvas.frame() { 0.0 } else { from.max(0.0) };
        let size = render::preview_size(&project.canvas, long_side);
        let fps = project.canvas.fps;
        let spec = RenderSpec { width: size.0, height: size.1, fps, start: from, end: total, video: true, audio: false, preview: true };
        let (mut vcmd, vjob) = render::build(project, &spec, true)?;
        vcmd.args(["-map", "[vout]", "-f", "rawvideo", "pipe:1"]);
        let mut vchild = vcmd.stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
        let mut vout = vchild.stdout.take().unwrap();

        let stop_flag = Arc::new(AtomicBool::new(false));
        let children = Arc::new(Mutex::new(vec![vchild]));
        let (ftx, frx) = bounded::<Vec<u8>>(6);
        let frame_len = (size.0 * size.1 * 4) as usize;
        {
            let repaint = repaint.clone();
            std::thread::spawn(move || {
                let _keep = vjob;
                loop {
                    let mut buf = vec![0u8; frame_len];
                    if vout.read_exact(&mut buf).is_err() {
                        break;
                    }
                    if ftx.send(buf).is_err() {
                        break;
                    }
                    repaint();
                }
            });
        }

        let audio_done = Arc::new(AtomicBool::new(true));
        if let Some(audio) = &self.audio {
            audio.reset();
            let aspec = RenderSpec { video: false, audio: true, ..spec };
            let (mut acmd, ajob) = render::build(project, &aspec, false)?;
            acmd.args(["-map", "[aout]", "-f", "f32le", "-ac", "2", "-ar", &audio.rate.to_string(), "pipe:1"]);
            let mut achild = acmd.stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
            let mut aout = achild.stdout.take().unwrap();
            children.lock().push(achild);
            audio_done.store(false, Ordering::Release);
            let shared = audio.shared.clone();
            let done = audio_done.clone();
            let stop = stop_flag.clone();
            let max_buffered = audio.rate as usize * 2 * 2; // two seconds of stereo
            std::thread::spawn(move || {
                let _keep = ajob;
                let mut bytes = vec![0u8; 16 * 1024];
                let mut carry: Vec<u8> = Vec::new();
                loop {
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    while shared.buf.lock().len() > max_buffered {
                        if stop.load(Ordering::Acquire) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    let n = match aout.read(&mut bytes) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    carry.extend_from_slice(&bytes[..n]);
                    let usable = carry.len() / 4 * 4;
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    let mut buf = shared.buf.lock();
                    for c in carry[..usable].chunks_exact(4) {
                        buf.push_back(f32::from_le_bytes([c[0], c[1], c[2], c[3]]));
                    }
                    drop(buf);
                    carry.drain(..usable);
                }
                done.store(true, Ordering::Release);
            });
        }

        self.session = Some(Session {
            start_t: from,
            end_t: total,
            fps: fps as f64,
            size,
            children,
            frames: frx,
            next_index: 0,
            started: None,
            audio_done,
            stop_flag,
            video_done: false,
        });
        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some(s) = self.session.take() {
            s.kill();
        }
        if let Some(a) = &self.audio {
            a.reset();
        }
    }

    fn elapsed(&self, s: &Session) -> f64 {
        match (&self.audio, s.started) {
            (_, None) => 0.0,
            (Some(a), Some(_)) if !s.audio_done.load(Ordering::Acquire) || a.buffered_secs() > 0.0 => {
                a.shared.played_frames.load(Ordering::Acquire) as f64 / a.rate as f64
            }
            (Some(a), Some(started)) => {
                // Audio finished early (should not happen); continue on the wall clock.
                let played = a.shared.played_frames.load(Ordering::Acquire) as f64 / a.rate as f64;
                played.max(started.elapsed().as_secs_f64())
            }
            (None, Some(started)) => started.elapsed().as_secs_f64(),
        }
    }

    /// Advances playback; call every UI frame while playing.
    pub fn tick(&mut self) -> Option<Tick> {
        let mut s = self.session.take()?;
        let mut frame = None;

        if s.started.is_none() {
            // Wait until the first frame is decoded and some audio is buffered.
            let audio_ready = match &self.audio {
                Some(a) => a.buffered_secs() >= 0.12 || s.audio_done.load(Ordering::Acquire),
                None => true,
            };
            if audio_ready && !s.frames.is_empty() {
                s.started = Some(Instant::now());
                if let Some(a) = &self.audio {
                    a.shared.running.store(true, Ordering::Release);
                }
            } else if s.frames.is_empty() && s.children.lock().first_mut().is_some_and(|c| matches!(c.try_wait(), Ok(Some(_)))) {
                // Video process exited without producing anything.
                s.kill();
                self.last_error = Some("Playback could not start".into());
                return Some(Tick { time: s.start_t, frame: None, ended: true });
            }
        }

        let elapsed = self.elapsed(&s);
        let target = (elapsed * s.fps).floor() as u64;
        while s.next_index <= target {
            match s.frames.try_recv() {
                Ok(f) => {
                    frame = Some((f, s.size));
                    s.next_index += 1;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    s.video_done = true;
                    break;
                }
            }
        }
        let time = (s.start_t + elapsed).min(s.end_t);
        let ended = time >= s.end_t - 1e-3 || (s.video_done && s.frames.is_empty() && elapsed > 0.0 && target >= s.next_index + 2);
        if ended {
            s.kill();
            if let Some(a) = &self.audio {
                a.reset();
            }
            return Some(Tick { time: s.end_t.min(time.max(s.start_t)), frame, ended: true });
        }
        self.session = Some(s);
        Some(Tick { time, frame, ended: false })
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        self.stop();
    }
}
