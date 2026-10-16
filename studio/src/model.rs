//! The project document: canvas, media assets, tracks and clips.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const FILE_EXTENSION: &str = "aivp";
pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Aspect {
    Landscape,
    Portrait,
    Square,
    Vertical4x5,
    Cinema,
}

impl Aspect {
    pub const ALL: [Aspect; 5] = [Aspect::Landscape, Aspect::Portrait, Aspect::Square, Aspect::Vertical4x5, Aspect::Cinema];

    pub fn ratio(self) -> (u32, u32) {
        match self {
            Aspect::Landscape => (16, 9),
            Aspect::Portrait => (9, 16),
            Aspect::Square => (1, 1),
            Aspect::Vertical4x5 => (4, 5),
            Aspect::Cinema => (21, 9),
        }
    }

    pub fn label(self) -> String {
        let (w, h) = self.ratio();
        format!("{w}:{h}")
    }

    pub fn use_case(self) -> &'static str {
        match self {
            Aspect::Landscape => "YouTube, websites",
            Aspect::Portrait => "Reels, Shorts, TikTok",
            Aspect::Square => "Feeds, LinkedIn",
            Aspect::Vertical4x5 => "Instagram feed",
            Aspect::Cinema => "Cinematic banners",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Resolution {
    Hd,
    FullHd,
    Qhd,
    Uhd,
}

impl Resolution {
    pub const ALL: [Resolution; 4] = [Resolution::Hd, Resolution::FullHd, Resolution::Qhd, Resolution::Uhd];

    /// Length of the shorter side in pixels.
    pub fn short_side(self) -> u32 {
        match self {
            Resolution::Hd => 720,
            Resolution::FullHd => 1080,
            Resolution::Qhd => 1440,
            Resolution::Uhd => 2160,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Resolution::Hd => "720p",
            Resolution::FullHd => "1080p",
            Resolution::Qhd => "1440p",
            Resolution::Uhd => "4K",
        }
    }
}

pub const FRAME_RATES: [u32; 4] = [24, 25, 30, 60];

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Canvas {
    pub aspect: Aspect,
    pub resolution: Resolution,
    pub fps: u32,
}

impl Default for Canvas {
    fn default() -> Self {
        Canvas { aspect: Aspect::Landscape, resolution: Resolution::FullHd, fps: 30 }
    }
}

impl Canvas {
    /// Pixel size, with both sides rounded to even numbers for H.264.
    pub fn size(&self) -> (u32, u32) {
        let (aw, ah) = self.aspect.ratio();
        let short = self.resolution.short_side() as f64;
        let (w, h) = if aw >= ah { (short * aw as f64 / ah as f64, short) } else { (short, short * ah as f64 / aw as f64) };
        let even = |v: f64| ((v / 2.0).round() as u32) * 2;
        (even(w), even(h))
    }

    pub fn summary(&self) -> String {
        format!("{} · {} · {} fps", self.aspect.label(), self.resolution.label(), self.fps)
    }
}

pub type Id = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaKind {
    Video,
    Audio,
    Image,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    pub id: Id,
    pub path: PathBuf,
    pub name: String,
    pub kind: MediaKind,
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub has_audio: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrackKind {
    Visual,
    Audio,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub id: Id,
    pub name: String,
    pub kind: TrackKind,
    pub muted: bool,
    pub locked: bool,
    pub hidden: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub version: u32,
    pub name: String,
    pub canvas: Canvas,
    pub assets: Vec<Asset>,
    pub tracks: Vec<Track>,
    pub next_id: Id,
}

impl Project {
    pub fn new(name: &str, canvas: Canvas) -> Project {
        let mut p = Project { version: FORMAT_VERSION, name: name.to_string(), canvas, assets: Vec::new(), tracks: Vec::new(), next_id: 1 };
        for (name, kind) in [("Text", TrackKind::Visual), ("Overlay", TrackKind::Visual), ("Main", TrackKind::Visual), ("Voiceover", TrackKind::Audio), ("Music", TrackKind::Audio)] {
            let id = p.alloc_id();
            p.tracks.push(Track { id, name: name.into(), kind, muted: false, locked: false, hidden: false });
        }
        p
    }

    pub fn alloc_id(&mut self) -> Id {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn load(path: &Path) -> anyhow::Result<Project> {
        let project: Project = serde_json::from_str(&std::fs::read_to_string(path)?)?;
        anyhow::ensure!(project.version <= FORMAT_VERSION, "This project was made with a newer version of the app");
        Ok(project)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canvas_sizes_are_even_and_match_presets() {
        let c = |aspect, resolution| Canvas { aspect, resolution, fps: 30 }.size();
        assert_eq!(c(Aspect::Landscape, Resolution::FullHd), (1920, 1080));
        assert_eq!(c(Aspect::Portrait, Resolution::FullHd), (1080, 1920));
        assert_eq!(c(Aspect::Vertical4x5, Resolution::FullHd), (1080, 1350));
        assert_eq!(c(Aspect::Cinema, Resolution::Hd), (1680, 720));
        assert_eq!(c(Aspect::Landscape, Resolution::Uhd), (3840, 2160));
    }

    #[test]
    fn project_round_trips() {
        let dir = std::env::temp_dir().join(format!("studio-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("p.aivp");
        let p = Project::new("Launch", Canvas::default());
        p.save(&path).unwrap();
        let q = Project::load(&path).unwrap();
        assert_eq!(q.name, "Launch");
        assert_eq!(q.tracks.len(), 5);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
