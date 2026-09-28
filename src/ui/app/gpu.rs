use super::thumbs::*;
use super::*;

impl App {
    pub(super) fn sync_gpu(&mut self) {
        let want = self.settings.gpu && !self.gpu_failed;
        if want == self.gpu.is_some() {
            return;
        }
        if !want {
            self.gpu = None;
            self.player.set_gpu(false);
            return;
        }
        let Some(gl) = self.gl.clone() else {
            self.gpu_failed = true; // no GL context at all (software / headless run)
            self.player.set_gpu(false);
            return;
        };
        match guarded(|| GpuRenderer::new(gl)) {
            Some(Ok(mut g)) => {
                g.text = Some(self.text.clone()); // Text nodes rasterise with the same fonts as everything else
                self.gpu = Some(g);
                self.player.set_gpu(true);
                self.toast(format!("GPU preview: {}", self.gpu_name));
            }
            Some(Err(e)) => self.gpu_off(&e),
            None => self.gpu_off("the renderer panicked"),
        }
    }

    /// Render one catalogue thumbnail per `EffectKind` over the stock image and hand them to the effects
    /// panel. Runs once per (stock image, size) - the GPU renderer owns the GL context, so this happens on
    /// the UI thread. Without a GPU the panel keeps its neutral named cards.
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
    pub(super) fn serve_gpu_exports(&mut self) -> bool {
        let mut served = false;
        while let Ok(req) = self.gpu_export.1.try_recv() {
            served = true;
            let mut out = Frame::default();
            let ok = {
                let App { gpu, project, .. } = self;
                match gpu.as_mut() {
                    Some(g) => {
                        out.resize(req.w, req.h);
                        guarded(|| g.render_frame(project, req.t, req.w, req.h, &req.layers, &mut out)).is_some()
                    }
                    None => false,
                }
            };
            if !ok {
                self.gpu_off("the renderer panicked");
            }
            // None tells the export thread to finish on the CPU compositor
            let _ = req.reply.send(ok.then_some(out));
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
        self.player.set_gpu(false);
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
