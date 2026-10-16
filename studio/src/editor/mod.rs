//! The editor screen: library, preview, inspector and timeline in resizable panes.

mod canvas_overlay;
pub mod export_dialog;
pub mod inspector;
mod library;
pub mod tools;
pub mod media_cache;
pub mod preview;
pub mod timeline;

use std::cell::Cell;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use iced::keyboard::{self, Key, key};
use iced::widget::{button, column, container, mouse_area, pane_grid, responsive, row, space, stack, text, text_input};
use iced::{Alignment, Color, Element, Event, Fill, Length, Point, Size, Subscription, Task, event, mouse, window};
use iced_aw::ContextMenu;

use self::inspector::Edit;
use self::media_cache::{AssetMedia, MediaCache};
use self::timeline::{HEADER_W, Lanes, RULER_H, Ruler, View};
use crate::clip::{TextLayer, Zoom};
use crate::doc::Doc;
use crate::edit::Edge;
use crate::media::{self, ProbeInfo};
use crate::model::{Id, MediaKind, Project, TrackKind};
use crate::render::frames::FrameServer;
use crate::render::scene;
use crate::settings::Layout;
use crate::ui::icons::{code, icon, tinted};
use crate::ui::tokens::*;
use crate::ui::widgets::{self, MenuItem};
use crate::ui::style;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Library,
    Preview,
    Inspector,
    Timeline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryTab {
    Media,
    Text,
    Transitions,
    Sounds,
}

impl LibraryTab {
    const ALL: [LibraryTab; 4] = [LibraryTab::Media, LibraryTab::Text, LibraryTab::Transitions, LibraryTab::Sounds];

