//! The inspector: properties of the selected clip.

use std::ops::RangeInclusive;

use iced::widget::{button, column, container, pick_list, row, slider, space, text, text_input, toggler};
use iced::{Alignment, Color, Element, Fill, Length};

use super::Message;
use super::timeline::clip_label;
use crate::clip::{Align, Clip, Content, Fit, MotionKind, Rgba, TransitionKind};
use crate::model::{Id, MediaKind, Project};
use crate::ui::icons::{code, icon, tinted};
use crate::ui::tokens::*;
use crate::ui::{style, widgets};

#[derive(Debug, Clone)]
pub enum Edit {
    Name(String),
    X(f64),
    Y(f64),
    Position(f64, f64),
    Scale(f64),
    Rotation(f64),
    Opacity(f64),
    Fit(Fit),
    ResetTransform,
    Radius(f64),
    BorderWidth(f64),
    BorderColor(Rgba),
    Shadow(bool),
    ShadowBlur(f64),
    ShadowOffset(f64),
    ShadowOpacity(f64),
    Brightness(f64),
    Contrast(f64),
    Saturation(f64),
    Temperature(f64),
    ResetGrade,
    Volume(f64),
    Mute(bool),
    FadeIn(f64),
    FadeOut(f64),
    Speed(f64),
    Text(String),
    Font(String),
    FontSize(f64),
    Weight(u16),
    TextColor(Rgba),
    Align(Align),
    TextBox(bool),
    Intro(Option<MotionKind>),
    Outro(Option<MotionKind>),
    IntroDuration(f64),
    OutroDuration(f64),
    Transition(Option<TransitionKind>),
    TransitionDuration(f64),
    AddZoom,
    RemoveZoom(Id),
    ZoomScale(Id, f64),
    ZoomFocusX(Id, f64),
    ZoomFocusY(Id, f64),
    ZoomFocus(Id, f64, f64),
}

impl Edit {
    /// Undo-merge key and description: repeated edits of the same field merge into one step.
    pub fn describe(&self) -> (&'static str, &'static str) {
        match self {
            Edit::Name(_) => ("name", "Rename clip"),
            Edit::X(_) | Edit::Y(_) | Edit::Position(..) => ("position", "Move"),
            Edit::Scale(_) => ("scale", "Scale"),
            Edit::Rotation(_) => ("rotation", "Rotate"),
            Edit::Opacity(_) => ("opacity", "Opacity"),
            Edit::Fit(_) => ("fit", "Framing"),
            Edit::ResetTransform => ("reset-transform", "Reset transform"),
            Edit::Radius(_) => ("radius", "Corner radius"),
            Edit::BorderWidth(_) | Edit::BorderColor(_) => ("border", "Border"),
            Edit::Shadow(_) | Edit::ShadowBlur(_) | Edit::ShadowOffset(_) | Edit::ShadowOpacity(_) => ("shadow", "Shadow"),
            Edit::Brightness(_) | Edit::Contrast(_) | Edit::Saturation(_) | Edit::Temperature(_) => ("grade", "Color"),
            Edit::ResetGrade => ("reset-grade", "Reset color"),
            Edit::Volume(_) | Edit::Mute(_) => ("volume", "Volume"),
            Edit::FadeIn(_) | Edit::FadeOut(_) => ("fade", "Audio fade"),
            Edit::Speed(_) => ("speed", "Speed"),
            Edit::Text(_) => ("text", "Edit text"),
            Edit::Font(_) | Edit::FontSize(_) | Edit::Weight(_) | Edit::TextColor(_) | Edit::Align(_) | Edit::TextBox(_) => ("text-style", "Text style"),
            Edit::Intro(_) | Edit::Outro(_) | Edit::IntroDuration(_) | Edit::OutroDuration(_) => ("motion", "Animation"),
            Edit::Transition(_) | Edit::TransitionDuration(_) => ("transition", "Transition"),
            Edit::AddZoom => ("add-zoom", "Add zoom"),
            Edit::RemoveZoom(_) => ("remove-zoom", "Remove zoom"),
            Edit::ZoomScale(..) | Edit::ZoomFocusX(..) | Edit::ZoomFocusY(..) | Edit::ZoomFocus(..) => ("zoom", "Zoom"),
        }
    }

