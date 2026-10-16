//! Colors, fonts, widget styling and small animation helpers.

use egui::{Color32, CornerRadius, FontFamily, FontId, Id, Margin, Pos2, Rect, Stroke, Vec2};

pub const BG: Color32 = Color32::from_rgb(9, 12, 18);
pub const PANEL: Color32 = Color32::from_rgb(16, 21, 31);
pub const SURFACE: Color32 = Color32::from_rgb(23, 30, 44);
pub const SURFACE_HI: Color32 = Color32::from_rgb(32, 42, 61);
pub const BORDER: Color32 = Color32::from_rgb(38, 49, 69);
pub const TEXT: Color32 = Color32::from_rgb(229, 235, 244);
pub const TEXT_DIM: Color32 = Color32::from_rgb(141, 155, 178);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(90, 103, 124);
pub const ACCENT: Color32 = Color32::from_rgb(47, 117, 240);
pub const ACCENT_HI: Color32 = Color32::from_rgb(96, 160, 255);
pub const AI_GLOW: Color32 = Color32::from_rgb(110, 205, 255);
pub const SUCCESS: Color32 = Color32::from_rgb(52, 199, 120);
pub const DANGER: Color32 = Color32::from_rgb(239, 80, 80);
pub const WARNING: Color32 = Color32::from_rgb(245, 166, 35);
pub const CLIP_VIDEO: Color32 = Color32::from_rgb(52, 104, 200);
pub const CLIP_MUSIC: Color32 = Color32::from_rgb(20, 160, 140);
pub const CLIP_TEXT: Color32 = Color32::from_rgb(214, 138, 40);
pub const PLAYHEAD: Color32 = Color32::from_rgb(255, 255, 255);

pub const R_SM: u8 = 6;
pub const R_MD: u8 = 10;
pub const R_LG: u8 = 14;

/// Standard UI animation length.
pub const ANIM: f32 = 0.16;

pub fn bold() -> FontFamily {
    FontFamily::Name("bold".into())
}

pub fn font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn bold_font(size: f32) -> FontId {
    FontId::new(size, bold())
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    // Phosphor icon font (used through `icons::*`), shared by every family.
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
    let load = |name: &str| std::fs::read(std::path::Path::new(&windir).join("Fonts").join(name)).ok();
    let mut bold_family = fonts.families[&FontFamily::Proportional].clone();
    if let Some(data) = load("segoeui.ttf") {
        fonts.font_data.insert("segoe".into(), egui::FontData::from_owned(data).into());
        fonts.families.get_mut(&FontFamily::Proportional).unwrap().insert(0, "segoe".into());
    }
    if let Some(data) = load("seguisb.ttf").or_else(|| load("segoeuib.ttf")) {
        fonts.font_data.insert("segoe-bold".into(), egui::FontData::from_owned(data).into());
        bold_family.insert(0, "segoe-bold".into());
    }
    // Symbols used by icons (play, scissors, ...).
    if let Some(data) = load("seguisym.ttf") {
        fonts.font_data.insert("segoe-symbol".into(), egui::FontData::from_owned(data).into());
        fonts.families.get_mut(&FontFamily::Proportional).unwrap().push("segoe-symbol".into());
        bold_family.push("segoe-symbol".into());
    }
    fonts.families.insert(bold(), bold_family);
    ctx.set_fonts(fonts);

    ctx.style_mut(|s| {
        s.animation_time = ANIM;
        s.spacing.item_spacing = Vec2::new(8.0, 6.0);
        s.spacing.button_padding = Vec2::new(12.0, 6.0);
        s.spacing.interact_size.y = 30.0;
        s.spacing.slider_width = 160.0;
        s.text_styles.insert(egui::TextStyle::Body, font(14.0));
        s.text_styles.insert(egui::TextStyle::Button, font(14.0));
        s.text_styles.insert(egui::TextStyle::Small, font(12.0));
        s.text_styles.insert(egui::TextStyle::Heading, bold_font(20.0));
        s.text_styles.insert(egui::TextStyle::Monospace, FontId::monospace(13.0));
        let v = &mut s.visuals;
        *v = egui::Visuals::dark();
        v.panel_fill = PANEL;
        v.window_fill = SURFACE;
        v.extreme_bg_color = BG;
        v.faint_bg_color = SURFACE;
        v.override_text_color = Some(TEXT);
        v.window_corner_radius = CornerRadius::same(R_LG);
        v.menu_corner_radius = CornerRadius::same(R_MD);
        v.window_stroke = Stroke::new(1.0_f32, BORDER);
        v.selection.bg_fill = ACCENT.gamma_multiply(0.55);
        v.selection.stroke = Stroke::new(1.0_f32, ACCENT_HI);
        v.hyperlink_color = ACCENT_HI;
        v.slider_trailing_fill = true;
        v.handle_shape = egui::style::HandleShape::Circle;
        for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open, &mut v.widgets.noninteractive] {
            w.corner_radius = CornerRadius::same(R_SM);
        }
        v.widgets.noninteractive.bg_fill = PANEL;
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
        v.widgets.inactive.bg_fill = SURFACE;
        v.widgets.inactive.weak_bg_fill = SURFACE;
        v.widgets.inactive.bg_stroke = Stroke::NONE;
        v.widgets.hovered.bg_fill = SURFACE_HI;
        v.widgets.hovered.weak_bg_fill = SURFACE_HI;
        v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, BORDER);
        v.widgets.hovered.expansion = 0.0;
        v.widgets.active.bg_fill = ACCENT.gamma_multiply(0.7);
        v.widgets.active.weak_bg_fill = SURFACE_HI;
        v.widgets.active.expansion = 0.0;
        v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
        v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT);
        v.widgets.active.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
    });
}

