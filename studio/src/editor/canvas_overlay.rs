//! Direct manipulation on the preview: click to select a layer, drag to move
//! (snapping to the canvas center), corner handles to scale, the knob to rotate.
//! In zoom-edit mode it shows the zoom's focus rectangle over the full frame.

use std::sync::Arc;

use iced::mouse;
use iced::widget::canvas::{self, Action, Frame, Geometry, Path, Stroke, Text};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme, Vector, alignment};

use super::Message;
use super::inspector::Edit;
use crate::clip::Zoom;
use crate::model::{Id, Project};
use crate::render::scene::{LayerSpec, Scene};
use crate::ui::tokens::*;

const HANDLE: f32 = 9.0;
const KNOB_GAP: f32 = 26.0;
const SNAP_PX: f32 = 7.0;

pub struct Overlay<'a> {
    pub project: &'a Project,
    pub scene: Arc<Scene>,
    pub selection: &'a [Id],
    /// The zoom being edited, if any: (clip, zoom).
    pub zoom_edit: Option<(Id, Zoom)>,
    /// Where the canvas sits inside the overlay, and its on-screen size.
    pub origin: Vector,
    pub display: Size,
}

#[derive(Debug, Clone, Copy)]
enum Drag {
    Move { clip: Id, grab: Vector, moved: bool },
    Scale { clip: Id, start_dist: f32, start_scale: f64 },
    Rotate { clip: Id, start_angle: f32, start_rotation: f64 },
    ZoomMove { clip: Id, zoom: Id, grab: Vector },
    ZoomScale { clip: Id, zoom: Id },
}

#[derive(Default)]
pub struct State {
    drag: Option<Drag>,
    guides: (bool, bool),
}

/// Canvas pixels per overlay pixel, and the conversions both ways.
struct Map {
    k: f32,
    origin: Vector,
}

impl Map {
    fn to_canvas(&self, p: Point) -> Point {
        Point::new((p.x - self.origin.x) * self.k, (p.y - self.origin.y) * self.k)
    }
    fn to_screen(&self, p: Point) -> Point {
        Point::new(p.x / self.k + self.origin.x, p.y / self.k + self.origin.y)
    }
}

fn rotate(v: Vector, angle: f32) -> Vector {
    let (s, c) = angle.sin_cos();
    Vector::new(v.x * c - v.y * s, v.x * s + v.y * c)
}

/// Whether canvas point `p` is inside the layer's rotated rectangle.
fn contains(l: &LayerSpec, p: Point) -> bool {
    let local = rotate(Vector::new(p.x - l.center[0], p.y - l.center[1]), -l.rotation);
    local.x.abs() <= l.size[0] / 2.0 && local.y.abs() <= l.size[1] / 2.0
}

/// Corners of a layer in canvas pixels: top-left, top-right, bottom-right, bottom-left.
fn corners(l: &LayerSpec) -> [Point; 4] {
    let (hw, hh) = (l.size[0] / 2.0, l.size[1] / 2.0);
    [(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)].map(|(x, y)| {
        let v = rotate(Vector::new(x, y), l.rotation);
        Point::new(l.center[0] + v.x, l.center[1] + v.y)
    })
}

/// The zoom's focus rectangle in canvas pixels, inside the layer's (unrotated) bounds.
fn zoom_rect(l: &LayerSpec, z: &Zoom) -> Rectangle {
    let span = 1.0 / z.scale.max(1.0) as f32;
    let half = span / 2.0;
    let cx = (z.focus_x as f32).clamp(half, 1.0 - half);
    let cy = (z.focus_y as f32).clamp(half, 1.0 - half);
    let (x0, y0) = (l.center[0] - l.size[0] / 2.0, l.center[1] - l.size[1] / 2.0);
    Rectangle::new(Point::new(x0 + (cx - half) * l.size[0], y0 + (cy - half) * l.size[1]), Size::new(span * l.size[0], span * l.size[1]))
}

