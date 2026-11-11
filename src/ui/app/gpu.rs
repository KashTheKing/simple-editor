use super::thumbs::*;
use super::*;

// ---- ws:export-pipeline ----
/// An export frame's reply channel and the buffer its pixels go into.
type ExportReply = (std::sync::mpsc::SyncSender<Option<Frame>>, Frame);

/// Render one export request: queue its readback in the renderer's pixel-pack ring (finishing the
/// oldest first when the ring is full), or - when the ring cannot take it - read it back right away.
fn serve_export_request(
    g: &mut GpuRenderer,
    project: &crate::model::Project,
    req: crate::engine::export::GpuFrameRequest,
    inflight: &mut std::collections::VecDeque<ExportReply>,
) {
    let crate::engine::export::GpuFrameRequest { layers, t, w, h, nv12, mut buf, reply } = req;
    // `inflight` mirrors the renderer's pending queue one for one
    if g.export_pending() >= crate::engine::gpu::READBACK_RING {
        if let Some((r, mut b)) = inflight.pop_front() {
            g.render_export_finish(&mut b);
            let _ = r.send(Some(b));
        }
    }
    if g.render_export_begin(project, t, w, h, &layers, nv12) {
        inflight.push_back((reply, buf));
    } else {
        // keep replies in order: everything queued before this one goes first
        while let Some((r, mut b)) = inflight.pop_front() {
            g.render_export_finish(&mut b);
            let _ = r.send(Some(b));
        }
        // a failed render has no picture (zeros = green in NV12): None finishes this export on the CPU
        let ok = g.render_export_sync(project, t, w, h, &layers, nv12, &mut buf);
        let _ = reply.send(ok.then_some(buf));
    }
}

impl App {
    pub(super) fn sync_gpu(&mut self) {
        let want = self.settings.gpu && !self.gpu_failed;
        if want == self.gpu.is_some() {
            return;
        }
        if !want {
            self.gpu = None;
            self.set_players_gpu(false);
            return;
        }
        let Some(gl) = self.gl.clone() else {
            self.gpu_failed = true; // no GL context at all (software / headless run)
            self.set_players_gpu(false);
            return;
        };
        match guarded(|| GpuRenderer::new(gl)) {
            Some(Ok(mut g)) => {
                g.text = Some(self.text.clone()); // Text nodes rasterise with the same fonts as everything else
                self.gpu = Some(g);
                self.set_players_gpu(true);
                self.toast(format!("GPU preview: {}", self.gpu_name));
            }
            Some(Err(e)) => self.gpu_off(&e),
            None => self.gpu_off("the renderer panicked"),
        }
    }

    // ---- ws:play-source-monitor ----
    /// GPU layer mode on/off for every player that shows video (the timeline's and the Source monitor's).
    fn set_players_gpu(&mut self, on: bool) {
        self.player.set_gpu(on);
        if let Some(s) = self.source.as_mut() {
            s.player.set_gpu(on);
        }
        if !on {
            self.source_gpu_tex = None; // its GL texture died with the renderer
        }
    }

    /// Give a secondary player (the Source monitor's) what `app.player` runs with: GPU layer mode when
    /// the renderer is up, the proxy map, and a share of the cache budget.
    pub(super) fn configure_player(&self, p: &mut Player) {
        configure(p, self.gpu.is_some(), self.settings.cache_mb, &self.proxy_map);
    }

    /// Render the Source player's newest layers into the renderer's own Source canvas and register it
    /// with egui (zero copy, like `gpu_preview_texture`). Called once per update, after the main
    /// preview's render. Nothing new from the player = nothing rendered: the last canvas stays valid.
    pub(super) fn source_gpu_render(&mut self, frame: &mut eframe::Frame) {
        if self.gpu.is_none() {
            self.source_gpu_tex = None;
            return;
        }
        let max_w = self.settings.preview_max_width;
        let (made, playing) = {
            let App { gpu, source, .. } = self;
            let (Some(gpu), Some(s)) = (gpu.as_mut(), source.as_mut()) else { return };
            // keep the layers until the pane has reported its size (the main preview's rule)
            if s.canvas.0 == 0 || s.canvas.1 == 0 {
                return;
            }
            let Some(layers) = s.player.take_layers() else { return };
            let (w, h) = clamp_canvas(s.canvas.0, s.canvas.1, max_w);
            let t = s.player.time();
            let project = &s.project;
            (guarded(|| gpu.render_source_texture(project, t, w, h, &layers)), s.player.is_playing())
        };
        let (tex, tw, th) = match made {
            Some(Some(m)) => m,
            Some(None) => return,
            None => {
                self.gpu_off("the renderer panicked");
                return;
            }
        };
        let id = match self.gpu_tex_ids.get(&tex) {
            Some(&id) => id,
            None => {
                let id = frame.register_native_glow_texture(tex);
                self.gpu_tex_ids.insert(tex, id);
                id
            }
        };
        self.source_gpu_tex = Some((id, [tw, th]));
        self.source_live = self.source_preview_frame(playing);
    }

