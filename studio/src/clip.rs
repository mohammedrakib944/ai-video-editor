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

/// Cropping, as fractions of the source frame cut from each edge.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Crop {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

impl Crop {
    /// Largest total crop on one axis, so something always stays visible.
    pub const MAX: f64 = 0.95;

    pub fn is_none(&self) -> bool {
        *self == Crop::default()
    }

    /// The same crop with every edge in range and opposite edges leaving at least 5% visible.
    pub fn clamped(self) -> Crop {
        let c = |v: f64| v.clamp(0.0, Crop::MAX);
        let (mut l, mut t, mut r, mut b) = (c(self.left), c(self.top), c(self.right), c(self.bottom));
        if l + r > Crop::MAX {
            let k = Crop::MAX / (l + r);
            (l, r) = (l * k, r * k);
        }
        if t + b > Crop::MAX {
            let k = Crop::MAX / (t + b);
            (t, b) = (t * k, b * k);
        }
        Crop { left: l, top: t, right: r, bottom: b }
    }
}

/// A blur clip: blurs the video and images under it, inside its box, while it's on
/// the timeline. It sits on a video track like a highlight; position comes from the
/// clip's transform, the box size is in canvas units.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BlurLayer {
    pub width: f64,
    pub height: f64,
    /// 0..100.
    pub strength: f64,
    #[serde(default)]
    pub ellipse: bool,
}

impl Default for BlurLayer {
    fn default() -> Self {
        BlurLayer { width: 560.0, height: 200.0, strength: 60.0, ellipse: false }
    }
}

/// Seconds a blur takes to fade in and out at the ends of its clip.
pub const BLUR_FADE: f64 = 0.15;

/// Most blur boxes that can cover one layer at the same moment.
pub const MAX_BLURS: usize = 4;

/// Which part of a video clip plays: after "Separate audio" the picture and the
/// sound are two clips of the same media.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Part {
    #[default]
    Whole,
    /// Picture only (its sound was separated).
    Picture,
    /// Sound only, on an audio track.
    Sound,
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

/// A volume change on part of a clip (from a range selection). Times are clip-relative.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Gain {
    pub id: Id,
    pub at: f64,
    pub duration: f64,
    /// Multiplies the clip volume inside the span (0 = silent).
    pub volume: f64,
}

/// Length of the smooth ramp at each edge of a `Gain`, so changes don't click.
pub const GAIN_RAMP: f64 = 0.06;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Audio {
    pub volume: f64,
    pub muted: bool,
    pub fade_in: f64,
    pub fade_out: f64,
    pub gains: Vec<Gain>,
}

impl Default for Audio {
    fn default() -> Self {
        Audio { volume: 1.0, muted: false, fade_in: 0.0, fade_out: 0.0, gains: Vec::new() }
    }
}

impl Audio {
    /// The range-gain factor at clip-relative time `t` (1 outside every range).
    pub fn gain_at(&self, t: f64) -> f64 {
        self.gains
            .iter()
            .map(|g| {
                let ramp = GAIN_RAMP.min(g.duration / 2.0).max(1e-4);
                let k = ((t - g.at) / ramp).min((g.at + g.duration - t) / ramp).clamp(0.0, 1.0);
                1.0 + (g.volume - 1.0) * k
            })
            .product()
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
    /// Words styled differently from the rest: sorted, non-overlapping character ranges.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub spans: Vec<TextSpan>,
}

/// Part of a text layer (characters `start..end`, counted in Unicode chars) with its own style.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextSpan {
    pub start: usize,
    pub end: usize,
    #[serde(flatten)]
    pub style: SpanStyle,
}

/// What a span changes; unset fields follow the layer.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpanStyle {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<Rgba>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weight: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// Size relative to the layer's (1.0 = the same).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    /// A marker-pen box behind the characters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlight: Option<Rgba>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub underline: bool,
}

impl SpanStyle {
    pub fn is_plain(&self) -> bool {
        *self == SpanStyle::default()
    }
}

/// One change to the style of part of a text layer.
#[derive(Debug, Clone, PartialEq)]
pub enum SpanChange {
    Color(Option<Rgba>),
    Weight(Option<u16>),
    Font(Option<String>),
    Scale(Option<f64>),
    Highlight(Option<Rgba>),
    Underline(bool),
    /// Back to the layer's style.
    Clear,
}

