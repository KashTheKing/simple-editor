use super::*;

impl App {
    pub(super) fn poll_probes(&mut self, ctx: &egui::Context) {
        if self.probes.is_empty() {
            return;
        }
        let mut landed: Vec<crate::engine::import::Probed> = Vec::new();
        self.probes.retain(|rx| loop {
            match rx.try_recv() {
                Ok(p) => landed.push(p),
                Err(std::sync::mpsc::TryRecvError::Empty) => break true,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break false,
            }
        });
        let mut any = false;
        for p in landed {
            match p.asset {
                Ok(a) => {
                    any |= crate::engine::import::adopt(&mut self.project, p.id, a);
                    self.settings.touch_recent(&p.path);
                }
                Err(e) => {
                    self.project.remove_asset(p.id);
                    self.toast(format!("Can't import {}: {e}", p.path));
                    any = true;
                }
            }
        }
        if any {
            // first media into an empty project: adopt its format (only knowable now)
            if self.project.is_empty() {
                if let Some(a) = self.project.assets.first().cloned() {
                    if a.has_video() && a.width > 0 {
                        self.project.width = a.width;
                        self.project.height = a.height;
                        if a.fps > 1.0 {
                            self.project.fps = a.fps;
                        }
                    }
                }
            }
            self.settings.save();
            self.after_edit();
        }
        if !self.probes.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
    }

    /// Start / stop the screen recorder with the options the window built (also driven by focus when
    /// `capture_on_blur` is on, which rebuilds them from settings + the window's region).
    pub(super) fn start_screen_capture(&mut self, opts: crate::engine::capture::ScreenCaptureOptions) {
        if self.screen_rec.is_some() || self.ffmpeg_missing() {
            return;
        }
        let out = opts.out.clone();
        if let Some(dir) = out.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match guarded(|| crate::engine::capture::start_screen(opts)) {
            Some(Ok(c)) => self.screen_rec = Some((c, out)),
            Some(Err(e)) => self.toast(format!("Screen recording failed: {e}")),
            None => self.toast("Screen recording is not available in this build"),
        }
    }

    pub(super) fn stop_screen_capture(&mut self) {
        let Some((c, out)) = self.screen_rec.take() else { return };
        if guarded(move || c.stop()).is_none() {
            return;
        }
        self.import_recording(out, None);
    }

    pub(super) fn start_voiceover(&mut self, opts: crate::engine::capture::VoiceoverOptions) {
        if self.voice_rec.is_some() || self.ffmpeg_missing() {
            return;
        }
        let out = opts.out.clone();
        if let Some(dir) = out.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match guarded(|| crate::engine::capture::start_voiceover(opts)) {
            Some(Ok(c)) => {
                self.voice_rec = Some((c, out, self.playhead));
                self.player.play(); // the take lines up with what you hear
            }
            Some(Err(e)) => self.toast(format!("Voiceover failed: {e}")),
            None => self.toast("Voiceover recording is not available in this build"),
        }
    }

    pub(super) fn stop_voiceover(&mut self) {
        let Some((c, out, at)) = self.voice_rec.take() else { return };
        self.player.pause();
        if guarded(move || c.stop()).is_none() {
            return;
        }
        self.import_recording(out, Some(at));
    }

    /// A finished recording: import it and (for a voiceover) drop it on the timeline at `at`.
    pub(super) fn import_recording(&mut self, out: PathBuf, at: Option<f64>) {
        // ffmpeg finalises the container a moment after it is asked to stop
        let deadline = Instant::now() + Duration::from_secs(3);
        while !out.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        if !out.exists() {
            self.toast(format!("Recording not written: {}", out.display()));
            return;
        }
        let ids = self.import_files(&[out.clone()]);
        match at {
            Some(t) => {
                self.push_undo();
                self.place_assets(&ids, t, None, DropMode::Place);
                self.after_edit();
                self.toast("Voiceover placed on the timeline");
            }
            None => {
                self.library.tab = 0;
                self.library.selected = ids.last().copied();
                self.toast(format!("Recording imported: {}", out.file_name().unwrap_or_default().to_string_lossy()));
            }
        }
    }

    /// dshow audio inputs, asked for once (the ffmpeg device probe takes ~a second).
    pub(super) fn audio_inputs(&mut self) -> Vec<(String, bool)> {
        if self.audio_inputs.is_none() {
            self.audio_inputs = Some(guarded(crate::engine::capture::audio_devices).unwrap_or_default());
        }
        self.audio_inputs.clone().unwrap_or_default()
    }

