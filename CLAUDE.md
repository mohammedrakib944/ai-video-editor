# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Two apps live in this workspace:
- **`studio/`** (binary `ai-video-studio`) — the Iced rewrite: a motion graphics editor for product showcases. All new work goes here.
- **root crate** (binary `ai-video-editor`) — the original egui app, kept working until the rewrite reaches parity. Treat it as reference.

The rest of this section describes the original app; see "Studio (the rewrite)" below for the new one.


A Rust (egui/eframe) desktop video editor for Windows that Claude edits live through MCP. FFmpeg and ffprobe are external binaries invoked as subprocesses (no ffmpeg bindings). See README.md for the user-facing feature list, keyboard shortcuts and the MCP tool list.

## Commands

```sh
cargo run                                   # run the editor
cargo run -- path/to/project.aive           # open a project directly
cargo run -- --mcp-stdio                    # stdio→HTTP MCP bridge (what Claude Desktop launches)
cargo test                                  # unit, egui_kittest UI, and FFmpeg pipeline tests
cargo test --lib project::                  # tests in one module
cargo test --test pipeline <name>           # one integration test
cargo test --test chat_e2e -- --ignored --nocapture                            # drives the real Claude Code (uses your account)
cargo test --release --test pipeline still_frame_latency -- --ignored --nocapture  # scrub latency timing
powershell -ExecutionPolicy Bypass -File scripts\package.ps1                     # portable dist\ folder (Windows)
```

`tests/pipeline.rs` generates fixtures with real FFmpeg and verifies output with ffprobe, so FFmpeg must be findable. Lookup order: `AIVE_FFMPEG_DIR`, `ffmpeg\` next to the exe, next to the exe, `vendor\ffmpeg\`, then `PATH`. `AIVE_MCP_PORT` overrides the MCP port (default 47821; the server tries the next 20 ports if busy).

## Architecture

**One shared editor state.** `shared::Shared` wraps `Arc<Mutex<project::Editor>>` and is cloned into the UI thread, the MCP HTTP server (one thread per request), and background import/export threads. `Editor` holds the `Project` plus undo/redo stacks, playhead, activity log, export status, and the cross-thread "mailbox" fields below.

**All edits go through `Editor::apply` / `apply_merge`.** These snapshot the whole `Project` (clone) before running the closure, roll back if it returns `Err`, push an undo entry, log the change tagged `Source::User` or `Source::Ai`, and bump `revision`. `apply_merge` with a `merge_key` coalesces consecutive edits (slider drags, typing, timeline trims) into one undo step. Edit logic lives as methods on `Project` (`split_at`, `delete_range`, `set_speed`, …); UI and MCP code only call them inside `apply`. Don't mutate `ed.project` directly or the change won't be undoable and the preview won't refresh.

**MCP and background threads never touch UI state.** They communicate via `Editor` fields that the UI consumes once per frame in `ui/mod.rs`: `revision` (UI re-clones its project snapshot when it changes), `seek_request` / `play_request` (taken and cleared), `ai_touched` (clip glow), then call `Shared::repaint()` to wake egui.

**One render path for everything.** `render::build` turns a `Project` + `RenderSpec` (time range, size, fps, video/audio, `preview` flag) into an ffmpeg command with a filter graph. The live preview (`playback.rs`), still frames (`render_frame`, also used by the `get_frame` MCP tool) and export (`render::export`) all use it, so a new clip effect must be added in `build` to appear anywhere. Temp files for each run go in a `JobDir` under `ffmpeg::work_dir()` (`%LOCALAPPDATA%\AIVideoEditor`, else the system temp dir) and are deleted on drop.

**Playback** (`playback.rs`): `play` spawns ffmpeg processes for video frames and audio; cpal audio is the master clock. Scrub stills are cached by `StillKey { revision, frame, size }`.

**Timeline model**: the main track is magnetic (a `Vec` of clips with no gaps; positions come from `main_starts()`), while music and text clips have absolute `start` times.

**MCP server** (`mcp.rs`): hand-rolled JSON-RPC over HTTP with `tiny_http`. Adding a tool means adding its schema in `tool_list()` and a branch in `call_tool()`, plus a friendly label in `chat::describe_tool` and the tool list in README.md. `--mcp-stdio` (`run_stdio_bridge`) forwards stdin lines to the running app's HTTP endpoint and launches the app if it isn't running.

**Built-in chat** (`chat.rs`, `ui/chat_panel.rs`): spawns the user's `claude` CLI in `-p` stream-json mode with `--strict-mcp-config` pointing only at this app's server (key `ai-video-editor`, so tools appear as `mcp__ai-video-editor__*`). Built-ins are limited to `Glob,Read`, and `--resume` keeps the session. `SYSTEM_PROMPT` there describes the `<editor>` context line (playhead, selected clip) prepended to each user message.

**Integrations** (`integrations.rs`): writes MCP config entries for Claude Code (`claude mcp add`), Claude Desktop (`claude_desktop_config.json`, including the MSIX `LocalCache` path) and Cursor (`~/.cursor/mcp.json`).

## Platform notes

The project is built on both Windows (GNU toolchain: `x86_64-pc-windows-gnu` + MinGW-w64) and Linux, so changes must keep both compiling. OS differences go in `src/platform.rs` (home dir, fonts via `WINDIR\Fonts` vs `fc-match`, `explorer` vs `xdg-open`, example paths, FFmpeg hint) or behind `#[cfg(windows)]`; keep Windows behavior unchanged when adding Linux support. Other OS-specific spots: `ffmpeg.rs` (`.exe`, `CREATE_NO_WINDOW`, `filter_script_option()` picks `-/filter_complex` for FFmpeg ≥ 7 or `-filter_complex_script` for 6.x), `chat::find_claude`, `integrations::connect` (`cmd /C claude` on Windows), `shared::strip_verbatim`, and the Linux-only eframe `wayland`/`x11` features in `Cargo.toml`.

