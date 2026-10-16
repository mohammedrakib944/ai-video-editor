//! Right panel "Assistant" tab: chat with Claude, with every tool call shown live.

use std::time::Instant;

use egui::text::{LayoutJob, TextFormat};
use egui::{pos2, vec2, Align2, Color32, CornerRadius, Id as EId, Key, Modifiers, Rect, RichText, Sense, Stroke, StrokeKind, TextureHandle, Ui};

use super::theme::{self, icons, *};
use super::{widgets, App, ToastKind};
use crate::chat::{describe_tool, ChatEvent};
use crate::project::{fmt_time, Track};

pub enum ToolStatus {
    Running,
    Done,
    Failed(String),
}

pub enum ChatItem {
    User { text: String, at: Instant },
    Assistant { text: String, at: Instant },
    Tool { id: String, label: String, status: ToolStatus, at: Instant, image: Option<TextureHandle> },
    Error { text: String, at: Instant },
}

impl ChatItem {
    fn at(&self) -> Instant {
        match self {
            ChatItem::User { at, .. } | ChatItem::Assistant { at, .. } | ChatItem::Tool { at, .. } | ChatItem::Error { at, .. } => *at,
        }
    }
}

const SUGGESTIONS: &[(&str, &str)] = &[
    (icons::SCISSORS, "Cut the first 3 seconds"),
    (icons::TEXT_T, "Add a title “My Trip” at the start"),
    (icons::FRAME_CORNERS, "Make it vertical for Reels and fill the frame"),
    (icons::GAUGE, "Speed up the second clip 2×"),
    (icons::CIRCLE_HALF, "Add a fade in and fade out"),
    (icons::EXPORT, "Export a 1080p MP4 to my Videos folder"),
];

impl App {
    /// Applies streamed events from the assistant to the chat timeline.
    pub(super) fn poll_chat(&mut self, ctx: &egui::Context) {
        for ev in self.chat.poll() {
            let now = Instant::now();
            match ev {
                ChatEvent::TextDelta(t) => match self.chat_items.last_mut() {
                    Some(ChatItem::Assistant { text, .. }) if self.chat_streaming => text.push_str(&t),
                    _ => {
                        self.chat_items.push(ChatItem::Assistant { text: t, at: now });
                        self.chat_streaming = true;
                    }
                },
                ChatEvent::Text(t) => {
                    self.chat_items.push(ChatItem::Assistant { text: t, at: now });
                    self.chat_streaming = false;
                }
                ChatEvent::ToolStart { id, name, input } => {
                    self.chat_streaming = false;
                    if name == "ToolSearch" {
                        continue;
                    }
                    self.chat_items.push(ChatItem::Tool { id, label: describe_tool(&name, &input), status: ToolStatus::Running, at: now, image: None });
                }
                ChatEvent::ToolDone { id, ok, summary, image } => {
                    for item in self.chat_items.iter_mut().rev() {
                        if let ChatItem::Tool { id: tid, status, image: img, .. } = item {
                            if *tid == id {
                                *status = if ok { ToolStatus::Done } else { ToolStatus::Failed(summary.trim_start_matches("Error: ").to_string()) };
                                if let Some(jpeg) = &image {
                                    if let Ok(dec) = image::load_from_memory(jpeg) {
                                        let rgba = dec.to_rgba8();
                                        let size = [rgba.width() as usize, rgba.height() as usize];
                                        let ci = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                                        *img = Some(ctx.load_texture(format!("chat-frame-{id}"), ci, egui::TextureOptions::LINEAR));
                                    }
                                }
                                break;
                            }
                        }
                    }
                }
                ChatEvent::TurnDone { error } => {
                    self.chat_streaming = false;
                    for item in self.chat_items.iter_mut() {
                        if let ChatItem::Tool { status: s @ ToolStatus::Running, .. } = item {
                            *s = ToolStatus::Failed("Interrupted".into());
                        }
                    }
                    if let Some(e) = error {
                        self.chat_items.push(ChatItem::Error { text: e, at: now });
                    }
                }
            }
        }
    }