impl SpanChange {
    fn apply(&self, s: &mut SpanStyle) {
        match self.clone() {
            SpanChange::Color(c) => s.color = c,
            SpanChange::Weight(w) => s.weight = w,
            SpanChange::Font(f) => s.font = f,
            SpanChange::Scale(v) => s.scale = v.map(|v| v.clamp(0.25, 4.0)).filter(|v| (v - 1.0).abs() > 1e-6),
            SpanChange::Highlight(c) => s.highlight = c,
            SpanChange::Underline(on) => s.underline = on,
            SpanChange::Clear => *s = SpanStyle::default(),
        }
    }
}

impl TextLayer {
    fn char_len(&self) -> usize {
        self.text.chars().count()
    }

    /// The style of the character at `i` (plain if no span covers it).
    pub fn style_at(&self, i: usize) -> SpanStyle {
        self.spans.iter().find(|s| s.start <= i && i < s.end).map(|s| s.style.clone()).unwrap_or_default()
    }

    /// Applies `change` to characters `start..end`, keeping whatever else those characters had.
    pub fn style_range(&mut self, start: usize, end: usize, change: &SpanChange) {
        let end = end.min(self.char_len());
        if start >= end {
            return;
        }
        // Cut the spans at the range's edges, so every span is fully inside or outside it.
        let mut runs: Vec<TextSpan> = Vec::new();
        for s in self.spans.drain(..) {
            for (a, b) in [(s.start, s.end.min(start)), (s.start.max(start), s.end.min(end)), (s.start.max(end), s.end)] {
                if a < b {
                    runs.push(TextSpan { start: a, end: b, style: s.style.clone() });
                }
            }
        }
        // Plain gaps inside the range become spans too, so they take the change.
        let mut at = start;
        let inside: Vec<(usize, usize)> = runs.iter().filter(|r| r.start >= start && r.end <= end).map(|r| (r.start, r.end)).collect();
        for (a, b) in inside {
            if at < a {
                runs.push(TextSpan { start: at, end: a, style: SpanStyle::default() });
            }
            at = b;
        }
        if at < end {
            runs.push(TextSpan { start: at, end, style: SpanStyle::default() });
        }
        for r in runs.iter_mut().filter(|r| r.start >= start && r.end <= end) {
            change.apply(&mut r.style);
        }
        self.spans = runs;
        self.normalize_spans();
    }

    /// Replaces the text, keeping each style on the characters it was on: text typed inside
    /// a styled word takes its style, text typed at its edges doesn't, and typing over a
    /// selection keeps the style it started with.
    pub fn set_text(&mut self, text: String) {
        let old: Vec<char> = self.text.chars().collect();
        let new: Vec<char> = text.chars().collect();
        let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
        let max_suffix = old.len().min(new.len()) - prefix;
        let suffix = old.iter().rev().zip(new.iter().rev()).take(max_suffix).take_while(|(a, b)| a == b).count();
        let (old_tail, new_tail) = (old.len() - suffix, new.len() - suffix);
        // Characters before the change stay; those after it move with the length change;
        // a span reaching into the replaced part covers the new text there.
        let start = |p: usize| if p < prefix { p } else if p >= old_tail { p - old_tail + new_tail } else { prefix };
        let end = |p: usize| if p <= prefix { p } else if p >= old_tail { p - old_tail + new_tail } else { new_tail };
        for s in &mut self.spans {
            (s.start, s.end) = (start(s.start), end(s.end));
        }
        self.text = text;
        self.normalize_spans();
    }

    /// Sorted, inside the text, no empty or plain spans, neighbors with the same style merged.
    fn normalize_spans(&mut self) {
        let len = self.char_len();
        let mut spans: Vec<TextSpan> = std::mem::take(&mut self.spans)
            .into_iter()
            .map(|s| TextSpan { start: s.start.min(len), end: s.end.min(len), style: s.style })
            .filter(|s| s.start < s.end && !s.style.is_plain())
            .collect();
        spans.sort_by_key(|s| s.start);
        let mut out: Vec<TextSpan> = Vec::new();
        for mut s in spans {
            if let Some(last) = out.last_mut() {
                // Overlaps (from hand-edited files or the AI) keep the earlier span.
                s.start = s.start.max(last.end);
                if s.start >= s.end {
                    continue;
                }
                if last.end == s.start && last.style == s.style {
                    last.end = s.end;
                    continue;
                }
            }
            out.push(s);
        }
        self.spans = out;
    }

