//! Reusable components built from the tokens and styles.

use iced::widget::tooltip::Position;
use iced::widget::{button, column, container, row, scrollable, space, text, tooltip};
use iced::{Alignment, Element, Fill, Length, Padding};

use super::icons::{self, icon};
use super::style;
use super::tokens::*;

pub fn label<'a, M: 'a>(s: impl text::IntoFragment<'a>) -> Element<'a, M> {
    text(s).size(T_SMALL).color(TEXT_3).into()
}

/// Small uppercase heading above a group of controls.
pub fn section<'a, M: 'a>(title: &str) -> Element<'a, M> {
    text(title.to_uppercase()).font(INTER_SEMIBOLD).size(10.5).color(TEXT_3).into()
}

pub fn kbd<'a, M: 'a>(keys: &'a str) -> Element<'a, M> {
    container(text(keys).font(INTER_MEDIUM).size(10.5)).padding([1, 5]).style(style::kbd).into()
}

pub fn divider<'a, M: 'a>() -> Element<'a, M> {
    container(space().width(Fill).height(1)).style(style::hairline).into()
}

pub fn vdivider<'a, M: 'a>(height: f32) -> Element<'a, M> {
    container(space().width(1).height(height)).style(style::hairline).into()
}

/// Wraps `content` in a styled tooltip with an optional shortcut hint.
pub fn tip<'a, M: 'a>(content: impl Into<Element<'a, M>>, label: &'a str, shortcut: Option<&'a str>, position: Position) -> Element<'a, M> {
    let mut body = row![text(label).size(T_SMALL)].spacing(S2).align_y(Alignment::Center);
    if let Some(keys) = shortcut {
        body = body.push(text(keys).size(T_CAPTION).color(TEXT_3));
    }
    tooltip(content, container(body).padding([5, 8]).style(style::tooltip), position).gap(6).snap_within_viewport(true).into()
}

pub fn icon_button<'a, M: Clone + 'a>(code: char, on_press: Option<M>) -> button::Button<'a, M> {
    button(container(icon(code, 15)).center(CONTROL_H - 2.0)).width(CONTROL_H - 2.0).height(CONTROL_H - 2.0).padding(0).style(style::ghost).on_press_maybe(on_press)
}

pub fn icon_toggle<'a, M: Clone + 'a>(code: char, active: bool, on_press: M) -> button::Button<'a, M> {
    button(container(icon(code, 15)).center(CONTROL_H - 2.0)).width(CONTROL_H - 2.0).height(CONTROL_H - 2.0).padding(0).style(style::toggle(active)).on_press(on_press)
}

