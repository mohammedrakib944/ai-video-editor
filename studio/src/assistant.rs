//! The built-in assistant: runs the user's own Claude Code headless, connected
//! to this editor's MCP server, and streams its replies and tool calls back.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::{Arc, Mutex};

use anyhow::{Context, anyhow};
use serde_json::{Value, json};

use crate::mcp::{self, SERVER_KEY};

#[derive(Debug, Clone)]
pub enum Event {
    TextDelta(String),
    Text(String),
    ToolStart { id: String, name: String, input: Value },
    ToolDone { id: String, ok: bool, summary: String, image: Option<Vec<u8>> },
    TurnDone { error: Option<String> },
}

const SYSTEM_PROMPT: &str = "You are the motion designer built into AI Video Editor, a motion graphics editor for product \
showcases: Apple-style product launches and SaaS explainers built from screen recordings. The user watches every change \
live in the preview and can undo anything, so act directly with the ai-video-editor tools instead of describing what you \
would do. Call get_project first when you need ids, tracks or times.

Each user message starts with an <editor> line: the playhead, the selected clip and, if set, the selected time range. When a \
range is selected, the user means that part of the timeline: put new elements and changes inside it.

Design like a premium product film:
- Typography: few words, big and bold. Inter or Poppins (600–700) for English, Hind Siliguri (600) for Bangla. Headlines \
around 90–140 units, subtitles 40–60. Keep text off the edges (x 0.1–0.9, y 0.1–0.9) and centered unless asked.
- Motion: give titles an entrance (slide_up or fade, 0.4–0.7 s) and an exit; pop for callouts and badges; crossfade or \
zoom_through between scenes. Use set_keyframes for slow drifts and scale-ups on hero shots.
- Screen recordings: use add_zoom to guide the eye to the UI being discussed (scale 1.6–2.5, focus on that element), with \
back-to-back zooms to pan between parts. Leave breathing room between zooms.
- Frames: rounded corners (24–48), a soft shadow and a thin border make face-cam and screenshot overlays look polished.
- Sound: a whoosh on transitions, a pop or click on callouts, a riser before a big reveal; keep effects subtle (volume 0.5–0.8).
- Colors: match the brand if the user gives one; otherwise white text on dark or the accent #7C6DFF.
After a few changes, use get_frame to check your work visually and fix anything that looks off (overlaps, text too small or \
cut off). Keep replies short and friendly: one or two sentences about what you changed.";

/// Locates the Claude Code executable.
pub fn find_claude() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) { &["claude.exe", "claude.cmd"] } else { &["claude"] };
    let home = crate::settings::home_dir();
    let mut candidates = vec![home.join(".local").join("bin").join(names[0])];
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            candidates.extend(names.iter().map(|n| dir.join(n)));
        }
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        candidates.push(PathBuf::from(appdata).join("npm").join("claude.cmd"));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// A friendly line for a tool call, shown in the chat.
pub fn describe_tool(name: &str, input: &Value) -> String {
    let short = name.strip_prefix(&format!("mcp__{SERVER_KEY}__")).unwrap_or(name);
    let t = |k: &str| input.get(k).and_then(Value::as_f64).map(|v| format!("{v:.1}s")).unwrap_or_default();
    let q = |k: &str| input.get(k).and_then(Value::as_str).map(|s| s.chars().take(36).collect::<String>().replace('\n', " ")).unwrap_or_default();
    match short {
        "get_project" => "Looking at the project".into(),
        "import_media" => "Importing media".into(),
        "add_media_clip" => format!("Placing media at {}", t("start")),
        "add_text" => format!("Adding “{}”", q("text")),
        "add_shape" => "Adding a shape".into(),
        "update_clip" => "Adjusting a clip".into(),
        "move_clip" => format!("Moving a clip to {}", t("start")),
        "trim_clip" => "Trimming a clip".into(),
        "split_clip" => format!("Splitting at {}", t("time")),
        "delete_clips" => "Removing clips".into(),
        "duplicate_clip" => "Duplicating a clip".into(),
        "add_zoom" => format!("Zooming in at {}", t("start")),
        "update_zoom" => "Adjusting a zoom".into(),
        "remove_zoom" => "Removing a zoom".into(),
        "set_keyframes" => format!("Animating {}", q("property")),
        "clear_keyframes" => "Clearing animation".into(),
        "add_sound" => format!("Adding a {} sound", q("sound")),
        "add_track" => "Adding a track".into(),
        "rename_track" => "Renaming a track".into(),
        "set_canvas" => "Changing the format".into(),
        "get_frame" => format!("Checking the frame at {}", t("time")),
        "seek" => format!("Moving the playhead to {}", t("time")),
        "play" => "Playing the preview".into(),
        "pause" => "Pausing".into(),
        "undo" => "Undoing".into(),
        "redo" => "Redoing".into(),
        "export_video" => "Exporting the video".into(),
        "get_export_status" => "Checking the export".into(),
        "Glob" => format!("Looking for files {}", q("pattern")),
        "Read" => "Reading a file".into(),
        "ToolSearch" => "Getting ready".into(),
        other => format!("Using {other}"),
    }
}

