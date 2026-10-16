//! The assistant's tools: their schemas, and how the editor carries them out.
//! Every change is one undo step, so the user can take back anything.

use std::path::PathBuf;

use anyhow::{Context, anyhow, bail};
use iced::Task;
use serde_json::{Value, json};

use super::{Editor, Message, SideTab};
use crate::clip::{Align, Clip, Content, Ease, Keyframe, Motion, MotionKind, Prop, Rgba, ShapeKind, ShapeLayer, TextLayer, Transition, TransitionKind, Zoom};
use crate::mcp::{self, Call};
use crate::model::{Aspect, Id, MediaKind, Project, Resolution, TrackKind};

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({ "name": name, "description": description, "inputSchema": { "type": "object", "properties": properties, "required": required } })
}

fn num(d: &str) -> Value {
    json!({ "type": "number", "description": d })
}
fn int(d: &str) -> Value {
    json!({ "type": "integer", "description": d })
}
fn string(d: &str) -> Value {
    json!({ "type": "string", "description": d })
}
fn boolean(d: &str) -> Value {
    json!({ "type": "boolean", "description": d })
}
fn choice(values: &[&str], d: &str) -> Value {
    json!({ "type": "string", "enum": values, "description": d })
}

const MOTIONS: &[&str] = &["none", "fade", "slide_up", "slide_down", "slide_left", "slide_right", "pop", "zoom_in"];
const TRANSITIONS: &[&str] = &["none", "crossfade", "slide", "slide_up", "push", "zoom_through", "wipe"];
const EASES: &[&str] = &["linear", "smooth", "ease_in", "ease_out", "back"];
const PROPS: &[&str] = &["x", "y", "scale", "rotation", "opacity"];

fn style_props() -> serde_json::Map<String, Value> {
    let v = json!({
        "x": num("Center x, fraction of canvas width (0 left, 1 right)"),
        "y": num("Center y, fraction of canvas height (0 top, 1 bottom)"),
        "scale": num("1 = media fits the canvas; text/shapes at their own size"),
        "rotation": num("Degrees, clockwise"),
        "opacity": num("0–1"),
        "fit": choice(&["fit", "fill"], "Media framing: whole frame visible, or fill the canvas"),
        "corner_radius": num("Rounded corners, canvas units"),
        "border_width": num("Border, canvas units (0 = none)"),
        "border_color": string("#RRGGBB"),
        "shadow": boolean("Soft drop shadow"),
        "shadow_blur": num("Shadow softness, canvas units"),
        "brightness": num("-1…1"),
        "contrast": num("-1…1"),
        "saturation": num("-1…1"),
        "warmth": num("-1…1"),
        "volume": num("0–2 (1 = original)"),
        "muted": boolean("Silence this clip"),
        "fade_in": num("Audio fade in, seconds"),
        "fade_out": num("Audio fade out, seconds"),
        "intro": choice(MOTIONS, "Entrance animation"),
        "intro_duration": num("Seconds"),
        "outro": choice(MOTIONS, "Exit animation"),
        "outro_duration": num("Seconds"),
        "transition": choice(TRANSITIONS, "Transition from the previous clip on the same track into this one"),
        "transition_duration": num("Seconds"),
    });
    v.as_object().unwrap().clone()
}

fn text_props() -> serde_json::Map<String, Value> {
    let v = json!({
        "text": string("The text; use \\n for line breaks. Bangla is fully supported."),
        "font": choice(&["Inter", "Poppins", "Hind Siliguri"], "Inter or Poppins for English, Hind Siliguri for Bangla"),
        "weight": int("400 regular, 500 medium, 600 semibold, 700 bold"),
        "size": num("Font size, canvas units (96 is a big headline at 1080p)"),
        "color": string("#RRGGBB"),
        "align": choice(&["left", "center", "right"], "Alignment of multi-line text"),
        "background": string("#RRGGBB box behind the text, or \"none\""),
    });
    v.as_object().unwrap().clone()
}

fn merged(parts: &[serde_json::Map<String, Value>], extra: Value) -> Value {
    let mut m = serde_json::Map::new();
    for p in parts {
        m.extend(p.clone());
    }
    m.extend(extra.as_object().unwrap().clone());
    Value::Object(m)
}