    /// Where an export renders its frames: on the GPU renderer (requests served on the UI thread by
    /// the queue below) when it is up, else the CPU compositor on the export thread.
    pub(super) fn export_frames(&self) -> crate::engine::export::FrameSource {
        match self.gpu {
            Some(_) => crate::engine::export::FrameSource::Gpu(self.gpu_export.0.clone()),
            None => crate::engine::export::FrameSource::Cpu,
        }
    }

    /// Serve the frames export threads and movie-mode prerender workers are waiting on. Called every
    /// frame; each request is answered on the GL context, so both run the same shaders as the preview.
    /// Returns true if any were served (the caller keeps repainting so a background export is never
    /// starved; prerender already repaints on its own while busy).
    ///
    /// ws:export-pipeline: serves for at most `SERVE_BUDGET` per update (an export keeps a few requests
    /// queued), reading each frame back through the renderer's pixel-pack ring so frame k's readback
    /// overlaps the rendering of k+1, k+2; whatever is still in flight is finished before returning.
    /// A reply is the request's own (recycled) buffer; a dropped reply sender means "finish on the CPU".
    pub(super) fn serve_gpu_exports(&mut self) -> bool {
        // ponytail: a fixed budget - most of a 60 Hz frame, so the UI stays responsive during export
        const SERVE_BUDGET: Duration = Duration::from_millis(12);
        let started = Instant::now();
        let mut served = false;
        // replies whose pixels are still in a pixel-pack buffer, in the renderer's completion order
        let mut inflight: std::collections::VecDeque<ExportReply> = std::collections::VecDeque::new();
        let mut dead = false;
        while !dead && started.elapsed() < SERVE_BUDGET {
            let Ok(req) = self.gpu_export.1.try_recv() else { break };
            served = true;
            let App { gpu, project, .. } = self;
            let Some(g) = gpu.as_mut() else {
                // None tells the export thread to finish on the CPU compositor
                let _ = req.reply.send(None);
                continue;
            };
            dead = guarded(|| serve_export_request(g, project, req, &mut inflight)).is_none();
        }
        if !dead && !inflight.is_empty() {
            if let Some(g) = self.gpu.as_mut() {
                dead = guarded(|| {
                    while let Some((reply, mut buf)) = inflight.pop_front() {
                        g.render_export_finish(&mut buf);
                        let _ = reply.send(Some(buf));
                    }
                })
                .is_none();
            }
        }
        if dead {
            inflight.clear(); // dropped senders: those export threads finish on the CPU
            self.gpu_off("the renderer panicked");
        }
        served
    }

    /// Fall back to the CPU compositor and say why, once.
    pub(super) fn gpu_off(&mut self, why: &str) {
        self.gpu = None;
        self.gpu_tex = None;
        self.gpu_tex_ids.clear();
        effects_ui::clear_thumbnails();
        self.effect_thumbs.clear();
        self.effect_thumbs_key = None;
        self.gpu_failed = true;
        self.set_players_gpu(false);
        self.toast(format!("GPU rendering unavailable ({why}) - using the CPU compositor"));
    }

