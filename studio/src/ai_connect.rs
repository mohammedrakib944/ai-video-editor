//! The Connect AI dialog: choose which AI service the assistant uses.

use iced::widget::{button, column, container, mouse_area, opaque, row, space, text};
use iced::{Alignment, Element, Fill};

use crate::settings::AiProvider;
use crate::ui::icons::{code, icon, tinted};
use crate::ui::tokens::*;
use crate::ui::{style, widgets};

#[derive(Debug, Clone)]
pub enum Message {
    Close,
    Connect(AiProvider),
    Disconnect,
    CopyUrl,
}

/// What the dialog knows about the connection.
pub struct State<'a> {
    pub connected: Option<AiProvider>,
    pub checking: bool,
    /// Version of the connected tool, or why connecting failed.
    pub detail: Option<&'a Result<String, String>>,
    pub mcp_url: String,
}

/// A small square mark with the service's initial, for the provider rows.
fn mark<'a, M: 'a>(letter: &'a str, active: bool) -> Element<'a, M> {
    container(text(letter).font(INTER_BOLD).size(T_LARGE).color(if active { ON_ACCENT } else { TEXT_2 }))
        .center(38)
        .style(move |t| if active { style::logo(t) } else { style::card(t) })
        .into()
}

fn status_chip<'a, M: 'a>(label: &'a str, color: iced::Color) -> Element<'a, M> {
    container(row![container(space().width(6).height(6)).style(style::swatch(color, 3.0)), text(label).size(T_CAPTION).color(TEXT_2)].spacing(6).align_y(Alignment::Center))
        .padding([3, 8])
        .style(style::segmented_track)
        .into()
}

pub fn view<'a>(s: State<'a>) -> Element<'a, Message> {
    let claude_connected = s.connected == Some(AiProvider::ClaudeCode);
    let claude_status: Element<'a, Message> = match (claude_connected, s.checking, s.detail) {
        (_, true, _) => status_chip("Checking…", ACCENT_HOVER),
        (true, _, Some(Ok(v))) => status_chip(v, SUCCESS),
        (true, _, _) => status_chip("Connected", SUCCESS),
        (false, _, Some(Err(_))) => status_chip("Not found", DANGER),
        _ => status_chip("Not connected", TEXT_4),
    };
    let claude_action: Element<'a, Message> = if claude_connected {
        widgets::secondary_button(None, "Disconnect", Some(Message::Disconnect)).into()
    } else {
        widgets::primary_button(Some(code::LINK), "Connect", (!s.checking).then_some(Message::Connect(AiProvider::ClaudeCode))).into()
    };
    let mut claude = column![
        row![
            mark("C", claude_connected),
            column![
                row![text("Claude Code").font(INTER_SEMIBOLD).size(T_LARGE), claude_status].spacing(S2).align_y(Alignment::Center),
                text("Uses your Claude subscription through the Claude Code app. No API key.").size(T_SMALL).color(TEXT_3),
            ]
            .spacing(4)
            .width(Fill),
            claude_action,
        ]
        .spacing(S3)
        .align_y(Alignment::Center),
    ]
    .spacing(S2);
    if let (false, Some(Err(e))) = (claude_connected, s.detail) {
        claude = claude.push(container(text(e).size(T_SMALL).color(TEXT_2)).padding([8, 10]).width(Fill).style(style::badge(DANGER)));
    }
    let chatgpt = row![
        mark("G", false),
        column![
            row![text("ChatGPT").font(INTER_SEMIBOLD).size(T_LARGE), status_chip("Coming next", TEXT_4)].spacing(S2).align_y(Alignment::Center),
            text("Through OpenAI's Codex app with your ChatGPT plan, or an OpenAI API key.").size(T_SMALL).color(TEXT_3),
        ]
        .spacing(4)
        .width(Fill),
    ]
    .spacing(S3)
    .align_y(Alignment::Center);

    let other = column![
        row![tinted(code::LINK, 13, TEXT_3), text("Other AI apps").font(INTER_SEMIBOLD).size(T_SMALL).color(TEXT_2)].spacing(7).align_y(Alignment::Center),
        text("Claude Desktop, Cursor and other MCP apps can edit this project too, while the editor is open:").size(T_SMALL).color(TEXT_3),
        row![
            container(text(s.mcp_url).font(INTER_MEDIUM).size(T_SMALL)).padding([7, 10]).width(Fill).style(style::inset),
            widgets::secondary_button(Some(code::COPY), "Copy", Some(Message::CopyUrl)),
        ]
        .spacing(S2)
        .align_y(Alignment::Center),
    ]
    .spacing(S2);

    let card = container(
        column![
            row![
                container(icon(code::SPARKLES, 16).color(ON_ACCENT)).center(34).style(style::logo),
                column![text("Connect AI").font(INTER_SEMIBOLD).size(T_TITLE), text("Pick the AI that edits with you. Everything it does can be undone.").size(T_SMALL).color(TEXT_3)].spacing(2).width(Fill),
                widgets::icon_button(code::X, Some(Message::Close)),
            ]
            .spacing(S3)
            .align_y(Alignment::Center),
            container(claude).padding(S4).width(Fill).style(style::card),
            container(chatgpt).padding(S4).width(Fill).style(|t| {
                let mut st = style::card(t);
                st.background = Some(SURFACE_1.into());
                st
            }),
            widgets::divider(),
            other,
        ]
        .spacing(S4),
    )
    .width(540)
    .padding(S6)
    .style(style::popover);
    let scrim = mouse_area(container(space().width(Fill).height(Fill)).style(style::scrim)).on_press(Message::Close);
    iced::widget::stack![scrim, container(opaque(card)).center(Fill)].into()
}

/// The title-bar chip that shows which AI is connected.
pub fn chip<'a, M: Clone + 'a>(connected: Option<AiProvider>, on_press: M) -> Element<'a, M> {
    let (label, color) = match connected {
        Some(p) => (p.name(), ACCENT),
        None => ("Connect AI", TEXT_4),
    };
    let content = row![container(space().width(7).height(7)).style(style::swatch(color, 3.5)), text(label).font(INTER_MEDIUM).size(T_SMALL)].spacing(7).align_y(Alignment::Center);
    button(content).padding([5, 10]).style(style::secondary).on_press(on_press).into()
}