pub fn schemas() -> Vec<Value> {
    vec![
        tool("get_project", "The whole project: canvas, tracks (top to bottom; higher video tracks draw on top), media library, every clip with its timing and properties, the playhead, the user's selected clips and selected time range. Call this first.", json!({}), &[]),
        tool(
            "import_media",
            "Import video, audio or image files (absolute paths) into the media library.",
            json!({ "paths": { "type": "array", "items": { "type": "string" } }, "add_to_timeline": boolean("Also place them on the timeline at the playhead") }),
            &["paths"],
        ),
        tool("add_media_clip", "Put a library item on the timeline. Videos/images go on a video track, audio on an audio track. Returns the clip id.", json!({ "media_id": int("Id from get_project media"), "start": num("Timeline seconds"), "track_id": int("Optional track") }), &["media_id", "start"]),
        tool(
            "add_text",
            "Add a text layer (titles, callouts, captions). Returns the clip id.",
            merged(&[text_props(), style_props()], json!({ "start": num("Timeline seconds"), "duration": num("Seconds (default 4)"), "track_id": int("Optional video track") })),
            &["text", "start"],
        ),
        tool(
            "add_shape",
            "Add a rectangle or ellipse (cards, highlights, backgrounds). Returns the clip id.",
            merged(&[style_props()], json!({
                "start": num("Timeline seconds"), "duration": num("Seconds (default 4)"),
                "shape": choice(&["rectangle", "ellipse", "triangle", "star", "hexagon", "ring"], "Default rectangle (use corner_radius for rounded cards and pills)"),
                "width": num("Canvas units"), "height": num("Canvas units"),
                "color": string("#RRGGBB fill"), "color_to": string("#RRGGBB second color for a gradient"),
                "gradient": choice(&["linear", "radial"], "Gradient style when color_to is set"),
                "gradient_angle": num("Linear gradient direction in degrees: 180 top→bottom, 90 left→right"),
                "thickness": num("Ring outline thickness, canvas units"),
                "track_id": int("Optional video track"),
            })),
            &["start", "width", "height", "color"],
        ),
        tool("update_clip", "Change any properties of a clip. Only the fields you pass change.", merged(&[style_props(), text_props()], json!({ "clip_id": int(""), "name": string("Label on the timeline"), "speed": num("Playback speed, 0.25–4 (video/audio)") })), &["clip_id"]),
        tool("move_clip", "Move a clip to a new start time (and optionally another track of the same kind).", json!({ "clip_id": int(""), "start": num("Timeline seconds"), "track_id": int("Optional") }), &["clip_id", "start"]),
        tool("trim_clip", "Change where a clip starts and/or ends on the timeline (media can't extend past its source).", json!({ "clip_id": int(""), "start": num("New start, timeline seconds"), "end": num("New end, timeline seconds") }), &["clip_id"]),
        tool("split_clip", "Split clips at a time. Without clip_id, splits every clip under that time.", json!({ "time": num("Timeline seconds"), "clip_id": int("Optional") }), &["time"]),
        tool("delete_clips", "Remove clips from the timeline.", json!({ "clip_ids": { "type": "array", "items": { "type": "integer" } } }), &["clip_ids"]),
        tool("duplicate_clip", "Copy a clip right after itself. Returns the new id.", json!({ "clip_id": int("") }), &["clip_id"]),
        tool(
            "add_zoom",
            "Camtasia-style zoom on a video or image clip: magnify part of the frame, easing in and out. Back-to-back zooms pan smoothly from one spot to the next. Use for screen recordings to highlight UI.",
            json!({
                "clip_id": int(""), "start": num("Timeline seconds"), "duration": num("Seconds"),
                "scale": num("Magnification, e.g. 1.6–2.5"), "focus_x": num("Point to zoom into, fraction of the frame width"), "focus_y": num("Fraction of the frame height"),
                "ease": num("Seconds to ease in/out (default 0.6)"),
            }),
            &["clip_id", "start", "duration", "scale", "focus_x", "focus_y"],
        ),
        tool("update_zoom", "Change a zoom region.", json!({ "clip_id": int(""), "zoom_id": int(""), "start": num(""), "duration": num(""), "scale": num(""), "focus_x": num(""), "focus_y": num(""), "ease": num("") }), &["clip_id", "zoom_id"]),
        tool("remove_zoom", "Remove a zoom region.", json!({ "clip_id": int(""), "zoom_id": int("") }), &["clip_id", "zoom_id"]),
        tool(
            "set_keyframes",
            "Animate a property with keyframes (replaces existing keys for that property). Use for custom motion: e.g. drift a product shot, scale a logo up, slide a card along a path.",
            json!({
                "clip_id": int(""), "property": choice(PROPS, "x/y are canvas fractions, rotation in degrees, scale and opacity as numbers"),
                "keys": { "type": "array", "items": { "type": "object", "properties": { "time": num("Timeline seconds"), "value": num(""), "ease": choice(EASES, "Easing to the next key") }, "required": ["time", "value"] } },
            }),
            &["clip_id", "property", "keys"],
        ),
        tool("clear_keyframes", "Remove keyframes from a clip (one property, or all).", json!({ "clip_id": int(""), "property": choice(PROPS, "Optional") }), &["clip_id"]),
        tool("add_sound", "Add a built-in sound effect on an audio track. Sounds: whoosh, swoosh, pop, click, ding, riser, impact, typing, chime.", json!({ "sound": choice(&["whoosh", "swoosh", "pop", "click", "ding", "riser", "impact", "typing", "chime"], ""), "start": num("Timeline seconds"), "volume": num("0–2, default 1") }), &["sound", "start"]),
        tool("add_track", "Add a track.", json!({ "kind": choice(&["video", "audio"], ""), "name": string("") }), &["kind"]),
        tool("rename_track", "Rename a track.", json!({ "track_id": int(""), "name": string("") }), &["track_id", "name"]),
        tool("set_canvas", "Change the video format.", json!({ "aspect": choice(&["16:9", "9:16", "1:1", "4:5", "21:9"], ""), "resolution": choice(&["720p", "1080p", "1440p", "4K"], ""), "fps": choice(&["24", "25", "30", "60"], "") }), &[]),
        tool("get_frame", "See the rendered video at a time (as an image), exactly as it will export. Use it to check your work.", json!({ "time": num("Timeline seconds (default: playhead)") }), &[]),
        tool("seek", "Move the playhead so the user sees a moment.", json!({ "time": num("Timeline seconds") }), &["time"]),
        tool("play", "Start playback in the preview.", json!({}), &[]),
        tool("pause", "Pause playback.", json!({}), &[]),
        tool("undo", "Undo the last change.", json!({}), &[]),
        tool("redo", "Redo.", json!({}), &[]),
        tool("export_video", "Render the project to an MP4 file in the background.", json!({ "path": string("Absolute .mp4 path"), "quality": choice(&["high", "standard", "small"], "Default high") }), &["path"]),
        tool("get_export_status", "Progress of the current export.", json!({}), &[]),
    ]
}

