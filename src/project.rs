//! Project model and every editing operation (shared by the UI and the MCP server).
//!
//! The main track is magnetic: clips play back to back, so there are never gaps
//! or overlaps to get wrong. Music and text clips sit at explicit times on their
//! own tracks. The main track defines the length of the video.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{anyhow, bail, Context};
use serde::{Deserialize, Serialize};

use crate::media::{MediaKind, ProbeInfo};

pub type Id = u64;

pub const MIN_CLIP: f64 = 0.1;
pub const DEFAULT_IMAGE_SECS: f64 = 5.0;
const EPS: f64 = 1e-6;

// ---------------------------------------------------------------------------
// Canvas presets
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

impl Default for Canvas {
    fn default() -> Self {
        Canvas { width: 1920, height: 1080, fps: 30 }
    }
}

pub const ASPECTS: &[(&str, u32, u32, &str)] = &[
    ("16:9", 16, 9, "YouTube"),
    ("9:16", 9, 16, "Shorts · Reels · TikTok"),
    ("1:1", 1, 1, "Square"),
    ("4:5", 4, 5, "Instagram post"),
    ("21:9", 21, 9, "Cinematic"),
];

/// Resolution presets named by the length of the short side.
pub const RESOLUTIONS: &[(&str, u32)] = &[("720p", 720), ("1080p", 1080), ("1440p", 1440), ("4K", 2160)];
pub const FRAME_RATES: &[u32] = &[24, 25, 30, 60];

fn even(v: f64) -> u32 {
    let v = v.round().max(2.0) as u32;
    v + (v % 2)
}

impl Canvas {
    /// Builds a canvas from an aspect ratio and short-side resolution.
    pub fn from_preset(aw: u32, ah: u32, short_side: u32, fps: u32) -> Canvas {
        let (width, height) = if aw >= ah {
            (even(short_side as f64 * aw as f64 / ah as f64), even(short_side as f64))
        } else {
            (even(short_side as f64), even(short_side as f64 * ah as f64 / aw as f64))
        };
        Canvas { width, height, fps }
    }

    pub fn parse_aspect(s: &str) -> anyhow::Result<(u32, u32)> {
        let s = s.trim();
        let (a, b) = s
            .split_once(':')
            .or_else(|| s.split_once('x'))
            .or_else(|| s.split_once('/'))
            .ok_or_else(|| anyhow!("Aspect ratio must look like \"16:9\""))?;
        let a: u32 = a.trim().parse().map_err(|_| anyhow!("Bad aspect ratio \"{s}\""))?;
        let b: u32 = b.trim().parse().map_err(|_| anyhow!("Bad aspect ratio \"{s}\""))?;
        if a == 0 || b == 0 || a > 100 || b > 100 {
            bail!("Bad aspect ratio \"{s}\"");
        }
        Ok((a, b))
    }

    pub fn parse_resolution(s: &str) -> anyhow::Result<u32> {
        let s = s.trim().to_ascii_lowercase();
        if let Some((_, v)) = RESOLUTIONS.iter().find(|(name, _)| name.to_ascii_lowercase() == s) {
            return Ok(*v);
        }
        let v: u32 = s
            .trim_end_matches('p')
            .parse()
            .map_err(|_| anyhow!("Resolution must be one of 720p, 1080p, 1440p, 4K"))?;
        if !(144..=4320).contains(&v) {
            bail!("Resolution {v}p is out of range");
        }
        Ok(v)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.width < 16 || self.height < 16 || self.width > 7680 || self.height > 7680 {
            bail!("Canvas size {}x{} is out of range (16..7680)", self.width, self.height);
        }
        if self.width % 2 != 0 || self.height % 2 != 0 {
            bail!("Canvas width and height must be even numbers");
        }
        if !(1..=120).contains(&self.fps) {
            bail!("Frame rate must be between 1 and 120");
        }
        Ok(())
    }

    pub fn aspect_label(&self) -> String {
        fn gcd(a: u32, b: u32) -> u32 {
            if b == 0 {
                a
            } else {
                gcd(b, a % b)
            }
        }
        for (name, aw, ah, _) in ASPECTS {
            if (self.width as u64 * *ah as u64).abs_diff(self.height as u64 * *aw as u64) <= (*ah.max(aw) as u64) * 2 {
                return name.to_string();
            }
        }
        let g = gcd(self.width, self.height).max(1);
        format!("{}:{}", self.width / g, self.height / g)
    }

    pub fn resolution_label(&self) -> String {
        let short = self.width.min(self.height);
        RESOLUTIONS
            .iter()
            .find(|(_, v)| *v == short)
            .map(|(n, _)| n.to_string())
            .unwrap_or_else(|| format!("{}x{}", self.width, self.height))
    }

    pub fn summary(&self) -> String {
        format!("{} · {} · {} fps", self.aspect_label(), self.resolution_label(), self.fps)
    }

    pub fn frame(&self) -> f64 {
        1.0 / self.fps.max(1) as f64
    }