    fn label(self) -> &'static str {
        match self {
            LibraryTab::Media => "Media",
            LibraryTab::Text => "Text",
            LibraryTab::Transitions => "Effects",
            LibraryTab::Sounds => "Sounds",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideTab {
    Inspector,
    Assistant,
}

/// Timeline tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Select,
    Blade,
    Range,
}

#[derive(Debug, Clone, Copy)]
pub enum TrackAction {
    Rename,
    ToggleHidden,
    ToggleLocked,
    ToggleMuted,
    MoveUp,
    MoveDown,
    Delete,
    AddVideo,
    AddAudio,
}

#[derive(Debug, Clone)]
pub enum Message {
    PaneResized(pane_grid::ResizeEvent),
    LibraryTab(LibraryTab),
    SideTab(SideTab),
    Tool(Tool),
    TogglePlay,
    ToggleMute,
    ToggleSnapping,
    Import,
    ImportPicked(Vec<PathBuf>),
    Probed(PathBuf, Result<ProbeInfo, String>),
    MediaLoaded(Id, AssetMedia),
    LibraryPress(Id),
    LibraryAdd(Id),
    LibraryRemove(Id),
    CursorMoved(Point),
    MouseReleased,
    Seek(f64),
    SetView(View),
    ZoomBy(f64),
    ZoomFit,
    Select(Option<Id>, bool),
    MoveClip { clip: Id, track: Id, start: f64 },
    Trim { clip: Id, edge: Edge, t: f64 },
    Split(Id, f64),
    SplitAtPlayhead,
    DeleteSelection,
    DuplicateSelection,
    SetRange(Option<(f64, f64)>),
    DropAsset { asset: Id, track: Option<Id>, t: f64 },
    Edit(Id, Edit),
    Undo,
    Redo,
    Tick(Instant),
    AddText,
    AddShape,
    ApplyTransition(crate::clip::TransitionKind),
    ApplyMotion { intro: bool, kind: crate::clip::MotionKind },
    AddSound(&'static str),
    SoundReady(&'static str, Result<(PathBuf, ProbeInfo), String>),
    PreviewSound(&'static str),
    SoundPreviewReady(&'static str, Result<(PathBuf, ProbeInfo), String>),
    SoundPreviewTick,
    ZoomEdit(Option<(Id, Id)>),
    AiTool(crate::mcp::Call),
    ToolImported(crate::mcp::Call, Vec<(PathBuf, Result<ProbeInfo, String>)>, bool),
    ToolSound(crate::mcp::Call, &'static str, Result<(PathBuf, ProbeInfo), String>),
    Track(Id, TrackAction),
    RenameInput(String),
    RenameCommit,
    Step(i32),
    GoStart,
    GoEnd,
    Escape,
    DismissToast,
    OpenExport,
    CloseExport,
    ExportPath(String),
    ExportBrowse,
    ExportBrowsed(Option<PathBuf>),
    ExportQuality(crate::render::export::Quality),
    StartExport,
    ExportProgress(crate::render::export::Progress),
    CancelExport,
    ExportRetry,
    RevealExport,
    PlayExport,
    Noop,
}

pub struct Editor {
    pub doc: Doc,
    pub path: Option<PathBuf>,
    panes: pane_grid::State<Pane>,
    library_tab: LibraryTab,
    pub side_tab: SideTab,
    tool: Tool,
    playing: Option<(Instant, f64)>,
    snapping: bool,
    playhead: f64,
    view: View,
    selection: Vec<Id>,
    range: Option<(f64, f64)>,
    media: MediaCache,
    lanes_cache: timeline::Caches,
    importing: Vec<PathBuf>,
    library_selected: Option<Id>,
    /// Library item pressed (drag starts once the cursor moves).
    press: Option<(Id, Point)>,
    pub dnd: Option<Id>,
    pub cursor: Point,
    renaming: Option<(Id, String)>,
    toast: Option<(String, bool, Instant)>,
    timeline_size: Cell<Size>,
    frames: FrameServer,
    export: Option<export_dialog::ExportDialog>,
    audio: Option<crate::playback::AudioPlayer>,
    audio_revision: u64,
    muted: bool,
    sound_preview: Option<(&'static str, crate::playback::AudioPlayer, Instant)>,
    zoom_edit: Option<(Id, Id)>,
}

impl Editor {
    pub fn new(project: Project, path: Option<PathBuf>, layout: Layout) -> Editor {
        use pane_grid::{Axis, Configuration as C};
        let config = C::Split {
            axis: Axis::Horizontal,
            ratio: layout.top_bottom,
            a: Box::new(C::Split {
                axis: Axis::Vertical,
                ratio: layout.library,
                a: Box::new(C::Pane(Pane::Library)),
                b: Box::new(C::Split {
                    axis: Axis::Vertical,
                    ratio: layout.preview_inspector,
                    a: Box::new(C::Pane(Pane::Preview)),
                    b: Box::new(C::Pane(Pane::Inspector)),
                }),
            }),
            b: Box::new(C::Pane(Pane::Timeline)),
        };
        Editor {
            doc: Doc::new(project),
            path,
            panes: pane_grid::State::with_configuration(config),
            library_tab: LibraryTab::Media,
            side_tab: SideTab::Inspector,
            tool: Tool::Select,
            playing: None,
            snapping: true,
            playhead: 0.0,
            view: View::default(),
            selection: Vec::new(),
            range: None,
            media: MediaCache::default(),
            lanes_cache: timeline::Caches::default(),
            importing: Vec::new(),
            library_selected: None,
            press: None,
            dnd: None,
            cursor: Point::ORIGIN,
            renaming: None,
            toast: None,
            timeline_size: Cell::new(Size::new(1000.0, 300.0)),
            frames: FrameServer::default(),
            export: None,
            audio: None,
            audio_revision: 0,
            muted: false,
            sound_preview: None,
            zoom_edit: None,
        }
    }

    pub fn project(&self) -> &Project {
        &self.doc.project
    }

    /// Starts loading thumbnails and waveforms for the project's media.
    pub fn boot(&mut self) -> Task<Message> {
        let assets = self.doc.project.assets.clone();
        self.media.ensure(&assets, Message::MediaLoaded)
    }

    /// Current split ratios, for saving the layout.
    pub fn layout(&self) -> Layout {
        use pane_grid::Node;
        let mut l = Layout::default();
        if let Node::Split { ratio, a, .. } = self.panes.layout() {
            l.top_bottom = *ratio;
            if let Node::Split { ratio, b, .. } = a.as_ref() {
                l.library = *ratio;
                if let Node::Split { ratio, .. } = b.as_ref() {
                    l.preview_inspector = *ratio;
                }
            }
        }
        l
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let mut subs = vec![event::listen_with(editor_event)];
        // Redraw every frame while playing, or while decoders are still catching up after a seek.
        if self.playing.is_some() || self.frames.busy() {
            subs.push(window::frames().map(Message::Tick));
        }
        if self.toast.is_some() {
            subs.push(iced::time::every(Duration::from_millis(500)).map(|_| Message::DismissToast));
        }
        if self.sound_preview.is_some() {
            subs.push(iced::time::every(Duration::from_millis(250)).map(|_| Message::SoundPreviewTick));
        }
        Subscription::batch(subs)
    }

    fn notify(&mut self, text: impl Into<String>, error: bool) {
        self.toast = Some((text.into(), error, Instant::now()));
    }

    /// Runs an edit; on failure shows the reason instead.
    fn edit<T>(&mut self, desc: &str, f: impl FnOnce(&mut Project) -> anyhow::Result<T>) -> Option<T> {
        let result = self.doc.apply(desc, f);
        self.after_edit(result)
    }

    fn edit_merge<T>(&mut self, desc: &str, key: Option<&str>, f: impl FnOnce(&mut Project) -> anyhow::Result<T>) -> Option<T> {
        let result = self.doc.apply_merge(desc, key, f);
        self.after_edit(result)
    }

    fn after_edit<T>(&mut self, result: anyhow::Result<T>) -> Option<T> {
        match result {
            Ok(v) => {
                self.lanes_cache.clear();
                Some(v)
            }
            Err(e) => {
                self.notify(e.to_string(), true);
                None
            }
        }
    }

    fn seek(&mut self, t: f64) {
        self.playhead = t.max(0.0);
        if self.playing.is_some() {
            self.start_playback();
        }
        self.follow_playhead();
    }

    /// (Re)starts playback from the playhead, with audio if there is any.
    fn start_playback(&mut self) {
        self.sound_preview = None;
        self.playing = Some((Instant::now(), self.playhead));
        self.audio = crate::playback::AudioPlayer::start(&self.doc.project, self.playhead, self.muted).ok();
        self.audio_revision = self.doc.revision;
    }

    fn stop_playback(&mut self) {
        self.playing = None;
        self.audio = None;
    }

    /// Scrolls the timeline so the playhead stays visible.
    fn follow_playhead(&mut self) {
        let width = (self.timeline_size.get().width - HEADER_W).max(100.0) as f64;
        let x = (self.playhead - self.view.scroll) * self.view.zoom;
        if x < 0.0 || x > width - 40.0 {
            self.view.scroll = (self.playhead - width * 0.2 / self.view.zoom).max(0.0);
            self.lanes_cache.clear();
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::PaneResized(pane_grid::ResizeEvent { split, ratio }) => self.panes.resize(split, ratio),
            Message::LibraryTab(t) => self.library_tab = t,
            Message::SideTab(t) => self.side_tab = t,
            Message::Tool(t) => self.tool = t,
            Message::ToggleSnapping => self.snapping = !self.snapping,
            Message::TogglePlay => {
                if self.playing.is_some() {
                    self.stop_playback();
                } else {
                    if self.playhead >= self.doc.project.duration() - 0.01 {
                        self.playhead = 0.0;
                    }
                    self.start_playback();
                }
            }
            Message::ToggleMute => {
                self.muted = !self.muted;
                if let Some(a) = &self.audio {
                    a.set_muted(self.muted);
                }
            }
            Message::Tick(now) => {
                self.frames.settle();
                self.frames.collect_idle();
                if let Some((started, from)) = self.playing {
                    // Edits during playback change the sound: restart the mix where we are.
                    if self.audio.is_some() && self.doc.revision != self.audio_revision {
                        self.start_playback();
                        return Task::none();
                    }
                    let t = match &self.audio {
                        Some(a) => match a.position() {
                            Some(p) => p,
                            None if a.stalled() => {
                                self.audio = None;
                                self.playing = Some((now, from));
                                from
                            }
                            // Wait for the first samples so picture and sound start together.
                            None => {
                                self.playing = Some((now, from));
                                from
                            }
                        },
                        None => from + now.duration_since(started).as_secs_f64(),
                    };
                    let end = self.doc.project.duration();
                    if t >= end {
                        self.playhead = end;
                        self.stop_playback();
                    } else {
                        self.playhead = t;
                    }
                    self.follow_playhead();
                }
            }
            Message::Import => {
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_title("Import media")
                            .add_filter("Video, audio and images", media::MEDIA_EXTENSIONS)
                            .pick_files()
                            .await
                            .map(|files| files.into_iter().map(|f| f.path().to_path_buf()).collect())
                            .unwrap_or_default()
                    },
                    Message::ImportPicked,
                );
            }
            Message::ImportPicked(paths) => {
                let mut tasks = Vec::new();
                for path in paths.into_iter().filter(|p| media::is_media_path(p)) {
                    if self.importing.contains(&path) || self.doc.project.assets.iter().any(|a| a.path == path) {
                        continue;
                    }
                    self.importing.push(path.clone());
                    tasks.push(Task::perform(
                        async move {
                            let p = path.clone();
                            let r = tokio::task::spawn_blocking(move || media::probe(&p).map_err(|e| e.to_string())).await.unwrap_or_else(|e| Err(e.to_string()));
                            (path, r)
                        },
                        |(path, r)| Message::Probed(path, r),
                    ));
                }
                self.library_tab = LibraryTab::Media;
                return Task::batch(tasks);
            }
            Message::Probed(path, result) => {
                self.importing.retain(|p| p != &path);
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                match result {
                    Ok(info) => {
                        if let Some(id) = self.edit(&format!("Import {name}"), |p| Ok(p.add_asset(&path, &info))) {
                            self.library_selected = Some(id);
                            let assets = self.doc.project.assets.clone();
                            return self.media.ensure(&assets, Message::MediaLoaded);
                        }
                    }
                    Err(e) => self.notify(format!("Couldn't import {name}: {e}"), true),
                }
            }
            Message::MediaLoaded(id, m) => {
                self.media.insert(id, m);
                self.lanes_cache.clear();
            }
            Message::LibraryPress(id) => {
                self.library_selected = Some(id);
                self.press = Some((id, self.cursor));
            }
            Message::LibraryAdd(id) => {
                self.press = None;
                let at = self.playhead;
                if let Some(clip) = self.edit("Add to timeline", |p| p.add_media_clip(id, None, at)) {
                    self.selection = vec![clip];
                }
            }
            Message::LibraryRemove(id) => {
                if self.edit("Remove media", |p| p.remove_asset(id)).is_some() && self.library_selected == Some(id) {
                    self.library_selected = None;
                }
            }
            Message::CursorMoved(p) => {
                self.cursor = p;
                if let Some((id, origin)) = self.press {
                    if self.dnd.is_none() && origin.distance(p) > 6.0 {
                        self.dnd = Some(id);
                    }
                }
            }
            Message::MouseReleased => {
                self.press = None;
                self.dnd = None;
            }
            Message::DropAsset { asset, track, t } => {
                self.press = None;
                self.dnd = None;
                // Fall back to the default track if the hovered one doesn't fit this media.
                let fits = |p: &Project| {
                    track.is_some_and(|tr| {
                        let (Some(tk), Some(a)) = (p.track(tr), p.asset(asset)) else { return false };
                        (tk.kind == TrackKind::Audio) == (a.kind == MediaKind::Audio) && !tk.locked
                    })
                };
                if let Some(clip) = self.edit("Add to timeline", |p| {
                    let track = if fits(p) { track } else { None };
                    p.add_media_clip(asset, track, t)
                }) {
                    self.selection = vec![clip];
                }
            }
            Message::Seek(t) => {
                self.seek(t);
            }
            Message::SetView(v) => {
                self.view = v;
                self.lanes_cache.clear();
            }
            Message::ZoomBy(f) => {
                let anchor = self.view.x(self.playhead).clamp(0.0, self.timeline_size.get().width - HEADER_W);
                self.view = self.view.zoomed(f, anchor);
                self.lanes_cache.clear();
            }
            Message::ZoomFit => {
                let width = (self.timeline_size.get().width - HEADER_W - 40.0).max(200.0) as f64;
                let dur = self.doc.project.duration().max(10.0);
                self.view = View { scroll: 0.0, zoom: (width / dur).clamp(2.0, 2400.0) };
                self.lanes_cache.clear();
            }
            Message::Select(id, additive) => {
                match id {
                    Some(id) if additive => {
                        if let Some(i) = self.selection.iter().position(|&s| s == id) {
                            self.selection.remove(i);
                        } else {
                            self.selection.push(id);
                        }
                    }
                    Some(id) => {
                        if !self.selection.contains(&id) {
                            self.selection = vec![id];
                        }
                    }
                    None => self.selection.clear(),
                }
                if !self.selection.is_empty() {
                    self.side_tab = SideTab::Inspector;
                }
                if self.zoom_edit.is_some_and(|(c, _)| !self.selection.contains(&c)) {
                    self.zoom_edit = None;
                }
                self.lanes_cache.clear();
            }
            Message::MoveClip { clip, track, start } => {
                self.edit("Move clip", |p| p.move_clip(clip, track, start));
            }
            Message::Trim { clip, edge, t } => {
                self.edit("Trim clip", |p| p.trim(clip, edge, t));
            }
            Message::Split(clip, t) => {
                if let Some(right) = self.edit("Split clip", |p| p.split(clip, t)) {
                    self.selection = vec![right];
                }
            }
            Message::SplitAtPlayhead => {
                let t = self.playhead;
                let targets: Vec<Id> = if self.selection.is_empty() {
                    self.doc.project.clips.iter().filter(|c| t > c.start && t < c.end()).map(|c| c.id).collect()
                } else {
                    self.selection.clone()
                };
                if targets.is_empty() {
                    self.notify("Move the playhead over a clip to split it", false);
                } else {
                    self.edit("Split", |p| {
                        let mut any = false;
                        for id in targets {
                            if p.clip(id).is_some_and(|c| t > c.start + 0.1 && t < c.end() - 0.1) {
                                p.split(id, t)?;
                                any = true;
                            }
                        }
                        anyhow::ensure!(any, "Move the playhead inside the clip to split it");
                        Ok(())
                    });
                }
            }
            Message::DeleteSelection => {
                let ids = std::mem::take(&mut self.selection);
                if !ids.is_empty() {
                    self.edit(if ids.len() == 1 { "Delete clip" } else { "Delete clips" }, |p| p.delete_clips(&ids));
                }
            }
            Message::DuplicateSelection => {
                let ids = self.selection.clone();
                if let Some(new) = self.edit("Duplicate", |p| ids.iter().map(|&id| p.duplicate_clip(id)).collect::<anyhow::Result<Vec<_>>>()) {
                    self.selection = new;
                }
            }
            Message::SetRange(r) => {
                self.range = r;
                if r.is_some() {
                    self.side_tab = SideTab::Assistant;
                }
            }
            Message::Edit(id, edit) => {
                let (key, desc) = edit.describe();
                let key = format!("{key}:{id}");
                let playhead = self.playhead;
                self.edit_merge(desc, Some(&key), |p| {
                    match edit {
                        Edit::Speed(s) => return p.set_speed(id, s),
                        Edit::AddZoom => {
                            let zid = p.alloc_id();
                            let c = p.clip_mut(id)?;
                            let at = (playhead - c.start).clamp(0.0, (c.duration - 2.0).max(0.0));
                            c.zooms.push(Zoom { id: zid, at, duration: 2.5f64.min(c.duration - at), scale: 1.8, focus_x: 0.5, focus_y: 0.5, ramp: 0.6 });
                            return Ok(());
                        }
                        _ => {}
                    }
                    edit.apply(p.clip_mut(id)?);
                    Ok(())
                });
            }
            Message::Undo => match self.doc.undo() {
                Some(d) => {
                    self.notify(format!("Undid: {d}"), false);
                    self.after_history();
                }
                None => self.notify("Nothing to undo", false),
            },
            Message::Redo => match self.doc.redo() {
                Some(d) => {
                    self.notify(format!("Redid: {d}"), false);
                    self.after_history();
                }
                None => self.notify("Nothing to redo", false),
            },
            Message::AddText => {
                let at = self.playhead;
                if let Some(id) = self.edit("Add text", |p| p.add_text_clip(None, at, crate::edit::DEFAULT_STILL, TextLayer::default())) {
                    self.selection = vec![id];
                    self.side_tab = SideTab::Inspector;
                }
            }
            Message::AddShape => {
                let at = self.playhead;
                let shape = crate::clip::ShapeLayer {
                    kind: crate::clip::ShapeKind::Rectangle,
                    width: 520.0,
                    height: 160.0,
                    fill: crate::clip::Rgba::hex(0x7C6DFF),
                    fill_to: Some(crate::clip::Rgba::hex(0xC56BFF)),
                };
                if let Some(id) = self.edit("Add shape", |p| {
                    let id = p.add_shape_clip(None, at, crate::edit::DEFAULT_STILL, shape)?;
                    p.clip_mut(id)?.look.corner_radius = 48.0;
                    Ok(id)
                }) {
                    self.selection = vec![id];
                    self.side_tab = SideTab::Inspector;
                }
            }
            Message::ApplyTransition(kind) => {
                let ids = self.selection.clone();
                self.edit("Add transition", |p| {
                    for id in &ids {
                        let c = p.clip_mut(*id)?;
                        c.transition = Some(crate::clip::Transition { kind, duration: c.transition.map(|t| t.duration).unwrap_or(0.6).min(c.duration / 2.0) });
                    }
                    Ok(())
                });
            }
            Message::ApplyMotion { intro, kind } => {
                let ids = self.selection.clone();
                self.edit(if intro { "Animate in" } else { "Animate out" }, |p| {
                    for id in &ids {
                        let c = p.clip_mut(*id)?;
                        let slot = if intro { &mut c.intro } else { &mut c.outro };
                        *slot = Some(crate::clip::Motion { kind, duration: slot.map(|m| m.duration).unwrap_or(0.6).min(c.duration / 2.0) });
                    }
                    Ok(())
                });
            }
            Message::AddSound(id) => return sound_task(id, Message::SoundReady),
            Message::SoundReady(_, Err(e)) | Message::SoundPreviewReady(_, Err(e)) => self.notify(e, true),
            Message::SoundReady(id, Ok((path, info))) => {
                let at = self.playhead;
                let name = crate::sfx::SOUNDS.iter().find(|s| s.id == id).map(|s| s.name).unwrap_or("Sound");
                if let Some(clip) = self.edit(&format!("Add {name}"), |p| {
                    let asset = p.add_asset(&path, &info);
                    if let Some(a) = p.assets.iter_mut().find(|a| a.id == asset) {
                        a.name = name.to_string();
                    }
                    p.add_media_clip(asset, None, at)
                }) {
                    self.selection = vec![clip];
                    let assets = self.doc.project.assets.clone();
                    return self.media.ensure(&assets, Message::MediaLoaded);
                }
            }
            Message::PreviewSound(id) => {
                if self.sound_preview.as_ref().is_some_and(|(s, _, _)| *s == id) {
                    self.sound_preview = None;
                } else {
                    return sound_task(id, Message::SoundPreviewReady);
                }
            }
            Message::SoundPreviewReady(id, Ok((path, info))) => {
                let mut p = Project::new("preview", self.doc.project.canvas);
                let asset = p.add_asset(&path, &info);
                if p.add_media_clip(asset, None, 0.0).is_ok() {
                    self.sound_preview = crate::playback::AudioPlayer::start(&p, 0.0, false).ok().map(|a| (id, a, Instant::now() + Duration::from_secs_f64(info.duration + 0.3)));
                }
            }
            Message::SoundPreviewTick => {
                if self.sound_preview.as_ref().is_some_and(|(_, _, until)| Instant::now() > *until) {
                    self.sound_preview = None;
                }
            }
            Message::AiTool(call) => return self.handle_tool(call),
            Message::ToolImported(call, results, add) => return self.finish_tool_import(call, results, add),
            Message::ToolSound(call, sound, r) => return self.finish_tool_sound(call, sound, r),
            Message::ZoomEdit(target) => {
                self.zoom_edit = target;
                if let Some((clip, zoom)) = target {
                    self.stop_playback();
                    self.selection = vec![clip];
                    self.side_tab = SideTab::Inspector;
                    let c = self.doc.project.clip(clip).cloned();
                    if let Some((c, z)) = c.as_ref().and_then(|c| c.zooms.iter().find(|z| z.id == zoom).map(|z| (c, *z))) {
                        // Show a moment inside the zoom so its clip is on screen.
                        let (a, b) = (c.start + z.at, c.start + z.at + z.duration);
                        if self.playhead < a || self.playhead > b {
                            self.seek(a + z.duration / 2.0);
                        }
                    }
                    self.lanes_cache.clear();
                }
            }
            Message::Track(id, action) => self.track_action(id, action),
            Message::RenameInput(s) => {
                if let Some((_, name)) = &mut self.renaming {
                    *name = s;
                }
            }
            Message::RenameCommit => {
                if let Some((id, name)) = self.renaming.take() {
                    self.edit("Rename track", |p| p.rename_track(id, &name));
                }
            }
            Message::Step(frames) => {
                let fps = self.doc.project.canvas.fps as f64;
                let t = ((self.playhead * fps).round() + frames as f64) / fps;
                self.seek(t);
            }
            Message::GoStart => self.seek(0.0),
            Message::GoEnd => {
                let end = self.doc.project.duration();
                self.seek(end);
            }
            Message::Escape => {
                if self.zoom_edit.take().is_some() {
                    return Task::none();
                }
                if self.renaming.take().is_none() {
                    self.range = None;
                    self.selection.clear();
                    self.tool = Tool::Select;
                    self.lanes_cache.clear();
                }
            }
            Message::DismissToast => {
                if self.toast.as_ref().is_some_and(|(_, err, at)| at.elapsed() > Duration::from_secs(if *err { 5 } else { 2 })) {
                    self.toast = None;
                }
            }
            Message::OpenExport => {
                if self.doc.project.duration() <= 0.0 {
                    self.notify("Add something to the timeline before exporting", false);
                } else if self.export.as_ref().is_none_or(|e| !e.is_running()) {
                    self.stop_playback();
                    self.export = Some(export_dialog::ExportDialog::new(&self.doc.project, self.path.as_deref()));
                }
            }
            Message::CloseExport => {
                if self.export.as_ref().is_some_and(|e| !e.is_running()) {
                    self.export = None;
                }
            }
            Message::ExportPath(p) => {
                if let Some(e) = &mut self.export {
                    e.path = p;
                }
            }
            Message::ExportBrowse => {
                let Some(e) = &self.export else { return Task::none() };
                let current = PathBuf::from(&e.path);
                let dir = current.parent().map(|d| d.to_path_buf()).filter(|d| d.is_dir()).unwrap_or_else(crate::settings::home_dir);
                let name = current.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "video.mp4".into());
                return Task::perform(
                    async move {
                        rfd::AsyncFileDialog::new().set_title("Export video").set_directory(dir).set_file_name(name).add_filter("MP4 video", &["mp4"]).save_file().await.map(|h| h.path().to_path_buf())
                    },
                    Message::ExportBrowsed,
                );
            }
            Message::ExportBrowsed(Some(p)) => {
                if let Some(e) = &mut self.export {
                    e.path = p.with_extension("mp4").display().to_string();
                }
            }
            Message::ExportBrowsed(None) => {}
            Message::ExportQuality(q) => {
                if let Some(e) = &mut self.export {
                    e.quality = q;
                }
            }
            Message::StartExport => return self.start_export(),
            Message::ExportProgress(p) => {
                use crate::render::export::Progress;
                if let Some(e) = &mut self.export {
                    match p {
                        Progress::Rendering { done, total, eta } => {
                            if let export_dialog::Phase::Running { done: d, total: t, eta: e2, .. } = &mut e.phase {
                                (*d, *t, *e2) = (done, total, eta);
                            }
                        }
                        Progress::Finished { path, seconds } => e.phase = export_dialog::Phase::Done { path, seconds },
                        Progress::Failed(err) => e.phase = export_dialog::Phase::Failed(err),
                        Progress::Cancelled => e.phase = export_dialog::Phase::Setup,
                    }
                }
            }
            Message::CancelExport => {
                if let Some(export_dialog::ExportDialog { phase: export_dialog::Phase::Running { cancel, .. }, .. }) = &self.export {
                    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
            Message::ExportRetry => {
                if let Some(e) = &mut self.export {
                    e.phase = export_dialog::Phase::Setup;
                }
            }
            Message::RevealExport | Message::PlayExport => {
                if let Some(export_dialog::ExportDialog { phase: export_dialog::Phase::Done { path, .. }, .. }) = &self.export {
                    if matches!(message, Message::RevealExport) {
                        crate::platform::reveal(path);
                    } else {
                        crate::platform::open(path);
                    }
                }
            }
            Message::Noop => {}
        }
        Task::none()
    }

    fn start_export(&mut self) -> Task<Message> {
        use crate::render::export::{self, Progress};
        use iced::futures::SinkExt;
        let Some(dialog) = &mut self.export else { return Task::none() };
        let mut path = PathBuf::from(dialog.path.trim());
        if path.extension().is_none_or(|e| !e.eq_ignore_ascii_case("mp4")) {
            path.set_extension("mp4");
        }
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        dialog.phase = export_dialog::Phase::Running { done: 0, total: 1, eta: None, cancel: cancel.clone() };
        let project = self.doc.project.clone();
        let quality = dialog.quality;
        let stream = iced::stream::channel(64, async move |mut tx: iced::futures::channel::mpsc::Sender<Progress>| {
            let (stx, srx) = std::sync::mpsc::channel::<Progress>();
            std::thread::Builder::new()
                .name("export".into())
                .spawn(move || {
                    let started = std::time::Instant::now();
                    let progress = |p: Progress| {
                        let _ = stx.send(p);
                    };
                    let result = export::export(&project, &path, quality, &cancel, &progress);
                    let last = match result {
                        Ok(path) => Progress::Finished { path, seconds: started.elapsed().as_secs_f64() },
                        Err(e) if e.to_string() == "cancelled" => Progress::Cancelled,
                        Err(e) => Progress::Failed(e.to_string()),
                    };
                    let _ = stx.send(last);
                })
                .ok();
            loop {
                match srx.try_recv() {
                    Ok(p) => {
                        let end = !matches!(p, Progress::Rendering { .. });
                        let _ = tx.send(p).await;
                        if end {
                            break;
                        }
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => tokio::time::sleep(Duration::from_millis(40)).await,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                }
            }
        });
        Task::run(stream, Message::ExportProgress)
    }

    fn after_history(&mut self) {
        let p = &self.doc.project;
        self.selection.retain(|id| p.clip(*id).is_some());
        self.lanes_cache.clear();
    }

    fn track_action(&mut self, id: Id, action: TrackAction) {
        match action {
            TrackAction::Rename => {
                if let Some(t) = self.doc.project.track(id) {
                    self.renaming = Some((id, t.name.clone()));
                }
            }
            TrackAction::ToggleHidden => {
                self.edit("Toggle track visibility", |p| {
                    let t = p.track_mut(id)?;
                    t.hidden = !t.hidden;
                    Ok(())
                });
            }
            TrackAction::ToggleLocked => {
                self.edit("Toggle track lock", |p| {
                    let t = p.track_mut(id)?;
                    t.locked = !t.locked;
                    Ok(())
                });
            }
            TrackAction::ToggleMuted => {
                self.edit("Toggle track mute", |p| {
                    let t = p.track_mut(id)?;
                    t.muted = !t.muted;
                    Ok(())
                });
            }
            TrackAction::MoveUp => {
                self.edit("Move track", |p| p.move_track(id, -1));
            }
            TrackAction::MoveDown => {
                self.edit("Move track", |p| p.move_track(id, 1));
            }
            TrackAction::Delete => {
                self.edit("Delete track", |p| p.delete_track(id));
                self.after_history();
            }
            TrackAction::AddVideo => {
                let index = self.doc.project.track_index(id);
                let n = self.doc.project.tracks.iter().filter(|t| t.kind == TrackKind::Visual).count() + 1;
                self.edit("Add track", |p| Ok(p.add_track(TrackKind::Visual, &format!("Video {n}"), index)));
            }
            TrackAction::AddAudio => {
                let index = self.doc.project.track_index(id).map(|i| i + 1);
                let n = self.doc.project.tracks.iter().filter(|t| t.kind == TrackKind::Audio).count() + 1;
                self.edit("Add track", |p| Ok(p.add_track(TrackKind::Audio, &format!("Audio {n}"), index)));
            }
        }
    }

    // ------------------------------------------------------------ view

    pub fn view(&self) -> Element<'_, Message> {
        let grid = pane_grid(&self.panes, |_, pane, _| {
            let (header, body): (Element<'_, Message>, Element<'_, Message>) = match pane {
                Pane::Library => (self.library_header(), self.library()),
                Pane::Preview => (pane_title("Preview", Some(self.doc.project.canvas.summary())), self.preview()),
                Pane::Inspector => (self.side_header(), self.inspector()),
                Pane::Timeline => (self.timeline_header(), self.timeline()),
            };
            pane_grid::Content::new(container(body).width(Fill).height(Fill))
                .title_bar(pane_grid::TitleBar::new(header).padding([0, 6]).style(style::panel_header))
                .style(style::panel)
        })
        .spacing(5)
        .min_size(160)
        .on_resize(10, Message::PaneResized)
        .style(|_| pane_grid::Style {
            hovered_region: pane_grid::Highlight { background: ACCENT_SOFT.into(), border: iced::Border { color: ACCENT, width: 1.0, radius: R_LG.into() } },
            picked_split: pane_grid::Line { color: ACCENT, width: 2.0 },
            hovered_split: pane_grid::Line { color: Color { a: 0.55, ..ACCENT }, width: 2.0 },
        });
        let body = container(grid).padding(5).width(Fill).height(Fill);
        let mut layers: Vec<Element<'_, Message>> = vec![body.into()];
        if let Some((msg, error, _)) = &self.toast {
            let toast = container(
                row![icon(if *error { code::TRIANGLE_ALERT } else { code::CIRCLE_CHECK }, 14).color(if *error { DANGER } else { SUCCESS }), text(msg).size(T_SMALL)]
                    .spacing(S2)
                    .align_y(Alignment::Center),
            )
            .padding([9, 14])
            .style(style::popover);
            layers.push(container(toast).center_x(Fill).align_bottom(Fill).padding(iced::Padding { bottom: 22.0, ..Default::default() }).into());
        }
        if let Some(dialog) = &self.export {
            layers.push(dialog.view(&self.doc.project));
        }
        stack(layers).into()
    }

    fn library_header(&self) -> Element<'_, Message> {
        let tabs = row(LibraryTab::ALL.iter().map(|&t| tab(t.label(), t == self.library_tab, Message::LibraryTab(t)))).spacing(2);
        header_row(tabs.into(), widgets::tip(widgets::icon_button(code::PLUS, Some(Message::Import)), "Import media", Some("Ctrl+I"), tip_pos()))
    }

    fn side_header(&self) -> Element<'_, Message> {
        let tabs = row![
            tab("Inspector", self.side_tab == SideTab::Inspector, Message::SideTab(SideTab::Inspector)),
            tab_icon(code::SPARKLES, "Assistant", self.side_tab == SideTab::Assistant, Message::SideTab(SideTab::Assistant)),
        ]
        .spacing(2);
        header_row(tabs.into(), space().into())
    }

    fn preview(&self) -> Element<'_, Message> {
        let (cw, ch) = self.doc.project.canvas.size();
        let stage = responsive(move |size: Size| {
            let fit = (size.width / cw as f32).min(size.height / ch as f32);
            let (w, h) = ((cw as f32 * fit).floor(), (ch as f32 * fit).floor());
            // Decode a little above the on-screen size so high-DPI screens stay sharp.
            self.frames.set_preview_width(w * 1.5);
            let project = &self.doc.project;
            // While editing a zoom, show that clip unzoomed so the whole frame is visible.
            let editing = self.zoom_edit.and_then(|(clip, zoom)| project.clip(clip).and_then(|c| c.zooms.iter().find(|z| z.id == zoom).map(|z| (clip, *z))));
            let unzoomed;
            let shown = match editing {
                Some((clip, _)) => {
                    let mut p = project.clone();
                    if let Ok(c) = p.clip_mut(clip) {
                        c.zooms.clear();
                    }
                    unzoomed = p;
                    &unzoomed
                }
                None => project,
            };
            let scene = std::sync::Arc::new(scene::evaluate(shown, self.playhead, (cw, ch), &self.frames.bind(project), self.playing.is_some()));
            let overlay = canvas_overlay::Overlay { project, scene: scene.clone(), selection: &self.selection, zoom_edit: editing };
            let canvas = stack![
                iced::widget::shader(preview::Preview { scene }).width(w).height(h),
                iced::widget::canvas(overlay).width(w).height(h),
            ];
            let frame = container(canvas).style(|_| container::Style {
                border: iced::Border { color: LINE, width: 1.0, radius: 2.0.into() },
                shadow: iced::Shadow { color: Color { a: 0.5, ..Color::BLACK }, offset: iced::Vector::new(0.0, 6.0), blur_radius: 24.0 },
                ..Default::default()
            });
            container(frame).center(Fill).into()
        });
        let duration = self.doc.project.duration();
        let transport = row![
            text(fmt_timecode_frames(self.playhead, self.doc.project.canvas.fps)).font(INTER_MEDIUM).size(T_BODY),
            text(format!(" / {}", fmt_timecode(duration))).size(T_BODY).color(TEXT_4),
            space().width(Fill),
            widgets::tip(widgets::icon_button(code::SKIP_BACK, Some(Message::GoStart)), "Go to start", Some("Home"), tip_pos()),
            widgets::tip(play_button(self.playing.is_some()), if self.playing.is_some() { "Pause" } else { "Play" }, Some("Space"), tip_pos()),
            widgets::tip(widgets::icon_button(code::SKIP_FORWARD, Some(Message::GoEnd)), "Go to end", Some("End"), tip_pos()),
            space().width(Fill),
            widgets::tip(widgets::icon_toggle(if self.muted { code::VOLUME_X } else { code::VOLUME_2 }, self.muted, Message::ToggleMute), if self.muted { "Unmute preview" } else { "Mute preview" }, Some("M"), tip_pos()),
            widgets::tip(widgets::icon_button(code::MAXIMIZE_2, None), "Full screen", Some("F"), tip_pos()),
        ]
        .spacing(4)
        .align_y(Alignment::Center);
        column![container(stage).padding([10, 14]).height(Fill), container(transport).padding([6, 12])].into()
    }

    fn inspector(&self) -> Element<'_, Message> {
        match self.side_tab {
            SideTab::Inspector => match self.selection.as_slice() {
                [] => inspector::empty(),
                [id] => match self.doc.project.clip(*id) {
                    Some(c) => inspector::view(&self.doc.project, c, self.zoom_edit.map(|(_, z)| z)),
                    None => inspector::empty(),
                },
                many => inspector::multiple(many.len()),
            },
            SideTab::Assistant => {
                let range: Element<'_, Message> = match self.range {
                    Some((a, b)) => container(
                        row![tinted(code::SQUARE_DASHED, 14, ACCENT_HOVER), text(format!("Selected {} – {}  ({:.1}s)", fmt_ruler(a), fmt_ruler(b), b - a)).size(T_SMALL)]
                            .spacing(S2)
                            .align_y(Alignment::Center),
                    )
                    .padding([8, 10])
                    .width(Fill)
                    .style(style::badge(ACCENT_HOVER))
                    .into(),
                    None => space().into(),
                };
                column![
                    container(range).padding([4, 10]),
                    widgets::empty_state(
                        code::SPARKLES,
                        "Assistant",
                        "Use the Range tool (R) to select part of the timeline, then describe the animation you want. Uses your Claude Code login.",
                        None,
                    )
                ]
                .into()
            }
        }
    }

