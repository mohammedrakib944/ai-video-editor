//! The library panel: media, text, effects and sounds.

use iced::widget::{button, column, container, image, mouse_area, row, space, stack, text};
use iced::{Alignment, Color, Element, Fill, Length, mouse};
use iced_aw::ContextMenu;

use super::{Editor, LibraryTab, Message, fmt_duration};
use crate::clip::{MotionKind, TransitionKind};
use crate::model::MediaKind;
use crate::ui::icons::{code, icon, tinted};
use crate::ui::tokens::*;
use crate::ui::widgets::{self, MenuItem};
use crate::ui::style;

impl Editor {
    pub(super) fn library(&self) -> Element<'_, Message> {
        match self.library_tab {
            LibraryTab::Media => self.media_library(),
            LibraryTab::Text => {
                let card = |title: &'static str, sub: &'static str, font: iced::Font, size: f32| {
                    button(column![text(title).font(font).size(size), text(sub).size(T_CAPTION).color(TEXT_3)].spacing(4))
                        .width(Fill)
                        .padding([12, 12])
                        .style(style::choice_card(false))
                        .on_press(Message::AddText)
                };
                widgets::scroll(
                    column![
                        widgets::section("Add text"),
                        card("Headline", "Big bold title", INTER_BOLD, 20.0),
                        card("Subtitle", "Supporting line", INTER_MEDIUM, 15.0),
                        card("শিরোনাম", "Bangla headline · Hind Siliguri", HIND_SEMIBOLD, 19.0),
                        text("Click to add at the playhead. Press T anytime.").size(T_CAPTION).color(TEXT_4),
                        space().height(6),
                        widgets::section("Shapes"),
                        shape_grid(),
                    ]
                    .spacing(S2)
                    .padding([8, 10]),
                )
                .height(Fill)
                .into()
            }
            LibraryTab::Transitions => self.effects_tab(),
            LibraryTab::Sounds => self.sounds_tab(),
        }
    }

    fn media_library(&self) -> Element<'_, Message> {
        let project = &self.doc.project;
        if project.assets.is_empty() && self.importing.is_empty() {
            return widgets::empty_state(
                code::UPLOAD,
                "Bring in your media",
                "Drop screen recordings, face-cam videos, voiceovers, images or logos here.",
                Some(widgets::primary_button(Some(code::PLUS), "Import media", Some(Message::Import)).into()),
            );
        }
        let mut cards: Vec<Element<'_, Message>> = project.assets.iter().map(|a| self.media_card(a)).collect();
        for path in &self.importing {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            cards.push(
                container(
                    column![
                        container(iced_aw::Spinner::new().width(Length::Fixed(20.0)).height(Length::Fixed(20.0)).circle_radius(2.0)).center_x(Fill).height(76).align_y(Alignment::Center).style(style::inset),
                        text(name).size(T_CAPTION).color(TEXT_3).wrapping(text::Wrapping::None),
                    ]
                    .spacing(6),
                )
                .padding(6)
                .into(),
            );
        }
        let grid = iced::widget::grid(cards).fluid(190).spacing(6).height(iced::widget::grid::Sizing::EvenlyDistribute(Length::Shrink));
        widgets::scroll(container(grid).padding([6, 8])).height(Fill).into()
    }

    fn media_card<'a>(&'a self, a: &'a crate::model::Asset) -> Element<'a, Message> {
        let selected = self.library_selected == Some(a.id);
        let thumb: Element<'a, Message> = match self.media.get(a.id).and_then(|m| m.cover.clone()) {
            Some(h) => image(h).width(Fill).height(76).content_fit(iced::ContentFit::Cover).border_radius(6.0).into(),
            None => {
                let glyph = match a.kind {
                    MediaKind::Audio => code::AUDIO_WAVEFORM,
                    MediaKind::Image => code::IMAGE,
                    MediaKind::Video => code::FILM,
                };
                let color = if a.kind == MediaKind::Audio { clip::AUDIO } else { TEXT_3 };
                container(tinted(glyph, 22, color)).center_x(Fill).height(76).align_y(Alignment::Center).style(style::inset).into()
            }
        };
        let missing = self.missing.contains(&a.id);
        let badge = if missing { "Missing".to_string() } else if a.kind == MediaKind::Image { "IMG".to_string() } else { fmt_duration(a.duration) };
        let thumb = stack![
            thumb,
            container(container(text(badge).font(INTER_SEMIBOLD).size(10)).padding([1, 5]).style(move |_| container::Style {
                background: Some(if missing { DANGER } else { Color { a: 0.7, ..Color::BLACK } }.into()),
                text_color: Some(if missing { ON_ACCENT } else { Color::WHITE }),
                border: iced::Border { radius: 4.0.into(), ..Default::default() },
                ..Default::default()
            }))
            .align_right(Fill)
            .align_bottom(76)
            .padding(5),
        ];
        let name_font = if a.name.is_ascii() { INTER_MEDIUM } else { HIND };
        let card = container(column![thumb, text(&a.name).font(name_font).size(T_CAPTION).wrapping(text::Wrapping::None)].spacing(6))
            .padding(5)
            .width(Fill)
            .style(move |t| {
                let mut s = style::card(t);
                if selected {
                    s.border = iced::Border { color: ACCENT, width: 1.5, radius: R_LG.into() };
                    s.background = Some(Color { a: 0.08, ..ACCENT }.into());
                } else {
                    s.background = None;
                    s.border.color = Color::TRANSPARENT;
                }
                s
            });
        let id = a.id;
        let area = mouse_area(card).on_press(Message::LibraryPress(id)).on_double_click(Message::LibraryAdd(id)).interaction(mouse::Interaction::Grab);
        ContextMenu::new(area, move || {
            widgets::menu(
                vec![
                    MenuItem::new(code::PLUS, "Add to timeline", Message::LibraryAdd(id)).shortcut("Double-click"),
                    MenuItem::new(code::FOLDER_OPEN, if missing { "Locate file…" } else { "Replace file…" }, Message::Relink(id)),
                    MenuItem::Separator,
                    MenuItem::new(code::TRASH_2, "Remove from project", Message::LibraryRemove(id)).danger(),
                ],
                220.0,
            )
        })
        .style(|_, _| iced_aw::style::context_menu::Style { background: Color::TRANSPARENT.into() })
        .into()
    }


    fn effects_tab(&self) -> Element<'_, Message> {
        let has_selection = !self.selection.is_empty();
        let card = |glyph: char, label: &'static str, msg: Message| -> Element<'_, Message> {
            button(column![container(icon(glyph, 18).color(ACCENT_HOVER)).center_x(Fill).height(30).align_y(Alignment::Center), text(label).font(INTER_MEDIUM).size(T_SMALL)].spacing(4).align_x(Alignment::Center))
                .width(Fill)
                .padding([10, 6])
                .style(style::choice_card(false))
                .on_press_maybe(has_selection.then_some(msg))
                .into()
        };
        let transitions = [
            (code::BLEND, "Crossfade", TransitionKind::Crossfade),
            (code::ARROW_LEFT_RIGHT, "Slide", TransitionKind::SlideLeft),
            (code::CHEVRON_UP, "Slide up", TransitionKind::SlideUp),
            (code::ZOOM_IN, "Zoom through", TransitionKind::ZoomThrough),
            (code::COLUMNS_3, "Wipe", TransitionKind::Wipe),
            (code::MOVE, "Push", TransitionKind::Push),
        ];
        let motions = [
            (code::SUN, "Fade", MotionKind::Fade),
            (code::CHEVRON_UP, "Slide up", MotionKind::SlideUp),
            (code::CHEVRON_DOWN, "Slide down", MotionKind::SlideDown),
            (code::CHEVRON_LEFT, "Slide left", MotionKind::SlideLeft),
            (code::CHEVRON_RIGHT, "Slide right", MotionKind::SlideRight),
            (code::SPARKLE, "Pop", MotionKind::Pop),
            (code::FOCUS, "Zoom in", MotionKind::ZoomIn),
        ];
        fn grid(items: Vec<Element<'_, Message>>) -> iced::widget::Grid<'_, Message> {
            iced::widget::grid(items).fluid(110).spacing(6).height(iced::widget::grid::Sizing::EvenlyDistribute(Length::Shrink))
        }
        let hint = if has_selection { "Applies to the selected clips." } else { "Select a clip on the timeline first." };
        widgets::scroll(
            column![
                text(hint).size(T_CAPTION).color(if has_selection { TEXT_3 } else { WARNING }),
                widgets::section("Transition in"),
                grid(transitions.iter().map(|&(g, l, k)| card(g, l, Message::ApplyTransition(k))).collect()),
                widgets::section("Animate in"),
                grid(motions.iter().map(|&(g, l, k)| card(g, l, Message::ApplyMotion { intro: true, kind: k })).collect()),
                widgets::section("Animate out"),
                grid(motions.iter().map(|&(g, l, k)| card(g, l, Message::ApplyMotion { intro: false, kind: k })).collect()),
            ]
            .spacing(S3)
            .padding([8, 10]),
        )
        .height(Fill)
        .into()
    }

    fn sounds_tab(&self) -> Element<'_, Message> {
        let rows = crate::sfx::SOUNDS.iter().map(|s| {
            let playing = self.sound_preview.as_ref().is_some_and(|(id, _, _)| *id == s.id);
            row![
                widgets::tip(
                    button(container(icon(if playing { code::AUDIO_LINES } else { code::PLAY }, 13)).center(30)).width(30).height(30).padding(0).style(style::toggle(playing)).on_press(Message::PreviewSound(s.id)),
                    "Listen",
                    None,
                    iced::widget::tooltip::Position::Top,
                ),
                column![text(s.name).font(INTER_MEDIUM).size(T_BODY), text(s.hint).size(T_CAPTION).color(TEXT_3).wrapping(text::Wrapping::None)].spacing(1).width(Fill),
                widgets::tip(widgets::icon_button(code::PLUS, Some(Message::AddSound(s.id))), "Add at playhead", None, iced::widget::tooltip::Position::Left),
            ]
            .spacing(S3)
            .align_y(Alignment::Center)
            .padding([6, 8])
            .into()
        });
        widgets::scroll(
            column![
                text("Original sounds, free to use anywhere. They go on an audio track at the playhead.").size(T_CAPTION).color(TEXT_3),
                column(rows).spacing(2),
            ]
            .spacing(S3)
            .padding([8, 10]),
        )
        .height(Fill)
        .into()
    }
}