    fn editor_context(&self) -> String {
        let p = &self.snapshot;
        let mut s = format!(
            "playhead {:.2}s of {:.2}s; canvas {}x{} {}",
            self.playhead,
            p.duration(),
            p.canvas.width,
            p.canvas.height,
            p.canvas.summary()
        );
        if let Some(id) = self.selected {
            match p.track_of(id) {
                Some(Track::Main) => {
                    let i = p.main_index(id).unwrap();
                    let start = p.main_starts()[i];
                    let name = p.asset(p.main[i].asset_id).map(|a| a.name.as_str()).unwrap_or("");
                    s += &format!("; selected main-track clip {id} \"{name}\" at {:.2}s–{:.2}s", start, start + p.main[i].duration());
                }
                Some(Track::Music) => s += &format!("; selected music clip {id}"),
                Some(Track::Text) => s += &format!("; selected text clip {id}"),
                None => {}
            }
        }
        s
    }

    pub(super) fn send_chat(&mut self, text: String) {
        let text = text.trim().to_string();
        if text.is_empty() || self.chat.busy {
            return;
        }
        let cwd = {
            let ed = self.shared.editor.lock();
            ed.path.as_ref().and_then(|p| p.parent().map(|d| d.to_path_buf()))
        }
        .or_else(|| Some(crate::platform::home_dir().join("Videos")).filter(|d| d.is_dir()))
        .or_else(|| Some(crate::platform::home_dir()).filter(|d| d.is_dir()))
        .unwrap_or_else(std::env::temp_dir);
        let context = self.editor_context();
        self.chat_items.push(ChatItem::User { text: text.clone(), at: Instant::now() });
        self.chat_streaming = false;
        if let Err(e) = self.chat.send(&text, &context, &cwd) {
            self.chat_items.push(ChatItem::Error { text: e.to_string(), at: Instant::now() });
        }
    }