    fn timeline_header(&self) -> Element<'_, Message> {
        let tools = row![
            widgets::tip(widgets::icon_toggle(code::MOUSE_POINTER_2, self.tool == Tool::Select, Message::Tool(Tool::Select)), "Select", Some("V"), tip_pos()),
            widgets::tip(widgets::icon_toggle(code::SCISSORS, self.tool == Tool::Blade, Message::Tool(Tool::Blade)), "Blade — click a clip to split", Some("B"), tip_pos()),
            widgets::tip(widgets::icon_toggle(code::SQUARE_DASHED, self.tool == Tool::Range, Message::Tool(Tool::Range)), "Range — select a part to animate", Some("R"), tip_pos()),
            container(widgets::vdivider(16.0)).padding([0, 4]),
            widgets::tip(widgets::icon_button(code::SPLIT, Some(Message::SplitAtPlayhead)), "Split at playhead", Some("S"), tip_pos()),
            widgets::tip(widgets::icon_button(code::TRASH_2, (!self.selection.is_empty()).then_some(Message::DeleteSelection)), "Delete", Some("Del"), tip_pos()),
            container(widgets::vdivider(16.0)).padding([0, 4]),
            widgets::tip(widgets::icon_toggle(code::MAGNET, self.snapping, Message::ToggleSnapping), "Snapping", Some("N"), tip_pos()),
        ]
        .spacing(2)
        .align_y(Alignment::Center);
        let zoom = row![
            widgets::tip(widgets::icon_button(code::ZOOM_OUT, Some(Message::ZoomBy(1.0 / 1.4))), "Zoom out", Some("−"), tip_pos()),
            widgets::tip(widgets::icon_button(code::SCAN, Some(Message::ZoomFit)), "Fit timeline", Some("Shift+Z"), tip_pos()),
            widgets::tip(widgets::icon_button(code::ZOOM_IN, Some(Message::ZoomBy(1.4))), "Zoom in", Some("+"), tip_pos()),
        ]
        .spacing(2);
        header_row(row![text("Timeline").font(INTER_SEMIBOLD).size(T_SMALL).color(TEXT_2), container(tools).padding([0, 10])].align_y(Alignment::Center).into(), zoom.into())
    }

