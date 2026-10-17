//! The built-in assistant: runs the user's own Claude Code headless, connected
//! to this editor's MCP server, and streams its replies and tool calls back.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
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
around 90–140 units, subtitles 40–60. Keep text off the edges (x 0.1–0.9, y 0.1–0.9) and centered unless asked. \
To make one keyword pop (the product name, a number), use style_text on just that word: the accent color, a marker \
highlight or a bolder weight.
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
        "style_text" => format!("Styling “{}”", q("words")),
        "arrange_clip" => if q("to") == "back" { "Sending a layer to the back".into() } else { "Bringing a layer to the front".into() },
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

/// What one Claude Code process has reported, collected by its reader thread.
#[derive(Default)]
struct Inbox {
    events: Vec<Event>,
    session: Option<String>,
    /// Claude Code got as far as starting the conversation (its `init` message).
    started: bool,
    /// The process closed its output: it has ended.
    exited: bool,
    /// What it wrote to stderr, once it has ended.
    stderr: String,
}

/// One Claude Code process. Each has its own inbox, so nothing a stopped process
/// still says can reach a later turn. Dropping it ends the process and everything it started.
struct Running {
    child: Child,
    group: crate::platform::ProcessGroup,
    stdin: ChildStdin,
    inbox: Arc<Mutex<Inbox>>,
    /// Names this process in its MCP requests, so the editor can refuse calls from a stopped one.
    tag: String,
    /// Started with `--resume`.
    resumed: bool,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.group.kill(&mut self.child);
    }
}

/// The built-in assistant. Owns at most one Claude Code process; the conversation
/// carries over from one process to the next through the session id (`--resume`).
pub struct Agent {
    claude: Option<PathBuf>,
    running: Option<Running>,
    session: Option<String>,
    /// A message was sent and its reply hasn't finished, failed or been stopped.
    pub busy: bool,
    /// The user connected an AI service in settings.
    pub enabled: bool,
}

