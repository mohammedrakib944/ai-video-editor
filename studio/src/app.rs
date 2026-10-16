//! Top-level state: which screen is showing, window state, settings and project files.

use std::path::{Path, PathBuf};

use iced::keyboard::{self, Key, key};
use iced::widget::{button, column, container, row, space, text};
use iced::window;
use iced::{Alignment, Element, Event, Fill, Subscription, Task, event};
use iced_aw::DropDown;

use crate::editor::{self, Editor, SideTab};
use crate::model::{FILE_EXTENSION, Project};
use crate::settings::{self, Settings};
use crate::start::{self, Start};
use crate::ui::chrome::{self, WindowAction};
use crate::ui::icons::{code, icon};
use crate::ui::tokens::*;
use crate::ui::widgets::{self, MenuItem};
use crate::ui::style;

enum Screen {
    Start(Start),
    Editor(Box<Editor>),
}

#[derive(Debug, Clone)]
pub enum Message {
    Window(WindowAction),
    WindowResized,
    Maximized(bool),
    Start(start::Message),
    Editor(editor::Message),
    AppMenu(bool),
    NewProject,
    Open,
    OpenPicked(Option<PathBuf>),
    Save,
    SaveAs,
    SavePicked(Option<PathBuf>),
    CloseProject,
    ToggleAssistant,
    Quit,
    Mcp(crate::mcp::Call),
}

pub struct App {
    settings: Settings,
    screen: Screen,
    maximized: bool,
    app_menu: bool,
    error: Option<String>,
}

impl App {
    pub fn new() -> (App, Task<Message>) {
        if let Err(e) = crate::mcp::start() {
            eprintln!("assistant server: {e}");
        }
        let error = (!crate::ffmpeg::is_available()).then(|| format!("FFmpeg was not found, so media can't be imported or previewed. {}", crate::ffmpeg::missing_hint()));
        let app = App { settings: Settings::load(), screen: Screen::Start(Start::default()), maximized: false, app_menu: false, error };
        let path = std::env::args().nth(1).map(PathBuf::from).filter(|p| p.is_file());
        let task = match path {
            Some(p) => Task::done(Message::OpenPicked(Some(p))),
            None => Task::none(),
        };
        (app, task)
    }

