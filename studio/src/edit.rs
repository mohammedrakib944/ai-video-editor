//! Every change to a project. Each operation either succeeds or leaves the
//! project untouched, and clips on one track never overlap.

use std::path::Path;

use anyhow::{Context, bail, ensure};

use crate::clip::{CalloutLayer, Clip, Content, Grade, Motion, MotionKind, Part, Shadow, ShapeLayer, TextLayer, Transition, TransitionKind, Zoom};
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

    /// The clip immediately before `clip` on its own track, by start time — even while the two
    /// overlap for a transition. No other clip can ever sit between them, since general overlap
    /// stays forbidden everywhere except through `set_transition`.
    pub fn prev_on_track(&self, clip: &Clip) -> Option<&Clip> {
        self.clips_on(clip.track).filter(|c| c.id != clip.id && c.start < clip.start + 1e-6).max_by(|a, b| a.start.total_cmp(&b.start))
    }

    /// Undoes clip `id`'s own transition, if it has one: slides it and every later clip on its
    /// track forward by the overlap, restoring the gapless, non-overlapping layout.
    fn unapply_transition(&mut self, id: Id) {
        let Some(clip) = self.clip(id) else { return };
        let Some(tr) = clip.transition else { return };
        let (track, start, duration) = (clip.track, clip.start, tr.duration);
        for c in self.clips.iter_mut().filter(|c| c.track == track && c.start >= start - 1e-6) {
            c.start += duration;
        }
        self.clip_mut(id).unwrap().transition = None;
    }

    /// Clears clip `id`'s own transition, and the transition of whichever clip immediately
    /// follows it if that one was using `id` as its partner. Called by any edit that moves or
    /// resizes `id` in a way a cached transition's overlap would no longer match.
    fn detach_transition(&mut self, id: Id) {
        let dependent = self.clips.iter().find(|c| c.id != id && c.transition.is_some() && self.prev_on_track(c).is_some_and(|p| p.id == id)).map(|c| c.id);
        if let Some(dep) = dependent {
            self.unapply_transition(dep);
        }
        self.unapply_transition(id);
    }

    /// Replaces clip `id`'s color grade wholesale (used by grade presets, and to paste a grade
    /// copied from another clip).
    pub fn set_grade(&mut self, id: Id, grade: Grade) -> anyhow::Result<()> {
        self.ensure_unlocked(id)?;
        self.clip_mut(id)?.grade = grade;
        Ok(())
    }

    /// Sets, resizes, or (with `kind: None`) removes the transition from clip `id`'s immediately
    /// preceding clip into it. Unlike every other edit, this one clip pair is allowed to overlap:
    /// `id` (and everything after it on the track) is pulled `duration` seconds earlier so the two
    /// clips genuinely share screen time and blend, instead of `id` fading in from nothing.
    pub fn set_transition(&mut self, id: Id, kind: Option<TransitionKind>, duration: f64) -> anyhow::Result<()> {
        self.ensure_unlocked(id)?;
        // Idempotent: undo any existing overlap first so resizing never compounds.
        self.unapply_transition(id);
        let Some(kind) = kind else { return Ok(()) };
        let clip = self.clip(id).context("That clip no longer exists")?.clone();
        let prev = self.prev_on_track(&clip).filter(|p| (p.end() - clip.start).abs() < 1e-6).context("Nothing to transition from")?.clone();
        let duration = duration.clamp(MIN_CLIP, clip.duration.min(prev.duration));
        for c in self.clips.iter_mut().filter(|c| c.track == clip.track && c.start >= clip.start - 1e-6) {
            c.start -= duration;
        }
        // Crossfade the audio too, instead of just summing it louder during the overlap.
        let fade_in = clip.audio.fade_in.max(duration);
        let fade_out = prev.audio.fade_out.max(duration);
        let c = self.clip_mut(id)?;
        c.transition = Some(Transition { kind, duration });
        c.audio.fade_in = fade_in;
        self.clip_mut(prev.id)?.audio.fade_out = fade_out;
        Ok(())
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

    pub fn rename(&mut self, name: &str) -> anyhow::Result<()> {
        let name = name.trim();
        ensure!(!name.is_empty(), "Project names can't be empty");
        self.name = name.to_string();
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

    /// A number that counts up or down over its first second, then holds at `to`.
    pub fn add_counter_clip(&mut self, track: Option<Id>, start: f64, duration: f64, counter: crate::clip::CounterLayer) -> anyhow::Result<Id> {
        self.add_clip(Content::Counter(counter), track, start, duration)
    }

    /// A blur box over whatever is below it, from `start` for `duration` seconds.
    pub fn add_blur_clip(&mut self, track: Option<Id>, start: f64, duration: f64, blur: crate::clip::BlurLayer) -> anyhow::Result<Id> {
        self.add_clip(Content::Blur(blur), track, start, duration)
    }

    /// A mask box restricting whatever is below it to only its shape (or, inverted, hiding
    /// its shape), from `start` for `duration` seconds.
    pub fn add_mask_clip(&mut self, track: Option<Id>, start: f64, duration: f64, mask: crate::clip::MaskLayer) -> anyhow::Result<Id> {
        self.add_clip(Content::Mask(mask), track, start, duration)
    }

    /// A glowing icon accent: pops in by default and glows via the clip's own shadow.
    pub fn add_callout_clip(&mut self, track: Option<Id>, start: f64, duration: f64, callout: CalloutLayer) -> anyhow::Result<Id> {
        let color = callout.color;
        let id = self.add_clip(Content::Callout(callout), track, start, duration)?;
        let c = self.clip_mut(id)?;
        c.look.shadow = Shadow { enabled: true, blur: 36.0, offset_y: 0.0, opacity: 0.85, color };
        c.intro = Some(Motion { kind: MotionKind::Pop, duration: 0.35, ease: None, delay: 0.0 });
        Ok(id)
    }

    /// Puts a picture in front of everything else (the top video track) or behind it (the
    /// bottom one), at the same time. A new track is added when that one is busy then.
    pub fn arrange(&mut self, id: Id, front: bool) -> anyhow::Result<()> {
        self.ensure_unlocked(id)?;
        let clip = self.clip(id).context("That clip no longer exists")?;
        ensure!(self.clip_kind(clip) == TrackKind::Visual, "Only pictures, text and shapes are layered");
        let (start, duration, current) = (clip.start, clip.duration, clip.track);
        let visual: Vec<Id> = self.tracks.iter().filter(|t| t.kind == TrackKind::Visual).map(|t| t.id).collect();
        let edge = if front { visual.first() } else { visual.last() }.copied();
        // Already the edge track: nothing else on it overlaps, so nothing can be in front (or behind).
        if edge == Some(current) {
            return Ok(());
        }
        let target = match edge.filter(|&t| !self.track(t).is_some_and(|t| t.locked) && !self.overlaps(t, start, duration, &[id])) {
            Some(t) => t,
            None => self.new_visual_track(front),
        };
        self.clip_mut(id)?.track = target;
        Ok(())
    }

    /// Moves a picture to a new track on top of all others, at `start` (dropped above the top track).
    pub fn move_to_new_top_track(&mut self, id: Id, start: f64) -> anyhow::Result<()> {
        self.ensure_unlocked(id)?;
        let clip = self.clip(id).context("That clip no longer exists")?;
        ensure!(self.clip_kind(clip) == TrackKind::Visual, "Only pictures, text and shapes go on video tracks");
        self.detach_transition(id);
        let track = self.new_visual_track(true);
        let clip = self.clip_mut(id)?;
        clip.track = track;
        clip.start = start.max(0.0);
        Ok(())
    }

    /// A new video track above all others (`top`) or below them.
    pub fn new_visual_track(&mut self, top: bool) -> Id {
        let n = self.tracks.iter().filter(|t| t.kind == TrackKind::Visual).count() + 1;
        self.add_track(TrackKind::Visual, &format!("Video {n}"), Some(if top { 0 } else { usize::MAX }))
    }

    /// Moves a clip to `track` at `start`, nudging it to the nearest free spot.
    pub fn move_clip(&mut self, id: Id, track: Id, start: f64) -> anyhow::Result<()> {
        let clip = self.clip(id).context("That clip no longer exists")?;
        let (kind, duration) = (self.clip_kind(clip), clip.duration);
        let tr = self.track(track).context("That track no longer exists")?;
        ensure!(tr.kind == kind, "This clip can't go on that kind of track");
        ensure!(!tr.locked, "That track is locked");
        self.detach_transition(id);
        let start = self.nearest_free(track, start, duration, &[id]);
        let clip = self.clip_mut(id)?;
        clip.track = track;
        clip.start = start;
        Ok(())
    }

    /// Drags one edge of a clip to timeline time `t`, clamped to the media and the neighbours.
    pub fn trim(&mut self, id: Id, edge: Edge, t: f64) -> anyhow::Result<()> {
        self.ensure_unlocked(id)?;
        // Either edge can change the boundary a transition owns; simplest and safest is to
        // always detach rather than track exactly which edge invalidates what.
        self.detach_transition(id);
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
                for p in &mut c.presses {
                    p.at -= shift;
                }
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
        // Drop presses trimmed off the front entirely; clamp ones only partly cut into.
        c.presses.retain(|p| p.at + p.duration > 0.0 && p.at < len);
        for p in &mut c.presses {
            p.at = p.at.max(0.0);
        }
        Ok(())
    }

    /// Splits a clip at timeline time `t`. Returns the id of the new right-hand clip.
    pub fn split(&mut self, id: Id, t: f64) -> anyhow::Result<Id> {
        self.ensure_unlocked(id)?;
        let clip = self.clip(id).context("That clip no longer exists")?.clone();
        ensure!(t > clip.start + MIN_CLIP && t < clip.end() - MIN_CLIP, "Move the playhead inside the clip to split it");
        ensure!(!clip.transition.is_some_and(|tr| t < clip.start + tr.duration), "Can't split inside a transition");
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
        right.presses = clip.presses.iter().filter(|p| p.at >= cut).map(|p| crate::clip::PressPulse { at: p.at - cut, ..*p }).collect();
        let left = self.clip_mut(id)?;
        left.duration = cut;
        left.outro = None;
        left.zooms.retain(|z| z.at < cut);
        left.audio.gains.retain(|g| g.at < cut);
        left.presses.retain(|p| p.at < cut);
        self.clips.push(right);
        Ok(new_id)
    }

    pub fn delete_clips(&mut self, ids: &[Id]) -> anyhow::Result<()> {
        ensure!(!ids.is_empty(), "Nothing selected");
        for &id in ids {
            self.ensure_unlocked(id)?;
        }
        // Undo any transition touching a deleted clip first, so a surviving partner's position
        // doesn't keep an orphaned overlap pull.
        for &id in ids {
            self.detach_transition(id);
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
        // Changing this clip's own duration invalidates any transition touching it; clips merely
        // rippled below keep theirs, since a uniform shift preserves their relative overlap.
        self.detach_transition(id);
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
        for p in &mut c.presses {
            p.at *= ratio;
            p.duration *= ratio;
        }
        if let Some(m) = &mut c.intro {
            m.duration *= ratio;
            m.delay *= ratio;
        }
        if let Some(m) = &mut c.outro {
            m.duration *= ratio;
        }
        Ok(())
    }

    /// Staggers the intro of each of `ids` (in the order given) so it starts `gap_frames` after
    /// the previous one — clips with no intro motion are left alone.
    pub fn stagger_group(&mut self, ids: &[Id], gap_frames: u32, fps: u32) -> anyhow::Result<()> {
        ensure!(ids.len() > 1, "Select more than one clip to stagger their entrances");
        let gap = gap_frames as f64 / (fps.max(1) as f64);
        for (i, &id) in ids.iter().enumerate() {
            self.ensure_unlocked(id)?;
            if let Some(m) = self.clip_mut(id)?.intro.as_mut() {
                m.delay = i as f64 * gap;
            }
        }
        Ok(())
    }

    /// Shared math behind `push_in`/`pull_back`: a camera move from `at` (clip-relative seconds,
    /// clamped to the clip) to the end of the clip, built from the same zoom mechanism as
    /// `add_zoom` rather than a new primitive.
    fn camera_move(&mut self, clip_id: Id, at: f64, push_in: bool) -> anyhow::Result<()> {
        self.ensure_unlocked(clip_id)?;
        let c = self.clip(clip_id).context("That clip no longer exists")?.clone();
        ensure!(c.asset().is_some() && self.clip_has_picture(&c), "Camera moves go on a video or image clip");
        let local = at.clamp(0.0, c.duration);
        let remaining = c.duration - local;
        ensure!(remaining >= 0.3, "There's no room for a camera move here");
        let ramp = 1.5f64.min(remaining / 2.0).max(0.1);
        if push_in {
            // Ramps in, then holds: the duration runs past the clip's own end so the symmetric
            // ramp-out (built into `camera_at`) never becomes visible.
            let id = self.alloc_id();
            self.clip_mut(clip_id)?.zooms.push(Zoom { id, at: local, duration: remaining + ramp, scale: 1.15, focus_x: 0.5, focus_y: 0.5, ramp });
        } else {
            // A near-instant zoom-in, then an adjacent zoom back to normal: back-to-back zooms
            // pan straight from one to the next, with no return-to-identity dip between them.
            let a_duration = 0.05f64.min(remaining * 0.1).max(0.01);
            let b_remaining = (remaining - a_duration).max(0.1);
            let b_ramp = ramp.min(b_remaining / 2.0).max(0.05);
            let aid = self.alloc_id();
            let bid = self.alloc_id();
            let c = self.clip_mut(clip_id)?;
            c.zooms.push(Zoom { id: aid, at: local, duration: a_duration, scale: 1.3, focus_x: 0.5, focus_y: 0.5, ramp: a_duration / 2.0 });
            c.zooms.push(Zoom { id: bid, at: local + a_duration, duration: b_remaining + b_ramp, scale: 1.0, focus_x: 0.5, focus_y: 0.5, ramp: b_ramp });
        }
        Ok(())
    }

    /// A slow, continuous zoom in from normal to a closer framing, held to the end of the clip.
    pub fn push_in(&mut self, clip_id: Id, at: f64) -> anyhow::Result<()> {
        self.camera_move(clip_id, at, true)
    }

    /// Starts punched in and pulls back to reveal the whole frame, held to the end of the clip.
    pub fn pull_back(&mut self, clip_id: Id, at: f64) -> anyhow::Result<()> {
        self.camera_move(clip_id, at, false)
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

    /// One-time fix-up for projects saved before transitions became a real overlap: a clip whose
    /// `transition` still only touches (rather than overlaps) its previous clip gets pulled into
    /// a genuine overlap now, so it keeps blending instead of silently becoming a hard cut. A
    /// transition with no previous clip at all was only ever a fake "animate in"; drop it.
    pub(crate) fn migrate_transitions(&mut self) {
        let mut legacy = Vec::new();
        let mut orphaned = Vec::new();
        for c in &self.clips {
            let Some(tr) = c.transition else { continue };
            match self.prev_on_track(c) {
                Some(p) if (p.end() - c.start).abs() < 1e-6 => legacy.push((c.id, tr.kind, tr.duration)),
                Some(_) => {} // already a genuine overlap: nothing to do
                None => orphaned.push(c.id),
            }
        }
        for (id, kind, duration) in legacy {
            // Clear the legacy marker first: `set_transition` starts by *undoing* whatever
            // overlap `id.transition` already implies, which would wrongly shift `id` (and
            // everything after it) forward by `duration` if left set, since a legacy transition
            // was never actually overlapping.
            if let Ok(c) = self.clip_mut(id) {
                c.transition = None;
            }
            let _ = self.set_transition(id, Some(kind), duration);
        }
        for id in orphaned {
            if let Ok(c) = self.clip_mut(id) {
                c.transition = None;
            }
        }
    }

    /// Duplicates a clip right after itself (or the nearest free spot).
    pub fn duplicate_clip(&mut self, id: Id) -> anyhow::Result<Id> {
        self.ensure_unlocked(id)?;
        let clip = self.clip(id).context("That clip no longer exists")?.clone();
        let start = self.nearest_free(clip.track, clip.end(), clip.duration, &[]);
        let new_id = self.alloc_id();
        self.clips.push(Clip { id: new_id, start, transition: None, ..clip });
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
    fn trimming_the_start_past_a_press_pulse_drops_or_clamps_it() {
        let (mut p, a) = project_with_video(10.0);
        let c = p.add_media_clip(a, None, 0.0).unwrap();
        let clip = p.clip_mut(c).unwrap();
        clip.presses.push(crate::clip::PressPulse { id: 1, at: 1.0, duration: 0.2 });
        clip.presses.push(crate::clip::PressPulse { id: 2, at: 3.0, duration: 0.2 });
        // Trimming the start to 2.0 moves everything 2.0s earlier in clip-relative time: the
        // first press (now fully before the new start) is dropped, the second is kept and
        // re-anchored to 1.0, never left with a negative `at`.
        p.trim(c, Edge::Start, 2.0).unwrap();
        let presses = &p.clip(c).unwrap().presses;
        assert_eq!(presses.len(), 1, "the press entirely trimmed off the front is dropped");
        assert!((presses[0].at - 1.0).abs() < 1e-9, "the surviving press is re-anchored, not left negative");
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
    fn stagger_group_delays_entrances_in_order() {
        let mut p = Project::new("t", Canvas { fps: 60, ..Default::default() });
        let ids: Vec<Id> = (0..3)
            .map(|i| {
                let id = p.add_text_clip(None, i as f64, DEFAULT_STILL, TextLayer::default()).unwrap();
                p.clip_mut(id).unwrap().intro = Some(crate::clip::Motion { kind: crate::clip::MotionKind::Fade, duration: 0.5, ease: None, delay: 0.0 });
                id
            })
            .collect();
        p.stagger_group(&ids, 3, 60).unwrap();
        for (i, &id) in ids.iter().enumerate() {
            let delay = p.clip(id).unwrap().intro.unwrap().delay;
            assert!((delay - i as f64 * 3.0 / 60.0).abs() < 1e-9, "clip {i} delay {delay}");
        }
        // A clip with no intro is left alone rather than erroring.
        let no_intro = p.add_text_clip(None, 10.0, DEFAULT_STILL, TextLayer::default()).unwrap();
        p.stagger_group(&[ids[0], no_intro], 2, 60).unwrap();
        assert!(p.clip(no_intro).unwrap().intro.is_none());
        assert!(p.stagger_group(&[ids[0]], 2, 60).is_err(), "needs more than one clip");
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

    #[test]
    fn bring_to_front_and_send_to_back() {
        let mut p = Project::new("t", Canvas::default());
        let visual = |p: &Project| p.tracks.iter().filter(|t| t.kind == TrackKind::Visual).map(|t| t.id).collect::<Vec<_>>();
        let (top, bottom) = (visual(&p)[0], *visual(&p).last().unwrap());
        // A title already on the top track, and a text on the bottom one under it.
        let title = p.add_text_clip(Some(top), 0.0, 5.0, TextLayer::default()).unwrap();
        let text = p.add_text_clip(Some(bottom), 1.0, 2.0, TextLayer::default()).unwrap();
        // The top track is busy then: a new track on top.
        p.arrange(text, true).unwrap();
        let new_top = visual(&p)[0];
        assert!(new_top != top && p.clip(text).unwrap().track == new_top);
        assert_eq!(p.clip(text).unwrap().start, 1.0, "it keeps its time");
        // Already in front: nothing changes.
        let before = p.clone();
        p.arrange(text, true).unwrap();
        assert_eq!(p, before);
        // Behind everything: the bottom track is free again.
        p.arrange(text, false).unwrap();
        assert_eq!(p.clip(text).unwrap().track, bottom);
        // The title goes to the back too, but the bottom track is busy now: a new bottom track.
        p.arrange(title, false).unwrap();
        assert_eq!(p.clip(title).unwrap().track, *visual(&p).last().unwrap());
        assert_ne!(p.clip(title).unwrap().track, bottom);
        assert!(p.tracks.iter().position(|t| t.id == p.clip(title).unwrap().track) < p.tracks.iter().position(|t| t.kind == TrackKind::Audio), "video tracks stay above audio");
        // Sound can't be layered; locked clips don't move.
        let a = p.add_asset(&PathBuf::from("/a.wav"), &crate::media::ProbeInfo { kind: MediaKind::Audio, duration: 3.0, width: 0, height: 0, fps: 0.0, has_audio: true });
        let sound = p.add_media_clip(a, None, 0.0).unwrap();
        assert!(p.arrange(sound, true).is_err());
        p.track_mut(bottom).unwrap().locked = true;
        assert!(p.arrange(text, true).is_err());
        assert!(p.move_to_new_top_track(text, 0.0).is_err(), "locked clips stay put");
        // Dropped above the top track: a new track on top, at the drop time.
        p.track_mut(bottom).unwrap().locked = false;
        let tracks = p.tracks.len();
        p.move_to_new_top_track(text, 2.5).unwrap();
        assert_eq!(p.tracks.len(), tracks + 1);
        assert_eq!((p.clip(text).unwrap().track, p.clip(text).unwrap().start), (p.tracks[0].id, 2.5));
        assert!(p.move_to_new_top_track(sound, 0.0).is_err());
    }

    #[test]
    fn push_in_ramps_a_zoom_held_past_the_clips_end() {
        let (mut p, a) = project_with_video(10.0);
        let clip = p.add_media_clip(a, None, 0.0).unwrap();
        p.push_in(clip, 2.0).unwrap();
        let c = p.clip(clip).unwrap();
        assert_eq!(c.zooms.len(), 1);
        let z = c.zooms[0];
        assert_eq!(z.at, 2.0);
        assert!(z.duration > c.duration - z.at, "runs past the clip's own end so the ramp-out never shows");
        assert_eq!((z.focus_x, z.focus_y), (0.5, 0.5));
        assert!(z.scale > 1.0, "a closer framing");
    }

    #[test]
    fn pull_back_adds_two_back_to_back_zooms_with_no_gap() {
        let (mut p, a) = project_with_video(10.0);
        let clip = p.add_media_clip(a, None, 0.0).unwrap();
        p.pull_back(clip, 1.0).unwrap();
        let c = p.clip(clip).unwrap();
        assert_eq!(c.zooms.len(), 2);
        let (first, second) = (c.zooms[0], c.zooms[1]);
        assert_eq!(first.at, 1.0);
        assert!(first.scale > second.scale, "starts punched in...");
        assert_eq!(second.scale, 1.0, "...and settles back to normal");
        assert!((second.at - (first.at + first.duration)).abs() < 1e-9, "back-to-back: no return-to-identity dip between them");
    }

    #[test]
    fn camera_moves_refuse_non_picture_clips_and_clips_with_no_room_left() {
        let mut p = Project::new("t", Canvas::default());
        let text = p.add_text_clip(None, 0.0, 5.0, TextLayer::default()).unwrap();
        assert!(p.push_in(text, 0.0).is_err(), "text has no camera to move");
        let (mut p, a) = project_with_video(10.0);
        let clip = p.add_media_clip(a, None, 0.0).unwrap();
        assert!(p.push_in(clip, 9.9).is_err(), "not enough room left for a camera move");
    }

    #[test]
    fn callouts_pop_in_and_glow_by_default() {
        let mut p = Project::new("t", Canvas::default());
        let layer = crate::clip::CalloutLayer::new(crate::clip::CalloutIcon::Check, crate::clip::Rgba::hex(0x2EF2A2));
        let id = p.add_callout_clip(None, 2.0, 1.3, layer).unwrap();
        let c = p.clip(id).unwrap();
        assert!(matches!(c.content, Content::Callout(cal) if cal.icon == crate::clip::CalloutIcon::Check));
        assert!(c.look.shadow.enabled, "the glow is on by default");
        assert_eq!(c.look.shadow.color, crate::clip::Rgba::hex(0x2EF2A2), "glow matches the icon color");
        assert_eq!(c.intro.map(|m| m.kind), Some(crate::clip::MotionKind::Pop), "pops in by default");
    }

    fn video_asset(p: &mut Project, len: f64) -> Id {
        let info = ProbeInfo { kind: MediaKind::Video, duration: len, width: 1920, height: 1080, fps: 30.0, has_audio: true };
        p.add_asset(&PathBuf::from(format!("/v{len}.mp4")), &info)
    }

    /// `c1` (4s, at 0) touching `c2` (10s, at 4) touching `c3` (10s, at 14), all on one track.
    fn three_clips_in_a_row(p: &mut Project) -> (Id, Id, Id) {
        let a4 = video_asset(p, 4.0);
        let a10 = video_asset(p, 10.0);
        let c1 = p.add_media_clip(a4, None, 0.0).unwrap();
        let track = p.clip(c1).unwrap().track;
        let c2 = p.add_media_clip(a10, Some(track), 4.0).unwrap();
        let c3 = p.add_media_clip(a10, Some(track), 14.0).unwrap();
        (c1, c2, c3)
    }

    #[test]
    fn set_transition_creates_overlap_and_ripples_later_clips() {
        let mut p = Project::new("t", Canvas::default());
        let (c1, c2, c3) = three_clips_in_a_row(&mut p);
        p.set_transition(c2, Some(TransitionKind::Crossfade), 1.0).unwrap();
        assert_eq!(p.clip(c1).unwrap().start, 0.0, "the outgoing clip doesn't move");
        assert_eq!(p.clip(c2).unwrap().start, 3.0, "pulled earlier by the transition's duration");
        assert_eq!(p.clip(c2).unwrap().transition.map(|t| (t.kind, t.duration)), Some((TransitionKind::Crossfade, 1.0)));
        assert_eq!(p.clip(c3).unwrap().start, 13.0, "later clips ripple along with it");
    }

    #[test]
    fn resizing_a_transition_does_not_compound() {
        let mut p = Project::new("t", Canvas::default());
        let (_, c2, c3) = three_clips_in_a_row(&mut p);
        p.set_transition(c2, Some(TransitionKind::Crossfade), 1.0).unwrap();
        p.set_transition(c2, Some(TransitionKind::Crossfade), 2.0).unwrap();
        assert_eq!(p.clip(c2).unwrap().start, 2.0, "re-based on the original cut, not the already-shifted one");
        assert_eq!(p.clip(c3).unwrap().start, 12.0);
    }

    #[test]
    fn removing_a_transition_restores_the_gapless_layout() {
        let mut p = Project::new("t", Canvas::default());
        let (_, c2, c3) = three_clips_in_a_row(&mut p);
        p.set_transition(c2, Some(TransitionKind::Crossfade), 1.0).unwrap();
        p.set_transition(c2, None, 0.0).unwrap();
        assert_eq!(p.clip(c2).unwrap().start, 4.0);
        assert!(p.clip(c2).unwrap().transition.is_none());
        assert_eq!(p.clip(c3).unwrap().start, 14.0);
    }

    #[test]
    fn transition_requires_a_touching_previous_clip() {
        let mut p = Project::new("t", Canvas::default());
        let a = video_asset(&mut p, 4.0);
        let first = p.add_media_clip(a, None, 0.0).unwrap();
        assert!(p.set_transition(first, Some(TransitionKind::Crossfade), 1.0).is_err(), "nothing before the first clip");
        let track = p.clip(first).unwrap().track;
        let after_gap = p.add_media_clip(a, Some(track), 10.0).unwrap();
        assert!(p.set_transition(after_gap, Some(TransitionKind::Crossfade), 1.0).is_err(), "a gap isn't touching");
    }

    #[test]
    fn migrating_a_legacy_touching_transition_creates_a_real_overlap() {
        let mut p = Project::new("t", Canvas::default());
        let (c1, c2, c3) = three_clips_in_a_row(&mut p);
        // A pre-overlap save: `transition` set but the clips still only touch, like a project
        // saved before transitions became a real overlap.
        p.clip_mut(c2).unwrap().transition = Some(Transition { kind: TransitionKind::Crossfade, duration: 1.0 });
        p.migrate_transitions();
        assert_eq!(p.clip(c1).unwrap().start, 0.0, "the outgoing clip doesn't move");
        assert_eq!(p.clip(c2).unwrap().start, 3.0, "pulled earlier into a genuine overlap, not left touching or gapped");
        assert_eq!(p.clip(c2).unwrap().transition.map(|t| (t.kind, t.duration)), Some((TransitionKind::Crossfade, 1.0)));
        assert_eq!(p.clip(c3).unwrap().start, 13.0, "later clips ripple along with it, with no gap introduced");
    }

    #[test]
    fn migrating_an_orphaned_transition_just_drops_it() {
        let mut p = Project::new("t", Canvas::default());
        let a = video_asset(&mut p, 4.0);
        let first = p.add_media_clip(a, None, 0.0).unwrap();
        p.clip_mut(first).unwrap().transition = Some(Transition { kind: TransitionKind::Crossfade, duration: 1.0 });
        p.migrate_transitions();
        assert_eq!(p.clip(first).unwrap().start, 0.0);
        assert!(p.clip(first).unwrap().transition.is_none(), "nothing before the first clip to blend from");
    }

    #[test]
    fn moving_a_transitioned_clip_detaches_it() {
        let mut p = Project::new("t", Canvas::default());
        let (_, c2, c3) = three_clips_in_a_row(&mut p);
        let track = p.clip(c2).unwrap().track;
        p.set_transition(c2, Some(TransitionKind::Crossfade), 1.0).unwrap();
        p.move_clip(c2, track, 30.0).unwrap();
        assert!(p.clip(c2).unwrap().transition.is_none());
        assert_eq!(p.clip(c3).unwrap().start, 14.0, "the rest of the track is un-rippled");
    }

    #[test]
    fn trimming_the_outgoing_clip_detaches_its_dependents_transition() {
        let mut p = Project::new("t", Canvas::default());
        let (c1, c2, _) = three_clips_in_a_row(&mut p);
        p.set_transition(c2, Some(TransitionKind::Crossfade), 1.0).unwrap();
        p.trim(c1, Edge::End, 2.0).unwrap();
        assert!(p.clip(c2).unwrap().transition.is_none());
        assert_eq!(p.clip(c2).unwrap().start, 4.0);
    }

    #[test]
    fn splitting_inside_a_transition_window_is_rejected() {
        let mut p = Project::new("t", Canvas::default());
        let (_, c2, _) = three_clips_in_a_row(&mut p);
        p.set_transition(c2, Some(TransitionKind::Crossfade), 1.0).unwrap();
        // c2 now runs 3.0..13.0, with the shared overlap over 3.0..4.0.
        assert!(p.split(c2, 3.5).is_err(), "would split inside the blend");
        assert!(p.split(c2, 5.0).is_ok());
    }

    #[test]
    fn deleting_a_transition_partner_detaches_the_survivor() {
        let mut p = Project::new("t", Canvas::default());
        let (c1, c2, c3) = three_clips_in_a_row(&mut p);
        p.set_transition(c2, Some(TransitionKind::Crossfade), 1.0).unwrap();
        p.delete_clips(&[c1]).unwrap();
        assert!(p.clip(c2).unwrap().transition.is_none());
        assert_eq!(p.clip(c2).unwrap().start, 4.0);
        assert_eq!(p.clip(c3).unwrap().start, 14.0);
    }

    #[test]
    fn duplicating_a_transitioned_clip_does_not_copy_it() {
        let mut p = Project::new("t", Canvas::default());
        let (_, c2, _) = three_clips_in_a_row(&mut p);
        p.set_transition(c2, Some(TransitionKind::Crossfade), 1.0).unwrap();
        let dup = p.duplicate_clip(c2).unwrap();
        assert!(p.clip(dup).unwrap().transition.is_none());
    }

    #[test]
    fn set_speed_clears_its_own_transition_but_preserves_a_rippled_one() {
        let mut p = Project::new("t", Canvas::default());
        // c0, then an untouched-by-any-transition cMid, then a c1-into-c2 transition further
        // along — resizing c0 should just uniformly ripple all three without touching c1/c2's pair.
        let a4 = video_asset(&mut p, 4.0);
        let a10 = video_asset(&mut p, 10.0);
        let c0 = p.add_media_clip(a4, None, 0.0).unwrap();
        let track = p.clip(c0).unwrap().track;
        let mid = p.add_media_clip(a10, Some(track), 4.0).unwrap();
        let c1 = p.add_media_clip(a10, Some(track), 14.0).unwrap();
        let c2 = p.add_media_clip(a10, Some(track), 24.0).unwrap();
        p.set_transition(c2, Some(TransitionKind::Crossfade), 1.0).unwrap();
        assert_eq!((p.clip(c1).unwrap().start, p.clip(c2).unwrap().start), (14.0, 23.0));
        p.set_speed(c0, 2.0).unwrap();
        assert_eq!(p.clip(c0).unwrap().duration, 2.0, "half the duration at double speed");
        // Everything after ripples by the same delta (-2.0), keeping c1/c2's overlap intact.
        assert_eq!(p.clip(mid).unwrap().start, 2.0);
        assert_eq!(p.clip(c1).unwrap().start, 12.0);
        assert_eq!(p.clip(c2).unwrap().start, 21.0);
        assert_eq!(p.clip(c2).unwrap().transition.map(|t| t.duration), Some(1.0), "a uniform ripple preserves it");
        // But resizing the outgoing clip of a pair directly must detach that pair's transition.
        p.set_speed(c1, 2.0).unwrap();
        assert!(p.clip(c2).unwrap().transition.is_none(), "resizing the outgoing clip itself detaches it");
    }
}