    /// Applies edits that only touch the clip itself.
    pub fn apply(&self, c: &mut Clip) {
        let t = &mut c.transform;
        let l = &mut c.look;
        let g = &mut c.grade;
        match self.clone() {
            Edit::Name(s) => c.name = (!s.trim().is_empty()).then(|| s.trim().to_string()),
            Edit::X(v) => t.x = v,
            Edit::Y(v) => t.y = v,
            Edit::Position(x, y) => {
                t.x = x;
                t.y = y;
            }
            Edit::Scale(v) => t.scale = v,
            Edit::Rotation(v) => t.rotation = v,
            Edit::Opacity(v) => t.opacity = v,
            Edit::Fit(f) => t.fit = f,
            Edit::ResetTransform => c.transform = crate::clip::Transform { fit: t.fit, ..Default::default() },
            Edit::Radius(v) => l.corner_radius = v,
            Edit::BorderWidth(v) => l.border_width = v,
            Edit::BorderColor(col) => l.border_color = col,
            Edit::Shadow(on) => l.shadow.enabled = on,
            Edit::ShadowBlur(v) => l.shadow.blur = v,
            Edit::ShadowOffset(v) => l.shadow.offset_y = v,
            Edit::ShadowOpacity(v) => l.shadow.opacity = v,
            Edit::Brightness(v) => g.brightness = v,
            Edit::Contrast(v) => g.contrast = v,
            Edit::Saturation(v) => g.saturation = v,
            Edit::Temperature(v) => g.temperature = v,
            Edit::ResetGrade => c.grade = Default::default(),
            Edit::Volume(v) => c.audio.volume = v,
            Edit::Mute(m) => c.audio.muted = m,
            Edit::FadeIn(v) => c.audio.fade_in = v,
            Edit::FadeOut(v) => c.audio.fade_out = v,
            Edit::Intro(k) => c.intro = k.map(|kind| crate::clip::Motion { kind, duration: c.intro.map(|m| m.duration).unwrap_or(0.6) }),
            Edit::Outro(k) => c.outro = k.map(|kind| crate::clip::Motion { kind, duration: c.outro.map(|m| m.duration).unwrap_or(0.6) }),
            Edit::IntroDuration(d) => {
                if let Some(m) = &mut c.intro {
                    m.duration = d;
                }
            }
            Edit::OutroDuration(d) => {
                if let Some(m) = &mut c.outro {
                    m.duration = d;
                }
            }
            Edit::Transition(k) => c.transition = k.map(|kind| crate::clip::Transition { kind, duration: c.transition.map(|t| t.duration).unwrap_or(0.6) }),
            Edit::TransitionDuration(d) => {
                if let Some(t) = &mut c.transition {
                    t.duration = d;
                }
            }
            Edit::RemoveZoom(id) => c.zooms.retain(|z| z.id != id),
            Edit::ZoomScale(id, v) => c.zooms.iter_mut().filter(|z| z.id == id).for_each(|z| z.scale = v),
            Edit::ZoomFocusX(id, v) => c.zooms.iter_mut().filter(|z| z.id == id).for_each(|z| z.focus_x = v),
            Edit::ZoomFocusY(id, v) => c.zooms.iter_mut().filter(|z| z.id == id).for_each(|z| z.focus_y = v),
            Edit::ZoomFocus(id, x, y) => c.zooms.iter_mut().filter(|z| z.id == id).for_each(|z| {
                z.focus_x = x;
                z.focus_y = y;
            }),
            edit => {
                if let Content::Text(tl) = &mut c.content {
                    match edit {
                        Edit::Text(s) => tl.text = s,
                        Edit::Font(f) => tl.font = f,
                        Edit::FontSize(v) => tl.size = v,
                        Edit::Weight(w) => tl.weight = w,
                        Edit::TextColor(col) => tl.color = col,
                        Edit::Align(a) => tl.align = a,
                        Edit::TextBox(on) => tl.background = on.then_some(Rgba(0.0, 0.0, 0.0, 0.55)),
                        _ => {}
                    }
                }
            }
        }
    }
}

