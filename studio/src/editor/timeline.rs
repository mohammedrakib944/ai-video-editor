//! The timeline: ruler and track lanes drawn on canvases, with all mouse interaction.
//!
//! Wheel scrolls tracks vertically (or time, when they all fit), Shift + wheel
//! scrolls time, Ctrl + wheel zooms around the cursor.

use iced::keyboard;
use iced::mouse::{self, ScrollDelta};
use iced::widget::canvas::{self, Action, Cache, Frame, Geometry, Path, Stroke, Text};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme, alignment};

use super::media_cache::{MediaCache, PEAKS_PER_SECOND};
use super::{Message, Tool};
use crate::clip::{Clip, Content};
use crate::edit::Edge;
use crate::model::{Id, MediaKind, Project, TrackKind};
use crate::ui::tokens::*;

pub const RULER_H: f32 = 30.0;
pub const HEADER_W: f32 = 210.0;
pub const TOP_PAD: f32 = 6.0;
pub const GAP: f32 = 4.0;
const EDGE_GRAB: f32 = 7.0;
const SNAP_PX: f32 = 9.0;

pub fn track_height(kind: TrackKind) -> f32 {
    match kind {
        TrackKind::Visual => 60.0,
        TrackKind::Audio => 46.0,
    }
}

/// Horizontal view of the timeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    /// Seconds at the left edge.
    pub scroll: f64,
    /// Pixels per second.
    pub zoom: f64,
}

impl Default for View {
    fn default() -> Self {
        View { scroll: 0.0, zoom: 48.0 }
    }
}

impl View {
    pub fn x(&self, t: f64) -> f32 {
        ((t - self.scroll) * self.zoom) as f32
    }

    pub fn t(&self, x: f32) -> f64 {
        self.scroll + x as f64 / self.zoom
    }

    pub fn zoomed(self, factor: f64, anchor_x: f32) -> View {
        let anchor = self.t(anchor_x);
        let zoom = (self.zoom * factor).clamp(2.0, 2400.0);
        View { zoom, scroll: (anchor - anchor_x as f64 / zoom).max(0.0) }
    }
}

/// Track rows: (track id, kind, top y, height).
pub fn rows(project: &Project) -> Vec<(Id, TrackKind, f32, f32)> {
    let mut y = TOP_PAD;
    project
        .tracks
        .iter()
        .map(|t| {
            let h = track_height(t.kind);
            let row = (t.id, t.kind, y, h);
            y += h + GAP;
            row
        })
        .collect()
}

pub fn content_height(project: &Project) -> f32 {
    TOP_PAD + project.tracks.iter().map(|t| track_height(t.kind) + GAP).sum::<f32>() + 24.0
}

fn clip_color(project: &Project, clip: &Clip) -> Color {
    match &clip.content {
        Content::Media { asset, .. } => match project.asset(*asset).map(|a| a.kind) {
            Some(MediaKind::Audio) => clip::AUDIO,
            Some(MediaKind::Image) => clip::IMAGE,
            _ => clip::VIDEO,
        },
        Content::Text(_) => clip::TEXT,
        Content::Shape(_) => clip::EFFECT,
    }
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    Color { r: a.r + (b.r - a.r) * t, g: a.g + (b.g - a.g) * t, b: a.b + (b.b - a.b) * t, a: 1.0 }
}

pub fn clip_label(project: &Project, clip: &Clip) -> String {
    if let Some(n) = &clip.name {
        return n.clone();
    }
    match &clip.content {
        Content::Media { asset, .. } => project.asset(*asset).map(|a| a.name.clone()).unwrap_or_else(|| "Missing media".into()),
        Content::Text(t) => t.text.lines().next().unwrap_or("Text").to_string(),
        Content::Shape(_) => "Shape".into(),
    }
}

pub fn fmt_ruler(t: f64, fine: bool) -> String {
    let t = t.max(0.0);
    let m = (t / 60.0).floor() as u64;
    let s = t - m as f64 * 60.0;
    if fine { format!("{m}:{s:05.2}") } else { format!("{m}:{:02}", s.floor() as u64) }
}

// ---------------------------------------------------------------- ruler

pub struct Ruler {
    pub view: View,
    pub playhead: f64,
    pub range: Option<(f64, f64)>,
}

