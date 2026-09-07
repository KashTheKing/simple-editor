use super::*;

impl App {
    pub(super) fn name_window(ctx: &egui::Context, title: &str, field: &mut Option<String>) -> Option<String> {
        let mut name = field.take()?;
        let mut open = true;
        let mut done = None;
        let mut cancel = false;
        egui::Window::new(title).open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            // focus only on the window's first frame — every frame would steal it from the panels behind it
            let id = ui.id().with("name");
            let first = ctx.read_response(id).is_none();
            let r = ui.add(egui::TextEdit::singleline(&mut name).id(id));
            if first {
                r.request_focus();
            }
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.horizontal(|ui| {
                if (ui.button("Save").clicked() || enter) && !name.trim().is_empty() {
                    done = Some(name.trim().to_string());
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
        if done.is_some() || cancel || !open {
            done
        } else {
            *field = Some(name);
            None
        }
    }

    pub(super) fn windows(&mut self, ctx: &egui::Context) {
        // ---- ws:registries-schema-hooks ----
        // A future workstream's non-blocking egui::Window (scopes, angle grid, cheat sheet, ...) is a
        // WINDOW_DRAWERS entry instead of a line added here. Nothing is registered yet.
        for f in WINDOW_DRAWERS {
            f(self, ctx);
        }
        self.url_window(ctx);
        self.compress_window(ctx);
        // "Convert To…" options (non-blocking; the timeline stays usable)
        if let Some((id, ext)) = self.convert_dialog.clone() {
            let name = self.project.asset(id).map(|a| a.name()).unwrap_or_default();
            let mut open = true;
            let mut start = false;
            let mut ext = ext;
            egui::Window::new("Convert To…").open(&mut open).resizable(false).show(ctx, |ui| {
                ui.label(&name);
                ui.horizontal(|ui| {
                    ui.label("Format");
                    egui::ComboBox::from_id_salt("convert_ext").selected_text(&ext).show_ui(ui, |ui| {
                        for t in crate::engine::convert::TARGETS {
                            ui.selectable_value(&mut ext, (*t).to_string(), *t);
                        }
                    });
                });
                ui.weak("Saved next to the source as <name>_converted.<ext> and added to the library.");
                start = ui.button("Convert").clicked();
            });
            match (open, start) {
                (_, true) => {
                    self.convert_dialog = None;
                    self.start_asset_convert(id, &ext);
                }
                (true, false) => self.convert_dialog = Some((id, ext)),
                (false, false) => self.convert_dialog = None,
            }
        }
        // background conversions / downloads: progress + cancel
        let convert_jobs: Vec<(Arc<Progress>, String)> = self
            .convert_jobs
            .iter()
            .map(|(p, o)| (p.clone(), o.file_name().unwrap_or_default().to_string_lossy().into_owned()))
            .collect();
        job_window(ctx, "Converting", &convert_jobs);
        let downloads: Vec<(Arc<Progress>, String)> =
            self.downloads.iter().map(|d| (d.progress.clone(), d.url.clone())).collect();
        job_window(ctx, "Downloading", &downloads);
        // retime (Ctrl+R)
        if self.retime.open {
            let changed = {
                let App { project, selection, playhead, undo, redo, retime, .. } = self;
                let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                retime::show(ctx, retime, project, selection, *playhead, &mut push)
            };
            if changed {
                self.after_edit();
            }
            // ---- ws:inspector-gallery ----
            if self.retime.want_curves {
                self.retime.want_curves = false;
                self.layout.reveal(Pane::Curves);
                self.layout_dirty = true;
            }
        }
        // export window
        if self.export_ui.open {
            self.detect_encoders_once();
            // the export always renders the MAIN timeline — show its size/lossless state, not the open sequence's
            let main = self.project.editing.is_some().then(|| self.export_project());
            let choice = {
                let App { project, settings, export_ui: st, encoders, export, .. } = self;
                export_ui::show(ctx, st, main.as_ref().unwrap_or(project), settings, encoders, export.is_some())
            };
            // ---- ws:export-deliver ----
            match choice {
                Some((c, false)) => {
                    self.start_export_choice(c);
                }
                Some((c, true)) => {
                    // the source-overwrite refusal runs again at pop time (start_export_choice); here it
                    // just saves the user a wait
                    if files::refuses_source(&self.project, &c.opts.out_path) {
                        self.toast(
                            "That file is a source of this project — use Overwrite Original Video (Ctrl+S) instead",
                        );
                    } else {
                        self.export_queue.push_back(c);
                        let n = self.export_queue.len();
                        self.toast(format!("Added to the render queue ({n} waiting)"));
                    }
                }
                None => {}
            }
        }
        // ---- ws:export-deliver: bakes (render in place / stabilize / denoise / slow-mo) ----
        let bakes: Vec<(Arc<Progress>, String)> = self.bake_jobs.iter().map(|j| (j.progress(), j.title())).collect();
        job_window(ctx, "Rendering in place", &bakes);
        // save template / save layout profile
        if let Some(name) = Self::name_window(ctx, "Save Template", &mut self.template_name) {
            let t = crate::engine::presets::capture_template(&name, &self.project, &self.selection);
            self.settings.templates.retain(|x| x.name != name);
            self.settings.templates.push(t);
            self.settings.save();
            self.toast(format!("Template '{name}' saved"));
        }
        if let Some(name) = Self::name_window(ctx, "Save Layout Profile", &mut self.profile_name) {
            let json = self.layout.to_json();
            self.settings.layout_profiles.retain(|x| x.name != name);
            self.settings.layout_profiles.push(crate::settings::LayoutProfile { name: name.clone(), json });
            self.settings.save();
            self.toast(format!("Layout profile '{name}' saved"));
        }
        // settings window
        if self.settings_ui.open {
            let backend_before = self.settings.decoder.clone();
            let cache_before = self.settings.cache_mb;
            let gpu_before = self.settings.gpu;
            let theme_before = self.settings.theme.clone();
            let look_before = self.settings.ui_look.clone();
            let palette_before = self.settings.palette.clone();
            let ctxmenu_before = self.settings.context_menu;
            // ---- ws:command-palette ----
            let keymap_before = self.settings.keymap_preset.clone();
            let ffdir_before = self.settings.ffmpeg_dir.clone();
            let ytdlp_dir_before = self.settings.ytdlp_dir.clone();
            self.detect_encoders_once();
            let mcp_status = match (&self.mcp, self.settings.mcp_enabled) {
                (Some((s, _)), _) => format!("running at {}", s.url()),
                (None, true) => "starting…".into(),
                (None, false) => "stopped".into(),
            };
            let inputs = self.audio_inputs();
            let changed = settings_ui::show(
                ctx,
                &mut self.settings_ui,
                &mut self.settings,
                &mut self.hotkeys,
                &self.encoders,
                &self.palette,
                &mcp_status,
                &self.gpu_name,
                &inputs,
            );
            // ---- ws:forgiveness ----
            if std::mem::take(&mut self.settings_ui.clear_caches) {
                caches::clear(self);
            }
            if changed {
                self.hotkeys.to_settings(&mut self.settings);
                self.settings.save();
                if self.settings.theme != theme_before
                    || self.settings.ui_look != look_before
                    || self.settings.palette != palette_before
                {
                    theme::apply(ctx, &self.settings.theme, &self.settings.palette, &self.settings.ui_look);
                }
                if self.settings.ffmpeg_dir != ffdir_before {
                    media::ffpipe::set_dir(&self.settings.ffmpeg_dir);
                    self.encoders.clear();
                }
                if self.settings.ytdlp_dir != ytdlp_dir_before {
                    media::ytdlp::set_dir(&self.settings.ytdlp_dir);
                }
                // the folder may have changed, and yt-dlp may have been installed since we last looked
                self.detect_ytdlp(ctx);
                if self.settings.gpu != gpu_before {
                    self.gpu_failed = false; // an explicit toggle retries the renderer
                }
                if self.settings.decoder != backend_before {
                    let b = self.backend();
                    self.player.set_backend(b);
                    self.waveforms.set_backend(b);
                    self.thumbs.set_backend(b);
                }
                if self.settings.cache_mb != cache_before {
                    self.player.set_cache_bytes(crate::playback::cache_budget_bytes(self.settings.cache_mb));
                }
                if self.settings.context_menu != ctxmenu_before {
                    let r = if self.settings.context_menu {
                        crate::contextmenu::install()
                    } else {
                        crate::contextmenu::uninstall()
                    };
                    if let Err(e) = r {
                        self.toast(format!("Context menu: {e}"));
                    }
                }
                // ---- ws:command-palette ----
                if self.settings.keymap_preset != keymap_before {
                    self.toast(format!("Keymap preset applied: {}", self.settings.keymap_preset));
                }
                // TODO(integration): text.lock().load_user_fonts(&settings.user_fonts) + refresh self.fonts
                // once the text rasterizer grows user-font support (engine-video agent).
            }
        }
        // "Export Frame…" (Ctrl+Shift+F)
        if self.frame_ui.open {
            let choice = {
                let App { frame_ui: st, project, settings, .. } = self;
                frame_ui::show(ctx, st, project, settings)
            };
            if let Some(c) = choice {
                self.settings.save();
                self.export_frame(c);
            }
        }
        // GLSL editor — Apply is an effect edit like any other (undo + re-render), and the compile log
        // goes straight back into the window so a rejected shader is never a silent no-op
        if shader_ui::show(ctx, &mut self.shader_ui) {
            if let Some((id, i)) = self.shader_ui.target {
                let src = self.shader_ui.src.clone();
                let snap = self.project.to_json();
                let changed = match self.project.clip_mut(id).and_then(|c| c.effects.get_mut(i)) {
                    Some(fx) if fx.kind == EffectKind::Shader && fx.shader != src => {
                        fx.shader = src.clone();
                        true
                    }
                    _ => false,
                };
                if changed {
                    push_undo_json(&mut self.undo, &mut self.redo, snap);
                    self.after_edit();
                }
                self.shader_ui.error = match self.gpu.as_mut() {
                    Some(g) => g.check_shader(&src).err().unwrap_or_default(),
                    None => "GPU renderer is off — this shader cannot be compiled or previewed.".into(),
                };
            }
        }
        // "Paste Attributes" (Ctrl+Alt+V) — one undo step for the whole paste
        if self.paste_ui.open {
            let name = self.attrs.as_ref().map(|c| c.name.clone()).unwrap_or_default();
            let targets = self.selection.len();
            let chosen = paste_ui::show(ctx, &mut self.paste_ui, &name, targets);
            if let Some(set) = chosen {
                if let Some(src) = self.attrs.clone() {
                    let snap = self.project.to_json();
                    let ids = self.selection.clone();
                    let n = self.project.paste_attributes(&src, &ids, set);
                    if n > 0 {
                        push_undo_json(&mut self.undo, &mut self.redo, snap);
                        self.after_edit();
                    }
                    self.toast(format!("Pasted attributes onto {n} clip(s)"));
                }
                self.paste_ui.open = false;
            }
        }
        // screen recording / voiceover
        if self.capture_ui.screen_open || self.capture_ui.voice_open {
            let resp = {
                let App { capture_ui: st, settings, palette, screen_rec, voice_rec, .. } = self;
                capture_ui::show(ctx, st, settings, screen_rec.is_some(), voice_rec.is_some(), palette)
            };
            if resp.stop_screen {
                self.stop_screen_capture();
            }
            if resp.stop_voice {
                self.stop_voiceover();
            }
            if let Some(o) = resp.screen.filter(|_| resp.start_screen) {
                self.start_screen_capture(o);
            }
            if let Some(o) = resp.voice.filter(|_| resp.start_voice) {
                self.start_voiceover(o);
            }
        }
        // imported timeline report — "Use this project" swaps it in
        if self.import_ui.open {
            let accept = {
                let App { import_ui: st, palette, .. } = self;
                import_ui::show(ctx, st, palette)
            };
            // ask first: cancelling the unsaved-changes prompt must keep the report (and the ffprobes
            // that built it), not throw the whole import away
            if accept {
                self.confirm_discard_then(move |app| {
                    if let Some(r) = app.import_ui.report.take() {
                        app.set_project(r.project, None);
                        app.toast("Imported timeline is now the project");
                    }
                    app.import_ui.open = false;
                });
            }
        }
        // export / convert progress — non-modal: keep editing while it runs
        if let Some((prog, kind)) = &self.export {
            let prog = prog.clone();
            let title = match kind {
                ExportKind::File { .. } => "Exporting…",
                ExportKind::Overwrite { .. } => "Saving over the original…",
            };
            egui::Window::new(title)
                .collapsible(false)
                .resizable(false)
                .default_pos(ctx.content_rect().center() - egui::vec2(180.0, 60.0))
                .show(ctx, |ui| {
                    ui.set_width(360.0);
                    ui.add(egui::ProgressBar::new(prog.fraction()).show_percentage());
                    ui.label(prog.status());
                    // ---- ws:export-deliver ----
                    let elapsed = crate::ui::duration_text(prog.elapsed().as_secs_f64());
                    match prog.eta() {
                        Some(eta) => {
                            ui.weak(format!("{elapsed} elapsed · ETA {}", crate::ui::duration_text(eta.as_secs_f64())))
                        }
                        None => ui.weak(format!("{elapsed} elapsed")),
                    };
                    let queued = self.export_queue.len();
                    if queued > 0 {
                        ui.weak(format!("{queued} more queued"));
                    }
                    if ui.button(if queued > 0 { "Cancel all" } else { "Cancel" }).clicked() {
                        prog.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
                        self.export_queue.clear();
                    }
                });
        }
    }
}