pub const FONTS: [&str; 3] = ["Inter", "Poppins", "Hind Siliguri"];

const PALETTE: [u32; 8] = [0xFFFFFF, 0x0B0B0F, 0x7C6DFF, 0xC56BFF, 0x4C8DFF, 0x34D399, 0xFBBF24, 0xF26B6B];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MotionChoice(Option<MotionKind>);

impl std::fmt::Display for MotionChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self.0 {
            None => "None",
            Some(MotionKind::Fade) => "Fade",
            Some(MotionKind::SlideUp) => "Slide up",
            Some(MotionKind::SlideDown) => "Slide down",
            Some(MotionKind::SlideLeft) => "Slide left",
            Some(MotionKind::SlideRight) => "Slide right",
            Some(MotionKind::Pop) => "Pop",
            Some(MotionKind::ZoomIn) => "Zoom in",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TransitionChoice(Option<TransitionKind>);

impl std::fmt::Display for TransitionChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self.0 {
            None => "None",
            Some(TransitionKind::Crossfade) => "Crossfade",
            Some(TransitionKind::SlideLeft) => "Slide",
            Some(TransitionKind::SlideUp) => "Slide up",
            Some(TransitionKind::Push) => "Push",
            Some(TransitionKind::ZoomThrough) => "Zoom through",
            Some(TransitionKind::Wipe) => "Wipe",
        })
    }
}

const TRANSITIONS: [TransitionChoice; 7] = [
    TransitionChoice(None),
    TransitionChoice(Some(TransitionKind::Crossfade)),
    TransitionChoice(Some(TransitionKind::SlideLeft)),
    TransitionChoice(Some(TransitionKind::SlideUp)),
    TransitionChoice(Some(TransitionKind::Push)),
    TransitionChoice(Some(TransitionKind::ZoomThrough)),
    TransitionChoice(Some(TransitionKind::Wipe)),
];

const MOTIONS: [MotionChoice; 8] = [
    MotionChoice(None),
    MotionChoice(Some(MotionKind::Fade)),
    MotionChoice(Some(MotionKind::SlideUp)),
    MotionChoice(Some(MotionKind::SlideDown)),
    MotionChoice(Some(MotionKind::SlideLeft)),
    MotionChoice(Some(MotionKind::SlideRight)),
    MotionChoice(Some(MotionKind::Pop)),
    MotionChoice(Some(MotionKind::ZoomIn)),
];