    pub(super) fn chat_ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        // Header.
        ui.horizontal(|ui| {
            let busy = self.chat.busy;
            let (r, _) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::hover());
            let glow = if busy { 0.5 + 0.5 * ((ui.input(|i| i.time) as f32) * 3.0).sin().abs() } else { 0.0 };
            ui.painter().circle_filled(r.center(), 15.0, alpha(ACCENT, 0.18 + 0.2 * glow));
            ui.painter().text(r.center(), Align2::CENTER_CENTER, icons::SPARKLE, theme::font(17.0), ACCENT_HI);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(RichText::new("Claude").font(theme::bold_font(15.0)).color(Color32::WHITE));
                let (txt, col) = if !self.chat.available() {
                    ("Not set up", WARNING)
                } else if busy {
                    ("Working on your video…", AI_GLOW)
                } else {
                    ("Ready to edit", TEXT_FAINT)
                };
                ui.label(RichText::new(txt).color(col).size(11.5));
            });
            if busy {
                ctx.request_repaint();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::icon_button(ui, icons::NOTE_PENCIL, "New chat", !self.chat_items.is_empty()).clicked() {
                    self.chat.reset();
                    self.chat_items.clear();
                    self.chat_streaming = false;
                }
            });
        });
        ui.add_space(8.0);

        if !self.chat.available() {
            self.chat_setup_ui(ui);
            return;
        }

        // Composer is pinned to the bottom; messages fill the rest.
        let composer_h = 112.0;
        let list_h = (ui.available_height() - composer_h).max(80.0);
        let list_rect = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), list_h));
        let mut list_ui = ui.new_child(egui::UiBuilder::new().max_rect(list_rect));
        let mut clicked_suggestion = None;
        egui::ScrollArea::vertical().id_salt("chat-scroll").stick_to_bottom(true).auto_shrink([false, false]).show(&mut list_ui, |ui| {
            ui.set_width(ui.available_width());
            if self.chat_items.is_empty() {
                clicked_suggestion = empty_state(ui);
            }
            let n = self.chat_items.len();
            for i in 0..n {
                let appear = theme::since(&ctx, self.chat_items[i].at(), 0.3);
                ui.add_space(6.0 * appear);
                ui.scope(|ui| {
                    ui.set_opacity(appear);
                    match &self.chat_items[i] {
                        ChatItem::User { text, .. } => user_bubble(ui, text, appear),
                        ChatItem::Assistant { text, .. } => assistant_text(ui, text),
                        ChatItem::Tool { label, status, image, at, .. } => tool_row(ui, label, status, image.as_ref(), *at),
                        ChatItem::Error { text, .. } => error_row(ui, text),
                    }
                });
            }
            let streaming_text = self.chat_streaming && matches!(self.chat_items.last(), Some(ChatItem::Assistant { .. }));
            let tool_running = matches!(self.chat_items.last(), Some(ChatItem::Tool { status: ToolStatus::Running, .. }));
            if self.chat.busy && !streaming_text && !tool_running {
                ui.add_space(6.0);
                typing_dots(ui);
            }
            ui.add_space(4.0);
        });
        ui.advance_cursor_after_rect(list_rect);
        if let Some(s) = clicked_suggestion {
            self.send_chat(s);
        }

        // Context chip: what "here" and "this clip" mean.
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let mut chip = format!("{}  {}", icons::CROSSHAIR, fmt_time(self.playhead));
            if let Some(id) = self.selected {
                let name = match self.snapshot.track_of(id) {
                    Some(Track::Main) => self.snapshot.main_index(id).and_then(|i| self.snapshot.asset(self.snapshot.main[i].asset_id)).map(|a| a.name.clone()),
                    Some(Track::Music) => Some("music clip".into()),
                    Some(Track::Text) => Some("text clip".into()),
                    None => None,
                };
                if let Some(n) = name {
                    chip += &format!("   {}  {}", icons::CURSOR_CLICK, n);
                }
            }
            ui.label(RichText::new(chip).color(TEXT_FAINT).size(11.5)).on_hover_text("Claude knows where the playhead is and which clip is selected, so you can say “here” or “this clip”.");
        });
        ui.add_space(4.0);

        // Composer.
        let input_id = EId::new("chat-input");
        let has_focus = ctx.memory(|m| m.has_focus(input_id));
        // `consume_key` ignores Shift, so check it first: Shift+Enter must insert a newline.
        let shift = ctx.input(|i| i.modifiers.shift);
        let send_now = has_focus && !shift && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
        let frame_stroke = if has_focus { Stroke::new(1.5_f32, alpha(ACCENT_HI, 0.8)) } else { Stroke::new(1.0_f32, BORDER) };
        egui::Frame::NONE.fill(SURFACE).stroke(frame_stroke).corner_radius(CornerRadius::same(R_LG)).inner_margin(egui::Margin { left: 12, right: 8, top: 8, bottom: 8 }).show(ui, |ui| {
            ui.horizontal(|ui| {
                let w = ui.available_width() - 44.0;
                let edit = egui::TextEdit::multiline(&mut self.chat_input)
                    .id(input_id)
                    .frame(false)
                    .desired_rows(2)
                    .desired_width(w)
                    .hint_text(RichText::new("Ask Claude to edit your video…").color(TEXT_FAINT))
                    .font(theme::font(14.0));
                ui.add(edit);
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                    let (r, resp) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::click());
                    let busy = self.chat.busy;
                    let enabled = busy || !self.chat_input.trim().is_empty();
                    let h = ctx.animate_bool_with_time(resp.id.with("h"), resp.hovered() && enabled, 0.12);
                    let fill = if busy { theme::lerp_color(SURFACE_HI, DANGER, 0.25 + 0.2 * h) } else if enabled { theme::lerp_color(ACCENT, ACCENT_HI, h) } else { SURFACE_HI };
                    ui.painter().circle_filled(r.center(), 17.0, fill);
                    let icon = if busy { icons::STOP } else { icons::ARROW_UP };
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, icon, theme::bold_font(17.0), if enabled { Color32::WHITE } else { TEXT_FAINT });
                    let resp = resp.on_hover_text(if busy { "Stop" } else { "Send (Enter)" });
                    if resp.clicked() && enabled {
                        if busy {
                            self.chat.stop();
                        } else {
                            let text = std::mem::take(&mut self.chat_input);
                            self.send_chat(text);
                        }
                    }
                });
            });
        });
        if send_now && !self.chat.busy && !self.chat_input.trim().is_empty() {
            let text = std::mem::take(&mut self.chat_input);
            self.send_chat(text);
            ctx.memory_mut(|m| m.request_focus(input_id));
        }
        ui.add_space(2.0);
        ui.label(RichText::new("Enter to send · Shift+Enter for a new line").color(TEXT_FAINT).size(10.5));
    }

    fn chat_setup_ui(&mut self, ui: &mut Ui) {
        theme::card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(format!("{}  Connect Claude Code", icons::PLUG)).font(theme::bold_font(15.0)));
            ui.add_space(6.0);
            ui.label(RichText::new("The assistant uses your own Claude Code, so there is nothing else to pay for or configure.").color(TEXT_DIM));
            ui.add_space(8.0);
            for (n, step) in ["Install Claude Code from claude.com/claude-code", "Open a terminal, run claude once and sign in", "Come back here and press Check again"].iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{}", n + 1)).font(theme::bold_font(13.0)).color(ACCENT_HI));
                    ui.label(RichText::new(*step).color(TEXT));
                });
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if widgets::icon_text_button(ui, icons::ARROWS_CLOCKWISE, "Check again", true, true).clicked() {
                    self.chat.recheck();
                    if !self.chat.available() {
                        self.toast(ToastKind::Info, "Claude Code still not found");
                    }
                }
                if widgets::secondary_button(ui, "Get Claude Code", true).clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab("https://claude.com/claude-code"));
                }
            });
        });
    }
}

