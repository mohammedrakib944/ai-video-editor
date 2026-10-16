//! State shared between the UI thread, the MCP server and background jobs.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::bail;
use parking_lot::Mutex;

use crate::media::{self, MediaKind};
use crate::project::{Editor, ExportSettings, ExportStatus, Id, Source};
use crate::render;

#[derive(Clone)]
pub struct Shared {
    pub editor: Arc<Mutex<Editor>>,
    pub importing: Arc<Mutex<Vec<Importing>>>,
    repaint: Arc<dyn Fn() + Send + Sync>,
}

#[derive(Clone)]
pub struct Importing {
    pub name: String,
    pub started: Instant,
}

/// Where imported media should go.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Placement {
    LibraryOnly,
    /// Videos/images go to the end of the main track, audio onto the music track at `music_start`.
    Timeline { main_index: Option<usize>, music_start: f64 },
}

pub struct ImportReport {
    pub added: Vec<(Id, String, MediaKind)>,
    pub clip_ids: Vec<Id>,
    pub errors: Vec<String>,
}

impl Shared {
    pub fn new(editor: Editor, repaint: Arc<dyn Fn() + Send + Sync>) -> Shared {
        Shared { editor: Arc::new(Mutex::new(editor)), importing: Arc::new(Mutex::new(Vec::new())), repaint }
    }

    pub fn repaint(&self) {
        (self.repaint)();
    }

    pub fn repaint_fn(&self) -> Arc<dyn Fn() + Send + Sync> {
        self.repaint.clone()
    }

    /// Probes files (slow, so outside the lock) and adds them in one undoable step.
    pub fn import(&self, paths: &[PathBuf], placement: Placement, source: Source) -> ImportReport {
        let mut report = ImportReport { added: Vec::new(), clip_ids: Vec::new(), errors: Vec::new() };
        let mut probed = Vec::new();
        for path in paths {
            let path = std::fs::canonicalize(path).map(strip_verbatim).unwrap_or_else(|_| path.clone());
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let marker = Importing { name: name.clone(), started: Instant::now() };
            self.importing.lock().push(marker);
            self.repaint();
            let result = media::probe(&path);
            self.importing.lock().retain(|m| m.name != name);
            match result {
                Ok(info) => probed.push((path, info)),
                Err(e) => report.errors.push(format!("{name}: {e}")),
            }
        }
        if probed.is_empty() {
            self.repaint();
            return report;
        }
        let desc = if probed.len() == 1 {
            format!("Import {}", probed[0].0.file_name().unwrap_or_default().to_string_lossy())
        } else {
            format!("Import {} files", probed.len())
        };
        let mut ed = self.editor.lock();
        let result = ed.apply(source, &desc, |p| {
            let mut added = Vec::new();
            let mut clips = Vec::new();
            let mut index = match placement {
                Placement::Timeline { main_index, .. } => main_index,
                Placement::LibraryOnly => None,
            };
            for (path, info) in &probed {
                let id = p.add_asset(path, info);
                added.push((id, p.asset(id).unwrap().name.clone(), info.kind));
                if let Placement::Timeline { music_start, .. } = placement {
                    let clip = if info.kind == MediaKind::Audio {
                        p.add_music(id, music_start, None)?
                    } else {
                        let c = p.insert_main(id, index, None)?;
                        index = index.map(|i| i + 1);
                        c
                    };
                    clips.push(clip);
                }
            }
            Ok((added, clips))
        });
        drop(ed);
        match result {
            Ok((added, clips)) => {
                report.added = added;
                report.clip_ids = clips;
            }
            Err(e) => report.errors.push(e.to_string()),
        }
        self.repaint();
        report
    }

    /// Imports on a background thread; the UI shows placeholders meanwhile.
    pub fn import_async(&self, paths: Vec<PathBuf>, placement: Placement, on_done: impl FnOnce(ImportReport) + Send + 'static) {
        let shared = self.clone();
        std::thread::spawn(move || {
            let report = shared.import(&paths, placement, Source::User);
            on_done(report);
        });
    }

    pub fn start_export(&self, settings: ExportSettings, source: Source) -> anyhow::Result<()> {
        let project = {
            let mut ed = self.editor.lock();
            if matches!(ed.export_status, ExportStatus::Running { .. }) {
                bail!("An export is already running");
            }
            if ed.project.duration() <= 0.0 {
                bail!("The timeline is empty — add a clip before exporting");
            }
            ed.cancel_export = false;
            ed.export_status = ExportStatus::Running { progress: 0.0, eta_secs: None, path: settings.path.clone() };
            ed.push_log(source, format!("Export started: {}", settings.path.display()), true);
            ed.project.clone()
        };
        self.repaint();
        let shared = self.clone();
        std::thread::Builder::new().name("export".into()).spawn(move || {
            let path = settings.path.clone();
            let result = render::export(&project, settings.canvas, settings.quality, &path, |p| {
                let mut ed = shared.editor.lock();
                if ed.cancel_export {
                    return false;
                }
                ed.export_status = ExportStatus::Running { progress: p.progress, eta_secs: p.eta_secs, path: path.clone() };
                drop(ed);
                shared.repaint();
                true
            });
            let mut ed = shared.editor.lock();
            ed.export_status = match result {
                Ok(secs) => {
                    ed.push_log(source, format!("Exported {}", path.display()), true);
                    ExportStatus::Done { path, secs }
                }
                Err(e) if e.to_string() == "cancelled" => {
                    ed.push_log(source, "Export cancelled".into(), true);
                    ExportStatus::Cancelled
                }
                Err(e) => {
                    ed.push_log(source, format!("{e}"), false);
                    ExportStatus::Failed { error: e.to_string() }
                }
            };
            drop(ed);
            shared.repaint();
        })?;
        Ok(())
    }

    pub fn cancel_export(&self) {
        self.editor.lock().cancel_export = true;
    }
}

/// `canonicalize` on Windows returns `\\?\C:\...`, which ffmpeg and users dislike.
fn strip_verbatim(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        if !rest.starts_with("UNC\\") {
            return PathBuf::from(rest);
        }
    }
    p
}

pub fn is_media_path(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| media::MEDIA_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}