Linux build needs `libasound2-dev` (cpal/ALSA) and FFmpeg on PATH. `cargo check --target x86_64-pc-windows-gnu` from Linux additionally needs `mingw-w64` (the `ring` crate compiles C).

## Studio (the rewrite)

```sh
cargo run -p studio                      # start screen
cargo run -p studio -- path/to/x.aivp    # open a project
cargo test -p studio                     # unit + FFmpeg/GPU integration tests (export, decoder, audio clock)
```

Layout of `studio/src`:
- `model.rs` / `clip.rs` — project document (`.aivp` JSON). Positions are canvas fractions; lengths are "canvas units" (px at a 1080 short side). Clips carry transform, look (corners/border/shadow), grade, audio, Camtasia-style `zooms` (`camera_at` pans between adjacent zooms), keyframes, intro/outro motions, transitions.
- `edit.rs` — every edit; clips never overlap on a track; new clips keep their time and go to a free track. `doc.rs` — undo history (`apply` / `apply_merge` for slider drags).
- `render/` — `scene::evaluate(project, t, size, sources)` → layers; `compositor` (wgpu, `layer.wgsl`) draws them in the preview (inside iced's render pass, `editor/preview.rs`) and headless for export/snapshots (`export.rs::Offscreen`). `frames.rs` runs one FFmpeg decoder per visible clip; `text.rs` rasterizes with cosmic-text (Bangla shaping); `audio.rs` builds the FFmpeg mix graph used by export and by `playback.rs` (cpal; audio is the master clock).
- `editor/` — panes, `timeline.rs` (two stacked canvases: iced draws images above meshes within one layer), `inspector.rs` (`Edit` enum), `canvas_overlay.rs` (select/move/scale/rotate, zoom focus editing), `tools.rs` (MCP tool schemas + handlers), `assistant_panel.rs`.
- `mcp.rs` — MCP server on 127.0.0.1:47822 (`AIVE_STUDIO_MCP_PORT`); calls reach the UI thread via a subscription and answer through a reply channel. `assistant.rs` runs the user's Claude Code headless against it.
- `ui/` — design system: `tokens.rs` (all colors/sizes/fonts), `style.rs`, `widgets.rs`, Lucide icons (`icons.rs`), window chrome (frameless; Windows uses native rounded corners/shadow, Linux draws them on a transparent window).

Conventions: every visual value comes from `ui/tokens.rs`; the user wants every screen premium-polished, so check new UI with a screenshot. FFmpeg filter-script files must outlive the FFmpeg process that reads them.
