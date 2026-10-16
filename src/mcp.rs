//! MCP server (Streamable HTTP transport, JSON responses) on localhost.
//!
//! Claude Code connects straight to `http://127.0.0.1:<port>/mcp`. Clients that
//! only speak stdio launch this exe with `--mcp-stdio`, which forwards to the
//! running app (see `bridge`).

use std::path::PathBuf;

use anyhow::{anyhow, bail, Context};
use base64::Engine;
use serde_json::{json, Value};

use crate::media::MediaKind;
use crate::project::{fmt_time, Canvas, ExportSettings, ExportStatus, Fit, Project, Quality, Source, TextClip, Track};
use crate::render;
use crate::shared::{Placement, Shared};

pub const DEFAULT_PORT: u16 = 47821;
const SERVER_NAME: &str = "ai-video-editor";

/// Port external clients are configured with (the first editor window owns it).
pub fn port() -> u16 {
    std::env::var("AIVE_MCP_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_PORT)
}

/// Port this window actually listens on (differs from `port()` when another window owns it).
static BOUND_PORT: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(0);

pub fn bound_port() -> Option<u16> {
    match BOUND_PORT.load(std::sync::atomic::Ordering::Relaxed) {
        0 => None,
        p => Some(p),
    }
}

/// URL for external clients.
pub fn url() -> String {
    format!("http://127.0.0.1:{}/mcp", port())
}

/// URL of this window's own server (used by the built-in chat).
pub fn own_url() -> String {
    format!("http://127.0.0.1:{}/mcp", bound_port().unwrap_or_else(port))
}

const INSTRUCTIONS: &str = "You are editing a video in AI Video Editor, a desktop app the user is watching live. \
Every change you make appears instantly in their preview and can be undone by them.\n\
Model: the MAIN track is magnetic — clips play back to back with no gaps, and it defines the video length. \
The MUSIC track holds audio clips at absolute times. The TEXT track holds titles/captions at absolute times. \
All times are in seconds. Clip and media ids come from get_project.\n\
Workflow: call get_project first; use get_frame to look at the result; prefer delete_range for cutting \
sections out (it closes the gap and keeps captions in sync); set_canvas changes aspect ratio/resolution; \
export_video renders the final file.";

/// Starts the server. If another window owns the main port, uses the next free one so
/// this window's built-in chat still edits this window.
pub fn start(shared: Shared) -> anyhow::Result<()> {
    let base = port();
    let mut last_err = None;
    let mut server = None;
    for p in base..base.saturating_add(20) {
        match tiny_http::Server::http(format!("127.0.0.1:{p}")) {
            Ok(s) => {
                BOUND_PORT.store(p, std::sync::atomic::Ordering::Relaxed);
                server = Some(s);
                break;
            }
            Err(e) => last_err = Some(e),
        }
    }
    let server = server.ok_or_else(|| anyhow!("could not open a local port: {}", last_err.map(|e| e.to_string()).unwrap_or_default()))?;
    std::thread::Builder::new().name("mcp-http".into()).spawn(move || {
        for request in server.incoming_requests() {
            let shared = shared.clone();
            // Each request on its own thread so a slow frame render never blocks others.
            std::thread::spawn(move || handle_http(shared, request));
        }
    })?;
    Ok(())
}

fn header<'a>(req: &'a tiny_http::Request, name: &'static str) -> Option<&'a str> {
    req.headers().iter().find(|h| h.field.equiv(name)).map(|h| h.value.as_str())
}

fn respond(req: tiny_http::Request, status: u16, body: Option<String>) {
    let mut resp = tiny_http::Response::from_string(body.unwrap_or_default()).with_status_code(status);
    resp.add_header(tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap());
    let _ = req.respond(resp);
}

fn handle_http(shared: Shared, mut req: tiny_http::Request) {
    // Block DNS-rebinding attacks from web pages: only local origins may call us.
    if let Some(origin) = header(&req, "Origin") {
        let local = ["http://localhost", "http://127.0.0.1", "https://localhost", "https://127.0.0.1"];
        if !local.iter().any(|l| origin.starts_with(l)) {
            return respond(req, 403, Some(r#"{"error":"forbidden origin"}"#.into()));
        }
    }
    let path = req.url().split('?').next().unwrap_or("").to_string();
    if path != "/mcp" {
        return respond(req, 404, Some(r#"{"error":"not found"}"#.into()));
    }
    match req.method() {
        tiny_http::Method::Post => {}
        tiny_http::Method::Delete => return respond(req, 200, None),
        _ => return respond(req, 405, None),
    }
    let mut body = String::new();
    if req.as_reader().read_to_string(&mut body).is_err() {
        return respond(req, 400, None);
    }
    match handle_body(&shared, &body) {
        Some(out) => respond(req, 200, Some(out)),
        None => respond(req, 202, None),
    }
}

/// Handles one JSON-RPC message or batch. Returns `None` when there is nothing to send back.
pub fn handle_body(shared: &Shared, body: &str) -> Option<String> {
    let msg: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => return Some(json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":format!("parse error: {e}")}}).to_string()),
    };
    if let Value::Array(items) = msg {
        let out: Vec<Value> = items.into_iter().filter_map(|m| handle_message(shared, m)).collect();
        return (!out.is_empty()).then(|| Value::Array(out).to_string());
    }
    handle_message(shared, msg).map(|v| v.to_string())
}

fn handle_message(shared: &Shared, msg: Value) -> Option<Value> {
    let id = msg.get("id").cloned();
    let method = msg["method"].as_str().unwrap_or("");
    let id = id?; // notifications get no response
    let result = match method {
        "initialize" => {
            let version = msg["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18").to_string();
            Ok(json!({
                "protocolVersion": version,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": SERVER_NAME, "title": "AI Video Editor", "version": env!("CARGO_PKG_VERSION") },
                "instructions": INSTRUCTIONS,
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_list() })),
        "tools/call" => {
            let name = msg["params"]["name"].as_str().unwrap_or("").to_string();
            let args = msg["params"]["arguments"].clone();
            let args = if args.is_null() { json!({}) } else { args };
            shared.editor.lock().last_ai_call = Some(std::time::Instant::now());
            shared.repaint();
            Ok(match call_tool(shared, &name, &args) {
                Ok(content) => json!({ "content": content, "isError": false }),
                Err(e) => json!({ "content": [{ "type": "text", "text": format!("Error: {e:#}") }], "isError": true }),
            })
        }
        "resources/list" => Ok(json!({ "resources": [] })),
        "prompts/list" => Ok(json!({ "prompts": [] })),
        _ => Err(json!({ "code": -32601, "message": format!("method not found: {method}") })),
    };
    Some(match result {
        Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
        Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": e }),
    })
}

// ---------------------------------------------------------------------------
// Tool definitions
// ---------------------------------------------------------------------------

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": { "type": "object", "properties": properties, "required": required },
    })
}

fn text_props() -> Value {
    json!({
        "text": { "type": "string", "description": "Text to show. Use \\n for line breaks." },
        "start": { "type": "number", "description": "Timeline time in seconds when the text appears" },
        "duration": { "type": "number", "description": "How long it stays on screen, in seconds" },
        "position": { "type": "string", "enum": ["top", "center", "bottom"], "description": "Preset placement (default bottom)" },
        "x": { "type": "number", "description": "Horizontal center as a fraction of width, 0..1 (overrides position)" },
        "y": { "type": "number", "description": "Vertical center as a fraction of height, 0..1 (overrides position)" },
        "size": { "type": "string", "description": "small, medium, large, huge, or a number = fraction of the frame's shorter side (e.g. 0.06). Long lines shrink automatically to fit." },
        "color": { "type": "string", "description": "Hex color like #FFFFFF" },
        "background": { "type": "boolean", "description": "Dark box behind the text for readability (default true)" },
        "fade": { "type": "boolean", "description": "Fade the text in and out (default true)" },
    })
}

fn tool_list() -> Vec<Value> {
    vec![
        tool("get_project", "Get the full project: canvas (aspect/resolution/fps), media library, every clip on every track with timeline times, playhead and export status. Call this first.", json!({}), &[]),
        tool(
            "import_media",
            "Import video, audio or image files (absolute Windows paths) into the media library. Optionally append them to the timeline: videos/images go to the end of the main track, audio onto the music track at time 0.",
            json!({
                "paths": { "type": "array", "items": { "type": "string" }, "description": "Absolute file paths" },
                "add_to_timeline": { "type": "boolean", "description": "Also place them on the timeline (default false)" },
            }),
            &["paths"],
        ),
        tool(
            "add_clip",
            "Place media from the library on the timeline. Video/image -> main track (at `index`, default end). Audio -> music track at `start`.",
            json!({
                "media_id": { "type": "integer" },
                "index": { "type": "integer", "description": "Main-track position (0 = first). Default: end" },
                "start": { "type": "number", "description": "Music track only: timeline start time in seconds (default 0)" },
                "source_in": { "type": "number", "description": "Start point inside the media, seconds (default 0)" },
                "source_out": { "type": "number", "description": "End point inside the media, seconds (default: end of media)" },
                "duration": { "type": "number", "description": "Images only: how long to show it (default 5)" },
            }),
            &["media_id"],
        ),
        tool("split", "Split the main-track clip at a timeline time into two clips.", json!({ "time": { "type": "number" } }), &["time"]),
        tool(
            "delete_range",
            "Cut timeline time [start, end) out of the main track and close the gap (ripple delete). Text clips shift with the video; music stays put. Best tool for removing unwanted sections.",
            json!({ "start": { "type": "number" }, "end": { "type": "number" } }),
            &["start", "end"],
        ),
        tool("delete_clip", "Delete a clip from any track. Main-track deletes close the gap.", json!({ "clip_id": { "type": "integer" } }), &["clip_id"]),
        tool(
            "trim_clip",
            "Change which part of the source media a main or music clip uses (source in/out points, seconds).",
            json!({ "clip_id": { "type": "integer" }, "source_in": { "type": "number" }, "source_out": { "type": "number" } }),
            &["clip_id"],
        ),
        tool(
            "move_clip",
            "Reorder a main-track clip (`index`), or move a music/text clip to a new timeline `start`.",
            json!({ "clip_id": { "type": "integer" }, "index": { "type": "integer" }, "start": { "type": "number" } }),
            &["clip_id"],
        ),
        tool(
            "set_clip",
            "Change clip properties. Main (video) clips: volume, fit, speed, fade_in, fade_out, brightness, contrast, saturation. Music clips: volume, fade_in, fade_out. Only the given fields change.",
            json!({
                "clip_id": { "type": "integer" },
                "volume": { "type": "number", "description": "0..2, 1 = original, 0 = mute" },
                "fit": { "type": "string", "enum": ["fit", "fill"], "description": "'fit' shows the whole picture with bars, 'fill' crops to fill the frame" },
                "speed": { "type": "number", "description": "0.25..4, 1 = normal. Changes the clip's length on the timeline" },
                "fade_in": { "type": "number", "description": "Seconds of fade from black/silence at the start" },
                "fade_out": { "type": "number", "description": "Seconds of fade to black/silence at the end" },
                "brightness": { "type": "number", "description": "-1..1, 0 = unchanged" },
                "contrast": { "type": "number", "description": "0..2, 1 = unchanged" },
                "saturation": { "type": "number", "description": "0..2, 1 = unchanged, 0 = black and white" },
            }),
            &["clip_id"],
        ),
        tool("duplicate_clip", "Copy a clip (any track) and place the copy right after it.", json!({ "clip_id": { "type": "integer" } }), &["clip_id"]),
        tool("add_text", "Add a title or caption on the text track.", text_props(), &["text", "start", "duration"]),
        tool(
            "update_text",
            "Change an existing text clip. Only the given fields change.",
            {
                let mut p = text_props();
                p["clip_id"] = json!({ "type": "integer" });
                p
            },
            &["clip_id"],
        ),
        tool(
            "set_canvas",
            "Change the project's aspect ratio, resolution and/or frame rate. Clips re-fit automatically.",
            json!({
                "aspect": { "type": "string", "description": "e.g. 16:9, 9:16, 1:1, 4:5, 21:9" },
                "resolution": { "type": "string", "description": "720p, 1080p, 1440p or 4K (short side)" },
                "fps": { "type": "integer", "description": "24, 25, 30 or 60" },
            }),
            &[],
        ),
        tool(
            "get_frame",
            "Render the edited video at a timeline time and return it as an image, exactly as it will export. Use it to check your edits visually.",
            json!({ "time": { "type": "number", "description": "Seconds (default: current playhead)" } }),
            &[],
        ),
        tool("seek", "Move the user's playhead to a time so they can see it in the preview.", json!({ "time": { "type": "number" } }), &["time"]),
        tool(
            "play",
            "Start playback in the user's preview (with sound), optionally from a time. Use it to show the user your edit.",
            json!({ "from": { "type": "number", "description": "Seconds (default: current playhead)" } }),
            &[],
        ),
        tool("pause", "Pause playback in the user's preview.", json!({}), &[]),
        tool(
            "export_video",
            "Start rendering the final MP4 in the background. Aspect/resolution/fps default to the project's. Poll get_export_status until done.",
            json!({
                "path": { "type": "string", "description": "Absolute output path ending in .mp4" },
                "aspect": { "type": "string" },
                "resolution": { "type": "string" },
                "fps": { "type": "integer" },
                "quality": { "type": "string", "enum": ["high", "standard", "small"] },
            }),
            &["path"],
        ),
        tool("get_export_status", "Progress of the current or last export.", json!({}), &[]),
        tool("cancel_export", "Cancel the running export.", json!({}), &[]),
        tool("undo", "Undo the last change (yours or the user's).", json!({}), &[]),
        tool("redo", "Redo the last undone change.", json!({}), &[]),
    ]
}

// ---------------------------------------------------------------------------
// Tool implementations
// ---------------------------------------------------------------------------

fn text_content(s: impl Into<String>) -> Vec<Value> {
    vec![json!({ "type": "text", "text": s.into() })]
}

fn f(args: &Value, key: &str) -> Option<f64> {
    args.get(key).and_then(Value::as_f64)
}

fn req_f(args: &Value, key: &str) -> anyhow::Result<f64> {
    f(args, key).ok_or_else(|| anyhow!("missing number `{key}`"))
}

fn req_id(args: &Value, key: &str) -> anyhow::Result<u64> {
    args.get(key).and_then(Value::as_u64).ok_or_else(|| anyhow!("missing integer `{key}`"))
}

fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

pub fn project_json(p: &Project, playhead: f64, export: &ExportStatus) -> Value {
    let starts = p.main_starts();
    json!({
        "name": p.name,
        "canvas": { "width": p.canvas.width, "height": p.canvas.height, "fps": p.canvas.fps, "aspect": p.canvas.aspect_label(), "resolution": p.canvas.resolution_label() },
        "duration": r3(p.duration()),
        "playhead": r3(playhead),
        "media": p.assets.iter().map(|a| json!({
            "media_id": a.id, "name": a.name, "kind": a.kind, "path": a.path,
            "duration": r3(a.duration), "width": a.width, "height": a.height, "has_audio": a.has_audio,
        })).collect::<Vec<_>>(),
        "main_track": p.main.iter().enumerate().map(|(i, c)| {
            let a = p.asset(c.asset_id);
            json!({
                "clip_id": c.id, "index": i, "media_id": c.asset_id, "name": a.map(|a| a.name.clone()),
                "start": r3(starts[i]), "end": r3(starts[i] + c.duration()),
                "source_in": r3(c.src_in), "source_out": r3(c.src_out), "volume": c.volume, "fit": c.fit,
            })
        }).collect::<Vec<_>>(),
        "music_track": p.music.iter().map(|c| json!({
            "clip_id": c.id, "media_id": c.asset_id, "name": p.asset(c.asset_id).map(|a| a.name.clone()),
            "start": r3(c.start), "end": r3(c.end()), "source_in": r3(c.src_in), "source_out": r3(c.src_out), "volume": c.volume,
        })).collect::<Vec<_>>(),
        "text_track": p.texts.iter().map(|t| json!({
            "clip_id": t.id, "text": t.text, "start": r3(t.start), "end": r3(t.end()),
            "x": t.x, "y": t.y, "size": t.size, "color": format!("#{:02X}{:02X}{:02X}", t.color[0], t.color[1], t.color[2]), "background": t.background,
        })).collect::<Vec<_>>(),
        "export": export_json(export),
    })
}

fn export_json(s: &ExportStatus) -> Value {
    match s {
        ExportStatus::Idle => json!({ "state": "idle" }),
        ExportStatus::Running { progress, eta_secs, path } => json!({ "state": "running", "progress_percent": (progress * 100.0).round(), "eta_seconds": eta_secs.map(|e| e.round()), "path": path }),
        ExportStatus::Done { path, secs } => json!({ "state": "done", "path": path, "took_seconds": secs.round() }),
        ExportStatus::Failed { error } => json!({ "state": "failed", "error": error }),
        ExportStatus::Cancelled => json!({ "state": "cancelled" }),
    }
}

/// Moves the user's playhead to where the AI just edited, so they see the change.
/// Does nothing while the preview is playing.
fn show_time(ed: &mut crate::project::Editor, t: f64) {
    if ed.playing {
        return;
    }
    let total = ed.project.duration();
    let t = t.clamp(0.0, (total - ed.project.canvas.frame()).max(0.0));
    ed.seek_request = Some(t);
    ed.playhead = t;
}

fn show_clip(ed: &mut crate::project::Editor, id: u64) {
    let p = &ed.project;
    let t = if let Some(i) = p.main_index(id) {
        Some(p.main_starts()[i])
    } else if let Some(m) = p.music.iter().find(|m| m.id == id) {
        Some(m.start)
    } else {
        p.texts.iter().find(|t| t.id == id).map(|t| t.start + (t.duration / 2.0).min(0.5))
    };
    if let Some(t) = t {
        show_time(ed, t);
    }
}

fn parse_color(s: &str) -> anyhow::Result<[u8; 3]> {
    let named = match s.to_ascii_lowercase().as_str() {
        "white" => Some([255, 255, 255]),
        "black" => Some([0, 0, 0]),
        "yellow" => Some([255, 221, 0]),
        "red" => Some([235, 64, 52]),
        "green" => Some([60, 200, 90]),
        "blue" => Some([60, 130, 246]),
        _ => None,
    };
    if let Some(c) = named {
        return Ok(c);
    }
    let h = s.trim_start_matches('#');
    if h.len() != 6 {
        bail!("color must be like #FFCC00");
    }
    let v = u32::from_str_radix(h, 16).map_err(|_| anyhow!("color must be like #FFCC00"))?;
    Ok([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

fn apply_text_args(t: &mut TextClip, args: &Value) -> anyhow::Result<()> {
    if let Some(s) = args.get("text").and_then(Value::as_str) {
        t.text = s.to_string();
    }
    if let Some(v) = f(args, "start") {
        t.start = v;
    }
    if let Some(v) = f(args, "duration") {
        t.duration = v;
    }
    if let Some(p) = args.get("position").and_then(Value::as_str) {
        t.x = 0.5;
        t.y = match p {
            "top" => 0.12,
            "center" | "middle" => 0.5,
            "bottom" => 0.85,
            _ => bail!("position must be top, center or bottom"),
        };
    }
    if let Some(v) = f(args, "x") {
        t.x = v.clamp(0.0, 1.0) as f32;
    }
    if let Some(v) = f(args, "y") {
        t.y = v.clamp(0.0, 1.0) as f32;
    }
    match args.get("size") {
        Some(Value::Number(n)) => t.size = n.as_f64().unwrap_or(0.06) as f32,
        Some(Value::String(s)) => {
            t.size = match s.as_str() {
                "small" => 0.04,
                "medium" => 0.06,
                "large" => 0.09,
                "huge" => 0.14,
                other => other.parse().map_err(|_| anyhow!("size must be small, medium, large, huge or a number"))?,
            }
        }
        _ => {}
    }
    if let Some(c) = args.get("color").and_then(Value::as_str) {
        t.color = parse_color(c)?;
    }
    if let Some(b) = args.get("background").and_then(Value::as_bool) {
        t.background = b;
    }
    if let Some(b) = args.get("fade").and_then(Value::as_bool) {
        t.fade = b;
    }
    Ok(())
}

fn canvas_from_args(base: Canvas, args: &Value) -> anyhow::Result<Canvas> {
    let (mut aw, mut ah) = (base.width, base.height);
    if let Some(a) = args.get("aspect").and_then(Value::as_str) {
        (aw, ah) = Canvas::parse_aspect(a)?;
    }
    let short = match args.get("resolution") {
        Some(Value::String(r)) => Canvas::parse_resolution(r)?,
        Some(Value::Number(n)) => Canvas::parse_resolution(&n.to_string())?,
        _ => base.width.min(base.height),
    };
    let fps = match args.get("fps").and_then(Value::as_u64) {
        Some(v) => v as u32,
        None => base.fps,
    };
    let c = Canvas::from_preset(aw, ah, short, fps);
    c.validate()?;
    Ok(c)
}

fn call_tool(shared: &Shared, name: &str, args: &Value) -> anyhow::Result<Vec<Value>> {
    let ai = Source::Ai;
    let done = |msg: String| -> anyhow::Result<Vec<Value>> {
        shared.repaint();
        Ok(text_content(msg))
    };
    match name {
        "get_project" => {
            let ed = shared.editor.lock();
            let v = project_json(&ed.project, ed.playhead, &ed.export_status);
            Ok(text_content(serde_json::to_string_pretty(&v)?))
        }
        "import_media" => {
            let paths: Vec<PathBuf> = args["paths"]
                .as_array()
                .ok_or_else(|| anyhow!("`paths` must be a list of file paths"))?
                .iter()
                .filter_map(|p| p.as_str().map(PathBuf::from))
                .collect();
            if paths.is_empty() {
                bail!("`paths` is empty");
            }
            let placement = if args["add_to_timeline"].as_bool().unwrap_or(false) {
                Placement::Timeline { main_index: None, music_start: 0.0 }
            } else {
                Placement::LibraryOnly
            };
            let report = shared.import(&paths, placement, ai);
            let mut out = json!({
                "imported": report.added.iter().map(|(id, n, k)| json!({ "media_id": id, "name": n, "kind": k })).collect::<Vec<_>>(),
                "clip_ids": report.clip_ids,
            });
            if !report.errors.is_empty() {
                out["errors"] = json!(report.errors);
            }
            if report.added.is_empty() {
                bail!("Nothing imported: {}", report.errors.join("; "));
            }
            Ok(text_content(out.to_string()))
        }
        "add_clip" => {
            let media_id = req_id(args, "media_id")?;
            let mut ed = shared.editor.lock();
            let asset = ed.project.asset(media_id).cloned().ok_or_else(|| anyhow!("No media with id {media_id}"))?;
            let src = match (f(args, "source_in"), f(args, "source_out"), f(args, "duration")) {
                (_, _, Some(d)) if asset.kind == MediaKind::Image => Some((0.0, d)),
                (None, None, _) => None,
                (a, b, _) => Some((a.unwrap_or(0.0), b.unwrap_or(asset.duration))),
            };
            let id = if asset.kind == MediaKind::Audio {
                let start = f(args, "start").unwrap_or(0.0);
                ed.apply(ai, &format!("Add {} to music", asset.name), |p| p.add_music(media_id, start, src))?
            } else {
                let index = args.get("index").and_then(Value::as_u64).map(|i| i as usize);
                ed.apply(ai, &format!("Add {}", asset.name), |p| p.insert_main(media_id, index, src))?
            };
            show_clip(&mut ed, id);
            drop(ed);
            done(format!("Added clip {id}"))
        }
        "split" => {
            let t = req_f(args, "time")?;
            let mut ed = shared.editor.lock();
            let id = ed.apply(ai, &format!("Split at {}", fmt_time(t)), |p| p.split_at(t))?;
            show_time(&mut ed, t);
            drop(ed);
            done(format!("Split done. New clip id {id} starts at {t:.3}s"))
        }
        "delete_range" => {
            let (a, b) = (req_f(args, "start")?, req_f(args, "end")?);
            if b <= a {
                bail!("end must be after start");
            }
            let mut ed = shared.editor.lock();
            ed.apply(ai, &format!("Cut {} – {}", fmt_time(a), fmt_time(b)), |p| p.delete_range(a, b))?;
            let total = ed.project.duration();
            show_time(&mut ed, a);
            drop(ed);
            done(format!("Removed {a:.3}s–{b:.3}s. New duration {total:.3}s"))
        }
        "delete_clip" => {
            let id = req_id(args, "clip_id")?;
            shared.editor.lock().apply(ai, "Delete clip", |p| p.delete_clip(id))?;
            done(format!("Deleted clip {id}"))
        }
        "trim_clip" => {
            let id = req_id(args, "clip_id")?;
            let mut ed = shared.editor.lock();
            match ed.project.track_of(id) {
                Some(Track::Main) => {
                    let c = ed.project.main[ed.project.main_index(id).unwrap()].clone();
                    let (a, b) = (f(args, "source_in").unwrap_or(c.src_in), f(args, "source_out").unwrap_or(c.src_out));
                    ed.apply(ai, "Trim clip", |p| p.trim_main(id, a, b))?;
                }
                Some(Track::Music) => {
                    let c = ed.project.music.iter().find(|c| c.id == id).unwrap().clone();
                    let (a, b) = (f(args, "source_in").unwrap_or(c.src_in), f(args, "source_out").unwrap_or(c.src_out));
                    ed.apply(ai, "Trim music", |p| p.update_music(id, None, Some((a, b)), None))?;
                }
                Some(Track::Text) => bail!("Use update_text with start/duration to change a text clip"),
                None => bail!("No clip with id {id}"),
            }
            drop(ed);
            done(format!("Trimmed clip {id}"))
        }
        "move_clip" => {
            let id = req_id(args, "clip_id")?;
            let mut ed = shared.editor.lock();
            match ed.project.track_of(id) {
                Some(Track::Main) => {
                    let index = args.get("index").and_then(Value::as_u64).ok_or_else(|| anyhow!("main-track clips move by `index`"))? as usize;
                    ed.apply(ai, "Move clip", |p| p.move_main(id, index))?;
                }
                Some(Track::Music) => {
                    let start = req_f(args, "start")?;
                    ed.apply(ai, "Move music", |p| p.update_music(id, Some(start), None, None))?;
                }
                Some(Track::Text) => {
                    let start = req_f(args, "start")?;
                    ed.apply(ai, "Move text", |p| {
                        p.text_mut(id)?.start = start.max(0.0);
                        Ok(())
                    })?;
                }
                None => bail!("No clip with id {id}"),
            }
            drop(ed);
            done(format!("Moved clip {id}"))
        }
        "set_clip" => {
            let id = req_id(args, "clip_id")?;
            let volume = f(args, "volume");
            let fit = match args.get("fit").and_then(Value::as_str) {
                Some("fit") => Some(Fit::Fit),
                Some("fill") => Some(Fit::Fill),
                Some(_) => bail!("fit must be 'fit' or 'fill'"),
                None => None,
            };
            let speed = f(args, "speed");
            let (fade_in, fade_out) = (f(args, "fade_in"), f(args, "fade_out"));
            let color = (f(args, "brightness").map(|v| v as f32), f(args, "contrast").map(|v| v as f32), f(args, "saturation").map(|v| v as f32));
            if volume.is_none() && fit.is_none() && speed.is_none() && fade_in.is_none() && fade_out.is_none() && color == (None, None, None) {
                bail!("give at least one property to change");
            }
            let mut ed = shared.editor.lock();
            ed.apply(ai, "Change clip", |p| {
                if let Some(v) = volume {
                    p.set_volume(id, v as f32)?;
                }
                if let Some(fit) = fit {
                    p.set_fit(id, fit)?;
                }
                if let Some(s) = speed {
                    p.set_speed(id, s)?;
                }
                if fade_in.is_some() || fade_out.is_some() {
                    p.set_fades(id, fade_in, fade_out)?;
                }
                if color != (None, None, None) {
                    p.set_color(id, color.0, color.1, color.2)?;
                }
                Ok(())
            })?;
            show_clip(&mut ed, id);
            drop(ed);
            done(format!("Updated clip {id}"))
        }
        "duplicate_clip" => {
            let id = req_id(args, "clip_id")?;
            let mut ed = shared.editor.lock();
            let new_id = ed.apply(ai, "Duplicate clip", |p| p.duplicate_clip(id))?;
            show_clip(&mut ed, new_id);
            drop(ed);
            done(format!("Duplicated clip {id}; the copy is clip {new_id}"))
        }
        "add_text" => {
            let mut t = TextClip::new("", 0.0, 3.0);
            apply_text_args(&mut t, args)?;
            let label = t.text.chars().take(24).collect::<String>();
            let mut ed = shared.editor.lock();
            let id = ed.apply(ai, &format!("Add text \"{label}\""), |p| p.add_text(t))?;
            show_clip(&mut ed, id);
            drop(ed);
            done(format!("Added text clip {id}"))
        }
        "update_text" => {
            let id = req_id(args, "clip_id")?;
            let mut ed = shared.editor.lock();
            ed.apply(ai, "Edit text", |p| {
                let t = p.text_mut(id)?;
                let mut copy = t.clone();
                apply_text_args(&mut copy, args)?;
                crate::project::validate_text(&copy)?;
                *t = copy;
                Ok(())
            })?;
            show_clip(&mut ed, id);
            drop(ed);
            done(format!("Updated text {id}"))
        }
        "set_canvas" => {
            let mut ed = shared.editor.lock();
            let canvas = canvas_from_args(ed.project.canvas, args)?;
            ed.apply(ai, &format!("Canvas {}", canvas.summary()), |p| p.set_canvas(canvas))?;
            drop(ed);
            done(format!("Canvas is now {}x{} ({})", canvas.width, canvas.height, canvas.summary()))
        }
        "get_frame" => {
            let (project, t) = {
                let ed = shared.editor.lock();
                (ed.project.clone(), f(args, "time").unwrap_or(ed.playhead))
            };
            let (w, h) = render::preview_size(&project.canvas, 768);
            let rgba = render::render_frame(&project, t, w, h)?;
            let jpeg = render::encode_jpeg(&rgba, w, h)?;
            Ok(vec![
                json!({ "type": "image", "data": base64::engine::general_purpose::STANDARD.encode(jpeg), "mimeType": "image/jpeg" }),
                json!({ "type": "text", "text": format!("Frame at {:.3}s ({}x{} preview of {}x{})", t, w, h, project.canvas.width, project.canvas.height) }),
            ])
        }
        "play" => {
            let mut ed = shared.editor.lock();
            let total = ed.project.duration();
            if total <= 0.0 {
                bail!("The timeline is empty");
            }
            if let Some(t) = f(args, "from") {
                let t = t.clamp(0.0, total);
                ed.seek_request = Some(t);
                ed.playhead = t;
            }
            ed.play_request = Some(true);
            let from = ed.playhead;
            drop(ed);
            done(format!("Playing from {from:.3}s (video is {total:.3}s long)"))
        }
        "pause" => {
            shared.editor.lock().play_request = Some(false);
            done("Paused".into())
        }
        "seek" => {
            let t = req_f(args, "time")?;
            let mut ed = shared.editor.lock();
            let t = t.clamp(0.0, ed.project.duration());
            ed.seek_request = Some(t);
            ed.playhead = t;
            drop(ed);
            done(format!("Playhead at {t:.3}s"))
        }
        "export_video" => {
            let path = PathBuf::from(args["path"].as_str().ok_or_else(|| anyhow!("missing `path`"))?);
            if !path.is_absolute() {
                bail!("`path` must be absolute, e.g. C:\\Users\\me\\Videos\\out.mp4");
            }
            let path = if path.extension().is_none() { path.with_extension("mp4") } else { path };
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            if ext != "mp4" && ext != "mov" {
                bail!("Only .mp4 and .mov output is supported");
            }
            let base = shared.editor.lock().project.canvas;
            let canvas = canvas_from_args(base, args)?;
            let quality = match args.get("quality").and_then(Value::as_str) {
                Some(q) => Quality::parse(q)?,
                None => Quality::Standard,
            };
            shared.start_export(ExportSettings { canvas, quality, path: path.clone() }, ai)?;
            done(format!("Export started: {} at {}x{} {}fps. Poll get_export_status.", path.display(), canvas.width, canvas.height, canvas.fps))
        }
        "get_export_status" => {
            let ed = shared.editor.lock();
            Ok(text_content(export_json(&ed.export_status).to_string()))
        }
        "cancel_export" => {
            shared.cancel_export();
            done("Cancelling export".into())
        }
        "undo" => {
            let r = shared.editor.lock().undo(ai);
            match r {
                Some(d) => done(format!("Undid: {d}")),
                None => bail!("Nothing to undo"),
            }
        }
        "redo" => {
            let r = shared.editor.lock().redo(ai);
            match r {
                Some(d) => done(format!("Redid: {d}")),
                None => bail!("Nothing to redo"),
            }
        }
        _ => bail!("Unknown tool `{name}`"),
    }
}

// ---------------------------------------------------------------------------
// stdio bridge for clients that cannot use HTTP
// ---------------------------------------------------------------------------

/// Forwards JSON-RPC lines from stdin to the running app and writes replies to stdout.
/// Starts the app if it is not running yet.
pub fn run_stdio_bridge() -> anyhow::Result<()> {
    use std::io::{BufRead, Write};
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let agent = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(600)).build();
    let mut launched = false;
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let mut attempt = 0;
        let reply = loop {
            match agent.post(&url()).set("Content-Type", "application/json").set("Accept", "application/json, text/event-stream").send_string(&line) {
                Ok(resp) if resp.status() == 202 => break None,
                Ok(resp) => break Some(resp.into_string().context("bad reply from app")?),
                Err(ureq::Error::Status(code, resp)) => break Some(resp.into_string().unwrap_or_else(|_| format!("{{\"error\":\"HTTP {code}\"}}"))),
                Err(_) if attempt < 60 => {
                    if !launched {
                        launched = true;
                        let exe = std::env::current_exe()?;
                        let _ = std::process::Command::new(exe).spawn();
                    }
                    attempt += 1;
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
                Err(e) => {
                    let id = serde_json::from_str::<Value>(&line).ok().and_then(|v| v.get("id").cloned());
                    break id.map(|id| json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32000, "message": format!("AI Video Editor is not running: {e}") } }).to_string());
                }
            }
        };
        if let Some(r) = reply {
            writeln!(stdout, "{}", r.trim())?;
            stdout.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Editor;
    use std::sync::Arc;

    fn shared() -> Shared {
        let mut ed = Editor::default();
        ed.has_project = true;
        Shared::new(ed, Arc::new(|| {}))
    }

    fn call(s: &Shared, method: &str, params: Value) -> Value {
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }).to_string();
        serde_json::from_str(&handle_body(s, &body).unwrap()).unwrap()
    }

    #[test]
    fn initialize_and_list_tools() {
        let s = shared();
        let r = call(&s, "initialize", json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "t", "version": "1" } }));
        assert_eq!(r["result"]["protocolVersion"], "2025-06-18");
        assert!(r["result"]["capabilities"]["tools"].is_object());
        let notif = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string();
        assert!(handle_body(&s, &notif).is_none());
        let r = call(&s, "tools/list", json!({}));
        let names: Vec<_> = r["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
        for n in ["get_project", "import_media", "delete_range", "get_frame", "export_video", "set_canvas", "duplicate_clip", "play"] {
            assert!(names.contains(&n.to_string()), "missing {n}");
        }
    }

    #[test]
    fn tool_errors_are_reported_not_thrown() {
        let s = shared();
        let r = call(&s, "tools/call", json!({ "name": "split", "arguments": { "time": 3.0 } }));
        assert_eq!(r["result"]["isError"], true);
        let r = call(&s, "tools/call", json!({ "name": "nope", "arguments": {} }));
        assert_eq!(r["result"]["isError"], true);
        let r = call(&s, "bogus/method", json!({}));
        assert_eq!(r["error"]["code"], -32601);
    }

    #[test]
    fn text_and_canvas_tools() {
        let s = shared();
        let r = call(&s, "tools/call", json!({ "name": "add_text", "arguments": { "text": "Hello", "start": 1, "duration": 2, "position": "top", "size": "large", "color": "#FFCC00" } }));
        assert_eq!(r["result"]["isError"], false, "{r}");
        let r = call(&s, "tools/call", json!({ "name": "set_canvas", "arguments": { "aspect": "9:16", "resolution": "720p" } }));
        assert_eq!(r["result"]["isError"], false, "{r}");
        let ed = s.editor.lock();
        assert_eq!((ed.project.canvas.width, ed.project.canvas.height), (720, 1280));
        assert_eq!(ed.project.texts[0].color, [255, 204, 0]);
        assert!((ed.project.texts[0].y - 0.12).abs() < 1e-6);
        assert!(ed.ai_touched.contains_key(&ed.project.texts[0].id));
    }
}
