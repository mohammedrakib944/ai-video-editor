//! In-app chat: runs the user's own Claude Code in headless mode, connected to
//! this editor's MCP server, and streams its replies and tool calls back.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::Arc;

use anyhow::{anyhow, Context};
use crossbeam_channel::{unbounded, Receiver, Sender};
use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::ffmpeg;
use crate::integrations::SERVER_KEY;
use crate::mcp;

/// What the UI hears from the assistant.
#[derive(Debug, Clone)]
pub enum ChatEvent {
    /// A piece of streamed reply text.
    TextDelta(String),
    /// A complete text block (used only when it was not streamed).
    Text(String),
    /// The assistant started using a tool.
    ToolStart { id: String, name: String, input: Value },
    /// A tool finished. `image` holds JPEG bytes when the tool returned a frame.
    ToolDone { id: String, ok: bool, summary: String, image: Option<Vec<u8>> },
    /// The reply is complete.
    TurnDone { error: Option<String> },
}

const SYSTEM_PROMPT: &str = "You are the editing assistant built into AI Video Editor, a Windows desktop video editor. \
The user is watching the editor while you work: every change you make with the ai-video-editor tools shows up live in \
their preview and timeline, and they can undo anything. Act on requests directly with the tools instead of describing \
what you would do. Call get_project first when you need clip ids or times. Use get_frame to check your work visually \
when it matters. Use Glob to find media files when the user mentions a folder. Paths are Windows paths. \
Each user message may start with an <editor> line describing the playhead and the selected clip; use it to resolve \
words like here, this clip or now. Keep replies short and friendly: one or two sentences saying what you changed.";

/// Locates the Claude Code executable.
pub fn find_claude() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default();
    let mut candidates = vec![home.join(".local").join("bin").join("claude.exe")];
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            candidates.push(dir.join("claude.exe"));
            candidates.push(dir.join("claude.cmd"));
        }
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        candidates.push(PathBuf::from(appdata).join("npm").join("claude.cmd"));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Friendly one-line description of a tool call, for the chat timeline.
pub fn describe_tool(name: &str, input: &Value) -> String {
    let short = name.strip_prefix(&format!("mcp__{SERVER_KEY}__")).unwrap_or(name);
    let t = |k: &str| input.get(k).and_then(Value::as_f64).map(crate::project::fmt_time);
    match short {
        "get_project" => "Reading the project".into(),
        "import_media" => {
            let n = input["paths"].as_array().map(|a| a.len()).unwrap_or(0);
            format!("Importing {n} file{}", if n == 1 { "" } else { "s" })
        }
        "add_clip" => "Adding a clip to the timeline".into(),
        "split" => format!("Splitting at {}", t("time").unwrap_or_default()),
        "delete_range" => format!("Cutting {} – {}", t("start").unwrap_or_default(), t("end").unwrap_or_default()),
        "delete_clip" => "Deleting a clip".into(),
        "trim_clip" => "Trimming a clip".into(),
        "move_clip" => "Moving a clip".into(),
        "duplicate_clip" => "Duplicating a clip".into(),
        "set_clip" => {
            let mut parts = Vec::new();
            if let Some(s) = input.get("speed").and_then(Value::as_f64) {
                parts.push(format!("speed {s}×"));
            }
            if input.get("fade_in").is_some() || input.get("fade_out").is_some() {
                parts.push("fades".into());
            }
            if input.get("brightness").is_some() || input.get("contrast").is_some() || input.get("saturation").is_some() {
                parts.push("color".into());
            }
            if input.get("volume").is_some() {
                parts.push("volume".into());
            }
            if let Some(f) = input.get("fit").and_then(Value::as_str) {
                parts.push(f.to_string());
            }
            if parts.is_empty() {
                "Adjusting a clip".into()
            } else {
                format!("Adjusting {}", parts.join(", "))
            }
        }
        "add_text" => {
            let text: String = input["text"].as_str().unwrap_or("").chars().take(32).collect();
            format!("Adding text “{}”", text.replace('\n', " "))
        }
        "update_text" => "Editing text".into(),
        "set_canvas" => {
            let a = input["aspect"].as_str().unwrap_or("");
            let r = input["resolution"].as_str().unwrap_or("");
            format!("Changing format {a} {r}").trim_end().to_string()
        }
        "get_frame" => match t("time") {
            Some(t) => format!("Looking at the frame at {t}"),
            None => "Looking at the preview".into(),
        },
        "seek" => format!("Moving the playhead to {}", t("time").unwrap_or_default()),
        "play" => "Playing the preview".into(),
        "pause" => "Pausing the preview".into(),
        "export_video" => {
            let file = input["path"].as_str().map(|p| Path::new(p).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()).unwrap_or_default();
            format!("Exporting {file}")
        }
        "get_export_status" => "Checking the export".into(),
        "cancel_export" => "Cancelling the export".into(),
        "undo" => "Undoing the last change".into(),
        "redo" => "Redoing".into(),
        "Glob" => format!("Looking for files {}", input["pattern"].as_str().unwrap_or("")),
        "Read" => "Reading a file".into(),
        "ToolSearch" => "Getting ready".into(),
        other => format!("Using {other}"),
    }
}

