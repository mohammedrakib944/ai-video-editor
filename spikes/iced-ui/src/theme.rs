//! Colors, fonts and widget styles for the prototype.

use iced::border::Radius;
use iced::font::Weight;
use iced::theme::Palette;
use iced::widget::{button, container, pick_list, text_input};
use iced::{Background, Border, Color, Font, Shadow, Theme, Vector};

pub const BG: Color = Color::from_rgb8(0x0C, 0x0D, 0x11);
pub const TITLE: Color = Color::from_rgb8(0x10, 0x12, 0x17);
pub const PANEL: Color = Color::from_rgb8(0x15, 0x17, 0x1D);
pub const PANEL_2: Color = Color::from_rgb8(0x1B, 0x1E, 0x26);
pub const PANEL_3: Color = Color::from_rgb8(0x23, 0x27, 0x31);
pub const BORDER: Color = Color::from_rgb8(0x25, 0x29, 0x33);
pub const TEXT: Color = Color::from_rgb8(0xE7, 0xE9, 0xEE);
pub const DIM: Color = Color::from_rgb8(0x8A, 0x92, 0xA3);
pub const FAINT: Color = Color::from_rgb8(0x5A, 0x61, 0x70);
pub const ACCENT: Color = Color::from_rgb8(0x5B, 0x8C, 0xFF);
pub const DANGER: Color = Color::from_rgb8(0xE5, 0x48, 0x4D);

pub const POPPINS: Font = Font::with_name("Poppins");
pub const POPPINS_MEDIUM: Font = Font { weight: Weight::Medium, ..POPPINS };
pub const POPPINS_SEMIBOLD: Font = Font { weight: Weight::Semibold, ..POPPINS };
pub const INTER: Font = Font::with_name("Inter");
pub const ROBOTO: Font = Font::with_name("Roboto");
pub const HIND: Font = Font::with_name("Hind Siliguri");
pub const HIND_SEMIBOLD: Font = Font { weight: Weight::Semibold, ..HIND };

pub fn theme() -> Theme {
    Theme::custom(
        "Studio",
        Palette { background: BG, text: TEXT, primary: ACCENT, success: Color::from_rgb8(0x3D, 0xD6, 0x8C), warning: Color::from_rgb8(0xF5, 0xB7, 0x4A), danger: DANGER },
    )
}

fn radius(r: f32) -> Radius {
    r.into()
}

pub fn root(_: &Theme) -> container::Style {
    container::Style { background: Some(BG.into()), text_color: Some(TEXT), ..Default::default() }
}

pub fn title_bar(_: &Theme) -> container::Style {
    container::Style {
        background: Some(TITLE.into()),
        border: Border { color: BORDER, width: 0.0, radius: radius(0.0) },
        ..Default::default()
    }
}

pub fn pane(_: &Theme) -> container::Style {
    container::Style {
        background: Some(PANEL.into()),
        border: Border { color: BORDER, width: 1.0, radius: radius(10.0) },
        ..Default::default()
    }
}

pub fn pane_header(_: &Theme) -> container::Style {
    container::Style {
        background: Some(PANEL.into()),
        border: Border { color: BORDER, width: 0.0, radius: Radius { top_left: 10.0, top_right: 10.0, bottom_right: 0.0, bottom_left: 0.0 } },
        ..Default::default()
    }
}

pub fn card(_: &Theme) -> container::Style {
    container::Style {
        background: Some(PANEL_2.into()),
        border: Border { color: BORDER, width: 1.0, radius: radius(8.0) },
        ..Default::default()
    }
}

pub fn preview_well(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Color::from_rgb8(0x08, 0x09, 0x0C).into()),
        border: Border { color: BORDER, width: 1.0, radius: radius(8.0) },
        ..Default::default()
    }
}

pub fn chip(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Color { a: 0.72, ..Color::BLACK }.into()),
        text_color: Some(TEXT),
        border: Border { color: Color { a: 0.12, ..Color::WHITE }, width: 1.0, radius: radius(6.0) },
        ..Default::default()
    }
}

pub fn menu(_: &Theme) -> container::Style {
    container::Style {
        background: Some(PANEL_2.into()),
        text_color: Some(TEXT),
        border: Border { color: PANEL_3, width: 1.0, radius: radius(10.0) },
        shadow: Shadow { color: Color { a: 0.5, ..Color::BLACK }, offset: Vector::new(0.0, 8.0), blur_radius: 24.0 },
        ..Default::default()
    }
}

pub fn track_header(selected: bool) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(if selected { PANEL_3 } else { PANEL_2 }.into()),
        border: Border { color: BORDER, width: 1.0, radius: radius(6.0) },
        ..Default::default()
    }
}

pub fn ghost(_: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => Some(Background::Color(PANEL_3)),
        button::Status::Pressed => Some(Background::Color(BORDER)),
        _ => None,
    };
    button::Style { background: bg, text_color: TEXT, border: Border { radius: radius(6.0), ..Default::default() }, ..Default::default() }
}

pub fn menu_item(danger: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: hovered.then(|| Background::Color(if danger { Color { a: 0.16, ..DANGER } } else { PANEL_3 })),
            text_color: if danger { DANGER } else { TEXT },
            border: Border { radius: radius(6.0), ..Default::default() },
            ..Default::default()
        }
    }
}

pub fn primary(_: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => Color::from_rgb8(0x6E, 0x9A, 0xFF),
        button::Status::Pressed => Color::from_rgb8(0x4A, 0x78, 0xE8),
        _ => ACCENT,
    };
    button::Style { background: Some(bg.into()), text_color: Color::WHITE, border: Border { radius: radius(8.0), ..Default::default() }, ..Default::default() }
}

/// Window controls; the close button turns red on hover like on Windows.
pub fn window_control(close: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let bg = match status {
            button::Status::Hovered if close => Some(DANGER),
            button::Status::Pressed if close => Some(Color::from_rgb8(0xC4, 0x2B, 0x30)),
            button::Status::Hovered => Some(PANEL_3),
            button::Status::Pressed => Some(BORDER),
            _ => None,
        };
        button::Style { background: bg.map(Background::Color), text_color: TEXT, ..Default::default() }
    }
}

pub fn input(_: &Theme, status: text_input::Status) -> text_input::Style {
    let border = match status {
        text_input::Status::Focused { .. } => ACCENT,
        text_input::Status::Hovered => PANEL_3,
        _ => BORDER,
    };
    text_input::Style {
        background: PANEL_2.into(),
        border: Border { color: border, width: 1.0, radius: radius(8.0) },
        icon: DIM,
        placeholder: FAINT,
        value: TEXT,
        selection: Color { a: 0.35, ..ACCENT },
    }
}

pub fn select(_: &Theme, status: pick_list::Status) -> pick_list::Style {
    let border = match status {
        pick_list::Status::Opened { .. } => ACCENT,
        pick_list::Status::Hovered => PANEL_3,
        _ => BORDER,
    };
    pick_list::Style {
        text_color: TEXT,
        placeholder_color: FAINT,
        handle_color: DIM,
        background: PANEL_2.into(),
        border: Border { color: border, width: 1.0, radius: radius(8.0) },
    }
}