    pub fn title(&self) -> String {
        match &self.screen {
            Screen::Start(_) => "AI Video Editor".into(),
            Screen::Editor(e) => format!("{}{} — AI Video Editor", e.project().name, if e.doc.is_dirty() { " •" } else { "" }),
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let editor = match &self.screen {
            Screen::Editor(e) => e.subscription().map(Message::Editor),
            Screen::Start(_) => Subscription::none(),
        };
        Subscription::batch([window::resize_events().map(|_| Message::WindowResized), event::listen_with(shortcut), editor, crate::mcp::subscription().map(Message::Mcp)])
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Window(action) => return window_task(action).chain(Task::done(Message::WindowResized)),
            Message::WindowResized => return window::latest().and_then(window::is_maximized).map(Message::Maximized),
            Message::Maximized(m) => self.maximized = m,
            Message::Start(msg) => {
                if let Screen::Start(start) = &mut self.screen {
                    start.update(&msg);
                }
                match msg {
                    start::Message::Create => return self.create_project(),
                    start::Message::Open => return self.update(Message::Open),
                    start::Message::OpenRecent(p) => return self.open_project(&p),
                    start::Message::ForgetRecent(p) => {
                        self.settings.recent.retain(|r| r.path != p);
                        self.settings.save();
                    }
                    _ => {}
                }
            }
            Message::Editor(msg) => {
                if let Screen::Editor(e) = &mut self.screen {
                    let resized = matches!(msg, editor::Message::PaneResized(_));
                    let task = e.update(msg).map(Message::Editor);
                    if resized {
                        self.settings.layout = e.layout();
                    }
                    return task;
                }
            }
            Message::AppMenu(open) => self.app_menu = open,
            Message::NewProject => {
                self.app_menu = false;
                self.close_project();
            }
            Message::Open => {
                self.app_menu = false;
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_title("Open project")
                            .add_filter("AI Video Editor project", &[FILE_EXTENSION])
                            .pick_file()
                            .await
                            .map(|h| h.path().to_path_buf())
                    },
                    Message::OpenPicked,
                );
            }
            Message::OpenPicked(Some(p)) => return self.open_project(&p),
            Message::OpenPicked(None) | Message::SavePicked(None) => {}
            Message::Save => {
                self.app_menu = false;
                let has_path = matches!(&self.screen, Screen::Editor(e) if e.path.is_some());
                if has_path {
                    self.save_editor(None);
                } else if matches!(self.screen, Screen::Editor(_)) {
                    return self.update(Message::SaveAs);
                }
            }
            Message::SaveAs => {
                self.app_menu = false;
                let Screen::Editor(e) = &self.screen else { return Task::none() };
                let name = format!("{}.{FILE_EXTENSION}", e.project().name);
                let dir = settings::projects_dir();
                return Task::perform(
                    async move {
                        let _ = std::fs::create_dir_all(&dir);
                        rfd::AsyncFileDialog::new()
                            .set_title("Save project")
                            .set_directory(&dir)
                            .set_file_name(name)
                            .add_filter("AI Video Editor project", &[FILE_EXTENSION])
                            .save_file()
                            .await
                            .map(|h| h.path().to_path_buf())
                    },
                    Message::SavePicked,
                );
            }
            Message::SavePicked(Some(p)) => self.save_editor(Some(p.with_extension(FILE_EXTENSION))),
            Message::CloseProject => {
                self.app_menu = false;
                self.close_project();
            }
            Message::ToggleAssistant => {
                if let Screen::Editor(e) = &mut self.screen {
                    e.side_tab = if e.side_tab == SideTab::Assistant { SideTab::Inspector } else { SideTab::Assistant };
                }
            }
            Message::Mcp(call) => match &mut self.screen {
                Screen::Editor(e) => return e.update(editor::Message::AiTool(call)).map(Message::Editor),
                Screen::Start(_) => call.answer(Err("No project is open. Ask the user to create or open a project first.".into())),
            },
            Message::Quit => {
                self.app_menu = false;
                self.persist();
                return window::latest().and_then(window::close);
            }
        }
        Task::none()
    }

    fn persist(&mut self) {
        if let Screen::Editor(e) = &self.screen {
            self.settings.layout = e.layout();
        }
        self.settings.save();
    }

    fn create_project(&mut self) -> Task<Message> {
        let Screen::Start(start) = &self.screen else { return Task::none() };
        let name = if start.name.trim().is_empty() { "Untitled project".to_string() } else { start.name.trim().to_string() };
        let project = Project::new(&name, start.canvas);
        let dir = settings::projects_dir();
        let path = unique_path(&dir, &name);
        let saved = std::fs::create_dir_all(&dir).map_err(anyhow::Error::from).and_then(|_| project.save(&path));
        let path = match saved {
            Ok(()) => {
                self.settings.touch_recent(&project.name, &path, project.canvas.summary());
                self.settings.save();
                Some(path)
            }
            Err(e) => {
                self.error = Some(format!("Could not save the new project: {e}"));
                None
            }
        };
        self.show_editor(project, path)
    }

    fn show_editor(&mut self, project: Project, path: Option<PathBuf>) -> Task<Message> {
        let mut editor = Editor::new(project, path, self.settings.layout);
        let task = editor.boot().map(Message::Editor);
        self.screen = Screen::Editor(Box::new(editor));
        task
    }

    fn open_project(&mut self, path: &Path) -> Task<Message> {
        match Project::load(path) {
            Ok(project) => {
                self.settings.touch_recent(&project.name, path, project.canvas.summary());
                self.settings.save();
                self.error = None;
                self.show_editor(project, Some(path.to_path_buf()))
            }
            Err(e) => {
                self.error = Some(format!("Could not open {}: {e}", path.display()));
                Task::none()
            }
        }
    }

    fn save_editor(&mut self, new_path: Option<PathBuf>) {
        let Screen::Editor(e) = &mut self.screen else { return };
        if let Some(p) = new_path {
            e.path = Some(p);
        }
        let Some(path) = e.path.clone() else { return };
        match e.doc.project.save(&path) {
            Ok(()) => {
                e.doc.mark_saved();
                self.settings.touch_recent(&e.doc.project.name, &path, e.doc.project.canvas.summary());
                self.settings.layout = e.layout();
                self.settings.save();
            }
            Err(err) => self.error = Some(format!("Could not save: {err}")),
        }
    }

    fn close_project(&mut self) {
        self.persist();
        self.screen = Screen::Start(Start::default());
    }

    pub fn view(&self) -> Element<'_, Message> {
        let (left, center, right): (Element<'_, Message>, Element<'_, Message>, Element<'_, Message>) = match &self.screen {
            Screen::Start(_) => (
                row![chrome::logo(), text("AI Video Editor").font(INTER_SEMIBOLD).size(T_BODY)].spacing(S2 + 2.0).align_y(Alignment::Center).into(),
                space().into(),
                space().into(),
            ),
            Screen::Editor(e) => self.editor_title(e),
        };
        let title_bar = chrome::title_bar(left, center, right, self.maximized, Message::Window);
        let body: Element<'_, Message> = match &self.screen {
            Screen::Start(s) => s.view(&self.settings.recent).map(Message::Start),
            Screen::Editor(e) => e.view().map(Message::Editor),
        };
        let mut content = column![title_bar, body];
        if let Some(err) = &self.error {
            content = content.push(
                container(row![icon(code::TRIANGLE_ALERT, 14).color(DANGER), text(err).size(T_SMALL).color(TEXT_1)].spacing(S2).align_y(Alignment::Center))
                    .padding([8, 14])
                    .width(Fill)
                    .style(style::badge(DANGER)),
            );
        }
        let mut window_content: Element<'_, Message> = content.into();
        if let Screen::Editor(e) = &self.screen {
            if let Some(asset) = e.dnd.and_then(|id| e.project().asset(id)) {
                let ghost = container(row![icon(code::FILM, 13), text(&asset.name).size(T_SMALL)].spacing(6).align_y(Alignment::Center)).padding([6, 10]).style(style::popover);
                let inset = if chrome::draws_frame(self.maximized) { chrome::SHADOW_INSET + 1.0 } else { 0.0 };
                window_content = iced::widget::stack![
                    window_content,
                    iced::widget::pin(iced::widget::opaque(ghost)).x(e.cursor.x - inset + 12.0).y(e.cursor.y - inset + 10.0),
                ]
                .into();
            }
        }
        chrome::frame(window_content, self.maximized, Message::Window)
    }

    fn editor_title<'a>(&'a self, e: &'a Editor) -> (Element<'a, Message>, Element<'a, Message>, Element<'a, Message>) {
        let menu_button = button(row![chrome::logo(), icon(code::CHEVRON_DOWN, 13).color(TEXT_3)].spacing(5).align_y(Alignment::Center))
            .padding([4, 6])
            .style(style::ghost)
            .on_press(Message::AppMenu(!self.app_menu));
        let menu = widgets::menu(
            vec![
                MenuItem::new(code::FILE_PLUS, "New project", Message::NewProject).shortcut("Ctrl+N"),
                MenuItem::new(code::FOLDER_OPEN, "Open…", Message::Open).shortcut("Ctrl+O"),
                MenuItem::Separator,
                MenuItem::new(code::SAVE, "Save", Message::Save).shortcut("Ctrl+S"),
                MenuItem::new(code::COPY, "Save as…", Message::SaveAs).shortcut("Ctrl+Shift+S"),
                MenuItem::Separator,
                MenuItem::new(code::X, "Close project", Message::CloseProject),
                MenuItem::new(code::ARROW_UP_RIGHT, "Quit", Message::Quit).shortcut("Ctrl+Q"),
            ],
            230.0,
        );
        let dropdown = DropDown::new(menu_button, menu, self.app_menu)
            .on_dismiss(Message::AppMenu(false))
            .alignment(iced_aw::drop_down::Alignment::BottomEnd)
            .offset(6.0);
        let status = if e.doc.is_dirty() { ("Unsaved", WARNING) } else if e.path.is_some() { ("Saved", TEXT_4) } else { ("Not saved", WARNING) };
        let left = row![
            dropdown,
            text(&e.doc.project.name).font(INTER_SEMIBOLD).size(T_BODY),
            container(row![container(space().width(6).height(6)).style(style::swatch(status.1, 3.0)), text(status.0).size(T_CAPTION).color(TEXT_3)].spacing(5).align_y(Alignment::Center))
                .padding([0, 4]),
        ]
        .spacing(S2)
        .align_y(Alignment::Center);
        let center = container(row![icon(code::MONITOR, 12).color(TEXT_3), text(e.doc.project.canvas.summary()).font(INTER_MEDIUM).size(T_SMALL).color(TEXT_2)].spacing(6).align_y(Alignment::Center))
            .padding([5, 10])
            .style(style::segmented_track);
        let pos = iced::widget::tooltip::Position::Bottom;
        let right = row![
            widgets::tip(widgets::icon_button(code::UNDO_2, e.doc.can_undo().then_some(Message::Editor(editor::Message::Undo))), e.doc.undo_label().unwrap_or("Undo"), Some("Ctrl+Z"), pos),
            widgets::tip(widgets::icon_button(code::REDO_2, e.doc.can_redo().then_some(Message::Editor(editor::Message::Redo))), e.doc.redo_label().unwrap_or("Redo"), Some("Ctrl+Shift+Z"), pos),
            container(widgets::vdivider(18.0)).padding([0, 6]),
            widgets::tip(widgets::icon_toggle(code::SPARKLES, e.side_tab == SideTab::Assistant, Message::ToggleAssistant), "Assistant", Some("Ctrl+K"), pos),
            space().width(6),
            widgets::tip(widgets::primary_button(Some(code::DOWNLOAD), "Export", Some(Message::Editor(editor::Message::OpenExport))), "Export video", Some("Ctrl+E"), pos),
        ]
        .spacing(2)
        .align_y(Alignment::Center);
        (left.into(), center.into(), right.into())
    }
}

