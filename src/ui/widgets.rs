//! Small animated widgets used across the app.

use egui::{pos2, vec2, Align2, Color32, CornerRadius, Id, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2};

use super::theme::{self, *};

/// Filled accent button with a hover lift.
pub fn primary_button(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    button_impl(ui, text, enabled, true, None)
}

pub fn secondary_button(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    button_impl(ui, text, enabled, false, None)
}

fn button_impl(ui: &mut Ui, text: &str, enabled: bool, primary: bool, width: Option<f32>) -> Response {
    let font = if primary { theme::bold_font(14.0) } else { theme::font(14.0) };
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, TEXT);
    let size = vec2(width.unwrap_or(galley.size().x + 28.0), 34.0);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let ctx = ui.ctx();
    let h = ctx.animate_bool_with_time(resp.id.with("h"), enabled && resp.hovered(), 0.12);
    let press = ctx.animate_bool_with_time(resp.id.with("p"), enabled && resp.is_pointer_button_down_on(), 0.08);
    let r = rect.shrink(press * 1.0);
    let (fill, text_color, stroke) = if primary {
        (theme::lerp_color(ACCENT, ACCENT_HI, h), Color32::WHITE, Stroke::NONE)
    } else {
        (theme::lerp_color(SURFACE, SURFACE_HI, h), TEXT, Stroke::new(1.0_f32, theme::lerp_color(BORDER, alpha(ACCENT_HI, 0.6), h)))
    };
    let p = ui.painter();
    if primary && enabled {
        p.rect_filled(r.expand(2.0 + 2.0 * h).translate(vec2(0.0, 1.0)), CornerRadius::same(R_SM + 3), alpha(ACCENT, 0.12 + 0.18 * h));
    }
    let fill = if enabled { fill } else { alpha(fill, 0.4) };
    p.rect(r, CornerRadius::same(R_SM), fill, stroke, StrokeKind::Inside);
    p.galley(r.center() - galley.size() / 2.0, galley, if enabled { text_color } else { alpha(text_color, 0.45) });
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// Square icon button. `glyph` is a single symbol character.
pub fn icon_button(ui: &mut Ui, glyph: &str, tooltip: &str, enabled: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(32.0), if enabled { Sense::click() } else { Sense::hover() });
    let h = ui.ctx().animate_bool_with_time(resp.id.with("h"), enabled && resp.hovered(), 0.12);
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(R_SM), alpha(SURFACE_HI, h));
    let color = if enabled { theme::lerp_color(TEXT_DIM, TEXT, h) } else { alpha(TEXT_FAINT, 0.6) };
    p.text(rect.center(), Align2::CENTER_CENTER, glyph, theme::font(16.0), color);
    let resp = resp.on_hover_text(tooltip);
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// Selectable pill with an animated selection fill.
pub fn chip(ui: &mut Ui, label: &str, sub: Option<&str>, selected: bool) -> Response {
    let g = ui.painter().layout_no_wrap(label.to_string(), theme::bold_font(13.5), TEXT);
    let sg = sub.map(|s| ui.painter().layout_no_wrap(s.to_string(), theme::font(11.0), TEXT_DIM));
    let w = g.size().x.max(sg.as_ref().map(|s| s.size().x).unwrap_or(0.0)) + 26.0;
    let hgt = if sg.is_some() { 46.0 } else { 32.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(w.max(64.0), hgt), Sense::click());
    let ctx = ui.ctx();
    let s = theme::ease_bool(ctx, resp.id.with("s"), selected, 0.16);
    let h = ctx.animate_bool_with_time(resp.id.with("h"), resp.hovered(), 0.12);
    let p = ui.painter();
    p.rect(
        rect,
        CornerRadius::same(R_SM + 2),
        theme::lerp_color(theme::lerp_color(SURFACE, SURFACE_HI, h), alpha(ACCENT, 0.28), s),
        Stroke::new(1.0 + s, theme::lerp_color(BORDER, ACCENT_HI, s)),
        StrokeKind::Inside,
    );
    let text_col = theme::lerp_color(TEXT_DIM, Color32::WHITE, s.max(h * 0.6));
    match sg {
        Some(sg) => {
            p.galley(pos2(rect.center().x - g.size().x / 2.0, rect.top() + 6.0), g, text_col);
            p.galley(pos2(rect.center().x - sg.size().x / 2.0, rect.bottom() - 6.0 - sg.size().y), sg, TEXT_DIM);
        }
        None => {
            p.galley(rect.center() - g.size() / 2.0, g, text_col);
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Aspect-ratio card showing the frame shape.
pub fn aspect_card(ui: &mut Ui, label: &str, hint: &str, aw: u32, ah: u32, selected: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(104.0, 112.0), Sense::click());
    let ctx = ui.ctx();
    let s = theme::ease_bool(ctx, resp.id.with("s"), selected, 0.18);
    let h = theme::ease_bool(ctx, resp.id.with("h"), resp.hovered(), 0.12);
    let lift = h * 2.0;
    let r = rect.translate(vec2(0.0, -lift));
    let p = ui.painter();
    if s > 0.0 {
        p.rect_filled(r.expand(3.0), CornerRadius::same(R_MD + 3), alpha(ACCENT, 0.18 * s));
    }
    p.rect(
        r,
        CornerRadius::same(R_MD),
        theme::lerp_color(theme::lerp_color(SURFACE, SURFACE_HI, h), Color32::from_rgb(22, 40, 76), s),
        Stroke::new(1.0 + s, theme::lerp_color(BORDER, ACCENT_HI, s)),
        StrokeKind::Inside,
    );
    // Frame shape preview.
    let box_area = Rect::from_center_size(pos2(r.center().x, r.top() + 38.0), vec2(56.0, 44.0));
    let scale = (box_area.width() / aw as f32).min(box_area.height() / ah as f32);
    let shape = Rect::from_center_size(box_area.center(), vec2(aw as f32 * scale, ah as f32 * scale));
    p.rect(shape, CornerRadius::same(3), alpha(theme::lerp_color(TEXT_FAINT, ACCENT_HI, s), 0.25), Stroke::new(1.5_f32, theme::lerp_color(TEXT_DIM, ACCENT_HI, s)), StrokeKind::Inside);
    p.text(pos2(r.center().x, r.top() + 74.0), Align2::CENTER_CENTER, label, theme::bold_font(15.0), theme::lerp_color(TEXT, Color32::WHITE, s));
    p.text(pos2(r.center().x, r.top() + 94.0), Align2::CENTER_CENTER, hint, theme::font(10.5), TEXT_DIM);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Smooth progress bar with a moving sheen.
pub fn progress_bar(ui: &mut Ui, id: Id, progress: f32, running: bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 10.0), Sense::hover());
    let shown = ui.ctx().animate_value_with_time(id, progress.clamp(0.0, 1.0), 0.35);
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(5), BG);
    let fill = Rect::from_min_size(rect.min, vec2(rect.width() * shown, rect.height()));
    if fill.width() > 1.0 {
        p.rect_filled(fill, CornerRadius::same(5), ACCENT);
        if running {
            let time = ui.input(|i| i.time) as f32;
            let x = fill.left() + ((time * 0.6).fract() * (fill.width() + 80.0)) - 40.0;
            let sheen = Rect::from_center_size(pos2(x, fill.center().y), vec2(40.0, fill.height())).intersect(fill);
            if sheen.width() > 0.0 {
                p.rect_filled(sheen, CornerRadius::same(5), alpha(Color32::WHITE, 0.18));
            }
            ui.ctx().request_repaint();
        }
    }
}

