//! Right panel: properties of the selected clip, and the activity log.

use egui::{vec2, Color32, CornerRadius, RichText, Sense, Stroke, StrokeKind, Ui};

use super::theme::{self, icons, *};
use super::{widgets, App, Dialog};
use crate::media::MediaKind;
use crate::project::{fmt_time, Fit, Project, Source, Track, MAX_SPEED, MIN_CLIP, MIN_SPEED};
use super::dialogs::CanvasForm;

const SWATCHES: [[u8; 3]; 7] = [[255, 255, 255], [255, 221, 0], [0, 0, 0], [235, 64, 52], [60, 200, 90], [60, 130, 246], [196, 160, 255]];

impl App {
    /// Applies an inspector edit; edits with the same key merge into one undo step.
    fn edit_merge(&mut self, desc: &str, key: &str, f: impl FnOnce(&mut Project) -> anyhow::Result<()>) {
        let _ = self.shared.editor.lock().apply_merge(Source::User, desc, Some(key), f);
    }

    pub(super) fn right_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("right")
            .exact_width(372.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(theme::floating_panel(0, 8, 0, 8))
            .show(ctx, |ui| {
                let items = [(icons::SPARKLE, "Assistant"), (icons::SLIDERS, "Properties"), (icons::CLOCK_COUNTER_CLOCKWISE, "History")];
                widgets::tabs(ui, egui::Id::new("right-tabs"), &items, &mut self.right_tab);
                ui.add_space(12.0);
                // Content slides in a little when switching tabs.
                let t = theme::ease_bool(ctx, egui::Id::new(("tab-anim", self.right_tab)), true, 0.2);
                let mut content = ui.new_child(egui::UiBuilder::new().max_rect(ui.available_rect_before_wrap().translate(vec2((1.0 - t) * 14.0, 0.0))));
                content.set_opacity(t);
                match self.right_tab {
                    0 => self.chat_ui(&mut content),
                    1 => {
                        egui::ScrollArea::vertical().id_salt("inspector-scroll").auto_shrink([false, false]).show(&mut content, |ui| {
                            let track = self.selected.and_then(|id| self.snapshot.track_of(id));
                            let a = theme::ease_bool(ctx, egui::Id::new(("insp", self.selected)), true, 0.18);
                            ui.set_opacity(a);
                            match track {
                                Some(Track::Main) => self.main_clip_ui(ui),
                                Some(Track::Music) => self.music_ui(ui),
                                Some(Track::Text) => self.text_ui(ui),
                                None => self.project_ui(ui),
                            }
                        });
                    }
                    _ => self.activity_ui(&mut content),
                }
            });
    }

    fn title(ui: &mut Ui, title: &str, sub: &str) {
        ui.label(RichText::new(title).font(theme::bold_font(16.0)).color(Color32::WHITE));
        if !sub.is_empty() {
            ui.label(RichText::new(sub).color(TEXT_FAINT).small());
        }
        ui.add_space(10.0);
    }

