//! Player mode: what a file opened from Explorer ("Open with", a double-click, a file argument) gets.
//! A media player first, the editor only when asked for.
//!
//! Nothing of the editor exists here: no project, panes, timeline, Library / Gallery, caches, MCP
//! server, jobs, autosave or settings. `main` starts the decoders (`Engine::start`) before the window
//! exists, so the file opens and its first frame decodes while the window and GL come up. The window
//! repaints only when a frame is due or the controls change; paused with the controls hidden it does
//! nothing at all.
//!
//! The chrome follows Windows Media Player (`chrome`): an Editor button where Media Player has Home,
//! the scrubber with elapsed / total, shuffle - previous - back 10 s - play - forward 30 s - next -
//! repeat, then tracks, volume, full screen, mini player and the ⋯ menu. Esc or the Editor button
//! builds the editor in this process (`Root`), with the file on the timeline at the current time.

pub mod chrome;
pub mod engine;
pub mod folder;
pub mod tracks;
pub mod video;

use crate::ui::autohide::{self, AutoHide};
use eframe::egui;
use engine::{Engine, Info};
use folder::{Playlist, Repeat};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracks::Tracks;

/// Skip buttons, as Media Player's.
pub const SKIP_BACK: f64 = 10.0;
pub const SKIP_FORWARD: f64 = 30.0;

/// How the picture fills the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Zoom {
    #[default]
    Fit,
    Fill,
    Stretch,
    Original,
}

impl Zoom {
    pub const ALL: [Zoom; 4] = [Zoom::Fit, Zoom::Fill, Zoom::Stretch, Zoom::Original];
    pub fn label(self) -> &'static str {
        match self {
            Zoom::Fit => "Fit",
            Zoom::Fill => "Fill",
            Zoom::Stretch => "Stretch",
            Zoom::Original => "Original size",
        }
    }
    /// Where a `size` picture goes inside `area` (points), with `ppp` physical pixels per point.
    pub fn place(self, area: egui::Rect, size: (u32, u32), ppp: f32) -> egui::Rect {
        let (w, h) = (size.0.max(1) as f32, size.1.max(1) as f32);
        let s = match self {
            Zoom::Fit => (area.width() / w).min(area.height() / h),
            Zoom::Fill => (area.width() / w).max(area.height() / h),
            Zoom::Stretch => return area,
            Zoom::Original => 1.0 / ppp,
        };
        egui::Rect::from_center_size(area.center(), egui::vec2(w * s, h * s))
    }
}

/// The ⋯ / tracks / volume flyouts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flyout {
    Tracks,
    Volume,
    More,
}

/// Remembered between launches in `%APPDATA%\SimpleEditor\player.json` (a few bytes; the editor's
/// settings file is never read in player mode).
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
struct Prefs {
    volume: f32,
    muted: bool,
    shuffle: bool,
    repeat: u8,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { volume: 1.0, muted: false, shuffle: false, repeat: 0 }
    }
}

fn prefs_path() -> PathBuf {
    crate::settings::Settings::dir().join("player.json")
}

/// The window before the mini player took it over.
#[derive(Clone, Copy, Debug)]
struct Restore {
    maximized: bool,
    fullscreen: bool,
    rect: Option<egui::Rect>,
}