    /// Render the timeline into a GL texture and register it with egui, so the preview paints the GPU's
    /// own canvas - no glReadPixels, no re-upload. None when there is no GPU (the caller falls back to
    /// `gpu_frame`). The texture is only valid for this frame, which is exactly how long it is painted.
    pub(super) fn gpu_preview_texture(
        &mut self,
        layers: &crate::engine::gpu::LayerSet,
        t: f64,
        w: u32,
        h: u32,
        frame: &mut eframe::Frame,
    ) -> Option<(egui::TextureId, [u32; 2])> {
        if self.gpu.is_none() || w == 0 || h == 0 {
            return None;
        }
        let made = {
            let App { gpu, project, .. } = self;
            let gpu = gpu.as_mut()?;
            guarded(|| gpu.render_preview_texture(project, t, w, h, layers)).flatten()
        };
        let (tex, tw, th) = made?;
        let id = match self.gpu_tex_ids.get(&tex) {
            Some(&id) => id,
            None => {
                let id = frame.register_native_glow_texture(tex);
                self.gpu_tex_ids.insert(tex, id);
                id
            }
        };
        Some((id, [tw, th]))
    }

    /// Render decoded layers with the GPU into a frame the preview can upload. None = the GPU path died
    /// (already switched off).
    pub(super) fn gpu_frame(
        &mut self,
        layers: &crate::engine::gpu::LayerSet,
        t: f64,
        w: u32,
        h: u32,
    ) -> Option<Arc<Frame>> {
        if self.gpu.is_none() || w == 0 || h == 0 {
            return None;
        }
        // reuse the buffer of the frame handed out last time, once the preview has uploaded it
        let mut out = match self.gpu_prev.take().map(Arc::try_unwrap) {
            Some(Ok(f)) => f,
            _ => Frame::default(),
        };
        let ok = {
            let App { gpu, project, .. } = self;
            let gpu = gpu.as_mut()?;
            guarded(|| gpu.render_frame(project, t, w, h, layers, &mut out)).is_some()
        };
        if !ok {
            self.gpu_off("the renderer panicked");
            return None;
        }
        out.pts = t;
        let frame = Arc::new(out);
        self.gpu_prev = Some(frame.clone());
        Some(frame)
    }

    /// One frame at `t`, `w` px wide, through the same path the preview uses (GPU when it is on, the
    /// player's compositor otherwise) - export-frame and the MCP tools.
    pub(super) fn render_frame_now(&mut self, t: f64, w: u32) -> Option<Arc<Frame>> {
        if self.gpu.is_some() {
            let (pw, ph) = (self.project.width.max(16), self.project.height.max(16));
            let w = w.clamp(16, pw);
            let h = ((ph as u64 * w as u64) / pw as u64).max(1) as u32;
            if let Some(layers) = self.player.layers_once(t, w) {
                if let Some(f) = self.gpu_frame(&layers, t, w, h) {
                    return Some(f);
                }
            }
        }
        self.player.render_once(t, w)
    }

    /// Movie mode: ask for the in/out range, or the whole timeline when there is none.
    pub(super) fn request_prerender(&mut self) {
        let a = self.project.in_point.unwrap_or(0.0);
        let b = self.project.out_point.unwrap_or_else(|| self.project.duration());
        if b > a {
            let App { prerender, project, .. } = self;
            if guarded(|| prerender.request(project, a, b)).is_none() {
                self.settings.movie_mode = false;
                self.toast("Movie mode is not available in this build");
            }
        }
    }

