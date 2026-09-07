use super::*;

pub(super) fn step_time(playhead: f64, fps: f64, forward: bool, duration: f64) -> f64 {
    let dt = 1.0 / fps.max(1.0);
    if forward {
        (playhead + dt).min(duration)
    } else {
        (playhead - dt).max(0.0)
    }
}

impl App {
    /// so re-clicking the selected row does not restart it. Pauses the timeline: previewing a source and
    /// the program monitor should not both be making sound at once.
    pub(super) fn start_lib_preview(&mut self, ctx: &egui::Context, path: PathBuf) {
        self.player.pause();
        if self.lib_preview.as_ref().is_some_and(|lp| lp.path == path) {
            return;
        }
        // an asset the project already knows carries its probed duration; anything else (a Global/
        // Recent file never imported) is probed on the spot — a single ffprobe call for metadata
        // only, cheap enough for a click the user is actively waiting on
        let asset = match self.project.assets.iter().find(|a| a.path == path.to_string_lossy()) {
            Some(a) => a.clone(),
            None => match crate::media::probe(&path.to_string_lossy(), self.backend()) {
                Ok(a) => a,
                Err(_) => {
                    self.lib_preview = None;
                    return;
                }
            },
        };
        let duration = asset.duration.max(crate::model::MIN_CLIP);
        let fps = if asset.fps > 0.0 { asset.fps } else { 30.0 };
        let has_video = asset.has_video();
        let is_image = asset.kind == ClipKind::Image;
        let mut player = Player::new(ctx.clone(), self.backend(), self.text.clone());
        player.set_project(&Project::from_media(asset));
        player.play();
        self.lib_preview =
            Some(LibPreview { path, player, duration, fps, has_video, is_image, heartbeat: Default::default() });
    }

    /// The library preview's current frame, uploaded for the pane to paint. None = nothing is previewing.
    /// Called exactly once per update (see `self.lib_preview_live`) - `Player::take_frame` consumes the
    /// buffered frame, so a second call in the same frame would come back empty.
    pub(super) fn lib_preview_frame(&mut self, ctx: &egui::Context) -> Option<library::PreviewFrame> {
        let lp = self.lib_preview.as_mut()?;
        let playing = lp.player.is_playing();
        if let Some(f) = lp.player.take_frame() {
            let (w, h) = (f.width as usize, f.height as usize);
            if w > 0 && h > 0 && f.rgba.len() == w * h * 4 {
                let img = egui::ColorImage::from_rgba_premultiplied([w, h], &f.rgba);
                match self.lib_preview_tex.as_mut() {
                    Some(t) if t.size() == [w, h] => t.set_partial([0, 0], img, egui::TextureOptions::LINEAR),
                    Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                    None => {
                        self.lib_preview_tex = Some(ctx.load_texture("lib_preview", img, egui::TextureOptions::LINEAR))
                    }
                }
            }
        }
        if playing {
            ctx.request_repaint();
        }
        let t = self.lib_preview_tex.as_ref()?;
        Some(library::PreviewFrame { tex: t.id(), size: [t.size()[0] as u32, t.size()[1] as u32], playing })
    }