pub struct PlayerApp {
    pub(crate) engine: Engine,
    pub(crate) info: Option<Info>,
    pub(crate) error: Option<String>,
    pub(crate) video: video::VideoTex,
    hide: AutoHide,
    clock0: Instant,
    pub(crate) playlist: Playlist,
    pub(crate) tracks: Option<Tracks>,
    tracks_rx: Option<Receiver<Tracks>>,
    /// The subtitle track switched on and its cues (loading: cues None).
    pub(crate) subtitle: Option<(usize, Option<Vec<(f64, f64, String)>>)>,
    subs_rx: Option<Receiver<(usize, Vec<(f64, f64, String)>)>>,
    pub(crate) flyout: Option<Flyout>,
    pub(crate) zoom: Zoom,
    pub(crate) info_open: bool,
    /// The scrubber is being dragged: (time under the pointer, when the last seek went out).
    pub(crate) scrub: Option<(f64, Instant)>,
    pub(crate) mini: Option<Restore>,
    pub(crate) fullscreen: bool,
    prefs: Prefs,
    prefs_dirty: bool,
    /// Esc / Editor button: (file, time) for `Root` to build the editor with.
    handoff: Option<(PathBuf, f64)>,
    window_shown: bool,
    /// Process start -> first frame on screen (`SE_BENCH_FIRST_FRAME`).
    pub(crate) first_present: Option<Duration>,
    bench: Option<PathBuf>,
    /// `SE_PLAYER_SHOT=<png>[|<seconds>]` (visual checks): save the window `seconds` (default 1) after
    /// it is revealed, then close. The player has no MCP, and screen capture sees no GL swap chain.
    shot: Option<(PathBuf, f64, bool)>,
    shown_at_frame: u64,
    /// Opening without a flash (`winpos::Reveal`).
    reveal: crate::winpos::Reveal,
    frames: u64,
    pub(crate) cover: Option<egui::TextureHandle>,
    cover_rx: Option<Receiver<Option<egui::ColorImage>>>,
    pub(crate) bars: Vec<f32>,
    pub(crate) icon: Option<egui::TextureHandle>,
    /// Single-click play/pause waits out a possible double-click (full screen).
    pending_click: Option<f64>,
    ended: bool,
}

