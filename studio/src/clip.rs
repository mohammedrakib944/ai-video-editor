//! Clips and everything that can be set on them.
//!
//! Positions are fractions of the canvas (0..1, so they survive resolution
//! changes). Lengths such as corner radius, border width and font size are in
//! "canvas units": pixels at a 1080-pixel short side, scaled for other sizes.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::model::Id;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rgba(pub f32, pub f32, pub f32, pub f32);

impl Rgba {
    pub const WHITE: Rgba = Rgba(1.0, 1.0, 1.0, 1.0);
    pub const BLACK: Rgba = Rgba(0.0, 0.0, 0.0, 1.0);

    pub fn hex(rgb: u32) -> Rgba {
        Rgba(((rgb >> 16) & 0xff) as f32 / 255.0, ((rgb >> 8) & 0xff) as f32 / 255.0, (rgb & 0xff) as f32 / 255.0, 1.0)
    }

    pub fn to_hex(self) -> String {
        let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        format!("#{:02X}{:02X}{:02X}", c(self.0), c(self.1), c(self.2))
    }

    pub fn parse(s: &str) -> Option<Rgba> {
        let s = s.trim().trim_start_matches('#');
        (s.len() == 6).then(|| u32::from_str_radix(s, 16).ok().map(Rgba::hex)).flatten()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Fit {
    /// Whole frame visible (letterboxed if needed).
    #[default]
    Contain,
    /// Frame fills the canvas (edges cropped).
    Cover,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Transform {
    pub x: f64,
    pub y: f64,
    pub scale: f64,
    pub rotation: f64,
    pub opacity: f64,
    pub fit: Fit,
}

impl Default for Transform {
    fn default() -> Self {
        Transform { x: 0.5, y: 0.5, scale: 1.0, rotation: 0.0, opacity: 1.0, fit: Fit::Contain }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shadow {
    pub enabled: bool,
    pub blur: f64,
    pub offset_y: f64,
    pub opacity: f64,
    pub color: Rgba,
}

impl Default for Shadow {
    fn default() -> Self {
        Shadow { enabled: false, blur: 40.0, offset_y: 16.0, opacity: 0.45, color: Rgba::BLACK }
    }
}

/// Frame styling for media and shapes: rounded corners, border, drop shadow.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Look {
    pub corner_radius: f64,
    pub border_width: f64,
    pub border_color: Rgba,
    pub shadow: Shadow,
}

impl Default for Look {
    fn default() -> Self {
        Look { corner_radius: 0.0, border_width: 0.0, border_color: Rgba(1.0, 1.0, 1.0, 0.9), shadow: Shadow::default() }
    }
}

/// Basic color grading. 0 means unchanged for every field.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Grade {
    pub brightness: f64,
    pub contrast: f64,
    pub saturation: f64,
    pub temperature: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Audio {
    pub volume: f64,
    pub muted: bool,
    pub fade_in: f64,
    pub fade_out: f64,
}

impl Default for Audio {
    fn default() -> Self {
        Audio { volume: 1.0, muted: false, fade_in: 0.0, fade_out: 0.0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Ease {
    Linear,
    #[default]
    Smooth,
    EaseIn,
    EaseOut,
    /// Overshoots slightly, for pops.
    Back,
}

impl Ease {
    pub fn apply(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Ease::Linear => t,
            // Cubic in-out.
            Ease::Smooth => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                }
            }
            Ease::EaseIn => t * t * t,
            Ease::EaseOut => 1.0 - (1.0 - t).powi(3),
            Ease::Back => {
                let (c1, c3) = (1.70158, 2.70158);
                1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
            }
        }
    }
}

/// A Camtasia-style zoom on part of the frame. Times are relative to the clip start.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Zoom {
    pub id: Id,
    pub at: f64,
    pub duration: f64,
    pub scale: f64,
    /// Point of the frame to zoom into, as fractions of the frame.
    pub focus_x: f64,
    pub focus_y: f64,
    /// Seconds to ease in and out.
    pub ramp: f64,
}

/// The camera at one moment: how much the frame is magnified and where.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub scale: f64,
    pub focus_x: f64,
    pub focus_y: f64,
}

impl Camera {
    pub const IDENTITY: Camera = Camera { scale: 1.0, focus_x: 0.5, focus_y: 0.5 };

    fn lerp(a: Camera, b: Camera, t: f64) -> Camera {
        Camera { scale: a.scale + (b.scale - a.scale) * t, focus_x: a.focus_x + (b.focus_x - a.focus_x) * t, focus_y: a.focus_y + (b.focus_y - a.focus_y) * t }
    }
}

/// Camera at clip-relative time `t`. Back-to-back zooms pan directly from one to the next.
pub fn camera_at(zooms: &[Zoom], t: f64) -> Camera {
    let mut sorted: Vec<&Zoom> = zooms.iter().collect();
    sorted.sort_by(|a, b| a.at.total_cmp(&b.at));
    for (i, z) in sorted.iter().enumerate() {
        let target = Camera { scale: z.scale, focus_x: z.focus_x, focus_y: z.focus_y };
        let (start, end) = (z.at, z.at + z.duration);
        if t < start || t > end {
            continue;
        }
        let ramp = z.ramp.clamp(0.0, z.duration / 2.0).max(1e-6);
        // Coming from the previous zoom if it ends exactly where this one starts.
        let from = match i.checked_sub(1).map(|j| sorted[j]) {
            Some(p) if (p.at + p.duration - start).abs() < 1e-3 => Camera { scale: p.scale, focus_x: p.focus_x, focus_y: p.focus_y },
            _ => Camera::IDENTITY,
        };
        let next_adjacent = sorted.get(i + 1).is_some_and(|n| (n.at - end).abs() < 1e-3);
        if t < start + ramp {
            return Camera::lerp(from, target, Ease::Smooth.apply((t - start) / ramp));
        }
        if t > end - ramp && !next_adjacent {
            return Camera::lerp(target, Camera::IDENTITY, Ease::Smooth.apply((t - (end - ramp)) / ramp));
        }
        return target;
    }
    Camera::IDENTITY
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Prop {
    X,
    Y,
    Scale,
    Rotation,
    Opacity,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    /// Clip-relative time.
    pub t: f64,
    pub value: f64,
    /// Easing from this keyframe to the next.
    pub ease: Ease,
}

/// Value of an animated property at `t`, or `None` if it has no keyframes.
pub fn sample(keys: &[Keyframe], t: f64) -> Option<f64> {
    let first = keys.first()?;
    if t <= first.t {
        return Some(first.value);
    }
    for w in keys.windows(2) {
        let (a, b) = (w[0], w[1]);
        if t <= b.t {
            let span = (b.t - a.t).max(1e-9);
            return Some(a.value + (b.value - a.value) * a.ease.apply((t - a.t) / span));
        }
    }
    Some(keys.last().unwrap().value)
}

/// Ready-made entrance/exit animations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MotionKind {
    Fade,
    SlideUp,
    SlideDown,
    SlideLeft,
    SlideRight,
    Pop,
    ZoomIn,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Motion {
    pub kind: MotionKind,
    pub duration: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransitionKind {
    Crossfade,
    SlideLeft,
    SlideUp,
    Push,
    ZoomThrough,
    Wipe,
}

/// Transition from the previous clip on the same track into this one.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transition {
    pub kind: TransitionKind,
    pub duration: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Align {
    Left,
    #[default]
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextLayer {
    pub text: String,
    pub font: String,
    pub weight: u16,
    pub size: f64,
    pub color: Rgba,
    pub align: Align,
    pub line_height: f64,
    pub letter_spacing: f64,
    /// Optional box behind the text.
    pub background: Option<Rgba>,
    pub padding: f64,
}

impl Default for TextLayer {
    fn default() -> Self {
        TextLayer {
            text: "Your headline".into(),
            font: "Inter".into(),
            weight: 700,
            size: 96.0,
            color: Rgba::WHITE,
            align: Align::Center,
            line_height: 1.15,
            letter_spacing: -1.0,
            background: None,
            padding: 24.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShapeKind {
    Rectangle,
    Ellipse,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ShapeLayer {
    pub kind: ShapeKind,
    /// Size in canvas units.
    pub width: f64,
    pub height: f64,
    pub fill: Rgba,
    /// Second color for a vertical gradient.
    pub fill_to: Option<Rgba>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Content {
    /// Video, image or audio from the media library. `src_in` is where playback starts in the file.
    Media { asset: Id, src_in: f64, speed: f64 },
    Text(TextLayer),
    Shape(ShapeLayer),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: Id,
    pub track: Id,
    /// Timeline position and length, in seconds.
    pub start: f64,
    pub duration: f64,
    pub content: Content,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub transform: Transform,
    #[serde(default)]
    pub look: Look,
    #[serde(default)]
    pub grade: Grade,
    #[serde(default)]
    pub audio: Audio,
    #[serde(default)]
    pub zooms: Vec<Zoom>,
    #[serde(default)]
    pub keyframes: BTreeMap<Prop, Vec<Keyframe>>,
    #[serde(default)]
    pub intro: Option<Motion>,
    #[serde(default)]
    pub outro: Option<Motion>,
    #[serde(default)]
    pub transition: Option<Transition>,
}

impl Clip {
    pub fn new(id: Id, track: Id, start: f64, duration: f64, content: Content) -> Clip {
        Clip {
            id,
            track,
            start,
            duration,
            content,
            name: None,
            transform: Transform::default(),
            look: Look::default(),
            grade: Grade::default(),
            audio: Audio::default(),
            zooms: Vec::new(),
            keyframes: BTreeMap::new(),
            intro: None,
            outro: None,
            transition: None,
        }
    }

    pub fn end(&self) -> f64 {
        self.start + self.duration
    }

    pub fn asset(&self) -> Option<Id> {
        match self.content {
            Content::Media { asset, .. } => Some(asset),
            _ => None,
        }
    }

    pub fn speed(&self) -> f64 {
        match self.content {
            Content::Media { speed, .. } => speed,
            _ => 1.0,
        }
    }

    /// Source-file time shown at timeline time `t`.
    pub fn source_time(&self, t: f64) -> f64 {
        match self.content {
            Content::Media { src_in, speed, .. } => src_in + (t - self.start) * speed,
            _ => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zoom(at: f64, duration: f64, scale: f64, fx: f64) -> Zoom {
        Zoom { id: 1, at, duration, scale, focus_x: fx, focus_y: 0.5, ramp: 0.5 }
    }

    #[test]
    fn camera_ramps_in_holds_and_ramps_out() {
        let z = [zoom(1.0, 3.0, 2.0, 0.25)];
        assert_eq!(camera_at(&z, 0.5), Camera::IDENTITY);
        assert_eq!(camera_at(&z, 2.0).scale, 2.0);
        let mid_in = camera_at(&z, 1.25);
        assert!(mid_in.scale > 1.0 && mid_in.scale < 2.0);
        let mid_out = camera_at(&z, 3.75);
        assert!(mid_out.scale > 1.0 && mid_out.scale < 2.0);
        assert_eq!(camera_at(&z, 5.0), Camera::IDENTITY);
    }

    #[test]
    fn adjacent_zooms_pan_without_zooming_out() {
        let z = [zoom(1.0, 2.0, 2.0, 0.25), zoom(3.0, 2.0, 2.0, 0.75)];
        // End of the first zoom holds instead of ramping out.
        assert_eq!(camera_at(&z, 2.9).scale, 2.0);
        // Start of the second pans across at full zoom.
        let pan = camera_at(&z, 3.25);
        assert!((pan.scale - 2.0).abs() < 1e-9);
        assert!(pan.focus_x > 0.25 && pan.focus_x < 0.75);
    }

    #[test]
    fn keyframes_interpolate_with_easing() {
        let keys = [Keyframe { t: 0.0, value: 0.0, ease: Ease::Linear }, Keyframe { t: 2.0, value: 10.0, ease: Ease::Linear }];
        assert_eq!(sample(&keys, -1.0), Some(0.0));
        assert_eq!(sample(&keys, 1.0), Some(5.0));
        assert_eq!(sample(&keys, 3.0), Some(10.0));
        assert_eq!(sample(&[], 1.0), None);
    }

    #[test]
    fn colors_round_trip_through_hex() {
        assert_eq!(Rgba::parse("#7C6DFF").unwrap().to_hex(), "#7C6DFF");
        assert!(Rgba::parse("nope").is_none());
    }
}
