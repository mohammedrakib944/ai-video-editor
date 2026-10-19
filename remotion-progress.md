# Remotion-style motion graphics — progress tracker

Long-term tracking file for this idea. Keep this file (not chat history) as the source of truth
across sessions — update the Log whenever the scope or status changes.

## What this is

Not an integration of the actual Remotion library (Node/React/Chromium) — that conflicts with
this project's lean, Rust-only, GNU-toolchain philosophy (see CLAUDE.md working rules). Instead,
the goal is to build the *techniques* that make Remotion output look good natively into this
editor's own rendering pipeline:

- Spring-based interpolation (not just the current `Ease::Linear/Smooth/EaseIn/EaseOut/Back` in
  `studio/src/clip.rs`)
- Richer, composable easing curves
- Composable/presetable motion templates (entrances, exits, camera moves) instead of the AI or
  the user hand-picking raw keyframe numbers every time

## Status: not started — idea stage

Raised in discussion on 2026-10-19; no design doc, no code changes yet.

## Relevant existing code (as of 2026-10-19)

- `studio/src/clip.rs:218` — `enum Ease { Linear, Smooth, EaseIn, EaseOut, Back }` and keyframe
  interpolation (`Keyframe.ease`)
- `studio/src/render/scene.rs::evaluate` — turns the project into `LayerSpec`s per frame; any new
  motion primitive has to flow through here (and `render/compositor.rs` + `layer.wgsl`) to render
  anywhere (preview and export share one renderer)
- `studio/src/assistant.rs:25` — `SYSTEM_PROMPT` already gives the AI motion/style guidance in
  prose; a preset library would give it (and manual editing) concrete building blocks instead

## Open questions / not yet decided

- Do we need true spring physics (mass/stiffness/damping) or is a curated set of easing curves +
  overshoot enough to get the "feel"?
- Where do motion presets live — new entries in `editor/library.rs`, or a new concept?
- Should presets be exposed as new MCP tools (`editor/tools.rs`) so the AI assistant calls them
  directly, instead of composing raw `set_keyframes` calls?

## Log

- 2026-10-19: Idea raised. Decided against a literal Remotion/Node integration. Captured as a
  long-term tracked item in this file instead of only in chat/memory. Related, separate discussion
  in the same session: baking a mandatory "professional by default" editing procedure into the AI
  assistant's system prompt (not yet started either — see chat history for that thread; worth its
  own tracking file if it grows).
- 2026-10-19 (later): Deliberately deprioritized this in favor of a cheaper, higher-leverage fix
  first — an `analyze_audio_beats` MCP tool (energy-based onset detection, no new dependency) plus
  a system-prompt rule so the AI syncs zooms/cuts/text to real beat times instead of guessing. That
  shipped and is tested (`studio/src/media.rs::detect_beats`, `editor/tools.rs::beat_source`,
  `assistant.rs` "Timing" rule). Also fixed an unrelated play/pause latency bug (`playback.rs`,
  `editor/mod.rs`) in the same session. This motion-preset/spring work is still untouched and is
  the natural next step once those are confirmed working in practice.
