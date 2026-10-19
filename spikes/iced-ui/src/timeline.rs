//! A canvas timeline with hundreds of clips: wheel scrolls, Ctrl + wheel zooms
//! around the cursor, dragging pans, clicking the ruler seeks.

use iced::keyboard;
use iced::mouse::{self, ScrollDelta};
use iced::widget::canvas::{self, Action, Cache, Frame, Geometry, Path, Stroke, Text};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme, alignment};

use crate::Message;
use crate::theme;

pub const RULER_H: f32 = 30.0;
pub const TRACK_H: f32 = 48.0;

pub struct Track {
    pub name: String,
    pub kind: &'static str,
    pub color: Color,
}

pub struct Clip {
    pub track: usize,
    pub start: f64,
    pub dur: f64,
    pub label: String,
}

pub struct Timeline {
    pub tracks: Vec<Track>,
    pub clips: Vec<Clip>,
    pub length: f64,
    cache: Cache,
}

impl Timeline {
    pub fn demo() -> Timeline {
        let tracks = vec![
            Track { name: "Titles".into(), kind: "Text", color: Color::from_rgb8(0x9B, 0x6B, 0xFF) },
            Track { name: "শিরোনাম (Bangla)".into(), kind: "Text", color: Color::from_rgb8(0xD9, 0x6B, 0xC8) },
            Track { name: "Screen recording".into(), kind: "Video", color: Color::from_rgb8(0x3E, 0x8E, 0xF7) },
            Track { name: "Face cam".into(), kind: "Video", color: Color::from_rgb8(0x2F, 0xB5, 0xA8) },
            Track { name: "Voiceover".into(), kind: "Audio", color: Color::from_rgb8(0x3D, 0xB8, 0x6E) },
            Track { name: "Sound effects".into(), kind: "Audio", color: Color::from_rgb8(0xE8, 0xA2, 0x3A) },
        ];
        let english = ["Intro", "Hero shot", "Feature zoom", "Dashboard", "Pricing", "Logo sting", "Call to action", "Whoosh", "Pop", "Riser"];
        let bangla = ["নতুন প্রোডাক্ট", "ক্ষমতা ও গতি", "স্মার্ট ড্যাশবোর্ড", "দাম মাত্র ৯৯৯", "শ্রেষ্ঠ অভিজ্ঞতা", "আজই শুরু করুন"];
        let mut clips = Vec::new();
        let mut seed = 7u64;
        let mut rand = move || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((seed >> 33) as f64) / (u32::MAX >> 1) as f64
        };
        for track in 0..tracks.len() {
            let mut t = rand() * 2.0;
            let mut n = 0;
            while t < 1800.0 {
                let dur = 2.0 + rand() * 9.0;
                let label = if track == 1 { bangla[n % bangla.len()].to_string() } else { english[(n + track) % english.len()].to_string() };
                clips.push(Clip { track, start: t, dur, label });
                t += dur + 0.4 + rand() * 3.0;
                n += 1;
            }
        }
        Timeline { tracks, clips, length: 1800.0, cache: Cache::new() }
    }

    pub fn content_height(&self) -> f32 {
        RULER_H + self.tracks.len() as f32 * TRACK_H + 6.0
    }

    pub fn invalidate(&self) {
        self.cache.clear();
    }

    pub fn canvas(&self, playhead: f64) -> TimelineCanvas<'_> {
        TimelineCanvas { timeline: self, playhead }
    }
}

pub struct TimelineCanvas<'a> {
    timeline: &'a Timeline,
    playhead: f64,
}

pub struct ViewState {
    scroll: f64,
    zoom: f64,
    modifiers: keyboard::Modifiers,
    drag: Option<(f32, f64)>,
}

impl Default for ViewState {
    fn default() -> Self {
        ViewState { scroll: 0.0, zoom: 36.0, modifiers: keyboard::Modifiers::default(), drag: None }
    }
}