pub fn section_label(ui: &mut Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(text.to_uppercase()).font(theme::bold_font(11.0)).color(TEXT_FAINT));
    ui.add_space(2.0);
}

/// A small coloured status dot that can pulse.
pub fn status_dot(ui: &mut Ui, color: Color32, pulse: bool) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
    let p = ui.painter();
    if pulse {
        let t = (ui.input(|i| i.time) as f32 * 1.6).fract();
        p.circle_filled(rect.center(), 4.0 + 6.0 * t, alpha(color, 0.35 * (1.0 - t)));
        ui.ctx().request_repaint();
    }
    p.circle_filled(rect.center(), 4.0, color);
}

/// Button with a leading icon. `primary` fills it with the accent color.
pub fn icon_text_button(ui: &mut Ui, icon: &str, text: &str, enabled: bool, primary: bool) -> Response {
    let label = format!("{icon}  {text}");
    button_impl(ui, &label, enabled, primary, None)
}

/// Icon button that stays highlighted while `on`.
pub fn toggle_icon(ui: &mut Ui, icon: &str, tooltip: &str, on: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::click());
    let ctx = ui.ctx();
    let h = ctx.animate_bool_with_time(resp.id.with("h"), resp.hovered(), 0.12);
    let s = theme::ease_bool(ctx, resp.id.with("on"), on, 0.15);
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(R_SM), theme::lerp_color(alpha(SURFACE_HI, h), alpha(ACCENT, 0.25), s));
    let color = theme::lerp_color(theme::lerp_color(TEXT_DIM, TEXT, h), ACCENT_HI, s);
    p.text(rect.center(), Align2::CENTER_CENTER, icon, theme::font(17.0), color);
    resp.on_hover_text(tooltip).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Segmented tab bar with a sliding highlight. Returns true when the selection changed.
pub fn tabs(ui: &mut Ui, id: Id, items: &[(&str, &str)], selected: &mut usize) -> bool {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(R_MD), BG);
    let n = items.len().max(1) as f32;
    let w = (rect.width() - 6.0) / n;
    let pos = ui.ctx().animate_value_with_time(id, *selected as f32, 0.2);
    let pill = Rect::from_min_size(pos2(rect.left() + 3.0 + pos * w, rect.top() + 3.0), vec2(w, rect.height() - 6.0));
    p.rect_filled(pill, CornerRadius::same(R_SM + 1), SURFACE_HI);
    let mut changed = false;
    for (i, (icon, label)) in items.iter().enumerate() {
        let r = Rect::from_min_size(pos2(rect.left() + 3.0 + i as f32 * w, rect.top() + 3.0), vec2(w, rect.height() - 6.0));
        let resp = ui.interact(r, id.with(i), Sense::click());
        let active = 1.0 - (pos - i as f32).abs().min(1.0);
        let h = ui.ctx().animate_bool_with_time(resp.id.with("h"), resp.hovered(), 0.12);
        let color = theme::lerp_color(theme::lerp_color(TEXT_FAINT, TEXT_DIM, h), Color32::WHITE, active);
        let icon_color = theme::lerp_color(color, ACCENT_HI, active);
        let galley = ui.painter().layout_no_wrap(label.to_string(), theme::bold_font(13.0), color);
        let total_w = 18.0 + galley.size().x;
        let x = r.center().x - total_w / 2.0;
        ui.painter().text(pos2(x + 7.0, r.center().y), Align2::CENTER_CENTER, *icon, theme::font(15.0), icon_color);
        ui.painter().galley(pos2(x + 18.0, r.center().y - galley.size().y / 2.0), galley, color);
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && *selected != i {
            *selected = i;
            changed = true;
        }
    }
    changed
}