#[derive(Default)]
pub struct RulerState {
    scrubbing: bool,
    modifiers: keyboard::Modifiers,
}

impl canvas::Program<Message> for Ruler {
    type State = RulerState;

    fn update(&self, state: &mut RulerState, event: &canvas::Event, bounds: Rectangle, cursor: mouse::Cursor) -> Option<Action<Message>> {
        match event {
            canvas::Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => {
                state.modifiers = *m;
                None
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let p = cursor.position_in(bounds)?;
                state.scrubbing = true;
                Some(Action::publish(Message::Seek(self.view.t(p.x).max(0.0))).and_capture())
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { position }) if state.scrubbing => {
                Some(Action::publish(Message::Seek(self.view.t(position.x - bounds.x).max(0.0))).and_capture())
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if state.scrubbing => {
                state.scrubbing = false;
                Some(Action::capture())
            }
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let p = cursor.position_in(bounds)?;
                let (dx, dy) = wheel(*delta);
                let view = if state.modifiers.command() { self.view.zoomed(1.0025f64.powf(dy as f64), p.x) } else { scrolled(self.view, dx + dy) };
                Some(Action::publish(Message::SetView(view)).and_capture())
            }
            _ => None,
        }
    }

    fn draw(&self, _state: &RulerState, renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let mut f = Frame::new(renderer, bounds.size());
        let size = bounds.size();
        f.fill_rectangle(Point::ORIGIN, size, SURFACE_1);
        f.fill_rectangle(Point::new(0.0, size.height - 1.0), Size::new(size.width, 1.0), LINE);
        if let Some((a, b)) = self.range {
            let (x0, x1) = (self.view.x(a), self.view.x(b));
            f.fill_rectangle(Point::new(x0, 0.0), Size::new(x1 - x0, size.height), Color { a: 0.18, ..ACCENT });
        }
        let steps = [0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1800.0];
        let step = steps.iter().copied().find(|s| s * self.view.zoom >= 84.0).unwrap_or(1800.0);
        let minor = step / 5.0;
        let first = (self.view.scroll / minor).floor() as i64;
        let last = ((self.view.scroll + size.width as f64 / self.view.zoom) / minor).ceil() as i64;
        for k in first.max(0)..=last {
            let t = k as f64 * minor;
            let x = self.view.x(t).round();
            let major = k % 5 == 0;
            let h = if major { 9.0 } else { 4.0 };
            f.fill_rectangle(Point::new(x, size.height - 1.0 - h), Size::new(1.0, h), if major { TEXT_3 } else { TEXT_4 });
            if major {
                f.fill_text(Text { content: fmt_ruler(t, step < 1.0), position: Point::new(x + 5.0, 7.0), color: TEXT_3, size: 10.5.into(), font: INTER_MEDIUM, ..Text::default() });
            }
        }
        // Playhead head.
        let x = self.view.x(self.playhead);
        if x > -8.0 && x < size.width + 8.0 {
            let head = Path::new(|b| {
                b.move_to(Point::new(x - 6.0, 3.0));
                b.line_to(Point::new(x + 6.0, 3.0));
                b.line_to(Point::new(x + 6.0, size.height - 10.0));
                b.line_to(Point::new(x, size.height - 3.0));
                b.line_to(Point::new(x - 6.0, size.height - 10.0));
                b.close();
            });
            f.fill(&head, ACCENT);
            f.fill_rectangle(Point::new(x - 0.75, size.height - 4.0), Size::new(1.5, 4.0), ACCENT);
        }
        vec![f.into_geometry()]
    }

    fn mouse_interaction(&self, state: &RulerState, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        if state.scrubbing || cursor.is_over(bounds) { mouse::Interaction::ResizingHorizontally } else { mouse::Interaction::default() }
    }
}

fn wheel(delta: ScrollDelta) -> (f32, f32) {
    match delta {
        ScrollDelta::Lines { x, y } => (x * 48.0, y * 48.0),
        ScrollDelta::Pixels { x, y } => (x, y),
    }
}

fn scrolled(view: View, px: f32) -> View {
    View { scroll: (view.scroll - px as f64 / view.zoom).max(0.0), ..view }
}

