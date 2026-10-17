//! Every change to a project. Each operation either succeeds or leaves the
//! project untouched, and clips on one track never overlap.

use std::path::Path;

use anyhow::{Context, bail, ensure};

use crate::clip::{Clip, Content, Part, ShapeLayer, TextLayer};
use crate::media::ProbeInfo;
use crate::model::{Asset, Id, MediaKind, Project, Track, TrackKind};

/// Shortest clip allowed, in seconds.
pub const MIN_CLIP: f64 = 0.1;
/// Default length for images, text and shapes.
pub const DEFAULT_STILL: f64 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Start,
    End,
}

impl Project {
    pub fn asset(&self, id: Id) -> Option<&Asset> {
        self.assets.iter().find(|a| a.id == id)
    }

    pub fn clip(&self, id: Id) -> Option<&Clip> {
        self.clips.iter().find(|c| c.id == id)
    }

    pub fn clip_mut(&mut self, id: Id) -> anyhow::Result<&mut Clip> {
        self.clips.iter_mut().find(|c| c.id == id).context("That clip no longer exists")
    }

    pub fn track(&self, id: Id) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }

    pub fn track_mut(&mut self, id: Id) -> anyhow::Result<&mut Track> {
        self.tracks.iter_mut().find(|t| t.id == id).context("That track no longer exists")
    }

    pub fn track_index(&self, id: Id) -> Option<usize> {
        self.tracks.iter().position(|t| t.id == id)
    }

    /// End of the last clip.
    pub fn duration(&self) -> f64 {
        self.clips.iter().map(Clip::end).fold(0.0, f64::max)
    }

    pub fn clips_on(&self, track: Id) -> impl Iterator<Item = &Clip> {
        self.clips.iter().filter(move |c| c.track == track)
    }

    /// Track kind a clip with this content needs.
    pub fn kind_for(&self, content: &Content) -> TrackKind {
        match content {
            Content::Media { asset, .. } => match self.asset(*asset).map(|a| a.kind) {
                Some(MediaKind::Audio) => TrackKind::Audio,
                _ => TrackKind::Visual,
            },
            _ => TrackKind::Visual,
        }
    }

    /// Track kind an existing clip needs (separated sound goes on audio tracks).
    pub fn clip_kind(&self, clip: &Clip) -> TrackKind {
        if clip.part == Part::Sound { TrackKind::Audio } else { self.kind_for(&clip.content) }
    }

    /// Whether the clip shows a picture (text, shapes, images, and video that isn't sound-only).
    pub fn clip_has_picture(&self, clip: &Clip) -> bool {
        match clip.content {
            Content::Media { asset, .. } => clip.part != Part::Sound && self.asset(asset).is_some_and(|a| a.is_visual()),
            _ => true,
        }
    }

    /// Whether the clip plays sound (audio, and video with sound that hasn't been separated).
    pub fn clip_has_sound(&self, clip: &Clip) -> bool {
        match clip.content {
            Content::Media { asset, .. } => clip.part != Part::Picture && self.asset(asset).is_some_and(|a| a.has_audio && a.kind != MediaKind::Image),
            _ => false,
        }
    }

    /// Longest the clip can be, given its source media.
    pub fn max_duration(&self, clip: &Clip) -> f64 {
        match clip.content {
            Content::Media { asset, src_in, speed } => match self.asset(asset) {
                Some(a) if a.kind != MediaKind::Image => ((a.duration - src_in) / speed).max(MIN_CLIP),
                _ => f64::INFINITY,
            },
            _ => f64::INFINITY,
        }
    }

    /// Fails if the clip sits on a locked track.
    pub fn ensure_unlocked(&self, clip: Id) -> anyhow::Result<()> {
        let c = self.clip(clip).context("That clip no longer exists")?;
        ensure!(!self.track(c.track).is_some_and(|t| t.locked), "That track is locked");
        Ok(())
    }

    fn overlaps(&self, track: Id, start: f64, duration: f64, ignore: &[Id]) -> bool {
        const EPS: f64 = 1e-6;
        self.clips_on(track).any(|c| !ignore.contains(&c.id) && start < c.end() - EPS && start + duration > c.start + EPS)
    }

    /// The free start time closest to `wanted` where a clip of `duration` fits on `track`.
    pub fn nearest_free(&self, track: Id, wanted: f64, duration: f64, ignore: &[Id]) -> f64 {
        let wanted = wanted.max(0.0);
        if !self.overlaps(track, wanted, duration, ignore) {
            return wanted;
        }
        let mut candidates = vec![0.0];
        for c in self.clips_on(track).filter(|c| !ignore.contains(&c.id)) {
            candidates.push(c.end());
            candidates.push(c.start - duration);
        }
        candidates
            .into_iter()
            .filter(|&s| s >= 0.0 && !self.overlaps(track, s, duration, ignore))
            .min_by(|a, b| (a - wanted).abs().total_cmp(&(b - wanted).abs()))
            .unwrap_or_else(|| self.clips_on(track).map(Clip::end).fold(0.0, f64::max))
    }

    pub fn add_track(&mut self, kind: TrackKind, name: &str, index: Option<usize>) -> Id {
        let id = self.alloc_id();
        let track = Track { id, name: name.to_string(), kind, muted: false, locked: false, hidden: false, height: None };
        // Visual tracks stay above audio tracks.
        let first_audio = self.tracks.iter().position(|t| t.kind == TrackKind::Audio).unwrap_or(self.tracks.len());
        let at = match (kind, index) {
            (TrackKind::Visual, Some(i)) => i.min(first_audio),
            (TrackKind::Visual, None) => first_audio,
            (TrackKind::Audio, Some(i)) => i.clamp(first_audio, self.tracks.len()),
            (TrackKind::Audio, None) => self.tracks.len(),
        };
        self.tracks.insert(at, track);
        id
    }

    pub fn rename_track(&mut self, id: Id, name: &str) -> anyhow::Result<()> {
        let name = name.trim();
        ensure!(!name.is_empty(), "Track names can't be empty");
        self.track_mut(id)?.name = name.to_string();
        Ok(())
    }

    pub fn delete_track(&mut self, id: Id) -> anyhow::Result<()> {
        let i = self.track_index(id).context("That track no longer exists")?;
        let kind = self.tracks[i].kind;
        ensure!(self.tracks.iter().filter(|t| t.kind == kind).count() > 1, "Keep at least one {} track", if kind == TrackKind::Audio { "audio" } else { "video" });
        self.tracks.remove(i);
        self.clips.retain(|c| c.track != id);
        Ok(())
    }

    /// Moves a track up (`delta` < 0) or down, staying within its kind's group.
    pub fn move_track(&mut self, id: Id, delta: i32) -> anyhow::Result<()> {
        let i = self.track_index(id).context("That track no longer exists")?;
        let j = i as i32 + delta;
        ensure!(j >= 0 && (j as usize) < self.tracks.len(), "Can't move further");
        ensure!(self.tracks[j as usize].kind == self.tracks[i].kind, "Video and audio tracks stay in their own groups");
        self.tracks.swap(i, j as usize);
        Ok(())
    }

    pub fn add_asset(&mut self, path: &Path, info: &ProbeInfo) -> Id {
        if let Some(existing) = self.assets.iter().find(|a| a.path == path) {
            return existing.id;
        }
        let id = self.alloc_id();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Media".into());
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

    pub fn remove_asset(&mut self, id: Id) -> anyhow::Result<()> {
        ensure!(!self.clips.iter().any(|c| c.asset() == Some(id)), "This media is used on the timeline. Delete those clips first.");
        self.assets.retain(|a| a.id != id);
        Ok(())
    }

    /// A track of `kind` that is free for the whole span, keeping the requested time.
    /// Text and shapes prefer the top; media prefers the bottom ("Main"). If every
    /// track is busy there, a new one is added (overlays above, audio below).
    fn free_track(&mut self, kind: TrackKind, prefer_top: bool, start: f64, duration: f64) -> Id {
        let mut tracks: Vec<Id> = self.tracks.iter().filter(|t| t.kind == kind && !t.locked).map(|t| t.id).collect();
        if !prefer_top && kind == TrackKind::Visual {
            tracks.reverse();
        }
        if let Some(t) = tracks.into_iter().find(|&t| !self.overlaps(t, start, duration, &[])) {
            return t;
        }
        let n = self.tracks.iter().filter(|t| t.kind == kind).count() + 1;
        match kind {
            TrackKind::Visual => self.add_track(kind, &format!("Video {n}"), Some(0)),
            TrackKind::Audio => self.add_track(kind, &format!("Audio {n}"), None),
        }
    }

    /// Places a clip at `start` on `track`, or on a free track of the right kind.
    /// With an explicit track that is busy, the clip moves to the nearest free time.
    pub fn add_clip(&mut self, content: Content, track: Option<Id>, start: f64, duration: f64) -> anyhow::Result<Id> {
        let kind = self.kind_for(&content);
        let prefer_top = !matches!(content, Content::Media { .. });
        let duration = duration.max(MIN_CLIP);
        let start = start.max(0.0);
        let track = match track {
            Some(t) => {
                let tr = self.track(t).context("That track no longer exists")?;
                ensure!(tr.kind == kind, "{} clips go on {} tracks", if kind == TrackKind::Audio { "Audio" } else { "Video" }, if kind == TrackKind::Audio { "audio" } else { "video" });
                ensure!(!tr.locked, "That track is locked");
                t
            }
            None => self.free_track(kind, prefer_top, start, duration),
        };
        let start = self.nearest_free(track, start, duration, &[]);
        let id = self.alloc_id();
        self.clips.push(Clip::new(id, track, start, duration, content));
        Ok(id)
    }

    pub fn add_media_clip(&mut self, asset: Id, track: Option<Id>, start: f64) -> anyhow::Result<Id> {
        self.add_media_clip_still(asset, track, start, DEFAULT_STILL)
    }

    /// Like `add_media_clip`, with images lasting `still` seconds.
    pub fn add_media_clip_still(&mut self, asset: Id, track: Option<Id>, start: f64, still: f64) -> anyhow::Result<Id> {
        let a = self.asset(asset).context("That media is no longer in the library")?;
        let duration = if a.kind == MediaKind::Image { still.max(MIN_CLIP) } else { a.duration };
        let id = self.add_clip(Content::Media { asset, src_in: 0.0, speed: 1.0 }, track, start, duration)?;
        if self.asset(asset).is_some_and(|a| a.kind == MediaKind::Image) {
            // Logos and screenshots usually shouldn't be blown up to the full frame.
            self.clip_mut(id)?.transform.scale = 0.6;
        }
        Ok(id)
    }

    pub fn add_text_clip(&mut self, track: Option<Id>, start: f64, duration: f64, text: TextLayer) -> anyhow::Result<Id> {
        self.add_clip(Content::Text(text), track, start, duration)
    }

    pub fn add_shape_clip(&mut self, track: Option<Id>, start: f64, duration: f64, shape: ShapeLayer) -> anyhow::Result<Id> {
        self.add_clip(Content::Shape(shape), track, start, duration)
    }

    /// A blur box over whatever is below it, from `start` for `duration` seconds.
    pub fn add_blur_clip(&mut self, track: Option<Id>, start: f64, duration: f64, blur: crate::clip::BlurLayer) -> anyhow::Result<Id> {
        self.add_clip(Content::Blur(blur), track, start, duration)
    }

    /// Moves a clip to `track` at `start`, nudging it to the nearest free spot.
    pub fn move_clip(&mut self, id: Id, track: Id, start: f64) -> anyhow::Result<()> {
        let clip = self.clip(id).context("That clip no longer exists")?;
        let (kind, duration) = (self.clip_kind(clip), clip.duration);
        let tr = self.track(track).context("That track no longer exists")?;
        ensure!(tr.kind == kind, "This clip can't go on that kind of track");
        ensure!(!tr.locked, "That track is locked");
        let start = self.nearest_free(track, start, duration, &[id]);
        let clip = self.clip_mut(id)?;
        clip.track = track;
        clip.start = start;
        Ok(())
    }

    /// Drags one edge of a clip to timeline time `t`, clamped to the media and the neighbours.
    pub fn trim(&mut self, id: Id, edge: Edge, t: f64) -> anyhow::Result<()> {
        self.ensure_unlocked(id)?;
        let clip = self.clip(id).context("That clip no longer exists")?.clone();
        let neighbours: Vec<&Clip> = self.clips_on(clip.track).filter(|c| c.id != id).collect();
        let prev_end = neighbours.iter().filter(|c| c.end() <= clip.start + 1e-6).map(|c| c.end()).fold(0.0, f64::max);
        let next_start = neighbours.iter().filter(|c| c.start >= clip.end() - 1e-6).map(|c| c.start).fold(f64::INFINITY, f64::min);
        let speed = clip.speed();
        let c = self.clip_mut(id)?;
        match edge {
            Edge::Start => {
                // max/min rather than clamp: neighbours can overlap by a rounding error.
                let mut new_start = t.max(prev_end).min(clip.end() - MIN_CLIP);
                if let Content::Media { src_in, .. } = &mut c.content {
                    // Can't reveal footage before the start of the file (unless it's a still image).
                    let earliest = clip.start - *src_in / speed;
                    new_start = new_start.max(earliest);
                    *src_in = (*src_in + (new_start - clip.start) * speed).max(0.0);
                }
                c.duration = clip.end() - new_start;
                c.start = new_start;
                // Keep relative times of zooms and keyframes anchored to the footage.
                let shift = new_start - clip.start;
                for z in &mut c.zooms {
                    z.at -= shift;
                }
                for k in c.keyframes.values_mut().flatten() {
                    k.t -= shift;
                }
                for g in &mut c.audio.gains {
                    g.at -= shift;
                }
                c.audio.gains.retain(|g| g.at + g.duration > 0.0);
            }
            Edge::End => {
                c.duration = (t.min(next_start) - clip.start).max(MIN_CLIP);
            }
        }
        let max = self.max_duration(&self.clip(id).unwrap().clone());
        let c = self.clip_mut(id)?;
        c.duration = c.duration.min(max);
        // Drop or shorten zooms and range volumes that now fall outside the clip.
        let len = c.duration;
        c.zooms.retain(|z| z.at + z.duration > 0.05 && z.at < len - 0.05);
        for z in &mut c.zooms {
            let end = (z.at + z.duration).min(len);
            z.at = z.at.max(0.0);
            z.duration = end - z.at;
        }
        c.audio.gains.retain(|g| g.at + g.duration > 0.01 && g.at < len - 0.01);
        for g in &mut c.audio.gains {
            let end = (g.at + g.duration).min(len);
            g.at = g.at.max(0.0);
            g.duration = end - g.at;
        }
        Ok(())
    }

    /// Splits a clip at timeline time `t`. Returns the id of the new right-hand clip.
    pub fn split(&mut self, id: Id, t: f64) -> anyhow::Result<Id> {
        self.ensure_unlocked(id)?;
        let clip = self.clip(id).context("That clip no longer exists")?.clone();
        ensure!(t > clip.start + MIN_CLIP && t < clip.end() - MIN_CLIP, "Move the playhead inside the clip to split it");
        let new_id = self.alloc_id();
        let mut right = clip.clone();
        right.id = new_id;
        right.start = t;
        right.duration = clip.end() - t;
        if let Content::Media { src_in, speed, .. } = &mut right.content {
            *src_in += (t - clip.start) * *speed;
        }
        right.intro = None;
        right.transition = None;
        let cut = t - clip.start;
        right.zooms = clip.zooms.iter().filter(|z| z.at + z.duration > cut).map(|z| crate::clip::Zoom { at: z.at - cut, ..*z }).collect();
        for k in right.keyframes.values_mut().flatten() {
            k.t -= cut;
        }
        right.audio.gains = clip.audio.gains.iter().filter(|g| g.at + g.duration > cut).map(|g| crate::clip::Gain { at: g.at - cut, ..*g }).collect();
        let left = self.clip_mut(id)?;
        left.duration = cut;
        left.outro = None;
        left.zooms.retain(|z| z.at < cut);
        left.audio.gains.retain(|g| g.at < cut);
        self.clips.push(right);
        Ok(new_id)
    }

    pub fn delete_clips(&mut self, ids: &[Id]) -> anyhow::Result<()> {
        ensure!(!ids.is_empty(), "Nothing selected");
        for &id in ids {
            self.ensure_unlocked(id)?;
        }
        self.clips.retain(|c| !ids.contains(&c.id));
        Ok(())
    }

    /// Changes playback speed, keeping the same footage: the clip gets shorter or longer,
    /// later clips on its track move along so none of it is lost, and zooms and range
    /// volumes stay on the same footage.
    pub fn set_speed(&mut self, id: Id, new_speed: f64) -> anyhow::Result<()> {
        ensure!((0.1..=8.0).contains(&new_speed), "Speed must be between 0.1× and 8×");
        self.ensure_unlocked(id)?;
        let clip = self.clip(id).context("That clip no longer exists")?.clone();
        let Content::Media { speed, .. } = clip.content else { bail!("Only video and audio clips have a speed") };
        let ratio = speed / new_speed;
        let new_duration = (clip.duration * ratio).max(MIN_CLIP);
        let delta = new_duration - clip.duration;
        let old_end = clip.end();
        for c in self.clips.iter_mut().filter(|c| c.track == clip.track && c.id != id && c.start >= old_end - 1e-6) {
            c.start = (c.start + delta).max(0.0);
        }
        let c = self.clip_mut(id)?;
        if let Content::Media { speed, .. } = &mut c.content {
            *speed = new_speed;
        }
        c.duration = new_duration;
        for z in &mut c.zooms {
            z.at *= ratio;
            z.duration *= ratio;
        }
        for g in &mut c.audio.gains {
            g.at *= ratio;
            g.duration *= ratio;
        }
        for k in c.keyframes.values_mut().flatten() {
            k.t *= ratio;
        }
        Ok(())
    }

    /// Moves or resizes a zoom along its clip without overlapping the clip's other zooms.
    pub fn set_zoom_timing(&mut self, clip: Id, zoom: Id, at: f64, duration: f64) -> anyhow::Result<()> {
        let c = self.clip_mut(clip)?;
        let len = c.duration;
        let others: Vec<(f64, f64)> = c.zooms.iter().filter(|z| z.id != zoom).map(|z| (z.at, z.at + z.duration)).collect();
        let z = c.zooms.iter_mut().find(|z| z.id == zoom).context("That zoom no longer exists")?;
        let mut at = at.clamp(0.0, (len - 0.3).max(0.0));
        let mut duration = duration.max(0.3).min(len - at);
        // Stay between the neighbours on either side of where it was dropped.
        let mid = at + duration / 2.0;
        let lo = others.iter().filter(|(_, e)| *e <= mid).map(|(_, e)| *e).fold(0.0, f64::max);
        let hi = others.iter().filter(|(s, _)| *s >= mid).map(|(s, _)| *s).fold(len, f64::min);
        ensure!(hi - lo >= 0.3, "There's no room for the zoom there");
        duration = duration.min(hi - lo);
        at = at.max(lo).min(hi - duration);
        z.at = at;
        z.duration = duration;
        Ok(())
    }

    /// Sets the volume of the part of `clip` between timeline times `a` and `b` (replacing
    /// earlier range changes that overlap it). `volume` 1 removes the change.
    pub fn set_range_volume(&mut self, clip: Id, a: f64, b: f64, volume: f64) -> anyhow::Result<Option<Id>> {
        self.ensure_unlocked(clip)?;
        let id = self.alloc_id();
        let c = self.clip_mut(clip)?;
        let (s, e) = ((a.min(b) - c.start).max(0.0), (a.max(b) - c.start).min(c.duration));
        ensure!(e - s >= 0.05, "The selected range doesn't cover this clip");
        c.audio.gains.retain(|g| g.at + g.duration <= s + 1e-6 || g.at >= e - 1e-6);
        if (volume - 1.0).abs() < 1e-3 {
            return Ok(None);
        }
        c.audio.gains.push(crate::clip::Gain { id, at: s, duration: e - s, volume: volume.clamp(0.0, 4.0) });
        c.audio.gains.sort_by(|x, y| x.at.total_cmp(&y.at));
        Ok(Some(id))
    }

    /// Separates a video clip's sound into its own clip on an audio track, at the same
    /// time and with the same volume settings. The video keeps playing, silently.
    pub fn separate_audio(&mut self, id: Id) -> anyhow::Result<Id> {
        self.ensure_unlocked(id)?;
        let clip = self.clip(id).context("That clip no longer exists")?.clone();
        let Content::Media { asset, .. } = clip.content else { bail!("Only video clips have sound to separate") };
        let a = self.asset(asset).context("That media is no longer in the library")?;
        ensure!(a.kind == MediaKind::Video, "Only video clips have sound to separate");
        ensure!(a.has_audio, "This video has no sound");
        ensure!(clip.part == Part::Whole, "The sound of this clip is already separate");
        let track = self.free_track(TrackKind::Audio, false, clip.start, clip.duration);
        let new_id = self.alloc_id();
        let mut sound = Clip::new(new_id, track, clip.start, clip.duration, clip.content.clone());
        sound.part = Part::Sound;
        sound.audio = clip.audio.clone();
        sound.name = clip.name.as_ref().map(|n| format!("{n} (audio)"));
        if self.track(clip.track).is_some_and(|t| t.hidden) {
            sound.audio.muted = true;
        }
        self.clips.push(sound);
        let video = self.clip_mut(id)?;
        video.part = Part::Picture;
        video.audio = Default::default();
        Ok(new_id)
    }

    /// Adds a timeline marker at `at` (a span when `duration` > 0), named "Marker N".
    pub fn add_marker(&mut self, at: f64, duration: f64, color: crate::clip::Rgba) -> Id {
        let id = self.alloc_id();
        let n = self.markers.len() + 1;
        self.markers.push(crate::model::Marker { id, at: at.max(0.0), duration: duration.max(0.0), name: format!("Marker {n}"), color });
        self.markers.sort_by(|a, b| a.at.total_cmp(&b.at));
        id
    }

    pub fn marker_mut(&mut self, id: Id) -> anyhow::Result<&mut crate::model::Marker> {
        self.markers.iter_mut().find(|m| m.id == id).context("That marker no longer exists")
    }

    /// Moves a marker so it starts at `at`, keeping the list in time order.
    pub fn move_marker(&mut self, id: Id, at: f64) -> anyhow::Result<()> {
        self.marker_mut(id)?.at = at.max(0.0);
        self.markers.sort_by(|a, b| a.at.total_cmp(&b.at));
        Ok(())
    }

    pub fn delete_marker(&mut self, id: Id) -> anyhow::Result<()> {
        let before = self.markers.len();
        self.markers.retain(|m| m.id != id);
        ensure!(self.markers.len() < before, "That marker no longer exists");
        Ok(())
    }

    /// Duplicates a clip right after itself (or the nearest free spot).
    pub fn duplicate_clip(&mut self, id: Id) -> anyhow::Result<Id> {
        self.ensure_unlocked(id)?;
        let clip = self.clip(id).context("That clip no longer exists")?.clone();
        let start = self.nearest_free(clip.track, clip.end(), clip.duration, &[]);
        let new_id = self.alloc_id();
        self.clips.push(Clip { id: new_id, start, ..clip });
        Ok(new_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Canvas;
    use std::path::PathBuf;

    fn project_with_video(len: f64) -> (Project, Id) {
        let mut p = Project::new("t", Canvas::default());
        let info = ProbeInfo { kind: MediaKind::Video, duration: len, width: 1920, height: 1080, fps: 30.0, has_audio: true };
        let a = p.add_asset(&PathBuf::from("/v.mp4"), &info);
        (p, a)
    }

    #[test]
    fn new_clips_keep_their_time_on_a_free_track() {
        let (mut p, a) = project_with_video(10.0);
        let c1 = p.add_media_clip(a, None, 0.0).unwrap();
        let c2 = p.add_media_clip(a, None, 3.0).unwrap();
        let (k1, k2) = (p.clip(c1).unwrap().clone(), p.clip(c2).unwrap().clone());
        assert_eq!(p.track(k1.track).unwrap().name, "Main");
        assert_eq!(k2.start, 3.0, "time is kept");
        assert_ne!(k1.track, k2.track, "the overlapping clip goes on the next free video track");
        // With every video track busy, a new one appears on top.
        let tracks = p.tracks.len();
        for _ in 0..3 {
            p.add_media_clip(a, None, 3.0).unwrap();
        }
        assert!(p.tracks.len() > tracks);
        assert_eq!(p.tracks[0].kind, TrackKind::Visual);
        // An explicit busy track still nudges the time instead.
        let c = p.add_media_clip(a, Some(k1.track), 1.0).unwrap();
        assert_eq!(p.clip(c).unwrap().start, 10.0);
    }

    #[test]
    fn audio_cannot_go_on_video_tracks() {
        let mut p = Project::new("t", Canvas::default());
        let info = ProbeInfo { kind: MediaKind::Audio, duration: 5.0, width: 0, height: 0, fps: 0.0, has_audio: true };
        let a = p.add_asset(&PathBuf::from("/a.wav"), &info);
        let visual = p.tracks[0].id;
        assert!(p.add_media_clip(a, Some(visual), 0.0).is_err());
        let id = p.add_media_clip(a, None, 0.0).unwrap();
        assert_eq!(p.track(p.clip(id).unwrap().track).unwrap().kind, TrackKind::Audio);
    }

    #[test]
    fn trim_respects_media_length_and_neighbours() {
        let (mut p, a) = project_with_video(10.0);
        let c = p.add_media_clip(a, None, 2.0).unwrap();
        p.trim(c, Edge::End, 30.0).unwrap();
        assert_eq!(p.clip(c).unwrap().duration, 10.0, "can't extend past the media");
        p.trim(c, Edge::Start, 4.0).unwrap();
        let clip = p.clip(c).unwrap();
        assert_eq!((clip.start, clip.duration), (4.0, 8.0));
        assert!(matches!(clip.content, Content::Media { src_in, .. } if (src_in - 2.0).abs() < 1e-9));
        p.trim(c, Edge::Start, 0.0).unwrap();
        assert_eq!(p.clip(c).unwrap().start, 2.0, "can't reveal footage before the file starts");
    }

    #[test]
    fn split_keeps_source_continuous() {
        let (mut p, a) = project_with_video(10.0);
        let c = p.add_media_clip(a, None, 0.0).unwrap();
        let r = p.split(c, 4.0).unwrap();
        assert_eq!(p.clip(c).unwrap().duration, 4.0);
        let right = p.clip(r).unwrap();
        assert_eq!((right.start, right.duration), (4.0, 6.0));
        assert_eq!(right.source_time(5.0), 5.0);
        assert!(p.split(c, 0.05).is_err());
    }

    #[test]
    fn speed_change_keeps_footage() {
        let (mut p, a) = project_with_video(10.0);
        let c = p.add_media_clip(a, None, 0.0).unwrap();
        p.set_speed(c, 2.0).unwrap();
        assert_eq!(p.clip(c).unwrap().duration, 5.0);
        assert_eq!(p.clip(c).unwrap().source_time(5.0), 10.0);
    }

    #[test]
    fn slowing_down_next_to_a_neighbour_loses_nothing() {
        let (mut p, a) = project_with_video(10.0);
        let c = p.add_media_clip(a, None, 0.0).unwrap();
        let track = p.clip(c).unwrap().track;
        let next = p.add_media_clip(a, Some(track), 10.0).unwrap();
        p.clip_mut(c).unwrap().zooms.push(crate::clip::Zoom { id: 99, at: 4.0, duration: 2.0, scale: 2.0, focus_x: 0.5, focus_y: 0.5, ramp: 0.5 });
        p.set_speed(c, 0.5).unwrap();
        assert_eq!(p.clip(c).unwrap().duration, 20.0);
        assert_eq!(p.clip(next).unwrap().start, 20.0, "the neighbour moves along");
        assert_eq!(p.clip(c).unwrap().zooms[0].at, 8.0, "the zoom stays on the same footage");
        p.set_speed(c, 1.0).unwrap();
        assert_eq!(p.clip(c).unwrap().duration, 10.0, "back to the full original length");
        assert_eq!(p.clip(next).unwrap().start, 10.0);
    }

    #[test]
    fn locked_tracks_refuse_edits() {
        let (mut p, a) = project_with_video(10.0);
        let c = p.add_media_clip(a, None, 0.0).unwrap();
        let track = p.clip(c).unwrap().track;
        p.track_mut(track).unwrap().locked = true;
        assert!(p.trim(c, Edge::End, 5.0).is_err());
        assert!(p.split(c, 5.0).is_err());
        assert!(p.delete_clips(&[c]).is_err());
        assert!(p.set_speed(c, 2.0).is_err());
    }

    #[test]
    fn moving_onto_another_clip_finds_the_nearest_gap() {
        let (mut p, a) = project_with_video(4.0);
        let c1 = p.add_media_clip(a, None, 0.0).unwrap();
        let c2 = p.add_media_clip(a, None, 10.0).unwrap();
        let track = p.clip(c1).unwrap().track;
        p.move_clip(c2, track, 1.0).unwrap();
        assert_eq!(p.clip(c2).unwrap().start, 4.0);
    }

    #[test]
    fn zoom_timing_stays_between_neighbours() {
        let (mut p, a) = project_with_video(20.0);
        let c = p.add_media_clip(a, None, 0.0).unwrap();
        let mk = |id, at, duration| crate::clip::Zoom { id, at, duration, scale: 2.0, focus_x: 0.5, focus_y: 0.5, ramp: 0.5 };
        p.clip_mut(c).unwrap().zooms = vec![mk(1, 2.0, 3.0), mk(2, 10.0, 3.0)];
        // Dragged onto its neighbour: pushed back next to it.
        p.set_zoom_timing(c, 2, 4.0, 3.0).unwrap();
        let z = p.clip(c).unwrap().zooms[1];
        assert_eq!((z.at, z.duration), (5.0, 3.0));
        // Stretched past the clip end: clamped.
        p.set_zoom_timing(c, 2, 18.0, 9.0).unwrap();
        let z = p.clip(c).unwrap().zooms[1];
        assert!(z.at + z.duration <= 20.0 + 1e-9);
    }

    #[test]
    fn range_volume_only_changes_the_range() {
        let mut p = Project::new("t", Canvas::default());
        let info = ProbeInfo { kind: MediaKind::Audio, duration: 10.0, width: 0, height: 0, fps: 0.0, has_audio: true };
        let a = p.add_asset(&PathBuf::from("/a.wav"), &info);
        let c = p.add_media_clip(a, None, 2.0).unwrap();
        p.set_range_volume(c, 4.0, 6.0, 0.25).unwrap();
        let audio = &p.clip(c).unwrap().audio;
        assert_eq!(audio.gain_at(1.0), 1.0, "before the range");
        assert!((audio.gain_at(3.0) - 0.25).abs() < 1e-9, "inside the range");
        assert_eq!(audio.gain_at(5.0), 1.0, "after the range");
        // Splitting keeps the range on the right half, at the right place.
        let right = p.split(c, 4.5).unwrap();
        let r = &p.clip(right).unwrap().audio;
        assert!((r.gain_at(0.5) - 0.25).abs() < 1e-9 && r.gain_at(2.0) == 1.0);
        // Setting 100% again removes the change.
        p.set_range_volume(right, 4.5, 6.0, 1.0).unwrap();
        assert!(p.clip(right).unwrap().audio.gains.is_empty());
    }

    #[test]
    fn separating_audio_makes_a_sound_clip_and_silences_the_video() {
        let (mut p, a) = project_with_video(10.0);
        let v = p.add_media_clip(a, None, 2.0).unwrap();
        p.clip_mut(v).unwrap().audio.volume = 0.5;
        let s = p.separate_audio(v).unwrap();
        let (video, sound) = (p.clip(v).unwrap().clone(), p.clip(s).unwrap().clone());
        assert_eq!(p.track(sound.track).unwrap().kind, TrackKind::Audio, "the sound goes on an audio track");
        assert_eq!((sound.start, sound.duration), (video.start, video.duration), "at the same time");
        assert_eq!(sound.audio.volume, 0.5, "with the same volume");
        assert!(p.clip_has_sound(&sound) && !p.clip_has_picture(&sound));
        assert!(p.clip_has_picture(&video) && !p.clip_has_sound(&video));
        assert!(p.separate_audio(v).is_err(), "only once");
        // The sound clip stays on audio tracks when moved.
        let video_track = video.track;
        assert!(p.move_clip(s, video_track, 0.0).is_err());
    }

    #[test]
    fn markers_stay_in_time_order() {
        let mut p = Project::new("t", Canvas::default());
        let red = crate::clip::Rgba::hex(0xFF0000);
        let late = p.add_marker(5.0, 0.0, red);
        let early = p.add_marker(1.0, 2.0, red);
        assert_eq!(p.markers.iter().map(|m| m.id).collect::<Vec<_>>(), vec![early, late]);
        p.move_marker(early, 9.0).unwrap();
        assert_eq!(p.markers[1].id, early);
        p.delete_marker(late).unwrap();
        assert!(p.delete_marker(late).is_err());
    }

    #[test]
    fn tracks_keep_video_above_audio() {
        let mut p = Project::new("t", Canvas::default());
        let v = p.add_track(TrackKind::Visual, "B-roll", None);
        let i = p.track_index(v).unwrap();
        assert_eq!(p.tracks[i + 1].kind, TrackKind::Audio);
        assert!(p.move_track(v, 1).is_err(), "can't move a video track into the audio group");
        p.rename_track(v, "  Screen  ").unwrap();
        assert_eq!(p.track(v).unwrap().name, "Screen");
        assert!(p.rename_track(v, "  ").is_err());
    }
}
