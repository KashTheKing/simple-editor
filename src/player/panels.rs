//! The player's windows and menus beyond the bar: the right-click menu over the picture, Properties /
//! Metadata (every tag, the cover / thumbnail, Explorer's Tags, presets, Clear all metadata), Upscale…
//! (the editor's AI enhancer) and the visualizer's controls.

use super::chrome::{chips, flyout_frame, heading, SPEEDS};
use super::{PlayerApp, SKIP_BACK, SKIP_FORWARD};
use crate::engine::enhance::{self, Model};
use crate::engine::tags::{self, CoverEdit, MediaTags, MetaPreset, TagEdit, FIELDS};
use crate::engine::visualizer::Look;
use crate::theme::Palette;
use crate::ui::menu;
use crate::ui::tools::on_accent;
use eframe::egui::{self, vec2, Align2, Color32};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

// ------------------------------------------------------------------------------------------- metadata

/// What the Metadata panel reads off the file (in the background).
pub struct Loaded {
    pub tags: MediaTags,
    pub keywords: Vec<String>,
    /// The file type holds Explorer Tags.
    pub kw_ok: bool,
    pub cover: Option<egui::ColorImage>,
}

/// The Metadata panel while it is open.
#[derive(Default)]
pub struct Meta {
    pub rx: Option<Receiver<Result<Loaded, String>>>,
    pub loaded: Option<Result<Loaded, String>>,
    /// Field values as edited (every field the panel shows).
    pub draft: BTreeMap<String, String>,
    pub keywords: String,
    pub cover: CoverEdit,
    pub cover_tex: Option<egui::TextureHandle>,
    /// The cover is being read (or a new picture loaded).
    pub cover_busy: bool,
    pub confirm_clear: bool,
    /// "Save as preset": the name being typed.
    pub preset_name: Option<String>,
    pub status: String,
}

impl Meta {
    /// Start reading `path`'s tags, Explorer tags and cover.
    pub fn open(path: &Path, ctx: &egui::Context) -> Meta {
        let (tx, rx) = std::sync::mpsc::channel();
        let (p, c) = (path.to_path_buf(), ctx.clone());
        std::thread::spawn(move || {
            let r = tags::read(&p).map(|t| {
                let cover = t.cover.and_then(|_| {
                    let dir = std::env::temp_dir().join(format!("se-player-cover-{}", std::process::id()));
                    let out = tags::extract_cover(&p, &dir).ok().flatten()?;
                    let img = super::chrome::cover_art(&out);
                    let _ = std::fs::remove_file(&out);
                    img
                });
                Loaded { keywords: tags::read_keywords(&p), kw_ok: tags::keywords_writable(&p), cover, tags: t }
            });
            let _ = tx.send(r);
            c.request_repaint();
        });
        Meta { rx: Some(rx), cover_busy: true, ..Default::default() }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.rx else { return };
        let Ok(r) = rx.try_recv() else { return };
        self.rx = None;
        self.cover_busy = false;
        if let Ok(l) = &r {
            self.draft = FIELDS.iter().map(|f| (f.key.to_string(), l.tags.get(f.key).to_string())).collect();
            self.keywords = l.keywords.join("; ");
            self.cover = CoverEdit::Keep;
            self.cover_tex = l.cover.clone().map(|img| ctx.load_texture("player-meta-cover", img, egui::TextureOptions::LINEAR));
        }
        self.loaded = Some(r);
    }

    /// The write the panel's edits are (None: nothing changed), and the Explorer tags to set.
    pub fn edit(&self) -> Option<(TagEdit, Option<Vec<String>>)> {
        let Some(Ok(l)) = &self.loaded else { return None };
        let set: BTreeMap<String, String> =
            self.draft.iter().filter(|(k, v)| l.tags.get(k) != v.as_str()).map(|(k, v)| (k.clone(), v.clone())).collect();
        let kw = tags::split_keywords(&self.keywords);
        let kw = (l.kw_ok && kw != l.keywords).then_some(kw);
        let e = TagEdit { set, cover: self.cover.clone(), clear: false };
        (!e.is_empty() || kw.is_some()).then_some((e, kw))
    }