fn empty_state(ui: &mut Ui) -> Option<String> {
    let mut picked = None;
    ui.add_space(18.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(icons::SPARKLE).size(34.0).color(ACCENT_HI));
        ui.add_space(4.0);
        ui.label(RichText::new("Edit by chatting").font(theme::bold_font(18.0)).color(Color32::WHITE));
        ui.label(RichText::new("Tell Claude what you want. You'll see every change happen live.").color(TEXT_DIM).size(12.5));
    });
    ui.add_space(14.0);
    for (icon, text) in SUGGESTIONS {
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
        let h = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
        let p = ui.painter();
        p.rect(r, CornerRadius::same(R_MD), theme::lerp_color(SURFACE, SURFACE_HI, h), Stroke::new(1.0_f32, theme::lerp_color(BORDER, alpha(ACCENT_HI, 0.6), h)), StrokeKind::Inside);
        p.text(pos2(r.left() + 16.0, r.center().y), Align2::CENTER_CENTER, *icon, theme::font(14.0), ACCENT_HI);
        p.text(pos2(r.left() + 32.0 + h * 2.0, r.center().y), Align2::LEFT_CENTER, *text, theme::font(13.0), theme::lerp_color(TEXT_DIM, TEXT, h));
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            picked = Some(text.to_string());
        }
        ui.add_space(4.0);
    }
    picked
}

fn user_bubble(ui: &mut Ui, text: &str, appear: f32) {
    let max_w = ui.available_width() * 0.85;
    let galley = ui.painter().layout(text.to_string(), theme::font(13.5), Color32::WHITE, max_w - 24.0);
    let size = galley.size() + vec2(24.0, 16.0);
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), size.y), Sense::hover());
    let r = Rect::from_min_size(pos2(row.right() - size.x + (1.0 - appear) * 12.0, row.top()), size);
    ui.painter().rect_filled(r, CornerRadius { nw: 14, ne: 14, sw: 14, se: 4 }, ACCENT);
    ui.painter().galley(r.min + vec2(12.0, 8.0), galley, Color32::WHITE);
}

fn assistant_text(ui: &mut Ui, text: &str) {
    ui.horizontal_top(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 11.0, alpha(ACCENT, 0.2));
        ui.painter().text(r.center(), Align2::CENTER_CENTER, icons::SPARKLE, theme::font(12.0), ACCENT_HI);
        let w = ui.available_width();
        let job = markdown_job(text, w);
        ui.add(egui::Label::new(job).wrap());
    });
}