/// Checks that Claude Code runs, without using any tokens. Returns its version line.
pub fn claude_version() -> Result<String, String> {
    let claude = find_claude().ok_or("Claude Code isn't installed. Install it from claude.com/claude-code and sign in once.")?;
    let mut cmd = if claude.extension().is_some_and(|e| e.eq_ignore_ascii_case("cmd")) {
        let mut c = std::process::Command::new("cmd");
        c.arg("/C").arg(&claude);
        c
    } else {
        std::process::Command::new(&claude)
    };
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.arg("--version").output().map_err(|e| format!("Couldn't run Claude Code: {e}"))?;
    let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || version.is_empty() {
        return Err("Claude Code didn't respond. Open a terminal, run `claude` once to sign in, then try again.".into());
    }
    Ok(version.split_whitespace().next().unwrap_or(&version).to_string())
}

struct Running {
    child: Child,
    stdin: ChildStdin,
}

pub struct Agent {
    claude: Option<PathBuf>,
    running: Option<Running>,
    session: Arc<Mutex<Option<String>>>,
    events: Arc<Mutex<Vec<Event>>>,
    pub busy: bool,
    /// The user connected an AI service in settings.
    pub enabled: bool,
}

impl Drop for Agent {
    fn drop(&mut self) {
        if let Some(mut r) = self.running.take() {
            let _ = r.child.kill();
        }
    }
}

impl Default for Agent {
    fn default() -> Self {
        Agent { claude: find_claude(), running: None, session: Arc::default(), events: Arc::default(), busy: false, enabled: false }
    }
}

impl Agent {
    /// Connected by the user and installed.
    pub fn available(&self) -> bool {
        self.enabled && self.claude.is_some()
    }

    pub fn recheck(&mut self) {
        self.claude = find_claude();
    }

