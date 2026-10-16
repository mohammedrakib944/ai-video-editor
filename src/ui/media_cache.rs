//! Background thumbnails and waveforms for the media library and timeline.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use crossbeam_channel::{unbounded, Receiver, Sender};
use egui::{ColorImage, TextureHandle, TextureOptions};

use crate::media::{self, MediaKind};
use crate::project::{Asset, Id};

pub const THUMB_W: u32 = 160;
pub const THUMB_H: u32 = 90;
pub const WAVE_PER_SEC: u32 = 50;

pub struct Thumb {
    pub tex: TextureHandle,
    pub loaded: Instant,
}

#[derive(Default)]
pub struct AssetMedia {
    /// Evenly spaced frames across the media; index i is at (i + 0.5) / n of the duration.
    pub thumbs: Vec<Option<Thumb>>,
    /// Normalized peaks at WAVE_PER_SEC per second.
    pub wave: Option<Arc<Vec<f32>>>,
    pub failed: bool,
}

impl AssetMedia {
    /// Best available thumbnail for a source time.
    pub fn thumb_at(&self, t: f64, duration: f64) -> Option<&Thumb> {
        let n = self.thumbs.len();
        if n == 0 {
            return None;
        }
        let want = if duration > 0.0 { ((t / duration) * n as f64).floor().clamp(0.0, (n - 1) as f64) as usize } else { 0 };
        // Nearest loaded one, searching outwards.
        (0..n).flat_map(|d| [want.checked_sub(d), Some(want + d)]).flatten().filter(|&i| i < n).find_map(|i| self.thumbs[i].as_ref())
    }

    pub fn cover(&self) -> Option<&Thumb> {
        let n = self.thumbs.len();
        if n == 0 {
            return None;
        }
        self.thumbs[n / 3].as_ref().or_else(|| self.thumbs.iter().flatten().next())
    }
}

enum Job {
    Thumb { id: Id, index: usize, path: PathBuf, kind: MediaKind, t: f64 },
    Wave { id: Id, path: PathBuf },
}

enum Done {
    Thumb { id: Id, index: usize, rgba: Vec<u8> },
    Wave { id: Id, peaks: Vec<f32> },
    Failed { id: Id },
}

pub struct MediaCache {
    pub assets: HashMap<Id, AssetMedia>,
    requested: HashSet<Id>,
    jobs: Sender<Job>,
    done: Receiver<Done>,
}

pub fn thumb_count(asset: &Asset) -> usize {
    match asset.kind {
        MediaKind::Video => ((asset.duration / 3.0).ceil() as usize).clamp(1, 32),
        MediaKind::Image => 1,
        MediaKind::Audio => 0,
    }
}

impl MediaCache {
    pub fn new(repaint: Arc<dyn Fn() + Send + Sync>) -> MediaCache {
        let (job_tx, job_rx) = unbounded::<Job>();
        let (done_tx, done_rx) = unbounded::<Done>();
        for n in 0..2 {
            let rx = job_rx.clone();
            let tx = done_tx.clone();
            let repaint = repaint.clone();
            std::thread::Builder::new()
                .name(format!("thumbs-{n}"))
                .spawn(move || {
                    while let Ok(job) = rx.recv() {
                        let msg = match job {
                            Job::Thumb { id, index, path, kind, t } => match media::thumbnail(&path, kind, t, THUMB_W, THUMB_H) {
                                Ok(rgba) => Done::Thumb { id, index, rgba },
                                Err(_) => Done::Failed { id },
                            },
                            Job::Wave { id, path } => match media::waveform(&path, WAVE_PER_SEC) {
                                Ok(mut peaks) => {
                                    let max = peaks.iter().cloned().fold(0.0f32, f32::max).max(1e-3);
                                    for p in &mut peaks {
                                        *p = (*p / max).sqrt();
                                    }
                                    Done::Wave { id, peaks }
                                }
                                Err(_) => Done::Failed { id },
                            },
                        };
                        if tx.send(msg).is_err() {
                            break;
                        }
                        repaint();
                    }
                })
                .expect("spawn thumbnail worker");
        }
        MediaCache { assets: HashMap::new(), requested: HashSet::new(), jobs: job_tx, done: done_rx }
    }

    /// Queues work for assets we have not seen yet.
    pub fn ensure(&mut self, assets: &[Asset]) {
        for a in assets {
            if !self.requested.insert(a.id) {
                continue;
            }
            let n = thumb_count(a);
            self.assets.insert(a.id, AssetMedia { thumbs: (0..n).map(|_| None).collect(), wave: None, failed: false });
            // Cover first, then fill in the strip from both ends towards the middle.
            let mut order: Vec<usize> = (0..n).collect();
            if n > 0 {
                order.swap(0, n / 3);
            }
            for i in order {
                let t = (i as f64 + 0.5) / n as f64 * a.duration;
                let _ = self.jobs.send(Job::Thumb { id: a.id, index: i, path: a.path.clone(), kind: a.kind, t });
            }
            if a.has_audio {
                let _ = self.jobs.send(Job::Wave { id: a.id, path: a.path.clone() });
            }
        }
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(done) = self.done.try_recv() {
            match done {
                Done::Thumb { id, index, rgba } => {
                    let img = ColorImage::from_rgba_unmultiplied([THUMB_W as usize, THUMB_H as usize], &rgba);
                    let tex = ctx.load_texture(format!("thumb-{id}-{index}"), img, TextureOptions::LINEAR);
                    if let Some(m) = self.assets.get_mut(&id) {
                        if let Some(slot) = m.thumbs.get_mut(index) {
                            *slot = Some(Thumb { tex, loaded: Instant::now() });
                        }
                    }
                }
                Done::Wave { id, peaks } => {
                    if let Some(m) = self.assets.get_mut(&id) {
                        m.wave = Some(Arc::new(peaks));
                    }
                }
                Done::Failed { id } => {
                    if let Some(m) = self.assets.get_mut(&id) {
                        m.failed = true;
                    }
                }
            }
        }
    }

    pub fn get(&self, id: Id) -> Option<&AssetMedia> {
        self.assets.get(&id)
    }
}
