//! The timeline: ruler, text track, magnetic main track and music track.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use egui::{pos2, vec2, Align2, Color32, CornerRadius, CursorIcon, Id as EId, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui};

use super::media_cache::{MediaCache, WAVE_PER_SEC};
use super::theme::{self, icons, *};
use super::AssetDrag;
use crate::media::MediaKind;
use crate::project::{fmt_time, AudioClip, Clip, Id, Project, Source, TextClip, MIN_CLIP};
use crate::shared::Shared;

const HEADER_W: f32 = 96.0;
const RULER_H: f32 = 26.0;
const TEXT_H: f32 = 34.0;
const MAIN_H: f32 = 72.0;
const MUSIC_H: f32 = 46.0;
const GAP: f32 = 6.0;
const EDGE: f32 = 7.0;
const SNAP_PX: f32 = 8.0;
const MAX_ZOOM: f32 = 400.0;

#[derive(Clone)]
enum Drag {
    Scrub,
    Reorder { id: Id, grab_dt: f64 },
    TrimMain { id: Id, left: bool, orig: Clip, origin: f32 },
    MoveMusic { id: Id, orig: AudioClip, origin: f32 },
    TrimMusic { id: Id, left: bool, orig: AudioClip, origin: f32 },
    MoveText { id: Id, orig: TextClip, origin: f32 },
    TrimText { id: Id, left: bool, orig: TextClip, origin: f32 },
}

struct Ghost {
    rect: Rect,
    color: Color32,
    born: Instant,
}

pub struct Timeline {
    /// Pixels per second: target and displayed (eased towards target).
    zoom: f32,
    zoom_shown: f32,
    /// Seconds at the left edge.
    scroll: f64,
    scroll_shown: f64,
    drag: Option<Drag>,
    drag_serial: u64,
    seen: HashSet<Id>,
    born: HashMap<Id, Instant>,
    last_rects: HashMap<Id, (Rect, Color32)>,
    ghosts: Vec<Ghost>,
    fitted_once: bool,
    snap_line: Option<(f64, Instant)>,
    open_properties: bool,
}

#[derive(Default)]
pub struct TimelineOutput {
    pub seek: Option<f64>,
    pub scrubbing: bool,
    pub dropped_asset: Option<(Id, DropTarget)>,
    /// A clip was double-clicked: show its properties.
    pub open_properties: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum DropTarget {
    Main(usize),
    Music(f64),
}

impl Default for Timeline {
    fn default() -> Self {
        Timeline {
            zoom: 40.0,
            zoom_shown: 40.0,
            scroll: 0.0,
            scroll_shown: 0.0,
            drag: None,
            drag_serial: 0,
            seen: HashSet::new(),
            born: HashMap::new(),
            last_rects: HashMap::new(),
            ghosts: Vec::new(),
            fitted_once: false,
            snap_line: None,
            open_properties: false,
        }
    }
}

fn ruler_step(zoom: f32) -> f64 {
    const STEPS: [f64; 12] = [0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1800.0];
    STEPS.into_iter().find(|s| *s as f32 * zoom >= 80.0).unwrap_or(3600.0)
}

fn ruler_label(t: f64) -> String {
    let m = (t / 60.0).floor() as u64;
    let s = t - m as f64 * 60.0;
    if (s - s.round()).abs() < 1e-6 {
        format!("{m}:{:02}", s.round() as u64)
    } else {
        format!("{m}:{s:04.1}")
    }
}

impl Timeline {
    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    pub fn zoom_to_fit(&mut self, duration: f64, width: f32) {
        if duration > 0.0 && width > 50.0 {
            self.zoom = ((width - 40.0) / duration as f32).clamp(2.0, MAX_ZOOM);
            self.scroll = 0.0;
        }
    }

    pub fn zoom_by(&mut self, factor: f32) {
        self.zoom = (self.zoom * factor).clamp(2.0, MAX_ZOOM);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ui: &mut Ui,
        shared: &Shared,
        project: &Arc<Project>,
        playhead: f64,
        playing: bool,
        selected: &mut Option<Id>,
        media: &MediaCache,
        ai_touched: &HashMap<Id, Instant>,
    ) -> TimelineOutput {
        let mut out = TimelineOutput::default();
        let ctx = ui.ctx().clone();
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        let total = project.duration();

        let full = ui.available_rect_before_wrap();
        let rect = Rect::from_min_size(full.min, vec2(full.width(), (RULER_H + TEXT_H + MAIN_H + MUSIC_H + GAP * 4.0).max(full.height())));
        let content_left = rect.left() + HEADER_W;
        let view_w = (rect.right() - content_left).max(10.0);

        if !self.fitted_once && total > 0.0 {
            self.fitted_once = true;
            self.zoom_to_fit(total, view_w);
            self.zoom_shown = self.zoom;
        }

        // ----- Input: wheel scroll and ctrl+wheel zoom ------------------------
        let bg = ui.interact(rect, ui.id().with("timeline-bg"), Sense::click_and_drag());
        if let Some(pos) = ui.ctx().pointer_hover_pos().filter(|p| rect.contains(*p)) {
            let (scroll, zoom_delta) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta()));
            if zoom_delta != 1.0 {
                let anchor_t = self.scroll_shown + ((pos.x - content_left) / self.zoom_shown) as f64;
                self.zoom = (self.zoom * zoom_delta).clamp(2.0, MAX_ZOOM);
                self.zoom_shown = self.zoom;
                self.scroll = anchor_t - ((pos.x - content_left) / self.zoom) as f64;
                self.scroll_shown = self.scroll;
            } else if scroll != egui::Vec2::ZERO {
                let d = if scroll.x.abs() > scroll.y.abs() { scroll.x } else { scroll.y };
                self.scroll -= (d / self.zoom) as f64;
            }
        }
        let max_scroll = (total - (view_w * 0.5 / self.zoom) as f64).max(0.0);
        self.scroll = self.scroll.clamp(0.0, max_scroll.max(0.0));

        // Follow the playhead while playing.
        if playing {
            let visible = (view_w / self.zoom) as f64;
            if playhead > self.scroll + visible * 0.88 || playhead < self.scroll {
                self.scroll = (playhead - visible * 0.12).max(0.0);
            }
        }