// ---------------------------------------------------------------- arguments

fn f(a: &Value, k: &str) -> Option<f64> {
    a.get(k).and_then(Value::as_f64)
}
fn req_f(a: &Value, k: &str) -> anyhow::Result<f64> {
    f(a, k).ok_or_else(|| anyhow!("`{k}` must be a number"))
}
fn id(a: &Value, k: &str) -> Option<Id> {
    a.get(k).and_then(Value::as_u64)
}
fn req_id(a: &Value, k: &str) -> anyhow::Result<Id> {
    id(a, k).ok_or_else(|| anyhow!("`{k}` must be an id from get_project"))
}
fn s<'a>(a: &'a Value, k: &str) -> Option<&'a str> {
    a.get(k).and_then(Value::as_str)
}
fn color(a: &Value, k: &str) -> anyhow::Result<Option<Rgba>> {
    match s(a, k) {
        None => Ok(None),
        Some(v) => Rgba::parse(v).map(Some).ok_or_else(|| anyhow!("`{k}` must be a color like #7C6DFF")),
    }
}

fn motion(v: &str) -> anyhow::Result<Option<MotionKind>> {
    Ok(Some(match v {
        "none" => return Ok(None),
        "fade" => MotionKind::Fade,
        "slide_up" => MotionKind::SlideUp,
        "slide_down" => MotionKind::SlideDown,
        "slide_left" => MotionKind::SlideLeft,
        "slide_right" => MotionKind::SlideRight,
        "pop" => MotionKind::Pop,
        "zoom_in" => MotionKind::ZoomIn,
        other => bail!("Unknown animation `{other}`"),
    }))
}

fn transition(v: &str) -> anyhow::Result<Option<TransitionKind>> {
    Ok(Some(match v {
        "none" => return Ok(None),
        "crossfade" => TransitionKind::Crossfade,
        "slide" => TransitionKind::SlideLeft,
        "slide_up" => TransitionKind::SlideUp,
        "push" => TransitionKind::Push,
        "zoom_through" => TransitionKind::ZoomThrough,
        "wipe" => TransitionKind::Wipe,
        other => bail!("Unknown transition `{other}`"),
    }))
}

fn prop(v: &str) -> anyhow::Result<Prop> {
    Ok(match v {
        "x" => Prop::X,
        "y" => Prop::Y,
        "scale" => Prop::Scale,
        "rotation" => Prop::Rotation,
        "opacity" => Prop::Opacity,
        other => bail!("Unknown property `{other}`"),
    })
}

fn ease(v: Option<&str>) -> Ease {
    match v {
        Some("linear") => Ease::Linear,
        Some("ease_in") => Ease::EaseIn,
        Some("ease_out") => Ease::EaseOut,
        Some("back") => Ease::Back,
        _ => Ease::Smooth,
    }
}

