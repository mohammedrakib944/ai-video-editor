// Hide the console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod assistant;
mod clip;
mod doc;
mod edit;
mod editor;
mod ffmpeg;
mod fonts;
mod mcp;
mod media;
mod model;
mod platform;
mod playback;
mod render;
mod settings;
mod sfx;
mod start;
mod ui;

use iced::{Size, window};

use crate::ui::tokens;

fn main() -> iced::Result {
    let mut application = iced::application(app::App::new, app::App::update, app::App::view)
        .title(app::App::title)
        .subscription(app::App::subscription)
        .theme(theme())
        .default_font(tokens::INTER)
        .antialiasing(true)
        .window(window_settings());
    for font in fonts::BUNDLED {
        application = application.font(font);
    }
    application.run()
}

fn theme() -> iced::Theme {
    iced::Theme::custom(
        "Studio",
        iced::theme::Palette {
            background: tokens::WINDOW,
            text: tokens::TEXT_1,
            primary: tokens::ACCENT,
            success: tokens::SUCCESS,
            warning: tokens::WARNING,
            danger: tokens::DANGER,
        },
    )
}

fn window_settings() -> window::Settings {
    window::Settings {
        size: Size::new(1480.0, 920.0),
        min_size: Some(Size::new(1100.0, 680.0)),
        decorations: false,
        transparent: false,
        platform_specific: platform_specific(),
        ..Default::default()
    }
}

#[cfg(windows)]
fn platform_specific() -> window::settings::PlatformSpecific {
    window::settings::PlatformSpecific {
        undecorated_shadow: true,
        corner_preference: window::settings::platform::CornerPreference::Round,
        ..Default::default()
    }
}

#[cfg(target_os = "linux")]
fn platform_specific() -> window::settings::PlatformSpecific {
    window::settings::PlatformSpecific { application_id: "ai-video-editor".into(), ..Default::default() }
}

#[cfg(not(any(windows, target_os = "linux")))]
fn platform_specific() -> window::settings::PlatformSpecific {
    Default::default()
}