        // Ease displayed zoom/scroll towards targets.
        let k = 1.0 - (-dt * 16.0).exp();
        self.zoom_shown += (self.zoom - self.zoom_shown) * k;
        self.scroll_shown += (self.scroll - self.scroll_shown) * k as f64;
        if (self.zoom - self.zoom_shown).abs() > 0.01 || (self.scroll - self.scroll_shown).abs() > 0.0005 {
            ctx.request_repaint();
        } else {
            self.zoom_shown = self.zoom;
            self.scroll_shown = self.scroll;
        }
        let zoom = self.zoom_shown;
        let scroll = self.scroll_shown;
        let x_of = |t: f64| content_left + ((t - scroll) as f32) * zoom;
        let t_of = |x: f32| scroll + ((x - content_left) / zoom) as f64;

        // ----- Layout --------------------------------------------------------------
        let ruler = Rect::from_min_size(pos2(content_left, rect.top()), vec2(view_w, RULER_H));
        let text_row = Rect::from_min_size(pos2(rect.left(), ruler.bottom() + GAP), vec2(rect.width(), TEXT_H));
        let main_row = Rect::from_min_size(pos2(rect.left(), text_row.bottom() + GAP), vec2(rect.width(), MAIN_H));
        let music_row = Rect::from_min_size(pos2(rect.left(), main_row.bottom() + GAP), vec2(rect.width(), MUSIC_H));
        let tracks_clip = Rect::from_min_max(pos2(content_left, rect.top()), rect.max);

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::ZERO, BG);

        // Track headers.
        for (row, label, icon, color) in [
            (text_row, "Text", icons::TEXT_T, CLIP_TEXT),
            (main_row, "Video", icons::FILM_STRIP, CLIP_VIDEO),
            (music_row, "Music", icons::MUSIC_NOTES, CLIP_MUSIC),
        ] {
            let lane = Rect::from_min_max(pos2(content_left, row.top()), pos2(rect.right(), row.bottom()));
            painter.rect_filled(lane, CornerRadius::same(R_SM), Color32::from_rgb(18, 21, 27));
            let head = Rect::from_min_size(row.min, vec2(HEADER_W - 8.0, row.height()));
            painter.rect_filled(head, CornerRadius::same(R_SM), SURFACE);
            painter.circle_filled(pos2(head.left() + 18.0, head.center().y), 10.0, alpha(color, 0.25));
            painter.text(pos2(head.left() + 18.0, head.center().y), Align2::CENTER_CENTER, icon, theme::bold_font(12.0), color);
            painter.text(pos2(head.left() + 34.0, head.center().y), Align2::LEFT_CENTER, label, theme::font(13.0), TEXT_DIM);
        }

        let tp = painter.with_clip_rect(tracks_clip);

        // Ruler.
        tp.rect_filled(ruler, CornerRadius::ZERO, PANEL);
        let step = ruler_step(zoom);
        let minor = step / 5.0;
        let t_start = (t_of(content_left) / minor).floor() * minor;
        let mut t = t_start.max(0.0);
        let t_end = t_of(rect.right());
        while t <= t_end {
            let x = x_of(t);
            let is_major = ((t / step).round() * step - t).abs() < 1e-6;
            let h = if is_major { 10.0 } else { 5.0 };
            tp.line_segment([pos2(x, ruler.bottom() - h), pos2(x, ruler.bottom())], Stroke::new(1.0_f32, if is_major { TEXT_FAINT } else { BORDER }));
            if is_major {
                tp.text(pos2(x + 4.0, ruler.top() + 3.0), Align2::LEFT_TOP, ruler_label(t), theme::font(11.0), TEXT_FAINT);
            }
            t += minor;
        }
        // End-of-video marker.
        if total > 0.0 {
            let x = x_of(total);
            tp.line_segment([pos2(x, ruler.bottom()), pos2(x, rect.bottom())], Stroke::new(1.0_f32, alpha(TEXT_FAINT, 0.5)));
        }

        let pointer = ui.ctx().pointer_interact_pos();
        let pointer_x = pointer.map(|p| p.x).unwrap_or(0.0);
        let starts = project.main_starts();
        let mut current_rects: HashMap<Id, (Rect, Color32)> = HashMap::new();
        let gesture = self.drag.is_some();
        let anim_secs = if matches!(self.drag, Some(Drag::TrimMain { .. })) { 0.0 } else { 0.2 };

        // Clip appear animation.
        let now = Instant::now();
        let all_ids: Vec<Id> = project.main.iter().map(|c| c.id).chain(project.music.iter().map(|c| c.id)).chain(project.texts.iter().map(|c| c.id)).collect();
        for id in &all_ids {
            if self.seen.insert(*id) {
                self.born.insert(*id, now);
            }
        }
        let appear = |born: &HashMap<Id, Instant>, id: Id| born.get(&id).map(|b| theme::since(&ctx, *b, 0.28)).unwrap_or(1.0);

        // ----- Main track ---------------------------------------------------------
        let reorder = match &self.drag {
            Some(Drag::Reorder { id, grab_dt }) => Some((*id, *grab_dt)),
            _ => None,
        };
        // While reordering, lay out the other clips around a gap at the insertion point.
        let mut layout: Vec<(usize, f64)> = Vec::new(); // (clip index, start)
        let mut insert_index = None;
        if let Some((rid, grab_dt)) = reorder {
            let ri = project.main_index(rid);
            let rdur = ri.map(|i| project.main[i].duration()).unwrap_or(0.0);
            let drag_start = t_of(pointer_x) - grab_dt;
            let drag_mid = drag_start + rdur / 2.0;
            let others: Vec<usize> = (0..project.main.len()).filter(|i| Some(*i) != ri).collect();
            let mut acc = 0.0;
            let mut idx = others.len();
            for (k, &i) in others.iter().enumerate() {
                let d = project.main[i].duration();
                if drag_mid < acc + d / 2.0 {
                    idx = k;
                    break;
                }
                acc += d;
            }
            insert_index = Some(idx);
            let mut t = 0.0;
            for (k, &i) in others.iter().enumerate() {
                if k == idx {
                    t += rdur;
                }
                layout.push((i, t));
                t += project.main[i].duration();
            }
        } else {
            layout = (0..project.main.len()).map(|i| (i, starts[i])).collect();
        }

        let main_lane = Rect::from_min_max(pos2(content_left, main_row.top()), pos2(rect.right(), main_row.bottom()));
        if project.main.is_empty() {
            let r = Rect::from_min_size(pos2(content_left + 8.0, main_row.top() + 8.0), vec2((view_w - 16.0).min(420.0), main_row.height() - 16.0));
            dashed_rect(&tp, r, alpha(TEXT_FAINT, 0.6));
            tp.text(r.center(), Align2::CENTER_CENTER, "Drop videos or images here", theme::font(13.0), TEXT_FAINT);
        }

        let mut hovered_edge = false;
        for &(i, start) in &layout {
            let clip = &project.main[i];
            let a_start = ctx.animate_value_with_time(EId::new(("cs", clip.id)), start as f32, anim_secs) as f64;
            let a_dur = ctx.animate_value_with_time(EId::new(("cd", clip.id)), clip.duration() as f32, anim_secs) as f64;
            let x0 = x_of(a_start);
            let x1 = x_of(a_start + a_dur);
            let grow = appear(&self.born, clip.id);
            let full_r = Rect::from_min_max(pos2(x0 + 1.0, main_row.top()), pos2(x1 - 1.0, main_row.bottom()));
            let r = Rect::from_center_size(full_r.center(), vec2(full_r.width(), full_r.height() * (0.7 + 0.3 * grow)));
            current_rects.insert(clip.id, (r, CLIP_VIDEO));
            if r.right() < content_left || r.left() > rect.right() {
                continue;
            }
            let resp = ui.interact(r.intersect(main_lane), EId::new(("clip", clip.id)), Sense::click_and_drag());
            let edge = edge_hit(&resp, r, pointer);
            if edge.is_some() && !gesture {
                hovered_edge = true;
            }
            self.paint_main_clip(&ctx, &tp, project, clip, r, media, grow, *selected == Some(clip.id), resp.hovered(), ai_touched.get(&clip.id), edge);
            self.clip_interactions(&resp, r, clip.id, selected, |left, origin| match left {
                Some(left) => Drag::TrimMain { id: clip.id, left, orig: clip.clone(), origin },
                None => Drag::Reorder { id: clip.id, grab_dt: t_of(origin) - start },
            });
        }

        // Dragged clip floats under the pointer.
        if let Some((rid, grab_dt)) = reorder {
            if let Some(i) = project.main_index(rid) {
                let clip = &project.main[i];
                let s = t_of(pointer_x) - grab_dt;
                let r = Rect::from_min_size(pos2(x_of(s), main_row.top() - 6.0), vec2(clip.duration() as f32 * zoom, MAIN_H));
                let fp = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, EId::new("drag-clip")));
                theme::shadow(&fp, r, R_MD as f32, 1.2);
                self.paint_main_clip(&ctx, &fp, project, clip, r, media, 1.0, true, true, None, None);
                if let Some(idx) = insert_index {
                    let others: Vec<&Clip> = project.main.iter().filter(|c| c.id != rid).collect();
                    let t: f64 = others.iter().take(idx).map(|c| c.duration()).sum();
                    insertion_marker(&tp, x_of(t), main_row);
                }
            }
        }

        // ----- Text track ---------------------------------------------------------
        for t in &project.texts {
            let a_start = ctx.animate_value_with_time(EId::new(("ts", t.id)), t.start as f32, if gesture { 0.0 } else { 0.2 }) as f64;
            let a_dur = ctx.animate_value_with_time(EId::new(("td", t.id)), t.duration as f32, if gesture { 0.0 } else { 0.2 }) as f64;
            let grow = appear(&self.born, t.id);
            let full_r = Rect::from_min_max(pos2(x_of(a_start) + 1.0, text_row.top() + 2.0), pos2(x_of(a_start + a_dur) - 1.0, text_row.bottom() - 2.0));
            let r = Rect::from_center_size(full_r.center(), vec2(full_r.width(), full_r.height() * (0.6 + 0.4 * grow)));
            current_rects.insert(t.id, (r, CLIP_TEXT));
            if r.right() < content_left || r.left() > rect.right() {
                continue;
            }
            let resp = ui.interact(r.intersect(tracks_clip), EId::new(("clip", t.id)), Sense::click_and_drag());
            let edge = edge_hit(&resp, r, pointer);
            if edge.is_some() && !gesture {
                hovered_edge = true;
            }
            let sel = *selected == Some(t.id);
            paint_simple_clip(&ctx, &tp, r, CLIP_TEXT, sel, resp.hovered(), ai_touched.get(&t.id), grow, edge);
            let label = t.text.lines().next().unwrap_or("");
            tp.with_clip_rect(r.shrink(4.0).intersect(tracks_clip)).text(pos2(r.left() + 10.0, r.center().y), Align2::LEFT_CENTER, format!("{}  {label}", icons::TEXT_T), theme::font(12.5), Color32::WHITE);
            self.clip_interactions(&resp, r, t.id, selected, |left, origin| match left {
                Some(left) => Drag::TrimText { id: t.id, left, orig: t.clone(), origin },
                None => Drag::MoveText { id: t.id, orig: t.clone(), origin },
            });
        }

        // ----- Music track --------------------------------------------------------
        for m in &project.music {
            let a_start = ctx.animate_value_with_time(EId::new(("ms", m.id)), m.start as f32, if gesture { 0.0 } else { 0.2 }) as f64;
            let a_dur = ctx.animate_value_with_time(EId::new(("md", m.id)), m.duration() as f32, if gesture { 0.0 } else { 0.2 }) as f64;
            let grow = appear(&self.born, m.id);
            let full_r = Rect::from_min_max(pos2(x_of(a_start) + 1.0, music_row.top() + 2.0), pos2(x_of(a_start + a_dur) - 1.0, music_row.bottom() - 2.0));
            let r = Rect::from_center_size(full_r.center(), vec2(full_r.width(), full_r.height() * (0.6 + 0.4 * grow)));
            current_rects.insert(m.id, (r, CLIP_MUSIC));
            if r.right() < content_left || r.left() > rect.right() {
                continue;
            }
            let resp = ui.interact(r.intersect(tracks_clip), EId::new(("clip", m.id)), Sense::click_and_drag());
            let edge = edge_hit(&resp, r, pointer);
            if edge.is_some() && !gesture {
                hovered_edge = true;
            }
            let sel = *selected == Some(m.id);
            paint_simple_clip(&ctx, &tp, r, CLIP_MUSIC, sel, resp.hovered(), ai_touched.get(&m.id), grow, edge);
            let cp = tp.with_clip_rect(r.shrink(2.0).intersect(tracks_clip));
            if let Some(wave) = media.get(m.asset_id).and_then(|a| a.wave.clone()) {
                paint_wave(&cp, &wave, r.shrink2(vec2(2.0, 6.0)), m.src_in, zoom, content_left, rect.right(), alpha(Color32::WHITE, 0.45), true);
            }
            let name = project.asset(m.asset_id).map(|a| a.name.as_str()).unwrap_or("");
            cp.text(pos2(r.left() + 8.0, r.top() + 4.0), Align2::LEFT_TOP, format!("{}  {name}", icons::MUSIC_NOTES), theme::font(11.5), Color32::WHITE);
            self.clip_interactions(&resp, r, m.id, selected, |left, origin| match left {
                Some(left) => Drag::TrimMusic { id: m.id, left, orig: m.clone(), origin },
                None => Drag::MoveMusic { id: m.id, orig: m.clone(), origin },
            });
        }

        // Clips that disappeared fade out instead of vanishing.
        for (id, (r, color)) in &self.last_rects {
            if !current_rects.contains_key(id) {
                self.ghosts.push(Ghost { rect: *r, color: *color, born: now });
            }
        }
        self.last_rects = current_rects;
        self.ghosts.retain(|g| g.born.elapsed().as_secs_f32() < 0.22);
        for g in &self.ghosts {
            let t = theme::since(&ctx, g.born, 0.22);
            let r = Rect::from_center_size(g.rect.center(), g.rect.size() * vec2(1.0, 1.0 - 0.4 * t));
            tp.rect_filled(r, CornerRadius::same(R_SM), alpha(g.color, 0.6 * (1.0 - t)));
        }

        // ----- Gestures in progress ----------------------------------------------
        if let Some(drag) = self.drag.clone() {
            let released = !ui.input(|i| i.pointer.primary_down());
            let key = format!("drag:{}", self.drag_serial);
            let dtime = |origin: f32| ((pointer_x - origin) / zoom) as f64;
            let snap_points = || -> Vec<f64> {
                let mut v = vec![0.0, playhead, total];
                v.extend(starts.iter().copied());
                v.extend(project.music.iter().flat_map(|m| [m.start, m.end()]));
                v.extend(project.texts.iter().flat_map(|t| [t.start, t.end()]));
                v
            };
            let snap = |t: f64, exclude: (f64, f64), this: &mut Option<(f64, Instant)>| -> f64 {
                let tol = (SNAP_PX / zoom) as f64;
                let best = snap_points()
                    .into_iter()
                    .filter(|p| (p - exclude.0).abs() > 1e-9 && (p - exclude.1).abs() > 1e-9)
                    .min_by(|a, b| (a - t).abs().partial_cmp(&(b - t).abs()).unwrap());
                match best {
                    Some(b) if (b - t).abs() < tol => {
                        if this.is_none_or(|(s, _)| (s - b).abs() > 1e-9) {
                            *this = Some((b, Instant::now()));
                        }
                        b
                    }
                    _ => t,
                }
            };
            let mut ed = shared.editor.lock();
            match &drag {
                Drag::Scrub => {
                    out.scrubbing = true;
                    out.seek = Some(t_of(pointer_x).clamp(0.0, total));
                }
                Drag::Reorder { id, .. } => {
                    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                    if released {
                        if let Some(idx) = insert_index {
                            if project.main_index(*id) != Some(idx) {
                                let _ = ed.apply(Source::User, "Move clip", |p| p.move_main(*id, idx));
                            }
                        }
                    }
                }
                Drag::TrimMain { id, left, orig, origin } => {
                    ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
                    let d = dtime(*origin);
                    let (a, b) = if *left {
                        ((orig.src_in + d * orig.speed).clamp(0.0, orig.src_out - MIN_CLIP), orig.src_out)
                    } else {
                        let max = project.asset(orig.asset_id).map(|a| if a.kind == MediaKind::Image { 3600.0 } else { a.duration }).unwrap_or(orig.src_out);
                        (orig.src_in, (orig.src_out + d * orig.speed).clamp(orig.src_in + MIN_CLIP, max))
                    };
                    let cur = project.main_index(*id).map(|i| &project.main[i]);
                    if cur.is_some_and(|c| (c.src_in - a).abs() > 1e-6 || (c.src_out - b).abs() > 1e-6) {
                        let _ = ed.apply_merge(Source::User, "Trim clip", Some(&key), |p| p.trim_main(*id, a, b));
                    }
                    // Show the frame at the edge being trimmed.
                    if let Some(i) = ed.project.main_index(*id) {
                        let s = ed.project.main_starts()[i];
                        let edge_t = if *left { s } else { s + ed.project.main[i].duration() - ed.project.canvas.frame() };
                        out.seek = Some(edge_t.max(0.0));
                    }
                }
                Drag::MoveMusic { id, orig, origin } => {
                    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                    let raw = (orig.start + dtime(*origin)).max(0.0);
                    let dur = orig.duration();
                    let mut start = snap(raw, (orig.start, orig.end()), &mut self.snap_line);
                    if (start - raw).abs() < 1e-9 {
                        start = snap(raw + dur, (orig.start, orig.end()), &mut self.snap_line) - dur;
                    }
                    let start = start.max(0.0);
                    let _ = ed.apply_merge(Source::User, "Move music", Some(&key), |p| p.update_music(*id, Some(start), None, None));
                }
                Drag::TrimMusic { id, left, orig, origin } => {
                    ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
                    let d = dtime(*origin);
                    let max = project.asset(orig.asset_id).map(|a| a.duration).unwrap_or(orig.src_out);
                    let (start, a, b) = if *left {
                        let d = d.clamp(-orig.src_in.min(orig.start), orig.duration() - MIN_CLIP);
                        (orig.start + d, orig.src_in + d, orig.src_out)
                    } else {
                        let d = d.clamp(-(orig.duration() - MIN_CLIP), max - orig.src_out);
                        (orig.start, orig.src_in, orig.src_out + d)
                    };
                    let _ = ed.apply_merge(Source::User, "Trim music", Some(&key), |p| p.update_music(*id, Some(start), Some((a, b)), None));
                }
                Drag::MoveText { id, orig, origin } => {
                    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                    let raw = (orig.start + dtime(*origin)).max(0.0);
                    let mut start = snap(raw, (orig.start, orig.end()), &mut self.snap_line);
                    if (start - raw).abs() < 1e-9 {
                        start = snap(raw + orig.duration, (orig.start, orig.end()), &mut self.snap_line) - orig.duration;
                    }
                    let start = start.max(0.0);
                    let _ = ed.apply_merge(Source::User, "Move text", Some(&key), |p| {
                        p.text_mut(*id)?.start = start;
                        Ok(())
                    });
                }
                Drag::TrimText { id, left, orig, origin } => {
                    ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
                    let d = dtime(*origin);
                    let (start, dur) = if *left {
                        let edge = snap(orig.start + d, (orig.start, orig.end()), &mut self.snap_line).clamp(0.0, orig.end() - MIN_CLIP);
                        (edge, orig.end() - edge)
                    } else {
                        let edge = snap(orig.end() + d, (orig.start, orig.end()), &mut self.snap_line).max(orig.start + MIN_CLIP);
                        (orig.start, edge - orig.start)
                    };
                    let _ = ed.apply_merge(Source::User, "Trim text", Some(&key), |p| {
                        let t = p.text_mut(*id)?;
                        t.start = start;
                        t.duration = dur;
                        Ok(())
                    });
                }
            }
            drop(ed);
            if released {
                self.drag = None;
                self.snap_line = None;
            }
            ctx.request_repaint();
        } else if hovered_edge {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }

        // Snap guide flash.
        if let Some((t, at)) = self.snap_line {
            if self.drag.is_some() {
                let a = 1.0 - theme::since(&ctx, at, 0.6) * 0.6;
                let x = x_of(t);
                tp.line_segment([pos2(x, ruler.bottom()), pos2(x, rect.bottom())], Stroke::new(1.5_f32, alpha(ACCENT_HI, a)));
            }
        }

        // ----- Ruler / background: scrub and deselect -------------------------
        let ruler_resp = ui.interact(ruler, ui.id().with("ruler"), Sense::click_and_drag());
        if ruler_resp.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        if (ruler_resp.drag_started() || ruler_resp.clicked()) && total > 0.0 {
            if let Some(p) = ruler_resp.interact_pointer_pos() {
                out.seek = Some(t_of(p.x).clamp(0.0, total));
            }
            if ruler_resp.drag_started() {
                self.drag = Some(Drag::Scrub);
                out.scrubbing = true;
            }
        }
        if bg.clicked() {
            *selected = None;
            if let Some(p) = bg.interact_pointer_pos() {
                if p.x > content_left && total > 0.0 {
                    out.seek = Some(t_of(p.x).clamp(0.0, total));
                }
            }
        }

        // ----- Drops from the media library ------------------------------------
        if let Some(payload) = egui::DragAndDrop::payload::<AssetDrag>(&ctx) {
            if let Some(p) = ctx.pointer_hover_pos().filter(|p| tracks_clip.contains(*p)) {
                let kind = project.asset(payload.0).map(|a| a.kind);
                let target = if kind == Some(MediaKind::Audio) {
                    let t = t_of(p.x).max(0.0);
                    let x = x_of(t);
                    let preview = Rect::from_min_size(pos2(x, music_row.top() + 2.0), vec2(80.0, music_row.height() - 4.0));
                    tp.rect_filled(preview, CornerRadius::same(R_SM), alpha(CLIP_MUSIC, 0.45));
                    insertion_marker(&tp, x, music_row);
                    DropTarget::Music(t)
                } else {
                    let t = t_of(p.x);
                    let mut idx = project.main.len();
                    for (i, c) in project.main.iter().enumerate() {
                        if t < starts[i] + c.duration() / 2.0 {
                            idx = i;
                            break;
                        }
                    }
                    let edge_t = if idx < starts.len() { starts[idx] } else { total };
                    insertion_marker(&tp, x_of(edge_t), main_row);
                    DropTarget::Main(idx)
                };
                if ctx.input(|i| i.pointer.any_released()) {
                    out.dropped_asset = Some((payload.0, target));
                    egui::DragAndDrop::clear_payload(&ctx);
                }
            }
        }

        // ----- Playhead ----------------------------------------------------------
        if total > 0.0 || playhead > 0.0 {
            let x = x_of(playhead);
            if x >= content_left - 1.0 && x <= rect.right() + 1.0 {
                tp.line_segment([pos2(x, ruler.top() + 4.0), pos2(x, rect.bottom())], Stroke::new(1.5_f32, PLAYHEAD));
                let head = Rect::from_center_size(pos2(x, ruler.top() + 9.0), vec2(12.0, 14.0));
                tp.add(Shape::convex_polygon(
                    vec![head.left_top(), head.right_top(), pos2(head.right(), head.bottom() - 4.0), pos2(x, head.bottom() + 2.0), pos2(head.left(), head.bottom() - 4.0)],
                    PLAYHEAD,
                    Stroke::NONE,
                ));
            }
        }

        ui.allocate_rect(rect, Sense::hover());
        out.open_properties = std::mem::take(&mut self.open_properties);
        out
    }

    /// Selection, drag start and edge detection shared by every clip type.
    fn clip_interactions(
        &mut self,
        resp: &Response,
        r: Rect,
        id: Id,
        selected: &mut Option<Id>,
        make: impl FnOnce(Option<bool>, f32) -> Drag,
    ) {
        if resp.clicked() {
            *selected = Some(id);
        }
        if resp.double_clicked() {
            *selected = Some(id);
            self.open_properties = true;
        }
        if resp.drag_started() && self.drag.is_none() {
            *selected = Some(id);
            self.drag_serial += 1;
            let origin = resp.interact_pointer_pos().map(|p| p.x).unwrap_or(0.0);
            // Decide trim-vs-move from where the press began: egui only reports a drag after a
            // few pixels of movement, by which time the pointer may have left the edge zone.
            let press = resp.ctx.input(|i| i.pointer.press_origin()).map(|p| p.x).unwrap_or(origin);
            self.drag = Some(make(edge_at(r, press), press));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_main_clip(
        &self,
        ctx: &egui::Context,
        p: &egui::Painter,
        project: &Project,
        clip: &Clip,
        r: Rect,
        media: &MediaCache,
        grow: f32,
        selected: bool,
        hovered: bool,
        ai: Option<&Instant>,
        edge: Option<bool>,
    ) {
        let asset = project.asset(clip.asset_id);
        let hover_t = ctx.animate_bool_with_time(EId::new(("ch", clip.id)), hovered || selected, 0.12);
        let base = theme::lerp_color(Color32::from_rgb(34, 52, 92), Color32::from_rgb(44, 66, 116), hover_t);
        p.rect_filled(r, CornerRadius::same(R_SM), alpha(base, 0.6 + 0.4 * grow));
        let inner = r.shrink(2.0);
        let cp = p.with_clip_rect(inner.intersect(p.clip_rect()));
        // Thumbnail strip.
        if let (Some(asset), Some(am)) = (asset, media.get(clip.asset_id)) {
            let th = inner.height();
            let tw = (th * 16.0 / 9.0).max(8.0);
            let visible_left = inner.left().max(p.clip_rect().left());
            let first = ((visible_left - inner.left()) / tw).floor().max(0.0) as i32;
            let mut x = inner.left() + first as f32 * tw;
            // Pixels per second of *source* media (differs from the timeline when speed != 1).
            let zoom = r.width() / (clip.src_out - clip.src_in).max(1e-6) as f32;
            while x < inner.right().min(p.clip_rect().right()) {
                let src_t = clip.src_in + ((x + tw / 2.0 - inner.left()) / zoom) as f64;
                if let Some(th_img) = am.thumb_at(src_t, asset.duration) {
                    let a = theme::since(ctx, th_img.loaded, 0.35);
                    let tile = Rect::from_min_size(pos2(x, inner.top()), vec2(tw, th));
                    cp.image(th_img.tex.id(), tile, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), alpha(Color32::WHITE, a * (0.75 + 0.25 * hover_t)));
                }
                x += tw;
            }
            // Waveform along the bottom.
            if let Some(wave) = &am.wave {
                let band = Rect::from_min_max(pos2(inner.left(), inner.bottom() - 16.0), inner.max);
                cp.rect_filled(band, CornerRadius::ZERO, Color32::from_black_alpha(110));
                paint_wave(&cp, wave, band.shrink2(vec2(0.0, 2.0)), clip.src_in, zoom, p.clip_rect().left(), p.clip_rect().right(), alpha(Color32::from_rgb(170, 200, 255), 0.7), false);
            }
            // Name chip.
            let speed = if (clip.speed - 1.0).abs() > 1e-3 { format!("  {} {}×", icons::GAUGE, (clip.speed * 100.0).round() / 100.0) } else { String::new() };
            let label = format!("{}  {}{speed}", asset.name, fmt_time(clip.duration()));
            let galley = p.layout_no_wrap(label, theme::font(11.5), Color32::WHITE);
            let chip = Rect::from_min_size(inner.left_top() + vec2(4.0, 4.0), galley.size() + vec2(10.0, 4.0));
            cp.rect_filled(chip, CornerRadius::same(4), Color32::from_black_alpha(150));
            cp.galley(chip.min + vec2(5.0, 2.0), galley, Color32::WHITE);
        }
        // Fade wedges.
        let px_per_s = r.width() / clip.duration().max(1e-6) as f32;
        for (fade, left) in [(clip.fade_in, true), (clip.fade_out, false)] {
            let w = (fade as f32 * px_per_s).min(r.width() / 2.0);
            if w > 2.0 {
                let (x0, x1) = if left { (r.left(), r.left() + w) } else { (r.right(), r.right() - w) };
                cp.add(Shape::convex_polygon(vec![pos2(x0, r.top()), pos2(x1, r.top()), pos2(x0, r.bottom())], Color32::from_black_alpha(120), Stroke::NONE));
                cp.line_segment([pos2(x1, r.top()), pos2(x0, r.bottom())], Stroke::new(1.0_f32, alpha(Color32::WHITE, 0.6)));
            }
        }
        if clip.has_color() {
            cp.text(pos2(inner.right() - 6.0, inner.top() + 4.0), Align2::RIGHT_TOP, icons::CIRCLE_HALF, theme::font(12.0), alpha(Color32::WHITE, 0.8));
        }
        paint_outline(ctx, p, r, selected, hover_t, ai, edge);
    }
}