/// Applies the shared style/text fields from `a` to a clip.
fn apply_props(c: &mut Clip, a: &Value) -> anyhow::Result<()> {
    let t = &mut c.transform;
    if let Some(v) = f(a, "x") {
        t.x = v;
    }
    if let Some(v) = f(a, "y") {
        t.y = v;
    }
    if let Some(v) = f(a, "scale") {
        t.scale = v.clamp(0.01, 20.0);
    }
    if let Some(v) = f(a, "rotation") {
        t.rotation = v;
    }
    if let Some(v) = f(a, "opacity") {
        t.opacity = v.clamp(0.0, 1.0);
    }
    match s(a, "fit") {
        Some("fit") => t.fit = crate::clip::Fit::Contain,
        Some("fill") => t.fit = crate::clip::Fit::Cover,
        _ => {}
    }
    let l = &mut c.look;
    if let Some(v) = f(a, "corner_radius") {
        l.corner_radius = v.max(0.0);
    }
    if let Some(v) = f(a, "border_width") {
        l.border_width = v.max(0.0);
    }
    if let Some(col) = color(a, "border_color")? {
        l.border_color = col;
    }
    if let Some(v) = a.get("shadow").and_then(Value::as_bool) {
        l.shadow.enabled = v;
    }
    if let Some(v) = f(a, "shadow_blur") {
        l.shadow.blur = v.max(0.0);
        l.shadow.enabled = true;
    }
    let g = &mut c.grade;
    for (k, field) in [("brightness", &mut g.brightness), ("contrast", &mut g.contrast), ("saturation", &mut g.saturation), ("warmth", &mut g.temperature)] {
        if let Some(v) = f(a, k) {
            *field = v.clamp(-1.0, 1.0);
        }
    }
    if let Some(v) = f(a, "volume") {
        c.audio.volume = v.clamp(0.0, 2.0);
    }
    if let Some(v) = a.get("muted").and_then(Value::as_bool) {
        c.audio.muted = v;
    }
    if let Some(v) = f(a, "fade_in") {
        c.audio.fade_in = v.max(0.0);
    }
    if let Some(v) = f(a, "fade_out") {
        c.audio.fade_out = v.max(0.0);
    }
    let half = (c.duration / 2.0).max(0.05);
    if let Some(v) = s(a, "intro") {
        c.intro = motion(v)?.map(|kind| Motion { kind, duration: c.intro.map(|m| m.duration).unwrap_or(0.6).min(half) });
    }
    if let (Some(d), Some(m)) = (f(a, "intro_duration"), c.intro.as_mut()) {
        m.duration = d.clamp(0.05, half);
    }
    if let Some(v) = s(a, "outro") {
        c.outro = motion(v)?.map(|kind| Motion { kind, duration: c.outro.map(|m| m.duration).unwrap_or(0.6).min(half) });
    }
    if let (Some(d), Some(m)) = (f(a, "outro_duration"), c.outro.as_mut()) {
        m.duration = d.clamp(0.05, half);
    }
    if let Some(v) = s(a, "transition") {
        c.transition = transition(v)?.map(|kind| Transition { kind, duration: c.transition.map(|t| t.duration).unwrap_or(0.6).min(half) });
    }
    if let (Some(d), Some(t)) = (f(a, "transition_duration"), c.transition.as_mut()) {
        t.duration = d.clamp(0.05, half);
    }
    if let Content::Text(tl) = &mut c.content {
        if let Some(v) = s(a, "text") {
            tl.text = v.replace("\\n", "\n");
        }
        if let Some(v) = s(a, "font") {
            tl.font = v.to_string();
        }
        if let Some(v) = a.get("weight").and_then(Value::as_u64) {
            tl.weight = (v as u16).clamp(100, 900);
        }
        if let Some(v) = f(a, "size") {
            tl.size = v.clamp(4.0, 1000.0);
        }
        if let Some(col) = color(a, "color")? {
            tl.color = col;
        }
        match s(a, "align") {
            Some("left") => tl.align = Align::Left,
            Some("center") => tl.align = Align::Center,
            Some("right") => tl.align = Align::Right,
            _ => {}
        }
        match s(a, "background") {
            Some("none") => tl.background = None,
            Some(_) => tl.background = color(a, "background")?.map(|c| Rgba(c.0, c.1, c.2, 0.6)),
            None => {}
        }
    }
    Ok(())
}

fn kind_name(p: &Project, c: &Clip) -> &'static str {
    match &c.content {
        Content::Text(_) => "text",
        Content::Shape(_) => "shape",
        Content::Media { asset, .. } => match p.asset(*asset).map(|a| a.kind) {
            Some(MediaKind::Audio) => "audio",
            Some(MediaKind::Image) => "image",
            _ => "video",
        },
    }
}