impl PlayerApp {
    /// `engine` was started by `main` before the window existed.
    pub fn new(ctx: &egui::Context, engine: Engine) -> PlayerApp {
        crate::perf::startup_mark("player: window + GL ready");
        ctx.set_fonts(crate::theme::player_fonts());
        ctx.set_theme(egui::ThemePreference::Dark);
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        let prefs: Prefs =
            std::fs::read(prefs_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        engine.set_volume(prefs.volume);
        engine.set_muted(prefs.muted);
        let repeat = match prefs.repeat {
            1 => Repeat::All,
            2 => Repeat::One,
            _ => Repeat::Off,
        };
        let c = ctx.clone();
        engine.set_wake(Arc::new(move || c.request_repaint()));
        PlayerApp {
            playlist: Playlist::new(prefs.shuffle, repeat),
            engine,
            info: None,
            error: None,
            video: Default::default(),
            hide: AutoHide::default(),
            clock0: Instant::now(),
            tracks: None,
            tracks_rx: None,
            subtitle: None,
            subs_rx: None,
            flyout: None,
            zoom: Zoom::Fit,
            info_open: false,
            scrub: None,
            mini: None,
            fullscreen: false,
            prefs,
            prefs_dirty: false,
            handoff: None,
            window_shown: false,
            first_present: None,
            bench: std::env::var_os("SE_BENCH_FIRST_FRAME").map(PathBuf::from),
            shot: std::env::var("SE_PLAYER_SHOT").ok().map(|v| {
                let (p, s) = v.split_once('|').unwrap_or((&v, "1"));
                (PathBuf::from(p), s.parse().unwrap_or(1.0), false)
            }),
            shown_at_frame: 0,
            reveal: Default::default(),
            frames: 0,
            cover: None,
            cover_rx: None,
            bars: Vec::new(),
            icon: None,
            pending_click: None,
            ended: false,
        }
    }

    pub fn path(&self) -> &Path {
        self.engine.path()
    }

    pub fn title(&self) -> String {
        self.path().file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    }

    fn now(&self) -> f64 {
        self.clock0.elapsed().as_secs_f64()
    }

    /// Esc / the Editor button: the editor takes over the window with this file at this time.
    pub fn open_editor(&mut self) {
        let t = self.engine.time();
        self.engine.set_playing(false);
        self.handoff = Some((self.path().to_path_buf(), t));
    }

    pub fn take_handoff(&mut self) -> Option<(PathBuf, f64)> {
        self.handoff.take()
    }

    pub fn toggle_play(&mut self) {
        if self.ended && !self.engine.is_playing() {
            self.ended = false;
            self.engine.seek(0.0);
        }
        self.engine.set_playing(!self.engine.is_playing());
    }

    pub fn skip(&mut self, by: f64) {
        self.ended = false;
        self.engine.skip(by);
    }

    pub fn seek(&mut self, t: f64) {
        self.ended = false;
        self.engine.seek(t);
    }

    /// Open another file in place (previous / next / repeat), playing.
    pub fn open(&mut self, ctx: &egui::Context, path: &Path) {
        let (vol, muted, rate) = (self.engine.volume(), self.engine.muted(), self.engine.rate());
        self.engine = Engine::start(path, 0.0, true, true);
        self.engine.set_volume(vol);
        self.engine.set_muted(muted);
        self.engine.set_rate(rate);
        let c = ctx.clone();
        self.engine.set_wake(Arc::new(move || c.request_repaint()));
        self.info = None;
        self.error = None;
        self.video.clear();
        self.tracks = None;
        self.tracks_rx = None;
        self.subtitle = None;
        self.subs_rx = None;
        self.cover = None;
        self.cover_rx = None;
        self.bars.clear();
        self.ended = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.title()));
    }

    pub fn next(&mut self, ctx: &egui::Context) {
        let cur = self.path().to_path_buf();
        if let Some(p) = self.playlist.next(&cur) {
            self.open(ctx, &p);
        }
    }

    /// Media Player's Previous: back to the start when more than 3 s in, else the previous file.
    pub fn prev(&mut self, ctx: &egui::Context) {
        if self.engine.time() > 3.0 {
            self.seek(0.0);
            return;
        }
        let cur = self.path().to_path_buf();
        if let Some(p) = self.playlist.prev(&cur) {
            self.open(ctx, &p);
        }
    }

    pub fn set_shuffle(&mut self, on: bool) {
        let cur = self.path().to_path_buf();
        self.playlist.set_shuffle(on, &cur);
        self.prefs.shuffle = on;
        self.prefs_dirty = true;
    }

    pub fn cycle_repeat(&mut self) {
        self.playlist.repeat = self.playlist.repeat.next();
        self.prefs.repeat = self.playlist.repeat as u8;
        self.prefs_dirty = true;
    }

    pub fn set_volume(&mut self, v: f32) {
        self.engine.set_volume(v);
        if v > 0.0 {
            self.engine.set_muted(false);
        }
        self.prefs.volume = self.engine.volume();
        self.prefs.muted = self.engine.muted();
        self.prefs_dirty = true;
    }

    pub fn toggle_mute(&mut self) {
        self.engine.set_muted(!self.engine.muted());
        self.prefs.muted = self.engine.muted();
        self.prefs_dirty = true;
    }

    /// The tracks, listed (ffprobe, off the UI thread) the first time anything asks.
    pub fn tracks(&mut self) -> Option<&Tracks> {
        if self.tracks.is_none() && self.tracks_rx.is_none() {
            let (tx, rx) = std::sync::mpsc::channel();
            let p = self.path().to_path_buf();
            std::thread::spawn(move || {
                let _ = tx.send(tracks::probe(&p));
            });
            self.tracks_rx = Some(rx);
        }
        self.tracks.as_ref()
    }

    /// Switch subtitle track `n` on (None = off); its cues are pulled out in the background.
    pub fn set_subtitle(&mut self, n: Option<usize>) {
        self.subs_rx = None;
        self.subtitle = None;
        let Some(n) = n else { return };
        let Some(track) = self.tracks.as_ref().and_then(|t| t.subtitles.get(n)).cloned() else { return };
        self.subtitle = Some((n, None));
        let (tx, rx) = std::sync::mpsc::channel();
        let p = self.path().to_path_buf();
        std::thread::spawn(move || {
            let _ = tx.send((n, tracks::cues(&p, &track).unwrap_or_default()));
        });
        self.subs_rx = Some(rx);
    }

    pub fn toggle_fullscreen(&mut self, ctx: &egui::Context) {
        if self.mini.is_some() {
            self.toggle_mini(ctx);
        }
        self.fullscreen = !self.fullscreen;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
    }

    /// Mini player: a small always-on-top window in the corner of the screen; again to restore.
    pub fn toggle_mini(&mut self, ctx: &egui::Context) {
        use egui::ViewportCommand as V;
        if let Some(r) = self.mini.take() {
            ctx.send_viewport_cmd(V::WindowLevel(egui::WindowLevel::Normal));
            if r.fullscreen {
                self.fullscreen = true;
                ctx.send_viewport_cmd(V::Fullscreen(true));
            } else if r.maximized {
                ctx.send_viewport_cmd(V::Maximized(true));
            } else if let Some(rect) = r.rect {
                ctx.send_viewport_cmd(V::OuterPosition(rect.min));
                ctx.send_viewport_cmd(V::InnerSize(rect.size()));
            }
            return;
        }
        let vp = ctx.input(|i| i.viewport().clone());
        self.mini = Some(Restore {
            maximized: vp.maximized.unwrap_or(false),
            fullscreen: self.fullscreen,
            rect: vp.outer_rect,
        });
        if self.fullscreen {
            self.fullscreen = false;
            ctx.send_viewport_cmd(V::Fullscreen(false));
        }
        ctx.send_viewport_cmd(V::Maximized(false));
        let aspect = self.info.as_ref().filter(|i| i.has_video && i.size.1 > 0).map_or(16.0 / 9.0, |i| i.size.0 as f32 / i.size.1 as f32);
        let size = egui::vec2(400.0, (400.0 / aspect).clamp(140.0, 400.0));
        ctx.send_viewport_cmd(V::InnerSize(size));
        if let Some(m) = vp.monitor_size {
            ctx.send_viewport_cmd(V::OuterPosition(egui::pos2(m.x - size.x - 24.0, m.y - size.y - 72.0)));
        }
        ctx.send_viewport_cmd(V::WindowLevel(egui::WindowLevel::AlwaysOnTop));
    }

    pub fn open_folder(&self) {
        let _ = std::process::Command::new("explorer.exe").arg(format!("/select,{}", self.path().display())).spawn();
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if self.info.is_none() && self.error.is_none() {
            match self.engine.info() {
                Some(Ok(i)) => {
                    if !i.has_video {
                        self.start_cover(ctx);
                    }
                    self.info = Some(i);
                }
                Some(Err(e)) => self.error = Some(e),
                None => {}
            }
        }
        if let Some(rx) = &self.tracks_rx {
            if let Ok(t) = rx.try_recv() {
                self.tracks = Some(t);
                self.tracks_rx = None;
            }
        }
        if let Some(rx) = &self.subs_rx {
            if let Ok((n, cues)) = rx.try_recv() {
                if self.subtitle.as_ref().is_some_and(|(m, _)| *m == n) {
                    self.subtitle = Some((n, Some(cues)));
                }
                self.subs_rx = None;
            }
        }
        if let Some(rx) = &self.cover_rx {
            if let Ok(img) = rx.try_recv() {
                self.cover = img.map(|img| ctx.load_texture("player-cover", img, egui::TextureOptions::LINEAR));
                self.cover_rx = None;
            }
        }
        if self.tracks_rx.is_some() || self.subs_rx.is_some() || self.cover_rx.is_some() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }

    /// Audio files: the embedded cover (or cover.jpg / folder.jpg beside it), with ffmpeg, off-thread.
    fn start_cover(&mut self, ctx: &egui::Context) {
        let (tx, rx) = std::sync::mpsc::channel();
        let p = self.path().to_path_buf();
        let c = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(chrome::cover_art(&p));
            c.request_repaint();
        });
        self.cover_rx = Some(rx);
    }

    /// Take the frame due now. Returns when the next one is due (seconds), if playing video.
    fn advance(&mut self, ctx: &egui::Context, frame: Option<&mut eframe::Frame>) -> Option<f64> {
        let t = self.engine.time();
        if let Some(f) = self.engine.frame_due(t) {
            let gl = frame.as_ref().and_then(|f| f.gl().cloned());
            self.video.show(ctx, &f, gl.as_ref(), frame);
        }
        if !self.engine.is_playing() {
            return None;
        }
        let next = self.engine.next_due()?;
        Some(((next - t) / self.engine.rate()).max(0.0))
    }

    /// The file played out: repeat / next per the playlist, else stop on the last frame.
    fn handle_end(&mut self, ctx: &egui::Context) {
        if !self.engine.is_playing() || !self.engine.at_end() || self.scrub.is_some() {
            return;
        }
        let cur = self.path().to_path_buf();
        match self.playlist.after_end(&cur) {
            Some(p) if p == cur => self.seek(0.0),
            Some(p) => self.open(ctx, &p),
            None => {
                self.engine.set_playing(false);
                self.ended = true;
                self.hide.poke(self.now());
            }
        }
    }

    fn keys(&mut self, ctx: &egui::Context) {
        use egui::{Key, Modifiers};
        if ctx.memory(|m| m.focused().is_some()) {
            return;
        }
        let (ctrl, none) = (Modifiers::CTRL, Modifiers::NONE);
        let k = |m: Modifiers, key: Key| ctx.input_mut(|i| i.consume_key(m, key));
        let mut poked = true;
        if k(none, Key::Space) || k(none, Key::K) {
            self.toggle_play();
        } else if k(ctrl, Key::ArrowLeft) {
            self.prev(ctx);
        } else if k(ctrl, Key::ArrowRight) {
            self.next(ctx);
        } else if k(none, Key::ArrowLeft) || k(none, Key::J) {
            self.skip(-SKIP_BACK);
        } else if k(none, Key::ArrowRight) {
            self.skip(SKIP_FORWARD);
        } else if k(none, Key::L) {
            self.skip(SKIP_BACK);
        } else if k(none, Key::ArrowUp) {
            self.set_volume(self.engine.volume() + 0.05);
        } else if k(none, Key::ArrowDown) {
            self.set_volume(self.engine.volume() - 0.05);
        } else if k(none, Key::M) {
            self.toggle_mute();
        } else if k(none, Key::F) || k(none, Key::F11) {
            self.toggle_fullscreen(ctx);
        } else if k(none, Key::Escape) {
            if self.flyout.is_some() || self.info_open {
                self.flyout = None;
                self.info_open = false;
            } else if self.fullscreen {
                self.toggle_fullscreen(ctx);
            } else if self.mini.is_some() {
                self.toggle_mini(ctx);
            } else {
                self.open_editor();
            }
        } else {
            poked = false;
        }
        if poked {
            self.hide.poke(self.now());
        }
    }

    /// The whole frame: everything but the window/GL plumbing (`update`), so a headless test can
    /// drive it with a bare `egui::Context`.
    pub fn frame(&mut self, ctx: &egui::Context, mut frame: Option<&mut eframe::Frame>) {
        self.frames += 1;
        self.poll(ctx);
        let next_frame_in = self.advance(ctx, frame.as_deref_mut());
        self.handle_end(ctx);
        self.keys(ctx);
        let now = self.now();
        if let Some(at) = self.pending_click {
            if now - at >= 0.25 {
                self.pending_click = None;
                self.toggle_play();
            } else {
                ctx.request_repaint_after(Duration::from_secs_f64(0.25 - (now - at)));
            }
        }
        let out = chrome::draw(self, ctx, now);
        if let Some(s) = out.repaint_in {
            ctx.request_repaint_after(Duration::from_secs_f64(s));
        }
        if let Some(s) = next_frame_in {
            ctx.request_repaint_after(Duration::from_secs_f64(s));
        }
        // audio only: the visualizer and the clock tick at 30 Hz while playing
        if self.engine.is_playing() && self.info.as_ref().is_some_and(|i| !i.has_video) {
            ctx.request_repaint_after(Duration::from_millis(33));
        }
    }

    pub(crate) fn click_video(&mut self, now: f64) {
        self.pending_click = Some(now);
    }

    fn shot_tick(&mut self, ctx: &egui::Context) {
        let Some((path, after, asked)) = self.shot.as_mut() else { return };
        if !self.window_shown {
            return;
        }
        let image = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = image {
            let [w, h] = img.size;
            let rgba = img.pixels.iter().flat_map(|c| c.to_array()).collect();
            let f = crate::media::Frame { width: w as u32, height: h as u32, rgba, ..Default::default() };
            let _ = std::fs::write(&*path, crate::mcp::png_encode(&f));
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if !*asked && self.engine.started.elapsed().as_secs_f64() >= *after {
            *asked = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        ctx.request_repaint_after(Duration::from_millis(30));
    }

    /// The click on the video only hid the controls (`autohide::Output::dismissed`): no play/pause.
    pub(crate) fn cancel_click(&mut self) {
        self.pending_click = None;
    }

    pub(crate) fn double_click_video(&mut self, ctx: &egui::Context) {
        self.pending_click = None;
        self.toggle_fullscreen(ctx);
    }

    pub(crate) fn autohide(&mut self, i: autohide::Input) -> autohide::Output {
        self.hide.step(i)
    }

    fn save_prefs(&mut self) {
        if std::mem::take(&mut self.prefs_dirty) {
            let p = prefs_path();
            if let Some(d) = p.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            if let Ok(b) = serde_json::to_vec(&self.prefs) {
                let _ = std::fs::write(p, b);
            }
        }
    }
}

impl eframe::App for PlayerApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if self.frames < 3 {
            crate::perf::startup_mark(&format!("player: update {}", self.frames + 1));
        }
        if !self.window_shown {
            // the first frame is usually decoded already (started before the window); give it a moment
            self.engine.wait_first(Duration::from_millis(250));
        }
        self.frame(ctx, Some(frame));
        let has_picture = self.video.shown.is_some() || self.info.as_ref().is_some_and(|i| !i.has_video) || self.error.is_some();
        // shown cloaked at once (maximised), revealed once a frame with the picture is on screen at
        // that size (`winpos::Reveal`) - or after 1.5 s, whatever it shows
        let ready = has_picture || self.engine.started.elapsed() > Duration::from_millis(1500);
        let just = if self.reveal.started() && !ready {
            ctx.request_repaint_after(Duration::from_millis(5));
            false
        } else {
            self.reveal.step(ctx, frame, !crate::winpos::background())
        };
        self.shot_tick(ctx);
        if just {
            // the clock starts with the picture: no frames skipped, no sound before the window
            self.engine.set_playing(true);
            self.window_shown = true;
            self.shown_at_frame = self.frames;
            ctx.request_repaint(); // the next update is the first one after a present
        } else if self.window_shown && self.first_present.is_none() && self.frames > self.shown_at_frame {
            if has_picture {
                let d = crate::perf::since_process_start();
                self.first_present = Some(d);
                #[cfg(debug_assertions)]
                eprintln!("player: first frame on screen {} ms after process start", d.as_millis());
                if let Some(b) = self.bench.take() {
                    let _ = std::fs::write(b, format!("{}", d.as_secs_f64() * 1000.0));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            } else {
                ctx.request_repaint_after(Duration::from_millis(5));
            }
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 1.0]
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_prefs();
    }
}