    /// Clear all metadata: every tag, the cover, the Explorer tags.
    pub fn clear_edit(&self) -> Option<(TagEdit, Option<Vec<String>>)> {
        let Some(Ok(l)) = &self.loaded else { return None };
        let cover = if l.tags.cover.is_some() { CoverEdit::Remove } else { CoverEdit::Keep };
        Some((TagEdit { clear: true, cover, ..Default::default() }, (l.kw_ok && !l.keywords.is_empty()).then(Vec::new)))
    }

    /// What Clear all metadata removes, in words (for its confirm).
    pub fn clear_summary(&self) -> String {
        let Some(Ok(l)) = &self.loaded else { return String::new() };
        let names: Vec<&str> = FIELDS.iter().filter(|f| !l.tags.get(f.key).is_empty()).map(|f| f.label).collect();
        let mut parts = Vec::new();
        parts.push(if names.is_empty() {
            "any tags the file holds (encoder, dates, chapters' names)".to_string()
        } else {
            format!("{} ({}), and any other tags the file holds", names.join(", "), names.len())
        });
        if l.tags.cover.is_some() {
            parts.push("the cover picture".into());
        }
        if !l.keywords.is_empty() {
            parts.push(format!("the Explorer tags ({})", l.keywords.join("; ")));
        }
        format!("Removes {}. The sound and picture stay.", parts.join(", "))
    }

    /// A preset's filled fields (and Explorer tags) into the draft.
    pub fn apply_preset(&mut self, p: &MetaPreset) {
        for (k, v) in p.values.iter().filter(|(_, v)| !v.trim().is_empty()) {
            self.draft.insert(k.clone(), v.clone());
        }
        if !p.keywords.trim().is_empty() {
            self.keywords = p.keywords.trim().to_string();
        }
    }

    /// The draft as a preset named `name` (the title and the numbers are the file's own, not kept).
    pub fn to_preset(&self, name: &str) -> MetaPreset {
        let values = self
            .draft
            .iter()
            .filter(|(k, v)| !v.trim().is_empty() && !matches!(k.as_str(), "title" | "track" | "disc" | "lyrics"))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        MetaPreset { name: name.trim().to_string(), values, keywords: self.keywords.clone() }
    }
}

