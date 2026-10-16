//! Every change to a project. Each operation either succeeds or leaves the
//! project untouched, and clips on one track never overlap.

use std::path::Path;

use anyhow::{Context, bail, ensure};

use crate::clip::{Clip, Content, ShapeLayer, TextLayer};
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
        let track = Track { id, name: name.to_string(), kind, muted: false, locked: false, hidden: false };
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

    /// First track of `kind`, preferring the bottom-most visual track ("Main") for media.
    fn default_track(&self, kind: TrackKind, prefer_top: bool) -> anyhow::Result<Id> {
        let mut tracks = self.tracks.iter().filter(|t| t.kind == kind && !t.locked);
        let t = if prefer_top { tracks.next() } else { tracks.last() };
        t.map(|t| t.id).context("No unlocked track to put this on")
    }

    /// Places a clip on `track` (or a sensible default) at the free spot nearest `start`.
    pub fn add_clip(&mut self, content: Content, track: Option<Id>, start: f64, duration: f64) -> anyhow::Result<Id> {
        let kind = self.kind_for(&content);
        let prefer_top = !matches!(content, Content::Media { .. });
        let track = match track {
            Some(t) => {
                let tr = self.track(t).context("That track no longer exists")?;
                ensure!(tr.kind == kind, "{} clips go on {} tracks", if kind == TrackKind::Audio { "Audio" } else { "Video" }, if kind == TrackKind::Audio { "audio" } else { "video" });
                ensure!(!tr.locked, "That track is locked");
                t
            }
            None => self.default_track(kind, prefer_top)?,
        };
        let duration = duration.max(MIN_CLIP);
        let start = self.nearest_free(track, start, duration, &[]);
        let id = self.alloc_id();
        self.clips.push(Clip::new(id, track, start, duration, content));
        Ok(id)
    }

    pub fn add_media_clip(&mut self, asset: Id, track: Option<Id>, start: f64) -> anyhow::Result<Id> {
        let a = self.asset(asset).context("That media is no longer in the library")?;
        let duration = if a.kind == MediaKind::Image { DEFAULT_STILL } else { a.duration };
        let id = self.add_clip(Content::Media { asset, src_in: 0.0, speed: 1.0 }, track, start, duration)?;
        if self.asset(asset).is_some_and(|a| a.kind == MediaKind::Image) {
            // Logos and screenshots usually shouldn't be blown up to the full frame.
            self.clip_mut(id)?.transform.scale = 0.6;
        }
        Ok(id)
    }

    pub fn add_text_clip(&mut self, track: Option<Id>, start: f64, text: TextLayer) -> anyhow::Result<Id> {
        self.add_clip(Content::Text(text), track, start, DEFAULT_STILL)
    }

    pub fn add_shape_clip(&mut self, track: Option<Id>, start: f64, shape: ShapeLayer) -> anyhow::Result<Id> {
        self.add_clip(Content::Shape(shape), track, start, DEFAULT_STILL)
    }

    /// Moves a clip to `track` at `start`, nudging it to the nearest free spot.
    pub fn move_clip(&mut self, id: Id, track: Id, start: f64) -> anyhow::Result<()> {
        let clip = self.clip(id).context("That clip no longer exists")?;
        let (kind, duration) = (self.kind_for(&clip.content), clip.duration);
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
        let clip = self.clip(id).context("That clip no longer exists")?.clone();
        let neighbours: Vec<&Clip> = self.clips_on(clip.track).filter(|c| c.id != id).collect();
        let prev_end = neighbours.iter().filter(|c| c.end() <= clip.start + 1e-6).map(|c| c.end()).fold(0.0, f64::max);
        let next_start = neighbours.iter().filter(|c| c.start >= clip.end() - 1e-6).map(|c| c.start).fold(f64::INFINITY, f64::min);
        let speed = clip.speed();
        let c = self.clip_mut(id)?;
        match edge {
            Edge::Start => {
                let mut new_start = t.clamp(prev_end, clip.end() - MIN_CLIP);
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
            }
            Edge::End => {
                c.duration = (t.min(next_start) - clip.start).max(MIN_CLIP);
            }
        }
        let max = self.max_duration(&self.clip(id).unwrap().clone());
        let c = self.clip_mut(id)?;
        c.duration = c.duration.min(max);
        Ok(())
    }

    /// Splits a clip at timeline time `t`. Returns the id of the new right-hand clip.
    pub fn split(&mut self, id: Id, t: f64) -> anyhow::Result<Id> {
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
        let left = self.clip_mut(id)?;
        left.duration = cut;
        left.outro = None;
        left.zooms.retain(|z| z.at < cut);
        self.clips.push(right);
        Ok(new_id)
    }

    pub fn delete_clips(&mut self, ids: &[Id]) -> anyhow::Result<()> {
        ensure!(!ids.is_empty(), "Nothing selected");
        self.clips.retain(|c| !ids.contains(&c.id));
        Ok(())
    }

    /// Changes playback speed, keeping the same source footage (so the clip gets shorter or longer).
    pub fn set_speed(&mut self, id: Id, new_speed: f64) -> anyhow::Result<()> {
        ensure!((0.1..=8.0).contains(&new_speed), "Speed must be between 0.1× and 8×");
        let clip = self.clip(id).context("That clip no longer exists")?.clone();
        let Content::Media { speed, .. } = clip.content else { bail!("Only video and audio clips have a speed") };
        let source_len = clip.duration * speed;
        let new_duration = (source_len / new_speed).max(MIN_CLIP);
        let next_start = self.clips_on(clip.track).filter(|c| c.id != id && c.start >= clip.end() - 1e-6).map(|c| c.start).fold(f64::INFINITY, f64::min);
        let c = self.clip_mut(id)?;
        if let Content::Media { speed, .. } = &mut c.content {
            *speed = new_speed;
        }
        c.duration = new_duration.min(next_start - clip.start);
        Ok(())
    }

    /// Duplicates a clip right after itself (or the nearest free spot).
    pub fn duplicate_clip(&mut self, id: Id) -> anyhow::Result<Id> {
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
    fn media_goes_to_the_main_track_without_overlap() {
        let (mut p, a) = project_with_video(10.0);
        let c1 = p.add_media_clip(a, None, 0.0).unwrap();
        let c2 = p.add_media_clip(a, None, 3.0).unwrap();
        let (c1, c2) = (p.clip(c1).unwrap(), p.clip(c2).unwrap());
        assert_eq!(c1.track, c2.track);
        assert_eq!(p.track(c1.track).unwrap().name, "Main");
        assert_eq!(c2.start, 10.0, "second clip is pushed after the first");
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
    fn moving_onto_another_clip_finds_the_nearest_gap() {
        let (mut p, a) = project_with_video(4.0);
        let c1 = p.add_media_clip(a, None, 0.0).unwrap();
        let c2 = p.add_media_clip(a, None, 10.0).unwrap();
        let track = p.clip(c1).unwrap().track;
        p.move_clip(c2, track, 1.0).unwrap();
        assert_eq!(p.clip(c2).unwrap().start, 4.0);
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
