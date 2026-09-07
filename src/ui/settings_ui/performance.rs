//! The Performance tab: GPU preview, preview render quality, playback cache size, proxy media, movie
//! mode and the effect-thumbnail source image. Extracted verbatim from settings_ui.rs (see mod.rs's
//! module doc for the whole settings window).

use crate::settings::Settings;
use eframe::egui;

/// `clear_caches`: ---- ws:forgiveness ---- — set true when the button below is clicked; the app
/// (windows.rs) reads it after calling `show`/`performance` and runs `caches::clear`, since this leaf
/// function has no `&mut App` to call it with directly.
pub(super) fn performance(ui: &mut egui::Ui, s: &mut Settings, gpu_name: &str, clear_caches: &mut bool) -> bool {
    let mut changed = false;
    egui::Grid::new("perf").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        ui.label("GPU preview");
        ui.vertical(|ui| {
            changed |= ui.checkbox(&mut s.gpu, "Render the preview with OpenGL shaders").changed();
            ui.weak(format!("Renderer: {gpu_name}"));
            ui.weak("Off (or when the driver refuses the shaders) the CPU compositor is used.");
        });
        ui.end_row();

        ui.label("Preview quality");
        ui.horizontal(|ui| {
            let mut q = s.preview_quality.clamp(25, 100);
            changed |= ui.add(egui::DragValue::new(&mut q).range(25..=100).suffix(" %").speed(1)).changed();
            s.preview_quality = q;
            ui.weak("of the preview size — lower is faster, the export is unaffected");
        });
        ui.end_row();

        ui.label("Playback cache");
        ui.horizontal(|ui| {
            let mut mb = s.cache_mb.min(16384);
            changed |= ui.add(egui::DragValue::new(&mut mb).range(0..=16384).suffix(" MB").speed(64)).changed();
            s.cache_mb = mb;
            let auto = crate::playback::cache_budget_bytes(0) >> 20;
            ui.weak(format!(
                "0 = automatic ({auto} MB here: ¼ of RAM, 512 MB–4 GB). Bigger survives longer 4K \
                 scrubs; decoded source frames use up to another quarter of it."
            ));
        });
        ui.end_row();

        // ---- ws:forgiveness ----
        ui.label("On-disk cache");
        ui.horizontal(|ui| {
            let mb = crate::ui::app::caches_bytes_for_ui() as f64 / 1e6;
            ui.weak(format!("{mb:.1} MB on disk (waveform peaks, thumbnails)"));
            if ui.button("Clear Caches").clicked() {
                *clear_caches = true;
            }
        });
        ui.end_row();

        ui.label("Proxy media");
        ui.vertical(|ui| {
            changed |= ui.checkbox(&mut s.use_proxies, "Play low-res all-intra proxies in the preview").changed();
            ui.horizontal(|ui| {
                let mut h = s.proxy_height.clamp(120, 2160);
                changed |= ui.add(egui::DragValue::new(&mut h).range(120..=2160).suffix(" px").speed(10)).changed();
                s.proxy_height = h;
                ui.weak("proxy height — built in the background, exports always use the originals");
            });
        });
        ui.end_row();

        ui.label("Movie mode");
        ui.vertical(|ui| {
            changed |= ui.checkbox(&mut s.movie_mode, "Play back pre-rendered frames").changed();
            ui.weak("Renders the in/out range (or the whole timeline) at full quality in the background.");
        });
        ui.end_row();

        ui.label("Effect thumbnails");
        ui.horizontal(|ui| {
            changed |= ui
                .add(
                    egui::TextEdit::singleline(&mut s.effect_thumb_image)
                        .desired_width(200.0)
                        .hint_text("built-in image"),
                )
                .changed();
            if ui.button("Browse…").clicked() {
                if let Some(p) =
                    rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg", "bmp", "webp"]).pick_file()
                {
                    s.effect_thumb_image = p.to_string_lossy().into_owned();
                    changed = true;
                }
            }
            if ui
                .add_enabled(!s.effect_thumb_image.is_empty(), egui::Button::new("Reset to default"))
                .on_hover_text("Render the catalogue over the image built into the app")
                .clicked()
            {
                s.effect_thumb_image.clear();
                changed = true;
            }
        });
        ui.end_row();
    });
    changed
}