fn window_task(action: WindowAction) -> Task<Message> {
    match action {
        WindowAction::Drag => window::latest().and_then(window::drag),
        WindowAction::ToggleMaximize => window::latest().and_then(window::toggle_maximize),
        WindowAction::Minimize => window::latest().and_then(|id| window::minimize(id, true)),
        WindowAction::Close => Task::done(Message::Quit),
        WindowAction::Resize(dir) => window::latest().and_then(move |id| window::drag_resize(id, dir)),
    }
}

fn shortcut(event: Event, status: event::Status, _window: window::Id) -> Option<Message> {
    let Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) = event else { return None };
    if status == event::Status::Captured || !modifiers.command() {
        return None;
    }
    match key.as_ref() {
        Key::Character("n") => Some(Message::NewProject),
        Key::Character("o") => Some(Message::Open),
        Key::Character("s") if modifiers.shift() => Some(Message::SaveAs),
        Key::Character("s") => Some(Message::Save),
        Key::Character("k") => Some(Message::ToggleAssistant),
        Key::Character("e") => Some(Message::Editor(editor::Message::OpenExport)),
        Key::Character("q") => Some(Message::Quit),
        Key::Named(key::Named::Escape) => Some(Message::AppMenu(false)),
        _ => None,
    }
}

/// `dir/name.aivp`, or `dir/name (2).aivp` if taken.
fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let safe: String = name.chars().map(|c| if r#"<>:"/\|?*"#.contains(c) { '_' } else { c }).collect();
    let mut path = dir.join(format!("{safe}.{FILE_EXTENSION}"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{safe} ({n}).{FILE_EXTENSION}"));
        n += 1;
    }
    path
}