struct Running {
    child: Child,
    stdin: ChildStdin,
}

pub struct ChatAgent {
    claude: Option<PathBuf>,
    running: Option<Running>,
    session_id: Arc<Mutex<Option<String>>>,
    tx: Sender<ChatEvent>,
    rx: Receiver<ChatEvent>,
    /// True from sending a message until its reply completes.
    pub busy: bool,
    repaint: Arc<dyn Fn() + Send + Sync>,
}

impl ChatAgent {
    pub fn new(repaint: Arc<dyn Fn() + Send + Sync>) -> ChatAgent {
        let (tx, rx) = unbounded();
        ChatAgent { claude: find_claude(), running: None, session_id: Arc::default(), tx, rx, busy: false, repaint }
    }

    pub fn available(&self) -> bool {
        self.claude.is_some()
    }

    pub fn recheck(&mut self) {
        self.claude = find_claude();
    }

    fn write_config_files() -> anyhow::Result<(PathBuf, PathBuf)> {
        let dir = ffmpeg::work_dir();
        let mcp_cfg = dir.join(format!("chat-mcp-{}.json", mcp::bound_port().unwrap_or_else(mcp::port)));
        let cfg = json!({ "mcpServers": { SERVER_KEY: { "type": "http", "url": mcp::own_url() } } });
        std::fs::write(&mcp_cfg, serde_json::to_string_pretty(&cfg)?)?;
        let prompt = dir.join("chat-system-prompt.txt");
        std::fs::write(&prompt, SYSTEM_PROMPT)?;
        Ok((mcp_cfg, prompt))
    }