impl Editor {
    fn project_json(&self) -> Value {
        let p = &self.doc.project;
        let (w, h) = p.canvas.size();
        let clips: Vec<Value> = p
            .clips
            .iter()
            .map(|c| {
                let mut o = json!({
                    "id": c.id, "track_id": c.track, "start": r3(c.start), "end": r3(c.end()), "kind": kind_name(p, c),
                    "name": super::timeline::clip_label(p, c),
                    "x": r3(c.transform.x), "y": r3(c.transform.y), "scale": r3(c.transform.scale), "rotation": r3(c.transform.rotation), "opacity": r3(c.transform.opacity),
                });
                let m = o.as_object_mut().unwrap();
                if let Content::Text(t) = &c.content {
                    m.insert("text".into(), json!(t.text));
                    m.insert("font".into(), json!(t.font));
                    m.insert("size".into(), json!(t.size));
                    m.insert("weight".into(), json!(t.weight));
                    m.insert("color".into(), json!(t.color.to_hex()));
                }
                if let Content::Media { asset, speed, src_in } = c.content {
                    m.insert("media_id".into(), json!(asset));
                    m.insert("source_start".into(), json!(r3(src_in)));
                    if speed != 1.0 {
                        m.insert("speed".into(), json!(speed));
                    }
                    m.insert("volume".into(), json!(c.audio.volume));
                }
                if c.look.corner_radius > 0.0 {
                    m.insert("corner_radius".into(), json!(c.look.corner_radius));
                }
                if c.look.shadow.enabled {
                    m.insert("shadow".into(), json!(true));
                }
                if let Some(i) = c.intro {
                    m.insert("intro".into(), json!(format!("{:?}", i.kind)));
                }
                if let Some(o2) = c.outro {
                    m.insert("outro".into(), json!(format!("{:?}", o2.kind)));
                }
                if let Some(t) = c.transition {
                    m.insert("transition".into(), json!(format!("{:?}", t.kind)));
                }
                if !c.zooms.is_empty() {
                    let zooms: Vec<Value> = c.zooms.iter().map(|z| json!({ "id": z.id, "start": r3(c.start + z.at), "end": r3(c.start + z.at + z.duration), "scale": z.scale, "focus_x": r3(z.focus_x), "focus_y": r3(z.focus_y) })).collect();
                    m.insert("zooms".into(), json!(zooms));
                }
                if !c.keyframes.is_empty() {
                    let keys: serde_json::Map<String, Value> =
                        c.keyframes.iter().map(|(k, v)| (format!("{k:?}").to_lowercase(), json!(v.iter().map(|kf| json!({ "time": r3(c.start + kf.t), "value": kf.value })).collect::<Vec<_>>()))).collect();
                    m.insert("keyframes".into(), Value::Object(keys));
                }
                o
            })
            .collect();
        json!({
            "canvas": { "aspect": p.canvas.aspect.label(), "resolution": p.canvas.resolution.label(), "fps": p.canvas.fps, "width": w, "height": h },
            "duration": r3(p.duration()),
            "playhead": r3(self.playhead),
            "selected_clip_ids": self.selection,
            "selected_range": self.range.map(|(a, b)| json!({ "start": r3(a), "end": r3(b) })),
            "tracks": p.tracks.iter().map(|t| json!({ "id": t.id, "name": t.name, "kind": if t.kind == TrackKind::Audio { "audio" } else { "video" }, "locked": t.locked, "hidden": t.hidden, "muted": t.muted })).collect::<Vec<_>>(),
            "media": p.assets.iter().map(|a| json!({ "id": a.id, "name": a.name, "kind": format!("{:?}", a.kind).to_lowercase(), "duration": r3(a.duration), "width": a.width, "height": a.height, "has_audio": a.has_audio, "path": a.path })).collect::<Vec<_>>(),
            "clips": clips,
        })
    }

    /// Runs an edit for the assistant and reports the outcome.
    fn ai_edit<T>(&mut self, desc: &str, f: impl FnOnce(&mut Project) -> anyhow::Result<T>) -> anyhow::Result<T> {
        let out = self.doc.apply(desc, f)?;
        self.lanes_cache.clear();
        self.after_history();
        self.notify(format!("Assistant: {desc}"), false);
        Ok(out)
    }