impl Default for Agent {
    fn default() -> Self {
        Agent { claude: find_claude(), running: None, session: None, busy: false, enabled: false }
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

    /// True if an MCP call tagged `run` comes from the reply in progress. Calls from a
    /// stopped or replaced process are refused, so nothing edits after the user pressed Stop.
    pub fn accepts(&self, run: &str) -> bool {
        self.busy && self.running.as_ref().is_some_and(|r| r.tag == run)
    }

    fn spawn(&mut self, cwd: &Path) -> anyhow::Result<()> {
        static RUNS: AtomicU64 = AtomicU64::new(0);
        let claude = self.claude.clone().ok_or_else(|| anyhow!("Claude Code is not installed"))?;
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let tag = format!("{}-{}-{nanos}", std::process::id(), RUNS.fetch_add(1, Ordering::Relaxed));
        let dir = crate::ffmpeg::work_dir();
        let cfg = dir.join(format!("assistant-mcp-{}.json", std::process::id()));
        let server = json!({ "type": "http", "url": mcp::url(), "headers": { mcp::RUN_HEADER: tag } });
        std::fs::write(&cfg, serde_json::to_string_pretty(&json!({ "mcpServers": { SERVER_KEY: server } }))?)?;
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
        let resumed = self.session.is_some();
        if let Some(id) = &self.session {
            args.extend(["--resume".into(), id.clone()]);
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
        crate::platform::own_process_group(&mut cmd);
        cmd.args(&args).current_dir(cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn().context("could not start Claude Code")?;
        let group = crate::platform::ProcessGroup::of(&child);
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let err_text: Arc<Mutex<Option<String>>> = Arc::default();
        let err_out = err_text.clone();
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = stderr.read_to_string(&mut s);
            *err_out.lock().unwrap() = Some(s);
        });
        let inbox: Arc<Mutex<Inbox>> = Arc::default();
        let reader_inbox = inbox.clone();
        let reader = std::thread::Builder::new().name("assistant-reader".into()).spawn(move || {
            let mut parser = StreamParser::default();
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                let events = parser.handle(&v);
                let mut inbox = reader_inbox.lock().unwrap();
                if let Some(id) = v["session_id"].as_str() {
                    inbox.session = Some(id.to_string());
                }
                inbox.started |= v["type"] == "system" && v["subtype"] == "init";
                inbox.events.extend(events);
            }
            // stderr closes along with stdout; give its reader a moment to finish.
            let mut stderr = None;
            for _ in 0..25 {
                stderr = err_text.lock().unwrap().take();
                if stderr.is_some() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            let mut inbox = reader_inbox.lock().unwrap();
            inbox.stderr = stderr.unwrap_or_default();
            inbox.exited = true;
        });
        if let Err(e) = reader {
            group.kill(&mut child);
            return Err(e.into());
        }
        self.running = Some(Running { child, group, stdin, inbox, tag, resumed });
        Ok(())
    }

    /// Sends a message. `context` describes the editor state in one line.
    pub fn send(&mut self, text: &str, context: &str, cwd: &Path) -> anyhow::Result<()> {
        anyhow::ensure!(!self.busy, "The assistant is still working on the last message");
        // A process that ended between replies is replaced; the conversation resumes.
        if self.running.as_mut().is_some_and(|r| !matches!(r.child.try_wait(), Ok(None))) {
            self.end_process();
        }
        if self.running.is_none() {
            self.spawn(cwd)?;
        }
        let content = format!("<editor>{context}</editor>\n{text}");
        let msg = json!({ "type": "user", "message": { "role": "user", "content": content } });
        let r = self.running.as_mut().unwrap();
        if let Err(e) = writeln!(r.stdin, "{msg}").and_then(|_| r.stdin.flush()) {
            self.end_process();
            return Err(anyhow!("could not talk to Claude Code: {e}"));
        }
        self.busy = true;
        Ok(())
    }

    /// Ends the process now, keeping its session id so the conversation can resume.
    fn end_process(&mut self) {
        if let Some(r) = self.running.take() {
            if let Some(id) = r.inbox.lock().unwrap().session.take() {
                self.session = Some(id);
            }
        }
    }

    /// Stops the current reply at once: the process and everything it started end, and
    /// whatever it was still saying is dropped. The conversation continues with the next
    /// message. Returns true if a reply was in progress.
    pub fn stop(&mut self) -> bool {
        self.end_process();
        std::mem::take(&mut self.busy)
    }

    /// Starts a fresh conversation.
    pub fn reset(&mut self) {
        self.stop();
        self.session = None;
    }

    /// Takes what the assistant said since the last poll. A reply always ends with exactly
    /// one `TurnDone`, also when Claude Code quits in the middle of it.
    pub fn poll(&mut self) -> Vec<Event> {
        let Some(r) = &self.running else { return Vec::new() };
        let resumed = r.resumed;
        let (mut events, exited, started, stderr) = {
            let mut inbox = r.inbox.lock().unwrap();
            if let Some(id) = inbox.session.take() {
                self.session = Some(id);
            }
            (std::mem::take(&mut inbox.events), inbox.exited, inbox.started, std::mem::take(&mut inbox.stderr))
        };
        if events.iter().any(|e| matches!(e, Event::TurnDone { .. })) {
            self.busy = false;
        }
        if exited {
            self.end_process();
            if std::mem::take(&mut self.busy) {
                let lines: Vec<&str> = stderr.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
                let mut msg = if lines.is_empty() { "Claude Code stopped unexpectedly.".to_string() } else { lines[lines.len().saturating_sub(6)..].join("\n") };
                // It couldn't even open the saved conversation: start a new one next time
                // instead of failing the same way on every message.
                if resumed && !started {
                    self.session = None;
                    msg.push_str("\nYour next message starts a new conversation.");
                }
                events.push(Event::TurnDone { error: Some(msg) });
            }
        }
        events
    }
}

/// Turns Claude Code's stream-json lines into chat events.
#[derive(Default)]
pub struct StreamParser {
    streamed: std::collections::HashSet<String>,
    current_msg: Option<String>,
}

impl StreamParser {
    pub fn handle(&mut self, v: &Value) -> Vec<Event> {
        let mut out = Vec::new();
        match v["type"].as_str().unwrap_or("") {
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

    /// A stand-in for Claude Code that does what `mode.txt` (in its working folder) says for each
    /// message. "slow" hands the work to a child process, as `claude.cmd` does with node.
    fn fake_claude(dir: &Path) -> PathBuf {
        let init = r#"{"type":"system","subtype":"init","session_id":"s1"}"#;
        let tool = r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"t1","name":"mcp__ai-video-editor__add_zoom","input":{"start":3}}]}}"#;
        let result = r#"{"type":"result","subtype":"success","is_error":false,"result":"done"}"#;
        #[cfg(windows)]
        let (main, slow, file) = (
            format!(
                "@echo off\necho %* > args.txt\n:loop\nset LINE=\nset /p LINE=\nif not defined LINE exit /b 0\nset /p MODE=<mode.txt\n\
                 if \"%MODE%\"==\"noresume\" (\n  echo No conversation found 1>&2\n  exit /b 1\n)\necho {init}\n\
                 if \"%MODE%\"==\"slow\" cmd /C \"%~dp0slow.cmd\"\nif \"%MODE%\"==\"crash\" (\n  echo boom 1>&2\n  exit /b 1\n)\necho {result}\ngoto loop\n"
            ),
            format!("@echo off\necho {tool}\nping -n 4 127.0.0.1 >nul\necho late> marker.txt\necho {result}\n"),
            "claude.cmd",
        );
        #[cfg(not(windows))]
        let (main, slow, file) = (
            format!(
                "#!/bin/sh\necho \"$@\" > args.txt\nwhile IFS= read -r line; do\n  mode=$(cat mode.txt)\n\
                 if [ \"$mode\" = noresume ]; then echo 'No conversation found' >&2; exit 1; fi\n  echo '{init}'\n\
                 if [ \"$mode\" = slow ]; then sh ./slow.sh; fi\n  if [ \"$mode\" = crash ]; then echo boom >&2; exit 1; fi\n  echo '{result}'\ndone\n"
            ),
            format!("#!/bin/sh\necho '{tool}'\nsleep 3\necho late > marker.txt\necho '{result}'\n"),
            "claude",
        );
        let crlf = |s: String| if cfg!(windows) { s.replace('\n', "\r\n") } else { s };
        std::fs::write(dir.join(if cfg!(windows) { "slow.cmd" } else { "slow.sh" }), crlf(slow)).unwrap();
        let path = dir.join(file);
        std::fs::write(&path, crlf(main)).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    /// Polls until `done` holds for everything received, or fails after 15 s.
    fn wait(agent: &mut Agent, done: impl Fn(&[Event]) -> bool) -> Vec<Event> {
        let (mut all, t) = (Vec::new(), std::time::Instant::now());
        while !done(&all) {
            assert!(t.elapsed().as_secs() < 15, "timed out; got {all:?}");
            all.extend(agent.poll());
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        all
    }

    fn turn_done(events: &[Event]) -> Vec<&Option<String>> {
        events.iter().filter_map(|e| if let Event::TurnDone { error } = e { Some(error) } else { None }).collect()
    }

    #[test]
    fn stop_send_again_and_crashes() {
        let dir = std::env::temp_dir().join(format!("studio-agent-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mode = |m: &str| std::fs::write(dir.join("mode.txt"), m).unwrap();
        let mut agent = Agent { claude: Some(fake_claude(&dir)), running: None, session: None, busy: false, enabled: true };

        // Stopped in the middle of a tool call: the whole process tree ends at once.
        mode("slow");
        agent.send("first", "", &dir).unwrap();
        assert!(agent.send("again", "", &dir).is_err(), "no second message while one is running");
        wait(&mut agent, |e| e.iter().any(|e| matches!(e, Event::ToolStart { .. })));
        let first_run = agent.running.as_ref().unwrap().tag.clone();
        assert!(agent.accepts(&first_run));
        assert!(agent.stop());
        assert!(!agent.busy && !agent.accepts(&first_run), "tool calls from the stopped reply are refused");
        assert!(agent.poll().is_empty(), "a stopped reply says nothing more");
        assert!(!agent.stop(), "stopping again is a no-op");

        // The next message gets a clean reply: no stale error, the conversation resumes.
        mode("fast");
        agent.send("second", "", &dir).unwrap();
        let events = wait(&mut agent, |e| !turn_done(e).is_empty());
        assert_eq!(turn_done(&events), [&None], "exactly one clean end: {events:?}");
        assert!(!agent.busy);
        let args = std::fs::read_to_string(dir.join("args.txt")).unwrap();
        assert!(args.contains("--resume") && args.contains("s1"), "resumed the conversation: {args}");
        let second_run = agent.running.as_ref().unwrap().tag.clone();
        assert_ne!(first_run, second_run);
        assert!(!agent.accepts(&second_run), "an idle assistant makes no calls");

        // Claude Code dies in the middle of a reply: one error with what it printed, then it recovers.
        mode("crash");
        agent.send("third", "", &dir).unwrap();
        let events = wait(&mut agent, |e| !turn_done(e).is_empty());
        assert!(matches!(turn_done(&events)[..], [Some(e)] if e.contains("boom")), "{events:?}");
        assert!(!agent.busy && agent.running.is_none() && agent.session.is_some());

        // The saved conversation can't be opened: the next message starts a new one.
        mode("noresume");
        agent.send("fourth", "", &dir).unwrap();
        let events = wait(&mut agent, |e| !turn_done(e).is_empty());
        assert!(matches!(turn_done(&events)[..], [Some(e)] if e.contains("new conversation")), "{events:?}");
        assert!(agent.session.is_none());
        mode("fast");
        agent.send("fifth", "", &dir).unwrap();
        let events = wait(&mut agent, |e| !turn_done(e).is_empty());
        assert_eq!(turn_done(&events), [&None]);
        assert!(!std::fs::read_to_string(dir.join("args.txt")).unwrap().contains("--resume"));

        // The work the stopped reply handed to its child process never finished.
        std::thread::sleep(std::time::Duration::from_millis(4500));
        assert!(!dir.join("marker.txt").exists(), "the stopped reply kept running in the background");
        drop(agent);
        std::fs::remove_dir_all(dir).ok();
    }
}