    fn timeline(&self) -> Element<'_, Message> {
        responsive(move |size| {
            self.timeline_size.set(size);
            let project = &self.doc.project;
            let content_h = timeline::content_height(project);
            let overflow = content_h > size.height - RULER_H;
            let add = DropDownless::add_track_button(project);
            let ruler = row![
                container(add).width(HEADER_W).height(RULER_H).padding([0, 8]).center_y(RULER_H),
                iced::widget::canvas(Ruler { view: self.view, playhead: self.playhead, range: self.range }).width(Fill).height(RULER_H),
            ];
            let lanes = Lanes {
                project,
                media: &self.media,
                selection: &self.selection,
                playhead: self.playhead,
                view: self.view,
                tool: self.tool,
                snapping: self.snapping,
                range: self.range,
                dnd: self.dnd,
                cache: &self.lanes_cache,
                overflow,
            };
            let body = row![
                container(self.track_headers()).width(HEADER_W),
                stack![
                    iced::widget::canvas(timeline::LanesBase(lanes)).width(Fill).height(content_h),
                    iced::widget::canvas(lanes).width(Fill).height(content_h),
                ],
            ];
            column![ruler, widgets::scroll(body).height(Fill)].into()
        })
        .into()
    }

    fn track_headers(&self) -> Element<'_, Message> {
        let project = &self.doc.project;
        let mut col = column![space().height(timeline::TOP_PAD)].spacing(0);
        for t in &project.tracks {
            let h = timeline::track_height(t.kind);
            let id = t.id;
            let (glyph, color) = match t.kind {
                TrackKind::Visual => (code::LAYERS, clip::VIDEO),
                TrackKind::Audio => (code::AUDIO_LINES, clip::AUDIO),
            };
            let name: Element<'_, Message> = match &self.renaming {
                Some((rid, value)) if *rid == id => text_input("Track name", value)
                    .id("track-rename")
                    .on_input(Message::RenameInput)
                    .on_submit(Message::RenameCommit)
                    .padding([3, 6])
                    .size(T_SMALL)
                    .style(style::input)
                    .into(),
                _ => {
                    let font = if t.name.is_ascii() { INTER_MEDIUM } else { HIND_SEMIBOLD };
                    mouse_area(text(&t.name).font(font).size(T_SMALL).wrapping(text::Wrapping::None)).on_double_click(Message::Track(id, TrackAction::Rename)).into()
                }
            };
            let small = |glyph: char, active: bool, action: TrackAction, tip: &'static str| {
                widgets::tip(
                    button(container(icon(glyph, 12).color(if active { WARNING } else { TEXT_3 })).center(22)).width(22).height(22).padding(0).style(style::ghost).on_press(Message::Track(id, action)),
                    tip,
                    None,
                    tip_pos(),
                )
            };
            let mut toggles = row![].spacing(0);
            if t.kind == TrackKind::Audio {
                toggles = toggles.push(small(if t.muted { code::VOLUME_X } else { code::VOLUME_2 }, t.muted, TrackAction::ToggleMuted, "Mute track"));
            } else {
                toggles = toggles.push(small(if t.hidden { code::EYE_OFF } else { code::EYE }, t.hidden, TrackAction::ToggleHidden, "Hide track"));
            }
            toggles = toggles.push(small(if t.locked { code::LOCK } else { code::LOCK_OPEN }, t.locked, TrackAction::ToggleLocked, "Lock track"));
            let header = container(row![tinted(glyph, 13, color), container(name).width(Fill).clip(true), toggles].spacing(8).align_y(Alignment::Center))
                .padding(iced::Padding { left: 10.0, right: 4.0, ..Default::default() })
                .height(h)
                .center_y(h)
                .style(style::card);
            let menu = ContextMenu::new(header, move || {
                widgets::menu(
                    vec![
                        MenuItem::new(code::PENCIL, "Rename track", Message::Track(id, TrackAction::Rename)),
                        MenuItem::Separator,
                        MenuItem::new(code::LAYERS, "Add video track", Message::Track(id, TrackAction::AddVideo)),
                        MenuItem::new(code::AUDIO_LINES, "Add audio track", Message::Track(id, TrackAction::AddAudio)),
                        MenuItem::Separator,
                        MenuItem::new(code::CHEVRON_UP, "Move up", Message::Track(id, TrackAction::MoveUp)),
                        MenuItem::new(code::CHEVRON_DOWN, "Move down", Message::Track(id, TrackAction::MoveDown)),
                        MenuItem::Separator,
                        MenuItem::new(code::TRASH_2, "Delete track", Message::Track(id, TrackAction::Delete)).danger(),
                    ],
                    210.0,
                )
            })
            .style(|_, _| iced_aw::style::context_menu::Style { background: Color::TRANSPARENT.into() });
            col = col.push(container(menu).padding(iced::Padding { left: 8.0, right: 6.0, bottom: timeline::GAP, top: 0.0 }));
        }
        col.into()
    }
}

