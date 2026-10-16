//! Window chrome: custom title bar, window controls and resize edges.
//!
//! The window has no OS frame. On Windows the OS still draws its rounded corners
//! and shadow (see `main.rs`); on Linux we inset the content and draw them ourselves.

use iced::widget::{button, column, container, mouse_area, row, space, stack};
use iced::window::Direction;
use iced::{Alignment, Element, Fill, Length, mouse};

use super::icons::{code, icon};
use super::style;
use super::tokens::*;

/// Messages the chrome emits; the app maps them to window tasks.
#[derive(Debug, Clone, Copy)]
pub enum WindowAction {
    Drag,
    ToggleMaximize,
    Minimize,
    Close,
    Resize(Direction),
}

/// Whether we draw our own rounded frame and shadow (Linux, not maximized).
pub fn draws_frame(maximized: bool) -> bool {
    cfg!(not(windows)) && !maximized
}

/// Space around the frame for the drawn shadow.
pub const SHADOW_INSET: f32 = 10.0;

pub fn title_bar<'a, M: Clone + 'a>(
    left: Element<'a, M>,
    center: Element<'a, M>,
    right: Element<'a, M>,
    maximized: bool,
    on_window: impl Fn(WindowAction) -> M + Copy + 'a,
) -> Element<'a, M> {
    let rounded = draws_frame(maximized);
    let control = |code: char, action: WindowAction, close: bool| {
        button(container(icon(code, if close { 16 } else { 14 })).center(Length::Fill))
            .width(46)
            .height(TITLE_BAR_H)
            .padding(0)
            .style(style::window_control(close))
            .on_press(on_window(action))
    };
    let controls = row![
        control(code::MINUS, WindowAction::Minimize, false),
        control(if maximized { code::COPY } else { code::SQUARE }, WindowAction::ToggleMaximize, false),
        control(code::X, WindowAction::Close, true),
    ];
    let bar = row![
        container(left).width(Fill).align_y(Alignment::Center),
        container(center).align_y(Alignment::Center),
        container(row![right, space().width(S3), controls].align_y(Alignment::Center)).width(Fill).align_right(Fill),
    ]
    .height(TITLE_BAR_H)
    .align_y(Alignment::Center);
    let bar = column![container(bar).padding(iced::Padding { left: S3, ..Default::default() }), container(space().width(Fill).height(1)).style(style::hairline)];
    mouse_area(container(bar).width(Fill).style(style::title_bar(rounded)))
        .on_press(on_window(WindowAction::Drag))
        .on_double_click(on_window(WindowAction::ToggleMaximize))
        .into()
}

/// The app's mark: a gradient tile with a play glyph.
pub fn logo<'a, M: 'a>() -> Element<'a, M> {
    container(icon(code::PLAY, 11).color(iced::Color::WHITE)).center(22).style(style::logo).into()
}

/// Wraps the window content in the frame, plus invisible resize edges when not maximized.
pub fn frame<'a, M: Clone + 'a>(content: Element<'a, M>, maximized: bool, on_window: impl Fn(WindowAction) -> M + Copy + 'a) -> Element<'a, M> {
    let rounded = draws_frame(maximized);
    let body = container(content).padding(if rounded { 1.0 } else { 0.0 }).width(Fill).height(Fill).style(style::window_frame(rounded));
    let framed: Element<'a, M> = if rounded { container(body).padding(SHADOW_INSET).width(Fill).height(Fill).into() } else { body.into() };
    if maximized {
        return framed;
    }
    stack![framed, resize_edges(on_window)].into()
}

fn resize_edges<'a, M: Clone + 'a>(on_window: impl Fn(WindowAction) -> M + Copy + 'a) -> Element<'a, M> {
    let e: f32 = if cfg!(windows) { 5.0 } else { SHADOW_INSET };
    let c = e + 6.0;
    let edge = |w: Length, h: Length, dir: Direction, cursor: mouse::Interaction| -> Element<'a, M> {
        mouse_area(space().width(w).height(h)).on_press(on_window(WindowAction::Resize(dir))).interaction(cursor).into()
    };
    use mouse::Interaction::{ResizingDiagonallyDown as Down, ResizingDiagonallyUp as Up, ResizingHorizontally as H, ResizingVertically as V};
    column![
        row![edge(c.into(), e.into(), Direction::NorthWest, Down), edge(Fill, e.into(), Direction::North, V), edge(c.into(), e.into(), Direction::NorthEast, Up)],
        row![edge(e.into(), Fill, Direction::West, H), space().width(Fill).height(Fill), edge(e.into(), Fill, Direction::East, H)],
        row![edge(c.into(), e.into(), Direction::SouthWest, Up), edge(Fill, e.into(), Direction::South, V), edge(c.into(), e.into(), Direction::SouthEast, Down)],
    ]
    .into()
}