    fn spawn(&mut self, cwd: &Path) -> anyhow::Result<()> {
        let claude = self.claude.clone().ok_or_else(|| anyhow!("Claude Code is not installed"))?;
        let (mcp_cfg, prompt) = Self::write_config_files()?;
        let allowed = format!("mcp__{SERVER_KEY},Glob,Read");
        let mut args: Vec<String> = vec![
            "-p".into(),
            "--input-format".into(),
            "stream-json".into(),
            "--output-format".into(),
            "stream-json".into(),
            "--verbose".into(),
            "--include-partial-messages".into(),
            "--mcp-config".into(),
            mcp_cfg.to_string_lossy().into_owned(),
            "--strict-mcp-config".into(),
            // Only file search/read built-ins; editing happens through the editor's tools.
            "--tools".into(),
            "Glob,Read".into(),
            "--allowedTools".into(),
            allowed,
            "--permission-mode".into(),
            "dontAsk".into(),
            "--setting-sources".into(),
            "".into(),
            "--append-system-prompt-file".into(),
            prompt.to_string_lossy().into_owned(),
        ];
        if let Some(id) = self.session_id.lock().clone() {
            args.push("--resume".into());
            args.push(id);
        }
        let is_cmd = claude.extension().is_some_and(|e| e.eq_ignore_ascii_case("cmd"));
        let mut cmd = if is_cmd {
            let mut c = ffmpeg::hidden_command("cmd");
            c.arg("/C").arg(&claude);
            c
        } else {
            ffmpeg::hidden_command(&claude)
        };
        cmd.args(&args).current_dir(cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn().context("could not start Claude Code")?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();

        let stderr_buf: Arc<Mutex<String>> = Arc::default();
        {
            let buf = stderr_buf.clone();
            std::thread::spawn(move || {
                let mut s = String::new();
                let _ = stderr.read_to_string(&mut s);
                buf.lock().push_str(&s);
            });
        }
        let tx = self.tx.clone();
        let session = self.session_id.clone();
        let repaint = self.repaint.clone();
        std::thread::Builder::new().name("chat-reader".into()).spawn(move || {
            let mut parser = StreamParser::default();
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                if let Some(id) = v["session_id"].as_str() {
                    *session.lock() = Some(id.to_string());
                }
                for ev in parser.handle(&v) {
                    let _ = tx.send(ev);
                }
                repaint();
            }
            // Process ended. If a reply was in progress, report why.
            if parser.in_turn {
                std::thread::sleep(std::time::Duration::from_millis(100));
                let err = stderr_buf.lock().trim().to_string();
                let msg = if err.is_empty() { "Claude Code stopped unexpectedly.".to_string() } else { last_lines(&err, 6) };
                let _ = tx.send(ChatEvent::TurnDone { error: Some(msg) });
                repaint();
            }
        })?;
        self.running = Some(Running { child, stdin });
        Ok(())
    }

    /// Sends a user message. `context` is a one-line description of the editor state.
    pub fn send(&mut self, text: &str, context: &str, cwd: &Path) -> anyhow::Result<()> {
        if self.busy {
            anyhow::bail!("Claude is still working on the last message");
        }
        let alive = self.running.as_mut().is_some_and(|r| matches!(r.child.try_wait(), Ok(None)));
        if !alive {
            self.running = None;
            self.spawn(cwd)?;
        }
        let content = format!("<editor>{context}</editor>\n{text}");
        let msg = json!({ "type": "user", "message": { "role": "user", "content": content } });
        let r = self.running.as_mut().unwrap();
        let write = writeln!(r.stdin, "{msg}").and_then(|_| r.stdin.flush());
        if let Err(e) = write {
            self.running = None;
            return Err(anyhow!("could not talk to Claude Code: {e}"));
        }
        self.busy = true;
        Ok(())
    }

    /// Stops the current reply. The conversation continues with the next message.
    pub fn stop(&mut self) {
        if let Some(mut r) = self.running.take() {
            let _ = r.child.kill();
            let _ = r.child.wait();
        }
        if self.busy {
            self.busy = false;
            let _ = self.tx.send(ChatEvent::TurnDone { error: Some("Stopped.".into()) });
        }
    }

    /// Starts a fresh conversation.
    pub fn reset(&mut self) {
        self.stop();
        *self.session_id.lock() = None;
        while self.rx.try_recv().is_ok() {}
    }

    pub fn poll(&mut self) -> Vec<ChatEvent> {
        let events: Vec<ChatEvent> = self.rx.try_iter().collect();
        if events.iter().any(|e| matches!(e, ChatEvent::TurnDone { .. })) {
            self.busy = false;
        }
        events
    }
}

impl Drop for ChatAgent {
    fn drop(&mut self) {
        if let Some(mut r) = self.running.take() {
            let _ = r.child.kill();
        }
    }
}