    fn spawn(&mut self, cwd: &Path) -> anyhow::Result<()> {
        let claude = self.claude.clone().ok_or_else(|| anyhow!("Claude Code is not installed"))?;
        let dir = crate::ffmpeg::work_dir();
        let cfg = dir.join(format!("assistant-mcp-{}.json", std::process::id()));
        std::fs::write(&cfg, serde_json::to_string_pretty(&json!({ "mcpServers": { SERVER_KEY: { "type": "http", "url": mcp::url() } } }))?)?;
        let prompt = dir.join(format!("assistant-prompt-{}.txt", std::process::id()));
        let os = if cfg!(windows) { "Windows" } else { "Linux" };
        std::fs::write(&prompt, format!("{SYSTEM_PROMPT}\nThe editor runs on {os}; file paths are {os} paths."))?;
        let mut args: Vec<String> = [
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--strict-mcp-config",
            // Only file search and reading; all editing goes through the editor's tools.
            "--tools",
            "Glob,Read",
            "--permission-mode",
            "dontAsk",
            "--setting-sources",
            "",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        args.extend(["--mcp-config".into(), cfg.display().to_string()]);
        args.extend(["--allowedTools".into(), format!("mcp__{SERVER_KEY},Glob,Read")]);
        args.extend(["--append-system-prompt-file".into(), prompt.display().to_string()]);
        if let Some(id) = self.session.lock().unwrap().clone() {
            args.extend(["--resume".into(), id]);
        }
        let is_cmd = claude.extension().is_some_and(|e| e.eq_ignore_ascii_case("cmd"));
        let mut cmd = if is_cmd {
            let mut c = std::process::Command::new("cmd");
            c.arg("/C").arg(&claude);
            c
        } else {
            std::process::Command::new(&claude)
        };
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        cmd.args(&args).current_dir(cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn().context("could not start Claude Code")?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let stderr_buf: Arc<Mutex<String>> = Arc::default();
        let buf = stderr_buf.clone();
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = stderr.read_to_string(&mut s);
            buf.lock().unwrap().push_str(&s);
        });
        let (events, session) = (self.events.clone(), self.session.clone());
        std::thread::Builder::new().name("assistant-reader".into()).spawn(move || {
            let mut parser = StreamParser::default();
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                if let Some(id) = v["session_id"].as_str() {
                    *session.lock().unwrap() = Some(id.to_string());
                }
                events.lock().unwrap().extend(parser.handle(&v));
            }
            if parser.in_turn {
                std::thread::sleep(std::time::Duration::from_millis(100));
                let err = stderr_buf.lock().unwrap().trim().to_string();
                let lines: Vec<&str> = err.lines().filter(|l| !l.trim().is_empty()).collect();
                let msg = if lines.is_empty() { "Claude Code stopped unexpectedly.".to_string() } else { lines[lines.len().saturating_sub(6)..].join("\n") };
                events.lock().unwrap().push(Event::TurnDone { error: Some(msg) });
            }
        })?;
        self.running = Some(Running { child, stdin });
        Ok(())
    }

    /// Sends a message. `context` describes the editor state in one line.
    pub fn send(&mut self, text: &str, context: &str, cwd: &Path) -> anyhow::Result<()> {
        anyhow::ensure!(!self.busy, "The assistant is still working on the last message");
        let alive = self.running.as_mut().is_some_and(|r| matches!(r.child.try_wait(), Ok(None)));
        if !alive {
            self.running = None;
            self.spawn(cwd)?;
        }
        let content = format!("<editor>{context}</editor>\n{text}");
        let msg = json!({ "type": "user", "message": { "role": "user", "content": content } });
        let r = self.running.as_mut().unwrap();
        if let Err(e) = writeln!(r.stdin, "{msg}").and_then(|_| r.stdin.flush()) {
            self.running = None;
            return Err(anyhow!("could not talk to Claude Code: {e}"));
        }
        self.busy = true;
        Ok(())
    }

    /// Stops the current reply; the conversation continues with the next message.
    pub fn stop(&mut self) {
        if let Some(mut r) = self.running.take() {
            let _ = r.child.kill();
            let _ = r.child.wait();
        }
        if self.busy {
            self.busy = false;
            self.events.lock().unwrap().push(Event::TurnDone { error: Some("Stopped.".into()) });
        }
    }

    /// Starts a fresh conversation.
    pub fn reset(&mut self) {
        self.stop();
        *self.session.lock().unwrap() = None;
        self.events.lock().unwrap().clear();
    }

    pub fn poll(&mut self) -> Vec<Event> {
        let events: Vec<Event> = std::mem::take(&mut *self.events.lock().unwrap());
        if events.iter().any(|e| matches!(e, Event::TurnDone { .. })) {
            self.busy = false;
        }
        events
    }
}

/// Turns Claude Code's stream-json lines into chat events.
#[derive(Default)]
pub struct StreamParser {
    streamed: std::collections::HashSet<String>,
    current_msg: Option<String>,
    pub in_turn: bool,
}