/// Synthesizes (if needed) and probes a built-in sound in the background.
fn sound_task(id: &'static str, done: fn(&'static str, Result<(PathBuf, ProbeInfo), String>) -> Message) -> Task<Message> {
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                let path = crate::sfx::path(id).map_err(|e| e.to_string())?;
                let info = media::probe(&path).map_err(|e| e.to_string())?;
                Ok((path, info))
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()))
        },
        move |r| done(id, r),
    )
}

/// Track creation button above the headers.
struct DropDownless;

impl DropDownless {
    fn add_track_button<'a>(project: &Project) -> Element<'a, Message> {
        let first_visual = project.tracks.iter().find(|t| t.kind == TrackKind::Visual).map(|t| t.id).unwrap_or(0);
        let last = project.tracks.last().map(|t| t.id).unwrap_or(0);
        row![
            widgets::tip(
                button(row![icon(code::PLUS, 12), text("Video").size(T_CAPTION)].spacing(4).align_y(Alignment::Center)).padding([3, 7]).style(style::ghost).on_press(Message::Track(first_visual, TrackAction::AddVideo)),
                "Add video track",
                None,
                tip_pos(),
            ),
            widgets::tip(
                button(row![icon(code::PLUS, 12), text("Audio").size(T_CAPTION)].spacing(4).align_y(Alignment::Center)).padding([3, 7]).style(style::ghost).on_press(Message::Track(last, TrackAction::AddAudio)),
                "Add audio track",
                None,
                tip_pos(),
            ),
        ]
        .spacing(2)
        .into()
    }
}

