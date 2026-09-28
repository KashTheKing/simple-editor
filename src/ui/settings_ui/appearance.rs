//! The Appearance tab: look (cozy/sharp), theme preset picker + import/export, per-`Palette`-field
//! colour overrides, background image, and per-pane/action icon overrides. Extracted verbatim from
//! settings_ui.rs (see mod.rs's module doc for the whole settings window).

use crate::settings::Settings;
use crate::ui::tools::Glyph;
use eframe::egui;

/// Mutates `s.palette` directly, so `theme::palette_with` (recomputed every frame in `App::update`)
/// picks it up immediately - no restart, no extra plumbing.
pub(super) fn appearance(ui: &mut egui::Ui, s: &mut Settings) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label("Look");
        changed |= crate::ui::combo(
            ui,
            "ui_look",
            &mut s.ui_look,
            &[("cozy", "Cozy (rounded)"), ("sharp", "Sharp")],
            Some(200.0),
        );
    });
    ui.add_space(4.0);
    let has_overrides =
        s.palette != crate::theme::PaletteOverride { mode: s.palette.mode.clone(), ..Default::default() };
    ui.horizontal(|ui| {
        ui.label("Theme");
        // one list: the three base modes, every built-in preset, and "Custom" once colours are tweaked
        let cur = if !s.palette.preset.is_empty() {
            s.palette.preset.clone()
        } else if has_overrides {
            "Custom".to_string()
        } else {
            match s.theme.as_str() {
                "dark" => "Dark",
                "light" => "Light",
                _ => "Follow System",
            }
            .to_string()
        };
        egui::ComboBox::from_id_salt("theme_preset").selected_text(cur.clone()).width(200.0).show_ui(ui, |ui| {
            for (label, theme) in [("Follow System", "system"), ("Dark", "dark"), ("Light", "light")] {
                if ui.selectable_label(cur == label, label).clicked() {
                    s.theme = theme.into();
                    s.palette = crate::theme::PaletteOverride {
                        mode: if theme == "system" { String::new() } else { theme.into() },
                        ..Default::default()
                    };
                    changed = true;
                }
            }
            ui.separator();
            for name in crate::theme::PRESETS {
                if ui.selectable_label(s.palette.preset == *name, *name).clicked() {
                    if let Some(ov) = crate::theme::preset(name) {
                        s.theme = ov.mode.clone();
                        s.palette = ov;
                        changed = true;
                    }
                }
            }
        });
        if ui.button("Export theme…").clicked() {
            let tf = crate::settings::ThemeFile {
                name: if s.palette.preset.is_empty() { "My theme".into() } else { s.palette.preset.clone() },
                theme: s.theme.clone(),
                ui_look: s.ui_look.clone(),
                palette: s.palette.clone(),
                bg_image: s.bg_image.clone(),
                bg_tint: s.bg_tint,
                bg_blur: s.bg_blur,
                panel_opacity: s.panel_opacity,
            };
            if let Some(out) = rfd::FileDialog::new()
                .add_filter("Simple Editor theme", &["sedit-theme"])
                .set_file_name("theme.sedit-theme")
                .save_file()
            {
                let _ = std::fs::write(&out, serde_json::to_string_pretty(&tf).unwrap_or_default());
            }
        }
        if ui.button("Import theme…").clicked() {
            if let Some(p) =
                rfd::FileDialog::new().add_filter("Simple Editor theme", &["sedit-theme", "json"]).pick_file()
            {
                if let Some(tf) = std::fs::read_to_string(&p)
                    .ok()
                    .and_then(|t| serde_json::from_str::<crate::settings::ThemeFile>(&t).ok())
                {
                    s.theme = tf.theme;
                    s.ui_look = tf.ui_look;
                    s.palette = tf.palette;
                    s.bg_image = tf.bg_image;
                    s.bg_tint = tf.bg_tint;
                    s.bg_blur = tf.bg_blur;
                    s.panel_opacity = tf.panel_opacity;
                    s.save(); // belt and braces: persist right away, whatever happens to this frame's flow
                    changed = true;
                }
            }
        }
    });
    ui.weak("Tweak any colour below to start a Custom theme from the one selected above.");
    ui.add_space(6.0);

    let base = crate::theme::palette_with(ui.ctx(), &s.palette);
    ui.horizontal(|ui| {
        ui.label("Preview");
        for c in [
            base.bg,
            base.panel,
            base.header,
            base.border,
            base.text,
            base.text_dim,
            base.accent,
            base.selection,
            base.keyframe,
            base.waveform,
        ] {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 2.0, c);
            ui.painter().rect_stroke(rect, 2.0, (1.0, base.border), egui::StrokeKind::Outside);
        }
    });
    ui.add_space(6.0);

    let mut tweaked = false;
    egui::Grid::new("appearance").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        tweaked |= color_row(ui, "Background", &mut s.palette.background, base.bg);
        tweaked |= color_row(ui, "Panel", &mut s.palette.panel, base.panel);
        tweaked |= color_row(ui, "Header", &mut s.palette.header, base.header);
        tweaked |= color_row(ui, "Border", &mut s.palette.border, base.border);
        tweaked |= color_row(ui, "Text", &mut s.palette.text, base.text);
        tweaked |= color_row(ui, "Text (dim)", &mut s.palette.text_dim, base.text_dim);
        tweaked |= color_row(ui, "Accent", &mut s.palette.accent, base.accent);
        tweaked |= color_row(ui, "Selection", &mut s.palette.selection, base.selection);
        tweaked |= color_row(ui, "Keyframe", &mut s.palette.keyframe, base.keyframe);
        tweaked |= color_row(ui, "Waveform", &mut s.palette.waveform, base.waveform);
    });
    ui.add_space(4.0);
    ui.collapsing("Clip colours", |ui| {
        egui::Grid::new("clip_colours").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            tweaked |= color_row(ui, "Video clips", &mut s.palette.clip_video, base.clip_video);
            tweaked |= color_row(ui, "Audio clips", &mut s.palette.clip_audio, base.clip_audio);
            tweaked |= color_row(ui, "Image clips", &mut s.palette.clip_image, base.clip_image);
            tweaked |= color_row(ui, "Text clips", &mut s.palette.clip_text, base.clip_text);
            tweaked |= color_row(ui, "Sequence clips", &mut s.palette.clip_sequence, base.clip_sequence);
            tweaked |= color_row(ui, "Shape clips", &mut s.palette.clip_shape, base.clip_shape);
            tweaked |= color_row(ui, "Adjustment clips", &mut s.palette.clip_adjust, base.clip_adjust);
        });
        ui.weak("A clip's custom label colour (right-click a clip) still wins over these.");
    });
    if tweaked {
        // editing a colour forks the current theme into "Custom" - the colours stay as a starting point
        s.palette.preset = String::new();
        if s.palette.mode.is_empty() || s.palette.mode == "system" {
            s.palette.mode = "custom".into(); // keep following Windows dark/light underneath
        }
        changed = true;
    }
    ui.add_space(8.0);
    ui.collapsing("Background image", |ui| {
        ui.weak("A picture behind the whole editor; lower the panel opacity to see it through the panes.");
        ui.horizontal(|ui| {
            ui.label("Image");
            let name = if s.bg_image.is_empty() {
                "(none)".to_string()
            } else {
                std::path::Path::new(&s.bg_image)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| s.bg_image.clone())
            };
            ui.label(name).on_hover_text(&s.bg_image);
            if ui.button("Browse…").clicked() {
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter("Images", &["png", "jpg", "jpeg", "bmp", "webp", "gif"])
                    .pick_file()
                {
                    s.bg_image = p.to_string_lossy().into_owned();
                    changed = true;
                }
            }
            if !s.bg_image.is_empty() && ui.button("Clear").clicked() {
                s.bg_image.clear();
                changed = true;
            }
        });
        ui.add_enabled_ui(!s.bg_image.is_empty(), |ui| {
            ui.horizontal(|ui| {
                ui.label("Tint");
                let mut c =
                    egui::Color32::from_rgba_unmultiplied(s.bg_tint[0], s.bg_tint[1], s.bg_tint[2], s.bg_tint[3]);
                if egui::color_picker::color_edit_button_srgba(ui, &mut c, egui::color_picker::Alpha::OnlyBlend)
                    .changed()
                {
                    let [r, g, b, a] = c.to_srgba_unmultiplied();
                    s.bg_tint = [r, g, b, a];
                    changed = true;
                }
                ui.label("Blur");
                changed |= ui.add(egui::DragValue::new(&mut s.bg_blur).range(0..=20)).changed();
            });
            ui.horizontal(|ui| {
                ui.label("Panel opacity");
                changed |= ui.add(egui::Slider::new(&mut s.panel_opacity, 60..=255)).changed();
            });
        });
    });
    ui.add_space(8.0);
    ui.collapsing("Icons", |ui| {
        ui.weak("Pick the glyph shown for each pane and menu action ('None' removes it, 'Default' restores).");
        egui::ScrollArea::vertical().max_height((ui.available_height() - 40.0).max(0.0)).show(ui, |ui| {
            egui::Grid::new("icons_panes").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
                for &p in crate::ui::layout::Pane::ALL {
                    changed |= icon_row(ui, s, format!("pane.{}", p.title()), p.title(), Some(p.glyph()));
                }
                for a in crate::hotkeys::Action::ALL {
                    changed |=
                        icon_row(ui, s, format!("action.{}", a.id()), a.label(), crate::ui::tools::action_glyph(*a));
                }
            });
        });
    });
    changed
}

