# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A motion graphics editor for product showcases (Apple-style launches, SaaS explainers), fully
usable by hand, with an optional AI assistant that edits through MCP. English and Bangla.
The Cargo workspace has one app:

- **`studio/`** (binary `ai-video-studio`, "AI Video Studio", Iced 0.14 + wgpu). The original egui
  app that used to live at the repo root was removed (it's in git history if ever needed).
- `spikes/iced-ui/` — the throwaway prototype that picked Iced (its own workspace; ignore).

README.md is the user-facing setup guide (Windows + Linux), feature list and shortcuts — keep it
in sync when adding features, shortcuts, env vars or file locations.

## Working rules

- **Never `git commit`** (or push) — the user makes all commits. Read-only git is fine.
- The project is built on **both Windows (GNU toolchain) and Linux** — changes must compile on
  both, and Windows behavior must not change when fixing Linux. OS-specific code goes in
  `studio/src/platform.rs` or behind `#[cfg(windows)]` / `#[cfg(target_os = "linux")]`.
- The user's own running app owns MCP port 47822. Test instances must use another port:
  `AIVE_STUDIO_MCP_PORT=47950`.
- Don't drive the user's Claude account (live assistant chats) without asking.
- UI must look premium: every color/size/font comes from `ui/tokens.rs`; check new UI visually
  with the debug snapshot below (not desktop screenshots — they capture the user's other windows).

## Commands

```sh
cargo run -p studio --release                    # start screen
cargo run -p studio --release -- path/to/x.aivp  # open a project
cargo test -p studio                             # unit + FFmpeg/GPU integration tests
cargo test -p studio range_volume                # tests matching a name
cargo build -p studio --release                  # release build check
```

Tests generate fixtures with real FFmpeg, so it must be findable (lookup in `studio/src/ffmpeg.rs`:
`AIVE_FFMPEG_DIR`, `ffmpeg/` next to the exe, next to the exe, workspace `vendor/ffmpeg/`, `PATH`).
Linux needs `libasound2-dev`. Studio doesn't depend on `ring`, so Windows needs only the Rust
GNU toolchain + MinGW-w64 (WinLibs). Windows packaging:
`scripts\package-studio.ps1`; Linux: `scripts/package-studio.sh`.

### Debug snapshots (debug builds only)

`cargo run -p studio -- project.aivp` with these env vars renders the real window to a PNG and exits:
`STUDIO_SNAPSHOT=out.png`, `STUDIO_SNAPSHOT_DELAY` (seconds), `STUDIO_SNAPSHOT_TIME` (playhead),
`STUDIO_SNAPSHOT_SCROLL`, `STUDIO_SNAPSHOT_SELECT=clip[,a,b]` (select a clip, optionally mark a range),
`STUDIO_SNAPSHOT_OPEN=ai|assistant|effects|settings` (open the Connect AI dialog, the assistant panel,
the Effects tab or Settings), `STUDIO_SNAPSHOT_ZOOM=2` (preview zoom), `STUDIO_SNAPSHOT_CHAT=reply.md` (a sample
conversation with that Markdown reply, no AI needed), `STUDIO_SNAPSHOT_REOPEN=1` (switch to the
assistant tab last), `STUDIO_SNAPSHOT_SEEK=seconds` (move the playhead). Set `APPDATA` to a scratch folder for test instances so they don't touch the
user's settings or recent projects.
Wired through `Message::Snapshot` / `DevSelect` in `editor/mod.rs`, all `cfg(debug_assertions)`.

## Studio architecture (`studio/src`)

**Document model.** `model.rs` (Project, Asset, Track with optional `height`, Canvas; saved as
`.aivp` JSON) and `clip.rs` (Clip with transform, look — corners/border/shadow, grade, audio with
fades and range `gains`, Camtasia-style `zooms` where `camera_at` pans between adjacent zooms,
keyframes, intro/outro motions, transitions, text, shape and blur layers (`Content::Blur`: a box that blurs
the media below it while the clip is on), `crop`, and `part` — Whole, or Picture/Sound after "Separate audio"; use
`Project::clip_kind`, `clip_has_picture`, `clip_has_sound` instead of the asset kind). Timeline
`markers` live on the Project (colored points or spans; not rendered). Positions are canvas
fractions; lengths are "canvas units" (px at a 1080 short side). Types derive `PartialEq`.

**All edits are methods in `edit.rs`, run through `doc.rs`.** `Doc::apply` / `apply_merge` clone
the project, run the edit on the draft, and only commit if it succeeded *and* changed something
(no-op edits make no undo step and don't dirty the project). `apply_merge` with a key coalesces
slider drags/typing within 900 ms. `next_id` never goes backwards across undo/redo because caches
are keyed by ids. Edit invariants: clips never overlap on a track; new clips keep their time and
go to a free (or new) track; locked tracks reject edits (`ensure_unlocked`); trim drops zooms/gains
outside the clip; `set_speed` ripples following clips and rescales zooms/gains/keyframes.
UI, MCP tools and tests all go through `Doc` — never mutate the project directly.

**One renderer for preview and export.** `render/scene.rs::evaluate(project, t, size, sources)`
→ `LayerSpec`s; `render/compositor.rs` + `layer.wgsl` (wgpu; SDF shapes, gradients, corners,
shadows) draws them inside iced's render pass for the preview (`editor/preview.rs`) and headless
for export/snapshots (`render/export.rs::Offscreen`). A new visual effect must go through scene +
compositor to appear anywhere. Crop shrinks the layer and its `uv`; zooms work inside the crop; a blur
clip adds no pixels: `scene::evaluate` maps its box onto each media layer below it (through rotation,
crop and zoom) as per-layer blur regions (up to 4; 9×9 Gaussian in `layer.wgsl`) and keeps an
invisible layer for picking. The
per-layer uniform is 320 bytes (stride 512) and carries a `view` transform: the preview can be
zoomed past its pane, so `preview.rs` draws only the visible part of the canvas rectangle. `render/frames.rs` (FrameServer) runs one FFmpeg decoder per
visible clip and caches images; `render/text.rs` rasterizes with cosmic-text (Bangla shaping);
`render/audio.rs::mix_graph` builds the FFmpeg audio mix used by export and playback.
`playback.rs` plays audio with cpal; the audio clock is the master clock.

**Editor UI** (`editor/`): `mod.rs` holds `Editor`, its `Message`s, keyboard shortcuts, dialogs,
autosave (2 s) and the timeline layout; `timeline.rs` (ruler, lanes, zoom bands, volume envelope),
`inspector.rs` (`Edit` enum → edits; `HexDraft` color inputs), `canvas_overlay.rs`
(select/move/scale/rotate, zoom-focus editing), `library.rs` (media/text/shape/effect/sound
presets), `assistant_panel.rs`, `export_dialog.rs`, `media_cache.rs` (thumbnails, filmstrips,
waveforms). `ui/` is the design system: `tokens.rs`, `style.rs`, `widgets.rs`, Lucide `icons.rs`,
`chrome.rs` (frameless title bar, motion logo). `icon.rs` rasterizes the same logo with plain std: the window icon, `build.rs` (embeds a
`.ico` in the Windows exe through `windres`/`rc.exe`) and `--write-icons DIR` (packaging) all use it.
Right-clicks on the lanes and the preview select what's under the cursor, then one `context_menu()`
serves both. The inspector takes an `inspector::Ctx` (edit modes, range, collapsed sections, search);
every section goes through `ctx.section(...)` so search and collapse work. Library effects are
`library::Effect`s: click applies to the selection, drag drops on a clip (`Message::DropEffect`).
Packaging: `scripts/package-studio.ps1` (Windows) and `scripts/package-studio.sh` (Linux). The window never draws its own frame; on Windows
it gets native rounded corners/shadow.

**AI.** `mcp.rs` serves MCP (JSON-RPC over `tiny_http`) on the first free port from 47822
(`AIVE_STUDIO_MCP_PORT`), local origins only; calls reach the UI thread through a subscription and
answer on a reply channel. Tools live in `editor/tools.rs` (schema + handler per tool) — adding a
tool means adding both there. `assistant.rs` runs the user's `claude` CLI headless against that
server (enabled only when connected); `ai_connect.rs` is the Connect AI dialog and title-bar chip;
the chosen provider persists in `settings.rs`. ChatGPT is shown as "Coming next" (undecided:
Codex CLI vs API key).

**Settings dialog.** `settings_dialog.rs` edits `settings::Prefs` (preview quality with Auto, UI
scale via `App::scale_factor`, snapping/mute defaults, default still length, autosave interval,
export quality, projects folder); `Editor::apply_prefs` applies them. Auto quality steps down while
`FrameServer::take_lag` reports late frames during playback and resets when playback stops.
The audio mix must keep `apad=whole_dur=…`: an unbounded `apad` makes long exports crawl.

**Other modules.** `settings.rs` (recent projects, layout, AI provider; `%APPDATA%\AIVideoEditor\studio.json`
/ `~/.config/ai-video-editor/studio.json`; projects default to `~/Videos/AI Video Editor`),
`sfx.rs` (9 synthesized sounds written to the config dir), `fonts.rs` (bundled Inter, Poppins,
Hind Siliguri), `ffmpeg.rs`, `platform.rs` (reveal/open in file manager), `main.rs` (on Linux,
removes `WAYLAND_DISPLAY` to run under XWayland so drag and drop works — winit has no Wayland DnD;
`STUDIO_WAYLAND=1` opts out).

## Gotchas

- Iced draws images above meshes within one layer — stack separate canvases to layer them
  (the timeline is `LanesBase` + `Lanes`). Stacked layers escape a scrollable's clipping, and
  images/text aren't clipped to canvas bounds, so clip manually (`on_canvas`).
- Canvas `with_clip` keeps absolute coordinates.
- FFmpeg filter-script files must outlive the FFmpeg process that reads them. Use
  `ffmpeg::filter_script_option()` (FFmpeg 7 `-/filter_complex` vs 6.x `-filter_complex_script`).
- Export writes through a cleanup guard so a cancelled/failed export leaves no partial file.

