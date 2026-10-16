//! One-click MCP setup for Claude Code, Claude Desktop and Cursor.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use serde_json::{json, Value};

use crate::mcp;

pub const SERVER_KEY: &str = "ai-video-editor";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Client {
    ClaudeCode,
    ClaudeDesktop,
    Cursor,
}

impl Client {
    pub const ALL: [Client; 3] = [Client::ClaudeCode, Client::ClaudeDesktop, Client::Cursor];

    pub fn label(self) -> &'static str {
        match self {
            Client::ClaudeCode => "Claude Code",
            Client::ClaudeDesktop => "Claude Desktop",
            Client::Cursor => "Cursor",
        }
    }
}

fn home() -> PathBuf {
    crate::platform::home_dir()
}

fn claude_desktop_configs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(appdata) = std::env::var_os("APPDATA") {
        out.push(PathBuf::from(appdata).join("Claude").join("claude_desktop_config.json"));
    }
    // Microsoft Store installs keep their config inside the package folder.
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        if let Ok(entries) = std::fs::read_dir(PathBuf::from(local).join("Packages")) {
            for e in entries.flatten() {
                if e.file_name().to_string_lossy().starts_with("Claude_") {
                    out.push(e.path().join("LocalCache").join("Roaming").join("Claude").join("claude_desktop_config.json"));
                }
            }
        }
    }
    out
}

fn read_json(path: &Path) -> anyhow::Result<Value> {
    if !path.is_file() {
        return Ok(json!({}));
    }
    let s = std::fs::read_to_string(path)?;
    if s.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(&s).with_context(|| format!("{} is not valid JSON; fix or delete it first", path.display()))
}

fn has_server(path: &Path) -> bool {
    read_json(path).map(|v| v["mcpServers"].get(SERVER_KEY).is_some()).unwrap_or(false)
}

/// Adds our server to an `mcpServers` JSON config, keeping a backup of the original.
fn write_server_entry(path: &Path, entry: Value) -> anyhow::Result<()> {
    let mut root = read_json(path)?;
    if !root.is_object() {
        bail!("{} has an unexpected format", path.display());
    }
    if path.is_file() {
        let _ = std::fs::copy(path, path.with_extension("json.bak"));
    }
    if !root["mcpServers"].is_object() {
        root["mcpServers"] = json!({});
    }
    root["mcpServers"][SERVER_KEY] = entry;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(&root)?)?;
    Ok(())
}

pub fn is_configured(client: Client) -> bool {
    match client {
        Client::ClaudeCode => has_server(&home().join(".claude.json")),
        Client::ClaudeDesktop => claude_desktop_configs().iter().any(|p| has_server(p)),
        Client::Cursor => has_server(&home().join(".cursor").join("mcp.json")),
    }
}

pub fn claude_code_command() -> String {
    format!("claude mcp add --transport http --scope user {SERVER_KEY} {}", mcp::url())
}

/// Registers the MCP server with the client. Returns a message for the user.
pub fn connect(client: Client) -> anyhow::Result<String> {
    match client {
        Client::ClaudeCode => {
            let run = |args: &[&str]| {
                #[cfg(windows)]
                let mut cmd = {
                    let mut c = crate::ffmpeg::hidden_command("cmd");
                    c.arg("/C").arg("claude");
                    c
                };
                #[cfg(not(windows))]
                let mut cmd = std::process::Command::new(crate::chat::find_claude().unwrap_or_else(|| "claude".into()));
                cmd.args(args);
                cmd.output()
            };
            // Remove an old entry first so the URL is always current.
            let _ = run(&["mcp", "remove", "--scope", "user", SERVER_KEY]);
            let url = mcp::url();
            let out = run(&["mcp", "add", "--transport", "http", "--scope", "user", SERVER_KEY, &url])
                .with_context(|| format!("could not run the `claude` command. Run this in a terminal instead:\n{}", claude_code_command()))?;
            if !out.status.success() {
                let err = String::from_utf8_lossy(&out.stderr);
                let msg = if err.contains("not recognized") || err.trim().is_empty() {
                    "Claude Code was not found. Install it, or run this in a terminal:".to_string()
                } else {
                    err.trim().to_string()
                };
                bail!("{msg}\n{}", claude_code_command());
            }
            Ok("Claude Code is connected. Start a new `claude` session and ask it to edit your video.".into())
        }
        Client::ClaudeDesktop => {
            let exe = std::env::current_exe()?;
            let entry = json!({ "command": exe, "args": ["--mcp-stdio"] });
            let configs = claude_desktop_configs();
            let mut wrote = 0;
            for path in &configs {
                // Only touch the Store config if that install exists.
                if path.parent().and_then(Path::parent).is_some_and(|p| p.is_dir()) || path == &configs[0] {
                    write_server_entry(path, entry.clone())?;
                    wrote += 1;
                }
            }
            if wrote == 0 {
                bail!("Claude Desktop config folder not found");
            }
            Ok("Claude Desktop is set up. Restart Claude Desktop to load the video tools.".into())
        }
        Client::Cursor => {
            write_server_entry(&home().join(".cursor").join("mcp.json"), json!({ "url": mcp::url() }))?;
            Ok("Cursor is set up. Reload Cursor to load the video tools.".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_entry_and_keeps_other_servers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cfg.json");
        std::fs::write(&path, r#"{"mcpServers":{"other":{"command":"x"}},"theme":"dark"}"#).unwrap();
        write_server_entry(&path, json!({ "url": "http://127.0.0.1:1/mcp" })).unwrap();
        let v = read_json(&path).unwrap();
        assert_eq!(v["mcpServers"]["other"]["command"], "x");
        assert_eq!(v["theme"], "dark");
        assert!(has_server(&path));
        assert!(path.with_extension("json.bak").is_file());
    }

    #[test]
    fn refuses_to_overwrite_broken_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cfg.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(write_server_entry(&path, json!({})).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
    }
}
