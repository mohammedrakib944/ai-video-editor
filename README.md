# AI Video Studio

A motion graphics editor for product showcases — Apple-style launch videos and SaaS explainers
made from screen recordings. Everything works by hand, and an optional AI assistant (your own
Claude Code) can edit alongside you: every change appears live and can be undone.

The app lives in `studio/` (binary `ai-video-studio`, built with Iced + wgpu).

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
| Settings (recent projects, layout, connected AI, preferences) | `%APPDATA%\AIVideoEditor\studio.json` | `~/.config/ai-video-editor/studio.json` |
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
  gap above the timeline to resize the whole area; it starts compact). Move, trim and snap clips;
  drag a clip above the top track to put it on a new track over everything; drag media in from the
  library. **Right-click a clip** (on the timeline or the preview) for split, duplicate, **separate
  audio** (the sound moves to its own audio track), mute, zoom, crop, **bring to front / send to back**,
  range volume and delete. On the preview, a selected element keeps the drag even where another
  element lies over it.
- **Split at the playhead** (`S`), **mark a range** with the playhead (`R` for A, then B — the
  range follows the playhead until you set B; or `I`/`O`). Then a **✓ Done** button appears: drag the
  A/B flags on the ruler to adjust the range, and click Done (or press `Enter` or `R`) to finish; ✕ clears it.
  Shift+drag on the ruler draws a range too.
- **Markers**: colored flags on the ruler (`Shift+M`, or the **Marker** button) — a span when a
  range is marked. Click one to jump to it and rename or recolor it in the inspector; drag to move.
- **Zoom & pan, Camtasia-style**: put the playhead where the zoom should start and press `Z`.
  Zooms appear as bands on the clip — drag to move, drag the ends to resize, click to aim the
  focus rectangle on the canvas. Pressing `Z` again right after a zoom adds a smooth **pan**.
- **Preview**: GPU-rendered, with audio. Click layers to select them; drag to move (snaps to the
  center), corners to scale, the knob to rotate, the inner dot to round the corners; shapes and
  markers resize from every side (Shift on a corner keeps the proportions). Handles work past the
  canvas edge. **Zoom the preview** from the toolbar (Fit, 25–400%), `Ctrl+0` / `Ctrl+1`, or
  Ctrl + wheel; the wheel or the middle button pans a zoomed preview. This never changes the video size.
- **Inspector** (search box on top; click a section title to collapse it): position, scale,
  rotation, opacity, framing; **crop** (sliders, or drag the edges on the canvas); rounded corners,
  border, drop shadow; color (brightness, contrast, saturation, warmth); speed; volume and fades; **range volume** (change or mute just the marked range of a clip — each changed
  part keeps its own slider); text (Inter, Poppins, Hind Siliguri for
  Bangla) — **select words in the text box** to give just them their own color, weight, font, size,
  marker highlight or underline (styles stay on their words while you keep typing); shapes (rectangle, ellipse, triangle, star, hexagon, ring, frame with linear/radial gradients
  at any angle); animations in/out; transitions.
- **Library tabs**: Media (grouped into videos, images and audio), Text (headline, subtitle, Bangla headline, shape presets), Effects
  (markers — box, circle, highlight, underline in a color you pick — and **blur**: they go on the
  timeline at the playhead for 3 s, or over the marked range, and you move and resize them on the
  preview; a blur only blurs the video under its box, only while it's on the timeline; plus
  transitions and animations: click to apply to the selection, or drag onto any clip), Sounds (9 original effects — whoosh, pop, click, riser…).
- **Assistant**: describe what you want; it sees the playhead, the selected clip and the marked
  range. Write on several lines freely: Enter starts a new line and only the Send button sends. Replies show
  formatted (headings, lists, code with a copy button, links) with a Copy button for the whole reply. Tool calls show
  as cards, and it can look at rendered frames to check its work. **Stop** ends the reply at once, including
  anything Claude Code started, and refuses edits still on the way; your next message continues the conversation.
- **Export** to MP4 (H.264 + AAC) with progress, time left and cancel. The export uses the same
  renderer as the preview, and a cancelled or failed export leaves no broken file.
- **Settings** (gear in the title bar, the app menu, or `Ctrl+,`): preview quality — **Auto**
  (sharp while paused, lighter during playback when the computer can't keep up), Full, High,
  Balanced or Draft; it never affects exports — interface size (90–125%), snapping on by default,
  how long new text/shapes/images last, how soon projects autosave, starting with the preview
  muted, the default export quality, and the folder for new projects. Changes apply right away.
- **Other AI apps** (Claude Desktop, Cursor, …) can connect to the MCP server shown in the
  Connect AI dialog (`http://127.0.0.1:47822/mcp`) while the app is open.

### Keyboard

| Key | Action |
|---|---|
| Space | Play / pause |
| S or B (Ctrl+B) | Split at the playhead |
| R · I · O | Mark range (A, then B, then Done) · set start · set end |
| Z | Add a zoom (or pan) at the playhead |
| T | Add text at the playhead |
| Delete / Backspace | Delete the selected clips (or the zoom being edited, or the selected marker) |
| Ctrl+D | Duplicate |
| Ctrl+] · Ctrl+[ | Bring to front · send to back |
| ← / → (Shift: 10 frames) | Step one frame |
| Home / End | Start / end |
| + / − · Shift+Z | Zoom the timeline in / out · fit |
| N · M | Snapping · mute preview |
| Shift+M | Add a marker (over the marked range, if any) |
| Ctrl+0 · Ctrl+1 · Ctrl+wheel | Fit the preview · 100% · zoom the preview |
| Ctrl+Z · Ctrl+Shift+Z or Ctrl+Y | Undo · redo |
| Ctrl+N · Ctrl+O · Ctrl+S · Ctrl+Shift+S | New · open · save · save as |
| Ctrl+I · Ctrl+E · Ctrl+K · Ctrl+, | Import · export · assistant · settings |
| Esc | Close a dialog, cancel a rename, leave zoom/crop editing, clear the selection |

### Packaging

- **Windows**: `powershell -ExecutionPolicy Bypass -File scriptspackage-studio.ps1` (add `-Zip`
  for a .zip) builds `distAI Video Studio` — the app with its icon, bundled FFmpeg, icon files
  and licenses. Close the app running from that folder first.
- **Linux**: `scripts/package-studio.sh` builds `dist/ai-video-studio-linux-<arch>/` and a `.tar.gz`
  with the app, the icon in every size, a desktop entry and `install.sh` (installs to `~/.local`,
  or system-wide with `PREFIX=/usr/local`). FFmpeg comes from the system.

The icon is drawn from code (`studio/src/icon.rs`); `ai-video-studio --write-icons DIR` writes it
as PNGs and an `.ico`.

