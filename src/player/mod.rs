//! Player mode: what a file opened from Explorer ("Open with", a double-click, a file argument) gets.
//! A media player first, the editor only when asked for.
//!
//! Nothing of the editor exists here: no project, panes, timeline, Library / Gallery, caches, MCP
//! server, jobs, autosave or settings. `main` starts the decoders (`Engine::start`) before the window
//! exists, so the file opens and its first frame decodes while the window and GL come up. The window
//! repaints only when a frame is due or the controls change; paused with the controls hidden it does
//! nothing at all.
//!
//! The chrome (`chrome`) wears the editor's theme - Settings ▸ Appearance's theme, palette, Look and
//! accent, the one thing read from the editor's settings - on a media player's layout: a title strip
//! with an Editor button, a bottom bar over the video with the scrub bar, the timecode, previous - back
//! 10 s - play - forward 10 s - next between Reverse and Loop, then speed, tracks (audio tracks mixed,
//! any combination), volume, full screen, mini player and More. Esc or the
//! Editor button builds the editor in this process (`Root`), with the file on the timeline at the
//! current time.

pub mod chrome;
pub mod convert;
pub mod engine;
pub mod folder;
pub mod strip;
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

/// The skip buttons.
pub const SKIP_BACK: f64 = 10.0;
pub const SKIP_FORWARD: f64 = 10.0;

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

/// The speed / tracks / More flyouts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flyout {
    Speed,
    Volume,
    Tracks,
    More,
}

/// The editor's look, out of `settings.json` (the rest of it is skipped over unread).
#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct Look {
    theme: String,
    palette: crate::theme::PaletteOverride,
    ui_look: String,
    /// Audio Only's visualizer style, shared with the editor's (`Settings::visualizer_style`).
    visualizer_style: String,
}

/// Remembered between launches in `%APPDATA%\SimpleEditor\player.json` (a few bytes).
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
struct Prefs {
    volume: f32,
    muted: bool,
    shuffle: bool,
    repeat: u8,
    /// Audio tracks ticked per file (lower-case path); a file not listed plays all of them.
    // ponytail: grows by one entry per file whose tracks were changed; prune if it ever gets big
    audio: std::collections::HashMap<String, Vec<usize>>,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { volume: 1.0, muted: false, shuffle: false, repeat: 0, audio: Default::default() }
    }
}

fn file_key(p: &Path) -> String {
    p.to_string_lossy().to_lowercase()
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
    /// Settings ▸ Appearance's colours (`theme::palette_with` per frame, as the editor).
    pub(crate) look: crate::theme::PaletteOverride,
    /// Single-click play/pause waits out a possible double-click (full screen).
    pending_click: Option<f64>,
    ended: bool,
    /// The seek bar's hover thumbnails (`strip`), started once the file's info is in.
    pub(crate) strip: Option<strip::Strip>,
    /// The name being edited (bottom left; Enter renames the file, Esc cancels).
    pub(crate) renaming: Option<String>,
    /// The Convert… panel's choices while it is open, and the conversion running.
    pub(crate) convert: Option<convert::Spec>,
    pub(crate) convert_job: Option<convert::Job>,
    /// A message at the bottom: text, until (app clock), and a file it is about (Open / Open Folder /
    /// Add to Project).
    pub(crate) toast: Option<(String, f64, Option<PathBuf>)>,
    /// Audio Only's visualizer and its style (name, shader source).
    pub(crate) orb: crate::ui::spiky_ball::Orb,
    pub(crate) viz: (String, String),
    /// `SE_PLAYER_SHOT_SETUP=hover=0.8`: the seek bar acts hovered at that fraction (a posted mouse
    /// move cannot hover the window).
    pub(crate) shot_hover: Option<f32>,
}

