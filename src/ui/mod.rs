//! The desktop app: layout, preview, playback control, shortcuts and dialogs.

mod chat_panel;
mod dialogs;
mod inspector;
mod library;
mod media_cache;
mod recent;
mod theme;
mod timeline;
mod widgets;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use egui::{pos2, vec2, Align2, Color32, ColorImage, CornerRadius, Id as EId, Key, Modifiers, Rect, RichText, Sense, Stroke, TextureHandle, TextureOptions};

use crate::chat::ChatAgent;
use crate::ffmpeg;
use crate::mcp;
use crate::playback::{Preview, StillKey};
use crate::project::{fmt_time, Canvas, Editor, ExportSettings, ExportStatus, Id, Project, Source, TextClip};
use crate::render;
use crate::shared::{Placement, Shared};
use dialogs::*;
use media_cache::MediaCache;
use theme::*;
use timeline::{DropTarget, Timeline};

/// Drag-and-drop payload: a media library item.
#[derive(Clone, Copy)]
pub struct AssetDrag(pub Id);

enum Dialog {
    NewProject { form: CanvasForm, name: String, can_cancel: bool },
    Settings { form: CanvasForm },
    Export,
    Connect(ConnectState),
    Unsaved(Pending),
    MissingFfmpeg,
}

#[derive(Clone, Copy, PartialEq)]
enum Pending {
    Close,
    New,
    Open,
}

#[derive(Clone, Copy, PartialEq)]
enum ToastKind {
    Info,
    Success,
    Error,
    Ai,
}

struct Toast {
    text: String,
    kind: ToastKind,
    born: Instant,
}

pub struct App {
    shared: Shared,
    preview: Preview,
    media: MediaCache,
    timeline: Timeline,
    snapshot: Arc<Project>,
    snap_rev: u64,
    preview_tex: Option<TextureHandle>,
    shown_key: Option<StillKey>,
    still_wait_since: Option<Instant>,
    playing: bool,
    play_rev: u64,
    playhead: f64,
    selected: Option<Id>,
    dialog: Option<Dialog>,
    dialog_open: bool,
    export_form: ExportForm,
    toasts: Vec<Toast>,
    mcp_error: Option<String>,
    allow_close: bool,
    export_was_running: bool,
    ai_touched: HashMap<Id, Instant>,
    title: String,
    log_seen: usize,
    lib_born: HashMap<Id, Instant>,
    text_buffer: Option<(Id, String)>,
    chat: ChatAgent,
    chat_items: Vec<chat_panel::ChatItem>,
    chat_input: String,
    chat_streaming: bool,
    /// 0 = Assistant, 1 = Properties, 2 = History.
    right_tab: usize,
    /// Long side of the preview render, matched to the on-screen size.
    preview_long: u32,
    looping: bool,
    muted: bool,
    /// Toasts posted from background threads.
    bg_toasts: Arc<parking_lot::Mutex<Vec<(ToastKind, String)>>>,
    recent: Vec<PathBuf>,
    /// Debug builds only: a chat message to send once clips are on the timeline (AIVE_AUTOCHAT).
    autochat: Option<String>,
}

fn app_icon() -> egui::IconData {
    let n = 64usize;
    let mut rgba = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            // Rounded square.
            let r = 14.0;
            let dx = (fx - fx.clamp(r, n as f32 - r)).abs();
            let dy = (fy - fy.clamp(r, n as f32 - r)).abs();
            let inside = (dx * dx + dy * dy).sqrt() <= r;
            let i = (y * n + x) * 4;
            if inside {
                let t = (fx + fy) / (2.0 * n as f32);
                rgba[i] = (30.0 + 40.0 * t) as u8;
                rgba[i + 1] = (100.0 + 70.0 * t) as u8;
                rgba[i + 2] = 250;
                rgba[i + 3] = 255;
                // Play triangle.
                let (ax, ay, h) = (24.0, 18.0, 28.0);
                if fx >= ax && fy >= ay && fy <= ay + h && (fx - ax) <= (h / 2.0 - (fy - ay - h / 2.0).abs()) * 1.6 {
                    rgba[i] = 255;
                    rgba[i + 1] = 255;
                    rgba[i + 2] = 255;
                }
            }
        }
    }
    egui::IconData { rgba, width: n as u32, height: n as u32 }
}

