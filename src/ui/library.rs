//! Left panel: the media library.

use std::time::Instant;

use egui::{pos2, vec2, Align2, Color32, CornerRadius, Id as EId, Rect, RichText, Sense, Stroke, StrokeKind};

use super::theme::{self, *};
use super::{widgets, App, AssetDrag};
use crate::media::MediaKind;
use crate::project::{fmt_time, Asset, Source};

impl App {
    pub(super) fn library_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("library").exact_width(276.0).resizable(false).show_separator_line(false).frame(theme::floating_panel(8, 0, 0, 8)).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Media").font(theme::bold_font(16.0)).color(Color32::WHITE));
                let n = self.snapshot.assets.len();
                if n > 0 {
                    ui.label(RichText::new(n.to_string()).color(TEXT_FAINT));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::primary_button(ui, "+ Import", true).on_hover_text("Ctrl+I").clicked() {
                        self.import_dialog();
                    }
                });
            });
            ui.add_space(10.0);

            let importing = self.shared.importing.lock().clone();
            let assets = self.snapshot.assets.clone();
            if assets.is_empty() && importing.is_empty() {
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 150.0), Sense::hover());
                super::timeline::dashed_rect(ui.painter(), r, alpha(TEXT_FAINT, 0.6));
                ui.painter().text(r.center() - vec2(0.0, 10.0), Align2::CENTER_CENTER, "No media yet", theme::bold_font(14.0), TEXT_DIM);
                ui.painter().text(r.center() + vec2(0.0, 12.0), Align2::CENTER_CENTER, "Drop files anywhere in the window", theme::font(12.0), TEXT_FAINT);
                return;
            }

            let col_w = (ui.available_width() - 10.0) / 2.0;
            let card_h = col_w * 9.0 / 16.0 + 40.0;
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let items = assets.len() + importing.len();
                let rows = items.div_ceil(2);
                for row in 0..rows {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        for col in 0..2 {
                            let i = row * 2 + col;
                            if i < assets.len() {
                                self.asset_card(ui, &assets[i], vec2(col_w, card_h));
                            } else if i < items {
                                let m = &importing[i - assets.len()];
                                shimmer_card(ui, vec2(col_w, card_h), &m.name, m.started);
                            }
                        }
                    });
                    ui.add_space(10.0);
                }
            });
        });
    }

    fn asset_card(&mut self, ui: &mut egui::Ui, asset: &Asset, size: egui::Vec2) {
        let ctx = ui.ctx().clone();
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
        let born = *self.lib_born.entry(asset.id).or_insert_with(Instant::now);
        let appear = theme::since(&ctx, born, 0.3);
        let h = theme::ease_bool(&ctx, resp.id.with("h"), resp.hovered(), 0.14);
        let r = rect.translate(vec2(0.0, -2.0 * h + (1.0 - appear) * 10.0));
        let p = ui.painter();
        let a = appear;
        if h > 0.0 {
            theme::shadow(p, r, R_MD as f32, 0.6 * h);
        }
        p.rect_filled(r, CornerRadius::same(R_MD), alpha(theme::lerp_color(SURFACE, SURFACE_HI, h), a));
        let thumb = Rect::from_min_size(r.min, vec2(r.width(), r.width() * 9.0 / 16.0));
        let media = self.media.get(asset.id);
        match asset.kind {
            MediaKind::Audio => {
                p.rect_filled(thumb, CornerRadius { nw: R_MD, ne: R_MD, sw: 0, se: 0 }, alpha(Color32::from_rgb(16, 64, 52), a));
                if let Some(wave) = media.and_then(|m| m.wave.clone()) {
                    let n = 40;
                    let bw = thumb.width() / n as f32;
                    for i in 0..n {
                        let idx = (i * wave.len()) / n;
                        let end = (((i + 1) * wave.len()) / n).max(idx + 1).min(wave.len());
                        let v = wave.get(idx..end).map(|s| s.iter().cloned().fold(0.0f32, f32::max)).unwrap_or(0.0);
                        let x = thumb.left() + (i as f32 + 0.5) * bw;
                        let hh = (v * thumb.height() * 0.35).max(1.0);
                        p.line_segment([pos2(x, thumb.center().y - hh), pos2(x, thumb.center().y + hh)], Stroke::new(bw * 0.6, alpha(CLIP_MUSIC, a)));
                    }
                } else {
                    p.text(thumb.center(), Align2::CENTER_CENTER, "♪", theme::font(28.0), alpha(CLIP_MUSIC, a));
                }
            }
            _ => {
                p.rect_filled(thumb, CornerRadius { nw: R_MD, ne: R_MD, sw: 0, se: 0 }, alpha(BG, a));
                if let Some(t) = media.and_then(|m| m.cover()) {
                    let fade = theme::since(&ctx, t.loaded, 0.35) * a;
                    let mut mesh = egui::Mesh::with_texture(t.tex.id());
                    mesh.add_rect_with_uv(thumb, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), alpha(Color32::WHITE, fade));
                    p.with_clip_rect(thumb).add(egui::Shape::mesh(mesh));
                } else if media.is_some_and(|m| m.failed) {
                    p.text(thumb.center(), Align2::CENTER_CENTER, "⚠", theme::font(22.0), alpha(WARNING, a));
                } else {
                    theme::spinner(p, thumb.center(), 8.0, alpha(TEXT_FAINT, a), ui.input(|i| i.time));
                    ctx.request_repaint();
                }
            }
        }
        // Duration badge.
        if asset.kind != MediaKind::Image {
            let g = p.layout_no_wrap(fmt_time(asset.duration), theme::font(10.5), Color32::WHITE);
            let br = Rect::from_min_size(thumb.right_bottom() - g.size() - vec2(12.0, 8.0), g.size() + vec2(8.0, 4.0));
            p.rect_filled(br, CornerRadius::same(4), Color32::from_black_alpha((170.0 * a) as u8));
            p.galley(br.min + vec2(4.0, 2.0), g, alpha(Color32::WHITE, a));
        }
        // Name and kind.
        let text_area = Rect::from_min_max(pos2(r.left() + 8.0, thumb.bottom() + 4.0), pos2(r.right() - 8.0, r.bottom()));
        let tp = p.with_clip_rect(text_area);
        tp.text(text_area.left_top(), Align2::LEFT_TOP, &asset.name, theme::font(12.5), alpha(TEXT, a));
        let kind = match asset.kind {
            MediaKind::Video => format!("Video · {}×{}", asset.width, asset.height),
            MediaKind::Audio => "Audio".to_string(),
            MediaKind::Image => format!("Image · {}×{}", asset.width, asset.height),
        };
        tp.text(text_area.left_top() + vec2(0.0, 17.0), Align2::LEFT_TOP, kind, theme::font(10.5), alpha(TEXT_FAINT, a));
        p.rect_stroke(r, CornerRadius::same(R_MD), Stroke::new(1.0_f32, theme::lerp_color(BORDER, alpha(ACCENT_HI, 0.8), h)), StrokeKind::Inside);

        // "+" quick add button.
        let mut added = false;
        if h > 0.01 {
            let br = Rect::from_center_size(thumb.right_top() + vec2(-16.0, 16.0), vec2(26.0, 26.0));
            let bresp = ui.interact(br, resp.id.with("add"), Sense::click());
            let bh = ctx.animate_bool_with_time(bresp.id, bresp.hovered(), 0.1);
            p.circle_filled(br.center(), 12.0 * (0.8 + 0.2 * h) + bh * 1.5, alpha(theme::lerp_color(ACCENT, ACCENT_HI, bh), h));
            p.text(br.center(), Align2::CENTER_CENTER, "+", theme::bold_font(16.0), alpha(Color32::WHITE, h));
            if bresp.on_hover_text("Add to timeline").clicked() {
                added = true;
            }
        }
        // Drag onto the timeline.
        if resp.drag_started() {
            egui::DragAndDrop::set_payload(&ctx, AssetDrag(asset.id));
        }
        if resp.dragged() && egui::DragAndDrop::has_payload_of_type::<AssetDrag>(&ctx) {
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            if let Some(pos) = ctx.pointer_hover_pos() {
                let fp = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, EId::new("asset-drag")));
                let fr = Rect::from_min_size(pos + vec2(10.0, 10.0), vec2(150.0, 38.0));
                theme::shadow(&fp, fr, 8.0, 1.0);
                fp.rect_filled(fr, CornerRadius::same(8), SURFACE_HI);
                fp.rect_stroke(fr, CornerRadius::same(8), Stroke::new(1.0_f32, ACCENT_HI), StrokeKind::Inside);
                if let Some(t) = media.and_then(|m| m.cover()) {
                    fp.image(t.tex.id(), Rect::from_min_size(fr.min + vec2(4.0, 4.0), vec2(53.0, 30.0)), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
                fp.with_clip_rect(fr.shrink(4.0)).text(fr.left_center() + vec2(62.0, 0.0), Align2::LEFT_CENTER, &asset.name, theme::font(12.0), TEXT);
            }
        }

        if added || resp.double_clicked() {
            self.add_asset_to_timeline(asset.id, None);
        }

        let id = asset.id;
        let name = asset.name.clone();
        resp.on_hover_text("Drag to the timeline, or double-click to add").context_menu(|ui| {
            if ui.button("Add to timeline").clicked() {
                ui.close_menu();
                self.add_asset_to_timeline(id, None);
            }
            if ui.button(RichText::new("Remove from project").color(DANGER)).clicked() {
                ui.close_menu();
                let _ = self.shared.editor.lock().apply(Source::User, &format!("Remove {name}"), |p| p.remove_asset(id));
            }
        });
    }
}