/// Ready-made shapes in the library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapePreset {
    Card,
    Pill,
    Circle,
    Triangle,
    Star,
    Hexagon,
    Ring,
    Spotlight,
}

impl ShapePreset {
    const ALL: [ShapePreset; 8] = [ShapePreset::Card, ShapePreset::Pill, ShapePreset::Circle, ShapePreset::Triangle, ShapePreset::Star, ShapePreset::Hexagon, ShapePreset::Ring, ShapePreset::Spotlight];

    fn label(self) -> &'static str {
        match self {
            ShapePreset::Card => "Card",
            ShapePreset::Pill => "Pill",
            ShapePreset::Circle => "Circle",
            ShapePreset::Triangle => "Triangle",
            ShapePreset::Star => "Star",
            ShapePreset::Hexagon => "Hexagon",
            ShapePreset::Ring => "Ring",
            ShapePreset::Spotlight => "Spotlight",
        }
    }

    /// The shape, and its corner radius.
    pub fn build(self) -> (crate::clip::ShapeLayer, f64) {
        use crate::clip::{Rgba, ShapeKind, ShapeLayer};
        let (lime, mint) = (Rgba::hex(0xD6FF00), Rgba::hex(0x2EF2A2));
        match self {
            ShapePreset::Card => (ShapeLayer::new(ShapeKind::Rectangle, 560.0, 300.0, Rgba::hex(0x232720), Some(Rgba::hex(0x14160F))), 40.0),
            ShapePreset::Pill => (ShapeLayer::new(ShapeKind::Rectangle, 420.0, 110.0, lime, Some(mint)), 55.0),
            ShapePreset::Circle => (ShapeLayer::new(ShapeKind::Ellipse, 320.0, 320.0, lime, Some(mint)), 0.0),
            ShapePreset::Triangle => (ShapeLayer::new(ShapeKind::Triangle, 320.0, 280.0, lime, Some(mint)), 16.0),
            ShapePreset::Star => (ShapeLayer::new(ShapeKind::Star, 320.0, 320.0, lime, Some(mint)), 8.0),
            ShapePreset::Hexagon => (ShapeLayer::new(ShapeKind::Hexagon, 320.0, 320.0, lime, Some(mint)), 12.0),
            ShapePreset::Ring => (ShapeLayer::new(ShapeKind::Ring, 320.0, 320.0, lime, Some(mint)), 0.0),
            ShapePreset::Spotlight => {
                let mut s = ShapeLayer::new(ShapeKind::Ellipse, 900.0, 900.0, Rgba(0.84, 1.0, 0.0, 0.55), Some(Rgba(0.84, 1.0, 0.0, 0.0)));
                s.radial = true;
                (s, 0.0)
            }
        }
    }
}