pub fn view<'a>(project: &'a Project, clip: &'a Clip, zoom_edit: Option<Id>) -> Element<'a, Message> {
    let id = clip.id;
    let e = move |edit: Edit| Message::Edit(id, edit);
    let kind = clip.asset().and_then(|a| project.asset(a)).map(|a| a.kind);
    let is_visual = !matches!(kind, Some(MediaKind::Audio));
    let has_audio = clip.asset().and_then(|a| project.asset(a)).is_some_and(|a| a.has_audio);
    let is_video = kind == Some(MediaKind::Video);

    let (glyph, color, kind_label) = match (&clip.content, kind) {
        (Content::Text(_), _) => (code::TYPE, clip::TEXT, "Text"),
        (Content::Shape(_), _) => (code::SQUARE, clip::EFFECT, "Shape"),
        (_, Some(MediaKind::Audio)) => (code::AUDIO_LINES, clip::AUDIO, "Audio"),
        (_, Some(MediaKind::Image)) => (code::IMAGE, clip::IMAGE, "Image"),
        _ => (code::FILM, clip::VIDEO, "Video"),
    };
    let header = container(
        row![
            container(tinted(glyph, 15, color)).center(32).style(style::badge(color)),
            column![
                text_input("Clip name", &clip.name.clone().unwrap_or_else(|| clip_label(project, clip)))
                    .on_input(move |s| e(Edit::Name(s)))
                    .font(INTER_SEMIBOLD)
                    .size(T_BODY)
                    .padding([3, 6])
                    .style(|t, s| {
                        let mut st = style::input(t, s);
                        if matches!(s, text_input::Status::Active) {
                            st.border.color = Color::TRANSPARENT;
                            st.background = Color::TRANSPARENT.into();
                        }
                        st
                    }),
                text(format!("{kind_label} · {:.2}s at {}", clip.duration, super::fmt_timecode(clip.start))).size(T_CAPTION).color(TEXT_3),
            ]
            .spacing(2)
            .width(Fill),
        ]
        .spacing(S3)
        .align_y(Alignment::Center),
    )
    .padding([S3, S3]);

    let mut col = column![header].spacing(S2);

    if let Content::Text(t) = &clip.content {
        let font_pick = pick_list(&FONTS[..], FONTS.iter().find(|f| **f == t.font).copied(), move |f: &str| e(Edit::Font(f.to_string())))
            .padding([6, 9])
            .text_size(T_BODY)
            .width(Fill)
            .style(style::select)
            .menu_style(style::select_menu);
        let weights = [(400u16, "Regular"), (500, "Medium"), (600, "Semibold"), (700, "Bold")];
        let aligns = [Align::Left, Align::Center, Align::Right];
        col = col.push(section(
            code::TYPE,
            "Text",
            column![
                text_input("Type your text…", &t.text).on_input(move |s| e(Edit::Text(s))).padding([8, 10]).size(T_BODY).font(if t.text.is_ascii() { INTER } else { HIND }).style(style::input),
                row![container(font_pick).width(Length::FillPortion(3)), container(widgets::segmented(&aligns, t.align, |a| match a {
                    Align::Left => "Left".into(),
                    Align::Center => "Center".into(),
                    Align::Right => "Right".into(),
                }, move |a| e(Edit::Align(a)))).width(Length::FillPortion(4))]
                .spacing(S2)
                .align_y(Alignment::Center),
                widgets::segmented(&weights.map(|w| w.0), t.weight, move |w| weights.iter().find(|x| x.0 == *w).map(|x| x.1).unwrap_or("").to_string(), move |w| e(Edit::Weight(w))),
                slider_row("Size", t.size, 12.0..=400.0, 1.0, |v| format!("{v:.0}"), move |v| e(Edit::FontSize(v))),
                color_row("Color", t.color, move |c| e(Edit::TextColor(c))),
                toggle_row("Background box", t.background.is_some(), move |on| e(Edit::TextBox(on))),
            ]
            .spacing(S3)
            .into(),
        ));
    }

    if is_visual {
        let tr = &clip.transform;
        let mut transform = column![
            slider_row("Position X", tr.x * 100.0, -50.0..=150.0, 0.1, |v| format!("{v:.0}%"), move |v| e(Edit::X(v / 100.0))),
            slider_row("Position Y", tr.y * 100.0, -50.0..=150.0, 0.1, |v| format!("{v:.0}%"), move |v| e(Edit::Y(v / 100.0))),
            slider_row("Scale", tr.scale * 100.0, 5.0..=400.0, 1.0, |v| format!("{v:.0}%"), move |v| e(Edit::Scale(v / 100.0))),
            slider_row("Rotation", tr.rotation, -180.0..=180.0, 0.5, |v| format!("{v:.0}°"), move |v| e(Edit::Rotation(v))),
            slider_row("Opacity", tr.opacity * 100.0, 0.0..=100.0, 1.0, |v| format!("{v:.0}%"), move |v| e(Edit::Opacity(v / 100.0))),
        ]
        .spacing(S3);
        if matches!(clip.content, Content::Media { .. }) {
            transform = transform.push(
                row![
                    text("Framing").size(T_SMALL).color(TEXT_2).width(84),
                    widgets::segmented(&[Fit::Contain, Fit::Cover], tr.fit, |f| if *f == Fit::Contain { "Fit".into() } else { "Fill".into() }, move |f| e(Edit::Fit(f))),
                ]
                .align_y(Alignment::Center),
            );
        }
        col = col.push(section_with_reset(code::MOVE, "Transform", e(Edit::ResetTransform), transform.into()));

        if !matches!(clip.content, Content::Text(_)) {
            let l = &clip.look;
            let mut frame = column![
                slider_row("Corners", l.corner_radius, 0.0..=200.0, 1.0, |v| format!("{v:.0}"), move |v| e(Edit::Radius(v))),
                slider_row("Border", l.border_width, 0.0..=40.0, 0.5, |v| format!("{v:.1}"), move |v| e(Edit::BorderWidth(v))),
            ]
            .spacing(S3);
            if l.border_width > 0.0 {
                frame = frame.push(color_row("Border color", l.border_color, move |c| e(Edit::BorderColor(c))));
            }
            frame = frame.push(toggle_row("Drop shadow", l.shadow.enabled, move |on| e(Edit::Shadow(on))));
            if l.shadow.enabled {
                frame = frame
                    .push(slider_row("Blur", l.shadow.blur, 0.0..=200.0, 1.0, |v| format!("{v:.0}"), move |v| e(Edit::ShadowBlur(v))))
                    .push(slider_row("Distance", l.shadow.offset_y, -100.0..=100.0, 1.0, |v| format!("{v:.0}"), move |v| e(Edit::ShadowOffset(v))))
                    .push(slider_row("Strength", l.shadow.opacity * 100.0, 0.0..=100.0, 1.0, |v| format!("{v:.0}%"), move |v| e(Edit::ShadowOpacity(v / 100.0))));
            }
            col = col.push(section(code::SQUARE, "Frame style", frame.into()));
        }

        let g = &clip.grade;
        col = col.push(section_with_reset(
            code::PALETTE,
            "Color",
            e(Edit::ResetGrade),
            column![
                slider_row("Brightness", g.brightness * 100.0, -100.0..=100.0, 1.0, |v| format!("{v:+.0}"), move |v| e(Edit::Brightness(v / 100.0))),
                slider_row("Contrast", g.contrast * 100.0, -100.0..=100.0, 1.0, |v| format!("{v:+.0}"), move |v| e(Edit::Contrast(v / 100.0))),
                slider_row("Saturation", g.saturation * 100.0, -100.0..=100.0, 1.0, |v| format!("{v:+.0}"), move |v| e(Edit::Saturation(v / 100.0))),
                slider_row("Warmth", g.temperature * 100.0, -100.0..=100.0, 1.0, |v| format!("{v:+.0}"), move |v| e(Edit::Temperature(v / 100.0))),
            ]
            .spacing(S3)
            .into(),
        ));

        let motion_pick = |current: Option<MotionKind>, f: fn(Option<MotionKind>) -> Edit| {
            pick_list(&MOTIONS[..], Some(MotionChoice(current)), move |m: MotionChoice| e(f(m.0)))
                .padding([6, 9])
                .text_size(T_BODY)
                .width(Fill)
                .style(style::select)
                .menu_style(style::select_menu)
        };
        let max_motion = (clip.duration / 2.0).max(0.1);
        let mut anim = column![row![text("In").size(T_SMALL).color(TEXT_2).width(84), motion_pick(clip.intro.map(|m| m.kind), Edit::Intro)].align_y(Alignment::Center)].spacing(S3);
        if let Some(m) = clip.intro {
            anim = anim.push(slider_row("Duration", m.duration, 0.1..=max_motion.max(0.11), 0.05, |v| format!("{v:.2}s"), move |v| e(Edit::IntroDuration(v))));
        }
        anim = anim.push(row![text("Out").size(T_SMALL).color(TEXT_2).width(84), motion_pick(clip.outro.map(|m| m.kind), Edit::Outro)].align_y(Alignment::Center));
        if let Some(m) = clip.outro {
            anim = anim.push(slider_row("Duration", m.duration, 0.1..=max_motion.max(0.11), 0.05, |v| format!("{v:.2}s"), move |v| e(Edit::OutroDuration(v))));
        }
        col = col.push(section(code::SPARKLES, "Animation", anim.into()));

        let transition_pick = pick_list(&TRANSITIONS[..], Some(TransitionChoice(clip.transition.map(|t| t.kind))), move |t: TransitionChoice| e(Edit::Transition(t.0)))
            .padding([6, 9])
            .text_size(T_BODY)
            .width(Fill)
            .style(style::select)
            .menu_style(style::select_menu);
        let mut trans = column![row![text("Style").size(T_SMALL).color(TEXT_2).width(84), transition_pick].align_y(Alignment::Center)].spacing(S3);
        if let Some(t) = clip.transition {
            trans = trans.push(slider_row("Duration", t.duration, 0.1..=max_motion.max(0.11), 0.05, |v| format!("{v:.2}s"), move |v| e(Edit::TransitionDuration(v))));
        }
        col = col.push(section(code::BLEND, "Transition in", trans.into()));
    }

    if is_video || kind == Some(MediaKind::Image) {
        let mut zooms = column![].spacing(S3);
        for (i, z) in clip.zooms.iter().enumerate() {
            let zid = z.id;
            let editing = zoom_edit == Some(zid);
            zooms = zooms.push(
                container(
                    column![
                        row![
                            tinted(code::ZOOM_IN, 13, BRAND_2),
                            text(format!("Zoom {}", i + 1)).font(INTER_SEMIBOLD).size(T_SMALL),
                            text(format!("{:.1}s – {:.1}s", z.at, z.at + z.duration)).size(T_CAPTION).color(TEXT_3),
                            space().width(Fill),
                            widgets::tip(
                                widgets::icon_toggle(code::SCAN, editing, Message::ZoomEdit(if editing { None } else { Some((id, zid)) })).width(24).height(24),
                                if editing { "Done editing" } else { "Edit on canvas" },
                                None,
                                iced::widget::tooltip::Position::Left,
                            ),
                            widgets::icon_button(code::TRASH_2, Some(e(Edit::RemoveZoom(zid)))).width(24).height(24),
                        ]
                        .spacing(6)
                        .align_y(Alignment::Center),
                        slider_row("Zoom", z.scale * 100.0, 100.0..=500.0, 1.0, |v| format!("{v:.0}%"), move |v| e(Edit::ZoomScale(zid, v / 100.0))),
                        slider_row("Focus X", z.focus_x * 100.0, 0.0..=100.0, 0.5, |v| format!("{v:.0}%"), move |v| e(Edit::ZoomFocusX(zid, v / 100.0))),
                        slider_row("Focus Y", z.focus_y * 100.0, 0.0..=100.0, 0.5, |v| format!("{v:.0}%"), move |v| e(Edit::ZoomFocusY(zid, v / 100.0))),
                    ]
                    .spacing(S2),
                )
                .padding(S3)
                .style(move |t| {
                    let mut s = style::inset(t);
                    if editing {
                        s.border = iced::Border { color: BRAND_2, width: 1.5, radius: R_MD.into() };
                    }
                    s
                }),
            );
        }
        zooms = zooms.push(widgets::secondary_button(Some(code::PLUS), "Add zoom at playhead", Some(e(Edit::AddZoom))).width(Fill));
        col = col.push(section(code::ZOOM_IN, "Zoom & pan", zooms.into()));
    }

    if let Content::Media { speed, .. } = clip.content {
        if kind != Some(MediaKind::Image) {
            col = col.push(section(
                code::GAUGE,
                "Speed",
                slider_row("Speed", speed * 100.0, 25.0..=400.0, 5.0, |v| format!("{:.2}×", v / 100.0), move |v| e(Edit::Speed(v / 100.0))),
            ));
        }
    }

    if has_audio {
        let a = &clip.audio;
        col = col.push(section(
            code::VOLUME_2,
            "Audio",
            column![
                slider_row("Volume", a.volume * 100.0, 0.0..=200.0, 1.0, |v| format!("{v:.0}%"), move |v| e(Edit::Volume(v / 100.0))),
                toggle_row("Mute", a.muted, move |m| e(Edit::Mute(m))),
                slider_row("Fade in", a.fade_in, 0.0..=5.0, 0.05, |v| format!("{v:.1}s"), move |v| e(Edit::FadeIn(v))),
                slider_row("Fade out", a.fade_out, 0.0..=5.0, 0.05, |v| format!("{v:.1}s"), move |v| e(Edit::FadeOut(v))),
            ]
            .spacing(S3)
            .into(),
        ));
    }

    widgets::scroll(col.padding(iced::Padding { top: 0.0, right: 10.0, bottom: 12.0, left: 4.0 })).height(Fill).into()
}