    /// Rounds a time to the nearest frame boundary.
    pub fn snap(&self, t: f64) -> f64 {
        let fps = self.fps.max(1) as f64;
        (t * fps).round() / fps
    }
}

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Asset {
    pub id: Id,
    pub path: PathBuf,
    pub name: String,
    pub kind: MediaKind,
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub has_audio: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fit {
    /// Whole picture visible, black bars if needed.
    #[default]
    Fit,
    /// Fills the frame, cropping edges.
    Fill,
}

/// A clip on the magnetic main track.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: Id,
    pub asset_id: Id,
    /// Source in/out points in seconds. For images, `src_in` is 0 and `src_out` is the duration.
    pub src_in: f64,
    pub src_out: f64,
    pub volume: f32,
    pub fit: Fit,
    /// Playback speed (0.25..4). The clip lasts `(src_out - src_in) / speed` on the timeline.
    #[serde(default = "one")]
    pub speed: f64,
    /// Fade from/to black (and silence), in timeline seconds.
    #[serde(default)]
    pub fade_in: f64,
    #[serde(default)]
    pub fade_out: f64,
    /// Color: brightness -1..1 (0 = unchanged), contrast and saturation 0..2 (1 = unchanged).
    #[serde(default)]
    pub brightness: f32,
    #[serde(default = "one_f32")]
    pub contrast: f32,
    #[serde(default = "one_f32")]
    pub saturation: f32,
}

fn one() -> f64 {
    1.0
}
fn one_f32() -> f32 {
    1.0
}

pub const MIN_SPEED: f64 = 0.25;
pub const MAX_SPEED: f64 = 4.0;

impl Clip {
    pub fn new(id: Id, asset_id: Id, src_in: f64, src_out: f64) -> Clip {
        Clip { id, asset_id, src_in, src_out, volume: 1.0, fit: Fit::Fit, speed: 1.0, fade_in: 0.0, fade_out: 0.0, brightness: 0.0, contrast: 1.0, saturation: 1.0 }
    }

    /// Length on the timeline.
    pub fn duration(&self) -> f64 {
        (self.src_out - self.src_in) / self.speed
    }

    /// Converts a timeline offset into this clip to a source offset.
    pub fn to_src(&self, timeline_offset: f64) -> f64 {
        timeline_offset * self.speed
    }

    pub fn has_color(&self) -> bool {
        self.brightness.abs() > 1e-3 || (self.contrast - 1.0).abs() > 1e-3 || (self.saturation - 1.0).abs() > 1e-3
    }
}

/// A clip on the music track, placed at an absolute timeline time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioClip {
    pub id: Id,
    pub asset_id: Id,
    pub start: f64,
    pub src_in: f64,
    pub src_out: f64,
    pub volume: f32,
    #[serde(default)]
    pub fade_in: f64,
    #[serde(default)]
    pub fade_out: f64,
}