/// Global events the editor cares about: cursor, mouse release, file drops and shortcuts.
fn editor_event(event: Event, status: event::Status, _window: window::Id) -> Option<Message> {
    match event {
        Event::Mouse(mouse::Event::CursorMoved { position }) => Some(Message::CursorMoved(position)),
        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => Some(Message::MouseReleased),
        Event::Window(window::Event::FileDropped(path)) => Some(Message::ImportPicked(vec![path])),
        Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) if status == event::Status::Ignored => {
            let cmd = modifiers.command();
            match key.as_ref() {
                Key::Named(key::Named::Space) => Some(Message::TogglePlay),
                Key::Named(key::Named::Delete | key::Named::Backspace) => Some(Message::DeleteSelection),
                Key::Named(key::Named::Home) => Some(Message::GoStart),
                Key::Named(key::Named::End) => Some(Message::GoEnd),
                Key::Named(key::Named::ArrowLeft) => Some(Message::Step(if modifiers.shift() { -10 } else { -1 })),
                Key::Named(key::Named::ArrowRight) => Some(Message::Step(if modifiers.shift() { 10 } else { 1 })),
                Key::Named(key::Named::Escape) => Some(Message::Escape),
                Key::Character("z") if cmd && modifiers.shift() => Some(Message::Redo),
                Key::Character("z") if cmd => Some(Message::Undo),
                Key::Character("y") if cmd => Some(Message::Redo),
                Key::Character("d") if cmd => Some(Message::DuplicateSelection),
                Key::Character("i") if cmd => Some(Message::Import),
                Key::Character("b") if cmd => Some(Message::SplitAtPlayhead),
                Key::Character("s") if !cmd => Some(Message::SplitAtPlayhead),
                Key::Character("v") if !cmd => Some(Message::Tool(Tool::Select)),
                Key::Character("b") => Some(Message::Tool(Tool::Blade)),
                Key::Character("r") if !cmd => Some(Message::Tool(Tool::Range)),
                Key::Character("n") if !cmd => Some(Message::ToggleSnapping),
                Key::Character("t") if !cmd => Some(Message::AddText),
                Key::Character("m") if !cmd => Some(Message::ToggleMute),
                Key::Character("=" | "+") => Some(Message::ZoomBy(1.4)),
                Key::Character("-") => Some(Message::ZoomBy(1.0 / 1.4)),
                Key::Character("Z") | Key::Character("z") if modifiers.shift() => Some(Message::ZoomFit),
                _ => None,
            }
        }
        _ => None,
    }
}

