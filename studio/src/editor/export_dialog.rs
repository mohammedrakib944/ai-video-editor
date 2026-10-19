//! The export dialog: pick a file and quality, watch progress, open the result.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use iced::widget::{column, container, mouse_area, opaque, row, space, text, text_input};
use iced::{Alignment, Element, Fill, Length};

use super::Message;
use crate::model::Project;
use crate::render::export::{self, Quality};
use crate::ui::icons::{code, icon, tinted};
use crate::ui::tokens::*;
use crate::ui::{style, widgets};

pub enum Phase {
    Setup,
    Running { done: u64, total: u64, eta: Option<f64>, cancel: Arc<AtomicBool> },
    Done { path: PathBuf, seconds: f64 },
    Failed(String),
}

pub struct ExportDialog {
    pub path: String,
    pub quality: Quality,
    pub phase: Phase,
}

impl ExportDialog {
    pub fn new(project: &Project, project_path: Option<&std::path::Path>) -> ExportDialog {
        let dir = project_path.and_then(|p| p.parent()).map(|p| p.to_path_buf()).unwrap_or_else(crate::settings::projects_dir);
        let safe: String = project.name.chars().map(|c| if r#"<>:"/\|?*"#.contains(c) { '_' } else { c }).collect();
        let mut path = dir.join(format!("{safe}.mp4"));
        let mut n = 2;
        while path.exists() {
            path = dir.join(format!("{safe} ({n}).mp4"));
            n += 1;
        }
        ExportDialog { path: path.display().to_string(), quality: Quality::High, phase: Phase::Setup }
    }

    pub fn is_running(&self) -> bool {
        matches!(self.phase, Phase::Running { .. })
    }

    pub fn view<'a>(&'a self, project: &'a Project) -> Element<'a, Message> {
        let body: Element<'a, Message> = match &self.phase {
            Phase::Setup => self.setup(project),
            Phase::Running { done, total, eta, .. } => running(*done, *total, *eta),
            Phase::Done { path, seconds } => done(path, *seconds),
            Phase::Failed(err) => failed(err),
        };
        let card = container(body).width(460).padding(S6).style(style::popover);
        let scrim = mouse_area(container(space().width(Fill).height(Fill)).style(style::scrim)).on_press(if self.is_running() { Message::Noop } else { Message::CloseExport });
        iced::widget::stack![scrim, container(opaque(card)).center(Fill)].into()
    }

    fn setup<'a>(&'a self, project: &'a Project) -> Element<'a, Message> {
        let (w, h) = project.canvas.size();
        let duration = project.duration();
        let mb = export::estimate_mb((w, h), project.canvas.fps, self.quality, duration);
        let chip = |glyph: char, label: String| container(row![tinted(glyph, 12, TEXT_3), text(label).size(T_SMALL).color(TEXT_2)].spacing(6).align_y(Alignment::Center)).padding([5, 9]).style(style::segmented_track);
        let valid = !self.path.trim().is_empty() && PathBuf::from(self.path.trim()).parent().is_some_and(|d| d.as_os_str().is_empty() || d.is_dir());
        column![
            row![
                container(icon(code::DOWNLOAD, 16).color(ON_ACCENT)).center(34).style(style::logo),
                column![text("Export video").font(INTER_SEMIBOLD).size(T_TITLE), text("MP4 · H.264 video with AAC audio").size(T_SMALL).color(TEXT_3)].spacing(2),
            ]
            .spacing(S3)
            .align_y(Alignment::Center),
            row![
                chip(code::MONITOR, format!("{w} × {h}")),
                chip(code::GAUGE, format!("{} fps", project.canvas.fps)),
                chip(code::CLOCK, super::fmt_timecode(duration)),
                chip(code::HARD_DRIVE, if mb < 1000.0 { format!("~{mb:.0} MB") } else { format!("~{:.1} GB", mb / 1024.0) }),
            ]
            .spacing(6),
            widgets::field(
                "Save to",
                row![
                    text_input("File path", &self.path).on_input(Message::ExportPath).padding([8, 10]).size(T_BODY).style(style::input),
                    widgets::secondary_button(Some(code::FOLDER_OPEN), "Browse", Some(Message::ExportBrowse)),
                ]
                .spacing(S2)
                .align_y(Alignment::Center)
            ),
            widgets::field("Quality", widgets::segmented(&Quality::ALL, self.quality, |q| q.label().to_string(), Message::ExportQuality)),
            row![
                space().width(Fill),
                widgets::ghost_button(None, "Cancel", Some(Message::CloseExport)),
                widgets::primary_button(Some(code::SPARKLES), "Export", (valid && duration > 0.0).then_some(Message::StartExport)),
            ]
            .spacing(S2)
            .align_y(Alignment::Center),
        ]
        .spacing(S5)
        .into()
    }
}