impl Overlay<'_> {
    fn layer(&self, clip: Id) -> Option<&LayerSpec> {
        self.scene.layers.iter().find(|l| l.clip == clip)
    }

    fn selected_layer(&self) -> Option<&LayerSpec> {
        match self.selection {
            [id] => self.layer(*id),
            _ => None,
        }
    }

    fn map(&self, _bounds: Rectangle) -> Map {
        Map { k: self.scene.width as f32 / self.display.width.max(1.0), origin: self.origin }
    }

    fn knob(&self, l: &LayerSpec, map: &Map) -> Point {
        let top_mid = rotate(Vector::new(0.0, -l.size[1] / 2.0 - KNOB_GAP * map.k), l.rotation);
        Point::new(l.center[0] + top_mid.x, l.center[1] + top_mid.y)
    }
}

impl canvas::Program<Message> for Overlay<'_> {
    type State = State;

    fn update(&self, state: &mut State, event: &canvas::Event, bounds: Rectangle, cursor: mouse::Cursor) -> Option<Action<Message>> {
        let map = self.map(bounds);
        let (cw, ch) = (self.scene.width as f32, self.scene.height as f32);
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let p = map.to_canvas(cursor.position_in(bounds)?);
                let near = |a: Point| a.distance(p) <= HANDLE * map.k;
                // Zoom editing takes the clicks first.
                if let Some((clip, z)) = self.zoom_edit {
                    let l = self.layer(clip)?;
                    let r = zoom_rect(l, &z);
                    let corners = [r.position(), Point::new(r.x + r.width, r.y), Point::new(r.x + r.width, r.y + r.height), Point::new(r.x, r.y + r.height)];
                    if corners.iter().any(|&c| near(c)) {
                        state.drag = Some(Drag::ZoomScale { clip, zoom: z.id });
                        return Some(Action::capture());
                    }
                    if r.contains(p) {
                        let center = Point::new(r.x + r.width / 2.0, r.y + r.height / 2.0);
                        state.drag = Some(Drag::ZoomMove { clip, zoom: z.id, grab: center - p });
                        return Some(Action::capture());
                    }
                    return Some(Action::capture());
                }
                if let Some(l) = self.selected_layer() {
                    let clip = self.project.clip(l.clip)?;
                    if near(self.knob(l, &map)) {
                        let angle = (p.y - l.center[1]).atan2(p.x - l.center[0]);
                        state.drag = Some(Drag::Rotate { clip: l.clip, start_angle: angle, start_rotation: clip.transform.rotation });
                        return Some(Action::capture());
                    }
                    if corners(l).iter().any(|&c| near(c)) {
                        let d = Point::new(l.center[0], l.center[1]).distance(p).max(1.0);
                        state.drag = Some(Drag::Scale { clip: l.clip, start_dist: d, start_scale: clip.transform.scale });
                        return Some(Action::capture());
                    }
                }
                let locked = |l: &LayerSpec| self.project.clip(l.clip).and_then(|c| self.project.track(c.track)).is_some_and(|t| t.locked);
                let hit = self.scene.layers.iter().rev().find(|l| contains(l, p) && !locked(l));
                match hit {
                    Some(l) => {
                        let clip = self.project.clip(l.clip)?;
                        let base = Point::new(clip.transform.x as f32 * cw, clip.transform.y as f32 * ch);
                        state.drag = Some(Drag::Move { clip: l.clip, grab: base - p, moved: false });
                        Some(Action::publish(Message::Select(Some(l.clip), false)).and_capture())
                    }
                    None => Some(Action::publish(Message::Select(None, false)).and_capture()),
                }
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                let drag = state.drag.as_mut()?;
                let p = map.to_canvas(Point::new(position.x - bounds.x, position.y - bounds.y));
                let msg = match drag {
                    Drag::Move { clip, grab, moved } => {
                        *moved = true;
                        let mut target = p + *grab;
                        let snap = SNAP_PX * map.k;
                        state.guides = ((target.x - cw / 2.0).abs() < snap, (target.y - ch / 2.0).abs() < snap);
                        if state.guides.0 {
                            target.x = cw / 2.0;
                        }
                        if state.guides.1 {
                            target.y = ch / 2.0;
                        }
                        Message::Edit(*clip, Edit::Position((target.x / cw) as f64, (target.y / ch) as f64))
                    }
                    Drag::Scale { clip, start_dist, start_scale } => {
                        let l = self.layer(*clip)?;
                        let d = Point::new(l.center[0], l.center[1]).distance(p);
                        Message::Edit(*clip, Edit::Scale((*start_scale * (d / *start_dist) as f64).clamp(0.02, 10.0)))
                    }
                    Drag::Rotate { clip, start_angle, start_rotation } => {
                        let l = self.layer(*clip)?;
                        let angle = (p.y - l.center[1]).atan2(p.x - l.center[0]);
                        let mut deg = *start_rotation + (angle - *start_angle).to_degrees() as f64;
                        deg = ((deg + 180.0).rem_euclid(360.0)) - 180.0;
                        // Snap to 15° steps when close.
                        let snapped = (deg / 15.0).round() * 15.0;
                        if (snapped - deg).abs() < 2.0 {
                            deg = snapped;
                        }
                        Message::Edit(*clip, Edit::Rotation(deg))
                    }
                    Drag::ZoomMove { clip, zoom, grab } => {
                        let l = self.layer(*clip)?;
                        let c = p + *grab;
                        let (x0, y0) = (l.center[0] - l.size[0] / 2.0, l.center[1] - l.size[1] / 2.0);
                        let fx = ((c.x - x0) / l.size[0]).clamp(0.0, 1.0) as f64;
                        let fy = ((c.y - y0) / l.size[1]).clamp(0.0, 1.0) as f64;
                        Message::Edit(*clip, Edit::ZoomFocus(*zoom, fx, fy))
                    }
                    Drag::ZoomScale { clip, zoom } => {
                        let l = self.layer(*clip)?;
                        let z = self.project.clip(*clip)?.zooms.iter().find(|z| z.id == *zoom)?;
                        let r = zoom_rect(l, z);
                        let center = Point::new(r.x + r.width / 2.0, r.y + r.height / 2.0);
                        // Half-size from the cursor, keeping the frame's aspect ratio.
                        let half_span = ((p.x - center.x).abs() / l.size[0]).max((p.y - center.y).abs() / l.size[1]);
                        let scale = (1.0 / (half_span * 2.0).max(0.05)).clamp(1.0, 6.0) as f64;
                        Message::Edit(*clip, Edit::ZoomScale(*zoom, scale))
                    }
                };
                Some(Action::publish(msg).and_capture())
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.guides = (false, false);
                state.drag.take().map(|_| Action::request_redraw().and_capture())
            }
            _ => None,
        }
    }

    fn draw(&self, state: &State, renderer: &Renderer, _theme: &Theme, bounds: Rectangle, cursor: mouse::Cursor) -> Vec<Geometry> {
        let mut f = Frame::new(renderer, bounds.size());
        let map = self.map(bounds);
        let s = |p: Point| map.to_screen(p);

        if let Some((clip, z)) = self.zoom_edit {
            if let Some(l) = self.layer(clip) {
                let r = zoom_rect(l, &z);
                let (a, b) = (s(r.position()), s(Point::new(r.x + r.width, r.y + r.height)));
                // Dim the canvas outside the focus area.
                let dim = Color { a: 0.55, ..Color::BLACK };
                let (ox, oy, w, h) = (self.origin.x, self.origin.y, self.display.width, self.display.height);
                let (ax, ay, bx, by) = (a.x.clamp(ox, ox + w), a.y.clamp(oy, oy + h), b.x.clamp(ox, ox + w), b.y.clamp(oy, oy + h));
                f.fill_rectangle(Point::new(ox, oy), Size::new(w, ay - oy), dim);
                f.fill_rectangle(Point::new(ox, by), Size::new(w, oy + h - by), dim);
                f.fill_rectangle(Point::new(ox, ay), Size::new(ax - ox, by - ay), dim);
                f.fill_rectangle(Point::new(bx, ay), Size::new(ox + w - bx, by - ay), dim);
                let rect = Path::rectangle(a, Size::new(b.x - a.x, b.y - a.y));
                f.stroke(&rect, Stroke::default().with_color(BRAND_2).with_width(2.0));
                for c in [a, Point::new(b.x, a.y), b, Point::new(a.x, b.y)] {
                    handle(&mut f, c, BRAND_2);
                }
                let label = format!("Zoom {:.0}%", z.scale * 100.0);
                let chip = Path::rounded_rectangle(Point::new(a.x, a.y - 24.0), Size::new(78.0, 19.0), 5.0.into());
                f.fill(&chip, BRAND_2);
                f.fill_text(Text { content: label, position: Point::new(a.x + 39.0, a.y - 14.5), color: ON_ACCENT, size: 11.0.into(), font: INTER_SEMIBOLD, align_x: alignment::Horizontal::Center.into(), align_y: alignment::Vertical::Center, ..Text::default() });
            }
            return vec![f.into_geometry()];
        }

        // Hover outline for layers that aren't selected.
        if state.drag.is_none() {
            if let Some(p) = cursor.position_in(bounds).map(|p| map.to_canvas(p)) {
                if let Some(l) = self.scene.layers.iter().rev().find(|l| contains(l, p)) {
                    if !self.selection.contains(&l.clip) {
                        let c = corners(l).map(s);
                        f.stroke(&poly(&c), Stroke::default().with_color(Color { a: 0.6, ..ACCENT }).with_width(1.0));
                    }
                }
            }
        }

        if let Some(l) = self.selected_layer() {
            let c = corners(l).map(s);
            f.stroke(&poly(&c), Stroke::default().with_color(ACCENT).with_width(1.5));
            for p in c {
                handle(&mut f, p, ACCENT);
            }
            let knob = s(self.knob(l, &map));
            let top_mid = Point::new((c[0].x + c[1].x) / 2.0, (c[0].y + c[1].y) / 2.0);
            f.stroke(&Path::line(top_mid, knob), Stroke::default().with_color(ACCENT).with_width(1.0));
            f.fill(&Path::circle(knob, 5.0), Color::WHITE);
            f.stroke(&Path::circle(knob, 5.0), Stroke::default().with_color(ACCENT).with_width(1.5));
        }

        let (gx, gy) = state.guides;
        let (ox, oy, w, h) = (self.origin.x, self.origin.y, self.display.width, self.display.height);
        if gx {
            f.fill_rectangle(Point::new(ox + w / 2.0 - 0.5, oy), Size::new(1.0, h), BRAND_2);
        }
        if gy {
            f.fill_rectangle(Point::new(ox, oy + h / 2.0 - 0.5), Size::new(w, 1.0), BRAND_2);
        }
        vec![f.into_geometry()]
    }

    fn mouse_interaction(&self, state: &State, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        match state.drag {
            Some(Drag::Move { moved: true, .. } | Drag::ZoomMove { .. }) => return mouse::Interaction::Grabbing,
            Some(Drag::Scale { .. } | Drag::ZoomScale { .. }) => return mouse::Interaction::ResizingDiagonallyDown,
            Some(Drag::Rotate { .. }) => return mouse::Interaction::Grabbing,
            _ => {}
        }
        let Some(p) = cursor.position_in(bounds) else { return mouse::Interaction::default() };
        let map = self.map(bounds);
        let p = map.to_canvas(p);
        let near = |a: Point| a.distance(p) <= HANDLE * map.k;
        if let Some((clip, z)) = self.zoom_edit {
            if let Some(l) = self.layer(clip) {
                let r = zoom_rect(l, &z);
                let cs = [r.position(), Point::new(r.x + r.width, r.y), Point::new(r.x + r.width, r.y + r.height), Point::new(r.x, r.y + r.height)];
                if cs.iter().any(|&c| near(c)) {
                    return mouse::Interaction::ResizingDiagonallyDown;
                }
                if r.contains(p) {
                    return mouse::Interaction::Grab;
                }
            }
            return mouse::Interaction::default();
        }
        if let Some(l) = self.selected_layer() {
            if near(self.knob(l, &map)) {
                return mouse::Interaction::Grab;
            }
            if corners(l).iter().any(|&c| near(c)) {
                return mouse::Interaction::ResizingDiagonallyDown;
            }
        }
        if self.scene.layers.iter().any(|l| contains(l, p)) { mouse::Interaction::Move } else { mouse::Interaction::default() }
    }
}

fn poly(points: &[Point; 4]) -> Path {
    Path::new(|b| {
        b.move_to(points[0]);
        for p in &points[1..] {
            b.line_to(*p);
        }
        b.close();
    })
}

fn handle(f: &mut Frame, at: Point, color: Color) {
    let r = Path::rounded_rectangle(Point::new(at.x - 4.5, at.y - 4.5), Size::new(9.0, 9.0), 2.0.into());
    f.fill(&r, Color::WHITE);
    f.stroke(&r, Stroke::default().with_color(color).with_width(1.5));
}