    /// Screen-capture options for the record-on-blur path, which has no window response to take them
    /// from. Same folder the Screen Recording window shows (`capture_ui::capture_dir`).
    pub(super) fn blur_capture_options(&self) -> crate::engine::capture::ScreenCaptureOptions {
        crate::engine::capture::ScreenCaptureOptions {
            out: capture_ui::capture_dir(&self.settings).join(format!("screen-{}.mp4", Settings::now())),
            fps: self.settings.capture_fps.clamp(1, 120),
            bitrate_kbps: self.settings.capture_bitrate_kbps,
            crf: self.settings.crf,
            region: if self.capture_ui.area == "region" { self.capture_ui.region } else { None },
            mic: self.settings.capture_mic.clone(),
            desktop_audio: self.settings.capture_desktop_audio,
            cursor: self.settings.capture_cursor,
        }
    }

    // ---------------- UI pieces ----------------

    /// Icon shown next to a menu action: the user's pick from Settings → Appearance → Icons wins,
    /// then the built-in defaults below. Abstract actions stay text-only.
    pub(super) fn screenshot_tick(&mut self, ctx: &egui::Context) {
        let Some(path) = self.screenshot.clone() else { return };
        // save when the screenshot event arrives
        let img = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = img {
            let [w, h] = img.size;
            let mut data = format!("P6\n{w} {h}\n255\n").into_bytes();
            for p in &img.pixels {
                data.extend_from_slice(&[p.r(), p.g(), p.b()]);
            }
            let _ = std::fs::write(&path, data);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            self.screenshot = None;
            return;
        }
        // shoot as soon as the first rendered frame is on screen (or after the timeout when nothing renders).
        // SE_SCREENSHOT_DELAY=<seconds> waits instead, for checks that need caches (thumbnails) warmed up.
        let elapsed = self.started.elapsed().as_secs_f32();
        let delay: Option<f32> = std::env::var("SE_SCREENSHOT_DELAY").ok().and_then(|v| v.parse().ok());
        let ready = match delay {
            Some(d) => elapsed > d,
            None => self.first_frame_at.is_some() || elapsed > 2.5,
        };
        if !self.screenshot_requested && ready {
            self.screenshot_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }

    /// Proxy media: keep an all-intra low-res proxy built for every video asset and hand the map to
    /// the player. Rescans every 2 s (a handful of stat calls); one ffmpeg transcode at a time.
    /// ponytail: no per-asset badge yet - the preview shows one aggregate "Building proxy" line.
    pub(super) fn sync_proxies(&mut self) {
        if let Some((src, _, p)) = &self.proxy_job {
            if !p.is_done() {
                return;
            }
            if let Some(e) = p.error() {
                eprintln!("proxy for {src}: {e}");
            }
            self.proxy_job = None;
            self.proxy_scan_at = None; // pick up the finished file (and start the next) right away
        }
        if self.proxy_scan_at.is_some_and(|t| t > Instant::now()) {
            return;
        }
        self.proxy_scan_at = Some(Instant::now() + Duration::from_secs(2));
        let h = self.settings.proxy_height.max(120);
        let mut map = std::collections::HashMap::new();
        let mut want: Option<(String, std::path::PathBuf)> = None;
        if self.settings.use_proxies {
            for a in &self.project.assets {
                // only real video that out-sizes the proxy: images/audio gain nothing, and neither
                // does footage already at or below proxy resolution
                if a.kind != crate::model::ClipKind::Video || a.height <= h || a.duration <= 0.0 {
                    continue;
                }
                let dst = crate::media::proxy::proxy_path(&a.path, h);
                if dst.exists() {
                    map.insert(a.path.clone(), dst.to_string_lossy().into_owned());
                } else if want.is_none() && std::path::Path::new(&a.path).exists() {
                    want = Some((a.path.clone(), dst));
                }
            }
        }
        if map != self.proxy_map {
            crate::media::proxy::set_ready(map.keys().cloned().collect()); // badges track the same push
            self.proxy_map = map.clone();
            self.player.set_proxies(map);
        }
        if let Some((src, dst)) = want {
            if crate::media::ffpipe::ffmpeg_exe().is_some() {
                let job = crate::media::proxy::generate(src.clone(), dst.clone(), h);
                self.proxy_job = Some((src, dst, job));
            }
        }
    }
}