fn shape_grid<'a>() -> Element<'a, Message> {
    let items = ShapePreset::ALL.iter().map(|&p| -> Element<'a, Message> {
        button(column![iced::widget::canvas(ShapeIcon(p)).width(34).height(34), text(p.label()).font(INTER_MEDIUM).size(T_CAPTION)].spacing(6).align_x(Alignment::Center))
            .width(Fill)
            .padding([10, 4])
            .style(style::choice_card(false))
            .on_press(Message::AddShape(p))
            .into()
    });
    iced::widget::grid(items.collect::<Vec<_>>()).fluid(84).spacing(6).height(iced::widget::grid::Sizing::EvenlyDistribute(Length::Shrink)).into()
}

/// A small drawing of a shape preset, in the brand gradient.
struct ShapeIcon(ShapePreset);

impl iced::widget::canvas::Program<Message> for ShapeIcon {
    type State = ();

    fn draw(&self, _: &(), renderer: &iced::Renderer, _: &iced::Theme, bounds: iced::Rectangle, _: mouse::Cursor) -> Vec<iced::widget::canvas::Geometry> {
        use iced::widget::canvas::{Frame, Gradient, Path, Stroke, gradient};
        use iced::{Point, Size};
        let mut f = Frame::new(renderer, bounds.size());
        let s = bounds.width.min(bounds.height);
        let c = Point::new(s / 2.0, s / 2.0);
        let fill = Gradient::Linear(gradient::Linear::new(Point::new(0.0, 0.0), Point::new(0.0, s)).add_stop(0.0, ACCENT).add_stop(1.0, BRAND_2));
        let poly = |pts: Vec<Point>| Path::new(|b| {
            b.move_to(pts[0]);
            for p in &pts[1..] {
                b.line_to(*p);
            }
            b.close();
        });
        match self.0 {
            ShapePreset::Card => f.fill(&Path::rounded_rectangle(Point::new(2.0, s * 0.2), Size::new(s - 4.0, s * 0.6), 6.0.into()), SURFACE_4),
            ShapePreset::Pill => f.fill(&Path::rounded_rectangle(Point::new(2.0, s * 0.32), Size::new(s - 4.0, s * 0.36), (s * 0.18).into()), fill),
            ShapePreset::Circle => f.fill(&Path::circle(c, s * 0.42), fill),
            ShapePreset::Triangle => f.fill(&poly(vec![Point::new(c.x, s * 0.1), Point::new(s * 0.92, s * 0.88), Point::new(s * 0.08, s * 0.88)]), fill),
            ShapePreset::Star => {
                let pts = (0..10)
                    .map(|i| {
                        let r = if i % 2 == 0 { s * 0.46 } else { s * 0.2 };
                        let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
                        Point::new(c.x + r * a.cos(), c.y + r * a.sin())
                    })
                    .collect();
                f.fill(&poly(pts), fill);
            }
            ShapePreset::Hexagon => {
                let pts = (0..6).map(|i| {
                    let a = std::f32::consts::FRAC_PI_6 + i as f32 * std::f32::consts::FRAC_PI_3;
                    Point::new(c.x + s * 0.44 * a.cos(), c.y + s * 0.44 * a.sin())
                });
                f.fill(&poly(pts.collect()), fill);
            }
            ShapePreset::Ring => f.stroke(&Path::circle(c, s * 0.36), Stroke::default().with_color(ACCENT).with_width(s * 0.13)),
            ShapePreset::Spotlight => {
                for (i, a) in [0.12, 0.2, 0.35].iter().enumerate() {
                    f.fill(&Path::circle(c, s * (0.46 - i as f32 * 0.12)), iced::Color { a: *a, ..ACCENT });
                }
            }
        }
        vec![f.into_geometry()]
    }
}

