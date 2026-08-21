//! Player: owns the render thread (Compositor + DecoderPool → latest Frame) and the audio thread
//! (Mixer + DecoderPool → ring buffer → cpal WASAPI output). The wall clock is the master when playing:
//! time = base_time + elapsed. Video frames are rendered at the project fps and published via
//! `take_frame`; `ctx.request_repaint()` is called whenever a new frame is ready.
//!
//! Commands (mpsc from the UI thread): SetProject(Arc<Project>), Seek(t), Play, Pause, Canvas(w,h),
//! Backend, ClearDecoders(ack). While paused, every Seek/SetProject/Canvas re-renders the current time
//! (latest request wins — drain the queue before rendering). Audio thread mirrors the clock: when playing
//! it keeps ~100 ms of mixed audio ahead of the clock in the ring; seek/pause flush the ring.
//! Reaching the project end pauses (the UI sees is_playing() flip).

use crate::engine::text::TextRasterizer;
use crate::media::{Backend, Frame};
use crate::model::Project;
use std::sync::{Arc, Mutex};

pub struct Player {
    #[allow(dead_code)]
    ctx: eframe::egui::Context,
    #[allow(dead_code)]
    text: Arc<Mutex<TextRasterizer>>,
    #[allow(dead_code)]
    backend: Backend,
    time: f64,
    playing: bool,
    duration: f64,
}

impl Player {
    pub fn new(ctx: eframe::egui::Context, backend: Backend, text: Arc<Mutex<TextRasterizer>>) -> Self {
        Self { ctx, text, backend, time: 0.0, playing: false, duration: 0.0 }
    }
    /// Replace the project (cheap clone; called after every edit). Re-renders the current frame.
    pub fn set_project(&mut self, project: &Project) {
        self.duration = project.duration();
        todo!("playback::Player::set_project")
    }
    pub fn set_backend(&mut self, _b: Backend) {
        todo!("playback::Player::set_backend")
    }
    /// Preview canvas size in pixels (the Player clamps width to `max_width`, keeping aspect).
    pub fn set_canvas(&mut self, _w: u32, _h: u32, _max_width: u32) {
        todo!("playback::Player::set_canvas")
    }
    pub fn play(&mut self) {
        self.playing = true;
        todo!("playback::Player::play")
    }
    pub fn pause(&mut self) {
        self.playing = false;
        todo!("playback::Player::pause")
    }
    pub fn toggle(&mut self) {
        if self.playing {
            self.pause()
        } else {
            self.play()
        }
    }
    pub fn is_playing(&self) -> bool {
        self.playing
    }
    /// Current timeline time (advances while playing).
    pub fn time(&self) -> f64 {
        self.time
    }
    /// Clamp to [0, duration] and re-render if paused.
    pub fn seek(&mut self, t: f64) {
        self.time = t.clamp(0.0, self.duration.max(0.0));
        todo!("playback::Player::seek")
    }
    /// Newest rendered frame not yet taken (UI uploads it to a texture).
    pub fn take_frame(&mut self) -> Option<Arc<Frame>> {
        todo!("playback::Player::take_frame")
    }
    /// Synchronously drop every decoder on both threads (before overwriting a source file).
    pub fn release_files(&mut self) {
        todo!("playback::Player::release_files")
    }
    pub fn duration(&self) -> f64 {
        self.duration
    }
}