/// One icon assignment: current glyph (or blank), and a picker with every glyph, None and Default.
fn icon_row(ui: &mut egui::Ui, s: &mut Settings, key: String, label: &str, default: Option<Glyph>) -> bool {
    use crate::ui::tools::{glyph_label, glyph_text_button};
    let mut changed = false;
    ui.label(label);
    let cur = match s.icon_overrides.get(&key) {
        Some(n) if n == "none" => None,
        Some(n) => Glyph::from_name(n).or(default),
        None => default,
    };
    ui.horizontal(|ui| {
        match cur {
            Some(g) => {
                glyph_label(ui, g, ui.visuals().text_color());
            }
            None => ui.add_space(18.0),
        }
        ui.menu_button("Change", |ui| {
            ui.set_max_width(260.0);
            ui.horizontal_wrapped(|ui| {
                for g in Glyph::ALL {
                    if glyph_text_button(ui, *g, "").on_hover_text(g.name()).clicked() {
                        s.icon_overrides.insert(key.clone(), g.name().to_string());
                        changed = true;
                        ui.close();
                    }
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("None").clicked() {
                    s.icon_overrides.insert(key.clone(), "none".into());
                    changed = true;
                    ui.close();
                }
                if ui.button("Default").clicked() {
                    s.icon_overrides.remove(&key);
                    changed = true;
                    ui.close();
                }
            });
        });
    });
    ui.end_row();
    changed
}

/// One overridable colour: checkbox turns the override on/off, the button edits it while on (shown
/// at `fallback` - today's derived colour - while off, so turning it on starts from what's in effect).
pub(super) fn color_row(ui: &mut egui::Ui, label: &str, ov: &mut Option<[u8; 3]>, fallback: egui::Color32) -> bool {
    let mut changed = false;
    ui.label(label);
    let mut on = ov.is_some();
    let mut rgb = ov.unwrap_or([fallback.r(), fallback.g(), fallback.b()]);
    ui.horizontal(|ui| {
        changed |= ui.checkbox(&mut on, "").changed();
        ui.add_enabled_ui(on, |ui| {
            changed |= egui::color_picker::color_edit_button_srgb(ui, &mut rgb).changed();
        });
    });
    ui.end_row();
    *ov = on.then_some(rgb);
    changed
}