fn section<'a>(glyph: char, title: &'a str, body: Element<'a, Message>) -> Element<'a, Message> {
    let head = row![tinted(glyph, 13, TEXT_3), text(title).font(INTER_SEMIBOLD).size(T_SMALL).color(TEXT_2)].spacing(7).align_y(Alignment::Center);
    container(column![head, body].spacing(S3)).padding(S3).width(Fill).style(style::card).into()
}

fn section_with_reset<'a>(glyph: char, title: &'a str, reset: Message, body: Element<'a, Message>) -> Element<'a, Message> {
    let head = row![
        tinted(glyph, 13, TEXT_3),
        text(title).font(INTER_SEMIBOLD).size(T_SMALL).color(TEXT_2),
        space().width(Fill),
        widgets::tip(widgets::icon_button(code::REFRESH_CW, Some(reset)).width(24).height(24), "Reset", None, iced::widget::tooltip::Position::Left),
    ]
    .spacing(7)
    .align_y(Alignment::Center);
    container(column![head, body].spacing(S3)).padding(S3).width(Fill).style(style::card).into()
}

fn slider_row<'a>(name: &'a str, value: f64, range: RangeInclusive<f64>, step: f64, fmt: impl Fn(f64) -> String, on_change: impl Fn(f64) -> Message + 'a) -> Element<'a, Message> {
    let v = value.clamp(*range.start(), *range.end());
    row![
        text(name).size(T_SMALL).color(TEXT_2).width(84),
        slider(range, v, on_change).step(step).style(style::slider).width(Fill),
        container(text(fmt(value)).font(INTER_MEDIUM).size(T_CAPTION).color(TEXT_1)).width(52).align_right(52),
    ]
    .spacing(S2)
    .align_y(Alignment::Center)
    .into()
}