impl PlayerApp {
    /// `engine` was started by `main` before the window existed.
    pub fn new(ctx: &egui::Context, engine: Engine) -> PlayerApp {
        crate::perf::startup_mark("player: window + GL ready");
        ctx.set_fonts(crate::theme::player_fonts());
        let look: Look = std::fs::read(crate::settings::Settings::path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        crate::theme::apply_visuals(ctx, &look.theme, &look.palette, &look.ui_look);
        let prefs: Prefs =
            std::fs::read(prefs_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        engine.set_volume(prefs.volume);
        engine.set_muted(prefs.muted);
        if let Some(v) = prefs.audio.get(&file_key(engine.path())) {
            engine.set_audio_streams(Some(v.clone()));
        }
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
            look: look.palette,
            pending_click: None,
            ended: false,
            strip: None,
            renaming: None,
            convert: None,
            convert_job: None,
            toast: None,
            orb: Default::default(),
            viz: (look.visualizer_style.clone(), String::new()),
            shot_hover: None,
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

    /// Add to Project: the editor takes over with `path` on the timeline.
    pub fn handoff_file(&mut self, path: PathBuf) {
        self.engine.set_playing(false);
        self.handoff = Some((path, 0.0));
    }

    pub fn take_handoff(&mut self) -> Option<(PathBuf, f64)> {
        self.handoff.take()
    }

    pub fn toggle_play(&mut self) {
        if self.ended && !self.engine.is_playing() {
            self.ended = false;
            // played out: again from the start (backwards: from the end)
            self.engine.seek(if self.engine.is_reverse() { self.engine.duration() } else { 0.0 });
        }
        self.engine.set_playing(!self.engine.is_playing());
    }

    /// Reverse: play backwards from here (playing), or forwards again.
    pub fn toggle_reverse(&mut self) {
        self.ended = false;
        self.engine.set_reverse(!self.engine.is_reverse());
        self.engine.set_playing(true);
    }

    pub fn looping(&self) -> bool {
        self.playlist.repeat == Repeat::One
    }

    /// Loop: the file repeats (both ways); remembered.
    pub fn set_loop(&mut self, on: bool) {
        self.playlist.repeat = if on { Repeat::One } else { Repeat::Off };
        self.prefs.repeat = self.playlist.repeat as u8;
        self.prefs_dirty = true;
    }

    /// Tick / untick audio track `n` (of `count`); the ticked ones are mixed. Remembered per file.
    pub fn set_audio_track(&mut self, n: usize, on: bool, count: usize) {
        let mut v = self.engine.audio_streams().unwrap_or_else(|| (0..count).collect());
        v.retain(|&m| m != n);
        if on {
            v.push(n);
            v.sort_unstable();
        }
        self.set_audio_selection(v, count);
    }

    /// Mix exactly these audio tracks (of `count`); none = no audio at all (nothing decoded or played,
    /// the clock runs off the system timer). Remembered per file.
    pub fn set_audio_selection(&mut self, v: Vec<usize>, count: usize) {
        let key = file_key(self.path());
        let all = v == (0..count).collect::<Vec<_>>();
        if all {
            self.prefs.audio.remove(&key);
        } else {
            self.prefs.audio.insert(key, v.clone());
        }
        self.prefs_dirty = true;
        self.engine.set_audio_streams((!all).then_some(v));
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
        self.open_at(ctx, path, 0.0, true);
    }

    /// Open `path` at `t`, playing or paused (a rename reopens the file where it was).
    pub fn open_at(&mut self, ctx: &egui::Context, path: &Path, t: f64, play: bool) {
        let (vol, muted, rate) = (self.engine.volume(), self.engine.muted(), self.engine.rate());
        let audio_only = self.engine.audio_only();
        self.engine = Engine::start(path, t, play, true);
        self.engine.set_audio_only(audio_only);
        self.engine.set_volume(vol);
        self.engine.set_muted(muted);
        self.engine.set_rate(rate);
        if let Some(v) = self.prefs.audio.get(&file_key(path)) {
            self.engine.set_audio_streams(Some(v.clone()));
        }
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
        self.strip = None;
        self.renaming = None;
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.title()));
    }

    pub(crate) fn say(&mut self, text: impl Into<String>, about: Option<PathBuf>) {
        let secs = if about.is_some() { 12.0 } else { 4.0 };
        self.toast = Some((text.into(), self.now() + secs, about));
    }

    /// Rename the file on disk to `stem` (its extension kept): the player lets go of it first (decoders,
    /// thumbnail jobs), then opens it again at the same moment.
    pub fn rename(&mut self, ctx: &egui::Context, stem: &str) {
        let from = self.path().to_path_buf();
        let to = match convert::rename_target(&from, stem) {
            Ok(to) if to == from => return,
            Ok(to) => to,
            Err(e) => return self.say(e, None),
        };
        let (t, play) = (self.engine.time(), self.engine.is_playing());
        self.strip = None;
        // a stand-in that opens nothing, so the old engine (and its file handles) goes now
        self.engine = Engine::start(Path::new(""), 0.0, false, false);
        let done = convert::rename_retrying(&from, &to);
        let back = if done.is_ok() { to.clone() } else { from.clone() };
        if let Some(v) = self.prefs.audio.remove(&file_key(&from)) {
            self.prefs.audio.insert(file_key(&back), v);
        }
        self.open_at(ctx, &back, t, play);
        self.playlist.scan(&back);
        if done.is_ok() {
            // an editor's Library holding this file relinks it (`media::renames`)
            crate::media::renames::record(&crate::media::renames::log_path(), &from, &to);
        }
        match done {
            Ok(()) => self.say(format!("Renamed to {}", to.file_name().unwrap_or_default().to_string_lossy()), None),
            Err(e) => self.say(format!("Couldn't rename: {e}"), None),
        }
    }

    /// Audio Only: no video decoded, the picture area shows the visualizer.
    pub fn set_audio_only(&mut self, on: bool) {
        self.engine.set_audio_only(on);
        if on {
            self.video.clear();
        }
    }

    /// Audio Only's visualizer style; saved as the editor's `visualizer_style` (one pick for both).
    pub fn set_visualizer(&mut self, name: &str) {
        self.viz = (name.to_string(), String::new());
        save_setting(&crate::settings::Settings::path(), "visualizer_style", serde_json::Value::String(name.into()));
    }

    /// Start converting with the panel's choices.
    pub fn start_convert(&mut self, spec: &convert::Spec) {
        if crate::media::ffpipe::ffmpeg_exe().is_none() {
            return self.say("Converting needs ffmpeg.exe", None);
        }
        let size = self.info.as_ref().map_or((0, 0), |i| i.size);
        let opts = convert::options(self.path(), spec, size, self.engine.audio_streams());
        self.convert_job = Some(convert::start(opts));
        self.convert = None;
    }

    /// The conversion finished (or failed): say so.
    fn poll_convert(&mut self) {
        let Some(j) = &self.convert_job else { return };
        if !j.progress.is_done() {
            return;
        }
        let (err, out) = (j.progress.error(), j.out.clone());
        self.convert_job = None;
        match err {
            None => {
                let shown = if crate::engine::convert::is_sequence(&out) { out.parent().map(Path::to_path_buf) } else { Some(out.clone()) };
                let name = shown.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.say(format!("Converted: {name}"), Some(out));
            }
            Some(e) if e == crate::engine::export::CANCELLED => self.say("Convert cancelled", None),
            Some(e) => self.say(format!("Convert failed: {e}"), None),
        }
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

    /// Repeat the folder (on) or not.
    pub fn set_repeat_folder(&mut self, on: bool) {
        self.playlist.repeat = if on { Repeat::All } else { Repeat::Off };
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
        self.poll_convert();
        if self.convert_job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        if self.toast.as_ref().is_some_and(|t| t.1 < self.now()) {
            self.toast = None;
        }
        if let Some(s) = &self.strip {
            s.set_playing(self.engine.is_playing());
        }
        if self.info.is_none() && self.error.is_none() {
            match self.engine.info() {
                Some(Ok(i)) => {
                    if !i.has_video {
                        self.start_cover(ctx);
                    } else if i.duration > 0.0 {
                        self.strip = Some(strip::Strip::start(&i.path, i.duration));
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
        // (backwards the next frame is due below `t`)
        Some(((next - t).abs() / self.engine.rate()).max(0.0))
    }

    /// The file played out: repeat / next per the playlist, else stop on the last frame.
    fn handle_end(&mut self, ctx: &egui::Context) {
        if !self.engine.is_playing() || !self.engine.at_end() || self.scrub.is_some() {
            return;
        }
        if self.engine.is_reverse() {
            // back at the start: Loop goes round from the end, else stop on the first frame
            if self.looping() {
                self.seek(self.engine.duration());
            } else {
                self.engine.set_playing(false);
                self.ended = true;
                self.hide.poke(self.now());
            }
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

    /// `SE_PLAYER_SHOT_SETUP=at=20,reverse,loop,rate=2,flyout=tracks` (visual checks, with `SE_PLAYER_SHOT`):
    /// the state to picture, set once the window shows - the player has no MCP to set it with.
    fn shot_setup(&mut self) {
        let Some(spec) = self.shot.as_ref().and(std::env::var("SE_PLAYER_SHOT_SETUP").ok()) else { return };
        for item in spec.split(',').map(str::trim) {
            match item.split_once('=') {
                Some(("rate", r)) => self.engine.set_rate(r.parse().unwrap_or(1.0)),
                Some(("at", t)) => self.seek(t.parse().unwrap_or(0.0)),
                Some(("flyout", "speed")) => self.flyout = Some(Flyout::Speed),
                Some(("flyout", "tracks")) => {
                    self.flyout = Some(Flyout::Tracks);
                    self.tracks();
                }
                Some(("flyout", "more")) => self.flyout = Some(Flyout::More),
                Some(("flyout", "volume")) => self.flyout = Some(Flyout::Volume),
                Some(("hover", k)) => self.shot_hover = k.parse().ok(),
                _ if item == "audio_only" => self.set_audio_only(true),
                _ if item == "pause" => self.engine.set_playing(false),
                _ if item == "convert" => self.convert = Some(Default::default()),
                _ if item == "rename" => self.renaming = Some(self.title()),
                _ if item == "reverse" => self.toggle_reverse(),
                _ if item == "loop" => self.playlist.repeat = Repeat::One,
                _ => {}
            }
        }
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
            self.shot_setup();
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

/// Set one key of the editor's `settings.json`, leaving every other key as it is.
fn save_setting(p: &Path, key: &str, v: serde_json::Value) {
    let mut j: serde_json::Value = std::fs::read(&p).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(|| serde_json::json!({}));
    if let Some(o) = j.as_object_mut() {
        o.insert(key.into(), v);
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(b) = serde_json::to_vec_pretty(&j) {
            let _ = std::fs::write(p, b);
        }
    }
}