impl AudioClip {
    pub fn duration(&self) -> f64 {
        self.src_out - self.src_in
    }
    pub fn end(&self) -> f64 {
        self.start + self.duration()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextClip {
    pub id: Id,
    pub text: String,
    pub start: f64,
    pub duration: f64,
    /// Center of the text, as a fraction of the frame (0..1).
    pub x: f32,
    pub y: f32,
    /// Font size as a fraction of the frame's shorter side (auto-shrinks to fit the width).
    pub size: f32,
    pub color: [u8; 3],
    pub background: bool,
    /// Fade the text in and out.
    #[serde(default)]
    pub fade: bool,
}

impl TextClip {
    pub fn end(&self) -> f64 {
        self.start + self.duration
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub format_version: u32,
    pub name: String,
    pub canvas: Canvas,
    pub assets: Vec<Asset>,
    pub main: Vec<Clip>,
    pub music: Vec<AudioClip>,
    pub texts: Vec<TextClip>,
    pub next_id: Id,
}

impl Default for Project {
    fn default() -> Self {
        Project::new("Untitled", Canvas::default())
    }
}

/// Which track a clip id lives on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Track {
    Main,
    Music,
    Text,
}

impl Project {
    pub fn new(name: &str, canvas: Canvas) -> Project {
        Project {
            format_version: 1,
            name: name.to_string(),
            canvas,
            assets: Vec::new(),
            main: Vec::new(),
            music: Vec::new(),
            texts: Vec::new(),
            next_id: 1,
        }
    }

    fn alloc_id(&mut self) -> Id {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn asset(&self, id: Id) -> Option<&Asset> {
        self.assets.iter().find(|a| a.id == id)
    }

    pub fn duration(&self) -> f64 {
        self.main.iter().map(Clip::duration).sum()
    }

    /// Timeline start of each main-track clip.
    pub fn main_starts(&self) -> Vec<f64> {
        let mut t = 0.0;
        self.main
            .iter()
            .map(|c| {
                let s = t;
                t += c.duration();
                s
            })
            .collect()
    }

    pub fn main_index(&self, id: Id) -> Option<usize> {
        self.main.iter().position(|c| c.id == id)
    }

    pub fn track_of(&self, id: Id) -> Option<Track> {
        if self.main.iter().any(|c| c.id == id) {
            Some(Track::Main)
        } else if self.music.iter().any(|c| c.id == id) {
            Some(Track::Music)
        } else if self.texts.iter().any(|c| c.id == id) {
            Some(Track::Text)
        } else {
            None
        }
    }

    /// Main-track clip at timeline time `t`, with the offset into it.
    pub fn main_clip_at(&self, t: f64) -> Option<(usize, f64)> {
        let mut start = 0.0;
        for (i, c) in self.main.iter().enumerate() {
            let end = start + c.duration();
            if t >= start - EPS && t < end - EPS {
                return Some((i, (t - start).max(0.0)));
            }
            start = end;
        }
        None
    }

    fn asset_checked(&self, id: Id) -> anyhow::Result<&Asset> {
        self.asset(id).ok_or_else(|| anyhow!("No media with id {id}"))
    }

    // ----- Assets ---------------------------------------------------------

    pub fn add_asset(&mut self, path: &Path, info: &ProbeInfo) -> Id {
        if let Some(existing) = self.assets.iter().find(|a| a.path == path) {
            return existing.id;
        }
        let id = self.alloc_id();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "media".into());
        self.assets.push(Asset {
            id,
            path: path.to_path_buf(),
            name,
            kind: info.kind,
            duration: info.duration,
            width: info.width,
            height: info.height,
            fps: info.fps,
            has_audio: info.has_audio,
        });
        id
    }

    /// Removes media from the library along with every clip that uses it.
    pub fn remove_asset(&mut self, id: Id) -> anyhow::Result<()> {
        self.asset_checked(id)?;
        self.main.retain(|c| c.asset_id != id);
        self.music.retain(|c| c.asset_id != id);
        self.assets.retain(|a| a.id != id);
        Ok(())
    }

    // ----- Main track -------------------------------------------------------

    /// Inserts a video or image on the main track. `index` defaults to the end.
    pub fn insert_main(&mut self, asset_id: Id, index: Option<usize>, src: Option<(f64, f64)>) -> anyhow::Result<Id> {
        let asset = self.asset_checked(asset_id)?.clone();
        let (src_in, src_out) = match asset.kind {
            MediaKind::Audio => bail!("\"{}\" is audio; add it to the music track instead", asset.name),
            MediaKind::Image => (0.0, src.map(|(a, b)| b - a).unwrap_or(DEFAULT_IMAGE_SECS)),
            MediaKind::Video => src.unwrap_or((0.0, asset.duration)),
        };
        let (src_in, src_out) = self.validate_src(&asset, src_in, src_out)?;
        let index = index.unwrap_or(self.main.len()).min(self.main.len());
        let id = self.alloc_id();
        self.main.insert(index, Clip::new(id, asset_id, src_in, src_out));
        Ok(id)
    }

    fn validate_src(&self, asset: &Asset, src_in: f64, src_out: f64) -> anyhow::Result<(f64, f64)> {
        if !src_in.is_finite() || !src_out.is_finite() {
            bail!("Times must be numbers");
        }
        if asset.kind == MediaKind::Image {
            let d = (src_out - src_in).clamp(MIN_CLIP, 3600.0);
            return Ok((0.0, d));
        }
        let src_in = src_in.clamp(0.0, asset.duration);
        let src_out = src_out.clamp(0.0, asset.duration);
        if src_out - src_in < MIN_CLIP - EPS {
            bail!(
                "Clip would be shorter than {MIN_CLIP}s (in {src_in:.3}s, out {src_out:.3}s, media length {:.3}s)",
                asset.duration
            );
        }
        Ok((src_in, src_out))
    }

    /// Splits the main-track clip under timeline time `t`. Returns the new right-hand clip id.
    pub fn split_at(&mut self, t: f64) -> anyhow::Result<Id> {
        let t = self.canvas.snap(t);
        let (i, offset) = self.main_clip_at(t).ok_or_else(|| anyhow!("No clip at {t:.3}s to split"))?;
        let clip = &self.main[i];
        if offset < MIN_CLIP - EPS || clip.duration() - offset < MIN_CLIP - EPS {
            bail!("Too close to the edge of a clip to split at {t:.3}s");
        }
        let cut = clip.src_in + clip.to_src(offset);
        let mut right = clip.clone();
        right.id = self.alloc_id();
        right.src_in = cut;
        self.main[i].src_out = cut;
        self.main.insert(i + 1, right.clone());
        Ok(right.id)
    }

    /// Changes the source in/out of a main or music clip, or the start/duration of a text.
    pub fn trim_main(&mut self, id: Id, src_in: f64, src_out: f64) -> anyhow::Result<()> {
        let i = self.main_index(id).ok_or_else(|| anyhow!("No main-track clip with id {id}"))?;
        let asset = self.asset_checked(self.main[i].asset_id)?.clone();
        let (a, b) = self.validate_src(&asset, src_in, src_out)?;
        self.main[i].src_in = a;
        self.main[i].src_out = b;
        Ok(())
    }

    pub fn move_main(&mut self, id: Id, to_index: usize) -> anyhow::Result<()> {
        let i = self.main_index(id).ok_or_else(|| anyhow!("No main-track clip with id {id}"))?;
        let clip = self.main.remove(i);
        let to = to_index.min(self.main.len());
        self.main.insert(to, clip);
        Ok(())
    }

    /// Deletes timeline time `[a, b)` from the main track and closes the gap.
    /// Text clips follow the video they belong to; music stays where it is.
    pub fn delete_range(&mut self, a: f64, b: f64) -> anyhow::Result<()> {
        let total = self.duration();
        let a = self.canvas.snap(a.max(0.0));
        let b = self.canvas.snap(b.min(total));
        if b - a < self.canvas.frame() / 2.0 {
            bail!("Nothing to delete between {a:.3}s and {b:.3}s");
        }
        let mut kept = Vec::new();
        let mut start = 0.0;
        for c in &self.main {
            let end = start + c.duration();
            // Part before the range.
            if start < a - EPS {
                let mut left = c.clone();
                left.src_out = c.src_in + c.to_src(end.min(a) - start);
                kept.push(left);
            }
            // Part after the range.
            if end > b + EPS {
                let mut right = c.clone();
                right.src_in = c.src_in + c.to_src(b.max(start) - start);
                kept.push(right);
            }
            start = end;
        }
        // A clip that straddles the range becomes two pieces; give the second a fresh id.
        let mut seen = std::collections::HashSet::new();
        for c in &mut kept {
            if !seen.insert(c.id) {
                c.id = self.next_id;
                self.next_id += 1;
            }
        }
        // Drop slivers shorter than a frame left by rounding.
        let frame = self.canvas.frame();
        kept.retain(|c| c.duration() >= frame / 2.0);
        self.main = kept;
        self.ripple_texts(a, b);
        Ok(())
    }

    fn ripple_texts(&mut self, a: f64, b: f64) {
        let cut = b - a;
        let mut out = Vec::new();
        for mut t in self.texts.drain(..) {
            let (s, e) = (t.start, t.end());
            if e <= a + EPS {
                out.push(t);
            } else if s >= b - EPS {
                t.start -= cut;
                out.push(t);
            } else {
                // Overlaps the cut: keep whatever lies outside it.
                let before = (a - s).max(0.0);
                let after = (e - b).max(0.0);
                let d = before + after;
                if d >= MIN_CLIP {
                    t.start = s.min(a);
                    t.duration = d;
                    out.push(t);
                }
            }
        }
        self.texts = out;
    }

    // ----- Music track ------------------------------------------------------

    pub fn add_music(&mut self, asset_id: Id, start: f64, src: Option<(f64, f64)>) -> anyhow::Result<Id> {
        let asset = self.asset_checked(asset_id)?.clone();
        if !asset.has_audio {
            bail!("\"{}\" has no audio", asset.name);
        }
        let (src_in, src_out) = src.unwrap_or((0.0, asset.duration));
        let (src_in, src_out) = self.validate_src(&asset, src_in, src_out)?;
        let id = self.alloc_id();
        self.music.push(AudioClip { id, asset_id, start: start.max(0.0), src_in, src_out, volume: 1.0, fade_in: 0.0, fade_out: 0.0 });
        Ok(id)
    }

    pub fn update_music(&mut self, id: Id, start: Option<f64>, src: Option<(f64, f64)>, volume: Option<f32>) -> anyhow::Result<()> {
        let i = self.music.iter().position(|c| c.id == id).ok_or_else(|| anyhow!("No music clip with id {id}"))?;
        let asset = self.asset_checked(self.music[i].asset_id)?.clone();
        if let Some((a, b)) = src {
            let (a, b) = self.validate_src(&asset, a, b)?;
            self.music[i].src_in = a;
            self.music[i].src_out = b;
        }
        if let Some(s) = start {
            self.music[i].start = s.max(0.0);
        }
        if let Some(v) = volume {
            self.music[i].volume = v.clamp(0.0, 2.0);
        }
        Ok(())
    }

    // ----- Text track -------------------------------------------------------

    pub fn add_text(&mut self, mut text: TextClip) -> anyhow::Result<Id> {
        validate_text(&text)?;
        text.id = self.alloc_id();
        let id = text.id;
        self.texts.push(text);
        Ok(id)
    }

    pub fn text_mut(&mut self, id: Id) -> anyhow::Result<&mut TextClip> {
        self.texts.iter_mut().find(|t| t.id == id).ok_or_else(|| anyhow!("No text clip with id {id}"))
    }

    // ----- Shared -----------------------------------------------------------

    pub fn delete_clip(&mut self, id: Id) -> anyhow::Result<()> {
        match self.track_of(id) {
            Some(Track::Main) => {
                let i = self.main_index(id).unwrap();
                let start = self.main_starts()[i];
                let end = start + self.main[i].duration();
                self.main.remove(i);
                self.ripple_texts(start, end);
            }
            Some(Track::Music) => self.music.retain(|c| c.id != id),
            Some(Track::Text) => self.texts.retain(|c| c.id != id),
            None => bail!("No clip with id {id}"),
        }
        Ok(())
    }

    pub fn set_volume(&mut self, id: Id, volume: f32) -> anyhow::Result<()> {
        let v = volume.clamp(0.0, 2.0);
        if let Some(c) = self.main.iter_mut().find(|c| c.id == id) {
            c.volume = v;
        } else if let Some(c) = self.music.iter_mut().find(|c| c.id == id) {
            c.volume = v;
        } else {
            bail!("No audio-capable clip with id {id}");
        }
        Ok(())
    }

    pub fn set_fit(&mut self, id: Id, fit: Fit) -> anyhow::Result<()> {
        let c = self.main.iter_mut().find(|c| c.id == id).ok_or_else(|| anyhow!("No main-track clip with id {id}"))?;
        c.fit = fit;
        Ok(())
    }

    /// Changes playback speed of a main-track video clip (the clip gets shorter or longer).
    pub fn set_speed(&mut self, id: Id, speed: f64) -> anyhow::Result<()> {
        if !speed.is_finite() {
            bail!("Speed must be a number");
        }
        let i = self.main_index(id).ok_or_else(|| anyhow!("No main-track clip with id {id}"))?;
        let kind = self.asset(self.main[i].asset_id).map(|a| a.kind);
        if kind == Some(MediaKind::Image) {
            bail!("Images have no speed; change their duration instead");
        }
        let c = &mut self.main[i];
        c.speed = speed.clamp(MIN_SPEED, MAX_SPEED);
        let max_fade = c.duration() / 2.0;
        c.fade_in = c.fade_in.min(max_fade);
        c.fade_out = c.fade_out.min(max_fade);
        Ok(())
    }

    /// Sets fade in/out (seconds) on a main or music clip. `None` leaves a side unchanged.
    pub fn set_fades(&mut self, id: Id, fade_in: Option<f64>, fade_out: Option<f64>) -> anyhow::Result<()> {
        let clean = |v: f64, max: f64| if v.is_finite() { v.clamp(0.0, max.max(0.0)) } else { 0.0 };
        if let Some(c) = self.main.iter_mut().find(|c| c.id == id) {
            let max = c.duration() / 2.0;
            if let Some(v) = fade_in {
                c.fade_in = clean(v, max);
            }
            if let Some(v) = fade_out {
                c.fade_out = clean(v, max);
            }
        } else if let Some(c) = self.music.iter_mut().find(|c| c.id == id) {
            let max = c.duration() / 2.0;
            if let Some(v) = fade_in {
                c.fade_in = clean(v, max);
            }
            if let Some(v) = fade_out {
                c.fade_out = clean(v, max);
            }
        } else {
            bail!("No video or music clip with id {id}");
        }
        Ok(())
    }

    /// Color adjustments on a main-track clip. `None` leaves a value unchanged.
    pub fn set_color(&mut self, id: Id, brightness: Option<f32>, contrast: Option<f32>, saturation: Option<f32>) -> anyhow::Result<()> {
        let c = self.main.iter_mut().find(|c| c.id == id).ok_or_else(|| anyhow!("No main-track clip with id {id}"))?;
        if let Some(v) = brightness {
            c.brightness = v.clamp(-1.0, 1.0);
        }
        if let Some(v) = contrast {
            c.contrast = v.clamp(0.0, 2.0);
        }
        if let Some(v) = saturation {
            c.saturation = v.clamp(0.0, 2.0);
        }
        Ok(())
    }

    /// Copies a clip and places the copy right after the original. Returns the new id.
    pub fn duplicate_clip(&mut self, id: Id) -> anyhow::Result<Id> {
        let new_id = self.alloc_id();
        match self.track_of(id) {
            Some(Track::Main) => {
                let i = self.main_index(id).unwrap();
                let mut c = self.main[i].clone();
                c.id = new_id;
                self.main.insert(i + 1, c);
            }
            Some(Track::Music) => {
                let mut c = self.music.iter().find(|c| c.id == id).unwrap().clone();
                c.id = new_id;
                c.start = c.end();
                self.music.push(c);
            }
            Some(Track::Text) => {
                let mut t = self.texts.iter().find(|t| t.id == id).unwrap().clone();
                t.id = new_id;
                t.start = t.end();
                self.texts.push(t);
            }
            None => bail!("No clip with id {id}"),
        }
        Ok(new_id)
    }

    pub fn set_canvas(&mut self, canvas: Canvas) -> anyhow::Result<()> {
        canvas.validate()?;
        self.canvas = canvas;
        Ok(())
    }

    // ----- Files ------------------------------------------------------------

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        let tmp = path.with_extension("aive.tmp");
        std::fs::write(&tmp, json).with_context(|| format!("could not write {}", tmp.display()))?;
        std::fs::rename(&tmp, path).with_context(|| format!("could not save {}", path.display()))?;
        Ok(())
    }

    pub fn load(path: &Path) -> anyhow::Result<Project> {
        let data = std::fs::read_to_string(path).with_context(|| format!("could not open {}", path.display()))?;
        let mut p: Project = serde_json::from_str(&data).context("this is not a valid project file")?;
        p.canvas.validate()?;
        // Keep ids unique even if the file was edited by hand.
        let max_id = p
            .assets
            .iter()
            .map(|a| a.id)
            .chain(p.main.iter().map(|c| c.id))
            .chain(p.music.iter().map(|c| c.id))
            .chain(p.texts.iter().map(|c| c.id))
            .max()
            .unwrap_or(0);
        p.next_id = p.next_id.max(max_id + 1);
        Ok(p)
    }

    pub fn missing_media(&self) -> Vec<&Asset> {
        self.assets.iter().filter(|a| !a.path.is_file()).collect()
    }
}

pub fn validate_text(t: &TextClip) -> anyhow::Result<()> {
    if t.text.trim().is_empty() {
        bail!("Text is empty");
    }
    if t.text.len() > 2000 {
        bail!("Text is too long (max 2000 characters)");
    }
    if !(t.start.is_finite() && t.start >= 0.0) {
        bail!("Text start must be 0 or more");
    }
    if !(t.duration.is_finite() && t.duration >= MIN_CLIP) {
        bail!("Text duration must be at least {MIN_CLIP}s");
    }
    if !(0.01..=0.5).contains(&t.size) {
        bail!("Text size must be between 0.01 and 0.5");
    }
    Ok(())
}

impl TextClip {
    pub fn new(text: &str, start: f64, duration: f64) -> TextClip {
        TextClip {
            id: 0,
            text: text.to_string(),
            start,
            duration,
            x: 0.5,
            y: 0.85,
            size: 0.06,
            color: [255, 255, 255],
            background: true,
            fade: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Editor: project + undo history + activity log, shared by UI and MCP
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    User,
    Ai,
}

pub struct LogEntry {
    pub source: Source,
    pub text: String,
    pub at: Instant,
    pub ok: bool,
}

#[derive(Clone)]
pub struct ExportSettings {
    pub canvas: Canvas,
    pub quality: Quality,
    pub path: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    High,
    Standard,
    Small,
}

impl Quality {
    pub fn crf(self) -> u32 {
        match self {
            Quality::High => 18,
            Quality::Standard => 22,
            Quality::Small => 27,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Quality::High => "High",
            Quality::Standard => "Standard",
            Quality::Small => "Small file",
        }
    }
    pub fn parse(s: &str) -> anyhow::Result<Quality> {
        match s.to_ascii_lowercase().as_str() {
            "high" => Ok(Quality::High),
            "standard" | "medium" => Ok(Quality::Standard),
            "small" | "low" => Ok(Quality::Small),
            _ => bail!("Quality must be high, standard or small"),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub enum ExportStatus {
    #[default]
    Idle,
    Running { progress: f32, eta_secs: Option<f64>, path: PathBuf },
    Done { path: PathBuf, secs: f64 },
    Failed { error: String },
    Cancelled,
}

struct UndoEntry {
    before: Project,
    desc: String,
    /// Consecutive edits with the same key (a slider drag, typing) become one undo step.
    merge_key: Option<String>,
    last_edit: Instant,
}

pub struct Editor {
    pub project: Project,
    undo: Vec<UndoEntry>,
    redo: Vec<(Project, String)>,
    /// Bumped on every change; the preview re-renders when it moves.
    pub revision: u64,
    saved_revision: u64,
    pub path: Option<PathBuf>,
    pub playhead: f64,
    /// Seek requested from outside the UI (e.g. by the AI).
    pub seek_request: Option<f64>,
    /// Play (true) or pause (false) requested from outside the UI.
    pub play_request: Option<bool>,
    /// Whether the preview is currently playing (kept in sync by the UI).
    pub playing: bool,
    pub log: Vec<LogEntry>,
    /// Clip id -> when the AI last touched it (for the glow animation).
    pub ai_touched: HashMap<Id, Instant>,
    pub export_status: ExportStatus,
    pub cancel_export: bool,
    /// False until the user creates or opens a project.
    pub has_project: bool,
    /// When an AI client last called a tool.
    pub last_ai_call: Option<Instant>,
}

const UNDO_LIMIT: usize = 200;

impl Default for Editor {
    fn default() -> Self {
        Editor::new(Project::default())
    }
}

impl Editor {
    pub fn new(project: Project) -> Editor {
        Editor {
            project,
            undo: Vec::new(),
            redo: Vec::new(),
            revision: 1,
            saved_revision: 1,
            path: None,
            playhead: 0.0,
            seek_request: None,
            play_request: None,
            playing: false,
            log: Vec::new(),
            ai_touched: HashMap::new(),
            export_status: ExportStatus::Idle,
            cancel_export: false,
            has_project: false,
            last_ai_call: None,
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }

    pub fn mark_saved(&mut self) {
        self.saved_revision = self.revision;
    }

    pub fn mark_saved_at(&mut self, revision: u64) {
        self.saved_revision = revision;
    }

    pub fn replace_project(&mut self, project: Project, path: Option<PathBuf>) {
        self.project = project;
        self.undo.clear();
        self.redo.clear();
        self.revision += 1;
        self.saved_revision = self.revision;
        self.path = path;
        self.playhead = 0.0;
        self.ai_touched.clear();
        self.has_project = true;
    }

    /// Runs an edit with undo support. Nothing changes if the edit fails.
    pub fn apply<T>(&mut self, source: Source, desc: &str, f: impl FnOnce(&mut Project) -> anyhow::Result<T>) -> anyhow::Result<T> {
        self.apply_merge(source, desc, None, f)
    }

    /// Like `apply`, but edits sharing `merge_key` within a short window collapse
    /// into a single undo step and a single log line.
    pub fn apply_merge<T>(
        &mut self,
        source: Source,
        desc: &str,
        merge_key: Option<&str>,
        f: impl FnOnce(&mut Project) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        let before = self.project.clone();
        match f(&mut self.project) {
            Ok(v) => {
                let merging = merge_key.is_some()
                    && self.redo.is_empty()
                    && self.undo.last().is_some_and(|e| {
                        // "drag:" keys are unique per mouse gesture, so they merge for the whole drag.
                        e.merge_key.as_deref() == merge_key
                            && (merge_key.is_some_and(|k| k.starts_with("drag:")) || e.last_edit.elapsed().as_secs_f32() < 1.5)
                    });
                if merging {
                    let e = self.undo.last_mut().unwrap();
                    e.last_edit = Instant::now();
                } else {
                    self.undo.push(UndoEntry {
                        before: before.clone(),
                        desc: desc.to_string(),
                        merge_key: merge_key.map(str::to_string),
                        last_edit: Instant::now(),
                    });
                    if self.undo.len() > UNDO_LIMIT {
                        self.undo.remove(0);
                    }
                }
                self.redo.clear();
                self.revision += 1;
                let total = self.project.duration();
                if self.playhead > total {
                    self.playhead = total;
                }
                if source == Source::Ai {
                    self.mark_ai_changes(&before);
                }
                if !merging {
                    self.push_log(source, desc.to_string(), true);
                }
                Ok(v)
            }
            Err(e) => {
                self.project = before;
                self.push_log(source, format!("{desc} failed: {e}"), false);
                Err(e)
            }
        }
    }

    fn mark_ai_changes(&mut self, before: &Project) {
        let now = Instant::now();
        let old_main: HashMap<Id, &Clip> = before.main.iter().map(|c| (c.id, c)).collect();
        for c in &self.project.main {
            if old_main.get(&c.id).is_none_or(|o| *o != c) {
                self.ai_touched.insert(c.id, now);
            }
        }
        for c in &self.project.music {
            if !before.music.iter().any(|o| o == c) {
                self.ai_touched.insert(c.id, now);
            }
        }
        for t in &self.project.texts {
            if !before.texts.iter().any(|o| o == t) {
                self.ai_touched.insert(t.id, now);
            }
        }
    }

    pub fn push_log(&mut self, source: Source, text: String, ok: bool) {
        self.log.push(LogEntry { source, text, at: Instant::now(), ok });
        if self.log.len() > 500 {
            self.log.remove(0);
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self, source: Source) -> Option<String> {
        let UndoEntry { before: prev, desc, .. } = self.undo.pop()?;
        let current = std::mem::replace(&mut self.project, prev);
        self.redo.push((current, desc.clone()));
        self.revision += 1;
        self.playhead = self.playhead.min(self.project.duration());
        self.push_log(source, format!("Undo: {desc}"), true);
        Some(desc)
    }

    pub fn redo(&mut self, source: Source) -> Option<String> {
        let (next, desc) = self.redo.pop()?;
        let current = std::mem::replace(&mut self.project, next);
        self.undo.push(UndoEntry { before: current, desc: desc.clone(), merge_key: None, last_edit: Instant::now() });
        self.revision += 1;
        self.playhead = self.playhead.min(self.project.duration());
        self.push_log(source, format!("Redo: {desc}"), true);
        Some(desc)
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|e| e.desc.as_str())
    }
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|(_, d)| d.as_str())
    }
}

/// Seconds as `m:ss.cc`.
pub fn fmt_time(t: f64) -> String {
    let t = t.max(0.0);
    let m = (t / 60.0).floor() as u64;
    let s = t - m as f64 * 60.0;
    format!("{m}:{s:05.2}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video_info(d: f64) -> ProbeInfo {
        ProbeInfo { kind: MediaKind::Video, duration: d, width: 1920, height: 1080, fps: 30.0, has_audio: true }
    }

    fn project_with(durations: &[f64]) -> Project {
        let mut p = Project::new("t", Canvas::default());
        for (i, d) in durations.iter().enumerate() {
            let id = p.add_asset(Path::new(&format!("C:/v{i}.mp4")), &video_info(*d));
            p.insert_main(id, None, None).unwrap();
        }
        p
    }

    #[test]
    fn presets_make_even_sizes() {
        assert_eq!(Canvas::from_preset(16, 9, 1080, 30), Canvas { width: 1920, height: 1080, fps: 30 });
        assert_eq!(Canvas::from_preset(9, 16, 1080, 30), Canvas { width: 1080, height: 1920, fps: 30 });
        assert_eq!(Canvas::from_preset(4, 5, 1080, 30), Canvas { width: 1080, height: 1350, fps: 30 });
        assert_eq!(Canvas::from_preset(21, 9, 1080, 30), Canvas { width: 2520, height: 1080, fps: 30 });
        assert_eq!(Canvas::from_preset(16, 9, 2160, 60).width, 3840);
        let odd = Canvas::from_preset(16, 9, 721, 30);
        assert!(odd.width % 2 == 0 && odd.height % 2 == 0);
        assert_eq!(Canvas::from_preset(9, 16, 720, 30).aspect_label(), "9:16");
        assert_eq!(Canvas::from_preset(4, 5, 1080, 30).summary(), "4:5 · 1080p · 30 fps");
    }

    #[test]
    fn parse_presets() {
        assert_eq!(Canvas::parse_aspect("9:16").unwrap(), (9, 16));
        assert!(Canvas::parse_aspect("0:1").is_err());
        assert_eq!(Canvas::parse_resolution("4k").unwrap(), 2160);
        assert_eq!(Canvas::parse_resolution("720").unwrap(), 720);
        assert!(Canvas::parse_resolution("huge").is_err());
    }

    #[test]
    fn split_keeps_total_duration() {
        let mut p = project_with(&[10.0]);
        let new_id = p.split_at(4.0).unwrap();
        assert_eq!(p.main.len(), 2);
        assert!((p.duration() - 10.0).abs() < 1e-9);
        assert!((p.main[0].src_out - 4.0).abs() < 1e-9);
        assert_eq!(p.main[1].id, new_id);
        assert!((p.main[1].src_in - 4.0).abs() < 1e-9);
        assert!(p.split_at(0.0).is_err(), "split at the very edge must fail");
        assert!(p.split_at(50.0).is_err(), "split past the end must fail");
    }

    #[test]
    fn delete_range_ripples_and_handles_straddling() {
        let mut p = project_with(&[10.0, 10.0]);
        p.add_text(TextClip::new("before", 1.0, 2.0)).unwrap();
        p.add_text(TextClip::new("after", 15.0, 2.0)).unwrap();
        p.add_text(TextClip::new("inside", 8.5, 1.0)).unwrap();
        p.delete_range(8.0, 12.0).unwrap();
        assert!((p.duration() - 16.0).abs() < 1e-9);
        assert_eq!(p.main.len(), 2);
        assert!((p.main[0].src_out - 8.0).abs() < 1e-9);
        assert!((p.main[1].src_in - 2.0).abs() < 1e-9);
        let ids: std::collections::HashSet<_> = p.main.iter().map(|c| c.id).collect();
        assert_eq!(ids.len(), 2);
        let after = p.texts.iter().find(|t| t.text == "after").unwrap();
        assert!((after.start - 11.0).abs() < 1e-9);
        assert!(p.texts.iter().all(|t| t.text != "inside"));
    }

    #[test]
    fn delete_range_inside_one_clip_creates_unique_ids() {
        let mut p = project_with(&[10.0]);
        p.delete_range(3.0, 5.0).unwrap();
        assert_eq!(p.main.len(), 2);
        assert_ne!(p.main[0].id, p.main[1].id);
        assert!((p.duration() - 8.0).abs() < 1e-9);
        assert!(p.next_id > p.main.iter().map(|c| c.id).max().unwrap());
    }

    #[test]
    fn trim_is_clamped_and_validated() {
        let mut p = project_with(&[10.0]);
        let id = p.main[0].id;
        p.trim_main(id, 2.0, 20.0).unwrap();
        assert!((p.main[0].src_out - 10.0).abs() < 1e-9);
        assert!(p.trim_main(id, 5.0, 5.01).is_err());
    }

    #[test]
    fn undo_redo_and_failed_edits() {
        let mut ed = Editor::new(project_with(&[10.0]));
        let rev = ed.revision;
        ed.apply(Source::User, "Split", |p| p.split_at(5.0)).unwrap();
        assert_eq!(ed.project.main.len(), 2);
        assert!(ed.revision > rev);
        // A failing edit leaves the project untouched and is not undoable.
        let before = ed.project.main.len();
        assert!(ed.apply(Source::Ai, "Bad", |p| p.split_at(99.0)).is_err());
        assert_eq!(ed.project.main.len(), before);
        ed.undo(Source::User).unwrap();
        assert_eq!(ed.project.main.len(), 1);
        ed.redo(Source::User).unwrap();
        assert_eq!(ed.project.main.len(), 2);
        assert!(ed.undo(Source::User).is_some());
        assert!(ed.undo(Source::User).is_none());
    }

    #[test]
    fn speed_changes_timeline_length_and_split_math() {
        let mut p = project_with(&[10.0]);
        let id = p.main[0].id;
        p.set_speed(id, 2.0).unwrap();
        assert!((p.duration() - 5.0).abs() < 1e-9);
        // Splitting at timeline 2s cuts the source at 4s.
        p.split_at(2.0).unwrap();
        assert!((p.main[0].src_out - 4.0).abs() < 1e-9);
        assert!((p.main[1].src_in - 4.0).abs() < 1e-9);
        assert!((p.duration() - 5.0).abs() < 1e-9);
        // Ripple delete respects speed too.
        p.delete_range(1.0, 3.0).unwrap();
        assert!((p.duration() - 3.0).abs() < 1e-9);
        assert!((p.main[0].src_out - 2.0).abs() < 1e-9);
        assert!((p.main[1].src_in - 6.0).abs() < 1e-9);
        p.set_speed(p.main[0].id, 99.0).unwrap();
        assert_eq!(p.main[0].speed, MAX_SPEED);
    }

    #[test]
    fn fades_color_and_duplicate() {
        let mut p = project_with(&[4.0]);
        let id = p.main[0].id;
        p.set_fades(id, Some(1.0), Some(10.0)).unwrap();
        assert_eq!(p.main[0].fade_in, 1.0);
        assert_eq!(p.main[0].fade_out, 2.0, "fades are capped at half the clip");
        p.set_color(id, Some(0.2), None, Some(5.0)).unwrap();
        assert!(p.main[0].has_color());
        assert_eq!(p.main[0].saturation, 2.0);
        let copy = p.duplicate_clip(id).unwrap();
        assert_eq!(p.main.len(), 2);
        assert_eq!(p.main[1].id, copy);
        assert_eq!(p.main[1].fade_in, 1.0);
        assert!((p.duration() - 8.0).abs() < 1e-9);
    }

    #[test]
    fn old_project_files_still_load() {
        let json = r#"{"format_version":1,"name":"old","canvas":{"width":1920,"height":1080,"fps":30},
            "assets":[],"main":[{"id":1,"asset_id":9,"src_in":0.0,"src_out":2.0,"volume":1.0,"fit":"fit"}],
            "music":[],"texts":[{"id":2,"text":"hi","start":0.0,"duration":1.0,"x":0.5,"y":0.5,"size":0.06,"color":[255,255,255],"background":true}],"next_id":3}"#;
        let p: Project = serde_json::from_str(json).unwrap();
        assert_eq!(p.main[0].speed, 1.0);
        assert_eq!(p.main[0].contrast, 1.0);
        assert!(!p.main[0].has_color());
        assert!(!p.texts[0].fade);
    }

    #[test]
    fn merged_edits_are_one_undo_step() {
        let mut ed = Editor::new(project_with(&[10.0]));
        let id = ed.project.main[0].id;
        for v in [0.9, 0.8, 0.7] {
            ed.apply_merge(Source::User, "Volume", Some("vol"), |p| p.set_volume(id, v)).unwrap();
        }
        assert!((ed.project.main[0].volume - 0.7).abs() < 1e-6);
        ed.undo(Source::User).unwrap();
        assert!((ed.project.main[0].volume - 1.0).abs() < 1e-6);
        assert!(!ed.can_undo());
    }

    #[test]
    fn ai_changes_are_highlighted() {
        let mut ed = Editor::new(project_with(&[10.0]));
        let new_id = ed.apply(Source::Ai, "Split", |p| p.split_at(5.0)).unwrap();
        assert!(ed.ai_touched.contains_key(&new_id));
    }

    #[test]
    fn audio_cannot_go_on_main_track() {
        let mut p = Project::new("t", Canvas::default());
        let info = ProbeInfo { kind: MediaKind::Audio, duration: 30.0, width: 0, height: 0, fps: 0.0, has_audio: true };
        let id = p.add_asset(Path::new("C:/a.mp3"), &info);
        assert!(p.insert_main(id, None, None).is_err());
        p.add_music(id, 2.0, None).unwrap();
        assert_eq!(p.music.len(), 1);
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.aive");
        let mut p = project_with(&[3.0, 4.0]);
        p.add_text(TextClip::new("hi", 0.0, 1.0)).unwrap();
        p.save(&path).unwrap();
        let q = Project::load(&path).unwrap();
        assert_eq!(q.main.len(), 2);
        assert_eq!(q.texts[0].text, "hi");
        assert_eq!(q.next_id, p.next_id);
    }
}