fn toggle_row<'a>(name: &'a str, on: bool, on_toggle: impl Fn(bool) -> Message + 'a) -> Element<'a, Message> {
    row![text(name).size(T_SMALL).color(TEXT_2).width(Fill), toggler(on).on_toggle(on_toggle).size(18).style(style::toggler)].align_y(Alignment::Center).into()
}

fn color_row<'a>(name: &'a str, value: Rgba, on_pick: impl Fn(Rgba) -> Message + Clone + 'a) -> Element<'a, Message> {
    let swatches = row(PALETTE.iter().map(|&hex| {
        let c = Rgba::hex(hex);
        let selected = c.to_hex() == value.to_hex();
        let on_pick = on_pick.clone();
        button(space().width(16).height(16))
            .padding(0)
            .style(move |_, status| button::Style {
                background: Some(Color::from_rgba(c.0, c.1, c.2, 1.0).into()),
                border: iced::Border {
                    color: if selected { Color::WHITE } else if matches!(status, button::Status::Hovered) { LINE_STRONG } else { LINE },
                    width: if selected { 2.0 } else { 1.0 },
                    radius: 9.0.into(),
                },
                ..Default::default()
            })
            .on_press(on_pick(Rgba(c.0, c.1, c.2, value.3)))
            .into()
    }))
    .spacing(5);
    let on_hex = on_pick.clone();
    let hex = text_input("#FFFFFF", &value.to_hex())
        .on_input(move |s| Rgba::parse(&s).map(|c| on_hex(Rgba(c.0, c.1, c.2, value.3))).unwrap_or(Message::Noop))
        .padding([4, 7])
        .size(T_CAPTION)
        .width(74)
        .style(style::input);
    column![row![text(name).size(T_SMALL).color(TEXT_2).width(Fill), hex].align_y(Alignment::Center), swatches].spacing(6).into()
}

pub fn empty<'a>() -> Element<'a, Message> {
    widgets::empty_state(code::MOUSE_POINTER_2, "Nothing selected", "Select a clip on the timeline to edit its position, style, color and animation.", None)
}

pub fn multiple<'a>(count: usize) -> Element<'a, Message> {
    let _ = icon(code::LAYERS, 1);
    widgets::empty_state(code::LAYERS, "Multiple clips", if count == 2 { "2 clips selected. Select one clip to edit it." } else { "Several clips selected. Select one clip to edit it." }, None)
}
