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

    /// Record a voiceover take at the playhead. `from_playhead` rolls the timeline under it (from the
    /// playhead, never rewinding to 0 at the end), so what you say lines up with what you hear.
    pub(super) fn start_voiceover(&mut self, opts: crate::engine::capture::VoiceoverOptions, from_playhead: bool) {
        if self.voice_rec.is_some() || self.ffmpeg_missing() {
            return;
        }
        let out = opts.out.clone();
        if let Some(dir) = out.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match guarded(|| crate::engine::capture::start_voiceover(opts)) {
            Some(Ok(c)) => {
                if from_playhead {
                    self.seek(self.playhead);
                    self.player.roll();
                }
                self.voice_rec = Some((c, out, (self.playhead, self.player.is_playing())));
            }
            Some(Err(e)) => self.toast(format!("Voiceover failed: {e}")),
            None => self.toast("Voiceover recording is not available in this build"),
        }
    }

    pub(super) fn stop_voiceover(&mut self) {
        let Some((c, out, (at, rolling))) = self.voice_rec.take() else { return };
        let lead = c.lead_in();
        self.player.pause();
        if guarded(move || c.stop()).is_none() {
            return;
        }
        self.capture_ui.last_take = Some((at, out.clone()));
        self.import_recording(out, Some(take_placement(at, rolling, lead)));
    }

    /// Retake: throw the last voiceover take away (its clips in one undo step; one still being finalised
    /// is never placed) and park the playhead where it started, so the next take records from there.
    pub(super) fn drop_last_take(&mut self) {
        let Some((at, out)) = self.capture_ui.last_take.take() else { return };
        self.pending_recordings.retain(|r| r.out != out);
        let ids = take_clips(&self.project, &out);
        if !ids.is_empty() {
            self.push_undo();
            self.project.delete_clips(&ids, false);
            self.after_edit();
        }
        self.seek(at);
    }

    /// A finished recording: import it and (for a voiceover) drop it on the timeline at `at`. ffmpeg
    /// finalises the container a moment after it is asked to stop, so the file is waited for by
    /// `poll_recordings` (per frame, up to 3 s) - never with a sleep on this thread.
    pub(super) fn import_recording(&mut self, out: PathBuf, at: Option<f64>) {
        self.pending_recordings.push(PendingRecording { out, at, deadline: Instant::now() + Duration::from_secs(3) });
    }

    // ---- ws:job-completion-hitches ----
    /// Per frame: import every recording whose file has landed; give up on those past their deadline.
    pub(super) fn poll_recordings(&mut self, ctx: &egui::Context) {
        if self.pending_recordings.is_empty() {
            return;
        }
        let now = Instant::now();
        let pending = std::mem::take(&mut self.pending_recordings);
        for r in pending {
            match recording_step(r.out.exists(), now, r.deadline) {
                RecordingStep::Wait => self.pending_recordings.push(r),
                RecordingStep::Import => self.place_recording(r.out, r.at),
                RecordingStep::GiveUp => self.toast(format!("Recording not written: {}", r.out.display())),
            }
        }
        if !self.pending_recordings.is_empty() {
            self.animate_until(ctx, Instant::now() + Duration::from_millis(50));
        }
    }

    /// Per frame: apply a finished `timeline.import` job's report (the MCP reply follows in
    /// `poll_mcp`, later this same frame, so a caller's next read sees the project already swapped).
    pub(super) fn poll_timeline_imports(&mut self, ctx: &egui::Context) {
        if self.pending_timeline_imports.is_empty() {
            return;
        }
        let pending = std::mem::take(&mut self.pending_timeline_imports);
        for (prog, holder, replace) in pending {
            if !prog.is_done() {
                self.pending_timeline_imports.push((prog, holder, replace));
                continue;
            }
            let Some(report) = holder.lock().unwrap_or_else(|e| e.into_inner()).take() else { continue };
            if replace {
                self.set_project(report.project, None);
            } else {
                self.import_ui.report = Some(report);
                self.import_ui.open = true;
            }
        }
        self.animate_until(ctx, Instant::now() + Duration::from_millis(200));
    }

    fn place_recording(&mut self, out: PathBuf, at: Option<f64>) {
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

    /// `--screenshot <out.ppm>`: save the first rendered frame and exit.
    pub(super) fn screenshot_tick(&mut self, ctx: &egui::Context) {
        let Some(path) = self.screenshot.clone() else { return };
        // save when the screenshot event arrives
        let img = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                // untagged only: a tagged one is an MCP `ui.screenshot` (tools_uikit.rs), not ours
                egui::Event::Screenshot { image, user_data, .. } if user_data.data.is_none() => Some(image.clone()),
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
        // SE_SCREENSHOT_DELAY=<seconds> waits instead, for checks that need caches (thumbnails) warmed up;
        // SE_SCREENSHOT_WHEN=<file> waits until that file exists, so a script can drive the UI over MCP first
        // (scripts/docs-shots.ps1 - unlike a live ui.screenshot, this path still captures while Windows is locked).
        let elapsed = self.started.elapsed().as_secs_f32();
        let delay: Option<f32> = std::env::var("SE_SCREENSHOT_DELAY").ok().and_then(|v| v.parse().ok());
        let ready = match (std::env::var_os("SE_SCREENSHOT_WHEN"), delay) {
            (Some(f), _) => std::path::Path::new(&f).exists(),
            (None, Some(d)) => elapsed > d,
            (None, None) => self.first_frame_at.is_some() || elapsed > 2.5,
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
                if !proxy_eligible(a, h) {
                    continue;
                }
                let dst = crate::media::proxy::proxy_path(&a.path, h);
                if dst.exists() {
                    map.insert(a.path.clone(), dst.to_string_lossy().into_owned());
                }
            }
            // ---- ws:jobs-panel ----
            want = pick_next_proxy(&proxy_candidates(&self.project.assets, h), self.proxy_next.as_deref());
            if want.is_some() {
                self.proxy_next = None;
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

// ---- ws:jobs-panel ----
/// Only real video that out-sizes the proxy: images/audio gain nothing, and neither does footage
/// already at or below proxy resolution. Nor, yet, a file still being probed for an alpha channel:
/// that decides which proxy it gets (`proxy::has_alpha`), and the next scan asks again.
fn proxy_eligible(a: &crate::model::Asset, h: u32) -> bool {
    a.kind == crate::model::ClipKind::Video
        && a.height > h
        && a.duration > 0.0
        && crate::media::proxy::has_alpha(&a.path).is_some()
}

/// Eligible assets whose proxy is not built yet (and whose source is on disk), in library order -
/// the Jobs pane's "queued proxies" and `pick_next_proxy`'s input.
pub(super) fn proxy_candidates(assets: &[crate::model::Asset], h: u32) -> Vec<(String, std::path::PathBuf)> {
    assets
        .iter()
        .filter(|a| proxy_eligible(a, h))
        .filter_map(|a| {
            let dst = crate::media::proxy::proxy_path(&a.path, h);
            (!dst.exists() && std::path::Path::new(&a.path).exists()).then(|| (a.path.clone(), dst))
        })
        .collect()
}

/// Which proxy builds next: `want_next` (the Jobs pane's "Build next") when it is still a candidate,
/// else the first candidate in library order (the pre-pane behaviour).
pub(crate) fn pick_next_proxy(
    cands: &[(String, std::path::PathBuf)],
    want_next: Option<&str>,
) -> Option<(String, std::path::PathBuf)> {
    want_next.and_then(|w| cands.iter().find(|(p, _)| p == w)).or(cands.first()).cloned()
}

#[cfg(test)]
mod proxy_tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn proxy_next_wins_over_library_order() {
        let c = |p: &str| (p.to_string(), PathBuf::from(format!("{p}.proxy")));
        let cands = vec![c("a"), c("b")];
        assert_eq!(pick_next_proxy(&cands, None).unwrap().0, "a", "library order by default");
        assert_eq!(pick_next_proxy(&cands, Some("b")).unwrap().0, "b", "Build next wins");
        // b already has a proxy / is missing on disk: `proxy_candidates` never lists it, so the
        // request falls back to library order instead of building something ineligible
        assert_eq!(pick_next_proxy(&cands[..1], Some("b")).unwrap().0, "a");
        assert!(pick_next_proxy(&[], Some("b")).is_none());
    }
}

// ---- ws:job-completion-hitches ----
/// A stopped recording whose container ffmpeg is still finalising - see `App::poll_recordings`.
pub(super) struct PendingRecording {
    pub(super) out: PathBuf,
    /// Voiceover: place it on the timeline here. Screen recording: library only.
    pub(super) at: Option<f64>,
    pub(super) deadline: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RecordingStep {
    Wait,
    Import,
    GiveUp,
}

/// Where a voiceover take lands: the playhead it started from, plus ffmpeg's device-open lead-in when
/// the timeline was rolling under it (sample 0 was captured that much later). A still timeline has no
/// lead-in to make up for.
pub(crate) fn take_placement(at: f64, rolling: bool, lead_in: f64) -> f64 {
    if rolling {
        at + lead_in
    } else {
        at
    }
}

/// The timeline clips cut from the recording at `out` (a voiceover take, for Retake).
pub(crate) fn take_clips(project: &Project, out: &std::path::Path) -> Vec<Id> {
    let Some(aid) = project.asset_by_path(&out.to_string_lossy()).map(|a| a.id) else { return Vec::new() };
    project.all_clips().filter(|(_, c)| c.asset == aid).map(|(_, c)| c.id).collect()
}

/// The per-frame decision for one pending recording, pure so it is testable without a live `App`.
pub(crate) fn recording_step(exists: bool, now: Instant, deadline: Instant) -> RecordingStep {
    if exists {
        RecordingStep::Import
    } else if now < deadline {
        RecordingStep::Wait
    } else {
        RecordingStep::GiveUp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_step_waits_imports_then_gives_up() {
        let now = Instant::now();
        let later = now + Duration::from_secs(3);
        assert_eq!(recording_step(false, now, later), RecordingStep::Wait);
        assert_eq!(recording_step(true, now, later), RecordingStep::Import);
        assert_eq!(recording_step(true, later, now), RecordingStep::Import, "a late file still imports");
        assert_eq!(recording_step(false, later, later), RecordingStep::GiveUp);
        assert_eq!(recording_step(false, later + Duration::from_secs(1), later), RecordingStep::GiveUp);
    }

    #[test]
    fn a_take_lands_after_the_lead_in_only_under_rolling_playback() {
        assert_eq!(take_placement(4.0, true, 0.8), 4.8, "from the playhead: sample 0 was heard 0.8 s in");
        assert_eq!(take_placement(4.0, false, 0.8), 4.0, "a still timeline: the take starts at the playhead");
    }

    #[test]
    fn retake_removes_only_the_last_takes_clips() {
        let mut p = Project::new();
        let take = PathBuf::from("C:/rec/voice-2.wav");
        let earlier = p.add_asset(crate::engine::import::placeholder("C:/rec/voice-1.wav"));
        let last = p.add_asset(crate::engine::import::placeholder(&take.to_string_lossy()));
        let ti = p.add_track(TrackKind::Audio);
        let ids: Vec<Id> = [earlier, last]
            .into_iter()
            .enumerate()
            .map(|(i, a)| {
                let id = p.new_id();
                let mut c = Clip::new(id, ClipKind::Audio, "vo", i as f64 * 5.0, 3.0);
                c.asset = a;
                p.tracks[ti].clips.push(c);
                id
            })
            .collect();
        assert_eq!(take_clips(&p, &take), vec![ids[1]]);
        p.delete_clips(&take_clips(&p, &take), false);
        assert!(take_clips(&p, &take).is_empty() && p.clip(ids[0]).is_some(), "the earlier take stays");
        assert!(take_clips(&p, Path::new("C:/rec/gone.wav")).is_empty());
    }

    #[test]
    fn import_recording_never_sleeps_on_the_caller() {
        let src = include_str!("jobs.rs");
        let start = src.find("fn import_recording(").expect("import_recording");
        let end = start + src[start..].find("\n    }\n").expect("end of fn");
        let body = &src[start..end];
        assert!(!body.contains("thread::sleep"), "import_recording must not block the UI thread:\n{body}");
        assert!(!body.contains("exists()"), "the file wait belongs to poll_recordings, per frame:\n{body}");
    }
}