    /// The Preview pane while a library asset is being previewed: the timeline's player and project are
    /// left untouched underneath (still decoding in the background, so resuming is instant), and this
    /// draws the previewed file's own frame with a transport bound to its own `Player` instead.
    pub(super) fn draw_lib_preview(&mut self, ui: &mut egui::Ui) {
        let Some(lp) = self.lib_preview.as_ref() else { return };
        let name = lp.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let path = lp.path.to_string_lossy().into_owned();
        let duration = lp.duration;
        let fps = lp.fps;
        let playhead = lp.player.time();
        let playing = lp.player.is_playing();
        let has_video = lp.has_video;
        let is_image = lp.is_image;
        let frame = self.lib_preview_live;
        let buffering = lp.player.is_buffering();
        let palette = self.palette;

        let (mut close, mut toggle, mut stop) = (false, false, false);
        let mut seek_to: Option<f64> = None;

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            if is_image {
                // a still image: just the close button and name, no transport for a single frame
                ui.horizontal(|ui| {
                    if markers_ui::x_button(ui).on_hover_text("Back to timeline").clicked() {
                        close = true;
                    }
                    ui.weak(name);
                });
            } else {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let mut b = |ui: &mut egui::Ui, icon: tools::Glyph, label: &str| -> bool {
                        tools::glyph_text_button(ui, icon, "").on_hover_text(label).clicked()
                    };
                    if b(ui, tools::Glyph::Jump(tools::Dir::Left), "Go to start") {
                        seek_to = Some(0.0);
                    }
                    if b(ui, tools::Glyph::Tri(tools::Dir::Left), "Step back one frame") {
                        seek_to = Some(step_time(playhead, fps, false, duration));
                    }
                    let pp_icon = if playing { tools::Glyph::Pause } else { tools::Glyph::Play };
                    let pp_label = if playing { "Pause" } else { "Play" };
                    if b(ui, pp_icon, pp_label) {
                        toggle = true;
                    }
                    if b(ui, tools::Glyph::Stop, "Stop") {
                        stop = true;
                    }
                    if b(ui, tools::Glyph::Tri(tools::Dir::Right), "Step forward one frame") {
                        seek_to = Some(step_time(playhead, fps, true, duration));
                    }
                    if b(ui, tools::Glyph::Jump(tools::Dir::Right), "Go to end") {
                        seek_to = Some(duration);
                    }
                    ui.add_space(8.0);
                    let tc = crate::ui::timecode;
                    ui.monospace(format!("{} / {}", tc(playhead, fps), tc(duration, fps)));
                    ui.add_space(8.0);
                    let cur =
                        preview::QUALITIES.iter().copied().find(|&q| q == self.settings.preview_quality).unwrap_or(100);
                    egui::ComboBox::from_id_salt("lib_preview_quality")
                        .selected_text(format!("{cur} %"))
                        .width(64.0)
                        .show_ui(ui, |ui| {
                            for q in preview::QUALITIES {
                                if ui.selectable_label(cur == q, format!("{q} %")).clicked() && q != cur {
                                    self.settings.preview_quality = q;
                                    self.settings.save();
                                }
                            }
                        });
                    ui.add_space(8.0);
                    if markers_ui::x_button(ui).on_hover_text("Back to timeline").clicked() {
                        close = true;
                    }
                    ui.weak(name);
                });
                // scrub bar: click or drag anywhere on it seeks — the one implementation, shared with
                // the timeline's own preview (dedupes what used to be a second copy of this painting).
                if let Some(t) = preview::scrub_bar(ui, duration, playhead, &palette, ui.available_width()) {
                    seek_to = Some(t);
                }
            }

            // video, filling whatever is left
            let (rect, resp) = ui.allocate_exact_size(ui.available_size_before_wrap(), egui::Sense::click());
            ui.painter().rect_filled(rect, 0.0, egui::Color32::BLACK);
            let lb = match frame {
                Some(f) => {
                    let aspect = f.size[0].max(1) as f32 / f.size[1].max(1) as f32;
                    let lb = preview::letterbox(rect, aspect, ui.pixels_per_point());
                    ui.painter().image(
                        f.tex,
                        lb,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                    lb
                }
                // no frame yet: the pane's whole rect stands in for the letterbox
                None => rect,
            };
            // set the canvas even before the first frame arrives: the player starts at 0x0 and
            // renders nothing until it is told a size, so gating this on `frame` deadlocked the
            // preview at a permanent black box
            let full_cw = (lb.width() * ui.pixels_per_point()) as u32;
            let full_ch = (lb.height() * ui.pixels_per_point()) as u32;
            let (cw, ch) = preview_canvas((full_cw, full_ch), self.settings.preview_quality);
            if let Some(lp) = self.lib_preview.as_mut() {
                lp.player.set_canvas(cw.max(16), ch.max(16), self.settings.preview_max_width);
            }
            // buffering: the clock is held while the read-ahead refills — say so over the video
            if buffering {
                ui.put(
                    egui::Rect::from_center_size(lb.center(), egui::vec2(32.0, 32.0)),
                    egui::Spinner::new().size(32.0),
                );
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
            }
            if !has_video {
                // cover art (an audio file's embedded picture, decoded through the same thumbnail
                // pipeline as any other video frame) wins over the visualizer when there is one
                let cover = self.thumbs.texture(ui.ctx(), &path, 0.0, (rect.height() * ui.pixels_per_point()) as u32);
                if let Some((tex, size)) = cover {
                    let aspect = size[0].max(1) as f32 / size[1].max(1) as f32;
                    let lb = preview::letterbox(rect, aspect, ui.pixels_per_point());
                    ui.painter().image(
                        tex,
                        lb,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                } else if self.settings.audio_visualizer {
                    let amplitude = self
                        .waveforms
                        .get(&path, 0)
                        .map(|p| crate::ui::heartbeat::amplitude_at(&[p], playhead, 1.0 / 30.0))
                        .unwrap_or(0.0);
                    if let Some(lp) = self.lib_preview.as_mut() {
                        lp.heartbeat.update(amplitude, ui.input(|i| i.stable_dt));
                        lp.heartbeat.paint(ui.painter(), rect, &palette);
                    }
                    ui.ctx().request_repaint();
                }
                resp.context_menu(|ui| {
                    let mut on = self.settings.audio_visualizer;
                    if ui.checkbox(&mut on, "Audio visualizer").changed() {
                        self.settings.audio_visualizer = on;
                        self.settings.save();
                    }
                    if ui.button("Close").clicked() {
                        ui.close();
                    }
                });
            }
        });

        if let Some(lp) = self.lib_preview.as_mut() {
            if toggle {
                lp.player.toggle();
            }
            if stop {
                lp.player.pause();
                lp.player.seek(0.0);
            }
            if let Some(t) = seek_to {
                lp.player.seek(t.clamp(0.0, duration));
            }
        }
        if close {
            self.lib_preview = None;
            self.lib_preview_tex = None;
            self.lib_preview_live = None;
        }
    }
}
