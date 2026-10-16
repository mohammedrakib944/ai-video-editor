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
                        button(row![container(space().width(36).height(22)).style(|_| container::Style {
                            background: Some(style::brand_gradient()),
                            border: iced::Border { radius: 6.0.into(), ..Default::default() },
                            ..Default::default()
                        }), column![text("Gradient card").font(INTER_SEMIBOLD).size(T_BODY), text("Rounded rectangle for callouts").size(T_CAPTION).color(TEXT_3)].spacing(2)].spacing(S3).align_y(Alignment::Center))
                            .width(Fill)
                            .padding([10, 12])
                            .style(style::choice_card(false))
                            .on_press(Message::AddShape),
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
        let badge = if a.kind == MediaKind::Image { "IMG".to_string() } else { fmt_duration(a.duration) };
        let thumb = stack![
            thumb,
            container(container(text(badge).font(INTER_SEMIBOLD).size(10)).padding([1, 5]).style(|_| container::Style {
                background: Some(Color { a: 0.7, ..Color::BLACK }.into()),
                text_color: Some(Color::WHITE),
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
