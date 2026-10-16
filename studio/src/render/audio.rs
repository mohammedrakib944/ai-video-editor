//! Builds the FFmpeg audio mix for a project: every audible clip trimmed,
//! sped up, faded, delayed to its timeline position and mixed.

use std::path::PathBuf;

use crate::clip::Content;
use crate::model::{MediaKind, Project, TrackKind};

/// One input file for the mix, with the seek and length to read.
pub struct AudioInput {
    pub path: PathBuf,
    pub seek: f64,
    pub length: f64,
}

/// FFmpeg inputs (starting at index `first_input`) and a filter graph ending in `[aout]`.
pub fn mix_graph(project: &Project, first_input: usize, total: f64) -> (Vec<AudioInput>, String) {
    let mut inputs = Vec::new();
    let mut chains = Vec::new();
    for clip in &project.clips {
        let Content::Media { asset, src_in, speed } = clip.content else { continue };
        let Some(a) = project.asset(asset) else { continue };
        let Some(track) = project.track(clip.track) else { continue };
        let audible = a.has_audio && a.kind != MediaKind::Image && !clip.audio.muted && clip.audio.volume > 0.0 && !(track.kind == TrackKind::Audio && track.muted);
        if !audible || clip.start >= total {
            continue;
        }
        let k = first_input + inputs.len();
        inputs.push(AudioInput { path: a.path.clone(), seek: src_in, length: clip.duration * speed + 0.05 });
        let mut f = format!("[{k}:a]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo");
        f.push_str(&atempo(speed));
        f.push_str(&format!(",atrim=0:{:.6},asetpts=PTS-STARTPTS", clip.duration));
        f.push_str(&format!(",volume={:.4}", clip.audio.volume));
        if clip.audio.fade_in > 0.0 {
            f.push_str(&format!(",afade=t=in:st=0:d={:.4}", clip.audio.fade_in.min(clip.duration)));
        }
        if clip.audio.fade_out > 0.0 {
            let d = clip.audio.fade_out.min(clip.duration);
            f.push_str(&format!(",afade=t=out:st={:.4}:d={:.4}", clip.duration - d, d));
        }
        let ms = (clip.start * 1000.0).round() as u64;
        f.push_str(&format!(",adelay=delays={ms}:all=1[a{}]", chains.len()));
        chains.push(f);
    }
    let n = chains.len();
    let mut graph = chains.join(";\n");
    if n == 0 {
        graph = format!("anullsrc=r=48000:cl=stereo,atrim=duration={total:.6}[aout]");
    } else {
        let labels: String = (0..n).map(|i| format!("[a{i}]")).collect();
        graph.push_str(&format!(";\n{labels}amix=inputs={n}:normalize=0:dropout_transition=0,apad,atrim=duration={total:.6}[aout]"));
    }
    (inputs, graph)
}

/// `atempo` only accepts 0.5–2.0 per stage, so larger changes are chained.
fn atempo(speed: f64) -> String {
    if (speed - 1.0).abs() < 1e-6 {
        return String::new();
    }
    let mut s = speed;
    let mut out = String::new();
    while s > 2.0 {
        out.push_str(",atempo=2.0");
        s /= 2.0;
    }
    while s < 0.5 {
        out.push_str(",atempo=0.5");
        s /= 0.5;
    }
    out.push_str(&format!(",atempo={s:.6}"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tempo_chains_stay_in_range() {
        assert_eq!(atempo(1.0), "");
        assert_eq!(atempo(4.0), ",atempo=2.0,atempo=2.000000");
        assert_eq!(atempo(0.25), ",atempo=0.5,atempo=0.500000");
    }

    #[test]
    fn silent_projects_still_produce_audio() {
        let p = Project::new("t", Default::default());
        let (inputs, graph) = mix_graph(&p, 1, 5.0);
        assert!(inputs.is_empty());
        assert!(graph.contains("anullsrc") && graph.ends_with("[aout]"));
    }
}
