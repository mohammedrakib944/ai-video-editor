//! The editor screen: library, preview, inspector and timeline in resizable panes.

use std::path::PathBuf;

use iced::widget::{button, column, container, pane_grid, responsive, row, space, text};
use iced::{Alignment, Color, Element, Fill, Length, Size};

use crate::model::{Project, TrackKind};
use crate::settings::Layout;
use crate::ui::icons::{code, icon, tinted};
use crate::ui::tokens::*;
use crate::ui::{style, widgets};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Library,
    Preview,
    Inspector,
    Timeline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryTab {
    Media,
    Text,
    Transitions,
    Sounds,
}

impl LibraryTab {
    const ALL: [LibraryTab; 4] = [LibraryTab::Media, LibraryTab::Text, LibraryTab::Transitions, LibraryTab::Sounds];

    fn label(self) -> &'static str {
        match self {
            LibraryTab::Media => "Media",
            LibraryTab::Text => "Text",
            LibraryTab::Transitions => "Effects",
            LibraryTab::Sounds => "Sounds",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideTab {
    Inspector,
    Assistant,
}

/// Timeline tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Select,
    Blade,
    Range,
}

#[derive(Debug, Clone)]
pub enum Message {
    PaneResized(pane_grid::ResizeEvent),
    LibraryTab(LibraryTab),
    SideTab(SideTab),
    Tool(Tool),
    TogglePlay,
    ToggleSnapping,
    Import,
}

pub struct Editor {
    pub project: Project,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    panes: pane_grid::State<Pane>,
    library_tab: LibraryTab,
    pub side_tab: SideTab,
    tool: Tool,
    playing: bool,
    snapping: bool,
    playhead: f64,
}

impl Editor {
    pub fn new(project: Project, path: Option<PathBuf>, layout: Layout) -> Editor {
        use pane_grid::{Axis, Configuration as C};
        let config = C::Split {
            axis: Axis::Horizontal,
            ratio: layout.top_bottom,
            a: Box::new(C::Split {
                axis: Axis::Vertical,
                ratio: layout.library,
                a: Box::new(C::Pane(Pane::Library)),
                b: Box::new(C::Split {
                    axis: Axis::Vertical,
                    ratio: layout.preview_inspector,
                    a: Box::new(C::Pane(Pane::Preview)),
                    b: Box::new(C::Pane(Pane::Inspector)),
                }),
            }),
            b: Box::new(C::Pane(Pane::Timeline)),
        };
        Editor {
            project,
            path,
            dirty: false,
            panes: pane_grid::State::with_configuration(config),
            library_tab: LibraryTab::Media,
            side_tab: SideTab::Inspector,
            tool: Tool::Select,
            playing: false,
            snapping: true,
            playhead: 0.0,
        }
    }

    /// Current split ratios, for saving the layout.
    pub fn layout(&self) -> Layout {
        use pane_grid::Node;
        let mut l = Layout::default();
        if let Node::Split { ratio, a, .. } = self.panes.layout() {
            l.top_bottom = *ratio;
            if let Node::Split { ratio, b, .. } = a.as_ref() {
                l.library = *ratio;
                if let Node::Split { ratio, .. } = b.as_ref() {
                    l.preview_inspector = *ratio;
                }
            }
        }
        l
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::PaneResized(pane_grid::ResizeEvent { split, ratio }) => self.panes.resize(split, ratio),
            Message::LibraryTab(t) => self.library_tab = t,
            Message::SideTab(t) => self.side_tab = t,
            Message::Tool(t) => self.tool = t,
            Message::TogglePlay => self.playing = !self.playing,
            Message::ToggleSnapping => self.snapping = !self.snapping,
            Message::Import => {}
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let grid = pane_grid(&self.panes, |_, pane, _| {
            let (header, body): (Element<'_, Message>, Element<'_, Message>) = match pane {
                Pane::Library => (self.library_header(), self.library()),
                Pane::Preview => (pane_title("Preview", Some(self.project.canvas.summary())), self.preview()),
                Pane::Inspector => (self.side_header(), self.inspector()),
                Pane::Timeline => (self.timeline_header(), self.timeline()),
            };
            pane_grid::Content::new(container(body).width(Fill).height(Fill))
                .title_bar(pane_grid::TitleBar::new(header).padding([0, 6]).style(style::panel_header))
                .style(style::panel)
        })
        .spacing(5)
        .min_size(160)
        .on_resize(10, Message::PaneResized)
        .style(|_| pane_grid::Style {
            hovered_region: pane_grid::Highlight { background: ACCENT_SOFT.into(), border: iced::Border { color: ACCENT, width: 1.0, radius: R_LG.into() } },
            picked_split: pane_grid::Line { color: ACCENT, width: 2.0 },
            hovered_split: pane_grid::Line { color: Color { a: 0.55, ..ACCENT }, width: 2.0 },
        });
        container(grid).padding(5).width(Fill).height(Fill).into()
    }