    // ---- ws:export-deliver ----
    /// Render Selection / `render.range`: pre-render an explicit `[a, b)` into the movie-mode cache
    /// without touching the in/out points (or the in/out request already in flight - `PreRender::
    /// request` merges ranges). Turns movie mode on if it was off, or the cache would never be read.
    pub(crate) fn request_prerender_range(&mut self, a: f64, b: f64) -> Result<(), &'static str> {
        let (a, b) = (a.max(0.0), b.min(self.project.duration()));
        if !(b > a) {
            return Err("Nothing to render - the range is empty");
        }
        self.settings.movie_mode = true;
        let App { prerender, project, .. } = self;
        if guarded(|| prerender.request(project, a, b)).is_none() {
            self.settings.movie_mode = false;
            return Err("Movie mode is not available in this build");
        }
        Ok(())
    }

    /// "Export Frame…" confirmed: render at the chosen size and write the image with ffmpeg.
    pub(super) fn export_frame(&mut self, opts: frame_ui::FrameExport) {
        let (rw, rh) = frame_render_size((self.project.width, self.project.height), opts.size);
        let scaler_before = self.project.scaler;
        self.project.scaler = opts.scaler;
        let frame = if opts.with_effects {
            self.render_frame_now(self.playhead, rw)
        } else {
            self.source_frame(self.playhead, rw, rh)
        };
        self.project.scaler = scaler_before;
        let Some(frame) = frame else {
            self.toast("Could not render that frame");
            return;
        };
        match write_image(&frame, &opts) {
            Ok(()) => self.toast_with_folder(format!("Frame saved to {}", opts.out.display()), opts.out),
            Err(e) => self.toast(format!("Frame export failed: {e}")),
        }
    }

    /// The decoded frame of the top-most visual clip under the playhead ("source frame only").
    pub(super) fn source_frame(&mut self, t: f64, w: u32, h: u32) -> Option<Arc<Frame>> {
        let clip = self
            .project
            .tracks
            .iter()
            .enumerate()
            .filter(|(i, tr)| tr.kind == TrackKind::Video && self.project.active(*i))
            .flat_map(|(_, tr)| tr.clips.iter())
            .filter(|c| c.enabled && c.contains(t) && c.uses_asset())
            .next_back()?;
        let asset = self.project.asset(clip.asset)?;
        let mut src = media::open_video(&asset.path, self.backend()).ok()?;
        let mut f = Frame::default();
        src.frame_at(clip.src_time(t).max(0.0), w, h, &mut f).then(|| Arc::new(f))
    }
}

// ---- ws:play-source-monitor ----
/// The Source monitor's share of `Settings.cache_mb`: it shows one file, the timeline's player needs the rest.
pub(super) const SOURCE_CACHE_DIV: usize = 8;

/// The three settings a secondary player takes from the app. A trait only so the test can record them -
/// `Player` sends them to its render thread and keeps nothing readable.
pub(crate) trait PlayerSetup {
    fn set_gpu(&mut self, on: bool);
    fn set_cache_bytes(&mut self, bytes: usize);
    fn set_proxies(&mut self, map: std::collections::HashMap<String, String>);
}

impl PlayerSetup for Player {
    fn set_gpu(&mut self, on: bool) {
        Player::set_gpu(self, on)
    }
    fn set_cache_bytes(&mut self, bytes: usize) {
        Player::set_cache_bytes(self, bytes)
    }
    fn set_proxies(&mut self, map: std::collections::HashMap<String, String>) {
        Player::set_proxies(self, map)
    }
}

fn configure(p: &mut impl PlayerSetup, gpu: bool, cache_mb: u32, proxies: &std::collections::HashMap<String, String>) {
    p.set_gpu(gpu);
    p.set_cache_bytes(crate::playback::cache_budget_bytes(cache_mb) / SOURCE_CACHE_DIV);
    p.set_proxies(proxies.clone());
}

#[cfg(test)]
mod play_source_monitor_tests {
    use super::*;

    #[derive(Default)]
    struct Rec {
        gpu: Option<bool>,
        bytes: Option<usize>,
        proxies: Option<std::collections::HashMap<String, String>>,
    }

    impl PlayerSetup for Rec {
        fn set_gpu(&mut self, on: bool) {
            self.gpu = Some(on);
        }
        fn set_cache_bytes(&mut self, bytes: usize) {
            self.bytes = Some(bytes);
        }
        fn set_proxies(&mut self, map: std::collections::HashMap<String, String>) {
            self.proxies = Some(map);
        }
    }

    /// `configure_player` hands the Source player the GPU flag, the proxy map and an eighth of the cache.
    #[test]
    fn configure_player_reaches_the_source_player() {
        let map: std::collections::HashMap<String, String> =
            [("C:/a.mp4".to_string(), "C:/proxies/a.mp4".to_string())].into();
        let mut r = Rec::default();
        configure(&mut r, true, 4096, &map);
        assert_eq!(r.gpu, Some(true));
        assert_eq!(r.bytes, Some((4096usize << 20) / 8), "a share of cache_mb, not the 512 MB default");
        assert_eq!(r.proxies.as_ref(), Some(&map));
        let mut r = Rec::default();
        configure(&mut r, false, 4096, &std::collections::HashMap::new());
        assert_eq!(r.gpu, Some(false), "no renderer = CPU frames");
    }
}