fn edge_hit(resp: &Response, r: Rect, pointer: Option<Pos2>) -> Option<bool> {
    let p = pointer.filter(|_| resp.hovered() || resp.is_pointer_button_down_on())?;
    edge_at(r, p.x)
}

/// Which trim handle (left = true) is under `x`, if any.
fn edge_at(r: Rect, x: f32) -> Option<bool> {
    if r.width() < EDGE * 3.0 {
        return None;
    }
    if (x - r.left()).abs() <= EDGE {
        Some(true)
    } else if (x - r.right()).abs() <= EDGE {
        Some(false)
    } else {
        None
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_simple_clip(ctx: &egui::Context, p: &egui::Painter, r: Rect, color: Color32, selected: bool, hovered: bool, ai: Option<&Instant>, grow: f32, edge: Option<bool>) {
    let hover_t = ctx.animate_bool_with_time(EId::new(("sh", r.left() as i32, r.top() as i32)), hovered || selected, 0.12);
    let fill = theme::lerp_color(color.gamma_multiply(0.55), color.gamma_multiply(0.8), hover_t);
    p.rect_filled(r, CornerRadius::same(R_SM), alpha(fill, 0.5 + 0.5 * grow));
    paint_outline(ctx, p, r, selected, hover_t, ai, edge);
}

fn paint_outline(ctx: &egui::Context, p: &egui::Painter, r: Rect, selected: bool, hover_t: f32, ai: Option<&Instant>, edge: Option<bool>) {
    // AI glow: a soft pulse that fades over two seconds after the AI touches a clip.
    if let Some(at) = ai {
        let age = at.elapsed().as_secs_f32();
        if age < 2.4 {
            ctx.request_repaint();
            let fade = 1.0 - (age / 2.4);
            let pulse = 0.6 + 0.4 * (age * 7.0).sin().abs();
            for i in 1..=3 {
                p.rect_stroke(r.expand(i as f32 * 1.5), CornerRadius::same(R_SM + i * 2), Stroke::new(1.5_f32, alpha(AI_GLOW, fade * pulse / i as f32)), StrokeKind::Outside);
            }
        }
    }
    if selected {
        p.rect_stroke(r, CornerRadius::same(R_SM), Stroke::new(2.0_f32, Color32::WHITE), StrokeKind::Inside);
    } else {
        p.rect_stroke(r, CornerRadius::same(R_SM), Stroke::new(1.0_f32, alpha(Color32::WHITE, 0.08 + 0.2 * hover_t)), StrokeKind::Inside);
    }
    // Trim handles.
    if selected || hover_t > 0.01 {
        let a = if selected { 1.0 } else { hover_t };
        for (left, x) in [(true, r.left() + 3.5), (false, r.right() - 3.5)] {
            let active = edge == Some(left);
            let h = if active { r.height() * 0.6 } else { r.height() * 0.36 };
            let w = if active { 4.0 } else { 3.0 };
            p.rect_filled(Rect::from_center_size(pos2(x, r.center().y), vec2(w, h)), CornerRadius::same(2), alpha(Color32::WHITE, a * if active { 1.0 } else { 0.7 }));
        }
    }
}

/// Peaks drawn as mirrored bars, two pixels apart.
#[allow(clippy::too_many_arguments)]
fn paint_wave(p: &egui::Painter, wave: &[f32], r: Rect, src_in: f64, zoom: f32, vis_left: f32, vis_right: f32, color: Color32, mirrored: bool) {
    let x_start = r.left().max(vis_left);
    let x_end = r.right().min(vis_right);
    let mut x = x_start;
    let mid = if mirrored { r.center().y } else { r.bottom() };
    let half = if mirrored { r.height() / 2.0 } else { r.height() };
    let per_px = WAVE_PER_SEC as f64 / zoom as f64;
    while x < x_end {
        let t0 = src_in + ((x - r.left()) / zoom) as f64;
        let i0 = (t0 * WAVE_PER_SEC as f64) as usize;
        let i1 = (i0 + (per_px * 2.0).ceil() as usize).max(i0 + 1).min(wave.len());
        if i0 >= wave.len() {
            break;
        }
        let v = wave[i0..i1].iter().cloned().fold(0.0f32, f32::max);
        let h = (v * half).max(0.5);
        let (y0, y1) = if mirrored { (mid - h, mid + h) } else { (mid - h, mid) };
        p.line_segment([pos2(x, y0), pos2(x, y1)], Stroke::new(1.2_f32, color));
        x += 2.0;
    }
}

fn insertion_marker(p: &egui::Painter, x: f32, row: Rect) {
    for (w, a) in [(8.0f32, 0.15f32), (4.0, 0.35), (2.0, 1.0)] {
        p.line_segment([pos2(x, row.top() - 2.0), pos2(x, row.bottom() + 2.0)], Stroke::new(w, alpha(ACCENT_HI, a)));
    }
}

pub fn dashed_rect(p: &egui::Painter, r: Rect, color: Color32) {
    let stroke = Stroke::new(1.2_f32, color);
    let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    for w in pts.windows(2) {
        p.extend(Shape::dashed_line(&[w[0], w[1]], stroke, 6.0, 5.0));
    }
}

#[cfg(test)]
mod tests {
    //! Headless interaction tests: real pointer events through the real widget.
    use super::*;
    use crate::media::ProbeInfo;
    use crate::project::{Canvas, Editor};
    use egui::{Event, Modifiers, PointerButton};
    use egui_kittest::Harness;
    use std::path::Path;

    struct State {
        tl: Timeline,
        shared: Shared,
        media: MediaCache,
        selected: Option<Id>,
        rect: Rect,
        playhead: f64,
        themed: bool,
        dropped: Option<(Id, DropTarget)>,
    }

    fn setup(durations: &[f64], music: Option<(f64, f64)>) -> State {
        let mut p = Project::new("t", Canvas::default());
        for (i, d) in durations.iter().enumerate() {
            let info = ProbeInfo { kind: MediaKind::Video, duration: *d, width: 1920, height: 1080, fps: 30.0, has_audio: false };
            let id = p.add_asset(Path::new(&format!("Z:/missing/v{i}.mp4")), &info);
            p.insert_main(id, None, None).unwrap();
        }
        if let Some((start, dur)) = music {
            let info = ProbeInfo { kind: MediaKind::Audio, duration: dur, width: 0, height: 0, fps: 0.0, has_audio: true };
            let id = p.add_asset(Path::new("Z:/missing/m.mp3"), &info);
            p.add_music(id, start, None).unwrap();
        }
        let shared = Shared::new(Editor::new(p), Arc::new(|| {}));
        State { tl: Timeline::default(), shared, media: MediaCache::new(Arc::new(|| {})), selected: None, rect: Rect::NOTHING, playhead: 0.0, themed: false, dropped: None }
    }

    fn harness(state: State) -> Harness<'static, State> {
        let mut h = Harness::builder().with_size(vec2(1100.0, 320.0)).build_ui_state(
            |ui, s: &mut State| {
                // Fonts registered now take effect next frame, so skip drawing this one.
                if !s.themed {
                    theme::install(ui.ctx());
                    s.themed = true;
                    ui.ctx().request_repaint();
                    return;
                }
                s.rect = ui.available_rect_before_wrap();
                let snap = Arc::new(s.shared.editor.lock().project.clone());
                let out = s.tl.show(ui, &s.shared, &snap, s.playhead, false, &mut s.selected, &s.media, &HashMap::new());
                if let Some(t) = out.seek {
                    s.playhead = t;
                }
                if out.dropped_asset.is_some() {
                    s.dropped = out.dropped_asset;
                }
            },
            state,
        );
        // Let appear animations settle.
        for _ in 0..3 {
            h.step();
        }
        h
    }

    fn x_of(h: &Harness<State>, t: f64) -> f32 {
        let s = h.state();
        s.rect.left() + HEADER_W + ((t - s.tl.scroll_shown) as f32) * s.tl.zoom_shown
    }

    fn main_y(h: &Harness<State>) -> f32 {
        h.state().rect.top() + RULER_H + GAP + TEXT_H + GAP + MAIN_H / 2.0
    }

    fn music_y(h: &Harness<State>) -> f32 {
        h.state().rect.top() + RULER_H + GAP + TEXT_H + GAP + MAIN_H + GAP + MUSIC_H / 2.0
    }

    fn push(h: &mut Harness<State>, e: Event) {
        h.input_mut().events.push(e);
        h.step();
    }

    fn button(pos: Pos2, pressed: bool) -> Event {
        Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
    }

    fn drag(h: &mut Harness<State>, from: Pos2, to: Pos2) {
        push(h, Event::PointerMoved(from));
        push(h, button(from, true));
        for i in 1..=12 {
            push(h, Event::PointerMoved(from + (to - from) * (i as f32 / 12.0)));
        }
        push(h, button(to, false));
        h.step();
    }

    #[test]
    fn trimming_the_right_edge_shortens_the_clip_in_one_undo_step() {
        let mut h = harness(setup(&[10.0], None));
        let zoom = h.state().tl.zoom_shown;
        let y = main_y(&h);
        let from = pos2(x_of(&h, 10.0) - 3.0, y);
        drag(&mut h, from, from - vec2(100.0, 0.0));
        let ed = h.state().shared.editor.lock();
        let d = ed.project.main[0].duration();
        let expected = 10.0 - 100.0 / zoom as f64;
        assert!((d - expected).abs() < 0.05, "duration {d}, expected {expected}");
        assert!((ed.project.main[0].src_in).abs() < 1e-9, "left edge must not move");
        drop(ed);
        assert!(h.state().tl.drag.is_none(), "drag must end on release");
        let mut ed = h.state().shared.editor.lock();
        ed.undo(Source::User).unwrap();
        assert!((ed.project.main[0].duration() - 10.0).abs() < 1e-9, "the whole drag is one undo step");
        assert!(!ed.can_undo());
    }

    #[test]
    fn trimming_the_left_edge_moves_the_in_point() {
        let mut h = harness(setup(&[10.0], None));
        let zoom = h.state().tl.zoom_shown;
        let from = pos2(x_of(&h, 0.0) + 3.0, main_y(&h));
        drag(&mut h, from, from + vec2(80.0, 0.0));
        let ed = h.state().shared.editor.lock();
        let c = &ed.project.main[0];
        assert!((c.src_in - 80.0 / zoom as f64).abs() < 0.05, "src_in {}", c.src_in);
        assert!((c.src_out - 10.0).abs() < 1e-9);
    }

    #[test]
    fn dragging_a_clip_body_reorders_it() {
        let mut h = harness(setup(&[4.0, 6.0], None));
        let first = h.state().shared.editor.lock().project.main[0].id;
        let y = main_y(&h);
        let (a, b) = (pos2(x_of(&h, 2.0), y), pos2(x_of(&h, 9.5), y));
        drag(&mut h, a, b);
        let ed = h.state().shared.editor.lock();
        assert_eq!(ed.project.main[1].id, first, "first clip should now be second");
        assert!((ed.project.duration() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn small_drags_near_an_edge_still_trim() {
        // The press is on the edge zone but the drag only registers after a few pixels.
        let mut h = harness(setup(&[10.0], None));
        let from = pos2(x_of(&h, 10.0) - 6.0, main_y(&h));
        drag(&mut h, from, from - vec2(30.0, 0.0));
        let ed = h.state().shared.editor.lock();
        assert_eq!(ed.project.main.len(), 1);
        assert!(ed.project.main[0].duration() < 10.0);
    }

    #[test]
    fn dragging_music_moves_it() {
        let mut h = harness(setup(&[10.0], Some((0.0, 4.0))));
        let zoom = h.state().tl.zoom_shown;
        let y = music_y(&h);
        let from = pos2(x_of(&h, 2.0), y);
        let dx = 3.0 * zoom;
        drag(&mut h, from, from + vec2(dx, 0.0));
        let ed = h.state().shared.editor.lock();
        let m = &ed.project.music[0];
        assert!((m.start - 3.0).abs() < 0.05, "music start {}", m.start);
        assert!((m.duration() - 4.0).abs() < 1e-9);
    }

    #[test]
    fn clicking_the_ruler_seeks() {
        let mut h = harness(setup(&[10.0], None));
        let p = pos2(x_of(&h, 3.0), h.state().rect.top() + RULER_H / 2.0);
        push(&mut h, Event::PointerMoved(p));
        push(&mut h, button(p, true));
        push(&mut h, button(p, false));
        assert!((h.state().playhead - 3.0).abs() < 0.05, "playhead {}", h.state().playhead);
    }

    #[test]
    fn dropping_library_media_reports_the_insert_position() {
        let mut h = harness(setup(&[4.0, 6.0], None));
        let asset = h.state().shared.editor.lock().project.assets[0].id;
        let p = pos2(x_of(&h, 3.5), main_y(&h));
        push(&mut h, Event::PointerMoved(p));
        egui::DragAndDrop::set_payload(&h.ctx, AssetDrag(asset));
        push(&mut h, Event::PointerMoved(p + vec2(1.0, 0.0)));
        push(&mut h, button(p, true));
        push(&mut h, button(p, false));
        match h.state().dropped {
            Some((id, DropTarget::Main(1))) => assert_eq!(id, asset),
            other => panic!("expected a drop between the clips, got {other:?}"),
        }
    }

    #[test]
    fn clicking_a_clip_selects_it() {
        let mut h = harness(setup(&[4.0, 6.0], None));
        let second = h.state().shared.editor.lock().project.main[1].id;
        let p = pos2(x_of(&h, 7.0), main_y(&h));
        push(&mut h, Event::PointerMoved(p));
        push(&mut h, button(p, true));
        push(&mut h, button(p, false));
        assert_eq!(h.state().selected, Some(second));
    }
}