fn tip_pos() -> iced::widget::tooltip::Position {
    iced::widget::tooltip::Position::Top
}

fn pane_title<'a>(title: &'a str, detail: Option<String>) -> Element<'a, Message> {
    let mut r = row![text(title).font(INTER_SEMIBOLD).size(T_SMALL).color(TEXT_2)].spacing(S2).align_y(Alignment::Center);
    if let Some(d) = detail {
        r = r.push(text(d).size(T_CAPTION).color(TEXT_4));
    }
    container(r).height(38).padding([0, 8]).center_y(38).into()
}

fn header_row<'a>(left: Element<'a, Message>, right: Element<'a, Message>) -> Element<'a, Message> {
    container(row![left, space().width(Fill), right].align_y(Alignment::Center)).height(38).padding([0, 2]).center_y(38).into()
}

fn tab<'a>(label: &'a str, active: bool, on_press: Message) -> Element<'a, Message> {
    button(text(label).font(INTER_MEDIUM).size(T_SMALL)).padding([5, 10]).style(style::segment(active)).on_press(on_press).into()
}

fn tab_icon<'a>(glyph: char, label: &'a str, active: bool, on_press: Message) -> Element<'a, Message> {
    let content = row![icon(glyph, 12), text(label).font(INTER_MEDIUM).size(T_SMALL)].spacing(5).align_y(Alignment::Center);
    button(content).padding([5, 10]).style(style::segment(active)).on_press(on_press).into()
}

fn play_button<'a>(playing: bool) -> Element<'a, Message> {
    button(container(icon(if playing { code::PAUSE } else { code::PLAY }, 15)).center(Length::Fill)).width(38).height(38).padding(0).style(style::primary).on_press(Message::TogglePlay).into()
}

pub fn fmt_timecode(t: f64) -> String {
    let total = t.max(0.0).floor() as u64;
    format!("{:02}:{:02}:{:02}", total / 3600, (total / 60) % 60, total % 60)
}

fn fmt_timecode_frames(t: f64, fps: u32) -> String {
    let frames = (t.max(0.0) * fps as f64).round() as u64;
    let fps = fps.max(1) as u64;
    let secs = frames / fps;
    format!("{:02}:{:02}:{:02}:{:02}", secs / 3600, (secs / 60) % 60, secs % 60, frames % fps)
}

fn fmt_ruler(t: f64) -> String {
    timeline::fmt_ruler(t, true)
}

fn fmt_duration(d: f64) -> String {
    let s = d.round() as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}