    fn library_header(&self) -> Element<'_, Message> {
        let tabs = row(LibraryTab::ALL.iter().map(|&t| tab(t.label(), t == self.library_tab, Message::LibraryTab(t)))).spacing(2);
        header_row(tabs.into(), widgets::tip(widgets::icon_button(code::PLUS, Some(Message::Import)), "Import media", Some("Ctrl+I"), tip_pos()))
    }

    fn side_header(&self) -> Element<'_, Message> {
        let tabs = row![
            tab("Inspector", self.side_tab == SideTab::Inspector, Message::SideTab(SideTab::Inspector)),
            tab_icon(code::SPARKLES, "Assistant", self.side_tab == SideTab::Assistant, Message::SideTab(SideTab::Assistant)),
        ]
        .spacing(2);
        header_row(tabs.into(), space().into())
    }

    fn library(&self) -> Element<'_, Message> {
        match self.library_tab {
            LibraryTab::Media => widgets::empty_state(
                code::UPLOAD,
                "Bring in your media",
                "Drop screen recordings, face-cam videos, voiceovers, images or logos here.",
                Some(widgets::primary_button(Some(code::PLUS), "Import media", Some(Message::Import)).into()),
            ),
            LibraryTab::Text => widgets::empty_state(code::TYPE, "Text styles", "Titles, lower thirds and captions — coming with the motion engine.", None),
            LibraryTab::Transitions => widgets::empty_state(code::BLEND, "Effects & transitions", "Fades, slides, zooms and animated reveals.", None),
            LibraryTab::Sounds => widgets::empty_state(code::AUDIO_WAVEFORM, "Sound effects", "Whooshes, pops, clicks and risers you can drag onto the timeline.", None),
        }
    }

    fn preview(&self) -> Element<'_, Message> {
        let (cw, ch) = self.project.canvas.size();
        let stage = responsive(move |size: Size| {
            let fit = (size.width / cw as f32).min(size.height / ch as f32).min(1.0);
            let (w, h) = ((cw as f32 * fit).floor(), (ch as f32 * fit).floor());
            let frame = container(
                column![tinted(code::CLAPPERBOARD, 22, TEXT_4), text("Your canvas").size(T_SMALL).color(TEXT_4)].spacing(6).align_x(Alignment::Center),
            )
            .center_x(w)
            .center_y(h)
            .style(|_| container::Style {
                background: Some(Color::BLACK.into()),
                border: iced::Border { color: LINE, width: 1.0, radius: 2.0.into() },
                shadow: iced::Shadow { color: Color { a: 0.5, ..Color::BLACK }, offset: iced::Vector::new(0.0, 6.0), blur_radius: 24.0 },
                ..Default::default()
            });
            container(frame).center(Fill).into()
        });
        let transport = row![
            text(fmt_timecode(self.playhead)).font(INTER_MEDIUM).size(T_BODY),
            text(" / 00:00:00").size(T_BODY).color(TEXT_4),
            space().width(Fill),
            widgets::tip(widgets::icon_button(code::SKIP_BACK, Some(Message::TogglePlay)), "Go to start", Some("Home"), tip_pos()),
            widgets::tip(play_button(self.playing), if self.playing { "Pause" } else { "Play" }, Some("Space"), tip_pos()),
            widgets::tip(widgets::icon_button(code::SKIP_FORWARD, Some(Message::TogglePlay)), "Go to end", Some("End"), tip_pos()),
            space().width(Fill),
            widgets::tip(widgets::icon_button(code::VOLUME_2, None), "Mute preview", Some("M"), tip_pos()),
            widgets::tip(widgets::icon_button(code::MAXIMIZE_2, None), "Full screen", Some("F"), tip_pos()),
        ]
        .spacing(4)
        .align_y(Alignment::Center);
        column![container(stage).padding([10, 14]).height(Fill), container(transport).padding([6, 12])].into()
    }

    fn inspector(&self) -> Element<'_, Message> {
        match self.side_tab {
            SideTab::Inspector => widgets::empty_state(code::MOUSE_POINTER_2, "Nothing selected", "Select a clip on the timeline to edit its position, style, color and animation.", None),
            SideTab::Assistant => widgets::empty_state(
                code::SPARKLES,
                "Assistant",
                "Select part of the timeline and describe the animation you want. Uses your Claude Code login.",
                None,
            ),
        }
    }

    fn timeline_header(&self) -> Element<'_, Message> {
        let tools = row![
            widgets::tip(widgets::icon_toggle(code::MOUSE_POINTER_2, self.tool == Tool::Select, Message::Tool(Tool::Select)), "Select", Some("V"), tip_pos()),
            widgets::tip(widgets::icon_toggle(code::SCISSORS, self.tool == Tool::Blade, Message::Tool(Tool::Blade)), "Blade — split clips", Some("B"), tip_pos()),
            widgets::tip(
                widgets::icon_toggle(code::SQUARE_DASHED, self.tool == Tool::Range, Message::Tool(Tool::Range)),
                "Range — select a part to animate",
                Some("R"),
                tip_pos(),
            ),
            container(widgets::vdivider(16.0)).padding([0, 4]),
            widgets::tip(widgets::icon_toggle(code::MAGNET, self.snapping, Message::ToggleSnapping), "Snapping", Some("N"), tip_pos()),
        ]
        .spacing(2)
        .align_y(Alignment::Center);
        let zoom = row![widgets::icon_button(code::ZOOM_OUT, None), widgets::icon_button(code::ZOOM_IN, None)].spacing(2);
        header_row(row![text("Timeline").font(INTER_SEMIBOLD).size(T_SMALL).color(TEXT_2), container(tools).padding([0, 10])].align_y(Alignment::Center).into(), zoom.into())
    }

    fn timeline(&self) -> Element<'_, Message> {
        let headers = column(self.project.tracks.iter().map(|t| {
            let (glyph, color) = match t.kind {
                TrackKind::Visual => (code::LAYERS, clip::VIDEO),
                TrackKind::Audio => (code::AUDIO_LINES, clip::AUDIO),
            };
            container(
                row![
                    tinted(glyph, 13, color),
                    text(&t.name).font(INTER_MEDIUM).size(T_SMALL).width(Fill).wrapping(text::Wrapping::None),
                    tinted(if t.hidden { code::EYE_OFF } else { code::EYE }, 13, TEXT_4),
                    tinted(if t.locked { code::LOCK } else { code::LOCK_OPEN }, 13, TEXT_4),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            )
            .padding([0, 10])
            .height(44)
            .center_y(44)
            .style(style::card)
            .into()
        }))
        .spacing(4)
        .width(196);
        let lanes = container(widgets::empty_state(code::FILM, "Your timeline is empty", "Import media and drag it here, or double-click a file in the library.", None))
            .width(Fill)
            .height(Fill)
            .style(style::inset);
        row![container(headers).padding([8, 8]), container(lanes).padding(iced::Padding { top: 8.0, right: 8.0, bottom: 8.0, left: 0.0 })].into()
    }
}

