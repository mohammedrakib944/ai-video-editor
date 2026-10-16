//! A project plus its undo history.

use std::time::{Duration, Instant};

use crate::model::Project;

const UNDO_LIMIT: usize = 200;
/// Edits with the same merge key closer together than this become one undo step.
const MERGE_WINDOW: Duration = Duration::from_millis(900);

struct Entry {
    before: Project,
    desc: String,
    merge_key: Option<String>,
    at: Instant,
}

pub struct Doc {
    pub project: Project,
    undo: Vec<Entry>,
    redo: Vec<(Project, String)>,
    /// Bumped on every change; views re-render when it moves.
    pub revision: u64,
    saved_revision: u64,
}

impl Doc {
    pub fn new(project: Project) -> Doc {
        Doc { project, undo: Vec::new(), redo: Vec::new(), revision: 1, saved_revision: 1 }
    }

    /// Runs an edit as one undo step. Nothing changes if it fails.
    pub fn apply<T>(&mut self, desc: &str, f: impl FnOnce(&mut Project) -> anyhow::Result<T>) -> anyhow::Result<T> {
        self.apply_merge(desc, None, f)
    }

    /// Like `apply`, but consecutive edits with the same `key` (a slider drag, typing) merge.
    pub fn apply_merge<T>(&mut self, desc: &str, key: Option<&str>, f: impl FnOnce(&mut Project) -> anyhow::Result<T>) -> anyhow::Result<T> {
        let mut draft = self.project.clone();
        let out = f(&mut draft)?;
        let merge = key.is_some()
            && self.redo.is_empty()
            && self.undo.last().is_some_and(|e| e.merge_key.as_deref() == key && e.at.elapsed() < MERGE_WINDOW);
        let before = std::mem::replace(&mut self.project, draft);
        if merge {
            self.undo.last_mut().unwrap().at = Instant::now();
        } else {
            self.undo.push(Entry { before, desc: desc.to_string(), merge_key: key.map(str::to_string), at: Instant::now() });
            if self.undo.len() > UNDO_LIMIT {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.revision += 1;
        Ok(out)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|e| e.desc.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|(_, d)| d.as_str())
    }

    pub fn undo(&mut self) -> Option<String> {
        let e = self.undo.pop()?;
        let current = std::mem::replace(&mut self.project, e.before);
        self.redo.push((current, e.desc.clone()));
        self.revision += 1;
        Some(e.desc)
    }

    pub fn redo(&mut self) -> Option<String> {
        let (next, desc) = self.redo.pop()?;
        let current = std::mem::replace(&mut self.project, next);
        self.undo.push(Entry { before: current, desc: desc.clone(), merge_key: None, at: Instant::now() });
        self.revision += 1;
        Some(desc)
    }

    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }

    pub fn mark_saved(&mut self) {
        self.saved_revision = self.revision;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Canvas, TrackKind};

    #[test]
    fn failed_edits_change_nothing_and_undo_restores() {
        let mut d = Doc::new(Project::new("t", Canvas::default()));
        let tracks = d.project.tracks.len();
        assert!(d.apply("bad", |p| p.rename_track(0, "x")).is_err());
        assert!(!d.can_undo());
        d.apply("Add track", |p| Ok(p.add_track(TrackKind::Visual, "B", None))).unwrap();
        assert_eq!(d.project.tracks.len(), tracks + 1);
        assert!(d.is_dirty());
        assert_eq!(d.undo().as_deref(), Some("Add track"));
        assert_eq!(d.project.tracks.len(), tracks);
        d.redo().unwrap();
        assert_eq!(d.project.tracks.len(), tracks + 1);
    }

    #[test]
    fn merged_edits_are_one_step() {
        let mut d = Doc::new(Project::new("t", Canvas::default()));
        let id = d.project.tracks[0].id;
        for name in ["a", "ab", "abc"] {
            d.apply_merge("Rename", Some("rename"), |p| p.rename_track(id, name)).unwrap();
        }
        d.undo();
        assert_eq!(d.project.tracks[0].name, "Text");
        assert!(!d.can_undo());
    }
}