fn progress_bar<'a>(fraction: f32) -> Element<'a, Message> {
    let fill = container(space().height(6)).width(Length::FillPortion(((fraction.clamp(0.0, 1.0)) * 1000.0) as u16 + 1)).style(|_| container::Style {
        background: Some(style::brand_gradient()),
        border: iced::Border { radius: 3.0.into(), ..Default::default() },
        ..Default::default()
    });
    let rest = space().width(Length::FillPortion((((1.0 - fraction.clamp(0.0, 1.0)) * 1000.0) as u16).max(1)));
    container(row![fill, rest]).width(Fill).style(|_| container::Style {
        background: Some(SURFACE_0.into()),
        border: iced::Border { radius: 3.0.into(), color: LINE, width: 1.0 },
        ..Default::default()
    })
    .into()
}

fn running<'a>(done: u64, total: u64, eta: Option<f64>) -> Element<'a, Message> {
    let f = done as f32 / total.max(1) as f32;
    let eta = match eta {
        Some(s) if s >= 60.0 => format!("about {} min {} s left", (s / 60.0) as u64, s as u64 % 60),
        Some(s) => format!("about {} s left", s.ceil() as u64),
        None => "estimating time…".into(),
    };
    column![
        row![iced_aw::Spinner::new().width(Length::Fixed(18.0)).height(Length::Fixed(18.0)).circle_radius(2.0), text("Rendering your video").font(INTER_SEMIBOLD).size(T_TITLE)].spacing(S3).align_y(Alignment::Center),
        progress_bar(f),
        row![text(format!("{:.0}%", f * 100.0)).font(INTER_SEMIBOLD).size(T_BODY), text(format!("·  frame {done} of {total}  ·  {eta}")).size(T_SMALL).color(TEXT_3)].spacing(6).align_y(Alignment::Center),
        row![space().width(Fill), widgets::secondary_button(Some(code::X), "Cancel export", Some(Message::CancelExport))],
    ]
    .spacing(S4)
    .into()
}

fn done<'a>(path: &'a std::path::Path, seconds: f64) -> Element<'a, Message> {
    column![
        row![
            container(icon(code::CIRCLE_CHECK, 18).color(SUCCESS)).center(34).style(style::badge(SUCCESS)),
            column![
                text("Your video is ready").font(INTER_SEMIBOLD).size(T_TITLE),
                text(format!("Rendered in {}", if seconds >= 60.0 { format!("{} min {} s", (seconds / 60.0) as u64, seconds as u64 % 60) } else { format!("{seconds:.0} s") })).size(T_SMALL).color(TEXT_3),
            ]
            .spacing(2),
        ]
        .spacing(S3)
        .align_y(Alignment::Center),
        container(text(path.display().to_string()).size(T_SMALL).color(TEXT_2)).padding([8, 10]).width(Fill).style(style::inset),
        row![
            widgets::secondary_button(Some(code::FOLDER_OPEN), "Show in folder", Some(Message::RevealExport)),
            widgets::secondary_button(Some(code::PLAY), "Play", Some(Message::PlayExport)),
            space().width(Fill),
            widgets::primary_button(None, "Done", Some(Message::CloseExport)),
        ]
        .spacing(S2),
    ]
    .spacing(S4)
    .into()
}

fn failed<'a>(err: &'a str) -> Element<'a, Message> {
    column![
        row![
            container(icon(code::TRIANGLE_ALERT, 18).color(DANGER)).center(34).style(style::badge(DANGER)),
            text("Export didn't finish").font(INTER_SEMIBOLD).size(T_TITLE),
        ]
        .spacing(S3)
        .align_y(Alignment::Center),
        container(text(err).size(T_SMALL).color(TEXT_2)).padding([8, 10]).width(Fill).style(style::inset),
        row![space().width(Fill), widgets::secondary_button(None, "Back", Some(Message::ExportRetry)), widgets::primary_button(None, "Close", Some(Message::CloseExport))].spacing(S2),
    ]
    .spacing(S4)
    .into()
}

