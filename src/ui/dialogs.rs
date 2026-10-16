//! Modal dialogs: new project / canvas settings, export, connect AI, unsaved changes.

use std::path::PathBuf;

use egui::{vec2, Align2, Color32, CornerRadius, Id, Margin, RichText, Stroke, Ui};

use super::theme::{self, icons, *};
use super::widgets::*;
use crate::integrations::{self, Client};
use crate::mcp;
use crate::project::{fmt_time, Canvas, ExportStatus, Quality, ASPECTS, FRAME_RATES, RESOLUTIONS};
use crate::render;

/// Draws an animated modal (backdrop fade + panel rise). Returns false once fully hidden.
pub fn modal(ctx: &egui::Context, key: &str, open: bool, width: f32, add: impl FnOnce(&mut Ui)) -> bool {
    let t = theme::ease_bool(ctx, Id::new(("modal", key)), open, 0.2);
    if t <= 0.001 && !open {
        return false;
    }
    let screen = ctx.screen_rect();
    egui::Area::new(Id::new(("modal-bg", key)))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .interactable(true)
        .show(ctx, |ui| {
            let (_, _resp) = ui.allocate_exact_size(screen.size(), egui::Sense::click_and_drag());
            ui.painter().rect_filled(screen, CornerRadius::ZERO, Color32::from_black_alpha((160.0 * t) as u8));
        });
    let area = egui::Area::new(Id::new(("modal", key)))
        .order(egui::Order::Foreground)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, (1.0 - t) * 24.0))
        .interactable(open)
        .show(ctx, |ui| {
            ui.set_opacity(t);
            let frame = egui::Frame::NONE
                .fill(SURFACE)
                .stroke(Stroke::new(1.0_f32, BORDER))
                .corner_radius(CornerRadius::same(R_LG + 2))
                .inner_margin(Margin::same(24))
                .shadow(egui::epaint::Shadow { offset: [0, 12], blur: 40, spread: 0, color: Color32::from_black_alpha((140.0 * t) as u8) });
            frame.show(ui, |ui| {
                ui.set_width(width);
                add(ui);
            });
        });
    ctx.move_to_top(area.response.layer_id);
    true
}

fn header(ui: &mut Ui, title: &str, subtitle: &str) {
    ui.label(RichText::new(title).font(theme::bold_font(22.0)).color(Color32::WHITE));
    ui.add_space(2.0);
    ui.label(RichText::new(subtitle).color(TEXT_DIM));
    ui.add_space(14.0);
}

// ---------------------------------------------------------------------------
// Canvas form (new project, project settings and export share it)
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct CanvasForm {
    pub aspect: (u32, u32),
    pub short: u32,
    pub fps: u32,
}

impl CanvasForm {
    pub fn from_canvas(c: &Canvas) -> CanvasForm {
        let label = c.aspect_label();
        let aspect = ASPECTS.iter().find(|(n, ..)| *n == label).map(|(_, a, b, _)| (*a, *b)).unwrap_or((c.width, c.height));
        CanvasForm { aspect, short: c.width.min(c.height), fps: c.fps }
    }

    pub fn canvas(&self) -> Canvas {
        Canvas::from_preset(self.aspect.0, self.aspect.1, self.short, self.fps)
    }
}

pub fn canvas_form_ui(ui: &mut Ui, form: &mut CanvasForm) {
    section_label(ui, "Aspect ratio");
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(10.0, 10.0);
        for (name, aw, ah, hint) in ASPECTS {
            if aspect_card(ui, name, hint, *aw, *ah, form.aspect == (*aw, *ah)).clicked() {
                form.aspect = (*aw, *ah);
            }
        }
    });
    ui.add_space(10.0);
    section_label(ui, "Resolution");
    ui.horizontal_wrapped(|ui| {
        for (name, short) in RESOLUTIONS {
            let c = Canvas::from_preset(form.aspect.0, form.aspect.1, *short, form.fps);
            if chip(ui, name, Some(&format!("{}×{}", c.width, c.height)), form.short == *short).clicked() {
                form.short = *short;
            }
        }
    });
    ui.add_space(10.0);
    section_label(ui, "Frame rate");
    ui.horizontal(|ui| {
        for fps in FRAME_RATES {
            if chip(ui, &format!("{fps} fps"), None, form.fps == *fps).clicked() {
                form.fps = *fps;
            }
        }
    });
}

pub enum CanvasDialogResult {
    None,
    Create(String, Canvas),
    Apply(Canvas),
    Open,
    OpenRecent(PathBuf),
    Cancel,
}

