# AI Video Editor

A motion graphics editor for product showcases — Apple-style launch videos and SaaS explainers
made from screen recordings. Everything works by hand, and an optional AI assistant (your own
Claude Code) can edit alongside you: every change appears live and can be undone.

This repository contains two apps:

| App | Folder | Binary | Status |
|---|---|---|---|
| **Studio** (Iced) | `studio/` | `ai-video-studio` | The current app. All new work happens here. |
| Original editor (egui) | repo root | `ai-video-editor` | Legacy, kept until Studio covers everything. |

The rest of this file is about **Studio** unless it says otherwise.

---

## Setup

You need **Rust**, **FFmpeg** (ffmpeg + ffprobe) and, for the assistant, **Claude Code**.
The same code builds on Windows and Linux.

### Windows

1. **Rust** (GNU toolchain — works without Visual Studio):
   ```powershell
   rustup-init.exe --default-host x86_64-pc-windows-gnu
   ```
2. **MinGW-w64** (the linker the GNU toolchain uses):
   ```powershell
   winget install BrechtSanders.WinLibs.POSIX.UCRT
   ```
   Open a new terminal afterwards so `gcc` is on `PATH`.
3. **FFmpeg**: download a `win64-gpl` build from https://github.com/BtbN/FFmpeg-Builds/releases
   and copy `ffmpeg.exe` and `ffprobe.exe` into `vendor\ffmpeg\` in this repo
   (`vendor\` is git-ignored). Putting them on `PATH` works too.
4. **Run**:
   ```powershell
   cargo run -p studio --release
   ```
   The first build takes several minutes; later builds are quick.

On Windows the window gets native rounded corners and shadow, and drag and drop from Explorer
works out of the box.

### Linux (tested on Ubuntu 24.04)

1. **Rust**:
   ```sh
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   source ~/.cargo/env        # or open a new terminal
   ```
2. **System packages** (ALSA headers for audio, FFmpeg 6.x or 7.x):
   ```sh
   sudo apt install build-essential pkg-config libasound2-dev ffmpeg
   ```
3. **Run**:
   ```sh
   cargo run -p studio --release
   ```

Notes for Linux:
- **Drag and drop**: the window library (winit) can't receive dropped files on Wayland, so on a
  Wayland desktop the app runs through XWayland automatically, where drag and drop works. If text
  looks slightly soft on a scaled display, `STUDIO_WAYLAND=1 cargo run -p studio --release` runs
  natively on Wayland (without drag and drop; use the **+** button to import instead).
- If you see `cargo: command not found`, run `source ~/.cargo/env` or open a new terminal.

### The AI assistant (optional, both platforms)

1. Install **Claude Code** from https://claude.com/claude-code and run `claude` once in a
   terminal to sign in. No API key is needed — it uses your Claude subscription.
2. In the app, click **Connect AI** in the title bar → **Connect** next to Claude Code.
   The chip then shows **Claude Code** with a lime dot.

The app finds Claude Code in `~/.local/bin`, on `PATH`, or (Windows) in `%APPDATA%\npm`.
Nothing is sent to any AI until you connect one.

### Run options

```sh
cargo run -p studio --release -- path/to/project.aivp   # open a project directly
cargo test -p studio                                    # tests (need FFmpeg; some use the GPU)
```

| Variable | Effect |
|---|---|
| `AIVE_FFMPEG_DIR` | Folder containing ffmpeg/ffprobe (overrides the search) |
| `AIVE_STUDIO_MCP_PORT` | Port for the assistant/MCP server (default 47822; the next free one is used if busy) |
| `STUDIO_WAYLAND=1` | Linux: stay on native Wayland instead of XWayland |

FFmpeg is searched in this order: `AIVE_FFMPEG_DIR`, `ffmpeg\` next to the app, next to the
app, `vendor/ffmpeg/` in the repo, then `PATH`.

### Where things are stored

| What | Windows | Linux |
|---|---|---|
| Settings (recent projects, layout, connected AI) | `%APPDATA%\AIVideoEditor\studio.json` | `~/.config/ai-video-editor/studio.json` |
| Built-in sound effects (made on first use) | `%APPDATA%\AIVideoEditor\sounds\` | `~/.config/ai-video-editor/sounds/` |
| New projects (`.aivp`) | `Videos\AI Video Editor\` | `~/Videos/AI Video Editor/` |

Projects save automatically a couple of seconds after each change.

---

## What you can do

- **Start screen**: pick a format (16:9, 9:16, 1:1, 4:5, 21:9 · 720p–4K · 24–60 fps) or reopen a
  recent project. Drop media on it to start a project from that file.
- **Import** with the **+** button, `Ctrl+I`, or by dragging files into the window. Thumbnails,
  filmstrips and waveforms load in the background. Missing files show a red badge — right-click →
  **Locate file…** to relink.
- **Timeline**: as many video and audio tracks as you like (right-click a track header to rename,
  add, move, lock, hide/mute or delete; drag a header's bottom edge to resize the track; drag the
  gap above the timeline to resize the whole area). Move, trim and snap clips; drag media in from
  the library.
- **Split at the playhead** (`S`), **mark a range** with the playhead (`R` for A then B, or `I`/`O`).
- **Zoom & pan, Camtasia-style**: put the playhead where the zoom should start and press `Z`.
  Zooms appear as bands on the clip — drag to move, drag the ends to resize, click to aim the
  focus rectangle on the canvas. Pressing `Z` again right after a zoom adds a smooth **pan**.
- **Preview**: GPU-rendered, with audio. Click layers to select them; drag to move (snaps to the
  center), corners to scale, the knob to rotate — handles work past the canvas edge.
- **Inspector**: position, scale, rotation, opacity, framing; rounded corners, border, drop shadow;
  color (brightness, contrast, saturation, warmth); speed; volume and fades; **range volume**
  (change or mute just the marked range of an audio clip); text (Inter, Poppins, Hind Siliguri for
  Bangla); shapes (rectangle, ellipse, triangle, star, hexagon, ring with linear/radial gradients
  at any angle); animations in/out; transitions.
- **Library tabs**: Media, Text (headline, subtitle, Bangla headline, shape presets), Effects
  (transitions and animations), Sounds (9 original effects — whoosh, pop, click, riser…).
- **Assistant**: describe what you want; it sees the playhead, the selected clip and the marked
  range. Tool calls show as cards, and it can look at rendered frames to check its work.
- **Export** to MP4 (H.264 + AAC) with progress, time left and cancel. The export uses the same
  renderer as the preview, and a cancelled or failed export leaves no broken file.
- **Other AI apps** (Claude Desktop, Cursor, …) can connect to the MCP server shown in the
  Connect AI dialog (`http://127.0.0.1:47822/mcp`) while the app is open.