pub fn run() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("AI Video Editor")
            .with_inner_size([1560.0, 960.0])
            .with_min_inner_size([1200.0, 720.0])
            .with_icon(Arc::new(app_icon())),
        vsync: true,
        ..Default::default()
    };
    eframe::run_native("AI Video Editor", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

impl App {
    fn new(cc: &eframe::CreationContext) -> App {
        theme::install(&cc.egui_ctx);
        let ctx = cc.egui_ctx.clone();
        let repaint: Arc<dyn Fn() + Send + Sync> = Arc::new(move || ctx.request_repaint());
        let shared = Shared::new(Editor::default(), repaint.clone());
        let mcp_error = mcp::start(shared.clone()).err().map(|e| e.to_string());
        let r2 = repaint.clone();
        let preview = Preview::new(move || r2());
        let snapshot = Arc::new(shared.editor.lock().project.clone());
        let mut app = App {
            preview,
            media: MediaCache::new(repaint.clone()),
            timeline: Timeline::default(),
            snapshot,
            snap_rev: 0,
            preview_tex: None,
            shown_key: None,
            still_wait_since: None,
            playing: false,
            play_rev: 0,
            playhead: 0.0,
            selected: None,
            dialog: None,
            dialog_open: false,
            export_form: ExportForm { canvas: CanvasForm::from_canvas(&Canvas::default()), quality: crate::project::Quality::Standard, path: String::new() },
            toasts: Vec::new(),
            mcp_error,
            allow_close: false,
            export_was_running: false,
            ai_touched: HashMap::new(),
            title: String::new(),
            log_seen: 0,
            lib_born: HashMap::new(),
            text_buffer: None,
            chat: ChatAgent::new(repaint.clone()),
            chat_items: Vec::new(),
            chat_input: String::new(),
            chat_streaming: false,
            right_tab: 0,
            preview_long: 1280,
            looping: false,
            muted: false,
            bg_toasts: Arc::default(),
            recent: recent::load(),
            autochat: if cfg!(debug_assertions) { std::env::var("AIVE_AUTOCHAT").ok() } else { None },
            shared,
        };
        // A project path on the command line (e.g. double-clicking a .aive file) opens directly.
        let arg_project = std::env::args().skip(1).map(PathBuf::from).find(|p| p.extension().is_some_and(|e| e == "aive"));
        if !ffmpeg::is_available() {
            app.open_dialog(Dialog::MissingFfmpeg);
        } else if let Some(path) = arg_project {
            match Project::load(&path) {
                Ok(p) => app.shared.editor.lock().replace_project(p, Some(path)),
                Err(e) => {
                    app.toast(ToastKind::Error, format!("Could not open project: {e:#}"));
                    app.open_dialog(Dialog::NewProject { form: CanvasForm::from_canvas(&Canvas::default()), name: "My video".into(), can_cancel: false });
                }
            }
        } else {
            app.open_dialog(Dialog::NewProject { form: CanvasForm::from_canvas(&Canvas::default()), name: "My video".into(), can_cancel: false });
        }
        if let Some(e) = &app.mcp_error {
            app.toast(ToastKind::Error, format!("AI connection unavailable: {e}"));
        }
        app
    }

    fn toast(&mut self, kind: ToastKind, text: impl Into<String>) {
        self.toasts.push(Toast { text: text.into(), kind, born: Instant::now() });
        if self.toasts.len() > 5 {
            self.toasts.remove(0);
        }
    }

    fn open_dialog(&mut self, d: Dialog) {
        self.dialog = Some(d);
        self.dialog_open = true;
    }

    fn close_dialog(&mut self) {
        self.dialog_open = false;
    }

    fn stop(&mut self) {
        if self.playing {
            self.preview.stop();
            self.playing = false;
        }
    }

    fn seek(&mut self, t: f64) {
        let total = self.snapshot.duration();
        self.playhead = t.clamp(0.0, total.max(0.0));
        self.shared.editor.lock().playhead = self.playhead;
        if self.playing {
            self.start_playback();
        }
    }

    fn start_playback(&mut self) {
        let repaint = self.shared.repaint_fn();
        // Playback streams raw frames, so cap its size a little below the still-frame size.
        let long = self.preview_long.min(1600);
        self.preview.set_muted(self.muted);
        match self.preview.play(&self.snapshot, self.playhead, long, repaint) {
            Ok(()) => {
                self.playing = true;
                self.play_rev = self.snap_rev;
                if self.playhead >= self.snapshot.duration() - self.snapshot.canvas.frame() {
                    self.playhead = 0.0;
                }
            }
            Err(e) => {
                self.playing = false;
                self.toast(ToastKind::Error, e.to_string());
            }
        }
    }

    fn toggle_play(&mut self) {
        if self.playing {
            self.stop();
        } else {
            self.start_playback();
        }
    }

    fn edit<T>(&mut self, desc: &str, f: impl FnOnce(&mut Project) -> anyhow::Result<T>) -> Option<T> {
        // Failures are logged by `apply` and surface as a toast via `sync`.
        self.shared.editor.lock().apply(Source::User, desc, f).ok()
    }

    fn split_at_playhead(&mut self) {
        let t = self.playhead;
        if let Some(id) = self.edit("Split", |p| p.split_at(t)) {
            self.selected = Some(id);
        }
    }

    fn delete_selected(&mut self) {
        if let Some(id) = self.selected.take() {
            self.edit("Delete clip", |p| p.delete_clip(id));
        }
    }

    fn duplicate_selected(&mut self) {
        if let Some(id) = self.selected {
            if let Some(new_id) = self.edit("Duplicate clip", |p| p.duplicate_clip(id)) {
                self.selected = Some(new_id);
            }
        }
    }

    /// Saves the current frame at full project resolution as a PNG.
    fn snapshot_frame(&mut self) {
        let project = self.snapshot.clone();
        if project.main.is_empty() {
            self.toast(ToastKind::Info, "Add a clip first");
            return;
        }
        let t = self.playhead;
        let name = format!("{} {}.png", project.name, fmt_time(t).replace(':', "-"));
        let Some(path) = rfd::FileDialog::new().add_filter("PNG image", &["png"]).set_file_name(name).set_title("Save frame").save_file() else { return };
        let path = path.with_extension("png");
        let toasts = self.bg_toasts.clone();
        let repaint = self.shared.repaint_fn();
        std::thread::spawn(move || {
            let (w, h) = (project.canvas.width, project.canvas.height);
            let result = render::render_frame(&project, t, w, h).and_then(|rgba| {
                image::RgbaImage::from_raw(w, h, rgba).ok_or_else(|| anyhow::anyhow!("bad frame")).and_then(|img| img.save(&path).map_err(Into::into))
            });
            let msg = match result {
                Ok(()) => (ToastKind::Success, format!("Saved frame {}", path.file_name().unwrap_or_default().to_string_lossy())),
                Err(e) => (ToastKind::Error, format!("Could not save frame: {e}")),
            };
            toasts.lock().push(msg);
            repaint();
        });
    }

    fn add_text_at_playhead(&mut self) {
        let start = self.playhead;
        if let Some(id) = self.edit("Add text", |p| p.add_text(TextClip::new("Your text", start, 3.0))) {
            self.selected = Some(id);
        }
    }

    fn undo(&mut self) {
        let r = self.shared.editor.lock().undo(Source::User);
        if r.is_none() {
            self.toast(ToastKind::Info, "Nothing to undo");
        }
    }

    fn redo(&mut self) {
        let r = self.shared.editor.lock().redo(Source::User);
        if r.is_none() {
            self.toast(ToastKind::Info, "Nothing to redo");
        }
    }

    fn import_dialog(&mut self) {
        let files = rfd::FileDialog::new()
            .add_filter("Media", crate::media::MEDIA_EXTENSIONS)
            .add_filter("All files", &["*"])
            .set_title("Import media")
            .pick_files();
        if let Some(files) = files {
            self.import(files, Placement::LibraryOnly);
        }
    }

    fn import(&mut self, files: Vec<PathBuf>, placement: Placement) {
        if files.is_empty() {
            return;
        }
        let shared = self.shared.clone();
        self.shared.import_async(files, placement, move |report| {
            if !report.errors.is_empty() {
                let mut ed = shared.editor.lock();
                for e in report.errors {
                    ed.push_log(Source::User, format!("Import failed — {e}"), false);
                }
            }
            shared.repaint();
        });
    }

    fn save(&mut self, save_as: bool) -> bool {
        let (project, path, revision) = {
            let ed = self.shared.editor.lock();
            (ed.project.clone(), ed.path.clone(), ed.revision)
        };
        let path = match path.filter(|_| !save_as) {
            Some(p) => p,
            None => {
                let Some(p) = rfd::FileDialog::new().add_filter("AI Video Editor project", &["aive"]).set_file_name(format!("{}.aive", project.name)).set_title("Save project").save_file() else {
                    return false;
                };
                p.with_extension("aive")
            }
        };
        match project.save(&path) {
            Ok(()) => {
                let mut ed = self.shared.editor.lock();
                ed.path = Some(path.clone());
                // Edits made while saving (e.g. by the AI) keep the project marked unsaved.
                ed.mark_saved_at(revision);
                drop(ed);
                self.recent = recent::push(&path);
                self.toast(ToastKind::Success, format!("Saved {}", path.file_name().unwrap_or_default().to_string_lossy()));
                true
            }
            Err(e) => {
                self.toast(ToastKind::Error, format!("Save failed: {e:#}"));
                false
            }
        }
    }

    fn open_project(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter("AI Video Editor project", &["aive"]).set_title("Open project").pick_file() else { return };
        self.open_path(path);
    }

    fn open_path(&mut self, path: PathBuf) {
        match Project::load(&path) {
            Ok(p) => {
                let missing: Vec<String> = p.missing_media().iter().map(|a| a.name.clone()).collect();
                self.stop();
                self.recent = recent::push(&path);
                self.shared.editor.lock().replace_project(p, Some(path));
                self.after_project_switch();
                self.close_dialog();
                if !missing.is_empty() {
                    self.toast(ToastKind::Error, format!("Missing media: {}", missing.join(", ")));
                }
            }
            Err(e) => self.toast(ToastKind::Error, format!("Could not open project: {e:#}")),
        }
    }

    fn after_project_switch(&mut self) {
        self.selected = None;
        self.playhead = 0.0;
        self.shown_key = None;
        self.timeline = Timeline::default();
    }

    /// Runs `p` now, or first asks to save unsaved changes.
    fn guarded(&mut self, p: Pending) {
        let dirty = {
            let ed = self.shared.editor.lock();
            let p = &ed.project;
            ed.is_dirty() && (!p.assets.is_empty() || !p.texts.is_empty() || ed.path.is_some())
        };
        if dirty {
            self.open_dialog(Dialog::Unsaved(p));
        } else {
            self.run_pending(p);
        }
    }

    fn run_pending(&mut self, p: Pending) {
        match p {
            Pending::Close => {
                self.allow_close = true;
            }
            Pending::New => {
                let canvas = self.snapshot.canvas;
                self.open_dialog(Dialog::NewProject { form: CanvasForm::from_canvas(&canvas), name: "My video".into(), can_cancel: true });
            }
            Pending::Open => self.open_project(),
        }
    }

    fn open_export(&mut self) {
        let (canvas, name, running) = {
            let ed = self.shared.editor.lock();
            (ed.project.canvas, ed.project.name.clone(), matches!(ed.export_status, ExportStatus::Running { .. }))
        };
        if !running {
            self.export_form.canvas = CanvasForm::from_canvas(&canvas);
            if self.export_form.path.is_empty() || PathBuf::from(&self.export_form.path).exists() {
                self.export_form.path = default_export_path(&name);
            }
            let mut ed = self.shared.editor.lock();
            if matches!(ed.export_status, ExportStatus::Cancelled | ExportStatus::Done { .. } | ExportStatus::Failed { .. }) {
                ed.export_status = ExportStatus::Idle;
            }
        }
        self.open_dialog(Dialog::Export);
    }

    // -----------------------------------------------------------------------
    // Per-frame sync with shared state
    // -----------------------------------------------------------------------

    fn sync(&mut self, ctx: &egui::Context) {
        self.media.poll(ctx);
        self.poll_chat(ctx);
        let posted: Vec<_> = std::mem::take(&mut *self.bg_toasts.lock());
        for (kind, text) in posted {
            self.toast(kind, text);
        }
        let mut new_logs = Vec::new();
        let play_request;
        {
            let mut ed = self.shared.editor.lock();
            if ed.revision != self.snap_rev {
                self.snapshot = Arc::new(ed.project.clone());
                self.snap_rev = ed.revision;
                self.media.ensure(&self.snapshot.assets);
                if let Some(id) = self.selected {
                    if self.snapshot.track_of(id).is_none() {
                        self.selected = None;
                    }
                }
            }
            if let Some(t) = ed.seek_request.take() {
                self.playhead = t;
                if self.playing {
                    self.preview.stop();
                    self.playing = false;
                }
            }
            play_request = ed.play_request.take();
            self.ai_touched = ed.ai_touched.clone();
            ed.ai_touched.retain(|_, at| at.elapsed().as_secs_f32() < 3.0);
            // The user's failed edits and imports become toasts. AI actions already show
            // in the activity feed and the AI handles its own errors, so they stay quiet.
            if ed.log.len() < self.log_seen {
                self.log_seen = 0;
            }
            for entry in &ed.log[self.log_seen..] {
                if entry.source == Source::User && !entry.ok {
                    new_logs.push((entry.source, entry.ok, entry.text.clone()));
                }
            }
            self.log_seen = ed.log.len();

            let running = matches!(ed.export_status, ExportStatus::Running { .. });
            if self.export_was_running && !running {
                let dialog_shows_it = matches!(self.dialog, Some(Dialog::Export)) && self.dialog_open;
                if !dialog_shows_it {
                    match &ed.export_status {
                        ExportStatus::Done { path, .. } => new_logs.push((Source::User, true, format!("Export finished: {}", path.display()))),
                        ExportStatus::Failed { error } => new_logs.push((Source::User, false, error.clone())),
                        _ => {}
                    }
                }
            }
            self.export_was_running = running;
            let total = ed.project.duration();
            if self.playhead > total {
                self.playhead = total;
            }
            ed.playhead = self.playhead;
            ed.playing = self.playing;
        }
        for (source, ok, text) in new_logs {
            let kind = match (source, ok) {
                (_, false) => ToastKind::Error,
                (Source::Ai, true) => ToastKind::Ai,
                (Source::User, true) => ToastKind::Success,
            };
            self.toast(kind, text);
        }
        match play_request {
            Some(true) if !self.playing => self.start_playback(),
            Some(false) => self.stop(),
            _ => {}
        }
    }

    fn update_preview(&mut self, ctx: &egui::Context) {
        let size = render::preview_size(&self.snapshot.canvas, self.preview_long);
        if self.playing {
            if self.snap_rev != self.play_rev && !self.timeline.is_dragging() {
                // The project changed under playback (e.g. the AI edited): restart seamlessly.
                self.start_playback();
            }
            if let Some(tick) = self.preview.tick() {
                if let Some((rgba, sz)) = tick.frame {
                    self.upload(ctx, &rgba, sz);
                    self.shown_key = None;
                }
                self.playhead = tick.time;
                if tick.ended {
                    self.playing = false;
                    if let Some(e) = self.preview.last_error.take() {
                        self.toast(ToastKind::Error, e);
                    } else if self.looping {
                        self.playhead = 0.0;
                        self.start_playback();
                    }
                }
            }
            ctx.request_repaint();
            return;
        }
        let snapshot = self.snapshot.clone();
        if let Some((key, rgba)) = self.preview.poll_stills() {
            // Show the newest frame of this revision even while scrubbing ahead of it.
            if key.revision == self.snap_rev && key.size == size {
                self.upload(ctx, &rgba, size);
                self.shown_key = Some(key);
            }
        }
        let want_frame = (self.playhead * snapshot.canvas.fps as f64).floor() as i64;
        let want = StillKey { revision: self.snap_rev, frame: want_frame, size };
        if self.shown_key != Some(want) {
            if let Some(rgba) = self.preview.request_still(&snapshot, self.snap_rev, self.playhead, size) {
                self.upload(ctx, &rgba, size);
                self.shown_key = Some(want);
                self.still_wait_since = None;
            } else if self.still_wait_since.is_none() {
                self.still_wait_since = Some(Instant::now());
            }
        } else {
            self.still_wait_since = None;
        }
    }

    fn upload(&mut self, ctx: &egui::Context, rgba: &[u8], size: (u32, u32)) {
        if rgba.len() != (size.0 * size.1 * 4) as usize {
            return;
        }
        // Frames are opaque, so premultiplied and straight alpha are the same: skip the conversion.
        let pixels = rgba.chunks_exact(4).map(|p| Color32::from_rgba_premultiplied(p[0], p[1], p[2], 255)).collect();
        let img = ColorImage { size: [size.0 as usize, size.1 as usize], pixels };
        match &mut self.preview_tex {
            Some(t) => t.set(img, TextureOptions::LINEAR),
            None => self.preview_tex = Some(ctx.load_texture("preview", img, TextureOptions::LINEAR)),
        }
    }

    // -----------------------------------------------------------------------
    // Keyboard
    // -----------------------------------------------------------------------

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.dialog.is_some() && self.dialog_open {
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                match &self.dialog {
                    Some(Dialog::NewProject { can_cancel: false, .. }) | Some(Dialog::MissingFfmpeg) => {}
                    _ => self.close_dialog(),
                }
            }
            return;
        }
        let typing = ctx.wants_keyboard_input();
        let cmd = Modifiers::COMMAND;
        let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        let k = |m: Modifiers, key: Key| ctx.input_mut(|i| i.consume_key(m, key));
        if k(cmd_shift, Key::Z) || k(cmd, Key::Y) {
            self.redo();
        } else if k(cmd, Key::Z) {
            self.undo();
        }
        if k(cmd_shift, Key::S) {
            self.save(true);
        } else if k(cmd, Key::S) {
            self.save(false);
        }
        if k(cmd, Key::O) {
            self.guarded(Pending::Open);
        }
        if k(cmd, Key::N) {
            self.guarded(Pending::New);
        }
        if k(cmd, Key::I) {
            self.import_dialog();
        }
        if k(cmd, Key::E) {
            self.open_export();
        }
        if typing {
            return;
        }
        let fr = self.snapshot.canvas.frame();
        if k(Modifiers::NONE, Key::Space) {
            self.toggle_play();
        }
        if k(Modifiers::SHIFT, Key::ArrowLeft) {
            self.seek(self.playhead - 1.0);
        } else if k(Modifiers::NONE, Key::ArrowLeft) {
            self.seek(self.playhead - fr);
        }
        if k(Modifiers::SHIFT, Key::ArrowRight) {
            self.seek(self.playhead + 1.0);
        } else if k(Modifiers::NONE, Key::ArrowRight) {
            self.seek(self.playhead + fr);
        }
        if k(Modifiers::NONE, Key::Home) {
            self.seek(0.0);
        }
        if k(Modifiers::NONE, Key::End) {
            self.seek(self.snapshot.duration());
        }
        if k(Modifiers::NONE, Key::S) || k(cmd, Key::B) {
            self.split_at_playhead();
        }
        if k(Modifiers::NONE, Key::Delete) || k(Modifiers::NONE, Key::Backspace) {
            self.delete_selected();
        }
        if k(Modifiers::NONE, Key::T) {
            self.add_text_at_playhead();
        }
        if k(cmd, Key::D) {
            self.duplicate_selected();
        }
        if k(Modifiers::NONE, Key::M) {
            self.muted = !self.muted;
            self.preview.set_muted(self.muted);
        }
        if k(Modifiers::NONE, Key::L) {
            self.looping = !self.looping;
        }
        if k(Modifiers::NONE, Key::Plus) || k(Modifiers::NONE, Key::Equals) {
            self.timeline.zoom_by(1.4);
        }
        if k(Modifiers::NONE, Key::Minus) {
            self.timeline.zoom_by(1.0 / 1.4);
        }
    }

    // -----------------------------------------------------------------------
    // Panels
    // -----------------------------------------------------------------------

    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top")
            .exact_height(56.0)
            .show_separator_line(false)
            .frame(egui::Frame::NONE.fill(BG).inner_margin(egui::Margin::symmetric(16, 0)))
            .show(ctx, |ui| {
                let bar = ui.max_rect();
                let (name, dirty, export) = {
                    let ed = self.shared.editor.lock();
                    (ed.project.name.clone(), ed.is_dirty(), ed.export_status.clone())
                };
                ui.horizontal_centered(|ui| {
                    // Logo: blue rounded tile with a play mark.
                    let (r, _) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::hover());
                    let p = ui.painter();
                    p.rect_filled(r, CornerRadius::same(9), ACCENT);
                    p.rect_filled(Rect::from_min_max(r.min, pos2(r.right(), r.center().y)), CornerRadius { nw: 9, ne: 9, sw: 0, se: 0 }, alpha(Color32::WHITE, 0.08));
                    let c = r.center() + vec2(1.0, 0.0);
                    p.add(egui::Shape::convex_polygon(vec![c + vec2(-4.5, -7.0), c + vec2(7.5, 0.0), c + vec2(-4.5, 7.0)], Color32::WHITE, Stroke::NONE));
                    ui.add_space(6.0);
                    ui.vertical(|ui| {
                        ui.add_space(9.0);
                        ui.spacing_mut().item_spacing.y = -2.0;
                        ui.label(RichText::new("AI Video Editor").font(theme::bold_font(14.5)).color(Color32::WHITE));
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            ui.label(RichText::new(&name).color(TEXT_FAINT).size(11.5));
                            let d = theme::ease_bool(ctx, EId::new("dirty"), dirty, 0.2);
                            if d > 0.01 {
                                ui.label(RichText::new("• unsaved").color(alpha(WARNING, d)).size(11.0));
                            }
                        });
                    });
                    ui.add_space(14.0);
                    ui.menu_button(RichText::new(format!("{}  File", icons::LIST)).color(TEXT_DIM), |ui| {
                        ui.set_min_width(230.0);
                        let item = |ui: &mut egui::Ui, icon: &str, label: &str, key: &str| {
                            ui.add(egui::Button::new(RichText::new(format!("{icon}   {label}"))).shortcut_text(key).min_size(vec2(220.0, 28.0)).frame(false)).clicked()
                        };
                        if item(ui, icons::FILE_PLUS, "New project…", "Ctrl+N") {
                            ui.close_menu();
                            self.guarded(Pending::New);
                        }
                        if item(ui, icons::FOLDER_OPEN, "Open project…", "Ctrl+O") {
                            ui.close_menu();
                            self.guarded(Pending::Open);
                        }
                        if item(ui, icons::FLOPPY_DISK, "Save", "Ctrl+S") {
                            ui.close_menu();
                            self.save(false);
                        }
                        if item(ui, icons::FLOPPY_DISK, "Save as…", "Ctrl+Shift+S") {
                            ui.close_menu();
                            self.save(true);
                        }
                        ui.separator();
                        if item(ui, icons::DOWNLOAD_SIMPLE, "Import media…", "Ctrl+I") {
                            ui.close_menu();
                            self.import_dialog();
                        }
                        if item(ui, icons::EXPORT, "Export video…", "Ctrl+E") {
                            ui.close_menu();
                            self.open_export();
                        }
                        if item(ui, icons::CAMERA, "Save current frame…", "") {
                            ui.close_menu();
                            self.snapshot_frame();
                        }
                        ui.separator();
                        if item(ui, icons::FRAME_CORNERS, "Project format…", "") {
                            ui.close_menu();
                            let c = self.snapshot.canvas;
                            self.open_dialog(Dialog::Settings { form: CanvasForm::from_canvas(&c) });
                        }
                        if item(ui, icons::PLUG, "Connect other AI apps…", "") {
                            ui.close_menu();
                            self.open_dialog(Dialog::Connect(ConnectState::new()));
                        }
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Export button doubles as a progress indicator.
                        let label = match &export {
                            ExportStatus::Running { progress, .. } => format!("Exporting {:.0}%", progress * 100.0),
                            _ => "Export".to_string(),
                        };
                        let resp = widgets::icon_text_button(ui, icons::EXPORT, &label, true, true);
                        if let ExportStatus::Running { progress, .. } = export {
                            let r = resp.rect;
                            let bar = Rect::from_min_size(pos2(r.left() + 8.0, r.bottom() - 4.0), vec2((r.width() - 16.0) * progress, 2.0));
                            ui.painter().rect_filled(bar, CornerRadius::same(1), Color32::WHITE);
                        }
                        if resp.on_hover_text("Export video (Ctrl+E)").clicked() {
                            self.open_export();
                        }
                        ui.add_space(10.0);
                        self.ai_pill(ui);
                    });
                });
                // Format chip centred in the bar.
                let c = self.snapshot.canvas;
                let chip_rect = Rect::from_center_size(bar.center(), vec2(220.0, 36.0));
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(chip_rect).layout(egui::Layout::centered_and_justified(egui::Direction::LeftToRight)));
                let label = format!("{}  {}", icons::FRAME_CORNERS, c.summary());
                if widgets::chip(&mut child, &label, None, false).on_hover_text("Change aspect ratio, resolution or frame rate").clicked() {
                    self.open_dialog(Dialog::Settings { form: CanvasForm::from_canvas(&c) });
                }
            });
    }

    fn ai_pill(&mut self, ui: &mut egui::Ui) {
        let last = self.shared.editor.lock().last_ai_call;
        let active = self.chat.busy || last.is_some_and(|t| t.elapsed().as_secs_f32() < 4.0);
        let (color, text) = if self.mcp_error.is_some() {
            (DANGER, "AI offline")
        } else if active {
            (AI_GLOW, "Claude is working…")
        } else {
            (SUCCESS, "AI ready")
        };
        if active {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        }
        let resp = ui
            .scope(|ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(text).color(color).size(12.0));
                    widgets::status_dot(ui, color, active);
                });
            })
            .response
            .interact(Sense::click())
            .on_hover_text("Open the assistant");
        if resp.clicked() {
            self.right_tab = 0;
        }
    }

    fn preview_panel(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().frame(theme::floating_panel(8, 8, 0, 8)).show(ctx, |ui| {
            let full = ui.available_rect_before_wrap();
            let transport_h = 54.0;
            let area = Rect::from_min_max(full.min, pos2(full.right(), full.bottom() - transport_h));
            let canvas = self.snapshot.canvas;
            let aspect = ctx.animate_value_with_time(EId::new("preview-aspect"), canvas.width as f32 / canvas.height as f32, 0.35);
            let avail = area.shrink(4.0);
            let (w, h) = if avail.width() / avail.height() > aspect { (avail.height() * aspect, avail.height()) } else { (avail.width(), avail.width() / aspect) };
            let frame = Rect::from_center_size(avail.center(), vec2(w, h));

            // Render the preview at the size it is shown (in physical pixels), so it stays sharp.
            if !self.playing && !self.timeline.is_dragging() {
                let want = frame.width().max(frame.height()) * ctx.pixels_per_point();
                let long = canvas.width.max(canvas.height) as f32;
                let q = (((want / 160.0).ceil() * 160.0).clamp(480.0, 1920.0)).min(long) as u32;
                if q != self.preview_long {
                    self.preview_long = q;
                }
            }

            let p = ui.painter().clone();
            p.rect_filled(area, CornerRadius::same(R_MD), BG);
            theme::shadow(&p, frame, 4.0, 0.8);
            p.rect_filled(frame, CornerRadius::same(4), Color32::BLACK);
            let empty = self.snapshot.main.is_empty();
            if empty {
                let importing = !self.shared.importing.lock().is_empty();
                let r = frame.shrink(18.0);
                let hover = ctx.input(|i| !i.raw.hovered_files.is_empty());
                timeline::dashed_rect(&p, r, alpha(if hover { ACCENT_HI } else { TEXT_FAINT }, 0.8));
                let c = r.center();
                if importing {
                    theme::spinner(&p, c - vec2(0.0, 24.0), 14.0, ACCENT_HI, ui.input(|i| i.time));
                    ctx.request_repaint();
                    p.text(c + vec2(0.0, 8.0), Align2::CENTER_CENTER, "Importing…", theme::bold_font(16.0), TEXT);
                } else {
                    p.text(c - vec2(0.0, 52.0), Align2::CENTER_CENTER, icons::FILM_STRIP, theme::font(34.0), alpha(ACCENT_HI, 0.8));
                    p.text(c - vec2(0.0, 14.0), Align2::CENTER_CENTER, "Drop videos, music or images here", theme::bold_font(17.0), TEXT);
                    p.text(c + vec2(0.0, 10.0), Align2::CENTER_CENTER, "or ask Claude to import them for you", theme::font(13.0), TEXT_DIM);
                    let btn = Rect::from_center_size(c + vec2(0.0, 50.0), vec2(160.0, 34.0));
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(btn));
                    if widgets::icon_text_button(&mut child, icons::DOWNLOAD_SIMPLE, "Import media", true, true).clicked() {
                        self.import_dialog();
                    }
                }
            } else if let Some(tex) = &self.preview_tex {
                p.image(tex.id(), frame, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            // Loading spinner if a still frame takes a while.
            if !self.playing && !empty && self.still_wait_since.is_some_and(|t| t.elapsed().as_secs_f32() > 0.25) {
                let c = frame.right_top() + vec2(-20.0, 20.0);
                p.circle_filled(c, 14.0, Color32::from_black_alpha(140));
                theme::spinner(&p, c, 8.0, Color32::WHITE, ui.input(|i| i.time));
                ctx.request_repaint();
            }
            if let Some(err) = &self.preview.last_error {
                if !empty {
                    p.text(frame.center_bottom() - vec2(0.0, 16.0), Align2::CENTER_BOTTOM, format!("Preview error: {}", err.lines().last().unwrap_or("")), theme::font(12.0), DANGER);
                }
            }
            // AI badge while Claude works.
            let ai_active = self.chat.busy || self.shared.editor.lock().last_ai_call.is_some_and(|t| t.elapsed().as_secs_f32() < 4.0);
            let a = theme::ease_bool(ctx, EId::new("ai-badge"), ai_active, 0.25);
            if a > 0.01 {
                let r = Rect::from_min_size(frame.left_top() + vec2(12.0, 12.0 - (1.0 - a) * 8.0), vec2(168.0, 30.0));
                p.rect_filled(r, CornerRadius::same(15), alpha(Color32::from_rgb(10, 30, 66), 0.92 * a));
                p.rect_stroke(r, CornerRadius::same(15), Stroke::new(1.0_f32, alpha(AI_GLOW, 0.7 * a)), egui::StrokeKind::Inside);
                p.text(r.center(), Align2::CENTER_CENTER, format!("{}  Claude is editing", icons::SPARKLE), theme::bold_font(12.5), alpha(Color32::WHITE, a));
                ctx.request_repaint();
            }

            let bar = Rect::from_min_max(pos2(full.left(), full.bottom() - transport_h + 6.0), full.max);
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(bar).layout(egui::Layout::left_to_right(egui::Align::Center)));
            self.transport(&mut child);
        });
    }

    fn transport(&mut self, ui: &mut egui::Ui) {
        let total = self.snapshot.duration();
        let bar = ui.max_rect();
        let fps = self.snapshot.canvas.fps.max(1) as f64;
        let frame_no = ((self.playhead * fps).round() as u64) % fps as u64;
        ui.painter().text(pos2(bar.left() + 4.0, bar.center().y), Align2::LEFT_CENTER, fmt_time(self.playhead), theme::bold_font(17.0), Color32::WHITE);
        ui.painter().text(pos2(bar.left() + 86.0, bar.center().y + 1.0), Align2::LEFT_CENTER, format!("/ {}   ·   frame {frame_no}", fmt_time(total)), theme::font(12.5), TEXT_FAINT);

        let fr = self.snapshot.canvas.frame();
        let has = total > 0.0;
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_center_size(bar.center(), vec2(236.0, 48.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        if widgets::icon_button(&mut child, icons::SKIP_BACK, "Go to start (Home)", has).clicked() {
            self.seek(0.0);
        }
        if widgets::icon_button(&mut child, icons::CARET_LEFT, "Previous frame (←)", has).clicked() {
            self.seek(self.playhead - fr);
        }
        child.add_space(6.0);
        // Morphing play/pause button.
        let (r, resp) = child.allocate_exact_size(vec2(46.0, 46.0), Sense::click());
        let h = child.ctx().animate_bool_with_time(resp.id.with("h"), resp.hovered(), 0.12);
        let t = theme::ease_bool(child.ctx(), EId::new("playpause"), self.playing, 0.18);
        let p = child.painter();
        p.circle_filled(r.center(), 22.0 + h * 1.5, alpha(ACCENT, 0.25 * h));
        p.circle_filled(r.center(), 20.0 + h, theme::lerp_color(ACCENT, ACCENT_HI, h));
        let c = r.center();
        let (s, bw) = (8.0, 4.5);
        let lerp = |a: egui::Pos2, b: egui::Pos2| a + (b - a) * t;
        let left_play = [c + vec2(-s * 0.6, -s), c + vec2(s * 0.25, -s * 0.47), c + vec2(s * 0.25, s * 0.47), c + vec2(-s * 0.6, s)];
        let left_pause = [c + vec2(-s * 0.75, -s), c + vec2(-s * 0.75 + bw, -s), c + vec2(-s * 0.75 + bw, s), c + vec2(-s * 0.75, s)];
        let right_play = [c + vec2(s * 0.25, -s * 0.47), c + vec2(s * 1.15, 0.0), c + vec2(s * 1.15, 0.0), c + vec2(s * 0.25, s * 0.47)];
        let right_pause = [c + vec2(s * 0.75 - bw, -s), c + vec2(s * 0.75, -s), c + vec2(s * 0.75, s), c + vec2(s * 0.75 - bw, s)];
        for (a, b) in [(left_play, left_pause), (right_play, right_pause)] {
            let pts: Vec<_> = (0..4).map(|i| lerp(a[i], b[i])).collect();
            p.add(egui::Shape::convex_polygon(pts, Color32::WHITE, Stroke::NONE));
        }
        if resp.on_hover_text("Play / pause (Space)").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            self.toggle_play();
        }
        child.add_space(6.0);
        if widgets::icon_button(&mut child, icons::CARET_RIGHT, "Next frame (→)", has).clicked() {
            self.seek(self.playhead + fr);
        }
        if widgets::icon_button(&mut child, icons::SKIP_FORWARD, "Go to end (End)", has).clicked() {
            self.seek(total);
        }

        let mut right = ui.new_child(egui::UiBuilder::new().max_rect(bar).layout(egui::Layout::right_to_left(egui::Align::Center)));
        if widgets::toggle_icon(&mut right, icons::CAMERA, "Save this frame as an image", false).clicked() {
            self.snapshot_frame();
        }
        let mute_icon = if self.muted { icons::SPEAKER_SLASH } else { icons::SPEAKER_HIGH };
        if widgets::toggle_icon(&mut right, mute_icon, if self.muted { "Unmute preview (M)" } else { "Mute preview (M)" }, self.muted).clicked() {
            self.muted = !self.muted;
            self.preview.set_muted(self.muted);
        }
        if widgets::toggle_icon(&mut right, icons::REPEAT, "Loop playback (L)", self.looping).clicked() {
            self.looping = !self.looping;
        }
    }

    fn timeline_panel(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("timeline")
            .resizable(false)
            .exact_height(300.0)
            .show_separator_line(false)
            .frame(theme::floating_panel(8, 8, 8, 8).inner_margin(egui::Margin { left: 12, right: 12, top: 10, bottom: 10 }))
            .show(ctx, |ui| {
                // Toolbar.
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let has_clip = !self.snapshot.main.is_empty();
                    let has_sel = self.selected.is_some();
                    let (can_undo, can_redo) = {
                        let ed = self.shared.editor.lock();
                        (ed.can_undo(), ed.can_redo())
                    };
                    if widgets::icon_button(ui, icons::ARROW_COUNTER_CLOCKWISE, "Undo (Ctrl+Z)", can_undo).clicked() {
                        self.undo();
                    }
                    if widgets::icon_button(ui, icons::ARROW_CLOCKWISE, "Redo (Ctrl+Y)", can_redo).clicked() {
                        self.redo();
                    }
                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(6.0);
                    if widgets::icon_button(ui, icons::SCISSORS, "Split at playhead (S)", has_clip).clicked() {
                        self.split_at_playhead();
                    }
                    if widgets::icon_button(ui, icons::COPY, "Duplicate (Ctrl+D)", has_sel).clicked() {
                        self.duplicate_selected();
                    }
                    if widgets::icon_button(ui, icons::TRASH, "Delete (Del)", has_sel).clicked() {
                        self.delete_selected();
                    }
                    if widgets::icon_button(ui, icons::TEXT_T, "Add text at playhead (T)", true).clicked() {
                        self.add_text_at_playhead();
                    }
                    if widgets::icon_button(ui, icons::SLIDERS, "Clip properties", has_sel).clicked() {
                        self.right_tab = 1;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let width = ui.max_rect().width();
                        if widgets::icon_button(ui, icons::ARROWS_IN, "Fit the whole video", has_clip).clicked() {
                            self.timeline.zoom_to_fit(self.snapshot.duration(), width - 110.0);
                        }
                        if widgets::icon_button(ui, icons::PLUS, "Zoom in (+)", true).clicked() {
                            self.timeline.zoom_by(1.4);
                        }
                        if widgets::icon_button(ui, icons::MINUS, "Zoom out (−)", true).clicked() {
                            self.timeline.zoom_by(1.0 / 1.4);
                        }
                        ui.label(RichText::new("Ctrl + scroll to zoom").color(TEXT_FAINT).size(11.5));
                    });
                });
                ui.add_space(6.0);
                let out = self.timeline.show(ui, &self.shared, &self.snapshot, self.playhead, self.playing, &mut self.selected, &self.media, &self.ai_touched);
                if let Some(t) = out.seek {
                    if out.scrubbing && self.playing {
                        self.stop();
                    }
                    self.seek(t);
                }
                if let Some((asset, target)) = out.dropped_asset {
                    self.add_asset_to_timeline(asset, Some(target));
                }
                if out.open_properties {
                    self.right_tab = 1;
                }
            });
    }

    fn add_asset_to_timeline(&mut self, asset_id: Id, target: Option<DropTarget>) {
        let Some(asset) = self.snapshot.asset(asset_id).cloned() else { return };
        let playhead = self.playhead;
        let result = if asset.kind == crate::media::MediaKind::Audio {
            let start = match target {
                Some(DropTarget::Music(t)) => t,
                _ => playhead,
            };
            self.edit(&format!("Add {} to music", asset.name), |p| p.add_music(asset_id, start, None))
        } else {
            let index = match target {
                Some(DropTarget::Main(i)) => Some(i),
                _ => None,
            };
            self.edit(&format!("Add {}", asset.name), |p| p.insert_main(asset_id, index, None))
        };
        if let Some(id) = result {
            self.selected = Some(id);
        }
    }

    // -----------------------------------------------------------------------
    // Overlays
    // -----------------------------------------------------------------------

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.dialog.take() else { return };
        let open = self.dialog_open;
        let mut keep = true;
        let key = match &dialog {
            Dialog::NewProject { .. } => "new",
            Dialog::Settings { .. } => "settings",
            Dialog::Export => "export",
            Dialog::Connect(_) => "connect",
            Dialog::Unsaved(_) => "unsaved",
            Dialog::MissingFfmpeg => "ffmpeg",
        };
        let width = match &dialog {
            Dialog::NewProject { .. } | Dialog::Settings { .. } | Dialog::Export => 590.0,
            Dialog::Unsaved(_) => 420.0,
            _ => 520.0,
        };
        let mut deferred: Option<Box<dyn FnOnce(&mut App)>> = None;
        let export_status = self.shared.editor.lock().export_status.clone();
        let duration = self.snapshot.duration();
        let mcp_error = self.mcp_error.clone();
        let export_form = &mut self.export_form;
        let recent = self.recent.clone();
        let visible = modal(ctx, key, open, width, |ui| match &mut dialog {
            Dialog::NewProject { form, name, can_cancel } => match canvas_dialog(ui, form, name, true, *can_cancel, &recent) {
                CanvasDialogResult::Create(name, canvas) => {
                    deferred = Some(Box::new(move |app: &mut App| {
                        app.stop();
                        app.shared.editor.lock().replace_project(Project::new(&name, canvas), None);
                        app.after_project_switch();
                        app.close_dialog();
                    }));
                }
                CanvasDialogResult::Open => deferred = Some(Box::new(|app: &mut App| app.open_project())),
                CanvasDialogResult::OpenRecent(path) => deferred = Some(Box::new(move |app: &mut App| app.open_path(path))),
                CanvasDialogResult::Cancel => deferred = Some(Box::new(|app: &mut App| app.close_dialog())),
                _ => {}
            },
            Dialog::Settings { form } => {
                let mut dummy = String::new();
                match canvas_dialog(ui, form, &mut dummy, false, true, &[]) {
                    CanvasDialogResult::Apply(c) => {
                        deferred = Some(Box::new(move |app: &mut App| {
                            if c != app.snapshot.canvas {
                                app.edit(&format!("Canvas {}", c.summary()), |p| p.set_canvas(c));
                            }
                            app.close_dialog();
                        }));
                    }
                    CanvasDialogResult::Cancel => deferred = Some(Box::new(|app: &mut App| app.close_dialog())),
                    _ => {}
                }
            }
            Dialog::Export => match export_dialog(ui, export_form, &export_status, duration) {
                ExportAction::Start(canvas, quality, path) => {
                    deferred = Some(Box::new(move |app: &mut App| {
                        app.stop();
                        if let Err(e) = app.shared.start_export(ExportSettings { canvas, quality, path }, Source::User) {
                            app.toast(ToastKind::Error, e.to_string());
                        }
                    }));
                }
                ExportAction::Cancel => deferred = Some(Box::new(|app: &mut App| app.shared.cancel_export())),
                ExportAction::Close => deferred = Some(Box::new(|app: &mut App| app.close_dialog())),
                ExportAction::Reset => {
                    deferred = Some(Box::new(|app: &mut App| {
                        let done = matches!(app.shared.editor.lock().export_status, ExportStatus::Done { .. });
                        app.shared.editor.lock().export_status = ExportStatus::Idle;
                        if done {
                            app.close_dialog();
                        }
                    }))
                }
                ExportAction::ShowInFolder(p) => {
                    let _ = std::process::Command::new("explorer").arg(format!("/select,{}", p.display())).spawn();
                }
                ExportAction::Play(p) => {
                    let _ = std::process::Command::new("explorer").arg(&p).spawn();
                }
                ExportAction::None => {}
            },
            Dialog::Connect(state) => {
                if connect_dialog(ui, state, mcp_error.as_deref()) {
                    deferred = Some(Box::new(|app: &mut App| app.close_dialog()));
                }
            }
            Dialog::Unsaved(pending) => {
                let what = match pending {
                    Pending::Close => "close",
                    Pending::New => "start a new project",
                    Pending::Open => "open another project",
                };
                let pending = *pending;
                match unsaved_dialog(ui, what) {
                    UnsavedChoice::Save => {
                        deferred = Some(Box::new(move |app: &mut App| {
                            app.close_dialog();
                            if app.save(false) {
                                app.run_pending(pending);
                            }
                        }))
                    }
                    UnsavedChoice::Discard => {
                        deferred = Some(Box::new(move |app: &mut App| {
                            app.close_dialog();
                            app.shared.editor.lock().mark_saved();
                            app.run_pending(pending);
                        }))
                    }
                    UnsavedChoice::Cancel => deferred = Some(Box::new(|app: &mut App| app.close_dialog())),
                    UnsavedChoice::None => {}
                }
            }
            Dialog::MissingFfmpeg => {
                ui.label(RichText::new("FFmpeg is missing").font(theme::bold_font(22.0)));
                ui.add_space(6.0);
                ui.label(RichText::new("AI Video Editor needs ffmpeg.exe and ffprobe.exe in a folder named \"ffmpeg\" next to the app. Reinstall the app or copy them there, then restart.").color(TEXT_DIM));
                ui.add_space(14.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::primary_button(ui, "Quit", true).clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            }
        });
        if !visible {
            keep = false;
        }
        // A new dialog opened by the deferred action replaces this one.
        if keep && self.dialog.is_none() {
            self.dialog = Some(dialog);
        }
        if let Some(f) = deferred {
            f(self);
        }
    }

    fn toasts(&mut self, ctx: &egui::Context) {
        const LIFE: f32 = 4.0;
        self.toasts.retain(|t| t.born.elapsed().as_secs_f32() < LIFE);
        if self.toasts.is_empty() {
            return;
        }
        ctx.request_repaint();
        let screen = ctx.screen_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, EId::new("toasts")));
        let mut y = screen.bottom() - 286.0;
        for t in self.toasts.iter().rev() {
            let age = t.born.elapsed().as_secs_f32();
            let enter = theme::ease_out(age / 0.25);
            let exit = ((LIFE - age) / 0.3).clamp(0.0, 1.0);
            let a = enter.min(exit);
            let (icon, color) = match t.kind {
                ToastKind::Info => ("ℹ", TEXT_DIM),
                ToastKind::Success => ("✓", SUCCESS),
                ToastKind::Error => ("!", DANGER),
                ToastKind::Ai => ("✦", AI_GLOW),
            };
            let galley = painter.layout(t.text.clone(), theme::font(13.0), TEXT, 320.0);
            let size = vec2(galley.size().x + 52.0, galley.size().y + 20.0);
            let x = screen.right() - 16.0 - size.x + (1.0 - enter) * 40.0;
            let r = Rect::from_min_size(pos2(x, y - size.y), size);
            theme::shadow(&painter, r, R_MD as f32, a);
            painter.rect_filled(r, CornerRadius::same(R_MD), alpha(SURFACE_HI, a));
            painter.rect_filled(Rect::from_min_size(r.min, vec2(3.0, r.height())).shrink2(vec2(0.0, 8.0)), CornerRadius::same(2), alpha(color, a));
            painter.text(pos2(r.left() + 20.0, r.center().y), Align2::CENTER_CENTER, icon, theme::bold_font(14.0), alpha(color, a));
            painter.galley(pos2(r.left() + 36.0, r.top() + 10.0), galley, alpha(TEXT, a));
            y -= (size.y + 8.0) * a.max(0.3);
        }
    }

    fn file_drop(&mut self, ctx: &egui::Context) {
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        let a = theme::ease_bool(ctx, EId::new("file-hover"), hovering, 0.15);
        if a > 0.01 {
            let screen = ctx.screen_rect();
            let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, EId::new("drop-overlay")));
            p.rect_filled(screen, CornerRadius::ZERO, Color32::from_rgba_unmultiplied(8, 20, 48, (150.0 * a) as u8));
            let r = screen.shrink(24.0 + (1.0 - a) * 12.0);
            p.rect_stroke(r, CornerRadius::same(R_LG + 6), Stroke::new(2.0_f32, alpha(ACCENT_HI, a)), egui::StrokeKind::Inside);
            p.text(r.center() - vec2(0.0, 12.0), Align2::CENTER_CENTER, "Drop to add to your video", theme::bold_font(26.0), alpha(Color32::WHITE, a));
            p.text(r.center() + vec2(0.0, 22.0), Align2::CENTER_CENTER, "Videos and images go on the main track · music on the music track", theme::font(14.0), alpha(TEXT_DIM, a));
        }
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if dropped.is_empty() {
            return;
        }
        if !self.shared.editor.lock().has_project {
            self.toast(ToastKind::Info, "Create a project first");
            return;
        }
        let (media, skipped): (Vec<_>, Vec<_>) = dropped.into_iter().partition(|p| crate::shared::is_media_path(p) || p.is_dir());
        let mut files = Vec::new();
        for p in media {
            if p.is_dir() {
                if let Ok(rd) = std::fs::read_dir(&p) {
                    let mut inner: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| crate::shared::is_media_path(p)).collect();
                    inner.sort();
                    files.extend(inner);
                }
            } else {
                files.push(p);
            }
        }
        if !skipped.is_empty() {
            self.toast(ToastKind::Info, format!("Skipped {} unsupported file(s)", skipped.len()));
        }
        self.import(files, Placement::Timeline { main_index: None, music_start: 0.0 });
    }

    fn window_title(&mut self, ctx: &egui::Context) {
        let (name, dirty) = {
            let ed = self.shared.editor.lock();
            (ed.project.name.clone(), ed.is_dirty())
        };
        let title = format!("{}{} — AI Video Editor", name, if dirty { " •" } else { "" });
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn handle_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close {
            let (dirty, has_content) = {
                let ed = self.shared.editor.lock();
                (ed.is_dirty(), !ed.project.main.is_empty() || !ed.project.texts.is_empty() || !ed.project.music.is_empty())
            };
            if dirty && has_content {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.open_dialog(Dialog::Unsaved(Pending::Close));
            }
        }
        if self.allow_close {
            self.allow_close = false;
            self.stop();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.sync(ctx);
        if self.autochat.is_some() && !self.snapshot.main.is_empty() {
            let msg = self.autochat.take().unwrap();
            self.send_chat(msg);
        }
        self.shortcuts(ctx);
        self.update_preview(ctx);
        self.top_bar(ctx);
        self.library_panel(ctx);
        self.right_panel(ctx);
        self.timeline_panel(ctx);
        self.preview_panel(ctx);
        self.dialogs(ctx);
        self.file_drop(ctx);
        self.toasts(ctx);
        self.window_title(ctx);
        self.handle_close(ctx);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        let c = BG;
        [c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0, 1.0]
    }
}
