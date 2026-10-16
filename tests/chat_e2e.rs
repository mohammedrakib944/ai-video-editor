//! End-to-end: the built-in chat drives the real Claude Code, which edits the project
//! through this app's MCP server. Uses the developer's Claude account, so it is ignored
//! by default: `cargo test --test chat_e2e -- --ignored --nocapture`

use std::sync::Arc;
use std::time::{Duration, Instant};

use ai_video_editor::chat::{ChatAgent, ChatEvent};
use ai_video_editor::mcp;
use ai_video_editor::project::Editor;
use ai_video_editor::shared::Shared;

#[test]
#[ignore]
fn chat_edits_the_project_through_claude_code() {
    std::env::set_var("AIVE_MCP_PORT", "47950");
    let mut ed = Editor::default();
    ed.has_project = true;
    let shared = Shared::new(ed, Arc::new(|| {}));
    mcp::start(shared.clone()).expect("mcp server");

    let mut agent = ChatAgent::new(Arc::new(|| {}));
    assert!(agent.available(), "Claude Code not installed");
    let cwd = std::env::temp_dir();
    agent
        .send("Add a text that says Hello World starting at 1 second, lasting 2 seconds, at the top.", "playhead 0.00s of 0.00s", &cwd)
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(180);
    let mut tools = Vec::new();
    let mut reply = String::new();
    let mut done = None;
    while Instant::now() < deadline && done.is_none() {
        for ev in agent.poll() {
            match ev {
                ChatEvent::ToolStart { name, .. } => tools.push(name),
                ChatEvent::TextDelta(t) | ChatEvent::Text(t) => reply.push_str(&t),
                ChatEvent::TurnDone { error } => done = Some(error),
                ChatEvent::ToolDone { .. } => {}
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    println!("tools: {tools:?}\nreply: {reply}");
    assert_eq!(done, Some(None), "turn did not finish cleanly");
    let ed = shared.editor.lock();
    let t = ed.project.texts.first().expect("Claude should have added a text clip");
    assert!(t.text.to_lowercase().contains("hello world"), "text was {:?}", t.text);
    assert!((t.start - 1.0).abs() < 0.01 && (t.duration - 2.0).abs() < 0.01);
    assert!(t.y < 0.3, "should be at the top");
    assert!(!agent.busy);
}