/// Properties / Metadata.
pub fn metadata_window(app: &mut PlayerApp, ctx: &egui::Context, pal: &Palette) {
    let Some(mut m) = app.meta.take() else { return };
    m.poll(ctx);
    let path = app.path().to_path_buf();
    let c = tags::container(&path);
    let has_video = app.info.as_ref().is_some_and(|i| i.has_video);
    let saving = app.meta_save.is_some();
    let mut open = true;
    let mut save: Option<(TagEdit, Option<Vec<String>>)> = None;
    let mut save_preset: Option<MetaPreset> = None;
    egui::Window::new("Properties")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .frame(flyout_frame(pal, ctx).inner_margin(egui::Margin::same(14)))
        .show(ctx, |ui| {
            ui.set_width(560.0);
            let l = match &m.loaded {
                None => {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(14.0));
                        ui.label(egui::RichText::new("Reading the file's tags…").color(pal.text));
                    });
                    return;
                }
                Some(Err(e)) => {
                    ui.label(egui::RichText::new(format!("Can't read the tags: {e}")).color(pal.text));
                    return;
                }
                Some(Ok(l)) => l,
            };
            let (had_cover, kw_ok) = (l.tags.cover.is_some(), l.kw_ok);
            ui.add_enabled_ui(!saving, |ui| {
                ui.horizontal_top(|ui| {
                    // ---- the cover / thumbnail
                    ui.vertical(|ui| {
                        ui.set_width(150.0);
                        let (rect, _) = ui.allocate_exact_size(vec2(150.0, 150.0), egui::Sense::hover());
                        let showing = !matches!(m.cover, CoverEdit::Remove);
                        match (&m.cover_tex, showing) {
                            _ if m.cover_busy => super::chrome::shimmer(ui, rect, pal),
                            (Some(t), true) => {
                                let s = t.size_vec2();
                                let k = (150.0 / s.x).min(150.0 / s.y);
                                ui.painter().image(t.id(), egui::Rect::from_center_size(rect.center(), s * k), egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                            }
                            _ => {
                                ui.painter().rect_filled(rect, 6.0, pal.text.gamma_multiply(0.06));
                                ui.painter().text(rect.center(), Align2::CENTER_CENTER, if has_video { "No thumbnail" } else { "No cover" }, egui::FontId::proportional(12.0), pal.text_dim);
                            }
                        }
                        ui.add_space(6.0);
                        if c.can_cover() {
                            if ui.button(if has_video { "Change thumbnail…" } else { "Change cover…" }).clicked() {
                                if let Some(p) = rfd::FileDialog::new().add_filter("Pictures", &["jpg", "jpeg", "png", "webp", "bmp"]).pick_file() {
                                    m.cover_tex = super::chrome::cover_art(&p).map(|img| ctx.load_texture("player-meta-cover", img, egui::TextureOptions::LINEAR));
                                    m.cover = CoverEdit::Set(p);
                                }
                            }
                            if has_video && ui.button("Use the current frame").clicked() {
                                let out = std::env::temp_dir().join(format!("se-player-frame-{}.jpg", std::process::id()));
                                match tags::grab_frame(&path, app.engine.time(), &out) {
                                    Ok(()) => {
                                        m.cover_tex = super::chrome::cover_art(&out).map(|img| ctx.load_texture("player-meta-cover", img, egui::TextureOptions::LINEAR));
                                        m.cover = CoverEdit::Set(out);
                                    }
                                    Err(e) => m.status = format!("Can't take that frame: {e}"),
                                }
                            }
                            if (had_cover || matches!(m.cover, CoverEdit::Set(_))) && ui.button("Remove").clicked() {
                                m.cover = if had_cover { CoverEdit::Remove } else { CoverEdit::Keep };
                            }
                        } else {
                            ui.label(egui::RichText::new(format!("{} files can't hold a {}", ext(&path), if has_video { "thumbnail" } else { "cover" })).size(12.0).color(pal.text_dim));
                        }
                    });
                    ui.add_space(12.0);
                    // ---- the tags
                    ui.vertical(|ui| {
                        egui::Grid::new("player-meta").num_columns(2).spacing(vec2(10.0, 5.0)).show(ui, |ui| {
                            for f in FIELDS.iter().filter(|f| (if has_video { f.video } else { f.audio }) && c.keeps(f.key)) {
                                let label = if f.key == "artist" && has_video { "Author" } else { f.label };
                                ui.label(egui::RichText::new(label).color(pal.text_dim));
                                let v = m.draft.entry(f.key.to_string()).or_default();
                                let edit = if f.multiline { egui::TextEdit::multiline(v).desired_rows(2) } else { egui::TextEdit::singleline(v) };
                                ui.add(edit.desired_width(300.0));
                                ui.end_row();
                            }
                            ui.label(egui::RichText::new("Tags").color(pal.text_dim)).on_hover_text("Explorer's Tags (Details ▸ Tags), separated by ;");
                            if kw_ok {
                                ui.add(egui::TextEdit::singleline(&mut m.keywords).desired_width(300.0).hint_text("travel; 2026"));
                            } else {
                                ui.label(egui::RichText::new(format!("{} files can't hold Explorer tags ({} can)", ext(&path), tags::KEYWORD_FORMATS.join(", ").to_uppercase())).size(12.0).color(pal.text_dim));
                            }
                            ui.end_row();
                        });
                    });
                });
                ui.add_space(8.0);
                // ---- presets
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("player-meta-preset").selected_text("Apply preset").show_ui(ui, |ui| {
                        if app.presets.is_empty() {
                            ui.weak("No presets yet");
                        }
                        for p in &app.presets {
                            if ui.selectable_label(false, &p.name).clicked() {
                                m.apply_preset(p);
                            }
                        }
                    });
                    match m.preset_name.take() {
                        Some(mut name) => {
                            let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(150.0).hint_text("Preset name"));
                            r.request_focus();
                            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                            m.preset_name = Some(name.clone());
                            if (ui.button("Save").clicked() || enter) && !name.trim().is_empty() {
                                save_preset = Some(m.to_preset(&name));
                                m.preset_name = None;
                            } else if ui.button("Cancel").clicked() {
                                m.preset_name = None;
                            }
                        }
                        None => {
                            if ui.button("Save as preset…").clicked() {
                                m.preset_name = Some(String::new());
                            }
                        }
                    }
                });
                ui.add_space(8.0);
                ui.separator();
                // ---- save / clear
                if m.confirm_clear {
                    ui.label(egui::RichText::new("Clear all metadata?").strong().color(pal.text));
                    ui.label(egui::RichText::new(m.clear_summary()).color(pal.text));
                    ui.horizontal(|ui| {
                        let b = egui::Button::new(egui::RichText::new("Clear all").color(on_accent(pal.accent))).fill(pal.accent);
                        if ui.add(b).clicked() {
                            save = m.clear_edit();
                            m.confirm_clear = false;
                        }
                        if ui.button("Cancel").clicked() {
                            m.confirm_clear = false;
                        }
                    });
                } else {
                    ui.horizontal(|ui| {
                        let pending = m.edit();
                        let b = egui::Button::new(egui::RichText::new("Save").color(on_accent(pal.accent))).fill(pal.accent);
                        if ui.add_enabled(pending.is_some(), b).clicked() {
                            save = pending;
                        }
                        if ui.add_enabled(m.edit().is_some(), egui::Button::new("Revert")).clicked() {
                            m.loaded = None;
                            m = Meta::open(&path, ctx);
                        }
                        if ui.button("Clear all metadata…").clicked() {
                            m.confirm_clear = true;
                        }
                    });
                }
            });
            if saving {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(14.0));
                    ui.label(egui::RichText::new("Saving…").color(pal.text));
                });
            } else if !m.status.is_empty() {
                ui.label(egui::RichText::new(&m.status).size(12.0).color(pal.text_dim));
            }
        });
    if let Some(p) = save_preset {
        app.save_preset(p);
        m.status = "Preset saved".into();
    }
    if let Some((edit, kw)) = save {
        m.status.clear();
        app.meta = Some(m);
        app.save_meta(ctx, edit, kw);
        return;
    }
    if open {
        app.meta = Some(m);
    }
}

