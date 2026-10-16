//! UI prototype for the Iced rewrite. Tests: custom title bar, resizable panes,
//! Bangla shaping and typing, bundled fonts, 60 fps GPU video, a large timeline.

mod theme;
mod timeline;
mod video;

use std::collections::VecDeque;
use std::fmt;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use iced::widget::canvas::{self, Frame, Geometry, Path as CanvasPath, Stroke};
use iced::widget::{
    button, column, container, mouse_area, pane_grid, pick_list, pin, row, scrollable, shader, slider, space, stack, text, text_input,
};
use iced::window::{self, Direction};
use iced::{Alignment, Color, Element, Event, Fill, Font, Length, Point, Rectangle, Renderer, Size, Subscription, Task, Theme, event, mouse};

use crate::timeline::Timeline;

fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title("AI Video Editor — UI prototype")
        .theme(theme::theme())
        .subscription(App::subscription)
        .font(include_bytes!("../fonts/Poppins-Regular.ttf").as_slice())
        .font(include_bytes!("../fonts/Poppins-Medium.ttf").as_slice())
        .font(include_bytes!("../fonts/Poppins-SemiBold.ttf").as_slice())
        .font(include_bytes!("../fonts/Inter[opsz,wght].ttf").as_slice())
        .font(include_bytes!("../fonts/Roboto[wdth,wght].ttf").as_slice())
        .font(include_bytes!("../fonts/HindSiliguri-Regular.ttf").as_slice())
        .font(include_bytes!("../fonts/HindSiliguri-SemiBold.ttf").as_slice())
        .default_font(theme::POPPINS)
        .window(window::Settings {
            size: Size::new(1440.0, 900.0),
            min_size: Some(Size::new(960.0, 600.0)),
            decorations: false,
            ..Default::default()
        })
        .run()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaneKind {
    Library,
    Preview,
    Inspector,
    Timeline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontChoice {
    Poppins,
    Inter,
    Roboto,
    HindSiliguri,
}

impl FontChoice {
    const ALL: [FontChoice; 4] = [FontChoice::Poppins, FontChoice::Inter, FontChoice::Roboto, FontChoice::HindSiliguri];

    fn font(self) -> Font {
        match self {
            FontChoice::Poppins => theme::POPPINS,
            FontChoice::Inter => theme::INTER,
            FontChoice::Roboto => theme::ROBOTO,
            FontChoice::HindSiliguri => theme::HIND,
        }
    }
}

impl fmt::Display for FontChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            FontChoice::Poppins => "Poppins",
            FontChoice::Inter => "Inter",
            FontChoice::Roboto => "Roboto",
            FontChoice::HindSiliguri => "Hind Siliguri",
        })
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    DragWindow,
    ToggleMaximize,
    Minimize,
    Close,
    ResizeWindow(Direction),
    PaneResized(pane_grid::ResizeEvent),
    Tick(Instant),
    TogglePlay,
    Seek(f64),
    FontPicked(FontChoice),
    SampleChanged(String),
    SizeChanged(f32),
    CursorMoved(Point),
    TrackMenu(usize),
    CloseMenu,
    StartRename(usize),
    RenameInput(String),
    FinishRename,
    AddTrack,
    DeleteTrack(usize),
    Noop,
}

struct Playback {
    decoder: video::Decoder,
    frame: Option<Arc<video::Frame>>,
    pending: Option<video::Frame>,
    playing: bool,
    base: f64,
    resumed: Instant,
    shown: u64,
    dropped: u64,
}

impl Playback {
    fn media_time(&self, now: Instant) -> f64 {
        if self.playing { self.base + now.duration_since(self.resumed).as_secs_f64() } else { self.base }
    }

    /// Shows the newest decoded frame that is due; frames that arrive too late are counted as dropped.
    fn advance(&mut self, now: Instant) {
        let due = (self.media_time(now) * self.decoder.fps) as u64;
        loop {
            let Some(f) = self.pending.take().or_else(|| self.decoder.try_next()) else { break };
            if f.index > due && self.frame.is_some() {
                self.pending = Some(f);
                break;
            }
            if let Some(prev) = &self.frame {
                self.dropped += f.index.saturating_sub(prev.index + 1);
            }
            self.shown += 1;
            self.frame = Some(Arc::new(f));
        }
    }
}