    fn project_ui(&mut self, ui: &mut Ui) {
        let p = self.snapshot.clone();
        Self::title(ui, "Project", "Select a clip to edit it");
        theme::card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            row(ui, "Canvas", &format!("{} × {}", p.canvas.width, p.canvas.height));
            row(ui, "Format", &p.canvas.summary());
            row(ui, "Length", &fmt_time(p.duration()));
            row(ui, "Clips", &format!("{} video · {} music · {} text", p.main.len(), p.music.len(), p.texts.len()));
            ui.add_space(6.0);
            if widgets::secondary_button(ui, "Change format…", true).clicked() {
                self.open_dialog(Dialog::Settings { form: CanvasForm::from_canvas(&p.canvas) });
            }
        });
        ui.add_space(10.0);
        widgets::section_label(ui, "Shortcuts");
        for (k, v) in [
            ("Space", "Play / pause"),
            ("S", "Split at playhead"),
            ("Del", "Delete clip"),
            ("T", "Add text"),
            ("← →", "Step one frame"),
            ("Ctrl+Z / Y", "Undo / redo"),
            ("Ctrl+S", "Save project"),
        ] {
            ui.horizontal(|ui| {
                ui.label(RichText::new(k).monospace().color(TEXT_DIM));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(RichText::new(v).color(TEXT_FAINT).small()));
            });
        }
    }

    fn main_clip_ui(&mut self, ui: &mut Ui) {
        let Some(id) = self.selected else { return };
        let p = self.snapshot.clone();
        let Some(i) = p.main_index(id) else { return };
        let clip = p.main[i].clone();
        let Some(asset) = p.asset(clip.asset_id).cloned() else { return };
        let start = p.main_starts()[i];
        Self::title(ui, &asset.name, &format!("Video clip · {} – {}", fmt_time(start), fmt_time(start + clip.duration())));

        theme::card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            if asset.kind == MediaKind::Image {
                let mut d = clip.duration();
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Duration").color(TEXT_DIM));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::DragValue::new(&mut d).speed(0.05).range(MIN_CLIP..=3600.0).suffix(" s").max_decimals(2)).changed() {
                            self.edit_merge("Change duration", &format!("dur{id}"), |p| p.trim_main(id, 0.0, d));
                        }
                    });
                });
            } else {
                let (mut a, mut b) = (clip.src_in, clip.src_out);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Start in source").color(TEXT_DIM));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::DragValue::new(&mut a).speed(0.02).range(0.0..=(b - MIN_CLIP)).suffix(" s").max_decimals(2)).changed() {
                            self.edit_merge("Trim clip", &format!("in{id}"), |p| p.trim_main(id, a, b));
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("End in source").color(TEXT_DIM));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::DragValue::new(&mut b).speed(0.02).range((a + MIN_CLIP)..=asset.duration).suffix(" s").max_decimals(2)).changed() {
                            self.edit_merge("Trim clip", &format!("out{id}"), |p| p.trim_main(id, a, b));
                        }
                    });
                });
                row(ui, "Length", &fmt_time(clip.duration()));
            }
        });
        ui.add_space(10.0);
        widgets::section_label(ui, "Framing");
        ui.horizontal(|ui| {
            if widgets::chip(ui, "Fit", Some("whole picture"), clip.fit == Fit::Fit).clicked() && clip.fit != Fit::Fit {
                self.edit("Fit to frame", |p| p.set_fit(id, Fit::Fit));
            }
            if widgets::chip(ui, "Fill", Some("crop to frame"), clip.fit == Fit::Fill).clicked() && clip.fit != Fit::Fill {
                self.edit("Fill frame", |p| p.set_fit(id, Fit::Fill));
            }
        });
        if asset.has_audio {
            ui.add_space(10.0);
            widgets::section_label(ui, "Volume");
            self.volume_slider(ui, id, clip.volume);
        }
        if asset.kind == MediaKind::Video {
            ui.add_space(10.0);
            widgets::section_label(ui, "Speed");
            ui.horizontal(|ui| {
                for s in [0.5, 1.0, 1.5, 2.0, 3.0] {
                    let sel = (clip.speed - s).abs() < 1e-3;
                    if widgets::chip(ui, &format!("{s}×"), None, sel).clicked() && !sel {
                        self.edit(&format!("Speed {s}×"), |p| p.set_speed(id, s));
                    }
                }
            });
            let mut sp = clip.speed;
            if ui.add(egui::Slider::new(&mut sp, MIN_SPEED..=MAX_SPEED).logarithmic(true).custom_formatter(|v, _| format!("{v:.2}×"))).changed() {
                self.edit_merge("Speed", &format!("speed{id}"), |p| p.set_speed(id, sp));
            }
        }
        ui.add_space(10.0);
        widgets::section_label(ui, "Fade");
        self.fade_sliders(ui, id, clip.fade_in, clip.fade_out, clip.duration());
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            widgets::section_label(ui, "Color");
            if clip.has_color() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Reset").clicked() {
                        self.edit("Reset color", |p| p.set_color(id, Some(0.0), Some(1.0), Some(1.0)));
                    }
                });
            }
        });
        let mut vals = [clip.brightness, clip.contrast, clip.saturation];
        for (i, (label, icon, range, key)) in [
            ("Brightness", icons::SUN, -1.0..=1.0, "bri"),
            ("Contrast", icons::CIRCLE_HALF, 0.0..=2.0, "con"),
            ("Saturation", icons::DROP, 0.0..=2.0, "sat"),
        ]
        .into_iter()
        .enumerate()
        {
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon).color(TEXT_FAINT));
                ui.add_sized(vec2(78.0, 18.0), egui::Label::new(RichText::new(label).color(TEXT_DIM).size(12.5)));
                if ui.add(egui::Slider::new(&mut vals[i], range).show_value(false)).changed() {
                    let v = vals;
                    self.edit_merge("Color", &format!("{key}{id}"), |p| p.set_color(id, Some(v[0]), Some(v[1]), Some(v[2])));
                }
            });
        }
        ui.add_space(14.0);
        self.clip_actions(ui);
    }

    fn fade_sliders(&mut self, ui: &mut Ui, id: u64, fade_in: f64, fade_out: f64, duration: f64) {
        let max = (duration / 2.0).min(5.0).max(0.0);
        let (mut fi, mut fo) = (fade_in, fade_out);
        ui.horizontal(|ui| {
            ui.add_sized(vec2(60.0, 18.0), egui::Label::new(RichText::new("In").color(TEXT_DIM).size(12.5)));
            if ui.add(egui::Slider::new(&mut fi, 0.0..=max).custom_formatter(|v, _| format!("{v:.1}s"))).changed() {
                self.edit_merge("Fade in", &format!("fi{id}"), |p| p.set_fades(id, Some(fi), None));
            }
        });
        ui.horizontal(|ui| {
            ui.add_sized(vec2(60.0, 18.0), egui::Label::new(RichText::new("Out").color(TEXT_DIM).size(12.5)));
            if ui.add(egui::Slider::new(&mut fo, 0.0..=max).custom_formatter(|v, _| format!("{v:.1}s"))).changed() {
                self.edit_merge("Fade out", &format!("fo{id}"), |p| p.set_fades(id, None, Some(fo)));
            }
        });
    }

    fn clip_actions(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            if widgets::icon_text_button(ui, icons::COPY, "Duplicate", true, false).on_hover_text("Ctrl+D").clicked() {
                self.duplicate_selected();
            }
            if widgets::icon_text_button(ui, icons::TRASH, "Delete", true, false).on_hover_text("Delete").clicked() {
                self.delete_selected();
            }
        });
    }

    fn music_ui(&mut self, ui: &mut Ui) {
        let Some(id) = self.selected else { return };
        let p = self.snapshot.clone();
        let Some(m) = p.music.iter().find(|m| m.id == id).cloned() else { return };
        let name = p.asset(m.asset_id).map(|a| a.name.clone()).unwrap_or_default();
        let max = p.asset(m.asset_id).map(|a| a.duration).unwrap_or(m.src_out);
        Self::title(ui, &name, &format!("Music · {} – {}", fmt_time(m.start), fmt_time(m.end())));
        theme::card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            let mut start = m.start;
            ui.horizontal(|ui| {
                ui.label(RichText::new("Starts at").color(TEXT_DIM));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::DragValue::new(&mut start).speed(0.05).range(0.0..=36000.0).suffix(" s").max_decimals(2)).changed() {
                        self.edit_merge("Move music", &format!("ms{id}"), |p| p.update_music(id, Some(start), None, None));
                    }
                });
            });
            let (mut a, mut b) = (m.src_in, m.src_out);
            ui.horizontal(|ui| {
                ui.label(RichText::new("From").color(TEXT_DIM));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::DragValue::new(&mut a).speed(0.05).range(0.0..=(b - MIN_CLIP)).suffix(" s").max_decimals(2)).changed() {
                        self.edit_merge("Trim music", &format!("mi{id}"), |p| p.update_music(id, None, Some((a, b)), None));
                    }
                });
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("To").color(TEXT_DIM));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::DragValue::new(&mut b).speed(0.05).range((a + MIN_CLIP)..=max).suffix(" s").max_decimals(2)).changed() {
                        self.edit_merge("Trim music", &format!("mo{id}"), |p| p.update_music(id, None, Some((a, b)), None));
                    }
                });
            });
        });
        ui.add_space(10.0);
        widgets::section_label(ui, "Volume");
        self.volume_slider(ui, id, m.volume);
        ui.add_space(10.0);
        widgets::section_label(ui, "Fade");
        self.fade_sliders(ui, id, m.fade_in, m.fade_out, m.duration());
        ui.add_space(14.0);
        self.clip_actions(ui);
    }

    fn text_ui(&mut self, ui: &mut Ui) {
        let Some(id) = self.selected else { return };
        let Some(t) = self.snapshot.texts.iter().find(|t| t.id == id).cloned() else { return };
        Self::title(ui, "Text", &format!("{} – {}", fmt_time(t.start), fmt_time(t.end())));

        // Keep a local buffer so an empty box while typing is allowed.
        let buf = match &mut self.text_buffer {
            Some((bid, s)) if *bid == id => s,
            _ => {
                self.text_buffer = Some((id, t.text.clone()));
                &mut self.text_buffer.as_mut().unwrap().1
            }
        };
        let resp = ui.add(egui::TextEdit::multiline(buf).desired_rows(3).desired_width(f32::INFINITY).margin(vec2(10.0, 8.0)).hint_text("Type your title…"));
        if resp.changed() && !buf.trim().is_empty() {
            let text = buf.clone();
            self.edit_merge("Edit text", &format!("tx{id}"), |p| {
                p.text_mut(id)?.text = text;
                Ok(())
            });
        } else if !resp.has_focus() && *buf != t.text && !buf.trim().is_empty() {
            *buf = t.text.clone();
        } else if !resp.has_focus() && buf.trim().is_empty() {
            *buf = t.text.clone();
        }
        ui.add_space(8.0);
        theme::card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            let (mut start, mut dur) = (t.start, t.duration);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Starts at").color(TEXT_DIM));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::DragValue::new(&mut start).speed(0.05).range(0.0..=36000.0).suffix(" s").max_decimals(2)).changed() {
                        self.edit_merge("Move text", &format!("ts{id}"), |p| {
                            p.text_mut(id)?.start = start;
                            Ok(())
                        });
                    }
                });
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Duration").color(TEXT_DIM));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::DragValue::new(&mut dur).speed(0.05).range(MIN_CLIP..=36000.0).suffix(" s").max_decimals(2)).changed() {
                        self.edit_merge("Text duration", &format!("td{id}"), |p| {
                            p.text_mut(id)?.duration = dur;
                            Ok(())
                        });
                    }
                });
            });
        });
        ui.add_space(10.0);
        widgets::section_label(ui, "Position");
        ui.horizontal(|ui| {
            for (label, y) in [("Top", 0.12f32), ("Center", 0.5), ("Bottom", 0.85)] {
                let sel = (t.y - y).abs() < 0.02 && (t.x - 0.5).abs() < 0.02;
                if widgets::chip(ui, label, None, sel).clicked() {
                    self.edit("Move text", |p| {
                        let tx = p.text_mut(id)?;
                        tx.x = 0.5;
                        tx.y = y;
                        Ok(())
                    });
                }
            }
        });
        let (mut x, mut y) = (t.x, t.y);
        ui.horizontal(|ui| {
            ui.label(RichText::new("X").color(TEXT_FAINT));
            if ui.add(egui::Slider::new(&mut x, 0.0..=1.0).show_value(false)).changed() {
                self.edit_merge("Move text", &format!("tx-pos{id}"), |p| {
                    p.text_mut(id)?.x = x;
                    Ok(())
                });
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("Y").color(TEXT_FAINT));
            if ui.add(egui::Slider::new(&mut y, 0.0..=1.0).show_value(false)).changed() {
                self.edit_merge("Move text", &format!("ty-pos{id}"), |p| {
                    p.text_mut(id)?.y = y;
                    Ok(())
                });
            }
        });
        ui.add_space(8.0);
        widgets::section_label(ui, "Style");
        let mut size = t.size;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Size").color(TEXT_FAINT));
            if ui.add(egui::Slider::new(&mut size, 0.02..=0.2).show_value(false)).changed() {
                self.edit_merge("Text size", &format!("tsz{id}"), |p| {
                    p.text_mut(id)?.size = size;
                    Ok(())
                });
            }
        });
        ui.horizontal(|ui| {
            for c in SWATCHES {
                let (r, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
                let sel = t.color == c;
                let h = ui.ctx().animate_bool_with_time(resp.id, resp.hovered() || sel, 0.12);
                let painter = ui.painter();
                painter.circle_filled(r.center(), 9.0 + 2.0 * h, Color32::from_rgb(c[0], c[1], c[2]));
                painter.circle_stroke(r.center(), 9.0 + 2.0 * h, Stroke::new(1.0_f32, BORDER));
                if sel {
                    painter.circle_stroke(r.center(), 13.0, Stroke::new(2.0_f32, ACCENT_HI));
                }
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !sel {
                    self.edit("Text color", |p| {
                        p.text_mut(id)?.color = c;
                        Ok(())
                    });
                }
            }
        });
        ui.add_space(4.0);
        let mut bg = t.background;
        if toggle(ui, &mut bg, "Dark box behind text").changed() {
            self.edit("Text background", |p| {
                p.text_mut(id)?.background = bg;
                Ok(())
            });
        }
        let mut fade = t.fade;
        if toggle(ui, &mut fade, "Fade in and out").changed() {
            self.edit("Text fade", |p| {
                p.text_mut(id)?.fade = fade;
                Ok(())
            });
        }
        ui.add_space(14.0);
        self.clip_actions(ui);
    }

    fn volume_slider(&mut self, ui: &mut Ui, id: u64, volume: f32) {
        let mut v = volume;
        ui.horizontal(|ui| {
            let r = ui.add(egui::Slider::new(&mut v, 0.0..=2.0).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)).custom_parser(|s| s.trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0)));
            if r.changed() {
                self.edit_merge("Volume", &format!("vol{id}"), |p| p.set_volume(id, v));
            }
        });
    }

    fn activity_ui(&mut self, ui: &mut Ui) {
        let (can_undo, can_redo) = {
            let ed = self.shared.editor.lock();
            (ed.can_undo(), ed.can_redo())
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new("History").font(theme::bold_font(15.0)).color(Color32::WHITE));
            ui.label(RichText::new("your edits and Claude's").color(TEXT_FAINT).small());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::icon_button(ui, icons::ARROW_CLOCKWISE, "Redo (Ctrl+Y)", can_redo).clicked() {
                    self.redo();
                }
                if widgets::icon_button(ui, icons::ARROW_COUNTER_CLOCKWISE, "Undo (Ctrl+Z)", can_undo).clicked() {
                    self.undo();
                }
            });
        });
        ui.add_space(8.0);
        let entries: Vec<(Source, bool, String, std::time::Instant)> = {
            let ed = self.shared.editor.lock();
            ed.log.iter().rev().take(200).map(|e| (e.source, e.ok, e.text.clone(), e.at)).collect()
        };
        if entries.is_empty() {
            ui.label(RichText::new("Edits appear here. Ask Claude to edit and watch them land live.").color(TEXT_FAINT).small());
            return;
        }
        let ctx = ui.ctx().clone();
        egui::ScrollArea::vertical().id_salt("activity").auto_shrink([false, false]).show(ui, |ui| {
            for (source, ok, text, at) in entries {
                let a = theme::since(&ctx, at, 0.35);
                let (icon, color) = match (source, ok) {
                    (_, false) => (icons::WARNING, DANGER),
                    (Source::Ai, _) => (icons::SPARKLE, AI_GLOW),
                    (Source::User, _) => (icons::CURSOR_CLICK, TEXT_FAINT),
                };
                let resp = egui::Frame::NONE
                    .fill(if source == Source::Ai && ok { alpha(Color32::from_rgb(24, 52, 100), 0.35 * a) } else { Color32::TRANSPARENT })
                    .corner_radius(CornerRadius::same(R_SM))
                    .inner_margin(egui::Margin::symmetric(8, 5))
                    .outer_margin(egui::Margin { left: ((1.0 - a) * 16.0) as i8, ..Default::default() })
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal_top(|ui| {
                            ui.label(RichText::new(icon).color(alpha(color, a)).strong());
                            ui.add(egui::Label::new(RichText::new(&text).color(alpha(TEXT, a)).size(12.5)).wrap());
                        });
                    });
                resp.response.on_hover_text(format!("{} ago", super::dialogs::human_secs(at.elapsed().as_secs_f64())));
            }
        });
    }
}

fn row(ui: &mut Ui, k: &str, v: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(k).color(TEXT_DIM));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(v));
    });
}

/// iOS-style animated switch.
fn toggle(ui: &mut Ui, on: &mut bool, label: &str) -> egui::Response {
    let mut resp = ui
        .horizontal(|ui| {
            let (r, resp) = ui.allocate_exact_size(vec2(36.0, 20.0), Sense::click());
            let t = theme::ease_bool(ui.ctx(), resp.id, *on, 0.15);
            let p = ui.painter();
            p.rect(r, CornerRadius::same(10), theme::lerp_color(SURFACE_HI, ACCENT, t), Stroke::new(1.0_f32, BORDER), StrokeKind::Inside);
            let cx = r.left() + 10.0 + t * (r.width() - 20.0);
            p.circle_filled(egui::pos2(cx, r.center().y), 7.0, Color32::WHITE);
            ui.label(RichText::new(label).color(TEXT_DIM));
            resp
        })
        .inner;
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    resp
}