impl StreamParser {
    pub fn handle(&mut self, v: &Value) -> Vec<Event> {
        let mut out = Vec::new();
        match v["type"].as_str().unwrap_or("") {
            "system" if v["subtype"] == "init" || v["subtype"] == "status" => self.in_turn = true,
            "stream_event" => {
                let ev = &v["event"];
                match ev["type"].as_str().unwrap_or("") {
                    "message_start" => self.current_msg = ev["message"]["id"].as_str().map(str::to_string),
                    "content_block_delta" if ev["delta"]["type"] == "text_delta" => {
                        if let Some(id) = &self.current_msg {
                            self.streamed.insert(id.clone());
                        }
                        if let Some(t) = ev["delta"]["text"].as_str() {
                            out.push(Event::TextDelta(t.to_string()));
                        }
                    }
                    _ => {}
                }
            }
            "assistant" => {
                let msg_id = v["message"]["id"].as_str().unwrap_or("").to_string();
                for block in v["message"]["content"].as_array().into_iter().flatten() {
                    match block["type"].as_str() {
                        Some("text") if !self.streamed.contains(&msg_id) => {
                            if let Some(t) = block["text"].as_str() {
                                out.push(Event::Text(t.to_string()));
                            }
                        }
                        Some("tool_use") => out.push(Event::ToolStart {
                            id: block["id"].as_str().unwrap_or("").to_string(),
                            name: block["name"].as_str().unwrap_or("").to_string(),
                            input: block["input"].clone(),
                        }),
                        _ => {}
                    }
                }
            }
            "user" => {
                for block in v["message"]["content"].as_array().into_iter().flatten().filter(|b| b["type"] == "tool_result") {
                    let id = block["tool_use_id"].as_str().unwrap_or("").to_string();
                    let ok = !block["is_error"].as_bool().unwrap_or(false);
                    let (mut summary, mut image) = (String::new(), None);
                    match &block["content"] {
                        Value::String(s) => summary = s.clone(),
                        Value::Array(items) => {
                            for it in items {
                                match it["type"].as_str() {
                                    Some("text") if summary.is_empty() => summary = it["text"].as_str().unwrap_or("").to_string(),
                                    Some("image") => {
                                        use base64::Engine;
                                        image = it["source"]["data"].as_str().and_then(|d| base64::engine::general_purpose::STANDARD.decode(d).ok());
                                    }
                                    _ => {}
                                }
                            }
                        }
                        _ => {}
                    }
                    out.push(Event::ToolDone { id, ok, summary, image });
                }
            }
            "result" => {
                self.in_turn = false;
                let failed = v["is_error"].as_bool().unwrap_or(false) || v["subtype"].as_str().is_some_and(|s| s != "success");
                let error = failed.then(|| v["result"].as_str().map(str::to_string).unwrap_or_else(|| format!("Claude Code reported an error ({})", v["subtype"].as_str().unwrap_or("unknown"))));
                out.push(Event::TurnDone { error });
            }
            _ => {}
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_recorded_turn() {
        let lines = [
            r#"{"type":"system","subtype":"init","session_id":"s1"}"#,
            r#"{"type":"stream_event","event":{"type":"message_start","message":{"id":"m1"}}}"#,
            r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"t1","name":"mcp__ai-video-editor__add_zoom","input":{"start":3}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":"Added zoom 7"}]}]}}"#,
            r#"{"type":"stream_event","event":{"type":"message_start","message":{"id":"m2"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Zoomed in"}}}"#,
            r#"{"type":"assistant","message":{"id":"m2","content":[{"type":"text","text":"Zoomed in"}]}}"#,
            r#"{"type":"result","subtype":"success","is_error":false,"result":"Zoomed in"}"#,
        ];
        let mut p = StreamParser::default();
        let events: Vec<Event> = lines.iter().flat_map(|l| p.handle(&serde_json::from_str(l).unwrap())).collect();
        assert!(!events.iter().any(|e| matches!(e, Event::Text(_))), "streamed text must not repeat");
        assert!(events.iter().any(|e| matches!(e, Event::ToolStart { name, .. } if name.ends_with("add_zoom"))));
        assert!(events.iter().any(|e| matches!(e, Event::ToolDone { ok: true, summary, .. } if summary == "Added zoom 7")));
        assert!(matches!(events.last(), Some(Event::TurnDone { error: None })));
        assert_eq!(describe_tool("mcp__ai-video-editor__add_zoom", &json!({ "start": 3.0 })), "Zooming in at 3.0s");
    }
}
