//! The Keyboard Shortcuts dialog: a read-only reference, reachable from the app menu
//! (or Ctrl+/) so shortcuts don't have to be learned one tooltip at a time.

use iced::widget::{column, container, mouse_area, opaque, row, space, text};
use iced::{Alignment, Element, Fill};

use crate::ui::icons::{code, icon};
use crate::ui::tokens::*;
use crate::ui::{style, widgets};

#[derive(Debug, Clone, Copy)]
pub enum Message {
    Close,
}

/// (group title, [(action, key combos)]).
const GROUPS: &[(&str, &[(&str, &[&str])])] = &[
    (
        "File",
        &[
            ("New project", &["Ctrl+N"]),
            ("Open project", &["Ctrl+O"]),
            ("Save", &["Ctrl+S"]),
            ("Save as", &["Ctrl+Shift+S"]),
            ("Settings", &["Ctrl+,"]),
            ("Quit", &["Ctrl+Q"]),
        ],
    ),
    (
        "Edit",
        &[
            ("Undo", &["Ctrl+Z"]),
            ("Redo", &["Ctrl+Shift+Z", "Ctrl+Y"]),
            ("Duplicate selection", &["Ctrl+D"]),
            ("Delete selection", &["Del"]),
            ("Bring to front", &["Ctrl+]"]),
            ("Send to back", &["Ctrl+["]),
            ("Import media", &["Ctrl+I"]),
        ],
    ),
    (
        "Playback",
        &[
            ("Play / Pause", &["Space"]),
            ("Go to start", &["Home"]),
            ("Go to end", &["End"]),
            ("Step one frame", &["←", "→"]),
            ("Step 10 frames", &["Shift+←", "Shift+→"]),
            ("Mute preview", &["M"]),
        ],
    ),
    (
        "Timeline & clips",
        &[
            ("Add text at playhead", &["T"]),
            ("Split at playhead", &["S", "B"]),
            ("Select a range", &["R"]),
            ("Done with range", &["Enter"]),
            ("Add zoom at playhead", &["Z"]),
            ("Add press pulse", &["P"]),
            ("Add marker", &["Shift+M"]),
            ("Toggle snapping", &["N"]),
            ("Zoom timeline in / out", &["+", "-"]),
            ("Fit timeline to window", &["Shift+Z"]),
        ],
    ),
    (
        "View",
        &[
            ("Fit preview to window", &["Ctrl+0"]),
            ("Preview at 100%", &["Ctrl+1"]),
            ("Toggle assistant", &["Ctrl+K"]),
            ("Export video", &["Ctrl+E"]),
            ("Cancel / close", &["Esc"]),
        ],
    ),
];

fn keys_row<'a>(keys: &'a [&'a str]) -> Element<'a, Message> {
    let mut r = row![].spacing(5).align_y(Alignment::Center);
    for (i, k) in keys.iter().enumerate() {
        if i > 0 {
            r = r.push(text("or").size(10.5).color(TEXT_4));
        }
        r = r.push(widgets::kbd(k));
    }
    r.into()
}

fn group<'a>(title: &'a str, rows: &'a [(&'a str, &'a [&'a str])]) -> Element<'a, Message> {
    let items = rows.iter().map(|(label, keys)| row![text(*label).size(T_SMALL).color(TEXT_2).width(Fill), keys_row(keys)].align_y(Alignment::Center).into());
    container(column![widgets::section(title), column(items).spacing(9)].spacing(S3)).padding(S4).width(Fill).style(style::card).into()
}

pub fn view<'a>() -> Element<'a, Message> {
    let groups = column(GROUPS.iter().map(|(title, rows)| group(title, rows))).spacing(S3);
    let header = row![
        container(icon(code::KEYBOARD, 16).color(ON_ACCENT)).center(34).style(style::logo),
        column![text("Keyboard shortcuts").font(INTER_SEMIBOLD).size(T_TITLE), text("Most actions are also one click away in the menus and toolbars.").size(T_SMALL).color(TEXT_3)].spacing(2).width(Fill),
        widgets::icon_button(code::X, Some(Message::Close)),
    ]
    .spacing(S3)
    .align_y(Alignment::Center);
    let footer = row![space().width(Fill), widgets::primary_button(None, "Done", Some(Message::Close))];
    let body = widgets::scroll(groups.padding(iced::Padding { right: 10.0, ..Default::default() })).height(iced::Length::Shrink);
    let card = container(column![header, container(body).max_height(520), widgets::divider(), footer].spacing(S4)).width(560).padding(S6).style(style::popover);
    let scrim = mouse_area(container(space().width(Fill).height(Fill)).style(style::scrim)).on_press(Message::Close);
    iced::widget::stack![scrim, container(opaque(card)).center(Fill)].into()
}