fn tool_row(ui: &mut Ui, label: &str, status: &ToolStatus, image: Option<&TextureHandle>, at: Instant) {
    let ctx = ui.ctx().clone();
    egui::Frame::NONE
        .fill(Color32::from_rgb(20, 30, 48))
        .stroke(Stroke::new(1.0_f32, alpha(AI_GLOW, if matches!(status, ToolStatus::Running) { 0.45 } else { 0.15 })))
        .corner_radius(CornerRadius::same(R_MD))
        .inner_margin(egui::Margin::symmetric(10, 7))
        .outer_margin(egui::Margin { left: 30, ..Default::default() })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                match status {
                    ToolStatus::Running => {
                        theme::spinner(ui.painter(), r.center(), 6.0, AI_GLOW, ui.input(|i| i.time));
                        ctx.request_repaint();
                    }
                    ToolStatus::Done => {
                        let t = theme::since(&ctx, at, 0.4);
                        ui.painter().text(r.center(), Align2::CENTER_CENTER, icons::CHECK, theme::bold_font(13.0 * (0.7 + 0.3 * t)), SUCCESS);
                    }
                    ToolStatus::Failed(_) => {
                        ui.painter().text(r.center(), Align2::CENTER_CENTER, icons::X, theme::bold_font(13.0), DANGER);
                    }
                }
                let col = if matches!(status, ToolStatus::Running) { Color32::WHITE } else { TEXT_DIM };
                ui.add(egui::Label::new(RichText::new(label).color(col).size(12.5)).truncate());
            });
            if let ToolStatus::Failed(msg) = status {
                if !msg.is_empty() {
                    ui.label(RichText::new(msg).color(Color32::from_rgb(255, 160, 160)).size(11.5));
                }
            }
            if let Some(tex) = image {
                let w = ui.available_width();
                let s = tex.size_vec2();
                let h = (w * s.y / s.x).min(220.0);
                let w = h * s.x / s.y;
                ui.add_space(4.0);
                ui.add(egui::Image::new(tex).fit_to_exact_size(vec2(w, h)).corner_radius(CornerRadius::same(R_SM)));
            }
        });
}

fn error_row(ui: &mut Ui, text: &str) {
    egui::Frame::NONE.fill(alpha(DANGER, 0.12)).corner_radius(CornerRadius::same(R_MD)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_top(|ui| {
            ui.label(RichText::new(icons::WARNING).color(DANGER));
            ui.add(egui::Label::new(RichText::new(text).color(Color32::from_rgb(255, 180, 180)).size(12.5)).wrap());
        });
    });
}

fn typing_dots(ui: &mut Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(60.0, 20.0), Sense::hover());
    let t = ui.input(|i| i.time) as f32;
    for k in 0..3 {
        let phase = (t * 4.0 - k as f32 * 0.6).sin() * 0.5 + 0.5;
        let c = pos2(r.left() + 36.0 + k as f32 * 10.0, r.center().y - phase * 3.0);
        ui.painter().circle_filled(c, 3.2, alpha(AI_GLOW, 0.4 + 0.6 * phase));
    }
    ui.ctx().request_repaint();
}

/// Minimal markdown: **bold**, `code`, bullet lines and headings.
fn markdown_job(text: &str, width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = width;
    let normal = TextFormat { font_id: theme::font(13.5), color: TEXT, ..Default::default() };
    let bold = TextFormat { font_id: theme::bold_font(13.5), color: Color32::WHITE, ..Default::default() };
    let code = TextFormat { font_id: egui::FontId::monospace(12.5), color: ACCENT_HI, background: SURFACE_HI, ..Default::default() };
    let lines: Vec<&str> = text.lines().collect();
    for (li, raw) in lines.iter().enumerate() {
        let mut line = raw.trim_end().to_string();
        let mut base = normal.clone();
        if let Some(rest) = line.trim_start().strip_prefix("#") {
            line = rest.trim_start_matches('#').trim().to_string();
            base = bold.clone();
        } else if let Some(rest) = line.trim_start().strip_prefix("- ").or_else(|| line.trim_start().strip_prefix("* ")) {
            line = format!("•  {rest}");
        }
        let mut is_bold = false;
        let mut is_code = false;
        let mut buf = String::new();
        let flush = |buf: &mut String, job: &mut LayoutJob, is_bold: bool, is_code: bool| {
            if !buf.is_empty() {
                let f = if is_code { code.clone() } else if is_bold { bold.clone() } else { base.clone() };
                job.append(buf, 0.0, f);
                buf.clear();
            }
        };
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if !is_code && chars[i] == '*' && chars.get(i + 1) == Some(&'*') {
                flush(&mut buf, &mut job, is_bold, is_code);
                is_bold = !is_bold;
                i += 2;
                continue;
            }
            if chars[i] == '`' {
                flush(&mut buf, &mut job, is_bold, is_code);
                is_code = !is_code;
                i += 1;
                continue;
            }
            buf.push(chars[i]);
            i += 1;
        }
        flush(&mut buf, &mut job, is_bold, is_code);
        if li + 1 < lines.len() {
            job.append("\n", 0.0, normal.clone());
        }
    }
    job
}