/// New project (`is_new`) or project settings.
pub fn canvas_dialog(ui: &mut Ui, form: &mut CanvasForm, name: &mut String, is_new: bool, can_cancel: bool, recent: &[PathBuf]) -> CanvasDialogResult {
    let mut result = CanvasDialogResult::None;
    if is_new {
        header(ui, "New project", "Pick the shape and quality of your video. You can change this any time.");
        if !recent.is_empty() {
            section_label(ui, "Continue where you left off");
            for path in recent.iter().take(4) {
                let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let dir = path.parent().map(|d| d.display().to_string()).unwrap_or_default();
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), egui::Sense::click());
                let h = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
                let p = ui.painter();
                p.rect_filled(r, CornerRadius::same(R_MD), theme::lerp_color(BG, SURFACE_HI, h));
                p.text(r.left_center() + vec2(18.0, 0.0), Align2::CENTER_CENTER, icons::FILE_VIDEO, theme::font(17.0), ACCENT_HI);
                p.text(r.left_top() + vec2(36.0, 5.0), Align2::LEFT_TOP, &name, theme::bold_font(13.5), TEXT);
                p.with_clip_rect(r.shrink(2.0)).text(r.left_top() + vec2(36.0, 22.0), Align2::LEFT_TOP, &dir, theme::font(11.0), TEXT_FAINT);
                p.text(r.right_center() - vec2(14.0, 0.0), Align2::RIGHT_CENTER, icons::CARET_RIGHT, theme::font(14.0), alpha(TEXT_DIM, h));
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    result = CanvasDialogResult::OpenRecent(path.clone());
                }
                ui.add_space(4.0);
            }
            ui.add_space(8.0);
        }
        section_label(ui, "Project name");
        ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY).margin(vec2(10.0, 8.0)));
        ui.add_space(10.0);
    } else {
        header(ui, "Project settings", "Clips re-fit automatically. This change can be undone.");
    }
    canvas_form_ui(ui, form);
    ui.add_space(18.0);
    let c = form.canvas();
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{} × {} · {} fps", c.width, c.height, c.fps)).color(TEXT_DIM));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if is_new {
                if primary_button(ui, "Create project", true).clicked() {
                    let n = if name.trim().is_empty() { "Untitled".to_string() } else { name.trim().to_string() };
                    result = CanvasDialogResult::Create(n, c);
                }
                if secondary_button(ui, "Open existing…", true).clicked() {
                    result = CanvasDialogResult::Open;
                }
                if can_cancel && secondary_button(ui, "Cancel", true).clicked() {
                    result = CanvasDialogResult::Cancel;
                }
            } else {
                if primary_button(ui, "Apply", true).clicked() {
                    result = CanvasDialogResult::Apply(c);
                }
                if secondary_button(ui, "Cancel", true).clicked() {
                    result = CanvasDialogResult::Cancel;
                }
            }
        });
    });
    result
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

pub struct ExportForm {
    pub canvas: CanvasForm,
    pub quality: Quality,
    pub path: String,
}

pub enum ExportAction {
    None,
    Start(Canvas, Quality, PathBuf),
    Cancel,
    Close,
    ShowInFolder(PathBuf),
    Play(PathBuf),
    Reset,
}

pub fn default_export_path(name: &str) -> String {
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default();
    let dir = [home.join("Videos"), home.join("Desktop"), home.clone()].into_iter().find(|d| d.is_dir()).unwrap_or(home);
    let safe: String = name.chars().map(|c| if r#"<>:"/\|?*"#.contains(c) { '_' } else { c }).collect();
    let safe = if safe.trim().is_empty() { "video".to_string() } else { safe.trim().to_string() };
    let mut candidate = dir.join(format!("{safe}.mp4"));
    let mut n = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{safe} ({n}).mp4"));
        n += 1;
    }
    candidate.to_string_lossy().into_owned()
}