fn labeled<'a, M: 'a>(code: Option<char>, label: &'a str, font: iced::Font) -> Element<'a, M> {
    let mut r = row![].spacing(7).align_y(Alignment::Center);
    if let Some(c) = code {
        r = r.push(icon(c, 14));
    }
    r.push(text(label).font(font).size(T_BODY)).into()
}

pub fn primary_button<'a, M: Clone + 'a>(code: Option<char>, label: &'a str, on_press: Option<M>) -> button::Button<'a, M> {
    button(labeled(code, label, INTER_SEMIBOLD)).padding([7, 14]).height(CONTROL_H).style(style::primary).on_press_maybe(on_press)
}

pub fn secondary_button<'a, M: Clone + 'a>(code: Option<char>, label: &'a str, on_press: Option<M>) -> button::Button<'a, M> {
    button(labeled(code, label, INTER_MEDIUM)).padding([7, 13]).height(CONTROL_H).style(style::secondary).on_press_maybe(on_press)
}

pub fn ghost_button<'a, M: Clone + 'a>(code: Option<char>, label: &'a str, on_press: Option<M>) -> button::Button<'a, M> {
    button(labeled(code, label, INTER_MEDIUM)).padding([7, 10]).height(CONTROL_H).style(style::ghost).on_press_maybe(on_press)
}

/// A row of mutually exclusive options.
pub fn segmented<'a, T, M>(options: &[T], selected: T, label: impl Fn(&T) -> String, on_select: impl Fn(T) -> M) -> Element<'a, M>
where
    T: Copy + PartialEq + 'a,
    M: Clone + 'a,
{
    let items = options.iter().map(|&o| {
        button(container(text(label(&o)).font(INTER_MEDIUM).size(T_SMALL)).center_x(Fill))
            .width(Fill)
            .padding([5, 10])
            .style(style::segment(o == selected))
            .on_press(on_select(o))
            .into()
    });
    container(row(items).spacing(2)).padding(2).style(style::segmented_track).into()
}

/// A labelled control stacked vertically.
pub fn field<'a, M: 'a>(name: &'a str, control: impl Into<Element<'a, M>>) -> Element<'a, M> {
    column![label(name), control.into()].spacing(6).into()
}

pub fn scroll<'a, M: 'a>(content: impl Into<Element<'a, M>>) -> scrollable::Scrollable<'a, M> {
    scrollable(content)
        .direction(scrollable::Direction::Vertical(scrollable::Scrollbar::new().width(6).scroller_width(6).margin(2)))
        .style(style::scrollbar)
}

pub enum MenuItem<'a, M> {
    Action { icon: Option<char>, label: &'a str, shortcut: Option<&'a str>, on_press: Option<M>, danger: bool },
    Separator,
}

impl<'a, M> MenuItem<'a, M> {
    pub fn new(icon: char, label: &'a str, on_press: M) -> Self {
        MenuItem::Action { icon: Some(icon), label, shortcut: None, on_press: Some(on_press), danger: false }
    }

    pub fn shortcut(self, keys: &'a str) -> Self {
        match self {
            MenuItem::Action { icon, label, on_press, danger, .. } => MenuItem::Action { icon, label, shortcut: Some(keys), on_press, danger },
            s => s,
        }
    }

    pub fn danger(self) -> Self {
        match self {
            MenuItem::Action { icon, label, shortcut, on_press, .. } => MenuItem::Action { icon, label, shortcut, on_press, danger: true },
            s => s,
        }
    }

    pub fn disabled(self) -> Self {
        match self {
            MenuItem::Action { icon, label, shortcut, danger, .. } => MenuItem::Action { icon, label, shortcut, on_press: None, danger },
            s => s,
        }
    }
}

/// A dropdown or context menu panel.
pub fn menu<'a, M: Clone + 'a>(items: Vec<MenuItem<'a, M>>, width: f32) -> Element<'a, M> {
    let rows = items.into_iter().map(|item| match item {
        MenuItem::Separator => container(divider()).padding([4, 6]).into(),
        MenuItem::Action { icon: code, label, shortcut, on_press, danger } => {
            let mut r = row![].spacing(9).align_y(Alignment::Center);
            r = r.push(container(code.map(|c| -> Element<'a, M> { icon(c, 14).into() }).unwrap_or_else(|| space().width(14).into())).width(14));
            r = r.push(text(label).size(T_BODY).width(Fill));
            if let Some(keys) = shortcut {
                r = r.push(text(keys).size(T_CAPTION).color(TEXT_3));
            }
            button(r).width(Fill).padding([6, 9]).style(style::menu_item(danger)).on_press_maybe(on_press).into()
        }
    });
    container(column(rows).spacing(1)).width(width).padding(5).style(style::popover).into()
}

pub fn empty_state<'a, M: Clone + 'a>(code: char, title: &'a str, body: &'a str, action: Option<Element<'a, M>>) -> Element<'a, M> {
    let mut col = column![
        container(icons::tinted(code, 20, TEXT_2)).center(44).style(style::card),
        text(title).font(INTER_SEMIBOLD).size(T_LARGE),
        text(body).size(T_SMALL).color(TEXT_3).align_x(Alignment::Center),
    ]
    .spacing(S2)
    .align_x(Alignment::Center)
    .max_width(260);
    if let Some(a) = action {
        col = col.push(space().height(4)).push(a);
    }
    container(col).center(Fill).padding(S4).into()
}

pub fn pad(top: f32, right: f32, bottom: f32, left: f32) -> Padding {
    Padding { top, right, bottom, left }
}

pub fn fixed(px: f32) -> Length {
    Length::Fixed(px)
}