struct App {
    panes: pane_grid::State<PaneKind>,
    maximized: bool,
    playback: Result<Playback, String>,
    ticks: VecDeque<Instant>,
    font_choice: FontChoice,
    sample: String,
    sample_size: f32,
    timeline: Timeline,
    timeline_offset: f64,
    selected_track: Option<usize>,
    menu: Option<(usize, Point)>,
    renaming: Option<(usize, String)>,
    cursor: Point,
}

impl App {
    fn new() -> App {
        use pane_grid::{Axis, Configuration as C};
        let config = C::Split {
            axis: Axis::Horizontal,
            ratio: 0.62,
            a: Box::new(C::Split {
                axis: Axis::Vertical,
                ratio: 0.2,
                a: Box::new(C::Pane(PaneKind::Library)),
                b: Box::new(C::Split {
                    axis: Axis::Vertical,
                    ratio: 0.74,
                    a: Box::new(C::Pane(PaneKind::Preview)),
                    b: Box::new(C::Pane(PaneKind::Inspector)),
                }),
            }),
            b: Box::new(C::Pane(PaneKind::Timeline)),
        };
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets").join("test1080p60.mp4");
        let playback = video::Decoder::open(&path).map(|decoder| Playback {
            decoder,
            frame: None,
            pending: None,
            playing: true,
            base: 0.0,
            resumed: Instant::now(),
            shown: 0,
            dropped: 0,
        });
        App {
            panes: pane_grid::State::with_configuration(config),
            maximized: false,
            playback,
            ticks: VecDeque::new(),
            font_choice: FontChoice::HindSiliguri,
            sample: "আমাদের নতুন প্রোডাক্ট — ক্ষমতা আর গতি".into(),
            sample_size: 30.0,
            timeline: Timeline::demo(),
            timeline_offset: 0.0,
            selected_track: None,
            menu: None,
            renaming: None,
            cursor: Point::ORIGIN,
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            window::frames().map(Message::Tick),
            event::listen_with(|event, _status, _window| match event {
                Event::Mouse(mouse::Event::CursorMoved { position }) => Some(Message::CursorMoved(position)),
                _ => None,
            }),
        ])
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::DragWindow => return window::latest().and_then(window::drag),
            Message::ToggleMaximize => {
                self.maximized = !self.maximized;
                return window::latest().and_then(window::toggle_maximize);
            }
            Message::Minimize => return window::latest().and_then(|id| window::minimize(id, true)),
            Message::Close => return window::latest().and_then(window::close),
            Message::ResizeWindow(direction) => return window::latest().and_then(move |id| window::drag_resize(id, direction)),
            Message::PaneResized(pane_grid::ResizeEvent { split, ratio }) => self.panes.resize(split, ratio),
            Message::Tick(now) => {
                self.ticks.push_back(now);
                while self.ticks.len() > 90 {
                    self.ticks.pop_front();
                }
                if let Ok(p) = &mut self.playback {
                    p.advance(now);
                }
            }
            Message::TogglePlay => {
                if let Ok(p) = &mut self.playback {
                    let now = Instant::now();
                    p.base = p.media_time(now);
                    p.resumed = now;
                    p.playing = !p.playing;
                }
            }
            Message::Seek(t) => self.timeline_offset = t - self.media_time(),
            Message::FontPicked(choice) => self.font_choice = choice,
            Message::SampleChanged(s) => self.sample = s,
            Message::SizeChanged(s) => self.sample_size = s,
            Message::CursorMoved(p) => self.cursor = p,
            Message::TrackMenu(i) => {
                self.selected_track = Some(i);
                self.menu = Some((i, self.cursor));
            }
            Message::CloseMenu => self.menu = None,
            Message::StartRename(i) => {
                self.menu = None;
                self.renaming = Some((i, self.timeline.tracks[i].name.clone()));
                return iced::widget::operation::focus("rename");
            }
            Message::RenameInput(s) => {
                if let Some((_, name)) = &mut self.renaming {
                    *name = s;
                }
            }
            Message::FinishRename => {
                if let Some((i, name)) = self.renaming.take() {
                    if !name.trim().is_empty() {
                        self.timeline.tracks[i].name = name.trim().to_string();
                    }
                }
            }
            Message::AddTrack => {
                self.menu = None;
                let n = self.timeline.tracks.len() + 1;
                self.timeline.tracks.push(timeline::Track { name: format!("Track {n}"), kind: "Video", color: Color::from_rgb8(0x6B, 0x7C, 0xFF) });
                self.timeline.invalidate();
            }
            Message::DeleteTrack(i) => {
                self.menu = None;
                self.timeline.tracks.remove(i);
                self.timeline.clips.retain(|c| c.track != i);
                for c in &mut self.timeline.clips {
                    if c.track > i {
                        c.track -= 1;
                    }
                }
                self.selected_track = None;
                self.timeline.invalidate();
            }
            Message::Noop => {}
        }
        Task::none()
    }

    fn media_time(&self) -> f64 {
        self.playback.as_ref().map(|p| p.media_time(Instant::now())).unwrap_or(0.0)
    }

    fn ui_fps(&self) -> f64 {
        match (self.ticks.front(), self.ticks.back()) {
            (Some(a), Some(b)) if self.ticks.len() > 1 => (self.ticks.len() - 1) as f64 / b.duration_since(*a).as_secs_f64().max(1e-6),
            _ => 0.0,
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let panes = pane_grid(&self.panes, |_pane, kind, _maximized| {
            let (title, body): (&str, Element<'_, Message>) = match kind {
                PaneKind::Library => ("Media & fonts", self.library()),
                PaneKind::Preview => ("Preview", self.preview()),
                PaneKind::Inspector => ("Inspector", self.inspector()),
                PaneKind::Timeline => ("Timeline", self.timeline_pane()),
            };
            let header = pane_grid::TitleBar::new(text(title).font(theme::POPPINS_SEMIBOLD).size(12).color(theme::DIM))
                .padding([8, 14])
                .style(theme::pane_header);
            pane_grid::Content::new(container(body).padding([4, 12]).width(Fill).height(Fill)).title_bar(header).style(theme::pane)
        })
        .spacing(6)
        .min_size(140)
        .on_resize(8, Message::PaneResized)
        .style(|_| pane_grid::Style {
            hovered_region: pane_grid::Highlight { background: Color { a: 0.08, ..theme::ACCENT }.into(), border: Default::default() },
            picked_split: pane_grid::Line { color: theme::ACCENT, width: 2.0 },
            hovered_split: pane_grid::Line { color: Color { a: 0.6, ..theme::ACCENT }, width: 2.0 },
        });

        let body = container(column![self.title_bar(), container(panes).padding(6).width(Fill).height(Fill)])
            .width(Fill)
            .height(Fill)
            .style(theme::root);

        let mut layers: Vec<Element<'_, Message>> = vec![body.into()];
        if let Some((track, pos)) = self.menu {
            layers.push(mouse_area(space().width(Fill).height(Fill)).on_press(Message::CloseMenu).on_right_press(Message::CloseMenu).into());
            layers.push(pin(self.track_menu(track)).x(pos.x).y(pos.y).into());
        }
        if !self.maximized {
            layers.push(resize_edges());
        }
        stack(layers).into()
    }

    fn title_bar(&self) -> Element<'_, Message> {
        let logo = container(space().width(14).height(14)).style(|_| container::Style {
            background: Some(iced::Gradient::Linear(
                iced::gradient::Linear::new(0.8).add_stop(0.0, theme::ACCENT).add_stop(1.0, Color::from_rgb8(0xB0, 0x6B, 0xFF)),
            ).into()),
            border: iced::Border { radius: 4.0.into(), ..Default::default() },
            ..Default::default()
        });
        let menu_button = |label| button(text(label).size(12.5).color(theme::DIM)).padding([5, 10]).style(theme::ghost).on_press(Message::Noop);
        let controls = row![
            window_button(Glyph::Minimize, Message::Minimize, false),
            window_button(if self.maximized { Glyph::Restore } else { Glyph::Maximize }, Message::ToggleMaximize, false),
            window_button(Glyph::Close, Message::Close, true),
        ];
        let bar = row![
            space().width(14),
            logo,
            space().width(10),
            text("AI Video Editor").font(theme::POPPINS_SEMIBOLD).size(13),
            space().width(14),
            menu_button("File"),
            menu_button("Edit"),
            menu_button("View"),
            menu_button("Help"),
            container(text("Product Launch — Draft").size(12.5).color(theme::DIM)).center_x(Fill),
            button(text("Export").font(theme::POPPINS_SEMIBOLD).size(12.5)).padding([6, 16]).style(theme::primary).on_press(Message::Noop),
            space().width(12),
            controls,
        ]
        .align_y(Alignment::Center)
        .height(42);
        mouse_area(container(bar).style(theme::title_bar).width(Fill))
            .on_press(Message::DragWindow)
            .on_double_click(Message::ToggleMaximize)
            .into()
    }

    fn library(&self) -> Element<'_, Message> {
        let media = [
            ("screen-recording.mp4", "Video · 4:12", Color::from_rgb8(0x3E, 0x8E, 0xF7)),
            ("face-cam.mov", "Video · 3:58", Color::from_rgb8(0x2F, 0xB5, 0xA8)),
            ("ভয়েসওভার.wav", "Audio · 2:30", Color::from_rgb8(0x3D, 0xB8, 0x6E)),
            ("logo.png", "Image", Color::from_rgb8(0x9B, 0x6B, 0xFF)),
            ("whoosh-soft.wav", "Sound effect · 0:01", Color::from_rgb8(0xE8, 0xA2, 0x3A)),
        ];
        let mut list = column![section("Media")].spacing(6);
        for (name, meta, color) in media {
            let thumb = container(space().width(36).height(26)).style(move |_| container::Style {
                background: Some(Color { a: 0.85, ..color }.into()),
                border: iced::Border { radius: 5.0.into(), ..Default::default() },
                ..Default::default()
            });
            let name_font = if name.is_ascii() { theme::POPPINS_MEDIUM } else { theme::HIND_SEMIBOLD };
            list = list.push(
                container(row![thumb, column![text(name).font(name_font).size(12.5), text(meta).size(11).color(theme::DIM)].spacing(1)].spacing(10).align_y(Alignment::Center))
                    .padding(8)
                    .width(Fill)
                    .style(theme::card),
            );
        }
        list = list.push(space().height(8)).push(section("Fonts"));
        let specimens = [
            ("Poppins", theme::POPPINS_SEMIBOLD, "Introducing the new Studio."),
            ("Inter", theme::INTER, "Built for pros. Made simple."),
            ("Roboto", theme::ROBOTO, "Faster than ever before."),
            ("Hind Siliguri", theme::HIND_SEMIBOLD, "আমার সোনার বাংলা"),
        ];
        for (name, font, sample) in specimens {
            list = list.push(
                container(column![text(name).size(11).color(theme::DIM), text(sample).font(font).size(17)].spacing(2)).padding([8, 10]).width(Fill).style(theme::card),
            );
        }
        scrollable(list.padding([4, 0])).height(Fill).into()
    }

    fn preview(&self) -> Element<'_, Message> {
        let (video, stats, time, playing): (Element<'_, Message>, String, f64, bool) = match &self.playback {
            Ok(p) => (
                shader(video::VideoView { frame: p.frame.clone() }).width(Fill).height(Fill).into(),
                format!(
                    "UI {:.0} fps  ·  video {}×{} @ {:.0}  ·  shown {}  ·  dropped {}",
                    self.ui_fps(),
                    p.decoder.width,
                    p.decoder.height,
                    p.decoder.fps,
                    p.shown,
                    p.dropped
                ),
                p.media_time(Instant::now()),
                p.playing,
            ),
            Err(e) => (container(text(format!("Video failed: {e}")).color(theme::DANGER)).center(Fill).into(), String::new(), 0.0, false),
        };
        let well = container(stack![video, container(container(text(stats).size(11)).padding([4, 8]).style(theme::chip)).padding(10)])
            .width(Fill)
            .height(Fill)
            .padding(1)
            .style(theme::preview_well);
        let transport = row![
            button(text(if playing { "Pause" } else { "Play" }).font(theme::POPPINS_MEDIUM).size(12.5)).padding([6, 14]).style(theme::primary).on_press(Message::TogglePlay),
            text(timeline::fmt_time(time % 20.0, true)).font(theme::ROBOTO).size(13),
            text("/ 0:20.00").size(12).color(theme::FAINT),
            space().width(Fill),
            text("YUV 4:2:0 → GPU").size(11.5).color(theme::DIM),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        column![well, transport].spacing(8).padding([0, 0]).into()
    }

    fn inspector(&self) -> Element<'_, Message> {
        let font = self.font_choice.font();
        let content = column![
            section("Text layer"),
            label("Content (type Bangla or English)"),
            text_input("Type here…", &self.sample).on_input(Message::SampleChanged).font(font).size(15).padding(10).style(theme::input),
            label("Font"),
            pick_list(&FontChoice::ALL[..], Some(self.font_choice), Message::FontPicked).padding(9).width(Fill).style(theme::select),
            label(&format!("Size  ·  {:.0} px", self.sample_size)),
            slider(12.0..=72.0, self.sample_size, Message::SizeChanged).step(1.0_f32),
            container(text(&self.sample).font(font).size(self.sample_size)).padding(14).width(Fill).style(theme::card),
            space().height(6),
            section("Shaping check"),
            container(
                column![
                    text("ক্ষ  ন্ত  কি  ত্র  শ্রী  স্ব  র্ক").font(theme::HIND_SEMIBOLD).size(26),
                    text("Conjuncts must join; ি must sit before its letter.").size(11).color(theme::DIM),
                ]
                .spacing(4)
            )
            .padding(12)
            .width(Fill)
            .style(theme::card),
        ]
        .spacing(8);
        scrollable(content.padding([4, 0])).height(Fill).into()
    }

    fn timeline_pane(&self) -> Element<'_, Message> {
        let mut headers = column![
            container(
                row![
                    text("Tracks").font(theme::POPPINS_SEMIBOLD).size(12).color(theme::DIM),
                    space().width(Fill),
                    button(text("+ Add").size(11.5)).padding([3, 8]).style(theme::ghost).on_press(Message::AddTrack),
                ]
                .align_y(Alignment::Center)
            )
            .height(timeline::RULER_H)
            .padding([0, 4])
        ];
        for (i, track) in self.timeline.tracks.iter().enumerate() {
            let name: Element<'_, Message> = match &self.renaming {
                Some((j, value)) if *j == i => text_input("Track name", value)
                    .id("rename")
                    .on_input(Message::RenameInput)
                    .on_submit(Message::FinishRename)
                    .size(12.5)
                    .padding([3, 6])
                    .style(theme::input)
                    .into(),
                _ => {
                    let font = if track.name.is_ascii() { theme::POPPINS_MEDIUM } else { theme::HIND_SEMIBOLD };
                    text(&track.name).font(font).size(12.5).wrapping(text::Wrapping::None).into()
                }
            };
            let dot = container(space().width(8).height(8)).style({
                let c = track.color;
                move |_| container::Style { background: Some(c.into()), border: iced::Border { radius: 4.0.into(), ..Default::default() }, ..Default::default() }
            });
            let header = container(
                row![dot, column![name, text(track.kind).size(10.5).color(theme::FAINT)].spacing(0).width(Fill)].spacing(8).align_y(Alignment::Center),
            )
            .padding([0, 10])
            .height(timeline::TRACK_H - 4.0)
            .width(Fill)
            .center_y(timeline::TRACK_H - 4.0)
            .clip(true)
            .style(theme::track_header(self.selected_track == Some(i)));
            headers = headers.push(
                container(mouse_area(header).on_right_press(Message::TrackMenu(i)).on_double_click(Message::StartRename(i))).padding([2, 0]),
            );
        }
        let playhead = self.timeline_offset + self.media_time();
        // Wheel over the canvas scrolls time; wheel over the track headers (or the scrollbar) scrolls tracks.
        scrollable(row![
            container(headers).width(180).padding([0, 6]),
            iced::widget::canvas(self.timeline.canvas(playhead)).width(Fill).height(self.timeline.content_height()),
        ])
        .height(Fill)
        .into()
    }

    fn track_menu(&self, track: usize) -> Element<'_, Message> {
        let item = |label: &'static str, msg: Message, danger: bool| {
            button(text(label).size(12.5)).width(Fill).padding([7, 10]).style(theme::menu_item(danger)).on_press(msg)
        };
        container(
            column![
                item("Rename track", Message::StartRename(track), false),
                item("Add track", Message::AddTrack, false),
                container(space().height(1).width(Fill)).style(|_| container::Style { background: Some(theme::BORDER.into()), ..Default::default() }).padding([4, 0]),
                item("Delete track", Message::DeleteTrack(track), true),
            ]
            .spacing(2),
        )
        .width(190)
        .padding(5)
        .style(theme::menu)
        .into()
    }
}