impl canvas::Program<Message> for TimelineCanvas<'_> {
    type State = ViewState;

    fn update(&self, state: &mut ViewState, event: &canvas::Event, bounds: Rectangle, cursor: mouse::Cursor) -> Option<Action<Message>> {
        let changed = |state: &mut ViewState| {
            let visible = bounds.width as f64 / state.zoom;
            state.scroll = state.scroll.clamp(0.0, (self.timeline.length - visible * 0.5).max(0.0));
            self.timeline.invalidate();
            Some(Action::request_redraw().and_capture())
        };
        match event {
            canvas::Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => {
                state.modifiers = *m;
                None
            }
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let pos = cursor.position_in(bounds)?;
                let (dx, dy) = match *delta {
                    ScrollDelta::Lines { x, y } => (x * 48.0, y * 48.0),
                    ScrollDelta::Pixels { x, y } => (x, y),
                };
                if state.modifiers.control() || state.modifiers.command() {
                    let anchor = state.scroll + pos.x as f64 / state.zoom;
                    state.zoom = (state.zoom * 1.0025f64.powf(dy as f64)).clamp(1.0, 1200.0);
                    state.scroll = anchor - pos.x as f64 / state.zoom;
                } else {
                    state.scroll -= (dy + dx) as f64 / state.zoom;
                }
                changed(state)
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let pos = cursor.position_in(bounds)?;
                if pos.y < RULER_H {
                    let t = state.scroll + pos.x as f64 / state.zoom;
                    Some(Action::publish(Message::Seek(t)).and_capture())
                } else {
                    state.drag = Some((pos.x, state.scroll));
                    Some(Action::capture())
                }
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                let (x0, s0) = state.drag?;
                state.scroll = s0 - (position.x - bounds.x - x0) as f64 / state.zoom;
                changed(state)
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.drag.take().map(|_| Action::capture())
            }
            _ => None,
        }
    }

    fn draw(&self, state: &ViewState, renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let tl = self.timeline;
        let content = tl.cache.draw(renderer, bounds.size(), |frame| draw_content(frame, tl, state));
        let mut overlay = Frame::new(renderer, bounds.size());
        let x = ((self.playhead - state.scroll) * state.zoom) as f32;
        if (0.0..=bounds.width).contains(&x) {
            overlay.fill_rectangle(Point::new(x - 0.75, 0.0), Size::new(1.5, bounds.height), theme::ACCENT);
            overlay.fill(&Path::rounded_rectangle(Point::new(x - 6.0, 4.0), Size::new(12.0, 16.0), 4.0.into()), theme::ACCENT);
        }
        vec![content, overlay.into_geometry()]
    }

    fn mouse_interaction(&self, state: &ViewState, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        match cursor.position_in(bounds) {
            _ if state.drag.is_some() => mouse::Interaction::Grabbing,
            Some(p) if p.y < RULER_H => mouse::Interaction::Pointer,
            Some(_) => mouse::Interaction::Grab,
            None => mouse::Interaction::default(),
        }
    }
}

fn draw_content(frame: &mut Frame, tl: &Timeline, state: &ViewState) {
    let size = frame.size();
    frame.fill_rectangle(Point::ORIGIN, size, theme::PANEL);
    for i in 0..tl.tracks.len() {
        let y = RULER_H + i as f32 * TRACK_H;
        let shade = if i % 2 == 0 { theme::PANEL } else { Color::from_rgb8(0x17, 0x19, 0x20) };
        frame.fill_rectangle(Point::new(0.0, y), Size::new(size.width, TRACK_H), shade);
    }

    // Ruler: pick a label step that keeps labels at least ~90 px apart.
    frame.fill_rectangle(Point::ORIGIN, Size::new(size.width, RULER_H), theme::PANEL_2);
    frame.fill_rectangle(Point::new(0.0, RULER_H - 1.0), Size::new(size.width, 1.0), theme::BORDER);
    let steps = [0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0];
    let step = steps.iter().copied().find(|s| s * state.zoom >= 90.0).unwrap_or(600.0);
    let minor = step / 5.0;
    let first = (state.scroll / minor).floor() as i64;
    let last = ((state.scroll + size.width as f64 / state.zoom) / minor).ceil() as i64;
    for k in first..=last {
        let t = k as f64 * minor;
        let x = ((t - state.scroll) * state.zoom) as f32;
        let major = k % 5 == 0;
        let h = if major { 10.0 } else { 5.0 };
        frame.fill_rectangle(Point::new(x, RULER_H - 1.0 - h), Size::new(1.0, h), if major { theme::DIM } else { theme::FAINT });
        if major {
            frame.fill_text(Text {
                content: fmt_time(t, step < 1.0),
                position: Point::new(x + 4.0, 9.0),
                color: theme::DIM,
                size: 11.0.into(),
                font: theme::POPPINS,
                ..Text::default()
            });
        }
    }

    let (t0, t1) = (state.scroll, state.scroll + size.width as f64 / state.zoom);
    for clip in tl.clips.iter().filter(|c| c.start < t1 && c.start + c.dur > t0) {
        let track = &tl.tracks[clip.track];
        let x = ((clip.start - state.scroll) * state.zoom) as f32;
        let w = (clip.dur * state.zoom) as f32 - 2.0;
        let y = RULER_H + clip.track as f32 * TRACK_H + 6.0;
        let h = TRACK_H - 12.0;
        let r = 6.0f32.min(w / 2.0).max(0.0);
        let body = Path::rounded_rectangle(Point::new(x, y), Size::new(w.max(1.0), h), r.into());
        frame.fill(&body, Color { a: 0.82, ..track.color });
        frame.stroke(&body, Stroke::default().with_color(Color { a: 0.35, ..Color::WHITE }).with_width(1.0));
        if w > 28.0 {
            let bangla = !clip.label.is_ascii();
            // Clip regions keep absolute coordinates.
            frame.with_clip(Rectangle::new(Point::new(x + 8.0, y), Size::new(w - 14.0, h)), |f| {
                f.fill_text(Text {
                    content: clip.label.clone(),
                    position: Point::new(x + 8.0, y + h / 2.0),
                    color: Color::WHITE,
                    size: 12.5.into(),
                    font: if bangla { theme::HIND_SEMIBOLD } else { theme::POPPINS_MEDIUM },
                    align_y: alignment::Vertical::Center,
                    ..Text::default()
                });
            });
        }
    }
}

pub fn fmt_time(t: f64, fine: bool) -> String {
    let t = t.max(0.0);
    let m = (t / 60.0).floor();
    let s = t - m * 60.0;
    if fine { format!("{}:{:05.2}", m, s) } else { format!("{}:{:02}", m, s.floor() as u32) }
}