fn tip_pos() -> iced::widget::tooltip::Position {
    iced::widget::tooltip::Position::Top
}

fn pane_title<'a>(title: &'a str, detail: Option<String>) -> Element<'a, Message> {
    let mut r = row![text(title).font(INTER_SEMIBOLD).size(T_SMALL).color(TEXT_2)].spacing(S2).align_y(Alignment::Center);
    if let Some(d) = detail {
        r = r.push(text(d).size(T_CAPTION).color(TEXT_4));
    }
    container(r).height(38).padding([0, 8]).center_y(38).into()
}

fn header_row<'a>(left: Element<'a, Message>, right: Element<'a, Message>) -> Element<'a, Message> {
    container(row![left, space().width(Fill), right].align_y(Alignment::Center)).height(38).padding([0, 2]).center_y(38).into()
}

fn tab<'a>(label: &'a str, active: bool, on_press: Message) -> Element<'a, Message> {
    button(text(label).font(INTER_MEDIUM).size(T_SMALL)).padding([5, 10]).style(style::segment(active)).on_press(on_press).into()
}

fn tab_icon<'a>(glyph: char, label: &'a str, active: bool, on_press: Message) -> Element<'a, Message> {
    let content = row![icon(glyph, 12), text(label).font(INTER_MEDIUM).size(T_SMALL)].spacing(5).align_y(Alignment::Center);
    button(content).padding([5, 10]).style(style::segment(active)).on_press(on_press).into()
}

fn play_button<'a>(playing: bool) -> Element<'a, Message> {
    button(container(icon(if playing { code::PAUSE } else { code::PLAY }, 15)).center(Length::Fill))
        .width(38)
        .height(38)
        .padding(0)
        .style(style::primary)
        .on_press(Message::TogglePlay)
        .into()
}

pub fn fmt_timecode(t: f64) -> String {
    let t = t.max(0.0);
    let total = t.floor() as u64;
    format!("{:02}:{:02}:{:02}", total / 3600, (total / 60) % 60, total % 60)
}