fn shimmer_card(ui: &mut egui::Ui, size: egui::Vec2, name: &str, started: Instant) {
    let (r, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter();
    p.rect_filled(r, CornerRadius::same(R_MD), SURFACE);
    let thumb = Rect::from_min_size(r.min, vec2(r.width(), r.width() * 9.0 / 16.0));
    p.rect_filled(thumb.shrink(6.0), CornerRadius::same(6), SURFACE_HI);
    let t = (started.elapsed().as_secs_f32() * 0.8).fract();
    let x = r.left() - 60.0 + t * (r.width() + 120.0);
    let band = Rect::from_min_max(pos2(x, r.top()), pos2(x + 50.0, r.bottom())).intersect(r);
    if band.width() > 0.0 {
        p.rect_filled(band, CornerRadius::same(R_MD), Color32::from_white_alpha(10));
    }
    p.with_clip_rect(r.shrink(8.0)).text(pos2(r.left() + 8.0, thumb.bottom() + 6.0), Align2::LEFT_TOP, name, theme::font(12.0), TEXT_DIM);
    p.text(pos2(r.left() + 8.0, thumb.bottom() + 23.0), Align2::LEFT_TOP, "Importing…", theme::font(10.5), TEXT_FAINT);
    ui.ctx().request_repaint();
}