fn last_lines(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// Turns Claude Code's stream-json events into chat events.
#[derive(Default)]
pub struct StreamParser {
    /// Message ids whose text arrived as streamed deltas.
    streamed: std::collections::HashSet<String>,
    current_msg: Option<String>,
    pub in_turn: bool,
}

impl StreamParser {
    pub fn handle(&mut self, v: &Value) -> Vec<ChatEvent> {
        let mut out = Vec::new();
        match v["type"].as_str().unwrap_or("") {
            "system" => {
                if v["subtype"] == "init" || v["subtype"] == "status" {
                    self.in_turn = true;
                }
            }
            "stream_event" => {
                let ev = &v["event"];
                match ev["type"].as_str().unwrap_or("") {
                    "message_start" => self.current_msg = ev["message"]["id"].as_str().map(str::to_string),
                    "content_block_delta" if ev["delta"]["type"] == "text_delta" => {
                        if let Some(id) = &self.current_msg {
                            self.streamed.insert(id.clone());
                        }
                        if let Some(t) = ev["delta"]["text"].as_str() {
                            out.push(ChatEvent::TextDelta(t.to_string()));
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
                                out.push(ChatEvent::Text(t.to_string()));
                            }
                        }
                        Some("tool_use") => out.push(ChatEvent::ToolStart {
                            id: block["id"].as_str().unwrap_or("").to_string(),
                            name: block["name"].as_str().unwrap_or("").to_string(),
                            input: block["input"].clone(),
                        }),
                        _ => {}
                    }
                }
            }
            "user" => {
                for block in v["message"]["content"].as_array().into_iter().flatten() {
                    if block["type"] != "tool_result" {
                        continue;
                    }
                    let id = block["tool_use_id"].as_str().unwrap_or("").to_string();
                    let ok = !block["is_error"].as_bool().unwrap_or(false);
                    let mut summary = String::new();
                    let mut image = None;
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
                    out.push(ChatEvent::ToolDone { id, ok, summary, image });
                }
            }
            "result" => {
                self.in_turn = false;
                let error = if v["is_error"].as_bool().unwrap_or(false) || v["subtype"].as_str().is_some_and(|s| s != "success") {
                    Some(v["result"].as_str().map(str::to_string).unwrap_or_else(|| format!("Claude Code reported an error ({})", v["subtype"].as_str().unwrap_or("unknown"))))
                } else {
                    None
                };
                out.push(ChatEvent::TurnDone { error });
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
            r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"t1","name":"mcp__ai-video-editor__delete_range","input":{"start":9,"end":11}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":"Removed 9.000s–11.000s"}]}]}}"#,
            r#"{"type":"stream_event","event":{"type":"message_start","message":{"id":"m2"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Done"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"!"}}}"#,
            r#"{"type":"assistant","message":{"id":"m2","content":[{"type":"text","text":"Done!"}]}}"#,
            r#"{"type":"result","subtype":"success","is_error":false,"result":"Done!"}"#,
        ];
        let mut p = StreamParser::default();
        let events: Vec<ChatEvent> = lines.iter().flat_map(|l| p.handle(&serde_json::from_str(l).unwrap())).collect();
        let text: String = events.iter().filter_map(|e| if let ChatEvent::TextDelta(t) = e { Some(t.as_str()) } else { None }).collect();
        assert_eq!(text, "Done!");
        assert!(!events.iter().any(|e| matches!(e, ChatEvent::Text(_))), "streamed text must not repeat");
        assert!(events.iter().any(|e| matches!(e, ChatEvent::ToolStart { name, .. } if name.ends_with("delete_range"))));
        assert!(events.iter().any(|e| matches!(e, ChatEvent::ToolDone { ok: true, .. })));
        assert!(matches!(events.last(), Some(ChatEvent::TurnDone { error: None })));
        assert!(!p.in_turn);
    }

    #[test]
    fn reports_errors() {
        let mut p = StreamParser::default();
        let ev = p.handle(&serde_json::from_str(r#"{"type":"result","subtype":"error_during_execution","is_error":true}"#).unwrap());
        assert!(matches!(&ev[0], ChatEvent::TurnDone { error: Some(_) }));
    }

    #[test]
    fn friendly_tool_names() {
        assert_eq!(describe_tool("mcp__ai-video-editor__delete_range", &json!({"start": 9, "end": 11})), "Cutting 0:09.00 – 0:11.00");
        assert_eq!(describe_tool("mcp__ai-video-editor__set_clip", &json!({"speed": 2})), "Adjusting speed 2×");
        assert_eq!(describe_tool("mcp__ai-video-editor__import_media", &json!({"paths": ["a"]})), "Importing 1 file");
    }
}