    /// Byte ranges of the text with the style of each (plain parts included), for rendering.
    pub fn styled_runs(&self) -> Vec<(std::ops::Range<usize>, Option<&SpanStyle>)> {
        let bytes: Vec<usize> = self.text.char_indices().map(|(b, _)| b).chain(std::iter::once(self.text.len())).collect();
        let byte = |c: usize| bytes[c.min(bytes.len() - 1)];
        let mut out = Vec::new();
        let mut at = 0;
        for s in &self.spans {
            if s.start > at {
                out.push((byte(at)..byte(s.start), None));
            }
            out.push((byte(s.start)..byte(s.end), Some(&s.style)));
            at = s.end;
        }
        if byte(at) < self.text.len() {
            out.push((byte(at)..self.text.len(), None));
        }
        out
    }
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
            spans: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShapeKind {
    Rectangle,
    Ellipse,
    Triangle,
    Star,
    Hexagon,
    /// An outlined ellipse.
    Ring,
    /// An outlined (rounded) rectangle.
    Frame,
}

impl std::fmt::Display for ShapeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

impl ShapeKind {
    pub const ALL: [ShapeKind; 7] = [ShapeKind::Rectangle, ShapeKind::Ellipse, ShapeKind::Triangle, ShapeKind::Star, ShapeKind::Hexagon, ShapeKind::Ring, ShapeKind::Frame];

    pub fn label(self) -> &'static str {
        match self {
            ShapeKind::Rectangle => "Rectangle",
            ShapeKind::Ellipse => "Ellipse",
            ShapeKind::Triangle => "Triangle",
            ShapeKind::Star => "Star",
            ShapeKind::Hexagon => "Hexagon",
            ShapeKind::Ring => "Ring",
            ShapeKind::Frame => "Frame",
        }
    }
}

fn default_angle() -> f64 {
    180.0
}