fn section<'a>(title: &'a str) -> Element<'a, Message> {
    text(title.to_uppercase()).font(theme::POPPINS_SEMIBOLD).size(10.5).color(theme::FAINT).into()
}

fn label<'a>(s: &str) -> Element<'a, Message> {
    text(s.to_string()).size(11.5).color(theme::DIM).into()
}

/// Invisible strips along the window edges that start a native resize (the window has no OS frame).
fn resize_edges<'a>() -> Element<'a, Message> {
    const E: f32 = 5.0;
    let edge = |w: Length, h: Length, dir: Direction, cursor: mouse::Interaction| -> Element<'a, Message> {
        mouse_area(space().width(w).height(h)).on_press(Message::ResizeWindow(dir)).interaction(cursor).into()
    };
    use mouse::Interaction::{ResizingDiagonallyDown as Diag, ResizingDiagonallyUp as AntiDiag, ResizingHorizontally as H, ResizingVertically as V};
    column![
        row![edge(E.into(), E.into(), Direction::NorthWest, Diag), edge(Fill, E.into(), Direction::North, V), edge(E.into(), E.into(), Direction::NorthEast, AntiDiag)],
        row![edge(E.into(), Fill, Direction::West, H), space().width(Fill).height(Fill), edge(E.into(), Fill, Direction::East, H)],
        row![edge(E.into(), E.into(), Direction::SouthWest, AntiDiag), edge(Fill, E.into(), Direction::South, V), edge(E.into(), E.into(), Direction::SouthEast, Diag)],
    ]
    .into()
}