pub fn export_dialog(ui: &mut Ui, form: &mut ExportForm, status: &ExportStatus, duration: f64) -> ExportAction {
    let mut action = ExportAction::None;
    match status {
        ExportStatus::Running { progress, eta_secs, path } => {
            header(ui, "Exporting…", &format!("{}", path.file_name().unwrap_or_default().to_string_lossy()));
            progress_bar(ui, Id::new("export-progress"), *progress, true);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{:.0}%", progress * 100.0)).font(theme::bold_font(15.0)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let eta = match eta_secs {
                        Some(e) => format!("about {} left", human_secs(*e)),
                        None => "estimating time…".into(),
                    };
                    ui.label(RichText::new(eta).color(TEXT_DIM));
                });
            });
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("You can keep editing — the export uses a snapshot.").color(TEXT_FAINT).small());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if secondary_button(ui, "Keep editing", true).clicked() {
                        action = ExportAction::Close;
                    }
                    if secondary_button(ui, "Cancel export", true).clicked() {
                        action = ExportAction::Cancel;
                    }
                });
            });
        }
        ExportStatus::Done { path, secs } => {
            ui.vertical_centered(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(64.0, 64.0), egui::Sense::hover());
                let t = theme::ease_bool(ui.ctx(), Id::new("export-done-anim"), true, 0.45);
                let p = ui.painter();
                p.circle_filled(rect.center(), 30.0 * (0.6 + 0.4 * t), alpha(SUCCESS, 0.2 * t));
                p.circle_filled(rect.center(), 22.0 * (0.5 + 0.5 * t), SUCCESS);
                let c = rect.center();
                let pts = [c + vec2(-9.0, 0.0), c + vec2(-3.0, 7.0), c + vec2(10.0, -7.0)];
                let shown = ((t - 0.3) / 0.7).clamp(0.0, 1.0);
                if shown > 0.0 {
                    let seg1 = (shown * 2.0).min(1.0);
                    let seg2 = ((shown - 0.5) * 2.0).clamp(0.0, 1.0);
                    p.line_segment([pts[0], pts[0] + (pts[1] - pts[0]) * seg1], Stroke::new(3.5_f32, Color32::WHITE));
                    if seg2 > 0.0 {
                        p.line_segment([pts[1], pts[1] + (pts[2] - pts[1]) * seg2], Stroke::new(3.5_f32, Color32::WHITE));
                    }
                }
                ui.add_space(8.0);
                ui.label(RichText::new("Export complete").font(theme::bold_font(20.0)).color(Color32::WHITE));
                ui.label(RichText::new(format!("{} · took {}", path.display(), human_secs(*secs))).color(TEXT_DIM));
            });
            ui.add_space(18.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if primary_button(ui, "Done", true).clicked() {
                        action = ExportAction::Reset;
                    }
                    if secondary_button(ui, "Play video", true).clicked() {
                        action = ExportAction::Play(path.clone());
                    }
                    if secondary_button(ui, "Show in folder", true).clicked() {
                        action = ExportAction::ShowInFolder(path.clone());
                    }
                });
            });
        }
        ExportStatus::Failed { error } => {
            header(ui, "Export failed", "Nothing was written. Your project is unchanged.");
            egui::Frame::NONE.fill(alpha(DANGER, 0.12)).corner_radius(CornerRadius::same(R_SM)).inner_margin(Margin::same(10)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(error).color(Color32::from_rgb(255, 170, 170)));
            });
            ui.add_space(14.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if primary_button(ui, "Try again", true).clicked() {
                    action = ExportAction::Reset;
                }
                if secondary_button(ui, "Close", true).clicked() {
                    action = ExportAction::Close;
                }
            });
        }
        ExportStatus::Idle | ExportStatus::Cancelled => {
            header(ui, "Export video", &format!("MP4 (H.264 + AAC) · length {}", fmt_time(duration)));
            canvas_form_ui(ui, &mut form.canvas);
            ui.add_space(10.0);
            section_label(ui, "Quality");
            ui.horizontal(|ui| {
                for q in [Quality::High, Quality::Standard, Quality::Small] {
                    if chip(ui, q.label(), None, form.quality == q).clicked() {
                        form.quality = q;
                    }
                }
            });
            ui.add_space(10.0);
            section_label(ui, "Save to");
            ui.horizontal(|ui| {
                let w = ui.available_width() - 96.0;
                ui.add(egui::TextEdit::singleline(&mut form.path).desired_width(w).margin(vec2(10.0, 8.0)));
                if secondary_button(ui, "Browse…", true).clicked() {
                    let current = PathBuf::from(&form.path);
                    let mut dlg = rfd::FileDialog::new().add_filter("MP4 video", &["mp4"]).set_title("Export video");
                    if let Some(dir) = current.parent().filter(|d| d.is_dir()) {
                        dlg = dlg.set_directory(dir);
                    }
                    if let Some(name) = current.file_name() {
                        dlg = dlg.set_file_name(name.to_string_lossy());
                    }
                    if let Some(p) = dlg.save_file() {
                        form.path = p.with_extension("mp4").to_string_lossy().into_owned();
                    }
                }
            });
            ui.add_space(16.0);
            let canvas = form.canvas.canvas();
            let size = render::estimate_size(&canvas, form.quality, duration);
            let path = PathBuf::from(form.path.trim());
            let problem = if duration <= 0.0 {
                Some("Add a clip to the timeline first.".to_string())
            } else if form.path.trim().is_empty() {
                Some("Choose where to save the video.".to_string())
            } else if !path.is_absolute() {
                Some("Use a full path, like C:\\Users\\you\\Videos\\video.mp4".to_string())
            } else if !path.parent().is_some_and(|d| d.is_dir()) {
                Some("That folder does not exist.".to_string())
            } else {
                None
            };
            ui.horizontal(|ui| {
                match &problem {
                    Some(p) => ui.label(RichText::new(p).color(WARNING)),
                    None => ui.label(RichText::new(format!("{}×{} · {} fps · about {}", canvas.width, canvas.height, canvas.fps, human_bytes(size))).color(TEXT_DIM)),
                };
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if primary_button(ui, "Export", problem.is_none()).clicked() {
                        let path = if path.extension().is_none() { path.with_extension("mp4") } else { path.clone() };
                        action = ExportAction::Start(canvas, form.quality, path);
                    }
                    if secondary_button(ui, "Cancel", true).clicked() {
                        action = ExportAction::Close;
                    }
                });
            });
        }
    }
    action
}