fn default_thickness() -> f64 {
    24.0
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ShapeLayer {
    pub kind: ShapeKind,
    /// Size in canvas units.
    pub width: f64,
    pub height: f64,
    pub fill: Rgba,
    /// Second color, for a gradient.
    pub fill_to: Option<Rgba>,
    /// Gradient direction in degrees, CSS-style: 180 runs top to bottom, 90 left to right.
    #[serde(default = "default_angle")]
    pub gradient_angle: f64,
    /// A radial gradient from the center instead of a linear one.
    #[serde(default)]
    pub radial: bool,
    /// Outline thickness for rings and frames, canvas units.
    #[serde(default = "default_thickness")]
    pub thickness: f64,
}

impl ShapeLayer {
    pub fn new(kind: ShapeKind, width: f64, height: f64, fill: Rgba, fill_to: Option<Rgba>) -> ShapeLayer {
        ShapeLayer { kind, width, height, fill, fill_to, gradient_angle: 180.0, radial: false, thickness: 24.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Content {
    /// Video, image or audio from the media library. `src_in` is where playback starts in the file.
    Media { asset: Id, src_in: f64, speed: f64 },
    Text(TextLayer),
    Shape(ShapeLayer),
    Blur(BlurLayer),
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
    #[serde(default)]
    pub crop: Crop,
    #[serde(default)]
    pub part: Part,
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
            crop: Crop::default(),
            part: Part::Whole,
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

    /// Timeline time at which source-file time `s` plays: the inverse of `source_time`.
    pub fn timeline_time(&self, s: f64) -> f64 {
        match self.content {
            Content::Media { src_in, speed, .. } => self.start + (s - src_in) / speed,
            _ => self.start,
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
    fn timeline_time_is_the_inverse_of_source_time() {
        let c = Clip::new(1, 1, 10.0, 5.0, Content::Media { asset: 1, src_in: 2.0, speed: 2.0 });
        // At timeline 10s (the clip's start) the source is at src_in; 1s of timeline is 2s of
        // source at 2x speed.
        assert_eq!(c.source_time(10.0), 2.0);
        assert_eq!(c.source_time(11.0), 4.0);
        assert_eq!(c.timeline_time(2.0), 10.0);
        assert_eq!(c.timeline_time(4.0), 11.0);
        for t in [10.0, 10.5, 12.3] {
            assert!((c.timeline_time(c.source_time(t)) - t).abs() < 1e-9, "round trip at t={t}");
        }
    }

    #[test]
    fn crop_always_leaves_part_of_the_frame() {
        let c = Crop { left: 0.8, top: -0.2, right: 0.6, bottom: 0.3 }.clamped();
        assert!((c.left + c.right - Crop::MAX).abs() < 1e-9, "opposite edges are scaled back");
        assert!(c.left > c.right, "keeping their proportions");
        assert_eq!((c.top, c.bottom), (0.0, 0.3));
        assert!(Crop::default().is_none());
    }

    #[test]
    fn colors_round_trip_through_hex() {
        assert_eq!(Rgba::parse("#7C6DFF").unwrap().to_hex(), "#7C6DFF");
        assert!(Rgba::parse("nope").is_none());
    }

    fn styled(text: &str) -> TextLayer {
        TextLayer { text: text.into(), ..Default::default() }
    }

    /// The styled words, as text.
    fn words(t: &TextLayer) -> Vec<String> {
        t.spans.iter().map(|s| t.text.chars().skip(s.start).take(s.end - s.start).collect()).collect()
    }

    #[test]
    fn styling_part_of_a_text_keeps_the_rest() {
        let mut t = styled("Make it fast today");
        t.style_range(8, 12, &SpanChange::Color(Some(Rgba::hex(0xD6FF00))));
        assert_eq!(words(&t), ["fast"]);
        // Bold over a range that half overlaps: the overlap keeps its color too.
        t.style_range(5, 10, &SpanChange::Weight(Some(800)));
        assert_eq!(words(&t), ["it ", "fa", "st"]);
        assert_eq!(t.style_at(9), SpanStyle { color: Some(Rgba::hex(0xD6FF00)), weight: Some(800), ..Default::default() });
        assert_eq!(t.style_at(6).color, None);
        assert_eq!(t.style_at(11).weight, None);
        // Clearing everything leaves no spans; neighbors with the same style merge.
        t.style_range(0, 18, &SpanChange::Clear);
        assert!(t.spans.is_empty());
        t.style_range(0, 4, &SpanChange::Underline(true));
        t.style_range(4, 7, &SpanChange::Underline(true));
        assert_eq!(words(&t), ["Make it"]);
        // A size back at 100% is no change.
        t.style_range(0, 7, &SpanChange::Underline(false));
        t.style_range(0, 4, &SpanChange::Scale(Some(1.0)));
        assert!(t.spans.is_empty());
    }

    #[test]
    fn styles_follow_their_words_while_typing() {
        let mut t = styled("Make it fast");
        t.style_range(8, 12, &SpanChange::Highlight(Some(Rgba::hex(0xFFB547))));
        t.set_text("Make it very fast".into());
        assert_eq!(words(&t), ["fast"], "text typed before a styled word pushes it along");
        t.set_text("Make it very faxst".into());
        assert_eq!(words(&t), ["faxst"], "typing inside a styled word extends it");
        t.set_text("Make it very faxst!".into());
        assert_eq!(words(&t), ["faxst"], "typing right after it doesn't");
        t.set_text("Make it very slow!".into());
        assert_eq!(words(&t), ["slow"], "typing over the whole word keeps its style");
        t.set_text("Make it very!".into());
        assert!(t.spans.is_empty(), "deleting the word removes its style");
        // Bangla: positions are characters, not bytes.
        let mut b = styled("নতুন অফার আজই");
        b.style_range(5, 9, &SpanChange::Color(Some(Rgba::hex(0xFF6363))));
        assert_eq!(words(&b), ["অফার"]);
        b.set_text("নতুন বড় অফার আজই".into());
        assert_eq!(words(&b), ["অফার"]);
        let runs = b.styled_runs();
        assert_eq!(runs.iter().filter(|r| r.1.is_some()).map(|r| &b.text[r.0.clone()]).collect::<Vec<_>>(), ["অফার"]);
        assert_eq!(runs.iter().map(|r| &b.text[r.0.clone()]).collect::<String>(), b.text, "the runs cover the whole text");
    }

    #[test]
    fn spans_load_from_files_and_bad_ones_are_cleaned() {
        let t: TextLayer = serde_json::from_str(r##"{ "text": "Hello", "spans": [ { "start": 0, "end": 99, "color": [1,0,0,1] }, { "start": 1, "end": 3, "underline": true } ] }"##).unwrap();
        let mut t = t;
        t.set_text(t.text.clone());
        assert_eq!(t.spans.len(), 1);
        assert_eq!((t.spans[0].start, t.spans[0].end), (0, 5));
        let plain: TextLayer = serde_json::from_str(r#"{ "text": "Old file" }"#).unwrap();
        assert!(plain.spans.is_empty());
        assert!(!serde_json::to_string(&plain).unwrap().contains("spans"), "no styles, nothing saved");
    }
}