    /// Carries out one tool call. Slow tools answer later from a task.
    pub fn handle_tool(&mut self, call: Call) -> Task<Message> {
        let a = call.args.clone();
        let result: anyhow::Result<String> = match call.name.as_str() {
            "get_project" => Ok(serde_json::to_string_pretty(&self.project_json()).unwrap_or_default()),
            "import_media" => {
                let paths: Vec<PathBuf> = a["paths"].as_array().map(|v| v.iter().filter_map(|p| p.as_str().map(PathBuf::from)).collect()).unwrap_or_default();
                if paths.is_empty() {
                    Err(anyhow!("`paths` is empty"))
                } else {
                    let add = a["add_to_timeline"].as_bool().unwrap_or(false);
                    return Task::perform(
                        async move {
                            tokio::task::spawn_blocking(move || paths.into_iter().map(|p| (p.clone(), crate::media::probe(&p).map_err(|e| e.to_string()))).collect::<Vec<_>>()).await.unwrap_or_default()
                        },
                        move |results| Message::ToolImported(call.clone(), results, add),
                    );
                }
            }
            "add_media_clip" => (|| {
                let (asset, start) = (req_id(&a, "media_id")?, req_f(&a, "start")?);
                let track = id(&a, "track_id");
                let clip = self.ai_edit("Add clip", |p| p.add_media_clip(asset, track, start))?;
                self.selection = vec![clip];
                Ok(format!("Added clip {clip}"))
            })(),
            "add_text" => (|| {
                let start = req_f(&a, "start")?;
                let track = id(&a, "track_id");
                let duration = f(&a, "duration").unwrap_or(4.0);
                let mut layer = TextLayer { text: s(&a, "text").unwrap_or("Text").replace("\\n", "\n"), ..Default::default() };
                if !layer.text.is_ascii() && s(&a, "font").is_none() {
                    layer.font = "Hind Siliguri".into();
                    layer.weight = 600;
                    layer.letter_spacing = 0.0;
                }
                let clip = self.ai_edit("Add text", |p| {
                    let id = p.add_text_clip(track, start, duration.max(0.1), layer)?;
                    let c = p.clip_mut(id)?;
                    apply_props(c, &a)?;
                    Ok(id)
                })?;
                self.selection = vec![clip];
                Ok(format!("Added text clip {clip}"))
            })(),
            "add_shape" => (|| {
                let start = req_f(&a, "start")?;
                let fill = color(&a, "color")?.context("`color` is required")?;
                let kind = match s(&a, "shape").unwrap_or("rectangle") {
                    "ellipse" | "circle" => ShapeKind::Ellipse,
                    "triangle" => ShapeKind::Triangle,
                    "star" => ShapeKind::Star,
                    "hexagon" => ShapeKind::Hexagon,
                    "ring" => ShapeKind::Ring,
                    _ => ShapeKind::Rectangle,
                };
                let mut shape = ShapeLayer::new(kind, req_f(&a, "width")?.max(1.0), req_f(&a, "height")?.max(1.0), fill, color(&a, "color_to")?);
                if let Some(v) = f(&a, "gradient_angle") {
                    shape.gradient_angle = v;
                }
                shape.radial = s(&a, "gradient") == Some("radial");
                if let Some(v) = f(&a, "thickness") {
                    shape.thickness = v.max(1.0);
                }
                let track = id(&a, "track_id");
                let duration = f(&a, "duration").unwrap_or(4.0);
                let clip = self.ai_edit("Add shape", |p| {
                    let id = p.add_shape_clip(track, start, duration.max(0.1), shape)?;
                    let c = p.clip_mut(id)?;
                    apply_props(c, &a)?;
                    Ok(id)
                })?;
                self.selection = vec![clip];
                Ok(format!("Added shape clip {clip}"))
            })(),
            "update_clip" => (|| {
                let clip = req_id(&a, "clip_id")?;
                self.ai_edit("Update clip", |p| {
                    if let Some(speed) = f(&a, "speed") {
                        p.set_speed(clip, speed)?;
                    }
                    let c = p.clip_mut(clip)?;
                    if let Some(n) = s(&a, "name") {
                        c.name = Some(n.to_string());
                    }
                    apply_props(c, &a)
                })?;
                Ok(format!("Updated clip {clip}"))
            })(),
            "move_clip" => (|| {
                let (clip, start) = (req_id(&a, "clip_id")?, req_f(&a, "start")?);
                let track = match id(&a, "track_id") {
                    Some(t) => t,
                    None => self.doc.project.clip(clip).context("Unknown clip")?.track,
                };
                self.ai_edit("Move clip", |p| p.move_clip(clip, track, start))?;
                let c = self.doc.project.clip(clip).unwrap();
                Ok(format!("Clip {clip} now at {:.2}–{:.2}s", c.start, c.end()))
            })(),
            "trim_clip" => (|| {
                let clip = req_id(&a, "clip_id")?;
                self.ai_edit("Trim clip", |p| {
                    if let Some(t) = f(&a, "start") {
                        p.trim(clip, crate::edit::Edge::Start, t)?;
                    }
                    if let Some(t) = f(&a, "end") {
                        p.trim(clip, crate::edit::Edge::End, t)?;
                    }
                    Ok(())
                })?;
                let c = self.doc.project.clip(clip).unwrap();
                Ok(format!("Clip {clip} now at {:.2}–{:.2}s", c.start, c.end()))
            })(),
            "split_clip" => (|| {
                let t = req_f(&a, "time")?;
                let targets: Vec<Id> = match id(&a, "clip_id") {
                    Some(c) => vec![c],
                    None => self.doc.project.clips.iter().filter(|c| t > c.start && t < c.end()).map(|c| c.id).collect(),
                };
                anyhow::ensure!(!targets.is_empty(), "No clip under {t:.2}s");
                let new = self.ai_edit("Split", |p| targets.iter().map(|&c| p.split(c, t)).collect::<anyhow::Result<Vec<_>>>())?;
                Ok(format!("Split; new clips {new:?}"))
            })(),
            "delete_clips" => (|| {
                let ids: Vec<Id> = a["clip_ids"].as_array().map(|v| v.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
                self.ai_edit("Delete clips", |p| p.delete_clips(&ids))?;
                Ok(format!("Deleted {} clip(s)", ids.len()))
            })(),
            "duplicate_clip" => (|| {
                let clip = req_id(&a, "clip_id")?;
                let new = self.ai_edit("Duplicate clip", |p| p.duplicate_clip(clip))?;
                Ok(format!("New clip {new}"))
            })(),
            "add_zoom" => (|| {
                let clip = req_id(&a, "clip_id")?;
                let (start, duration, scale) = (req_f(&a, "start")?, req_f(&a, "duration")?, req_f(&a, "scale")?);
                let (fx, fy) = (req_f(&a, "focus_x")?, req_f(&a, "focus_y")?);
                let ramp = f(&a, "ease").unwrap_or(0.6);
                let zid = self.ai_edit("Add zoom", |p| {
                    let zid = p.alloc_id();
                    let c = p.clip_mut(clip)?;
                    let at = (start - c.start).clamp(0.0, c.duration);
                    c.zooms.push(Zoom { id: zid, at, duration: duration.min(c.duration - at).max(0.2), scale: scale.clamp(1.0, 6.0), focus_x: fx.clamp(0.0, 1.0), focus_y: fy.clamp(0.0, 1.0), ramp });
                    Ok(zid)
                })?;
                Ok(format!("Added zoom {zid}"))
            })(),
            "update_zoom" => (|| {
                let (clip, zid) = (req_id(&a, "clip_id")?, req_id(&a, "zoom_id")?);
                self.ai_edit("Update zoom", |p| {
                    let c = p.clip_mut(clip)?;
                    let start = c.start;
                    let z = c.zooms.iter_mut().find(|z| z.id == zid).context("Unknown zoom")?;
                    if let Some(v) = f(&a, "start") {
                        z.at = (v - start).max(0.0);
                    }
                    if let Some(v) = f(&a, "duration") {
                        z.duration = v.max(0.2);
                    }
                    if let Some(v) = f(&a, "scale") {
                        z.scale = v.clamp(1.0, 6.0);
                    }
                    if let Some(v) = f(&a, "focus_x") {
                        z.focus_x = v.clamp(0.0, 1.0);
                    }
                    if let Some(v) = f(&a, "focus_y") {
                        z.focus_y = v.clamp(0.0, 1.0);
                    }
                    if let Some(v) = f(&a, "ease") {
                        z.ramp = v.max(0.0);
                    }
                    Ok(())
                })?;
                Ok("Updated zoom".into())
            })(),
            "remove_zoom" => (|| {
                let (clip, zid) = (req_id(&a, "clip_id")?, req_id(&a, "zoom_id")?);
                self.ai_edit("Remove zoom", |p| {
                    p.clip_mut(clip)?.zooms.retain(|z| z.id != zid);
                    Ok(())
                })?;
                Ok("Removed zoom".into())
            })(),
            "set_keyframes" => (|| {
                let clip = req_id(&a, "clip_id")?;
                let pr = prop(s(&a, "property").unwrap_or(""))?;
                let keys = a["keys"].as_array().context("`keys` must be a list")?.clone();
                self.ai_edit("Animate", |p| {
                    let c = p.clip_mut(clip)?;
                    let mut ks: Vec<Keyframe> = keys
                        .iter()
                        .map(|k| Ok(Keyframe { t: req_f(k, "time")? - c.start, value: req_f(k, "value")?, ease: ease(s(k, "ease")) }))
                        .collect::<anyhow::Result<_>>()?;
                    ks.sort_by(|x, y| x.t.total_cmp(&y.t));
                    c.keyframes.insert(pr, ks);
                    Ok(())
                })?;
                Ok("Keyframes set".into())
            })(),
            "clear_keyframes" => (|| {
                let clip = req_id(&a, "clip_id")?;
                let pr = s(&a, "property").map(prop).transpose()?;
                self.ai_edit("Clear animation", |p| {
                    let c = p.clip_mut(clip)?;
                    match pr {
                        Some(pr) => {
                            c.keyframes.remove(&pr);
                        }
                        None => c.keyframes.clear(),
                    }
                    Ok(())
                })?;
                Ok("Keyframes cleared".into())
            })(),
            "add_sound" => {
                let sound = s(&a, "sound").unwrap_or("").to_string();
                match crate::sfx::SOUNDS.iter().find(|x| x.id == sound) {
                    None => Err(anyhow!("Unknown sound `{sound}`")),
                    Some(def) => {
                        let id: &'static str = def.id;
                        return Task::perform(
                            async move {
                                tokio::task::spawn_blocking(move || {
                                    let path = crate::sfx::path(id).map_err(|e| e.to_string())?;
                                    let info = crate::media::probe(&path).map_err(|e| e.to_string())?;
                                    Ok((path, info))
                                })
                                .await
                                .unwrap_or_else(|e| Err(e.to_string()))
                            },
                            move |r| Message::ToolSound(call.clone(), id, r),
                        );
                    }
                }
            }
            "add_track" => (|| {
                let kind = if s(&a, "kind") == Some("audio") { TrackKind::Audio } else { TrackKind::Visual };
                let name = s(&a, "name").unwrap_or(if kind == TrackKind::Audio { "Audio" } else { "Video" }).to_string();
                let t = self.ai_edit("Add track", |p| Ok(p.add_track(kind, &name, None)))?;
                Ok(format!("Added track {t}"))
            })(),
            "rename_track" => (|| {
                let (t, name) = (req_id(&a, "track_id")?, s(&a, "name").unwrap_or("").to_string());
                self.ai_edit("Rename track", |p| p.rename_track(t, &name))?;
                Ok("Renamed".into())
            })(),
            "set_canvas" => (|| {
                let mut canvas = self.doc.project.canvas;
                if let Some(v) = s(&a, "aspect") {
                    canvas.aspect = Aspect::ALL.into_iter().find(|x| x.label() == v).context("Unknown aspect")?;
                }
                if let Some(v) = s(&a, "resolution") {
                    canvas.resolution = Resolution::ALL.into_iter().find(|x| x.label().eq_ignore_ascii_case(v)).context("Unknown resolution")?;
                }
                if let Some(v) = s(&a, "fps").and_then(|v| v.parse().ok()).or_else(|| a.get("fps").and_then(Value::as_u64).map(|v| v as u32)) {
                    canvas.fps = v;
                }
                self.ai_edit("Change format", |p| {
                    p.canvas = canvas;
                    Ok(())
                })?;
                Ok(format!("Canvas is now {}", canvas.summary()))
            })(),
            "get_frame" => {
                let t = f(&a, "time").unwrap_or(self.playhead).max(0.0);
                self.seek(t);
                let project = self.doc.project.clone();
                std::thread::spawn(move || {
                    let r = crate::render::export::snapshot_jpeg(&project, t, 960).map(|jpeg| mcp::image_jpeg(&jpeg, &format!("Frame at {t:.2}s"))).map_err(|e| e.to_string());
                    call.answer(r);
                });
                return Task::none();
            }
            "seek" => req_f(&a, "time").map(|t| {
                self.seek(t);
                format!("Playhead at {t:.2}s")
            }),
            "play" => {
                if self.playing.is_none() {
                    self.start_playback();
                }
                Ok("Playing".into())
            }
            "pause" => {
                self.stop_playback();
                Ok("Paused".into())
            }
            "undo" => self.doc.undo().map(|d| {
                self.after_history();
                format!("Undid: {d}")
            }).context("Nothing to undo"),
            "redo" => self.doc.redo().map(|d| {
                self.after_history();
                format!("Redid: {d}")
            }).context("Nothing to redo"),
            "export_video" => (|| {
                let path = s(&a, "path").context("`path` is required")?;
                anyhow::ensure!(std::path::Path::new(path).is_absolute(), "Use an absolute path");
                let mut dialog = super::export_dialog::ExportDialog::new(&self.doc.project, self.path.as_deref());
                dialog.path = path.to_string();
                dialog.quality = match s(&a, "quality") {
                    Some("standard") => crate::render::export::Quality::Standard,
                    Some("small") => crate::render::export::Quality::Small,
                    _ => crate::render::export::Quality::High,
                };
                anyhow::ensure!(self.export.as_ref().is_none_or(|e| !e.is_running()), "An export is already running");
                self.export = Some(dialog);
                Ok("Export started — check get_export_status".into())
            })()
            .map(|msg: String| {
                call.answer(Ok(mcp::text(&msg)));
                String::new()
            }),
            "get_export_status" => Ok(match self.export.as_ref().map(|e| &e.phase) {
                Some(super::export_dialog::Phase::Running { done, total, .. }) => format!("Rendering: {done}/{total} frames"),
                Some(super::export_dialog::Phase::Done { path, .. }) => format!("Done: {}", path.display()),
                Some(super::export_dialog::Phase::Failed(e)) => format!("Failed: {e}"),
                _ => "No export running".into(),
            }),
            other => Err(anyhow!("Unknown tool `{other}`")),
        };
        if call.name == "export_video" {
            return match result {
                Ok(_) => self.start_export(),
                Err(e) => {
                    call.answer(Err(e.to_string()));
                    Task::none()
                }
            };
        }
        call.answer(result.map(mcp::text).map_err(|e| e.to_string()));
        if self.side_tab != SideTab::Assistant && call.name != "get_project" {
            self.lanes_cache.clear();
        }
        Task::none()
    }

    /// Finishes `import_media` once files are probed.
    pub fn finish_tool_import(&mut self, call: Call, results: Vec<(PathBuf, Result<crate::media::ProbeInfo, String>)>, add: bool) -> Task<Message> {
        let at = self.playhead;
        let mut report = Vec::new();
        let mut ids = Vec::new();
        for (path, r) in results {
            match r {
                Ok(info) => match self.ai_edit("Import media", |p| {
                    let id = p.add_asset(&path, &info);
                    let clip = if add { Some(p.add_media_clip(id, None, at)?) } else { None };
                    Ok((id, clip))
                }) {
                    Ok((id, clip)) => {
                        ids.push(id);
                        report.push(format!("{} → media {id}{}", path.display(), clip.map(|c| format!(", clip {c}")).unwrap_or_default()));
                    }
                    Err(e) => report.push(format!("{}: {e}", path.display())),
                },
                Err(e) => report.push(format!("{}: {e}", path.display())),
            }
        }
        call.answer(Ok(mcp::text(report.join("\n"))));
        let assets = self.doc.project.assets.clone();
        self.media.ensure(&assets, Message::MediaLoaded)
    }

    /// Finishes `add_sound` once the sound file exists.
    pub fn finish_tool_sound(&mut self, call: Call, sound: &'static str, r: Result<(PathBuf, crate::media::ProbeInfo), String>) -> Task<Message> {
        let (volume, start) = (f(&call.args, "volume"), f(&call.args, "start").unwrap_or(self.playhead));
        let name = crate::sfx::SOUNDS.iter().find(|s| s.id == sound).map(|s| s.name).unwrap_or("Sound");
        let result = r.map_err(|e| anyhow!(e)).and_then(|(path, info)| {
            self.ai_edit(&format!("Add {name}"), |p| {
                let asset = p.add_asset(&path, &info);
                if let Some(a) = p.assets.iter_mut().find(|a| a.id == asset) {
                    a.name = name.to_string();
                }
                let clip = p.add_media_clip(asset, None, start)?;
                if let Some(v) = volume {
                    p.clip_mut(clip)?.audio.volume = v.clamp(0.0, 2.0);
                }
                Ok(clip)
            })
        });
        call.answer(result.map(|c| mcp::text(format!("Added {name} as clip {c}"))).map_err(|e| e.to_string()));
        let assets = self.doc.project.assets.clone();
        self.media.ensure(&assets, Message::MediaLoaded)
    }
}

fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}