#[derive(Clone, Copy)]
enum Glyph {
    Minimize,
    Maximize,
    Restore,
    Close,
}

/// Crisp vector window-control icons, so they don't depend on a font.
impl canvas::Program<Message> for Glyph {
    type State = ();

    fn draw(&self, _: &(), renderer: &Renderer, _: &Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut f = Frame::new(renderer, bounds.size());
        let s = Stroke::default().with_color(theme::TEXT).with_width(1.0);
        let c = bounds.size();
        let (x, y, d) = ((c.width - 10.0) / 2.0, (c.height - 10.0) / 2.0, 10.0);
        match self {
            Glyph::Minimize => f.stroke(&CanvasPath::line(Point::new(x, y + d / 2.0), Point::new(x + d, y + d / 2.0)), s),
            Glyph::Maximize => f.stroke(&CanvasPath::rectangle(Point::new(x + 0.5, y + 0.5), Size::new(d - 1.0, d - 1.0)), s),
            Glyph::Restore => {
                f.stroke(&CanvasPath::rectangle(Point::new(x + 0.5, y + 2.5), Size::new(d - 3.0, d - 3.0)), s);
                f.stroke(&CanvasPath::line(Point::new(x + 2.5, y + 0.5), Point::new(x + d - 0.5, y + 0.5)), s);
                f.stroke(&CanvasPath::line(Point::new(x + d - 0.5, y + 0.5), Point::new(x + d - 0.5, y + d - 2.5)), s);
            }
            Glyph::Close => {
                f.stroke(&CanvasPath::line(Point::new(x, y), Point::new(x + d, y + d)), s);
                f.stroke(&CanvasPath::line(Point::new(x + d, y), Point::new(x, y + d)), s);
            }
        }
        vec![f.into_geometry()]
    }
}

fn window_button<'a>(glyph: Glyph, msg: Message, close: bool) -> Element<'a, Message> {
    button(iced::widget::canvas(glyph).width(46).height(42)).padding(0).style(theme::window_control(close)).on_press(msg).into()
}