pub fn human_secs(s: f64) -> String {
    let s = s.max(0.0).round() as u64;
    if s < 60 {
        format!("{s}s")
    } else {
        format!("{}m {:02}s", s / 60, s % 60)
    }
}

pub fn human_bytes(b: f64) -> String {
    if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.0} MB", b / 1e6)
    } else {
        format!("{:.0} KB", (b / 1e3).max(1.0))
    }
}

// ---------------------------------------------------------------------------
// Connect AI
// ---------------------------------------------------------------------------

pub struct ConnectState {
    pub configured: Vec<(Client, bool)>,
    pub message: Option<(bool, String)>,
}

impl ConnectState {
    pub fn new() -> ConnectState {
        ConnectState { configured: Client::ALL.iter().map(|c| (*c, integrations::is_configured(*c))).collect(), message: None }
    }
}

pub fn connect_dialog(ui: &mut Ui, state: &mut ConnectState, server_error: Option<&str>) -> bool {
    let mut close = false;
    header(ui, "Connect your AI", "Let Claude edit this project. You see every change live and can undo anything.");
    if let Some(err) = server_error {
        egui::Frame::NONE.fill(alpha(DANGER, 0.12)).corner_radius(CornerRadius::same(R_SM)).inner_margin(Margin::same(10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(format!("The AI connection could not start: {err}")).color(Color32::from_rgb(255, 170, 170)));
        });
        ui.add_space(10.0);
    }
    for (client, configured) in state.configured.iter_mut() {
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(client.label()).font(theme::bold_font(15.0)));
                    let (txt, col) = if *configured { ("Connected", SUCCESS) } else { ("Not connected", TEXT_FAINT) };
                    ui.label(RichText::new(txt).color(col).small());
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if *configured { "Reconnect" } else { "Connect" };
                    let resp = if *configured { secondary_button(ui, label, true) } else { primary_button(ui, label, true) };
                    if resp.clicked() {
                        match integrations::connect(*client) {
                            Ok(msg) => {
                                *configured = true;
                                state.message = Some((true, msg));
                            }
                            Err(e) => state.message = Some((false, format!("{e:#}"))),
                        }
                    }
                });
            });
        });
        ui.add_space(8.0);
    }
    if let Some((ok, msg)) = &state.message {
        let col = if *ok { SUCCESS } else { WARNING };
        egui::Frame::NONE.fill(alpha(col, 0.1)).corner_radius(CornerRadius::same(R_SM)).inner_margin(Margin::same(10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(msg).color(col));
        });
        ui.add_space(8.0);
    }
    section_label(ui, "Manual setup");
    ui.horizontal(|ui| {
        ui.label(RichText::new(mcp::url()).monospace().color(TEXT_DIM));
        if ui.small_button("Copy URL").clicked() {
            ui.ctx().copy_text(mcp::url());
        }
        if ui.small_button("Copy Claude Code command").clicked() {
            ui.ctx().copy_text(integrations::claude_code_command());
        }
    });
    ui.add_space(14.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if primary_button(ui, "Done", true).clicked() {
            close = true;
        }
    });
    close
}

// ---------------------------------------------------------------------------
// Unsaved changes
// ---------------------------------------------------------------------------

pub enum UnsavedChoice {
    None,
    Save,
    Discard,
    Cancel,
}

pub fn unsaved_dialog(ui: &mut Ui, what: &str) -> UnsavedChoice {
    let mut c = UnsavedChoice::None;
    header(ui, "Save changes?", &format!("Your project has unsaved changes. Save before you {what}?"));
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if primary_button(ui, "Save", true).clicked() {
            c = UnsavedChoice::Save;
        }
        if secondary_button(ui, "Don't save", true).clicked() {
            c = UnsavedChoice::Discard;
        }
        if secondary_button(ui, "Cancel", true).clicked() {
            c = UnsavedChoice::Cancel;
        }
    });
    c
}