// ---------------------------------------------------------------- lanes

#[derive(Clone, Copy)]
pub struct Lanes<'a> {
    pub project: &'a Project,
    pub media: &'a MediaCache,
    pub selection: &'a [Id],
    pub playhead: f64,
    pub view: View,
    pub tool: Tool,
    pub snapping: bool,
    pub range: Option<(f64, f64)>,
    /// Asset being dragged in from the library.
    pub dnd: Option<Id>,
    pub cache: &'a Caches,
    /// Whether the tracks overflow vertically (then the plain wheel scrolls them).
    pub overflow: bool,
}

#[derive(Debug, Clone, Copy)]
enum Drag {
    Pan { x0: f32, scroll0: f64 },
    Move { clip: Id, grab: f64, track: Id, start: f64, press: Point, moved: bool },
    Trim { clip: Id, edge: Edge, t: f64 },
    Range { anchor: f64, current: f64 },
}

#[derive(Default)]
pub struct LanesState {
    drag: Option<Drag>,
    modifiers: keyboard::Modifiers,
    hover: Option<Point>,
    guide: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Zone {
    Body,
    Edge(Edge),
}

impl Lanes<'_> {
    fn row_at(&self, y: f32) -> Option<(Id, TrackKind, f32, f32)> {
        rows(self.project).into_iter().find(|&(_, _, top, h)| y >= top - GAP / 2.0 && y < top + h + GAP / 2.0)
    }

    fn row_of(&self, track: Id) -> Option<(Id, TrackKind, f32, f32)> {
        rows(self.project).into_iter().find(|r| r.0 == track)
    }

    fn hit(&self, p: Point) -> Option<(&Clip, Zone)> {
        let (track, _, top, h) = self.row_at(p.y)?;
        if p.y < top || p.y > top + h {
            return None;
        }
        let t = self.view.t(p.x);
        let clip = self.project.clips_on(track).find(|c| t >= c.start && t <= c.end())?;
        let (x0, x1) = (self.view.x(clip.start), self.view.x(clip.end()));
        let grab = EDGE_GRAB.min((x1 - x0) / 4.0);
        let zone = if p.x - x0 <= grab {
            Zone::Edge(Edge::Start)
        } else if x1 - p.x <= grab {
            Zone::Edge(Edge::End)
        } else {
            Zone::Body
        };
        Some((clip, zone))
    }

    fn snap_points(&self, exclude: Option<Id>) -> Vec<f64> {
        let mut pts = vec![0.0, self.playhead];
        for c in self.project.clips.iter().filter(|c| Some(c.id) != exclude) {
            pts.push(c.start);
            pts.push(c.end());
        }
        if let Some((a, b)) = self.range {
            pts.extend([a, b]);
        }
        pts
    }

    /// Snaps `t` to the nearest point within a few pixels. Returns the time and the guide position.
    fn snap(&self, t: f64, exclude: Option<Id>) -> (f64, Option<f64>) {
        if !self.snapping {
            return (t, None);
        }
        let limit = SNAP_PX as f64 / self.view.zoom;
        self.snap_points(exclude)
            .into_iter()
            .map(|p| (p, (p - t).abs()))
            .filter(|&(_, d)| d <= limit)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(p, _)| (p, Some(p)))
            .unwrap_or((t, None))
    }

    /// Snaps a moving clip by whichever of its edges is closer to a snap point.
    fn snap_span(&self, start: f64, duration: f64, exclude: Id) -> (f64, Option<f64>) {
        let (s, gs) = self.snap(start, Some(exclude));
        let (e, ge) = self.snap(start + duration, Some(exclude));
        match (gs, ge) {
            (Some(_), Some(_)) if (e - (start + duration)).abs() < (s - start).abs() => (e - duration, ge),
            (Some(_), _) => (s, gs),
            (None, Some(_)) => (e - duration, ge),
            _ => (start, None),
        }
    }
}