### Keyboard

| Key | Action |
|---|---|
| Space | Play / pause |
| S or B (Ctrl+B) | Split at the playhead |
| R · I · O | Mark range (A, then B) · set start · set end |
| Z | Add a zoom (or pan) at the playhead |
| T | Add text at the playhead |
| Delete / Backspace | Delete the selected clips (or the zoom being edited) |
| Ctrl+D | Duplicate |
| ← / → (Shift: 10 frames) | Step one frame |
| Home / End | Start / end |
| + / − · Shift+Z | Zoom the timeline in / out · fit |
| N · M | Snapping · mute preview |
| Ctrl+Z · Ctrl+Shift+Z or Ctrl+Y | Undo · redo |
| Ctrl+N · Ctrl+O · Ctrl+S · Ctrl+Shift+S | New · open · save · save as |
| Ctrl+I · Ctrl+E · Ctrl+K | Import · export · assistant |
| Esc | Close a dialog, cancel a rename, leave zoom editing, clear the selection |

---

## Original editor (legacy)

The egui app at the repo root still builds: `cargo run` (or `cargo test`). It uses the same
FFmpeg setup; its MCP server listens on port 47821 (`AIVE_MCP_PORT`), and
`powershell -ExecutionPolicy Bypass -File scripts\package.ps1` builds its portable Windows folder.