fn ext(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_else(|| "These".into())
}

// -------------------------------------------------------------------------------------------- upscale

/// Upscale…'s choices.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UpscaleSpec {
    pub scale: u32,
    pub model: Model,
}

impl Default for UpscaleSpec {
    fn default() -> Self {
        UpscaleSpec { scale: 2, model: Model::Real }
    }
}

/// Where an upscale of `src` goes: beside it, never over a file.
pub fn upscale_out(src: &Path, spec: UpscaleSpec, still: bool) -> PathBuf {
    let name = enhance::output_name(src, spec.scale, spec.model, still);
    let (stem, e) = name.rsplit_once('.').unwrap_or((&name, "mp4"));
    let dir = src.parent().unwrap_or(Path::new("."));
    let mut out = dir.join(format!("{stem}.{e}"));
    let mut n = 2;
    while out.exists() {
        out = dir.join(format!("{stem}_{n}.{e}"));
        n += 1;
    }
    out
}

/// Upscale…: scale and model, then a background job (progress and Cancel at the bottom). Without the
/// enhancer installed, its install button instead.
pub fn upscale_window(app: &mut PlayerApp, ctx: &egui::Context, pal: &Palette) {
    let Some(mut spec) = app.upscale.take() else { return };
    let (mut open, mut go) = (true, false);
    egui::Window::new("Upscale")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .frame(flyout_frame(pal, ctx).inner_margin(egui::Margin::same(14)))
        .show(ctx, |ui| {
            ui.set_width(340.0);
            if !enhance::available() {
                ui.label(egui::RichText::new("The AI enhancer isn't installed").color(pal.text));
                ui.add_space(6.0);
                match enhance::installing() {
                    Some(p) => {
                        ui.add(egui::ProgressBar::new(p.fraction()).desired_width(300.0).fill(pal.accent).text(p.status()));
                        ctx.request_repaint_after(std::time::Duration::from_millis(250));
                    }
                    None => {
                        let b = egui::Button::new(egui::RichText::new("Install the AI enhancer").color(on_accent(pal.accent))).fill(pal.accent);
                        if ui.add(b).clicked() {
                            enhance::start_install();
                        }
                        if let Some(e) = enhance::install_error() {
                            ui.label(egui::RichText::new(e).size(12.0).color(pal.text_dim));
                        }
                    }
                }
                return;
            }
            heading(ui, pal, "Scale");
            let scales = [2u32, 3, 4];
            let labels: Vec<String> = scales.iter().map(|s| format!("{s}×")).collect();
            if let Some(i) = chips(ui, pal, &labels, scales.iter().position(|s| *s == spec.scale).unwrap_or(0)) {
                spec.scale = scales[i];
            }
            heading(ui, pal, "Model");
            let models = [Model::Real, Model::Game];
            let labels = vec!["Real-world".to_string(), "Games and animation".to_string()];
            if let Some(i) = chips(ui, pal, &labels, models.iter().position(|m| *m == spec.model).unwrap_or(0)) {
                spec.model = models[i];
            }
            if let Some(i) = &app.info {
                ui.add_space(4.0);
                let (w, h) = i.size;
                ui.label(egui::RichText::new(format!("{w} × {h} → {} × {}", w * spec.scale, h * spec.scale)).size(12.0).color(pal.text_dim));
            }
            ui.add_space(8.0);
            let busy = app.upscale_job.is_some();
            ui.horizontal(|ui| {
                let b = egui::Button::new(egui::RichText::new("Upscale").color(on_accent(pal.accent))).fill(pal.accent);
                if ui.add_enabled(!busy, b).on_disabled_hover_text("An upscale is running").clicked() {
                    go = true;
                }
                ui.label(egui::RichText::new("Saved next to the file").size(12.0).color(pal.text_dim));
            });
        });
    if go {
        app.start_upscale(spec);
    } else if open {
        app.upscale = Some(spec);
    }
}