impl canvas::Program<Message> for Lanes<'_> {
    type State = LanesState;

    fn update(&self, state: &mut LanesState, event: &canvas::Event, bounds: Rectangle, cursor: mouse::Cursor) -> Option<Action<Message>> {
        let local = cursor.position_in(bounds);
        match event {
            canvas::Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => {
                state.modifiers = *m;
                None
            }
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let p = local?;
                let (dx, dy) = wheel(*delta);
                if state.modifiers.command() {
                    return Some(Action::publish(Message::SetView(self.view.zoomed(1.0025f64.powf(dy as f64), p.x))).and_capture());
                }
                if state.modifiers.shift() || !self.overflow || dx.abs() > dy.abs() {
                    let px = if dx.abs() > dy.abs() { dx } else { dy };
                    return Some(Action::publish(Message::SetView(scrolled(self.view, px))).and_capture());
                }
                None
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(button)) => {
                let p = local?;
                if *button == mouse::Button::Middle {
                    state.drag = Some(Drag::Pan { x0: p.x, scroll0: self.view.scroll });
                    return Some(Action::capture());
                }
                if *button != mouse::Button::Left {
                    return None;
                }
                let t = self.view.t(p.x).max(0.0);
                match self.tool {
                    Tool::Blade => {
                        let (clip, _) = self.hit(p)?;
                        let (t, _) = self.snap(t, Some(clip.id));
                        Some(Action::publish(Message::Split(clip.id, t)).and_capture())
                    }
                    Tool::Range => {
                        let (t, guide) = self.snap(t, None);
                        state.guide = guide;
                        state.drag = Some(Drag::Range { anchor: t, current: t });
                        Some(Action::capture())
                    }
                    Tool::Select => match self.hit(p) {
                        Some((clip, Zone::Edge(edge))) => {
                            let t = if edge == Edge::Start { clip.start } else { clip.end() };
                            state.drag = Some(Drag::Trim { clip: clip.id, edge, t });
                            Some(Action::publish(Message::Select(Some(clip.id), false)).and_capture())
                        }
                        Some((clip, Zone::Body)) => {
                            state.drag = Some(Drag::Move { clip: clip.id, grab: t - clip.start, track: clip.track, start: clip.start, press: p, moved: false });
                            let additive = state.modifiers.shift() || state.modifiers.command();
                            Some(Action::publish(Message::Select(Some(clip.id), additive)).and_capture())
                        }
                        None => {
                            state.drag = Some(Drag::Pan { x0: p.x, scroll0: self.view.scroll });
                            Some(Action::publish(Message::Select(None, false)).and_capture())
                        }
                    },
                }
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                let p = Point::new(position.x - bounds.x, position.y - bounds.y);
                state.hover = local;
                let Some(drag) = state.drag.as_mut() else {
                    // Keep the drop preview following the cursor during a library drag.
                    return self.dnd.map(|_| Action::request_redraw());
                };
                match drag {
                    Drag::Pan { x0, scroll0 } => {
                        let view = View { scroll: (*scroll0 - (p.x - *x0) as f64 / self.view.zoom).max(0.0), ..self.view };
                        return Some(Action::publish(Message::SetView(view)).and_capture());
                    }
                    Drag::Move { clip, grab, track, start, press, moved } => {
                        if !*moved && press.distance(p) < 4.0 {
                            return None;
                        }
                        *moved = true;
                        let c = self.project.clip(*clip)?;
                        let kind = self.project.kind_for(&c.content);
                        if let Some((row, row_kind, _, _)) = self.row_at(p.y) {
                            if row_kind == kind && !self.project.track(row).is_some_and(|t| t.locked) {
                                *track = row;
                            }
                        }
                        let (s, guide) = self.snap_span((self.view.t(p.x) - *grab).max(0.0), c.duration, c.id);
                        *start = s;
                        state.guide = guide;
                    }
                    Drag::Trim { clip, t, .. } => {
                        let (s, guide) = self.snap(self.view.t(p.x).max(0.0), Some(*clip));
                        *t = s;
                        state.guide = guide;
                    }
                    Drag::Range { current, .. } => {
                        let (s, guide) = self.snap(self.view.t(p.x).max(0.0), None);
                        *current = s;
                        state.guide = guide;
                    }
                }
                Some(Action::request_redraw().and_capture())
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(_)) => {
                state.guide = None;
                if let (Some(asset), Some(p)) = (self.dnd, local) {
                    let track = self.row_at(p.y).map(|r| r.0);
                    let (t, _) = self.snap(self.view.t(p.x).max(0.0), None);
                    return Some(Action::publish(Message::DropAsset { asset, track, t }).and_capture());
                }
                let msg = match state.drag.take()? {
                    Drag::Move { clip, track, start, moved: true, .. } => Message::MoveClip { clip, track, start },
                    Drag::Move { .. } | Drag::Pan { .. } => return Some(Action::request_redraw().and_capture()),
                    Drag::Trim { clip, edge, t } => Message::Trim { clip, edge, t },
                    Drag::Range { anchor, current } => {
                        let (a, b) = (anchor.min(current), anchor.max(current));
                        Message::SetRange(((b - a) * self.view.zoom > 3.0).then_some((a, b)))
                    }
                };
                Some(Action::publish(msg).and_capture())
            }
            canvas::Event::Mouse(mouse::Event::CursorLeft) => {
                state.hover = None;
                Some(Action::request_redraw())
            }
            _ => None,
        }
    }

    fn draw(&self, state: &LanesState, renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let deco = self.cache.deco.draw(renderer, bounds.size(), |f| self.draw_layer(f, Layer::Deco));
        let mut o = Frame::new(renderer, bounds.size());
        let h = bounds.height;

        // Range selection.
        let range = match state.drag {
            Some(Drag::Range { anchor, current }) => Some((anchor.min(current), anchor.max(current))),
            _ => self.range,
        };
        if let Some((a, b)) = range {
            let (x0, x1) = (self.view.x(a), self.view.x(b));
            o.fill_rectangle(Point::new(x0, 0.0), Size::new(x1 - x0, h), Color { a: 0.10, ..ACCENT });
            for x in [x0, x1] {
                o.fill_rectangle(Point::new(x - 0.5, 0.0), Size::new(1.0, h), Color { a: 0.8, ..ACCENT });
            }
        }

        // Move / trim previews.
        match state.drag {
            Some(Drag::Move { clip, track, start, moved: true, .. }) => {
                if let (Some(c), Some((_, _, top, rh))) = (self.project.clip(clip), self.row_of(track)) {
                    let r = Rectangle::new(Point::new(self.view.x(start), top), Size::new((c.duration * self.view.zoom) as f32, rh));
                    ghost(&mut o, r, clip_color(self.project, c));
                }
            }
            Some(Drag::Trim { clip, edge, t }) => {
                if let (Some(c), Some((_, _, top, rh))) = (self.project.clip(clip), self.row_of(self.project.clip(clip).map(|c| c.track).unwrap_or(0))) {
                    let (s, e) = if edge == Edge::Start { (t.min(c.end() - 0.1), c.end()) } else { (c.start, t.max(c.start + 0.1)) };
                    let r = Rectangle::new(Point::new(self.view.x(s), top), Size::new(((e - s) * self.view.zoom) as f32, rh));
                    ghost(&mut o, r, clip_color(self.project, c));
                }
            }
            _ => {}
        }

        // Drop preview for a library drag.
        if let (Some(asset), Some(p)) = (self.dnd, state.hover) {
            if let (Some(a), Some((_, kind, top, rh))) = (self.project.asset(asset), self.row_at(p.y)) {
                let fits = (kind == TrackKind::Audio) == (a.kind == MediaKind::Audio);
                let dur = if a.kind == MediaKind::Image { crate::edit::DEFAULT_STILL } else { a.duration };
                let r = Rectangle::new(Point::new(p.x, top), Size::new((dur * self.view.zoom) as f32, rh));
                ghost(&mut o, r, if fits { ACCENT } else { DANGER });
            }
        }

        if let Some(g) = state.guide {
            let x = self.view.x(g);
            o.fill_rectangle(Point::new(x - 0.5, 0.0), Size::new(1.0, h), WARNING);
        }

        // Playhead.
        let x = self.view.x(self.playhead);
        o.fill_rectangle(Point::new(x - 0.75, 0.0), Size::new(1.5, h), ACCENT);

        // Hovered clip edge.
        if self.tool == Tool::Select && state.drag.is_none() {
            if let Some((c, Zone::Edge(edge))) = state.hover.and_then(|p| self.hit(p)) {
                if let Some((_, _, top, rh)) = self.row_of(c.track) {
                    let x = self.view.x(if edge == Edge::Start { c.start } else { c.end() });
                    let bar = Path::rounded_rectangle(Point::new(if edge == Edge::Start { x } else { x - 4.0 }, top + 2.0), Size::new(4.0, rh - 4.0), 2.0.into());
                    o.fill(&bar, Color::WHITE);
                }
            }
        }
        if self.tool == Tool::Blade {
            if let Some(p) = state.hover {
                o.fill_rectangle(Point::new(p.x - 0.5, 0.0), Size::new(1.0, h), DANGER);
            }
        }
        vec![deco, o.into_geometry()]
    }

    fn mouse_interaction(&self, state: &LanesState, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        match state.drag {
            Some(Drag::Move { moved: true, .. }) | Some(Drag::Pan { .. }) => return mouse::Interaction::Grabbing,
            Some(Drag::Trim { .. }) => return mouse::Interaction::ResizingHorizontally,
            _ => {}
        }
        let Some(p) = cursor.position_in(bounds) else { return mouse::Interaction::default() };
        match self.tool {
            Tool::Blade if self.hit(p).is_some() => mouse::Interaction::Crosshair,
            Tool::Range => mouse::Interaction::Text,
            Tool::Select => match self.hit(p) {
                Some((_, Zone::Edge(_))) => mouse::Interaction::ResizingHorizontally,
                Some((_, Zone::Body)) => mouse::Interaction::Pointer,
                None => mouse::Interaction::default(),
            },
            _ => mouse::Interaction::default(),
        }
    }
}