/// 0..1 eased value that follows `on`.
pub fn ease_bool(ctx: &egui::Context, id: Id, on: bool, secs: f32) -> f32 {
    let t = ctx.animate_bool_with_time(id, on, secs);
    ease_out(t)
}

pub fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// Seconds since `at`, as an eased 0..1 over `dur`. Requests repaints while running.
pub fn since(ctx: &egui::Context, at: std::time::Instant, dur: f32) -> f32 {
    let t = at.elapsed().as_secs_f32() / dur;
    if t < 1.0 {
        ctx.request_repaint();
    }
    ease_out(t)
}

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

pub fn alpha(c: Color32, a: f32) -> Color32 {
    c.gamma_multiply(a.clamp(0.0, 1.0))
}

pub fn card_frame() -> egui::Frame {
    egui::Frame::NONE.fill(SURFACE).corner_radius(CornerRadius::same(R_MD)).inner_margin(Margin::same(12))
}

/// A soft drop shadow behind `rect`.
pub fn shadow(painter: &egui::Painter, rect: Rect, radius: f32, strength: f32) {
    for i in 1..=6 {
        let grow = i as f32 * 2.5;
        painter.rect_filled(
            rect.expand(grow).translate(Vec2::new(0.0, grow * 0.6)),
            CornerRadius::same((radius + grow).min(255.0) as u8),
            Color32::from_black_alpha((strength * 22.0 / i as f32) as u8),
        );
    }
}

/// Small spinner made of a rotating arc.
pub fn spinner(painter: &egui::Painter, center: Pos2, r: f32, color: Color32, time: f64) {
    let start = time as f32 * 5.0;
    let pts: Vec<Pos2> = (0..=24)
        .map(|i| {
            let a = start + i as f32 / 24.0 * std::f32::consts::PI * 1.4;
            center + Vec2::new(a.cos(), a.sin()) * r
        })
        .collect();
    painter.add(egui::Shape::line(pts, Stroke::new(2.0_f32, color)));
}

/// Icon glyphs (Phosphor, regular weight).
pub mod icons {
    pub use egui_phosphor::regular::*;
}

/// Panel frame for the floating, rounded side panels.
pub fn floating_panel(left: i8, right: i8, top: i8, bottom: i8) -> egui::Frame {
    egui::Frame::NONE
        .fill(PANEL)
        .corner_radius(CornerRadius::same(R_LG))
        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(26, 33, 47)))
        .inner_margin(Margin::same(14))
        .outer_margin(Margin { left, right, top, bottom })
}