// ----------------------------------------------------------------------------------------- visualizer

/// The visualizer's controls (Audio Only or a song without a cover): the style, then how it reacts -
/// intensity, sensitivity, a band preset - and the style's own params (colour, glow, palette ...).
/// Adjustable before playing; saved with the editor's settings (`Settings::visualizer_look`).
pub fn visualizer_controls(app: &mut PlayerApp, ui: &mut egui::Ui, pal: &Palette) {
    let cur = crate::engine::visualizer::last_used(&app.viz.0);
    heading(ui, pal, "Style");
    let names = crate::engine::visualizer::style_names();
    if let Some(i) = chips(ui, pal, &names, names.iter().position(|n| *n == cur.name).unwrap_or(0)) {
        app.set_visualizer(&names[i]);
    }
    let mut look = app.vis_look.clone();
    let slider = |ui: &mut egui::Ui, label: &str, v: &mut f32, lo: f32, hi: f32| {
        ui.horizontal(|ui| {
            ui.add_sized(vec2(80.0, 18.0), egui::Label::new(egui::RichText::new(label).color(pal.text)));
            ui.add(egui::Slider::new(v, lo..=hi).fixed_decimals(2));
        });
    };
    heading(ui, pal, "Reacts");
    slider(ui, "Intensity", &mut look.intensity, 0.25, 3.0);
    slider(ui, "Sensitivity", &mut look.mix.sensitivity, 0.25, 4.0);
    heading(ui, pal, "Bands");
    let mut presets = vec!["Flat".to_string()];
    presets.extend(crate::model::AudioMix::PRESETS.iter().map(|s| s.to_string()));
    let on = presets.iter().position(|p| *p == look.mix.preset).unwrap_or(0);
    if let Some(i) = chips(ui, pal, &presets, on) {
        let sens = look.mix.sensitivity;
        look.mix = if i == 0 { Default::default() } else { crate::model::AudioMix::preset(&presets[i]).unwrap_or_default() };
        look.mix.sensitivity = sens;
    }
    let params = Look::look_params(&cur.src);
    if !params.is_empty() {
        heading(ui, pal, "Look");
        for p in params {
            let mut v = look.value(&cur.name, &p.id, p.default) as f32;
            let label = p.id.replace('_', " ");
            ui.horizontal(|ui| {
                ui.add_sized(vec2(80.0, 18.0), egui::Label::new(egui::RichText::new(capitalise(&label)).color(pal.text)));
                if ui.add(egui::Slider::new(&mut v, p.min as f32..=p.max as f32).fixed_decimals(2)).changed() {
                    look.set(&cur.name, &p.id, v as f64);
                }
            });
        }
    }
    if ui.button("Reset").clicked() {
        look = Look::default();
    }
    if look != app.vis_look {
        app.vis_look = look;
        app.vis_dirty = true;
    }
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

// ---------------------------------------------------------------------------------- right-click menu

/// Right-click on the picture: the editor's menu rows, with what the bar and its flyouts hold.
pub fn context_menu(app: &mut PlayerApp, ui: &mut egui::Ui, ctx: &egui::Context) {
    let playing = app.engine.is_playing();
    if menu::row(ui, None, if playing { "Pause" } else { "Play" }, "Space").clicked() {
        app.toggle_play();
    }
    if menu::check(ui, app.engine.is_reverse(), "Reverse", "").clicked() {
        app.toggle_reverse();
    }
    let looping = app.looping();
    if menu::check(ui, looping, "Loop", "").clicked() {
        app.set_loop(!looping);
    }
    if menu::row(ui, None, "Back 10 seconds", "Left").clicked() {
        app.skip(-SKIP_BACK);
    }
    if menu::row(ui, None, "Forward 10 seconds", "Right").clicked() {
        app.skip(SKIP_FORWARD);
    }
    ui.separator();
    menu::sub(ui, None, "Speed", |ui| {
        let rate = app.engine.rate();
        for s in SPEEDS {
            let label = if s == 1.0 { "Normal".to_string() } else { super::chrome::speed_label(s) };
            if menu::check(ui, (s - rate).abs() < 1e-6, &label, "").clicked() {
                app.engine.set_rate(s);
            }
        }
    });
    let has_video = app.info.as_ref().is_some_and(|i| i.has_video);
    if has_video {
        menu::sub(ui, None, "Resolution", |ui| {
            if menu::check(ui, app.res.is_none(), "Auto", "").clicked() {
                app.set_resolution(None);
            }
            for h in app.res_options() {
                if menu::check(ui, app.res == Some(h), &format!("{h}p"), "").clicked() {
                    app.set_resolution(Some(h));
                }
            }
        });
    }
    let streams = app.info.as_ref().map(|i| i.audio_streams.clone()).unwrap_or_default();
    if !streams.is_empty() {
        menu::sub(ui, None, "Audio tracks", |ui| {
            let on = app.engine.audio_streams();
            let none = on.as_ref().is_some_and(|v| v.is_empty());
            if menu::check(ui, none, "None", "").clicked() {
                app.set_audio_selection(if none { (0..streams.len()).collect() } else { vec![] }, streams.len());
            }
            let names = app.tracks().cloned();
            for (n, a) in streams.iter().enumerate() {
                let name = names.as_ref().and_then(|t| t.audio.get(n)).map_or_else(|| a.label(), |t| t.label.clone());
                let ticked = on.as_ref().is_none_or(|v| v.contains(&n));
                if menu::check(ui, ticked, &name, "").clicked() {
                    app.set_audio_track(n, !ticked, streams.len());
                }
            }
        });
    }
    menu::sub(ui, None, "Subtitles", |ui| {
        let cur = app.subtitle.as_ref().map(|(n, _)| *n);
        if menu::check(ui, cur.is_none(), "Off", "").clicked() {
            app.set_subtitle(None);
        }
        match app.tracks().cloned() {
            None => {
                ui.weak("Reading tracks…");
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
            Some(t) => {
                for (i, s) in t.subtitles.iter().enumerate() {
                    if s.bitmap {
                        ui.add_enabled_ui(false, |ui| menu::row(ui, None, &format!("{} (picture)", s.label), ""));
                    } else if menu::check(ui, cur == Some(i), &s.label, "").clicked() {
                        app.set_subtitle(Some(i));
                    }
                }
            }
        }
    });
    ui.separator();
    if has_video {
        let ao = app.engine.audio_only();
        if menu::check(ui, ao, "Audio Only", "").clicked() {
            app.set_audio_only(!ao);
        }
    } else {
        let viz = app.song_viz;
        if menu::check(ui, viz, "Visualizer", "").on_hover_text("Instead of the cover").clicked() {
            app.set_song_visualizer(!viz);
        }
    }
    if !streams.is_empty() {
        let on = app.wave_seek_on();
        let tip = "Songs and Audio Only: hovering the seek bar shows the whole song's waveform";
        if menu::check(ui, on, "Waveform seek bar", "").on_hover_text(tip).clicked() {
            app.set_wave_seek(!on);
        }
    }
    menu::sub(ui, None, "Visualizer style", |ui| {
        ui.set_width(300.0);
        let pal = crate::theme::palette_with(ctx, &app.look);
        visualizer_controls(app, ui, &pal);
    });
    ui.separator();
    if menu::check(ui, app.fullscreen, "Full screen", "F").clicked() {
        app.toggle_fullscreen(ctx);
    }
    if menu::check(ui, app.mini.is_some(), "Mini player", "").clicked() {
        app.toggle_mini(ctx);
    }
    ui.separator();
    if has_video && menu::row(ui, None, "Screenshot frame", "").clicked() {
        app.screenshot();
    }
    if menu::row(ui, None, "Convert…", "").clicked() {
        app.convert = Some(Default::default());
    }
    if menu::row(ui, None, "Upscale…", "").clicked() {
        app.upscale = Some(Default::default());
    }
    if menu::row(ui, None, "Properties / Metadata…", "").clicked() {
        app.open_metadata(ctx);
    }
    ui.separator();
    if menu::row(ui, None, "Rename", "").clicked() {
        app.renaming = Some(app.title());
    }
    if menu::row(ui, None, "Open in Editor", "Esc").clicked() {
        app.open_editor();
    }
    if menu::row(ui, None, "Show in folder", "").clicked() {
        app.open_folder();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loaded(vals: &[(&str, &str)], cover: bool, kw: &[&str]) -> Meta {
        let mut t = MediaTags::default();
        for (k, v) in vals {
            t.values.insert(k.to_string(), v.to_string());
        }
        t.cover = cover.then_some(1);
        let l = Loaded { tags: t, keywords: kw.iter().map(|s| s.to_string()).collect(), kw_ok: true, cover: None };
        let mut m = Meta::default();
        m.draft = FIELDS.iter().map(|f| (f.key.to_string(), l.tags.get(f.key).to_string())).collect();
        m.keywords = l.keywords.join("; ");
        m.loaded = Some(Ok(l));
        m
    }

    #[test]
    fn the_panel_writes_only_what_changed() {
        let mut m = loaded(&[("title", "Song"), ("artist", "A")], false, &["x"]);
        assert!(m.edit().is_none(), "nothing edited, nothing to save");
        m.draft.insert("artist".into(), "B".into());
        let (e, kw) = m.edit().unwrap();
        assert_eq!(e.set.into_iter().collect::<Vec<_>>(), [("artist".to_string(), "B".to_string())]);
        assert!(kw.is_none(), "tags untouched");
        m.keywords = "x; y".into();
        assert_eq!(m.edit().unwrap().1.unwrap(), ["x", "y"]);
    }

    #[test]
    fn clear_all_says_what_it_removes_and_removes_it() {
        let m = loaded(&[("title", "Song"), ("genre", "Pop")], true, &["road"]);
        let s = m.clear_summary();
        assert!(s.contains("Title, Genre (2)") && s.contains("cover picture") && s.contains("road"), "{s}");
        let (e, kw) = m.clear_edit().unwrap();
        assert!(e.clear && e.cover == CoverEdit::Remove && kw == Some(vec![]));
    }

    #[test]
    fn presets_fill_the_draft_and_the_draft_makes_a_preset() {
        let mut m = loaded(&[("title", "Song")], false, &[]);
        let mut p = MetaPreset { name: "Mine".into(), keywords: "a; b".into(), ..Default::default() };
        p.values.insert("artist".into(), "Kash".into());
        p.values.insert("genre".into(), "".into());
        m.apply_preset(&p);
        assert_eq!((m.draft["artist"].as_str(), m.draft["title"].as_str(), m.keywords.as_str()), ("Kash", "Song", "a; b"));
        let back = m.to_preset(" Again ");
        assert_eq!(back.name, "Again");
        assert!(!back.values.contains_key("title"), "a file's own title is not part of a preset");
        assert_eq!(back.values.get("artist").map(String::as_str), Some("Kash"));
    }

    #[test]
    fn an_upscale_never_overwrites() {
        let d = std::env::temp_dir().join(format!("se-player-up-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let src = d.join("clip.mp4");
        let spec = UpscaleSpec { scale: 2, model: Model::Real };
        let first = upscale_out(&src, spec, false);
        assert_eq!(first.parent(), Some(d.as_path()));
        std::fs::write(&first, b"x").unwrap();
        let second = upscale_out(&src, spec, false);
        assert_ne!(first, second);
        let _ = std::fs::remove_dir_all(&d);
    }
}