fn ghost(f: &mut Frame, r: Rectangle, color: Color) {
    let path = Path::rounded_rectangle(r.position(), r.size(), 6.0.into());
    f.fill(&path, Color { a: 0.22, ..color });
    f.stroke(&path, Stroke::default().with_color(color).with_width(1.5));
}

/// The bottom canvas: lanes, clip bodies and filmstrips. It sits under `Lanes`
/// in a stack because images always draw above shapes within one layer.
pub struct LanesBase<'a>(pub Lanes<'a>);

impl canvas::Program<Message> for LanesBase<'_> {
    type State = ();

    fn draw(&self, _: &(), renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        vec![self.0.cache.base.draw(renderer, bounds.size(), |f| self.0.draw_layer(f, Layer::Base))]
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Layer {
    /// Lane and clip backgrounds plus filmstrip images.
    Base,
    /// Everything drawn over the images: masks, waveforms, labels, outlines.
    Deco,
}

/// Cached drawings of the two lane canvases.
pub struct Caches {
    base: Cache,
    deco: Cache,
}

impl Default for Caches {
    fn default() -> Self {
        Caches { base: Cache::new(), deco: Cache::new() }
    }
}

impl Caches {
    pub fn clear(&self) {
        self.base.clear();
        self.deco.clear();
    }
}

impl Lanes<'_> {
    fn draw_layer(&self, f: &mut Frame, layer: Layer) {
        let size = f.size();
        if layer == Layer::Base {
            f.fill_rectangle(Point::ORIGIN, size, SURFACE_0);
        }
        let (t0, t1) = (self.view.scroll, self.view.t(size.width));
        for (track, kind, top, h) in rows(self.project) {
            if layer == Layer::Base {
                let lane = Path::rounded_rectangle(Point::new(0.0, top), Size::new(size.width, h), 6.0.into());
                f.fill(&lane, if kind == TrackKind::Audio { Color::from_rgb8(0x10, 0x13, 0x13) } else { SURFACE_1 });
            }
            let dim = self.project.track(track).is_some_and(|t| t.hidden || t.muted);
            for clip in self.project.clips_on(track).filter(|c| c.start < t1 && c.end() > t0) {
                let x0 = self.view.x(clip.start);
                let w = (clip.duration * self.view.zoom) as f32;
                let r = Rectangle::new(Point::new(x0, top), Size::new(w.max(2.0), h));
                let selected = self.selection.contains(&clip.id);
                match layer {
                    Layer::Base => self.draw_clip_base(f, clip, r, dim),
                    Layer::Deco => self.draw_clip_deco(f, clip, r, selected, dim),
                }
            }
        }
    }

    fn filmstrip(&self, clip: &Clip) -> Option<(&crate::model::Asset, &super::media_cache::AssetMedia)> {
        let a = clip.asset().and_then(|a| self.project.asset(a))?;
        let m = self.media.get(a.id)?;
        (a.is_visual() && !m.strip.is_empty()).then_some((a, m))
    }

    fn draw_clip_base(&self, f: &mut Frame, clip: &Clip, r: Rectangle, dim: bool) {
        let color = clip_color(self.project, clip);
        let radius = 6.0f32.min(r.width / 2.0);
        let body = Path::rounded_rectangle(r.position(), r.size(), radius.into());
        f.fill(&body, mix(SURFACE_1, color, if dim { 0.18 } else { 0.32 }));
        let inner = Rectangle::new(Point::new(r.x + 1.0, r.y + 1.0), Size::new((r.width - 2.0).max(0.0), r.height - 2.0));
        let Some((a, m)) = self.filmstrip(clip) else { return };
        if inner.width <= 4.0 {
            return;
        }
        let tile_w = (inner.height * m.strip_w as f32 / super::media_cache::STRIP_H as f32).max(8.0);
        let first = ((inner.x.max(-tile_w) - inner.x) / tile_w).floor().max(0.0) as i32;
        let canvas_w = f.size().width;
        f.with_clip(inner, |f| {
            let mut i = first;
            loop {
                let x = inner.x + i as f32 * tile_w;
                if x > inner.x + inner.width || x > canvas_w {
                    break;
                }
                let t = self.view.t(x + tile_w / 2.0).clamp(clip.start, clip.end());
                let src = clip.source_time(t);
                let idx = if a.kind == MediaKind::Image || a.duration <= 0.0 { 0 } else { ((src / a.duration) * m.strip.len() as f64).floor() as usize };
                let handle = &m.strip[idx.min(m.strip.len() - 1)];
                let img = canvas::Image::new(handle.clone()).opacity(if dim { 0.35_f32 } else { 0.92 });
                f.draw_image(Rectangle::new(Point::new(x, inner.y), Size::new(tile_w, inner.height)), img);
                i += 1;
            }
        });
    }

    fn draw_clip_deco(&self, f: &mut Frame, clip: &Clip, r: Rectangle, selected: bool, dim: bool) {
        let color = clip_color(self.project, clip);
        let radius = 6.0f32.min(r.width / 2.0);
        let body = Path::rounded_rectangle(r.position(), r.size(), radius.into());
        let inner = Rectangle::new(Point::new(r.x + 1.0, r.y + 1.0), Size::new((r.width - 2.0).max(0.0), r.height - 2.0));
        let has_strip = self.filmstrip(clip).is_some();
        if has_strip {
            corner_masks(f, r, radius, SURFACE_1);
            // Darken the top so the label stays readable over busy footage.
            let shade = Path::rounded_rectangle(r.position(), Size::new(r.width, 22.0f32.min(r.height)), radius.into());
            f.fill(&shade, Color { a: 0.28, ..Color::BLACK });
        }

        // Waveform for audio.
        let asset = clip.asset().and_then(|a| self.project.asset(a));
        let peaks = clip.asset().and_then(|a| self.media.get(a)).and_then(|m| m.peaks.clone());
        if let (Some(a), Some(peaks)) = (asset, peaks) {
            if a.kind == MediaKind::Audio && !peaks.is_empty() {
                let mid = r.y + r.height * 0.6;
                let amp = r.height * 0.34 * (clip.audio.volume.min(2.0) as f32);
                let wave_color = Color { a: if dim { 0.3 } else { 0.85 }, ..mix(color, Color::WHITE, 0.25) };
                let (x_start, x_end) = (inner.x.max(0.0), (inner.x + inner.width).min(f.size().width));
                let step = 2.0;
                let rate = PEAKS_PER_SECOND as f64;
                let path = Path::new(|b| {
                    let mut x = x_start;
                    while x < x_end {
                        let ia = (clip.source_time(self.view.t(x)) * rate).max(0.0) as usize;
                        let ib = ((clip.source_time(self.view.t(x + step)) * rate).ceil() as usize).max(ia + 1);
                        let peak = peaks.get(ia.min(peaks.len())..ib.min(peaks.len())).and_then(|s| s.iter().copied().reduce(f32::max)).unwrap_or(0.0);
                        let hh = (peak.sqrt() * amp).max(0.5);
                        b.rectangle(Point::new(x, mid - hh), Size::new(step - 0.6, hh * 2.0));
                        x += step;
                    }
                });
                f.fill(&path, wave_color);
            }
        }

        // Text clips preview their text.
        if let Content::Text(t) = &clip.content {
            if r.width > 40.0 {
                f.with_clip(inner, |f| {
                    f.fill_text(Text {
                        content: t.text.replace('\n', " "),
                        position: Point::new(r.x + 10.0, r.y + r.height * 0.5),
                        color: Color { a: if dim { 0.4 } else { 0.92 }, ..Color::WHITE },
                        size: 15.0.into(),
                        font: if t.text.is_ascii() { INTER_BOLD } else { HIND_SEMIBOLD },
                        align_y: alignment::Vertical::Center,
                        ..Text::default()
                    });
                });
            }
        }

        // Zoom regions as pills along the bottom.
        for z in &clip.zooms {
            let zx = self.view.x(clip.start + z.at);
            let zw = (z.duration * self.view.zoom) as f32;
            let pill = Path::rounded_rectangle(Point::new(zx + 2.0, r.y + r.height - 10.0), Size::new((zw - 4.0).max(4.0), 6.0), 3.0.into());
            f.fill(&pill, BRAND_2);
            f.stroke(&pill, Stroke::default().with_color(Color { a: 0.6, ..Color::BLACK }).with_width(1.0));
        }

        // Label chip (text clips show their content instead).
        if r.width > 34.0 && !matches!(clip.content, Content::Text(_)) {
            let label = clip_label(self.project, clip);
            let font = if label.is_ascii() { INTER_SEMIBOLD } else { HIND_SEMIBOLD };
            f.with_clip(Rectangle::new(Point::new(r.x + 4.0, r.y + 3.0), Size::new((r.width - 8.0).max(0.0), 18.0)), |f| {
                f.fill_text(Text {
                    content: label,
                    position: Point::new(r.x + 8.0, r.y + 12.0),
                    color: Color::WHITE,
                    size: 11.0.into(),
                    font,
                    align_y: alignment::Vertical::Center,
                    ..Text::default()
                });
            });
        }

        let (line, width) = if selected { (Color::WHITE, 2.0) } else { (Color { a: 0.55, ..color }, 1.0) };
        f.stroke(&body, Stroke::default().with_color(line).with_width(width));
        if selected {
            let glow = Path::rounded_rectangle(Point::new(r.x - 2.0, r.y - 2.0), Size::new(r.width + 4.0, r.height + 4.0), (radius + 2.0).into());
            f.stroke(&glow, Stroke::default().with_color(Color { a: 0.4, ..ACCENT }).with_width(2.0));
        }
    }
}

/// Paints the corners outside a rounded rectangle, hiding square image corners.
fn corner_masks(f: &mut Frame, r: Rectangle, radius: f32, bg: Color) {
    if radius < 1.0 {
        return;
    }
    let corners = [
        (Point::new(r.x, r.y), Point::new(r.x + radius, r.y + radius)),
        (Point::new(r.x + r.width, r.y), Point::new(r.x + r.width - radius, r.y + radius)),
        (Point::new(r.x + r.width, r.y + r.height), Point::new(r.x + r.width - radius, r.y + r.height - radius)),
        (Point::new(r.x, r.y + r.height), Point::new(r.x + radius, r.y + r.height - radius)),
    ];
    for (corner, center) in corners {
        let path = Path::new(|b| {
            let dx = (corner.x - center.x).signum();
            let dy = (corner.y - center.y).signum();
            b.move_to(corner);
            // Quarter arc from the horizontal edge to the vertical edge.
            for i in 0..=8 {
                let a = std::f32::consts::FRAC_PI_2 * (1.0 - i as f32 / 8.0);
                b.line_to(Point::new(center.x + dx * radius * a.cos(), center.y + dy * radius * a.sin()));
            }
            b.close();
        });
        f.fill(&path, bg);
    }
}