/// The app eframe runs: the player until the editor is asked for, then the editor - in the same
/// window and process. The editor is built at that moment, not before.
pub enum Root {
    Player(Box<PlayerApp>),
    Editor(Box<crate::ui::app::App>),
}

impl Root {
    fn hand_off(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let Root::Player(p) = self else { return };
        let Some((path, t)) = p.take_handoff() else { return };
        p.save_prefs();
        use egui::ViewportCommand as V;
        if p.fullscreen {
            ctx.send_viewport_cmd(V::Fullscreen(false));
        }
        if p.mini.is_some() {
            ctx.send_viewport_cmd(V::WindowLevel(egui::WindowLevel::Normal));
            ctx.send_viewport_cmd(V::Maximized(true));
        }
        ctx.send_viewport_cmd(V::Decorations(true));
        ctx.send_viewport_cmd(V::MinInnerSize(egui::vec2(900.0, 560.0)));
        let app = crate::ui::app::App::from_player(ctx, frame.gl().cloned(), &path, t);
        *self = Root::Editor(Box::new(app));
        ctx.request_repaint();
    }
}

impl eframe::App for Root {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        match self {
            Root::Player(p) => p.update(ctx, frame),
            Root::Editor(e) => e.update(ctx, frame),
        }
        self.hand_off(ctx, frame);
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if let Root::Editor(e) = self {
            e.raw_input_hook(ctx, raw_input);
        }
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        match self {
            Root::Player(p) => p.clear_color(visuals),
            Root::Editor(e) => e.clear_color(visuals),
        }
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        match self {
            Root::Player(p) => p.on_exit(gl),
            Root::Editor(e) => e.on_exit(gl),
        }
    }
}

#[cfg(test)]
mod tests;
